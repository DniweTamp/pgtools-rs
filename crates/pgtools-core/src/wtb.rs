//! `WTB` and `WTA`+`WTP` texture pack parser, extractor, repacker, and Wii U -> PC
//! converter (`lib/bayonetta/wtb.rb`, `wtx_extractor.rb`, `wtx_creator.rb`, `wtb_convert_wiiu_pc.rb`,
//! and `XT1 Studio` `WtbBank.cs`).

use crate::io::{align, checked_count, BinReader, BinWriter, Endian};
use crate::texture_conv::{
    convert_gtx_to_dds, inspect_texture_blob, parse_bntx_textures,
    texture_blob_has_pixel_payload_at, TextureDimension, TextureMetadata,
};
use crate::uvd::compute_uvd_group_id;

/// Bayonetta 3 / Astral Chain (`SwitchXt1`) `table2` (`flags`) bitmasks when read as a little-endian `u32`
/// (`[b0, b1, b2, b3]` on disk -> `u32::from_le_bytes([b0, b1, b2, b3])`).
pub const XT1_FLAG_HAS_CRC32: u32 = 0x0000_0020; // b0 & 0x20: texture carries a CRC32 hash in table3
pub const XT1_FLAG_BASE: u32 = 0x6000_0000;      // b3 & 0x60: base bits, always set on valid XT1 records
pub const XT1_FLAG_SRGB: u32 = 0x0400_0000;      // b3 & 0x04: sRGB color space
pub const XT1_FLAG_CUBEMAP: u32 = 0x8000_0000;   // b3 & 0x80: cubemap / env-map usage
use crate::{PgError, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

pub fn wtb_alignment_for_ext(ext: &str) -> usize {
    match ext {
        ".dds" | ".xt1" => 0x1000,
        ".gtx" | ".bntx" => 0x2000,
        _ => 0x20,
    }
}

/// Identifies the specific game/platform layout of a `.wtb` or `.wta`+`.wtp` container
/// so the UI only displays fields/flag bitmasks that are valid for that format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WtbGameProfile {
    /// Bayonetta 1 / Vanquish / MGRR PC `.wtb` or `.wta`+`.wtp` containing `.dds` textures (no `XT1` headers, no DXGI info table).
    Bayo1PcDds,
    /// NieR:Automata PC `.wta`+`.wtp` containing `.dds` textures with 20-byte DXGI info structs per entry.
    NierAutomataDds,
    /// Bayonetta 2 Wii U big-endian `\0BTW` containing `.gtx` (`GX2Surface`) textures.
    Bayo2WiiUGtx,
    /// Bayonetta 2 Switch `.wtb` or `.wta`+`.wtp` containing `.bntx` (`BNTX` / `BRTI`) textures.
    Bayo2SwitchBntx,
    /// Astral Chain / Bayonetta 3 Switch `.wtb` or `.wta`+`.wtp` with 56-byte (`0x38`) `XT1\0` headers in `offset_texture_infos`.
    SwitchXt1,
}

impl WtbGameProfile {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Bayo1PcDds => "Bayonetta 1 / MGRR / Vanquish (PC DDS)",
            Self::NierAutomataDds => "NieR:Automata (PC DDS + Info Table)",
            Self::Bayo2WiiUGtx => "Bayonetta 2 (Wii U GX2 / GTX)",
            Self::Bayo2SwitchBntx => "Bayonetta 2 (Switch BNTX)",
            Self::SwitchXt1 => "Astral Chain / Bayonetta 3 (Switch XT1)",
        }
    }

    /// Returns true only when `table2` (`flags`) uses the confirmed Bayonetta 3 / XT1 Studio
    /// little-endian `u32` bitmask (`0x00000020` Has CRC32, `0x60000000` Base, `0x04000000` sRGB, `0x80000000` Cubemap).
    pub fn has_confirmed_xt1_flags(&self) -> bool {
        matches!(self, Self::SwitchXt1)
    }

    pub fn default_entry_flags(self, is_split_wta_wtp: bool, has_table3_crc: bool) -> u32 {
        match self {
            Self::SwitchXt1 => {
                XT1_FLAG_BASE | if has_table3_crc { XT1_FLAG_HAS_CRC32 } else { 0 }
            }
            Self::Bayo2WiiUGtx | Self::NierAutomataDds => {
                if is_split_wta_wtp {
                    0x6000_0020
                } else {
                    0x6000_0002
                }
            }
            Self::Bayo2SwitchBntx => 0,
            Self::Bayo1PcDds => {
                if is_split_wta_wtp {
                    0x2000_0000
                } else {
                    0x2000_0004
                }
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WtbTextureEntry {
    pub index: usize,
    pub extension: String,
    pub flags: u32,
    pub texture_id: Option<u32>,
    pub metadata: Option<TextureMetadata>,
    #[serde(default)]
    pub content_hash: String,
    #[serde(skip)]
    pub data: Vec<u8>,
    #[serde(default)]
    pub original_offset: Option<u32>,
    #[serde(default)]
    pub original_size: Option<u32>,
}

impl WtbTextureEntry {
    #[inline]
    pub fn has_pixel_payload(&self) -> bool {
        texture_blob_has_pixel_payload_at(&self.data, Some(self.index))
    }

    /// Synchronize `table2` (`flags`) sRGB (`0x0400_0000`) and cubemap (`0x8000_0000`) bits
    /// from `self.metadata` ONLY when `game_profile == WtbGameProfile::SwitchXt1`.
    /// Never modifies non-SwitchXt1 banks, and never enables the unknown `AF_L8` (`0x0800_0000`) bit by default.
    pub fn sync_xt1_flags_from_metadata(&mut self, game_profile: WtbGameProfile) {
        if game_profile != WtbGameProfile::SwitchXt1 {
            return;
        }
        self.flags |= XT1_FLAG_BASE;
        if let Some(ref meta) = self.metadata {
            if meta.format.is_srgb() {
                self.flags |= XT1_FLAG_SRGB;
            } else {
                self.flags &= !XT1_FLAG_SRGB;
            }
            if meta.dimension == TextureDimension::TexCube {
                self.flags |= XT1_FLAG_CUBEMAP;
            } else {
                self.flags &= !XT1_FLAG_CUBEMAP;
            }
        }
    }
}

fn compute_entry_hash(index: usize, data: &[u8]) -> String {
    let mut hasher = crate::scene::Fnv1a64::new();
    hasher.update(&(index as u32).to_le_bytes());
    hasher.update(data);
    hasher.finish_hex()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WtbFile {
    pub endian: Endian,
    pub is_split_wta_wtp: bool,
    pub game_profile: WtbGameProfile,
    pub unknown: u32,
    pub entries: Vec<WtbTextureEntry>,
    pub texture_infos: Vec<u32>,
    /// Original per-texture mip offsets (big-endian GX2 WTA only), used to preserve the mip
    /// table when a header-only bank is re-serialized without rebuilding its WTP payload.
    #[serde(default)]
    pub original_mip_offsets: Vec<u32>,
}

impl WtbFile {
    pub fn new(endian: Endian, is_split_wta_wtp: bool) -> Self {
        Self {
            endian,
            is_split_wta_wtp,
            game_profile: if endian.is_big() {
                WtbGameProfile::Bayo2WiiUGtx
            } else {
                WtbGameProfile::Bayo1PcDds
            },
            unknown: 1,
            entries: Vec::new(),
            texture_infos: Vec::new(),
            original_mip_offsets: Vec::new(),
        }
    }

    pub fn new_empty(is_split_wta_wtp: bool, game_profile: WtbGameProfile) -> Self {
        let endian = if game_profile == WtbGameProfile::Bayo2WiiUGtx {
            Endian::Big
        } else {
            Endian::Little
        };
        let unknown = if game_profile == WtbGameProfile::SwitchXt1 {
            3
        } else {
            1
        };
        Self {
            endian,
            is_split_wta_wtp,
            game_profile,
            unknown,
            entries: Vec::new(),
            texture_infos: Vec::new(),
            original_mip_offsets: Vec::new(),
        }
    }

    /// Construct a virtual `WtbFile` from a standalone Switch `.bntx` container so all `BRTI`
    /// textures inside the `.bntx` can be inspected, previewed, and exported individually.
    pub fn from_bntx_bytes(bntx_bytes: &[u8]) -> Result<Self> {
        let parsed = parse_bntx_textures(bntx_bytes)?;
        let mut entries = Vec::with_capacity(parsed.len());
        for (i, tex) in parsed.into_iter().enumerate() {
            let tid = compute_uvd_group_id(&tex.name);
            let dimension = if tex.surface_dim == 3 || tex.array_length == 6 {
                TextureDimension::TexCube
            } else if tex.depth > 1 || tex.array_length > 1 {
                TextureDimension::TexAnimatedLayers
            } else {
                TextureDimension::Tex2D
            };
            let layer_count = match dimension {
                TextureDimension::TexCube => 6,
                TextureDimension::TexAnimatedLayers => tex.depth.max(tex.array_length).max(1),
                TextureDimension::Tex2D => 1,
            };
            let meta = TextureMetadata {
                container_ext: ".bntx".to_string(),
                format: tex.format,
                dimension,
                width: tex.width,
                height: tex.height,
                depth: tex.depth,
                layer_count,
                mip_count: tex.mip_count as u32,
                data_size: tex.image_size as usize,
                name: Some(tex.name),
            };
            let content_hash = compute_entry_hash(i, bntx_bytes);
            entries.push(WtbTextureEntry {
                index: i,
                extension: ".bntx".to_string(),
                flags: 0,
                texture_id: Some(tid),
                metadata: Some(meta),
                content_hash,
                // Store the parent BNTX bytes; sub-texture decoding uses index `i` when needed.
                data: bntx_bytes.to_vec(),
                original_offset: None,
                original_size: None,
            });
        }
        Ok(Self {
            endian: Endian::Little,
            is_split_wta_wtp: false,
            game_profile: WtbGameProfile::Bayo2SwitchBntx,
            unknown: 1,
            entries,
            texture_infos: Vec::new(),
            original_mip_offsets: Vec::new(),
        })
    }

    /// Fast reader that extracts only the texture IDs from a `.wtb`, `.wta`, or `.bntx` header
    /// without slicing or inspecting the texture payloads.
    pub fn read_texture_ids_only(wtb_or_wta: &[u8]) -> Result<Vec<u32>> {
        if wtb_or_wta.len() >= 4 && &wtb_or_wta[0..4] == b"BNTX" {
            let parsed = parse_bntx_textures(wtb_or_wta)?;
            return Ok(parsed
                .into_iter()
                .map(|tex| crc32fast::hash(tex.name.as_bytes()))
                .collect());
        }
        if wtb_or_wta.len() < 0x20 {
            return Err(PgError::InvalidData("WTB/WTA shorter than 32-byte header".into()));
        }
        let endian = match &wtb_or_wta[0..4] {
            b"WTB\0" => Endian::Little,
            b"\0BTW" => Endian::Big,
            other => {
                return Err(PgError::InvalidMagic {
                    expected: "WTB\\0 or \\0BTW",
                    actual: other.to_vec(),
                })
            }
        };
        let mut r = BinReader::new(wtb_or_wta, endian);
        r.seek(8)?;
        let num_tex = r.read_u32()? as usize;
        let offset_texture_offsets = r.read_u32()? as usize;
        let _offset_texture_sizes = r.read_u32()? as usize;
        let _offset_texture_flags = r.read_u32()? as usize;
        let offset_texture_idx = r.read_u32()? as usize;
        if num_tex == 0 {
            return Ok(Vec::new());
        }
        checked_count(
            num_tex,
            4,
            wtb_or_wta.len().saturating_sub(offset_texture_offsets),
            "WTB texture_offsets",
        )?;
        if offset_texture_idx != 0 {
            checked_count(
                num_tex,
                4,
                wtb_or_wta.len().saturating_sub(offset_texture_idx),
                "WTB texture_idx",
            )?;
            r.seek(offset_texture_idx)?;
            let mut ids = Vec::with_capacity(num_tex);
            for _ in 0..num_tex {
                ids.push(r.read_u32()?);
            }
            Ok(ids)
        } else {
            Ok((0..num_tex as u32).collect())
        }
    }

    pub fn from_bytes(wtb_or_wta: &[u8], wtp: Option<&[u8]>) -> Result<Self> {
        if wtb_or_wta.len() >= 4 && &wtb_or_wta[0..4] == b"BNTX" {
            return Self::from_bntx_bytes(wtb_or_wta);
        }
        if wtb_or_wta.len() < 0x20 {
            return Err(PgError::InvalidData("WTB/WTA shorter than 32-byte header".into()));
        }
        let magic = &wtb_or_wta[0..4];
        let (endian, mut default_ext) = match magic {
            b"WTB\0" => {
                let mut ext = ".dds";
                if let Some(wtp_bytes) = wtp {
                    if wtp_bytes.len() >= 4 && &wtp_bytes[0..4] == b"BNTX" {
                        ext = ".bntx";
                    }
                }
                (Endian::Little, ext)
            }
            b"\0BTW" => (Endian::Big, ".gtx"),
            other => {
                return Err(PgError::InvalidMagic {
                    expected: "WTB\\0 or \\0BTW",
                    actual: other.to_vec(),
                })
            }
        };

        let mut r = BinReader::new(wtb_or_wta, endian);
        let _id = r.read_bytes(4)?;
        let unknown = r.read_u32()?;
        let num_tex = r.read_u32()? as usize;
        let offset_texture_offsets = r.read_u32()? as usize;
        let offset_texture_sizes = r.read_u32()? as usize;
        let offset_texture_flags = r.read_u32()? as usize;
        let offset_texture_idx = r.read_u32()? as usize;
        let offset_texture_infos = r.read_u32()? as usize;

        if offset_texture_infos != 0
            && num_tex > 0
            && offset_texture_infos + 4 <= wtb_or_wta.len()
            && &wtb_or_wta[offset_texture_infos..offset_texture_infos + 4] == b"XT1\0"
        {
            default_ext = ".xt1";
        }

        if num_tex > 0 {
            checked_count(
                num_tex,
                4,
                wtb_or_wta.len().saturating_sub(offset_texture_offsets),
                "WTB texture_offsets",
            )?;
        }

        let mipmap_offsets: Vec<usize> = if endian.is_big()
            && (wtp.is_some() || offset_texture_infos != 0)
            && wtb_or_wta.len() >= 0x24
        {
            let off_mip = r.read_u32()? as usize;
            if off_mip != 0 {
                checked_count(
                    num_tex,
                    4,
                    wtb_or_wta.len().saturating_sub(off_mip),
                    "WTB mipmap_offsets",
                )?;
                r.seek(off_mip)?;
                let mut mips = Vec::with_capacity(num_tex);
                for _ in 0..num_tex {
                    mips.push(r.read_u32()? as usize);
                }
                mips
            } else {
                vec![0; num_tex]
            }
        } else {
            Vec::new()
        };

        r.seek(offset_texture_offsets)?;
        let mut texture_offsets = Vec::with_capacity(num_tex);
        for _ in 0..num_tex {
            texture_offsets.push(r.read_u32()? as usize);
        }

        let mut texture_sizes = vec![0usize; num_tex];
        if offset_texture_sizes != 0 {
            r.seek(offset_texture_sizes)?;
            for slot in &mut texture_sizes {
                *slot = r.read_u32()? as usize;
            }
        }

        let mut texture_flags = vec![0u32; num_tex];
        if offset_texture_flags != 0 {
            r.seek(offset_texture_flags)?;
            for slot in &mut texture_flags {
                *slot = r.read_u32()?;
            }
        }

        let mut texture_idx = Vec::new();
        if offset_texture_idx != 0 {
            r.seek(offset_texture_idx)?;
            for _ in 0..num_tex {
                texture_idx.push(r.read_u32()?);
            }
        }

        if default_ext == ".dds" {
            for i in 0..num_tex {
                let off = texture_offsets[i];
                let source = wtp.unwrap_or(wtb_or_wta);
                if texture_sizes[i] > 4 && off + 4 <= source.len() && &source[off..off + 4] == b"BNTX" {
                    default_ext = ".bntx";
                    break;
                }
            }
        }

        let is_header_only_wta = wtp.is_none()
            && num_tex > 0
            && (texture_offsets[0] < 0x20
                || texture_offsets
                    .iter()
                    .zip(texture_sizes.iter())
                    .any(|(&off, &sz)| sz > 0 && off.saturating_add(sz) > wtb_or_wta.len()));
        let effective_wtp: Option<&[u8]> = if is_header_only_wta {
            Some(&[])
        } else {
            wtp
        };

        let mut texture_infos = Vec::new();
        let mut entries = Vec::with_capacity(num_tex);

        for i in 0..num_tex {
            let off = texture_offsets[i];
            let sz = texture_sizes[i];
            let blob = match (effective_wtp, default_ext, endian) {
                (None, ".xt1", _) => {
                    let hdr = r.slice_at(offset_texture_infos + i * 0x38, 0x38)?;
                    let body = if off + sz <= wtb_or_wta.len() {
                        &wtb_or_wta[off..off + sz]
                    } else if off <= wtb_or_wta.len() {
                        &wtb_or_wta[off..]
                    } else {
                        &[][..]
                    };
                    let mut v = Vec::with_capacity(0x38 + body.len());
                    v.extend_from_slice(hdr);
                    v.extend_from_slice(body);
                    v
                }
                (Some(wtp_bytes), ".xt1", _) => {
                    let hdr = r.slice_at(offset_texture_infos + i * 0x38, 0x38)?;
                    let end = (off + sz).min(wtp_bytes.len());
                    let body = if off < end { &wtp_bytes[off..end] } else { &[][..] };
                    let mut v = Vec::with_capacity(0x38 + body.len());
                    v.extend_from_slice(hdr);
                    v.extend_from_slice(body);
                    v
                }
                (None, _, _) => {
                    let end = (off + sz).min(wtb_or_wta.len());
                    if off < end {
                        wtb_or_wta[off..end].to_vec()
                    } else {
                        Vec::new()
                    }
                }
                (Some(wtp_bytes), _, Endian::Big) => {
                    // Bayonetta 2 Wii U WTA + WTP -> assemble valid `.gtx` (`Gfx2` + `BLK{` blocks)
                    let gx2 = r.slice_at(offset_texture_infos + i * 0xC0, 0x9C)?;
                    let num_mipmap = u32::from_be_bytes(gx2[0x10..0x14].try_into().unwrap());
                    let data_len = u32::from_be_bytes(gx2[0x20..0x24].try_into().unwrap()) as usize;
                    let mip_len = u32::from_be_bytes(gx2[0x28..0x2C].try_into().unwrap()) as usize;

                    let mut out = Vec::with_capacity(0x100 + data_len + mip_len);
                    out.extend_from_slice(b"Gfx2\x00\x00\x00\x20\x00\x00\x00\x07\x00\x00\x00\x01\x00\x00\x00\x02\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00");
                    out.extend_from_slice(b"BLK{\x00\x00\x00\x20\x00\x00\x00\x01\x00\x00\x00\x00\x00\x00\x00\x0B\x00\x00\x00\x9C\x00\x00\x00\x00\x00\x00\x00\x00");
                    out.extend_from_slice(gx2);
                    out.extend_from_slice(b"BLK{\x00\x00\x00\x20\x00\x00\x00\x01\x00\x00\x00\x00\x00\x00\x00\x0C");
                    out.extend_from_slice(&gx2[0x20..0x24]);
                    out.extend_from_slice(&[0u8; 8]);
                    if off + data_len <= wtp_bytes.len() {
                        out.extend_from_slice(&wtp_bytes[off..off + data_len]);
                    }
                    if num_mipmap > 1 {
                        let mip_off = mipmap_offsets.get(i).copied().unwrap_or(0);
                        out.extend_from_slice(b"BLK{\x00\x00\x00\x20\x00\x00\x00\x01\x00\x00\x00\x00\x00\x00\x00\x0D");
                        out.extend_from_slice(&gx2[0x28..0x2C]);
                        out.extend_from_slice(&[0u8; 8]);
                        if mip_off + mip_len <= wtp_bytes.len() {
                            out.extend_from_slice(&wtp_bytes[mip_off..mip_off + mip_len]);
                        }
                    }
                    out.extend_from_slice(b"BLK{\x00\x00\x00\x20\x00\x00\x00\x01\x00\x00\x00\x00\x00\x00\x00\x01\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00");
                    out
                }
                (Some(wtp_bytes), _, Endian::Little) => {
                    let end = (off + sz).min(wtp_bytes.len());
                    if off < end {
                        wtp_bytes[off..end].to_vec()
                    } else {
                        Vec::new()
                    }
                }
            };

            let metadata = inspect_texture_blob(&blob).ok();
            let content_hash = compute_entry_hash(i, &blob);
            entries.push(WtbTextureEntry {
                index: i,
                extension: default_ext.to_string(),
                flags: texture_flags[i],
                texture_id: texture_idx.get(i).copied(),
                metadata,
                content_hash,
                data: blob,
                original_offset: Some(off as u32),
                original_size: Some(sz as u32),
            });
        }

        if effective_wtp.is_some() && !endian.is_big() && default_ext != ".xt1" && offset_texture_infos != 0 {
            if offset_texture_infos + 20 * num_tex <= wtb_or_wta.len() {
                r.seek(offset_texture_infos)?;
                for _ in 0..(5 * num_tex) {
                    texture_infos.push(r.read_u32()?);
                }
            }
        }

        let game_profile = match default_ext {
            ".xt1" => WtbGameProfile::SwitchXt1,
            ".gtx" => WtbGameProfile::Bayo2WiiUGtx,
            ".bntx" => WtbGameProfile::Bayo2SwitchBntx,
            _ => {
                if !texture_infos.is_empty() {
                    WtbGameProfile::NierAutomataDds
                } else {
                    WtbGameProfile::Bayo1PcDds
                }
            }
        };

        Ok(Self {
            endian,
            is_split_wta_wtp: effective_wtp.is_some(),
            game_profile,
            unknown,
            entries,
            texture_infos,
            original_mip_offsets: mipmap_offsets
                .iter()
                .map(|&v| v.min(u32::MAX as usize) as u32)
                .collect(),
        })
    }

    pub fn push(&mut self, data: Vec<u8>, mut flags: u32, texture_id: Option<u32>) -> Result<()> {
        let ext = match data.get(0..4) {
            Some(b"XT1\0") => ".xt1",
            Some(b"Gfx2") => ".gtx",
            Some(b"DDS ") => ".dds",
            Some(b"BNTX") => ".bntx",
            _ if data.len() >= 84 && &data[80..84] == b"BNTX" => ".bntx",
            _ => return Err(PgError::InvalidData("Unsupported texture magic in WTB::push".into())),
        };
        if self.entries.is_empty() {
            self.game_profile = match ext {
                ".xt1" => WtbGameProfile::SwitchXt1,
                ".gtx" => WtbGameProfile::Bayo2WiiUGtx,
                ".bntx" => WtbGameProfile::Bayo2SwitchBntx,
                _ => WtbGameProfile::Bayo1PcDds,
            };
        }
        let has_crc = texture_id.is_some()
            || self.entries.iter().any(|e| {
                e.texture_id.is_some()
                    || (self.game_profile == WtbGameProfile::SwitchXt1
                        && (e.flags & XT1_FLAG_HAS_CRC32) != 0)
            });
        if flags == 0 {
            if self.game_profile == WtbGameProfile::SwitchXt1 || ext == ".xt1" {
                flags = self
                    .game_profile
                    .default_entry_flags(self.is_split_wta_wtp, has_crc);
            } else if let Some(first) = self.entries.first() {
                flags = first.flags;
            } else {
                flags = self
                    .game_profile
                    .default_entry_flags(self.is_split_wta_wtp, has_crc);
            }
        }
        let tid = if (self.endian.is_big() || ext == ".xt1") && texture_id.is_none() && has_crc {
            Some(crc32fast::hash(&data) & 0x7FFF_FFFF)
        } else {
            texture_id
        };
        let metadata = inspect_texture_blob(&data).ok();
        let index = self.entries.len();
        let content_hash = compute_entry_hash(index, &data);
        let mut entry = WtbTextureEntry {
            index,
            extension: ext.to_string(),
            flags,
            texture_id: tid,
            metadata,
            content_hash,
            data,
            original_offset: None,
            original_size: None,
        };
        if self.game_profile == WtbGameProfile::SwitchXt1 {
            entry.sync_xt1_flags_from_metadata(self.game_profile);
        }
        self.entries.push(entry);
        Ok(())
    }

    /// Serialize into `(wtb_or_wta_bytes, optional_wtp_bytes)`.
    pub fn to_bytes(&self) -> (Vec<u8>, Option<Vec<u8>>) {
        let num_tex = self.entries.len();
        let default_profile_ext = match self.game_profile {
            WtbGameProfile::SwitchXt1 => ".xt1",
            WtbGameProfile::Bayo2WiiUGtx => ".gtx",
            WtbGameProfile::Bayo2SwitchBntx => ".bntx",
            _ => ".dds",
        };
        let first_ext = self
            .entries
            .first()
            .map(|e| e.extension.as_str())
            .unwrap_or(default_profile_ext);
        let has_idx = self.entries.iter().any(|e| e.texture_id.is_some());

        let mut last_offset = if self.is_split_wta_wtp && (self.endian.is_big() || first_ext == ".bntx") {
            0x40usize
        } else {
            0x20usize
        };
        let offset_texture_offsets = last_offset;
        last_offset = align(last_offset + 4 * num_tex, 0x20);
        let offset_texture_sizes = last_offset;
        last_offset = align(last_offset + 4 * num_tex, 0x20);
        let offset_texture_flags = last_offset;
        let offset_texture_idx = if has_idx {
            last_offset = align(last_offset + 4 * num_tex, 0x20);
            last_offset
        } else {
            0
        };

        if !self.is_split_wta_wtp {
            last_offset += 4 * num_tex;
            let offset_texture_infos = if first_ext == ".xt1" {
                let info_off = align(last_offset, 0x20);
                last_offset = align(info_off + 0x38 * num_tex, 0x10);
                info_off
            } else {
                0
            };

            let mut texture_offsets = Vec::with_capacity(num_tex);
            for e in &self.entries {
                let al = wtb_alignment_for_ext(&e.extension);
                let sz = if e.extension == ".xt1" {
                    e.data.len().saturating_sub(0x38)
                } else {
                    e.data.len()
                };
                let tmp = align(last_offset, al);
                last_offset = align(tmp + sz, al);
                texture_offsets.push(tmp as u32);
            }

            let mut w = BinWriter::with_capacity(last_offset, self.endian);
            if self.endian.is_big() {
                w.write_bytes(b"\0BTW");
            } else {
                w.write_bytes(b"WTB\0");
            }
            w.write_u32(self.unknown);
            w.write_u32(num_tex as u32);
            w.write_u32(offset_texture_offsets as u32);
            w.write_u32(offset_texture_sizes as u32);
            w.write_u32(offset_texture_flags as u32);
            w.write_u32(offset_texture_idx as u32);
            w.write_u32(offset_texture_infos as u32);

            w.seek(offset_texture_offsets);
            for &off in &texture_offsets {
                w.write_u32(off);
            }
            w.seek(offset_texture_sizes);
            for e in &self.entries {
                let sz = if e.extension == ".xt1" {
                    e.data.len().saturating_sub(0x38)
                } else {
                    e.data.len()
                };
                w.write_u32(sz as u32);
            }
            w.seek(offset_texture_flags);
            for e in &self.entries {
                w.write_u32(e.flags);
            }
            if offset_texture_idx != 0 {
                w.seek(offset_texture_idx);
                for (i, e) in self.entries.iter().enumerate() {
                    w.write_u32(e.texture_id.unwrap_or(i as u32));
                }
            }
            for (i, e) in self.entries.iter().enumerate() {
                if e.extension == ".xt1" && e.data.len() >= 0x38 {
                    w.seek(offset_texture_infos + i * 0x38);
                    w.write_bytes(&e.data[..0x38]);
                    w.seek(texture_offsets[i] as usize);
                    w.write_bytes(&e.data[0x38..]);
                } else {
                    w.seek(texture_offsets[i] as usize);
                    w.write_bytes(&e.data);
                }
            }
            w.seek(last_offset);
            (w.into_inner(), None)
        } else {
            // Split WTA + WTP
            let is_gx2_wta = self.endian.is_big() && first_ext == ".gtx";
            let offset_texture_infos = if is_gx2_wta
                || (first_ext != ".bntx" && (first_ext == ".xt1" || !self.texture_infos.is_empty()))
            {
                last_offset = align(last_offset + 4 * num_tex, 0x20);
                last_offset
            } else {
                0
            };
            let offset_mipmap_offsets = if is_gx2_wta {
                let mip_table_off = align(offset_texture_infos + 0xC0 * num_tex, 0x20);
                last_offset = align(mip_table_off + 4 * num_tex, 0x20);
                mip_table_off
            } else if first_ext == ".xt1" {
                last_offset = align(last_offset + 0x38 * num_tex, 0x10);
                0
            } else if offset_texture_infos != 0 {
                last_offset = align(last_offset + 20 * num_tex, 0x20);
                0
            } else {
                last_offset = align(last_offset + 4 * num_tex, 0x20);
                0
            };

            // Disassemble synthetic .gtx entries into (gx2_0x9C, base_data, mip_data) when writing Wii U WTA+WTP
            let mut gx2_parts: Vec<([u8; 0x9C], Vec<u8>, Vec<u8>)> = Vec::new();
            if is_gx2_wta {
                for e in &self.entries {
                    let mut gx2 = [0u8; 0x9C];
                    let mut base_slice = Vec::new();
                    let mut mip_slice = Vec::new();
                    if e.data.len() >= 0x40 + 0x9C && &e.data[0..4] == b"Gfx2" {
                        gx2.copy_from_slice(&e.data[0x40..0x40 + 0x9C]);
                        let num_mip = u32::from_be_bytes(gx2[0x10..0x14].try_into().unwrap());
                        let data_len = u32::from_be_bytes(gx2[0x20..0x24].try_into().unwrap()) as usize;
                        let mip_len = u32::from_be_bytes(gx2[0x28..0x2C].try_into().unwrap()) as usize;
                        let base_start = 0x40 + 0x9C + 0x20;
                        if base_start + data_len <= e.data.len() {
                            base_slice.extend_from_slice(&e.data[base_start..base_start + data_len]);
                        }
                        if num_mip > 1 {
                            let mip_start = base_start + data_len + 0x20;
                            if mip_start + mip_len <= e.data.len() {
                                mip_slice.extend_from_slice(&e.data[mip_start..mip_start + mip_len]);
                            }
                        }
                    }
                    gx2_parts.push((gx2, base_slice, mip_slice));
                }
            }

            let any_wtp_payload = self.entries.iter().any(|e| e.has_pixel_payload());
            let mut offset_wtp = 0usize;
            let mut texture_offsets = Vec::with_capacity(num_tex);
            let mut mipmap_offsets = vec![0u32; num_tex];
            let mut texture_sizes = Vec::with_capacity(num_tex);

            if !any_wtp_payload {
                for (i, e) in self.entries.iter().enumerate() {
                    texture_offsets.push(e.original_offset.unwrap_or(0));
                    texture_sizes.push(e.original_size.unwrap_or(0));
                    if is_gx2_wta {
                        mipmap_offsets[i] = self.original_mip_offsets.get(i).copied().unwrap_or(0);
                    }
                }
            } else if is_gx2_wta {
                for (i, (_gx2, base_data, mip_data)) in gx2_parts.iter().enumerate() {
                    let al = 0x2000usize;
                    let base_off = align(offset_wtp, al);
                    offset_wtp = align(base_off + base_data.len(), al);
                    texture_offsets.push(base_off as u32);
                    let mip_off = if !mip_data.is_empty() {
                        let mo = align(offset_wtp, al);
                        offset_wtp = align(mo + mip_data.len(), al);
                        mo as u32
                    } else {
                        0
                    };
                    mipmap_offsets[i] = mip_off;
                    texture_sizes.push((base_data.len() + mip_data.len()) as u32);
                }
            } else {
                for e in &self.entries {
                    let al = wtb_alignment_for_ext(&e.extension);
                    let sz = if e.extension == ".xt1" {
                        e.data.len().saturating_sub(0x38)
                    } else {
                        e.data.len()
                    };
                    let tmp = align(offset_wtp, al);
                    offset_wtp = align(tmp + sz, al);
                    texture_offsets.push(tmp as u32);
                    texture_sizes.push(sz as u32);
                }
            }

            let mut w_wta = BinWriter::with_capacity(last_offset, self.endian);
            if self.endian.is_big() {
                w_wta.write_bytes(b"\0BTW");
            } else {
                w_wta.write_bytes(b"WTB\0");
            }
            w_wta.write_u32(self.unknown);
            w_wta.write_u32(num_tex as u32);
            w_wta.write_u32(offset_texture_offsets as u32);
            w_wta.write_u32(offset_texture_sizes as u32);
            w_wta.write_u32(offset_texture_flags as u32);
            w_wta.write_u32(offset_texture_idx as u32);
            w_wta.write_u32(offset_texture_infos as u32);
            if is_gx2_wta {
                w_wta.write_u32(offset_mipmap_offsets as u32);
            }

            w_wta.seek(offset_texture_offsets);
            for &off in &texture_offsets {
                w_wta.write_u32(off);
            }
            w_wta.seek(offset_texture_sizes);
            for &sz in &texture_sizes {
                w_wta.write_u32(sz);
            }
            w_wta.seek(offset_texture_flags);
            for e in &self.entries {
                w_wta.write_u32(e.flags);
            }
            if offset_texture_idx != 0 {
                w_wta.seek(offset_texture_idx);
                for (i, e) in self.entries.iter().enumerate() {
                    w_wta.write_u32(e.texture_id.unwrap_or(i as u32));
                }
            }
            if offset_texture_infos != 0 {
                w_wta.seek(offset_texture_infos);
                if is_gx2_wta {
                    for (i, (gx2, _, _)) in gx2_parts.iter().enumerate() {
                        w_wta.seek(offset_texture_infos + i * 0xC0);
                        w_wta.write_bytes(gx2);
                    }
                    w_wta.seek(offset_mipmap_offsets);
                    for &mo in &mipmap_offsets {
                        w_wta.write_u32(mo);
                    }
                } else if first_ext == ".xt1" {
                    for (i, e) in self.entries.iter().enumerate() {
                        if e.data.len() >= 0x38 {
                            w_wta.seek(offset_texture_infos + i * 0x38);
                            w_wta.write_bytes(&e.data[..0x38]);
                        }
                    }
                } else {
                    for &info in &self.texture_infos {
                        w_wta.write_u32(info);
                    }
                }
            }
            w_wta.seek(last_offset);

            let mut w_wtp = BinWriter::with_capacity(offset_wtp, self.endian);
            if is_gx2_wta {
                for (i, (_, base_data, mip_data)) in gx2_parts.iter().enumerate() {
                    w_wtp.seek(texture_offsets[i] as usize);
                    w_wtp.write_bytes(base_data);
                    if !mip_data.is_empty() && mipmap_offsets[i] != 0 {
                        w_wtp.seek(mipmap_offsets[i] as usize);
                        w_wtp.write_bytes(mip_data);
                    }
                }
            } else {
                for (i, e) in self.entries.iter().enumerate() {
                    w_wtp.seek(texture_offsets[i] as usize);
                    if e.extension == ".xt1" && e.data.len() >= 0x38 {
                        w_wtp.write_bytes(&e.data[0x38..]);
                    } else {
                        w_wtp.write_bytes(&e.data);
                    }
                }
            }
            if !any_wtp_payload {
                (w_wta.into_inner(), None)
            } else {
                w_wtp.seek(offset_wtp);
                (w_wta.into_inner(), Some(w_wtp.into_inner()))
            }
        }
    }

    /// Convert a Wii U `.wtb` or `.wta`+`.wtp` (containing `.gtx` textures) into a PC `.wtb`
    /// (containing `.dds` textures) purely in Rust without `TexConv2.exe`.
    pub fn convert_wiiu_to_pc(&self) -> Result<Self> {
        let mut pc_wtb = Self::new(Endian::Little, false);
        pc_wtb.unknown = self.unknown;

        for e in &self.entries {
            let mut flags = e.flags;
            if self.is_split_wta_wtp {
                flags &= !0x20;
                if flags == 0x7000_0000 {
                    flags = 0x1000_0000;
                } else {
                    flags = 0x2000_0000;
                }
            } else {
                flags &= !0x2;
            }
            let dds_bytes = if e.extension == ".gtx" {
                convert_gtx_to_dds(&e.data)?
            } else {
                e.data.clone()
            };
            pc_wtb.push(dds_bytes, flags, e.texture_id)?;
        }
        Ok(pc_wtb)
    }

    pub fn extract_to_dir(&self, output_dir: &Path, stem: &str, original_ext: &str) -> Result<()> {
        fs::create_dir_all(output_dir)?;
        for (i, e) in self.entries.iter().enumerate() {
            let fname = format!("{}_{:03}{}", stem, i, e.extension);
            fs::write(output_dir.join(fname), &e.data)?;
        }
        let meta_dir = output_dir.join(".metadata");
        fs::create_dir_all(&meta_dir)?;
        let flags: Vec<u32> = self.entries.iter().map(|e| e.flags).collect();
        let idx: Vec<u32> = self
            .entries
            .iter()
            .filter_map(|e| e.texture_id)
            .collect();
        fs::write(
            meta_dir.join("texture_flags.yaml"),
            serde_yaml::to_string(&flags).map_err(|e| PgError::Serialization(e.to_string()))?,
        )?;
        fs::write(
            meta_dir.join("texture_idx.yaml"),
            serde_yaml::to_string(&idx).map_err(|e| PgError::Serialization(e.to_string()))?,
        )?;
        fs::write(
            meta_dir.join("texture_infos.yaml"),
            serde_yaml::to_string(&self.texture_infos)
                .map_err(|e| PgError::Serialization(e.to_string()))?,
        )?;
        fs::write(
            meta_dir.join("big.yaml"),
            serde_yaml::to_string(&self.endian.is_big())
                .map_err(|e| PgError::Serialization(e.to_string()))?,
        )?;
        fs::write(
            meta_dir.join("unknown.yaml"),
            serde_yaml::to_string(&self.unknown)
                .map_err(|e| PgError::Serialization(e.to_string()))?,
        )?;
        fs::write(
            meta_dir.join("extension.yaml"),
            serde_yaml::to_string(&original_ext)
                .map_err(|e| PgError::Serialization(e.to_string()))?,
        )?;
        Ok(())
    }
}
