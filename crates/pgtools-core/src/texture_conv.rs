//! Self-contained, pure-Rust texture conversion, swizzling, and BCn/ASTC/RGBA8 codec pipeline.
//!
//! Full feature parity with `XT1 Studio` and `Switch-Toolbox`:
//! - DirectDraw Surface (`.dds`) header parsing and creation (BC1..BC7, RGBA8, BGRA8, R8, RG8, RGBA16F)
//! - Wii U GX2 (`.gtx`) `Gfx2` / `BLK{` parsing, exact Wii U `AddrLib` (`GX2.cs`) MacroTiled/MicroTiled
//!   surface alignment & swizzle/deswizzle (including `pipeSwizzle` & `bankSwizzle` from `swizzleValue`),
//!   and `.gtx` <-> `.dds` conversion
//! - Nintendo Switch `.xt1` (Bayonetta 3 / Astral Chain) exact `GobSwizzle` & `SurfaceLayout` (including
//!   wide-pitch `0x403` and single-mip ASTC pitch recovery) + all 14 ASTC footprints (`4x4`..`12x12`)
//! - Nintendo Switch `.bntx` (`BNTX` / `NX  ` / `BRTI` / `BRTD`) container parser, Tegra X1 GOB deswizzler,
//!   and in-place texture replacement
//! - `Solidify` multi-source BFS alpha edge-bleeding filter + Box mipmap chain generator (`MipChain`)
//! - Dynamic `nvtt30205.dll` / `nvtt.dll` hardware/native compressor loader with automatic pure-Rust
//!   fallback encoder (BC1, BC2, BC3, BC4, BC5, RGBA8, BGRA8, R8).

use crate::astc::{
    decode_astc_linear_to_rgba8, decode_bc6h_block, decode_bc7_block,
    encode_rgba8_to_astc_linear, encode_rgba8_to_bc6h_linear, encode_rgba8_to_bc7_linear,
};
use crate::io::{align, BinReader, BinWriter, Endian, F16};
use crate::{PgError, Result};
use image::{ImageBuffer, RgbaImage};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TextureFormat {
    AfL8Unorm,
    R8Unorm,
    Rg8Unorm,
    Rgba8,
    Rgba8Srgb,
    Bgra8,
    Bgra8Srgb,
    Rgba16Float,
    Bc1,
    Bc1Srgb,
    Bc2,
    Bc2Srgb,
    Bc3,
    Bc3Srgb,
    Bc4,
    Bc5,
    Bc6hUf16,
    Bc6hSf16,
    Bc7,
    Bc7Srgb,
    /// `(block_width, block_height, is_srgb)` — covers all 14 ASTC footprints (`4x4`..`12x12`)
    Astc(u8, u8, bool),
    Unknown(u32),
}

pub const ASTC_FOOTPRINTS: [(u8, u8, &str); 14] = [
    (4, 4, "4x4"),
    (5, 4, "5x4"),
    (5, 5, "5x5"),
    (6, 5, "6x5"),
    (6, 6, "6x6"),
    (8, 5, "8x5"),
    (8, 6, "8x6"),
    (8, 8, "8x8"),
    (10, 5, "10x5"),
    (10, 6, "10x6"),
    (10, 8, "10x8"),
    (10, 10, "10x10"),
    (12, 10, "12x10"),
    (12, 12, "12x12"),
];

impl TextureFormat {
    pub const fn block_dimensions(self) -> (usize, usize) {
        match self {
            Self::AfL8Unorm
            | Self::R8Unorm
            | Self::Rg8Unorm
            | Self::Rgba8
            | Self::Rgba8Srgb
            | Self::Bgra8
            | Self::Bgra8Srgb
            | Self::Rgba16Float => (1, 1),
            Self::Astc(bw, bh, _) => (bw as usize, bh as usize),
            _ => (4, 4),
        }
    }

    pub const fn bytes_per_block(self) -> usize {
        match self {
            Self::AfL8Unorm | Self::R8Unorm => 1,
            Self::Rg8Unorm => 2,
            Self::Rgba8 | Self::Rgba8Srgb | Self::Bgra8 | Self::Bgra8Srgb => 4,
            Self::Rgba16Float | Self::Bc1 | Self::Bc1Srgb | Self::Bc4 => 8,
            Self::Bc2
            | Self::Bc2Srgb
            | Self::Bc3
            | Self::Bc3Srgb
            | Self::Bc5
            | Self::Bc6hUf16
            | Self::Bc6hSf16
            | Self::Bc7
            | Self::Bc7Srgb
            | Self::Astc(_, _, _)
            | Self::Unknown(_) => 16,
        }
    }

    pub const fn is_srgb(self) -> bool {
        matches!(
            self,
            Self::Rgba8Srgb
                | Self::Bgra8Srgb
                | Self::Bc1Srgb
                | Self::Bc2Srgb
                | Self::Bc3Srgb
                | Self::Bc7Srgb
                | Self::Astc(_, _, true)
        )
    }

    pub fn display_name(self) -> String {
        match self {
            Self::AfL8Unorm => "AF_L8_UNORM (0x01)".into(),
            Self::R8Unorm => "R8_UNORM".into(),
            Self::Rg8Unorm => "R8G8_UNORM".into(),
            Self::Rgba8 => "RGBA8_UNORM".into(),
            Self::Rgba8Srgb => "RGBA8_SRGB".into(),
            Self::Bgra8 => "BGRA8_UNORM".into(),
            Self::Bgra8Srgb => "BGRA8_SRGB".into(),
            Self::Rgba16Float => "RGBA16_FLOAT".into(),
            Self::Bc1 => "BC1_UNORM (DXT1)".into(),
            Self::Bc1Srgb => "BC1_SRGB (DXT1)".into(),
            Self::Bc2 => "BC2_UNORM (DXT3)".into(),
            Self::Bc2Srgb => "BC2_SRGB (DXT3)".into(),
            Self::Bc3 => "BC3_UNORM (DXT5)".into(),
            Self::Bc3Srgb => "BC3_SRGB (DXT5)".into(),
            Self::Bc4 => "BC4_UNORM (ATI1)".into(),
            Self::Bc5 => "BC5_UNORM (ATI2)".into(),
            Self::Bc6hUf16 => "BC6H_UF16".into(),
            Self::Bc6hSf16 => "BC6H_SF16".into(),
            Self::Bc7 => "BC7_UNORM".into(),
            Self::Bc7Srgb => "BC7_SRGB".into(),
            Self::Astc(bw, bh, srgb) => {
                if srgb {
                    format!("ASTC_LDR_{bw}x{bh}_SRGB")
                } else {
                    format!("ASTC_LDR_{bw}x{bh}_UNORM")
                }
            }
            Self::Unknown(id) => format!("UNKNOWN (0x{id:X})"),
        }
    }

    pub fn from_xt1_code(code: u32) -> Self {
        match code {
            0x01 => Self::AfL8Unorm,
            0x0D => Self::Rg8Unorm,
            0x1F => Self::Bgra8,
            0x25 | 0x6D => Self::Rgba8,
            0x38 => Self::Rgba8Srgb,
            0x40 => Self::Rgba16Float,
            0x42 => Self::Bc1,
            0x43 => Self::Bc2,
            0x44 => Self::Bc3,
            0x45 | 0x49 => Self::Bc4,
            0x46 => Self::Bc1Srgb,
            0x47 => Self::Bc2Srgb,
            0x48 => Self::Bc3Srgb,
            0x4A | 0x4B => Self::Bc5,
            0x4D | 0x51 => Self::Bc7,
            0x4E | 0x52 => Self::Bc7Srgb,
            0x50 => Self::Bc6hUf16,
            0x79..=0x86 => {
                let (bw, bh, _) = ASTC_FOOTPRINTS[(code - 0x79) as usize];
                Self::Astc(bw, bh, false)
            }
            0x87..=0x94 => {
                let (bw, bh, _) = ASTC_FOOTPRINTS[(code - 0x87) as usize];
                Self::Astc(bw, bh, true)
            }
            other => Self::Unknown(other),
        }
    }

    pub fn to_xt1_code(self) -> u32 {
        match self {
            Self::AfL8Unorm | Self::R8Unorm => 0x01,
            Self::Rg8Unorm => 0x0D,
            Self::Bgra8 | Self::Bgra8Srgb => 0x1F,
            Self::Rgba8 => 0x25,
            Self::Rgba8Srgb => 0x38,
            Self::Rgba16Float => 0x40,
            Self::Bc1 => 0x42,
            Self::Bc2 => 0x43,
            Self::Bc3 => 0x44,
            Self::Bc4 => 0x45,
            Self::Bc1Srgb => 0x46,
            Self::Bc2Srgb => 0x47,
            Self::Bc3Srgb => 0x48,
            Self::Bc5 => 0x4A,
            Self::Bc7 => 0x4D,
            Self::Bc7Srgb => 0x4E,
            Self::Bc6hUf16 | Self::Bc6hSf16 => 0x50,
            Self::Astc(bw, bh, srgb) => {
                let idx = ASTC_FOOTPRINTS
                    .iter()
                    .position(|&(w, h, _)| w == bw && h == bh)
                    .unwrap_or(0) as u32;
                if srgb { 0x87 + idx } else { 0x79 + idx }
            }
            Self::Unknown(v) => v,
        }
    }

    pub fn from_bntx_surface_format(fmt: u32) -> Self {
        match fmt {
            0x0201 => Self::R8Unorm,
            0x0901 | 0x0902 => Self::Rg8Unorm,
            0x0B01 | 0x0B02 => Self::Rgba8,
            0x0B06 => Self::Rgba8Srgb,
            0x0C01 => Self::Bgra8,
            0x0C06 => Self::Bgra8Srgb,
            0x1405 => Self::Rgba16Float,
            0x1A01 => Self::Bc1,
            0x1A06 => Self::Bc1Srgb,
            0x1B01 => Self::Bc2,
            0x1B06 => Self::Bc2Srgb,
            0x1C01 => Self::Bc3,
            0x1C06 => Self::Bc3Srgb,
            0x1D01 | 0x1D02 => Self::Bc4,
            0x1E01 | 0x1E02 => Self::Bc5,
            0x1F05 => Self::Bc6hSf16,
            0x1F0A => Self::Bc6hUf16,
            0x2001 => Self::Bc7,
            0x2006 => Self::Bc7Srgb,
            0x2D01..=0x3A06 => {
                let type_id = (fmt >> 8) as usize;
                let srgb = (fmt & 0xFF) == 0x06;
                if (0x2D..=0x3A).contains(&type_id) {
                    let (bw, bh, _) = ASTC_FOOTPRINTS[type_id - 0x2D];
                    Self::Astc(bw, bh, srgb)
                } else {
                    Self::Unknown(fmt)
                }
            }
            other => Self::Unknown(other),
        }
    }

    pub fn to_bntx_surface_format(self) -> u32 {
        match self {
            Self::R8Unorm | Self::AfL8Unorm => 0x0201,
            Self::Rg8Unorm => 0x0901,
            Self::Rgba8 => 0x0B01,
            Self::Rgba8Srgb => 0x0B06,
            Self::Bgra8 => 0x0C01,
            Self::Bgra8Srgb => 0x0C06,
            Self::Rgba16Float => 0x1405,
            Self::Bc1 => 0x1A01,
            Self::Bc1Srgb => 0x1A06,
            Self::Bc2 => 0x1B01,
            Self::Bc2Srgb => 0x1B06,
            Self::Bc3 => 0x1C01,
            Self::Bc3Srgb => 0x1C06,
            Self::Bc4 => 0x1D01,
            Self::Bc5 => 0x1E01,
            Self::Bc6hSf16 => 0x1F05,
            Self::Bc6hUf16 => 0x1F0A,
            Self::Bc7 => 0x2001,
            Self::Bc7Srgb => 0x2006,
            Self::Astc(bw, bh, srgb) => {
                let idx = ASTC_FOOTPRINTS
                    .iter()
                    .position(|&(w, h, _)| w == bw && h == bh)
                    .unwrap_or(0) as u32;
                let type_id = 0x2D + idx;
                (type_id << 8) | if srgb { 0x06 } else { 0x01 }
            }
            Self::Unknown(v) => v,
        }
    }

    pub fn to_dxgi_format(self) -> Option<u32> {
        match self {
            Self::Rgba16Float => Some(10),
            Self::Rgba8Srgb => Some(29),
            Self::Rg8Unorm => Some(49),
            Self::R8Unorm | Self::AfL8Unorm => Some(61),
            Self::Bc1Srgb => Some(72),
            Self::Bc2Srgb => Some(75),
            Self::Bc3Srgb => Some(78),
            Self::Bgra8Srgb => Some(91),
            Self::Bc6hUf16 => Some(95),
            Self::Bc6hSf16 => Some(96),
            Self::Bc7 => Some(98),
            Self::Bc7Srgb => Some(99),
            Self::Astc(bw, bh, srgb) => {
                let idx = ASTC_FOOTPRINTS
                    .iter()
                    .position(|&(w, h, _)| w == bw && h == bh)
                    .unwrap_or(0) as u32;
                // Standard DXGI_FORMAT_ASTC_4X4_UNORM = 134, SRGB = 135, stepping by 4 per footprint
                Some(134 + idx * 4 + if srgb { 1 } else { 0 })
            }
            _ => None,
        }
    }

    pub fn from_dxgi_format(dxgi: u32) -> Self {
        match dxgi {
            10 => Self::Rgba16Float,
            28 => Self::Rgba8,
            29 => Self::Rgba8Srgb,
            49 => Self::Rg8Unorm,
            61 => Self::R8Unorm,
            70 | 71 => Self::Bc1,
            72 => Self::Bc1Srgb,
            73 | 74 => Self::Bc2,
            75 => Self::Bc2Srgb,
            76 | 77 => Self::Bc3,
            78 => Self::Bc3Srgb,
            79 | 80 | 81 => Self::Bc4,
            82 | 83 | 84 => Self::Bc5,
            87 => Self::Bgra8,
            91 => Self::Bgra8Srgb,
            94 | 95 => Self::Bc6hUf16,
            96 => Self::Bc6hSf16,
            97 | 98 => Self::Bc7,
            99 => Self::Bc7Srgb,
            134..=189 => {
                let rel = dxgi - 134;
                let fp_idx = (rel / 4) as usize;
                let srgb = (rel % 4) == 1;
                if fp_idx < ASTC_FOOTPRINTS.len() {
                    let (bw, bh, _) = ASTC_FOOTPRINTS[fp_idx];
                    Self::Astc(bw, bh, srgb)
                } else {
                    Self::Astc(4, 4, srgb)
                }
            }
            _ => Self::Rgba8,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum TextureDimension {
    #[default]
    Tex2D,
    TexAnimatedLayers,
    TexCube,
}

impl TextureDimension {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Tex2D => "2D",
            Self::TexAnimatedLayers => "Animated 2D",
            Self::TexCube => "Cube",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum EncodeQuality {
    Fastest = 0,
    #[default]
    Normal = 1,
    Production = 2,
    Highest = 3,
}

impl EncodeQuality {
    pub fn from_str_name(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "fast" | "fastest" | "0" => Self::Fastest,
            "production" | "prod" | "2" => Self::Production,
            "highest" | "high" | "best" | "3" => Self::Highest,
            _ => Self::Normal,
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Fastest => "Fast",
            Self::Normal => "Normal",
            Self::Production => "Production",
            Self::Highest => "Highest",
        }
    }
}

fn default_layer_count() -> u32 {
    1
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TextureMetadata {
    pub container_ext: String,
    pub format: TextureFormat,
    #[serde(default)]
    pub dimension: TextureDimension,
    pub width: u32,
    pub height: u32,
    pub depth: u32,
    #[serde(default = "default_layer_count")]
    pub layer_count: u32,
    pub mip_count: u32,
    pub data_size: usize,
    pub name: Option<String>,
}

// ============================================================================
// TEGRA X1 GOB SWIZZLE & XT1 SURFACE LAYOUT (Ported 1:1 from XT1 Studio)
// ============================================================================

pub mod gob_swizzle {
    pub const GOB_WIDTH_BYTES: usize = 64;
    pub const GOB_HEIGHT_BYTES: usize = 8;
    pub const GOB_SIZE_BYTES: usize = GOB_WIDTH_BYTES * GOB_HEIGHT_BYTES;

    #[inline]
    pub fn block_depth(depth: usize) -> usize {
        let d_half = depth + depth / 2;
        if d_half >= 16 {
            16
        } else if d_half >= 8 {
            8
        } else if d_half >= 4 {
            4
        } else if d_half >= 2 {
            2
        } else {
            1
        }
    }

    #[inline]
    pub fn mip_block_depth(mip_depth: usize, mut gob_depth: usize) -> usize {
        while mip_depth <= gob_depth / 2 && gob_depth > 1 {
            gob_depth /= 2;
        }
        gob_depth
    }

    #[inline]
    pub fn block_height_mip0(height_blocks: usize) -> usize {
        let h_half = height_blocks + height_blocks / 2;
        if h_half >= 128 {
            16
        } else if h_half >= 64 {
            8
        } else if h_half >= 32 {
            4
        } else if h_half >= 16 {
            2
        } else {
            1
        }
    }

    #[inline]
    pub fn block_height_log2(height_blocks: usize) -> u32 {
        match block_height_mip0(height_blocks) {
            16 => 4,
            8 => 3,
            4 => 2,
            2 => 1,
            _ => 0,
        }
    }

    #[inline]
    pub fn mip_block_height(height_blocks: usize, block_height_mip0: usize) -> usize {
        let mut bh = block_height_mip0.max(1);
        while height_blocks <= (bh / 2) * GOB_HEIGHT_BYTES && bh > 1 {
            bh /= 2;
        }
        bh
    }

    #[inline]
    fn gob_offset(x: usize, y: usize) -> usize {
        (x % 64) / 32 * 256
            + (y % 8) / 2 * 64
            + (x % 32) / 16 * 32
            + (y % 2) * 16
            + (x % 16)
    }

    #[inline]
    fn gob_address_x(x_bytes: usize, block_size_bytes: usize) -> usize {
        (x_bytes / GOB_WIDTH_BYTES) * block_size_bytes
    }

    #[inline]
    fn gob_address_y(
        y: usize,
        block_height_bytes: usize,
        block_size_bytes: usize,
        image_width_in_gobs: usize,
    ) -> usize {
        (y / block_height_bytes) * block_size_bytes * image_width_in_gobs
            + ((y % block_height_bytes) / GOB_HEIGHT_BYTES) * GOB_SIZE_BYTES
    }

    #[inline]
    pub fn pitch_in_gobs(width_blocks: usize, bytes_per_block: usize, pitch_bytes: usize) -> usize {
        let stride = if pitch_bytes > 0 {
            pitch_bytes
        } else {
            width_blocks * bytes_per_block
        };
        (stride + GOB_WIDTH_BYTES - 1) / GOB_WIDTH_BYTES
    }

    #[inline]
    fn slice_size(
        width_in_gobs: usize,
        height_blocks: usize,
        block_height: usize,
        block_depth: usize,
    ) -> usize {
        let rob_size = GOB_SIZE_BYTES * block_height * block_depth * width_in_gobs;
        let bh_rows = block_height * GOB_HEIGHT_BYTES;
        ((height_blocks + bh_rows - 1) / bh_rows) * rob_size
    }

    #[inline]
    fn gob_address_z(z: usize, block_height: usize, block_depth: usize, slice_sz: usize) -> usize {
        (z / block_depth) * slice_sz + (z & (block_depth - 1)) * GOB_SIZE_BYTES * block_height
    }

    pub fn deswizzle_volume(
        source: &[u8],
        destination: &mut [u8],
        width_blocks: usize,
        height_blocks: usize,
        depth_blocks: usize,
        bytes_per_block: usize,
        block_height: usize,
        block_depth: usize,
        pitch_bytes: usize,
    ) {
        let bh = block_height.max(1);
        let bd = block_depth.max(1);
        let width_in_gobs = pitch_in_gobs(width_blocks, bytes_per_block, pitch_bytes);
        let sl_size = slice_size(width_in_gobs, height_blocks, bh, bd);
        let block_size_bytes = GOB_SIZE_BYTES * bh * bd;
        let block_height_bytes = GOB_HEIGHT_BYTES * bh;
        let row_bytes = width_blocks * bytes_per_block;

        for z in 0..depth_blocks {
            let offset_z = gob_address_z(z, bh, bd, sl_size);
            for y0 in (0..height_blocks).step_by(GOB_HEIGHT_BYTES) {
                let offset_y =
                    gob_address_y(y0, block_height_bytes, block_size_bytes, width_in_gobs);
                for x0 in (0..row_bytes).step_by(GOB_WIDTH_BYTES) {
                    let gob_addr = offset_z + offset_y + gob_address_x(x0, block_size_bytes);
                    let x_limit = GOB_WIDTH_BYTES.min(row_bytes - x0);
                    let y_limit = GOB_HEIGHT_BYTES.min(height_blocks - y0);
                    for y in 0..y_limit {
                        let linear_row = (z * height_blocks + y0 + y) * row_bytes + x0;
                        for x in 0..x_limit {
                            let src_idx = gob_addr + gob_offset(x, y);
                            let dst_idx = linear_row + x;
                            if src_idx < source.len() && dst_idx < destination.len() {
                                destination[dst_idx] = source[src_idx];
                            }
                        }
                    }
                }
            }
        }
    }

    pub fn swizzle_volume(
        source: &[u8],
        destination: &mut [u8],
        width_blocks: usize,
        height_blocks: usize,
        depth_blocks: usize,
        bytes_per_block: usize,
        block_height: usize,
        block_depth: usize,
        pitch_bytes: usize,
    ) {
        let bh = block_height.max(1);
        let bd = block_depth.max(1);
        let width_in_gobs = pitch_in_gobs(width_blocks, bytes_per_block, pitch_bytes);
        let sl_size = slice_size(width_in_gobs, height_blocks, bh, bd);
        let block_size_bytes = GOB_SIZE_BYTES * bh * bd;
        let block_height_bytes = GOB_HEIGHT_BYTES * bh;
        let row_bytes = width_blocks * bytes_per_block;

        for z in 0..depth_blocks {
            let offset_z = gob_address_z(z, bh, bd, sl_size);
            for y0 in (0..height_blocks).step_by(GOB_HEIGHT_BYTES) {
                let offset_y =
                    gob_address_y(y0, block_height_bytes, block_size_bytes, width_in_gobs);
                for x0 in (0..row_bytes).step_by(GOB_WIDTH_BYTES) {
                    let gob_addr = offset_z + offset_y + gob_address_x(x0, block_size_bytes);
                    let x_limit = GOB_WIDTH_BYTES.min(row_bytes - x0);
                    let y_limit = GOB_HEIGHT_BYTES.min(height_blocks - y0);
                    for y in 0..y_limit {
                        let linear_row = (z * height_blocks + y0 + y) * row_bytes + x0;
                        for x in 0..x_limit {
                            let dst_idx = gob_addr + gob_offset(x, y);
                            let src_idx = linear_row + x;
                            if src_idx < source.len() && dst_idx < destination.len() {
                                destination[dst_idx] = source[src_idx];
                            }
                        }
                    }
                }
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Xt1MipInfo {
    pub index: u32,
    pub width: u32,
    pub height: u32,
    pub depth: u32,
    pub width_blocks: usize,
    pub height_blocks: usize,
    pub block_height: usize,
    pub block_depth: usize,
    pub lines: usize,
    pub pitch: usize,
    pub slice_surface_size: usize,
    pub slice_count: usize,
    pub offset: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Xt1LayoutResult {
    pub format: TextureFormat,
    pub tex_type: u32,
    pub computed_image_size: usize,
    pub stored_image_size: u64,
    pub size_matches: bool,
    pub mips: Vec<Xt1MipInfo>,
    pub face_count: usize,
    pub face_size: usize,
    pub face_stride: usize,
    pub note: Option<String>,
}

/// Compute the exact `XT1` surface mip layout matching `SurfaceLayout.cs` in `XT1 Studio`.
pub fn compute_xt1_layout(
    width: u32,
    height: u32,
    depth: u32,
    mip_count: u32,
    tex_type: u32,
    layout: u32,
    format: TextureFormat,
    stored_image_size: u64,
) -> Xt1LayoutResult {
    let mips = mip_count.clamp(1, 32);
    let blk_log2 = (layout & 7) as usize;
    let (bw, bh) = format.block_dimensions();
    let bpb = format.bytes_per_block();
    let bpp_log2 = bpb.ilog2() as usize;
    let is_3d = tex_type == 2;
    let is_cube = tex_type == 3 || tex_type == 8;
    let base_depth = depth.max(1) as usize;
    let block_depth_log2 = ((layout >> 4) & 7) as usize;
    let gob_depth = if is_3d {
        if block_depth_log2 > 0 {
            1usize << block_depth_log2
        } else {
            gob_swizzle::block_depth(base_depth)
        }
    } else {
        1
    };

    let w0_blocks = ((width.max(1) as usize) + bw - 1) / bw;
    let wide_pitch = (layout & 0x400) != 0 && blk_log2 == 3 && (w0_blocks * bpb) % 1024 != 0;

    let build_mips = |pitch_override: Option<usize>| -> (Vec<Xt1MipInfo>, usize) {
        let bh_mip0 = if is_3d { 1 } else { (1usize << blk_log2).max(1) };
        let mut cursor = 0usize;
        let mut list = Vec::with_capacity(mips as usize);

        for m in 0..mips {
            let w = (width >> m).max(1);
            let h = (height >> m).max(1);
            let d = if is_3d { (base_depth >> m).max(1) } else { 1 };
            let wb = ((w as usize) + bw - 1) / bw;
            let hb = ((h as usize) + bh - 1) / bh;
            let lines = 8 * gob_swizzle::mip_block_height(hb, bh_mip0);

            let width_bytes = wb * bpb;
            let is_small = width_bytes <= (1usize << (10usize.saturating_sub(bpp_log2)))
                || hb <= (1usize << (3 + blk_log2))
                || (is_3d && d < (1usize << block_depth_log2));
            let used_wide = wide_pitch && !is_small;
            let pitch = pitch_override.unwrap_or_else(|| {
                if used_wide {
                    align(width_bytes, 1024)
                } else {
                    align(width_bytes, 64)
                }
            });
            let surface = pitch * align(hb, lines);
            let offset = align(cursor, 512);

            list.push(Xt1MipInfo {
                index: m,
                width: w,
                height: h,
                depth: d as u32,
                width_blocks: wb,
                height_blocks: hb,
                block_height: (lines / 8).max(1),
                block_depth: if is_3d {
                    gob_swizzle::mip_block_depth(d, gob_depth)
                } else {
                    1
                },
                lines,
                pitch,
                slice_surface_size: surface,
                slice_count: d,
                offset,
            });
            cursor = offset + surface * d;
        }
        (list, cursor)
    };

    let (mut mip_list, mut cursor) = build_mips(None);
    let face_count = if is_cube { 6 } else { 1 };
    let face_size = cursor;
    let base_mip_surface = mip_list.first().map(|m| m.slice_surface_size).unwrap_or(0);
    let cube_page_align = is_cube && base_mip_surface >= 4096;
    let face_stride = if cube_page_align {
        align(face_size, 4096)
    } else {
        face_size
    };
    let mut computed = if is_cube {
        face_count * face_stride
    } else {
        cursor
    };

    let mut note = None;
    if computed as u64 != stored_image_size && mips == 1 && !is_cube && mip_list.len() == 1 {
        let mip = &mip_list[0];
        let padded_rows = align(mip.height_blocks, mip.lines);
        let units = padded_rows * mip.slice_count;
        let needed = mip.width_blocks * bpb;
        if units > 0 && stored_image_size % (units as u64) == 0 {
            let candidate = (stored_image_size / (units as u64)) as usize;
            if candidate != mip.pitch && candidate >= needed && candidate % 64 == 0 {
                let (new_list, new_cur) = build_mips(Some(candidate));
                mip_list = new_list;
                cursor = new_cur;
                computed = cursor;
                if computed as u64 == stored_image_size {
                    note = Some(format!(
                        "Row pitch recovered from stored imageSize ({} bytes)",
                        mip_list[0].pitch
                    ));
                }
            }
        }
    }

    let size_matches = computed as u64 == stored_image_size;
    if !size_matches {
        note = Some(format!(
            "stored imageSize {stored_image_size} != computed {computed}"
        ));
    }

    Xt1LayoutResult {
        format,
        tex_type,
        computed_image_size: computed,
        stored_image_size,
        size_matches,
        mips: mip_list,
        face_count,
        face_size,
        face_stride,
        note,
    }
}

// ============================================================================
// WII U GX2 ADDRLIB TILING & SWIZZLE (Ported from Switch-Toolbox GX2.cs)
// ============================================================================

pub mod gx2_addrlib {
    use super::TextureFormat;

    const BANK_SWAP_ORDER: [u32; 10] = [0, 1, 3, 2, 6, 7, 5, 4, 0, 0];

    pub fn is_bcn(fmt: u32) -> bool {
        matches!(fmt & 0x3F, 0x31..=0x35)
    }

    pub fn bits_per_element(fmt: u32) -> u32 {
        match fmt & 0x3F {
            0x01 | 0x02 => 8,
            0x05 | 0x06 | 0x07 | 0x08 | 0x0A | 0x0B | 0x0C => 16,
            0x0D | 0x0E | 0x0F | 0x10 | 0x11 | 0x16 | 0x19 | 0x1A | 0x1B => 32,
            0x1D | 0x1E | 0x1F | 0x20 | 0x31 | 0x34 => 64,
            0x22 | 0x23 | 0x32 | 0x33 | 0x35 => 128,
            _ => 32,
        }
    }

    pub fn gx2_format_to_texture_format(gx2_fmt: u32) -> TextureFormat {
        let srgb = (gx2_fmt & 0x400) != 0;
        match gx2_fmt & 0x3F {
            0x01 => TextureFormat::R8Unorm,
            0x07 => TextureFormat::Rg8Unorm,
            0x1A => {
                if srgb {
                    TextureFormat::Rgba8Srgb
                } else {
                    TextureFormat::Rgba8
                }
            }
            0x20 => TextureFormat::Rgba16Float,
            0x31 => {
                if srgb {
                    TextureFormat::Bc1Srgb
                } else {
                    TextureFormat::Bc1
                }
            }
            0x32 => {
                if srgb {
                    TextureFormat::Bc2Srgb
                } else {
                    TextureFormat::Bc2
                }
            }
            0x33 => {
                if srgb {
                    TextureFormat::Bc3Srgb
                } else {
                    TextureFormat::Bc3
                }
            }
            0x34 => TextureFormat::Bc4,
            0x35 => TextureFormat::Bc5,
            other => TextureFormat::Unknown(other),
        }
    }

    fn compute_surface_thickness(tile_mode: u32) -> u32 {
        match tile_mode {
            3 | 7 | 11 | 13 | 15 => 4,
            16 | 17 => 8,
            _ => 1,
        }
    }

    fn is_thick_macro_tiled(tile_mode: u32) -> bool {
        matches!(tile_mode, 7 | 11 | 13 | 15)
    }

    fn is_bank_swapped_tile_mode(tile_mode: u32) -> bool {
        matches!(tile_mode, 8 | 9 | 10 | 11 | 14 | 15)
    }

    fn compute_macro_tile_aspect_ratio(tile_mode: u32) -> u32 {
        match tile_mode {
            5 | 9 => 2,
            6 | 10 => 4,
            _ => 1,
        }
    }

    fn compute_surface_rotation_from_tile_mode(tile_mode: u32) -> u32 {
        match tile_mode {
            4..=11 => 2,
            12..=15 => 1,
            _ => 0,
        }
    }

    fn compute_surface_bank_swapped_width(
        tile_mode: u32,
        bpp: u32,
        mut num_samples: u32,
        pitch: u32,
    ) -> u32 {
        if !is_bank_swapped_tile_mode(tile_mode) {
            return 0;
        }
        let bytes_per_sample = 8 * bpp;
        let slices_per_tile = if bytes_per_sample != 0 {
            let samples_per_tile = 2048 / bytes_per_sample;
            (num_samples / samples_per_tile.max(1)).max(1)
        } else {
            1
        };
        if is_thick_macro_tiled(tile_mode) {
            num_samples = 4;
        }
        let bytes_per_tile_slice = num_samples * bytes_per_sample / slices_per_tile;
        let factor = compute_macro_tile_aspect_ratio(tile_mode);
        let swap_tiles = (128 / bpp.max(1)).max(1);
        let swap_width = swap_tiles * 32;
        let height_bytes = (num_samples * factor * bpp * 2 / slices_per_tile).max(1);
        let swap_max = 0x4000 / height_bytes;
        let swap_min = 256 / bytes_per_tile_slice.max(1);
        let mut bank_swap_width = swap_max.min(swap_min.max(swap_width));
        while bank_swap_width >= 2 * pitch && bank_swap_width > 1 {
            bank_swap_width >>= 1;
        }
        bank_swap_width
    }

    fn compute_pixel_index_within_micro_tile(
        x: u32,
        y: u32,
        z: u32,
        bpp: u32,
        tile_mode: u32,
        is_depth: bool,
    ) -> u32 {
        let thickness = compute_surface_thickness(tile_mode);
        let (pb0, pb1, pb2, pb3, pb4, pb5) = if is_depth {
            (
                x & 1,
                y & 1,
                (x & 2) >> 1,
                (y & 2) >> 1,
                (x & 4) >> 2,
                (y & 4) >> 2,
            )
        } else {
            match bpp {
                8 => (
                    x & 1,
                    (x & 2) >> 1,
                    (x & 4) >> 2,
                    (y & 2) >> 1,
                    y & 1,
                    (y & 4) >> 2,
                ),
                0x10 => (
                    x & 1,
                    (x & 2) >> 1,
                    (x & 4) >> 2,
                    y & 1,
                    (y & 2) >> 1,
                    (y & 4) >> 2,
                ),
                0x20 | 0x60 => (
                    x & 1,
                    (x & 2) >> 1,
                    y & 1,
                    (x & 4) >> 2,
                    (y & 2) >> 1,
                    (y & 4) >> 2,
                ),
                0x40 => (
                    x & 1,
                    y & 1,
                    (x & 2) >> 1,
                    (x & 4) >> 2,
                    (y & 2) >> 1,
                    (y & 4) >> 2,
                ),
                0x80 => (
                    y & 1,
                    x & 1,
                    (x & 2) >> 1,
                    (x & 4) >> 2,
                    (y & 2) >> 1,
                    (y & 4) >> 2,
                ),
                _ => (
                    x & 1,
                    (x & 2) >> 1,
                    y & 1,
                    (x & 4) >> 2,
                    (y & 2) >> 1,
                    (y & 4) >> 2,
                ),
            }
        };
        let pb6 = if thickness > 1 { z & 1 } else { 0 };
        let pb7 = if thickness > 1 { (z & 2) >> 1 } else { 0 };
        let pb8 = if thickness == 8 { (z & 4) >> 2 } else { 0 };
        (pb8 << 8)
            | (pb7 << 7)
            | (pb6 << 6)
            | (32 * pb5)
            | (16 * pb4)
            | (8 * pb3)
            | (4 * pb2)
            | (2 * pb1)
            | pb0
    }

    fn compute_addr_from_coord_micro_tiled(
        x: u32,
        y: u32,
        slice: u32,
        bpp: u32,
        pitch: u32,
        height: u32,
        tile_mode: u32,
        is_depth: bool,
    ) -> usize {
        let micro_tile_thickness = if tile_mode == 3 { 4 } else { 1 };
        let micro_tile_bytes = (64 * micro_tile_thickness * bpp + 7) / 8;
        let micro_tiles_per_row = pitch >> 3;
        let mtx = x >> 3;
        let mty = y >> 3;
        let mtz = slice / micro_tile_thickness;
        let micro_tile_offset =
            (micro_tile_bytes as usize) * ((mtx + mty * micro_tiles_per_row) as usize);
        let slice_bytes =
            ((pitch as usize) * (height as usize) * (micro_tile_thickness as usize) * (bpp as usize)
                + 7)
                / 8;
        let slice_offset = (mtz as usize) * slice_bytes;
        let pixel_index =
            compute_pixel_index_within_micro_tile(x, y, slice, bpp, tile_mode, is_depth);
        let pixel_offset = ((bpp * pixel_index) >> 3) as usize;
        pixel_offset + micro_tile_offset + slice_offset
    }

    #[allow(clippy::too_many_arguments)]
    fn compute_addr_from_coord_macro_tiled(
        x: u32,
        y: u32,
        slice: u32,
        sample: u32,
        bpp: u32,
        pitch: u32,
        height: u32,
        mut num_samples: u32,
        tile_mode: u32,
        is_depth: bool,
        pipe_swizzle: u32,
        bank_swizzle: u32,
    ) -> usize {
        let micro_tile_thickness = compute_surface_thickness(tile_mode);
        let micro_tile_bits = num_samples * bpp * (micro_tile_thickness * 64);
        let micro_tile_bytes = (micro_tile_bits + 7) / 8;
        let pixel_index =
            compute_pixel_index_within_micro_tile(x, y, slice, bpp, tile_mode, is_depth);
        let bytes_per_sample = micro_tile_bytes / num_samples.max(1);

        let (sample_offset, pixel_offset) = if is_depth {
            (bpp * sample, num_samples * bpp * pixel_index)
        } else {
            (
                sample * (micro_tile_bits / num_samples.max(1)),
                bpp * pixel_index,
            )
        };
        let mut elem_offset = pixel_offset + sample_offset;
        let (num_sample_splits, sample_slice) = if num_samples <= 1 || micro_tile_bytes <= 2048 {
            (1, 0)
        } else {
            let samples_per_slice = (2048 / bytes_per_sample.max(1)).max(1);
            let splits = num_samples / samples_per_slice;
            num_samples = samples_per_slice;
            let tile_slice_bits = micro_tile_bits / splits.max(1);
            let ss = elem_offset / tile_slice_bits.max(1);
            elem_offset %= tile_slice_bits.max(1);
            (splits, ss)
        };
        elem_offset = (elem_offset + 7) / 8;

        let mut pipe = ((y >> 3) ^ (x >> 3)) & 1;
        let mut bank = (((y >> 5) ^ (x >> 3)) & 1) | (2 * (((y >> 4) ^ (x >> 4)) & 1));
        let swizzle_bits = pipe_swizzle + 2 * bank_swizzle;
        let mut bank_pipe = pipe + 2 * bank;
        let rotation = compute_surface_rotation_from_tile_mode(tile_mode);
        let mut slice_in = slice;
        if is_thick_macro_tiled(tile_mode) {
            slice_in >>= 2;
        }
        bank_pipe ^= (2 * sample_slice * 3) ^ (swizzle_bits + slice_in * rotation);
        bank_pipe %= 8;
        pipe = bank_pipe % 2;
        bank = bank_pipe / 2;

        let slice_bytes = ((height as u64)
            * (pitch as u64)
            * (micro_tile_thickness as u64)
            * (bpp as u64)
            * (num_samples as u64)
            + 7)
            / 8;
        let slice_offset = slice_bytes
            * ((sample_slice + num_sample_splits * slice) as u64)
            / (micro_tile_thickness as u64);

        let (macro_tile_pitch, macro_tile_height) = match tile_mode {
            5 | 9 => (16u32, 32u32),
            6 | 10 => (8u32, 64u32),
            _ => (32u32, 16u32),
        };

        let macro_tiles_per_row = (pitch / macro_tile_pitch).max(1);
        let macro_tile_bytes = ((num_samples as u64)
            * (micro_tile_thickness as u64)
            * (bpp as u64)
            * (macro_tile_height as u64)
            * (macro_tile_pitch as u64)
            + 7)
            / 8;
        let mtx = x / macro_tile_pitch;
        let mty = y / macro_tile_height;
        let macro_tile_offset =
            ((mtx + macro_tiles_per_row * mty) as u64) * macro_tile_bytes;

        if is_bank_swapped_tile_mode(tile_mode) {
            let bank_swap_w = compute_surface_bank_swapped_width(tile_mode, bpp, 1, pitch).max(1);
            let swap_idx = (macro_tile_pitch * mtx / bank_swap_w) as usize;
            bank ^= BANK_SWAP_ORDER[swap_idx & 3];
        }

        let total_offset = (elem_offset as u64) + ((macro_tile_offset + slice_offset) >> 3);
        (((bank as u64) << 9)
            | ((pipe as u64) << 8)
            | (total_offset & 255)
            | ((total_offset & !255u64) << 3)) as usize
    }

    #[allow(clippy::too_many_arguments)]
    pub fn deswizzle_gx2_surface(
        width: u32,
        height: u32,
        depth: u32,
        gx2_format: u32,
        aa: u32,
        use_flags: u32,
        tile_mode: u32,
        swizzle_val: u32,
        pitch: u32,
        slice: u32,
        src: &[u8],
    ) -> Vec<u8> {
        let bpp = bits_per_element(gx2_format);
        let bytes_per_elem = (bpp / 8) as usize;
        let (w_elems, h_elems) = if is_bcn(gx2_format) {
            ((width + 3) / 4, (height + 3) / 4)
        } else {
            (width.max(1), height.max(1))
        };

        let pipe_swizzle = (swizzle_val >> 8) & 1;
        let bank_swizzle = (swizzle_val >> 9) & 3;
        let addr_tile_mode = if tile_mode == 16 { 0 } else { tile_mode };
        let is_depth = (use_flags & 4) != 0;
        let num_samples = 1u32 << aa;

        let aligned_pitch = pitch.max(w_elems);
        let aligned_height = if addr_tile_mode >= 4 {
            let (_, mth) = match addr_tile_mode {
                5 | 9 => (16, 32),
                6 | 10 => (8, 64),
                _ => (32, 16),
            };
            (h_elems + mth - 1) & !(mth - 1)
        } else if addr_tile_mode >= 2 {
            (h_elems + 7) & !7
        } else {
            h_elems
        };

        let mut out = vec![0u8; (w_elems as usize) * (h_elems as usize) * bytes_per_elem];
        for y in 0..h_elems {
            for x in 0..w_elems {
                let src_pos = match addr_tile_mode {
                    0 | 1 => {
                        let slice_off = aligned_pitch * aligned_height * slice;
                        ((y * aligned_pitch + x + slice_off) as usize) * bytes_per_elem
                    }
                    2 | 3 => compute_addr_from_coord_micro_tiled(
                        x,
                        y,
                        slice,
                        bpp,
                        aligned_pitch,
                        aligned_height,
                        addr_tile_mode,
                        is_depth,
                    ),
                    _ => compute_addr_from_coord_macro_tiled(
                        x,
                        y,
                        slice,
                        0,
                        bpp,
                        aligned_pitch,
                        aligned_height,
                        num_samples,
                        addr_tile_mode,
                        is_depth,
                        pipe_swizzle,
                        bank_swizzle,
                    ),
                };
                let dst_pos = ((y * w_elems + x) as usize) * bytes_per_elem;
                if src_pos + bytes_per_elem <= src.len() && dst_pos + bytes_per_elem <= out.len() {
                    out[dst_pos..dst_pos + bytes_per_elem]
                        .copy_from_slice(&src[src_pos..src_pos + bytes_per_elem]);
                }
            }
        }
        let _ = depth;
        out
    }
}

// ============================================================================
// NINTENDO SWITCH BNTX CONTAINER PARSER (Ported from Switch-Toolbox BNTX.cs)
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BntxTextureRecord {
    #[serde(default)]
    pub brti_offset: usize,
    pub name: String,
    pub flags: u8,
    pub dim: u8,
    pub tile_mode: u16,
    pub swizzle: u16,
    pub mip_count: u16,
    pub sample_count: u32,
    pub surface_format: u32,
    pub format: TextureFormat,
    pub access_flags: u32,
    pub width: u32,
    pub height: u32,
    pub depth: u32,
    pub array_length: u32,
    pub texture_layout: u32,
    pub texture_layout2: u32,
    pub image_size: u32,
    pub alignment: i32,
    pub channel_comps: [u8; 4],
    pub surface_dim: u32,
    pub block_height_log2: u32,
    pub mip_offsets: Vec<usize>,
}

/// Parse all `BRTI` texture headers inside a `.bntx` file (including Kill la Kill 80-byte prefixed BNTX).
pub fn parse_bntx_textures(data: &[u8]) -> Result<Vec<BntxTextureRecord>> {
    let base_off = if data.len() >= 0x28 && &data[0..4] == b"BNTX" {
        0usize
    } else if data.len() >= 80 + 0x28 && &data[80..84] == b"BNTX" {
        80usize
    } else {
        return Err(PgError::InvalidData("Missing BNTX magic".into()));
    };

    let bntx = &data[base_off..];
    let mut r = BinReader::new(bntx, Endian::Little);
    r.seek(0x0C)?;
    let bom = r.read_u16()?;
    if bom == 0xFFFE {
        r.endian = Endian::Big;
    }

    // Scan for `NX  ` target header (normally at 0x20) or scan directly for `BRTI` blocks
    let mut brti_offsets = Vec::new();
    if bntx.len() >= 0x48 && &bntx[0x20..0x24] == b"NX  " {
        r.seek(0x24)?;
        let tex_count = r.read_u32()? as usize;
        let tex_arr_ptr = r.read_u64()? as usize;
        if tex_count > 0 && tex_count <= 4096 && tex_arr_ptr + tex_count * 8 <= bntx.len() {
            r.seek(tex_arr_ptr)?;
            for _ in 0..tex_count {
                let ptr = r.read_u64()? as usize;
                if ptr + 0x70 <= bntx.len() && &bntx[ptr..ptr + 4] == b"BRTI" {
                    brti_offsets.push(ptr);
                }
            }
        }
    }

    if brti_offsets.is_empty() {
        for pos in 0..bntx.len().saturating_sub(0x70) {
            if &bntx[pos..pos + 4] == b"BRTI" {
                brti_offsets.push(pos);
            }
        }
    }

    let mut records = Vec::with_capacity(brti_offsets.len());
    for ptr in brti_offsets {
        r.seek(ptr + 16)?;
        let flags = r.read_u8()?;
        let dim = r.read_u8()?;
        let tile_mode = r.read_u16()?;
        let swizzle = r.read_u16()?;
        let mip_count = r.read_u16()?.max(1);
        let sample_count = r.read_u32()?;
        let surface_format = r.read_u32()?;
        let access_flags = r.read_u32()?;
        let width = r.read_u32()?;
        let height = r.read_u32()?;
        let depth = r.read_u32()?.max(1);
        let array_length = r.read_u32()?.max(1);
        let texture_layout = r.read_u32()?;
        let texture_layout2 = r.read_u32()?;
        let _reserved = r.read_bytes(20)?;
        let image_size = r.read_u32()?;
        if width == 0 || height == 0 || width > 16384 || height > 16384 || image_size == 0 {
            continue;
        }
        let alignment = r.read_i32()?;
        let comp_sel = r.read_u32()?;
        let channel_comps = [
            (comp_sel & 0xFF) as u8,
            ((comp_sel >> 8) & 0xFF) as u8,
            ((comp_sel >> 16) & 0xFF) as u8,
            ((comp_sel >> 24) & 0xFF) as u8,
        ];
        let surface_dim = r.read_u32()?;
        let name_ptr = r.read_u64()? as usize;
        let _parent_ptr = r.read_u64()?;
        let mip_ptrs_offset = r.read_u64()? as usize;

        let name = if name_ptr + 2 < bntx.len() {
            r.seek(name_ptr)?;
            let slen = r.read_u16()? as usize;
            if name_ptr + 2 + slen <= bntx.len() {
                String::from_utf8_lossy(&bntx[name_ptr + 2..name_ptr + 2 + slen]).into_owned()
            } else {
                format!("bntx_tex_{}", records.len())
            }
        } else {
            format!("bntx_tex_{}", records.len())
        };

        let mut mip_offsets = Vec::with_capacity(mip_count as usize);
        if mip_ptrs_offset + (mip_count as usize) * 8 <= bntx.len() {
            r.seek(mip_ptrs_offset)?;
            for _ in 0..mip_count {
                mip_offsets.push((r.read_u64()? as usize) + base_off);
            }
        }

        let format = TextureFormat::from_bntx_surface_format(surface_format);
        let (_, bh) = format.block_dimensions();
        let hb0 = ((height as usize) + bh - 1) / bh;
        let block_height_log2 = if (flags & 1) != 0 {
            texture_layout & 7
        } else {
            gob_swizzle::block_height_log2(hb0)
        };

        records.push(BntxTextureRecord {
            brti_offset: base_off + ptr,
            name,
            flags,
            dim,
            tile_mode,
            swizzle,
            mip_count,
            sample_count,
            surface_format,
            format,
            access_flags,
            width,
            height,
            depth,
            array_length,
            texture_layout,
            texture_layout2,
            image_size,
            alignment,
            channel_comps,
            surface_dim,
            block_height_log2,
            mip_offsets,
        });
    }

    if records.is_empty() {
        return Err(PgError::InvalidData("No valid BRTI textures found in BNTX".into()));
    }
    Ok(records)
}

// ============================================================================
// SOLIDIFY FILTER & MIPMAP GENERATOR (Ported from XT1 Studio Solidify.cs & MipChain.cs)
// ============================================================================

/// Multi-source BFS alpha edge-bleed ("Solidify" filter from `XT1 Studio`).
/// Fills RGB of transparent pixels (`alpha <= opaque_above`) from the nearest opaque neighbors
/// while leaving the alpha channel intact so bilinear filtering and block compression never produce dark fringes.
pub fn solidify_rgba_image(src: &RgbaImage, opaque_above: u8) -> RgbaImage {
    let mut out = src.clone();
    let w = out.width() as usize;
    let h = out.height() as usize;
    let count = w * h;
    if count == 0 {
        return out;
    }

    let raw = out.as_mut();
    let mut known = vec![false; count];
    let mut queue = VecDeque::with_capacity(count / 2);

    for p in 0..count {
        if raw[p * 4 + 3] > opaque_above {
            known[p] = true;
            queue.push_back(p);
        }
    }
    if queue.is_empty() || queue.len() == count {
        return out;
    }

    while let Some(p) = queue.pop_front() {
        let px = (p % w) as isize;
        let py = (p / w) as isize;
        for dy in -1..=1 {
            for dx in -1..=1 {
                if dx == 0 && dy == 0 {
                    continue;
                }
                let nx = px + dx;
                let ny = py + dy;
                if nx < 0 || ny < 0 || nx >= w as isize || ny >= h as isize {
                    continue;
                }
                let n = (ny as usize) * w + (nx as usize);
                if known[n] {
                    continue;
                }

                let mut r_sum = 0u32;
                let mut g_sum = 0u32;
                let mut b_sum = 0u32;
                let mut samples = 0u32;
                for ey in -1..=1 {
                    for ex in -1..=1 {
                        if ex == 0 && ey == 0 {
                            continue;
                        }
                        let ax = nx + ex;
                        let ay = ny + ey;
                        if ax < 0 || ay < 0 || ax >= w as isize || ay >= h as isize {
                            continue;
                        }
                        let q = (ay as usize) * w + (ax as usize);
                        if !known[q] {
                            continue;
                        }
                        r_sum += raw[q * 4] as u32;
                        g_sum += raw[q * 4 + 1] as u32;
                        b_sum += raw[q * 4 + 2] as u32;
                        samples += 1;
                    }
                }
                if samples == 0 {
                    continue;
                }
                raw[n * 4] = (r_sum / samples) as u8;
                raw[n * 4 + 1] = (g_sum / samples) as u8;
                raw[n * 4 + 2] = (b_sum / samples) as u8;
                known[n] = true;
                queue.push_back(n);
            }
        }
    }

    out
}

/// Generate a full box-filtered mipmap chain from `base_image`.
pub fn generate_mip_chain(base_image: &RgbaImage, max_mips: u32) -> Vec<RgbaImage> {
    let mut chain = Vec::new();
    let mut cur = base_image.clone();
    let total = max_mips.max(1);
    chain.push(cur.clone());

    for _ in 1..total {
        let w = cur.width();
        let h = cur.height();
        if w <= 1 && h <= 1 {
            break;
        }
        let nw = (w / 2).max(1);
        let nh = (h / 2).max(1);
        let mut next = RgbaImage::new(nw, nh);
        for y in 0..nh {
            for x in 0..nw {
                let x0 = (x * 2).min(w - 1);
                let x1 = (x * 2 + 1).min(w - 1);
                let y0 = (y * 2).min(h - 1);
                let y1 = (y * 2 + 1).min(h - 1);
                let p00 = cur.get_pixel(x0, y0).0;
                let p10 = cur.get_pixel(x1, y0).0;
                let p01 = cur.get_pixel(x0, y1).0;
                let p11 = cur.get_pixel(x1, y1).0;
                let avg = [
                    ((p00[0] as u16 + p10[0] as u16 + p01[0] as u16 + p11[0] as u16 + 2) / 4) as u8,
                    ((p00[1] as u16 + p10[1] as u16 + p01[1] as u16 + p11[1] as u16 + 2) / 4) as u8,
                    ((p00[2] as u16 + p10[2] as u16 + p01[2] as u16 + p11[2] as u16 + 2) / 4) as u8,
                    ((p00[3] as u16 + p10[3] as u16 + p01[3] as u16 + p11[3] as u16 + 2) / 4) as u8,
                ];
                next.put_pixel(x, y, image::Rgba(avg));
            }
        }
        chain.push(next.clone());
        cur = next;
    }

    chain
}

// ============================================================================
// NVTT 3.2.5 DLL DYNAMIC ENCODER + PURE-RUST FALLBACK BLOCK ENCODER
// ============================================================================

pub const NVTT_DEFAULT_INSTALL_DIRS: [&str; 2] = [
    r"C:\Program Files\NVIDIA Corporation\NVIDIA Texture Tools",
    r"C:\Program Files (x86)\NVIDIA Corporation\NVIDIA Texture Tools",
];

/// Locate `nvtt30205.dll` / `nvtt.dll` using `XT1 Studio` `ToolLocator` lookup order:
/// 1. Explicit configured path (`nvtt_dll_path`)
/// 2. Application executable directory
/// 3. Current working directory
/// 4. Standard NVIDIA Texture Tools install directories
/// 5. System `PATH`
pub fn locate_nvtt_dll(explicit_path: Option<&Path>) -> Option<std::path::PathBuf> {
    if let Some(p) = explicit_path {
        if !p.as_os_str().is_empty() && p.is_file() {
            return Some(p.to_path_buf());
        }
    }
    let dll_names = ["nvtt30205.dll", "nvtt.dll", "libnvtt.so", "libnvtt.dylib"];
    let mut search_dirs = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            search_dirs.push(parent.to_path_buf());
        }
    }
    if let Ok(cwd) = std::env::current_dir() {
        search_dirs.push(cwd);
    }
    for d in NVTT_DEFAULT_INSTALL_DIRS {
        search_dirs.push(Path::new(d).to_path_buf());
    }
    if let Some(path_os) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path_os) {
            if !dir.as_os_str().is_empty() {
                search_dirs.push(dir);
            }
        }
    }
    for dir in search_dirs {
        for name in dll_names {
            let cand = dir.join(name);
            if cand.is_file() {
                return Some(cand);
            }
        }
    }
    None
}

fn open_nvtt_library(path: &Path) -> std::result::Result<libloading::Library, String> {
    #[cfg(target_os = "windows")]
    {
        // LOAD_WITH_ALTERED_SEARCH_PATH (0x00000008) so sibling DLLs (vcomp140.dll, etc.)
        // next to nvtt30205.dll in Program Files resolve automatically.
        unsafe {
            libloading::os::windows::Library::load_with_flags(path, 0x0000_0008)
                .map(libloading::Library::from)
                .map_err(|e| e.to_string())
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        unsafe { libloading::Library::new(path).map_err(|e| e.to_string()) }
    }
}

/// Probe whether `nvtt30205.dll` is available and exports the NVTT 3 C API, returning
/// `(is_nvtt_active, human_readable_status)`.
pub fn get_nvtt_status_description(explicit_path: Option<&Path>) -> (bool, String) {
    let Some(dll_path) = locate_nvtt_dll(explicit_path) else {
        return (
            false,
            "Built-in pure-Rust encoder active (nvtt30205.dll not found)".to_string(),
        );
    };
    match open_nvtt_library(&dll_path) {
        Ok(lib) => unsafe {
            type VersionFn = unsafe extern "C" fn() -> u32;
            if let Ok(ver_fn) = lib.get::<VersionFn>(b"nvttVersion\0") {
                let v = ver_fn();
                return (
                    true,
                    format!("NVTT {v} active ({})", dll_path.display()),
                );
            }
            if lib
                .get::< unsafe extern "C" fn()>(b"pgtools_nvtt_encode_rgba8\0")
                .is_ok()
            {
                return (
                    true,
                    format!("NVTT wrapper active ({})", dll_path.display()),
                );
            }
            (
                false,
                format!(
                    "Built-in encoder active ({} missing nvttVersion export)",
                    dll_path.display()
                ),
            )
        },
        Err(e) => (
            false,
            format!(
                "Built-in encoder active (failed to load {}: {e})",
                dll_path.display()
            ),
        ),
    }
}

/// Map `TextureFormat` to `NvttFormat` enum (`nvtt_wrapper.h` 3.2.5), promoting `BC1` (1) to `BC1a` (2)
/// when `has_alpha` is true, and mapping all 14 ASTC LDR footprints (`17..=30`).
fn to_nvtt_c_format(format: TextureFormat, has_alpha: bool) -> Option<i32> {
    match format {
        TextureFormat::Bc1 | TextureFormat::Bc1Srgb => Some(if has_alpha { 2 } else { 1 }),
        TextureFormat::Bc2 | TextureFormat::Bc2Srgb => Some(3),
        TextureFormat::Bc3 | TextureFormat::Bc3Srgb => Some(4),
        TextureFormat::Bc4 => Some(6),
        TextureFormat::Bc5 => Some(9),
        TextureFormat::Bc6hUf16 => Some(13), // NvttFormat_BC6U
        TextureFormat::Bc6hSf16 => Some(14), // NvttFormat_BC6S
        TextureFormat::Bc7 | TextureFormat::Bc7Srgb => Some(15),
        TextureFormat::Astc(bw, bh, _) => {
            let idx = ASTC_FOOTPRINTS
                .iter()
                .position(|&(w, h, _)| w == bw && h == bh)?;
            Some(17 + idx as i32) // NvttFormat_ASTC_LDR_4x4 = 17 .. 12x12 = 30
        }
        _ => None,
    }
}

thread_local! {
    static NVTT_CAPTURE_BUF: std::cell::RefCell<Vec<u8>> = const { std::cell::RefCell::new(Vec::new()) };
}

unsafe extern "C" fn nvtt_cb_begin_image(
    size: i32,
    _width: i32,
    _height: i32,
    _depth: i32,
    _face: i32,
    _mip: i32,
) {
    NVTT_CAPTURE_BUF.with(|cell| {
        let mut buf = cell.borrow_mut();
        buf.clear();
        if size > 0 {
            buf.reserve(size as usize);
        }
    });
}

unsafe extern "C" fn nvtt_cb_write_data(data: *const u8, size: i32) -> i32 {
    if !data.is_null() && size > 0 {
        let slice = std::slice::from_raw_parts(data, size as usize);
        NVTT_CAPTURE_BUF.with(|cell| {
            cell.borrow_mut().extend_from_slice(slice);
        });
    }
    1
}

unsafe extern "C" fn nvtt_cb_end_image() {}

/// Compress a linear `RgbaImage` into raw compressed blocks using official `nvtt30205.dll`
/// (`nvtt_wrapper.h` 3.2.5 C API), matching `Xt1.Core/Nvtt/NvttNative.cs`.
/// Returns `None` if `nvtt30205.dll` is not present or does not support `format`.
pub fn try_encode_with_nvtt_dll_quality(
    img: &RgbaImage,
    format: TextureFormat,
    quality: EncodeQuality,
    nvtt_dll_path: Option<&Path>,
) -> Option<Vec<u8>> {
    let has_alpha = img.pixels().any(|p| p.0[3] < 255);
    let nvtt_format_id = to_nvtt_c_format(format, has_alpha)?;
    let dll_path = locate_nvtt_dll(nvtt_dll_path)?;
    let lib = open_nvtt_library(&dll_path).ok()?;

    let (bw, bh) = format.block_dimensions();
    let bx = ((img.width() as usize) + bw - 1) / bw;
    let by = ((img.height() as usize) + bh - 1) / bh;
    let expected_bytes = bx * by * format.bytes_per_block();
    if expected_bytes == 0 {
        return None;
    }

    // 1. Official NVTT 3.2.5 C API (`nvttCreateSurface`, `nvttContextCompress`, etc.)
    unsafe {
        type CreateSurfaceFn = unsafe extern "C" fn() -> *mut std::ffi::c_void;
        type DestroySurfaceFn = unsafe extern "C" fn(*mut std::ffi::c_void);
        type SetImageDataFn = unsafe extern "C" fn(
            *mut std::ffi::c_void,
            i32,
            i32,
            i32,
            i32,
            *const std::ffi::c_void,
            i32,
            *mut std::ffi::c_void,
        ) -> i32;
        type SetSurfaceAlphaModeFn = unsafe extern "C" fn(*mut std::ffi::c_void, i32);
        type CreateContextFn = unsafe extern "C" fn() -> *mut std::ffi::c_void;
        type DestroyContextFn = unsafe extern "C" fn(*mut std::ffi::c_void);
        type CreateCompOptsFn = unsafe extern "C" fn() -> *mut std::ffi::c_void;
        type DestroyCompOptsFn = unsafe extern "C" fn(*mut std::ffi::c_void);
        type SetCompFormatFn = unsafe extern "C" fn(*mut std::ffi::c_void, i32);
        type SetCompQualityFn = unsafe extern "C" fn(*mut std::ffi::c_void, i32);
        type SetCompQuantFn = unsafe extern "C" fn(*mut std::ffi::c_void, i32, i32, i32, i32);
        type CreateOutOptsFn = unsafe extern "C" fn() -> *mut std::ffi::c_void;
        type DestroyOutOptsFn = unsafe extern "C" fn(*mut std::ffi::c_void);
        type SetOutContainerFn = unsafe extern "C" fn(*mut std::ffi::c_void, i32);
        type SetOutHeaderFn = unsafe extern "C" fn(*mut std::ffi::c_void, i32);
        type SetOutSrgbFn = unsafe extern "C" fn(*mut std::ffi::c_void, i32);
        type BeginCb = unsafe extern "C" fn(i32, i32, i32, i32, i32, i32);
        type WriteCb = unsafe extern "C" fn(*const u8, i32) -> i32;
        type EndCb = unsafe extern "C" fn();
        type SetOutHandlerFn =
            unsafe extern "C" fn(*mut std::ffi::c_void, BeginCb, WriteCb, EndCb);
        type ContextOutputHeaderFn = unsafe extern "C" fn(
            *mut std::ffi::c_void,
            *mut std::ffi::c_void,
            i32,
            *mut std::ffi::c_void,
            *mut std::ffi::c_void,
        ) -> i32;
        type ContextCompressFn = unsafe extern "C" fn(
            *mut std::ffi::c_void,
            *mut std::ffi::c_void,
            i32,
            i32,
            *mut std::ffi::c_void,
            *mut std::ffi::c_void,
        ) -> i32;

        if let (
            Ok(create_surface),
            Ok(destroy_surface),
            Ok(set_image_data),
            Ok(set_alpha_mode),
            Ok(create_context),
            Ok(destroy_context),
            Ok(create_comp),
            Ok(destroy_comp),
            Ok(set_comp_format),
            Ok(set_comp_quality),
            Ok(set_comp_quant),
            Ok(create_out),
            Ok(destroy_out),
            Ok(set_out_container),
            Ok(set_out_header),
            Ok(set_out_srgb),
            Ok(set_out_handler),
            Ok(ctx_out_header),
            Ok(ctx_compress),
        ) = (
            lib.get::<CreateSurfaceFn>(b"nvttCreateSurface\0"),
            lib.get::<DestroySurfaceFn>(b"nvttDestroySurface\0"),
            lib.get::<SetImageDataFn>(b"nvttSurfaceSetImageData\0"),
            lib.get::<SetSurfaceAlphaModeFn>(b"nvttSetSurfaceAlphaMode\0"),
            lib.get::<CreateContextFn>(b"nvttCreateContext\0"),
            lib.get::<DestroyContextFn>(b"nvttDestroyContext\0"),
            lib.get::<CreateCompOptsFn>(b"nvttCreateCompressionOptions\0"),
            lib.get::<DestroyCompOptsFn>(b"nvttDestroyCompressionOptions\0"),
            lib.get::<SetCompFormatFn>(b"nvttSetCompressionOptionsFormat\0"),
            lib.get::<SetCompQualityFn>(b"nvttSetCompressionOptionsQuality\0"),
            lib.get::<SetCompQuantFn>(b"nvttSetCompressionOptionsQuantization\0"),
            lib.get::<CreateOutOptsFn>(b"nvttCreateOutputOptions\0"),
            lib.get::<DestroyOutOptsFn>(b"nvttDestroyOutputOptions\0"),
            lib.get::<SetOutContainerFn>(b"nvttSetOutputOptionsContainer\0"),
            lib.get::<SetOutHeaderFn>(b"nvttSetOutputOptionsOutputHeader\0"),
            lib.get::<SetOutSrgbFn>(b"nvttSetOutputOptionsSrgbFlag\0"),
            lib.get::<SetOutHandlerFn>(b"nvttSetOutputOptionsOutputHandler\0"),
            lib.get::<ContextOutputHeaderFn>(b"nvttContextOutputHeader\0"),
            lib.get::<ContextCompressFn>(b"nvttContextCompress\0"),
        ) {
            // NVTT's 8-bit input expects BGRA channel order (`NvttInputFormat::Bgra8Ub = 0`).
            let raw = img.as_raw();
            let mut bgra = vec![0u8; raw.len()];
            for (dst, src) in bgra.chunks_exact_mut(4).zip(raw.chunks_exact(4)) {
                dst[0] = src[2];
                dst[1] = src[1];
                dst[2] = src[0];
                dst[3] = src[3];
            }

            let surface = create_surface();
            let context = create_context();
            let comp = create_comp();
            let out = create_out();

            let result = (|| -> Option<Vec<u8>> {
                if surface.is_null() || context.is_null() || comp.is_null() || out.is_null() {
                    return None;
                }
                set_alpha_mode(surface, if has_alpha { 1 } else { 0 });
                let ok_img = set_image_data(
                    surface,
                    0, // NvttInputFormat_Bgra8Ub
                    img.width() as i32,
                    img.height() as i32,
                    1,
                    bgra.as_ptr() as *const std::ffi::c_void,
                    0,
                    std::ptr::null_mut(),
                );
                if ok_img == 0 {
                    return None;
                }
                set_comp_format(comp, nvtt_format_id);
                set_comp_quality(comp, quality as i32);
                set_comp_quant(comp, 0, 0, 0, 127);

                set_out_container(out, 1); // NvttContainer_Dds10
                set_out_header(out, 0);    // Raw blocks only (no DDS header)
                set_out_srgb(out, if format.is_srgb() { 1 } else { 0 });
                set_out_handler(out, nvtt_cb_begin_image, nvtt_cb_write_data, nvtt_cb_end_image);

                if ctx_out_header(context, surface, 1, comp, out) == 0 {
                    return None;
                }
                if ctx_compress(context, surface, 0, 0, comp, out) == 0 {
                    return None;
                }
                NVTT_CAPTURE_BUF.with(|cell| {
                    let captured = cell.borrow();
                    if captured.len() == expected_bytes {
                        Some(captured.clone())
                    } else {
                        None
                    }
                })
            })();

            if !out.is_null() {
                destroy_out(out);
            }
            if !comp.is_null() {
                destroy_comp(comp);
            }
            if !context.is_null() {
                destroy_context(context);
            }
            if !surface.is_null() {
                destroy_surface(surface);
            }
            if result.is_some() {
                return result;
            }
        }

        // 2. Fallback check for single-call wrapper symbol if present
        type NvttEncodeRgba8 = unsafe extern "C" fn(
            rgba_ptr: *const u8,
            width: u32,
            height: u32,
            format: i32,
            out_ptr: *mut u8,
            out_cap: usize,
        ) -> usize;
        if let Ok(sym) = lib.get::<NvttEncodeRgba8>(b"pgtools_nvtt_encode_rgba8\0") {
            let mut out = vec![0u8; expected_bytes];
            let written = sym(
                img.as_raw().as_ptr(),
                img.width(),
                img.height(),
                nvtt_format_id,
                out.as_mut_ptr(),
                out.len(),
            );
            if written == expected_bytes {
                return Some(out);
            }
        }
    }
    None
}

/// Encode an `RgbaImage` into linear compressed/uncompressed blocks for `format`.
/// Attempts to use `nvtt30205.dll` if available, falling back automatically
/// to our pure-Rust BC1–BC7 / ASTC / RGBA8 / BGRA8 / R8 encoder.
pub fn encode_linear_surface(
    img: &RgbaImage,
    format: TextureFormat,
    nvtt_dll_path: Option<&Path>,
) -> Vec<u8> {
    encode_linear_surface_with_quality(img, format, EncodeQuality::Normal, nvtt_dll_path)
}

pub fn encode_linear_surface_with_quality(
    img: &RgbaImage,
    format: TextureFormat,
    quality: EncodeQuality,
    nvtt_dll_path: Option<&Path>,
) -> Vec<u8> {
    if let Some(hw_encoded) = try_encode_with_nvtt_dll_quality(img, format, quality, nvtt_dll_path) {
        return hw_encoded;
    }
    let w = img.width() as usize;
    let h = img.height() as usize;
    let raw = img.as_raw();

    match format {
        TextureFormat::AfL8Unorm | TextureFormat::R8Unorm => {
            let mut out = Vec::with_capacity(w * h);
            for p in raw.chunks_exact(4) {
                out.push(p[0]);
            }
            out
        }
        TextureFormat::Rg8Unorm => {
            let mut out = Vec::with_capacity(w * h * 2);
            for p in raw.chunks_exact(4) {
                out.push(p[0]);
                out.push(p[1]);
            }
            out
        }
        TextureFormat::Bgra8 | TextureFormat::Bgra8Srgb => {
            let mut out = raw.clone();
            for p in out.chunks_exact_mut(4) {
                p.swap(0, 2);
            }
            out
        }
        TextureFormat::Rgba8 | TextureFormat::Rgba8Srgb => raw.clone(),
        TextureFormat::Rgba16Float => {
            let mut out = Vec::with_capacity(w * h * 8);
            for p in raw.chunks_exact(4) {
                for &ch in p {
                    let f = (ch as f32) / 255.0;
                    out.extend_from_slice(&F16::from_f32(f).0.to_le_bytes());
                }
            }
            out
        }
        TextureFormat::Bc1 | TextureFormat::Bc1Srgb => encode_bc1_surface(img),
        TextureFormat::Bc2 | TextureFormat::Bc2Srgb => encode_bc2_surface(img),
        TextureFormat::Bc3 | TextureFormat::Bc3Srgb => encode_bc3_surface(img),
        TextureFormat::Bc4 => encode_bc4_surface(img, 0),
        TextureFormat::Bc5 => encode_bc5_surface(img),
        TextureFormat::Bc6hUf16 => encode_rgba8_to_bc6h_linear(img, false),
        TextureFormat::Bc6hSf16 => encode_rgba8_to_bc6h_linear(img, true),
        TextureFormat::Bc7 | TextureFormat::Bc7Srgb => encode_rgba8_to_bc7_linear(img),
        TextureFormat::Astc(bw, bh, _) => encode_rgba8_to_astc_linear(img, bw as usize, bh as usize),
        _ => raw.clone(),
    }
}

fn rgb888_to_rgb565(r: u8, g: u8, b: u8) -> u16 {
    (((r as u16) >> 3) << 11) | (((g as u16) >> 2) << 5) | ((b as u16) >> 3)
}

fn encode_bc1_block_rgba(pixels: &[[u8; 4]; 16]) -> [u8; 8] {
    let mut min_r = 255u8;
    let mut min_g = 255u8;
    let mut min_b = 255u8;
    let mut max_r = 0u8;
    let mut max_g = 0u8;
    let mut max_b = 0u8;

    for p in pixels {
        min_r = min_r.min(p[0]);
        min_g = min_g.min(p[1]);
        min_b = min_b.min(p[2]);
        max_r = max_r.max(p[0]);
        max_g = max_g.max(p[1]);
        max_b = max_b.max(p[2]);
    }

    let mut c0 = rgb888_to_rgb565(max_r, max_g, max_b);
    let mut c1 = rgb888_to_rgb565(min_r, min_g, min_b);
    if c0 < c1 {
        std::mem::swap(&mut c0, &mut c1);
    } else if c0 == c1 {
        if c0 > 0 {
            c1 = c0 - 1;
        } else {
            c0 = 1;
        }
    }

    let rgb0 = rgb565_to_rgb888(c0);
    let rgb1 = rgb565_to_rgb888(c1);
    let pal = [
        rgb0,
        rgb1,
        [
            ((2 * rgb0[0] as u16 + rgb1[0] as u16) / 3) as u8,
            ((2 * rgb0[1] as u16 + rgb1[1] as u16) / 3) as u8,
            ((2 * rgb0[2] as u16 + rgb1[2] as u16) / 3) as u8,
        ],
        [
            ((rgb0[0] as u16 + 2 * rgb1[0] as u16) / 3) as u8,
            ((rgb0[1] as u16 + 2 * rgb1[1] as u16) / 3) as u8,
            ((rgb0[2] as u16 + 2 * rgb1[2] as u16) / 3) as u8,
        ],
    ];

    let mut indices = 0u32;
    for (i, p) in pixels.iter().enumerate() {
        let mut best_idx = 0u32;
        let mut best_dist = u32::MAX;
        for (pi, c) in pal.iter().enumerate() {
            let dr = (p[0] as i32) - (c[0] as i32);
            let dg = (p[1] as i32) - (c[1] as i32);
            let db = (p[2] as i32) - (c[2] as i32);
            let dist = (dr * dr + dg * dg + db * db) as u32;
            if dist < best_dist {
                best_dist = dist;
                best_idx = pi as u32;
            }
        }
        indices |= best_idx << (i * 2);
    }

    let mut out = [0u8; 8];
    out[0..2].copy_from_slice(&c0.to_le_bytes());
    out[2..4].copy_from_slice(&c1.to_le_bytes());
    out[4..8].copy_from_slice(&indices.to_le_bytes());
    out
}

fn encode_bc4_block_channel(pixels: &[[u8; 4]; 16], ch: usize) -> [u8; 8] {
    let mut min_v = 255u8;
    let mut max_v = 0u8;
    for p in pixels {
        min_v = min_v.min(p[ch]);
        max_v = max_v.max(p[ch]);
    }
    let (a0, a1) = if max_v == min_v {
        if max_v > 0 { (max_v, max_v - 1) } else { (1, 0) }
    } else {
        (max_v, min_v)
    };
    let mut lut = [0u8; 8];
    lut[0] = a0;
    lut[1] = a1;
    for i in 1..7 {
        lut[i + 1] = (((7 - i) as u16 * a0 as u16 + i as u16 * a1 as u16) / 7) as u8;
    }

    let mut bits = 0u64;
    for (i, p) in pixels.iter().enumerate() {
        let v = p[ch];
        let mut best_idx = 0u64;
        let mut best_diff = u8::MAX;
        for (li, &lv) in lut.iter().enumerate() {
            let diff = v.abs_diff(lv);
            if diff < best_diff {
                best_diff = diff;
                best_idx = li as u64;
            }
        }
        bits |= best_idx << (i * 3);
    }

    let mut out = [0u8; 8];
    out[0] = a0;
    out[1] = a1;
    let b_le = bits.to_le_bytes();
    out[2..8].copy_from_slice(&b_le[0..6]);
    out
}

fn extract_4x4_block(img: &RgbaImage, bx: usize, by: usize) -> [[u8; 4]; 16] {
    let w = img.width() as usize;
    let h = img.height() as usize;
    let mut block = [[0u8; 4]; 16];
    for py in 0..4 {
        let y = (by * 4 + py).min(h.saturating_sub(1));
        for px in 0..4 {
            let x = (bx * 4 + px).min(w.saturating_sub(1));
            block[py * 4 + px] = img.get_pixel(x as u32, y as u32).0;
        }
    }
    block
}

fn encode_bc1_surface(img: &RgbaImage) -> Vec<u8> {
    let bx_count = ((img.width() as usize) + 3) / 4;
    let by_count = ((img.height() as usize) + 3) / 4;
    let mut out = Vec::with_capacity(bx_count * by_count * 8);
    for by in 0..by_count {
        for bx in 0..bx_count {
            let blk = extract_4x4_block(img, bx, by);
            out.extend_from_slice(&encode_bc1_block_rgba(&blk));
        }
    }
    out
}

fn encode_bc2_surface(img: &RgbaImage) -> Vec<u8> {
    let bx_count = ((img.width() as usize) + 3) / 4;
    let by_count = ((img.height() as usize) + 3) / 4;
    let mut out = Vec::with_capacity(bx_count * by_count * 16);
    for by in 0..by_count {
        for bx in 0..bx_count {
            let blk = extract_4x4_block(img, bx, by);
            let mut alpha_bits = 0u64;
            for (i, p) in blk.iter().enumerate() {
                let a4 = (p[3] >> 4) as u64;
                alpha_bits |= a4 << (i * 4);
            }
            out.extend_from_slice(&alpha_bits.to_le_bytes());
            out.extend_from_slice(&encode_bc1_block_rgba(&blk));
        }
    }
    out
}

fn encode_bc3_surface(img: &RgbaImage) -> Vec<u8> {
    let bx_count = ((img.width() as usize) + 3) / 4;
    let by_count = ((img.height() as usize) + 3) / 4;
    let mut out = Vec::with_capacity(bx_count * by_count * 16);
    for by in 0..by_count {
        for bx in 0..bx_count {
            let blk = extract_4x4_block(img, bx, by);
            out.extend_from_slice(&encode_bc4_block_channel(&blk, 3));
            out.extend_from_slice(&encode_bc1_block_rgba(&blk));
        }
    }
    out
}

fn encode_bc4_surface(img: &RgbaImage, channel: usize) -> Vec<u8> {
    let bx_count = ((img.width() as usize) + 3) / 4;
    let by_count = ((img.height() as usize) + 3) / 4;
    let mut out = Vec::with_capacity(bx_count * by_count * 8);
    for by in 0..by_count {
        for bx in 0..bx_count {
            let blk = extract_4x4_block(img, bx, by);
            out.extend_from_slice(&encode_bc4_block_channel(&blk, channel));
        }
    }
    out
}

fn encode_bc5_surface(img: &RgbaImage) -> Vec<u8> {
    let bx_count = ((img.width() as usize) + 3) / 4;
    let by_count = ((img.height() as usize) + 3) / 4;
    let mut out = Vec::with_capacity(bx_count * by_count * 16);
    for by in 0..by_count {
        for bx in 0..bx_count {
            let blk = extract_4x4_block(img, bx, by);
            out.extend_from_slice(&encode_bc4_block_channel(&blk, 0));
            out.extend_from_slice(&encode_bc4_block_channel(&blk, 1));
        }
    }
    out
}

/// Encode an `RgbaImage` into a complete `.xt1` (56-byte `XT1\0` header + Tegra X1 GOB-swizzled mip chain).
pub fn encode_rgba_to_xt1(
    base_img: &RgbaImage,
    target_format: TextureFormat,
    generate_mips: bool,
    apply_solidify: bool,
    make_opaque: bool,
    template_header: Option<&[u8]>,
) -> Vec<u8> {
    encode_rgba_to_xt1_with_nvtt(
        base_img,
        target_format,
        generate_mips,
        apply_solidify,
        make_opaque,
        template_header,
        None,
    )
}

pub fn encode_rgba_to_xt1_with_nvtt(
    base_img: &RgbaImage,
    target_format: TextureFormat,
    generate_mips: bool,
    apply_solidify: bool,
    make_opaque: bool,
    template_header: Option<&[u8]>,
    nvtt_dll_path: Option<&Path>,
) -> Vec<u8> {
    let mut processed = if apply_solidify {
        solidify_rgba_image(base_img, 0)
    } else {
        base_img.clone()
    };
    if make_opaque {
        for p in processed.pixels_mut() {
            p.0[3] = 255;
        }
    }

    let max_mips = if generate_mips {
        (processed.width().max(processed.height()) as f32)
            .log2()
            .floor() as u32
            + 1
    } else {
        1
    };
    let mip_images = generate_mip_chain(&processed, max_mips);
    let mip_count = mip_images.len() as u32;

    // Preserve exact requested target_format (including all 14 ASTC footprints, BC7, BC6H, etc.)
    let enc_format = target_format;

    let (_, bh) = enc_format.block_dimensions();
    let hb0 = ((processed.height() as usize) + bh - 1) / bh;
    let blk_log2 = gob_swizzle::block_height_log2(hb0);

    let layout_res = compute_xt1_layout(
        processed.width(),
        processed.height(),
        1,
        mip_count,
        1,
        blk_log2,
        enc_format,
        0,
    );

    let bpb = enc_format.bytes_per_block();
    let mut payload = vec![0u8; layout_res.computed_image_size];
    for (m, mip_info) in layout_res.mips.iter().enumerate() {
        let linear = encode_linear_surface(&mip_images[m], enc_format, nvtt_dll_path);
        let mut swizzled = vec![0u8; mip_info.slice_surface_size];
        gob_swizzle::swizzle_volume(
            &linear,
            &mut swizzled,
            mip_info.width_blocks,
            mip_info.height_blocks,
            1,
            bpb,
            mip_info.block_height,
            1,
            mip_info.pitch,
        );
        if mip_info.offset + swizzled.len() <= payload.len() {
            payload[mip_info.offset..mip_info.offset + swizzled.len()].copy_from_slice(&swizzled);
        }
    }

    let mut hdr = [0u8; 0x38];
    if let Some(tpl) = template_header {
        if tpl.len() >= 0x38 {
            hdr.copy_from_slice(&tpl[0..0x38]);
        }
    } else {
        hdr[0..4].copy_from_slice(b"XT1\0");
        hdr[4..8].copy_from_slice(&0x00010000u32.to_le_bytes());
        hdr[0x34..0x38].copy_from_slice(&0x00010007u32.to_le_bytes());
    }
    hdr[0..4].copy_from_slice(b"XT1\0");
    hdr[0x08..0x10].copy_from_slice(&(payload.len() as u64).to_le_bytes());
    hdr[0x10..0x14].copy_from_slice(&0x38u32.to_le_bytes());
    let codec_tag: u16 = if enc_format == TextureFormat::AfL8Unorm {
        0x6661
    } else {
        0
    };
    let mip_word = (mip_count & 0xFFFF) | ((codec_tag as u32) << 16);
    hdr[0x14..0x18].copy_from_slice(&mip_word.to_le_bytes());
    hdr[0x18..0x1C].copy_from_slice(&1u32.to_le_bytes()); // T_2D
    hdr[0x1C..0x20].copy_from_slice(&enc_format.to_xt1_code().to_le_bytes());
    hdr[0x20..0x24].copy_from_slice(&processed.width().to_le_bytes());
    hdr[0x24..0x28].copy_from_slice(&processed.height().to_le_bytes());
    hdr[0x28..0x2C].copy_from_slice(&1u32.to_le_bytes());
    let prev_layout = u32::from_le_bytes(hdr[0x30..0x34].try_into().unwrap());
    let new_layout = (prev_layout & !0x407u32) | blk_log2;
    hdr[0x30..0x34].copy_from_slice(&new_layout.to_le_bytes());

    let mut full = Vec::with_capacity(0x38 + payload.len());
    full.extend_from_slice(&hdr);
    full.extend_from_slice(&payload);
    full
}

// ============================================================================
// DDS / GTX / XT1 / BNTX INSPECTION & DECODING
// ============================================================================

/// Build a standard 128-byte (or 148-byte DX10) `.dds` header + payload.
pub fn build_dds_file(
    width: u32,
    height: u32,
    mip_count: u32,
    format: TextureFormat,
    payload: &[u8],
) -> Vec<u8> {
    let mut w = BinWriter::with_capacity(148 + payload.len(), Endian::Little);
    w.write_bytes(b"DDS ");
    w.write_u32(124);
    let (bw, bh) = format.block_dimensions();
    let is_block = bw > 1 || bh > 1;
    let flags = 0x1
        | 0x2
        | 0x4
        | 0x1000
        | if is_block { 0x80000 } else { 0x8 }
        | if mip_count > 1 { 0x20000 } else { 0 };
    w.write_u32(flags);
    w.write_u32(height);
    w.write_u32(width);

    let pitch_or_linear = if is_block {
        (((width as usize) + bw - 1) / bw)
            * (((height as usize) + bh - 1) / bh)
            * format.bytes_per_block()
    } else {
        (width as usize) * format.bytes_per_block()
    };
    w.write_u32(pitch_or_linear as u32);
    w.write_u32(1);
    w.write_u32(mip_count.max(1));
    for _ in 0..11 {
        w.write_u32(0);
    }

    let dx10_code = match format {
        TextureFormat::Bc6hUf16
        | TextureFormat::Bc6hSf16
        | TextureFormat::Bc7
        | TextureFormat::Bc7Srgb
        | TextureFormat::Astc(_, _, _)
        | TextureFormat::Rgba16Float
        | TextureFormat::Rg8Unorm
        | TextureFormat::R8Unorm
        | TextureFormat::AfL8Unorm => format.to_dxgi_format(),
        _ => None,
    };

    w.write_u32(32);
    if dx10_code.is_some() {
        w.write_u32(0x4);
        w.write_bytes(b"DX10");
        for _ in 0..5 {
            w.write_u32(0);
        }
    } else {
        match format {
            TextureFormat::Bc1 | TextureFormat::Bc1Srgb => {
                w.write_u32(0x4);
                w.write_bytes(b"DXT1");
                for _ in 0..5 {
                    w.write_u32(0);
                }
            }
            TextureFormat::Bc2 | TextureFormat::Bc2Srgb => {
                w.write_u32(0x4);
                w.write_bytes(b"DXT3");
                for _ in 0..5 {
                    w.write_u32(0);
                }
            }
            TextureFormat::Bc3 | TextureFormat::Bc3Srgb => {
                w.write_u32(0x4);
                w.write_bytes(b"DXT5");
                for _ in 0..5 {
                    w.write_u32(0);
                }
            }
            TextureFormat::Bc4 => {
                w.write_u32(0x4);
                w.write_bytes(b"ATI1");
                for _ in 0..5 {
                    w.write_u32(0);
                }
            }
            TextureFormat::Bc5 => {
                w.write_u32(0x4);
                w.write_bytes(b"ATI2");
                for _ in 0..5 {
                    w.write_u32(0);
                }
            }
            TextureFormat::Bgra8 | TextureFormat::Bgra8Srgb => {
                w.write_u32(0x41);
                w.write_u32(0);
                w.write_u32(32);
                w.write_u32(0x00FF_0000);
                w.write_u32(0x0000_FF00);
                w.write_u32(0x0000_00FF);
                w.write_u32(0xFF00_0000);
            }
            _ => {
                w.write_u32(0x41);
                w.write_u32(0);
                w.write_u32(32);
                w.write_u32(0x0000_00FF);
                w.write_u32(0x0000_FF00);
                w.write_u32(0x00FF_0000);
                w.write_u32(0xFF00_0000);
            }
        }
    }

    w.write_u32(0x1000 | if mip_count > 1 { 0x400008 } else { 0 });
    for _ in 0..4 {
        w.write_u32(0);
    }

    if let Some(dxgi) = dx10_code {
        w.write_u32(dxgi);
        w.write_u32(3); // D3D10_RESOURCE_DIMENSION_TEXTURE2D
        w.write_u32(0); // miscFlag
        w.write_u32(1); // arraySize
        w.write_u32(0); // miscFlags2
    }

    w.write_bytes(payload);
    w.into_inner()
}

/// Encode an `RgbaImage` into a `.dds` file with explicit format, mipmap chain generation,
/// Solidify alpha edge-bleeding, and opaque override.
pub fn encode_rgba_to_dds_with_options(
    base_img: &RgbaImage,
    target_format: TextureFormat,
    generate_mips: bool,
    apply_solidify: bool,
    make_opaque: bool,
) -> Vec<u8> {
    encode_rgba_to_dds_with_nvtt(
        base_img,
        target_format,
        generate_mips,
        apply_solidify,
        make_opaque,
        None,
    )
}

pub fn encode_rgba_to_dds_with_nvtt(
    base_img: &RgbaImage,
    target_format: TextureFormat,
    generate_mips: bool,
    apply_solidify: bool,
    make_opaque: bool,
    nvtt_dll_path: Option<&Path>,
) -> Vec<u8> {
    let mut processed = if apply_solidify {
        solidify_rgba_image(base_img, 0)
    } else {
        base_img.clone()
    };
    if make_opaque {
        for p in processed.pixels_mut() {
            p.0[3] = 255;
        }
    }

    let max_mips = if generate_mips {
        (processed.width().max(processed.height()) as f32)
            .log2()
            .floor() as u32
            + 1
    } else {
        1
    };
    let mip_images = generate_mip_chain(&processed, max_mips);
    let mip_count = mip_images.len() as u32;

    let mut payload = Vec::new();
    for mip_img in &mip_images {
        payload.extend_from_slice(&encode_linear_surface(mip_img, target_format, nvtt_dll_path));
    }
    build_dds_file(
        processed.width(),
        processed.height(),
        mip_count,
        target_format,
        &payload,
    )
}

/// Losslessly swizzle a `.dds` file (DX9 or DX10, including BC1..BC7, ASTC, RGBA8) directly into
/// a Nintendo Switch `.xt1` texture without decoding or re-encoding blocks.
pub fn swizzle_dds_to_xt1(
    dds_bytes: &[u8],
    template_header: Option<&[u8]>,
) -> Result<Vec<u8>> {
    let meta = inspect_texture_blob(dds_bytes)?;
    if meta.container_ext != ".dds" {
        return Err(PgError::InvalidData("Expected .dds input for swizzle_dds_to_xt1".into()));
    }
    let header_size = if dds_bytes.len() >= 148 && &dds_bytes[84..88] == b"DX10" {
        148usize
    } else {
        128usize
    };
    if dds_bytes.len() < header_size {
        return Err(PgError::InvalidData("Truncated DDS header".into()));
    }
    let dds_payload = &dds_bytes[header_size..];

    let (_bw, bh) = meta.format.block_dimensions();
    let bpb = meta.format.bytes_per_block();
    let hb0 = ((meta.height as usize) + bh - 1) / bh;
    let blk_log2 = gob_swizzle::block_height_log2(hb0);

    let layout_res = compute_xt1_layout(
        meta.width,
        meta.height,
        meta.depth.max(1),
        meta.mip_count.max(1),
        1,
        blk_log2,
        meta.format,
        0,
    );

    let mut payload = vec![0u8; layout_res.computed_image_size];
    let mut dds_cursor = 0usize;

    for mip_info in &layout_res.mips {
        let linear_len = mip_info.width_blocks * mip_info.height_blocks * bpb;
        let mut linear = vec![0u8; linear_len];
        if dds_cursor < dds_payload.len() {
            let avail = (dds_payload.len() - dds_cursor).min(linear_len);
            linear[..avail].copy_from_slice(&dds_payload[dds_cursor..dds_cursor + avail]);
            dds_cursor += avail;
        }
        let mut swizzled = vec![0u8; mip_info.slice_surface_size];
        gob_swizzle::swizzle_volume(
            &linear,
            &mut swizzled,
            mip_info.width_blocks,
            mip_info.height_blocks,
            1,
            bpb,
            mip_info.block_height,
            1,
            mip_info.pitch,
        );
        if mip_info.offset + swizzled.len() <= payload.len() {
            payload[mip_info.offset..mip_info.offset + swizzled.len()].copy_from_slice(&swizzled);
        }
    }

    let mut hdr = [0u8; 0x38];
    if let Some(tpl) = template_header {
        if tpl.len() >= 0x38 {
            hdr.copy_from_slice(&tpl[0..0x38]);
        }
    } else {
        hdr[0..4].copy_from_slice(b"XT1\0");
        hdr[4..8].copy_from_slice(&0x00010000u32.to_le_bytes());
        hdr[0x34..0x38].copy_from_slice(&0x00010007u32.to_le_bytes());
    }
    hdr[0..4].copy_from_slice(b"XT1\0");
    hdr[0x08..0x10].copy_from_slice(&(payload.len() as u64).to_le_bytes());
    hdr[0x10..0x14].copy_from_slice(&0x38u32.to_le_bytes());
    let codec_tag: u16 = if meta.format == TextureFormat::AfL8Unorm {
        0x6661
    } else {
        0
    };
    let mip_word = (meta.mip_count.max(1) & 0xFFFF) | ((codec_tag as u32) << 16);
    hdr[0x14..0x18].copy_from_slice(&mip_word.to_le_bytes());
    hdr[0x18..0x1C].copy_from_slice(&1u32.to_le_bytes());
    hdr[0x1C..0x20].copy_from_slice(&meta.format.to_xt1_code().to_le_bytes());
    hdr[0x20..0x24].copy_from_slice(&meta.width.to_le_bytes());
    hdr[0x24..0x28].copy_from_slice(&meta.height.to_le_bytes());
    hdr[0x28..0x2C].copy_from_slice(&meta.depth.max(1).to_le_bytes());
    let prev_layout = u32::from_le_bytes(hdr[0x30..0x34].try_into().unwrap());
    let new_layout = (prev_layout & !0x407u32) | blk_log2;
    hdr[0x30..0x34].copy_from_slice(&new_layout.to_le_bytes());

    let mut full = Vec::with_capacity(0x38 + payload.len());
    full.extend_from_slice(&hdr);
    full.extend_from_slice(&payload);
    Ok(full)
}

/// Rebuild a `.bntx` container when one or more `BRTI` sub-textures change format, dimensions,
/// mip count, or byte size (e.g., ASTC -> BC recompression or cross-format texture replacement).
/// Automatically updates `BRTI` headers, `u64` `mip_offsets`, `BRTD` size, and trailing `_RLT` relocation section.
pub fn rebuild_bntx_container_subtexture(
    bntx_bytes: &[u8],
    sub_index: usize,
    new_format: TextureFormat,
    new_width: u32,
    new_height: u32,
    new_block_height_log2: u32,
    new_swizzled_mips: &[Vec<u8>],
) -> Result<Vec<u8>> {
    let base_off = if bntx_bytes.len() >= 0x28 && &bntx_bytes[0..4] == b"BNTX" {
        0usize
    } else if bntx_bytes.len() >= 80 + 0x28 && &bntx_bytes[80..84] == b"BNTX" {
        80usize
    } else {
        return Err(PgError::InvalidData("Missing BNTX magic".into()));
    };

    let recs = parse_bntx_textures(bntx_bytes)?;
    let target_idx = sub_index.min(recs.len().saturating_sub(1));
    let target_rec = &recs[target_idx];

    // Check if the new mips fit the exact existing layout & dimensions for fast in-place overwrite
    let same_geom = new_width == target_rec.width
        && new_height == target_rec.height
        && new_swizzled_mips.len() == target_rec.mip_offsets.len()
        && new_format.block_dimensions() == target_rec.format.block_dimensions()
        && new_format.bytes_per_block() == target_rec.format.bytes_per_block();

    if same_geom {
        let mut out = bntx_bytes.to_vec();
        let new_surf_fmt = new_format.to_bntx_surface_format();
        if target_rec.brti_offset + 32 <= out.len()
            && &out[target_rec.brti_offset..target_rec.brti_offset + 4] == b"BRTI"
        {
            out[target_rec.brti_offset + 28..target_rec.brti_offset + 32]
                .copy_from_slice(&new_surf_fmt.to_le_bytes());
        }
        for (m, &mip_off) in target_rec.mip_offsets.iter().enumerate() {
            if let Some(swizzled) = new_swizzled_mips.get(m) {
                if mip_off < out.len() {
                    let copy_len = (out.len() - mip_off).min(swizzled.len());
                    out[mip_off..mip_off + copy_len].copy_from_slice(&swizzled[..copy_len]);
                }
            }
        }
        return Ok(out);
    }

    // Full relocating BRTD repack for changed block footprint (e.g. ASTC -> BC), mip count, or dimensions:
    // 1. Collect existing swizzled mip surfaces for all other sub-textures
    let mut all_tex_mips: Vec<Vec<Vec<u8>>> = Vec::with_capacity(recs.len());
    for (idx, rec) in recs.iter().enumerate() {
        if idx == target_idx {
            all_tex_mips.push(new_swizzled_mips.to_vec());
        } else {
            let (bw, bh) = rec.format.block_dimensions();
            let bpb = rec.format.bytes_per_block();
            let bh_mip0 = 1usize << rec.block_height_log2;
            let mut mips = Vec::with_capacity(rec.mip_offsets.len());
            for (m, &mip_off) in rec.mip_offsets.iter().enumerate() {
                let mw = (rec.width >> m).max(1) as usize;
                let mh = (rec.height >> m).max(1) as usize;
                let wb = (mw + bw - 1) / bw;
                let hb = (mh + bh - 1) / bh;
                let mip_bh = gob_swizzle::mip_block_height(hb, bh_mip0);
                let pitch = align(wb * bpb, 64);
                let surf_size = pitch * align(hb, mip_bh * 8);
                let mut buf = vec![0u8; surf_size];
                if mip_off < bntx_bytes.len() {
                    let avail = (bntx_bytes.len() - mip_off).min(surf_size);
                    buf[..avail].copy_from_slice(&bntx_bytes[mip_off..mip_off + avail]);
                }
                mips.push(buf);
            }
            all_tex_mips.push(mips);
        }
    }

    // 2. Locate first mip offset, BRTD block header, and trailing _RLT relocation table
    let first_mip_off = recs
        .iter()
        .filter_map(|r| r.mip_offsets.first().copied())
        .min()
        .ok_or_else(|| PgError::InvalidData("BNTX has no mip offsets".into()))?;

    let mut brtd_off = None;
    let search_limit = first_mip_off.min(bntx_bytes.len().saturating_sub(16));
    for pos in (base_off..search_limit).rev() {
        if &bntx_bytes[pos..pos + 4] == b"BRTD" {
            brtd_off = Some(pos);
            break;
        }
    }

    let rlt_off_18 = if base_off + 0x1C <= bntx_bytes.len() {
        u32::from_le_bytes(bntx_bytes[base_off + 0x18..base_off + 0x1C].try_into().unwrap()) as usize
    } else {
        0
    };
    let rlt_off_14 = if base_off + 0x18 <= bntx_bytes.len() {
        u32::from_le_bytes(bntx_bytes[base_off + 0x14..base_off + 0x18].try_into().unwrap()) as usize
    } else {
        0
    };
    let (old_rlt_abs, rlt_hdr_field_off) = if rlt_off_18 > 0
        && base_off + rlt_off_18 + 4 <= bntx_bytes.len()
        && &bntx_bytes[base_off + rlt_off_18..base_off + rlt_off_18 + 4] == b"_RLT"
    {
        (Some(base_off + rlt_off_18), 0x18usize)
    } else if rlt_off_14 > 0
        && base_off + rlt_off_14 + 4 <= bntx_bytes.len()
        && &bntx_bytes[base_off + rlt_off_14..base_off + rlt_off_14 + 4] == b"_RLT"
    {
        (Some(base_off + rlt_off_14), 0x14usize)
    } else {
        (None, 0x18usize)
    };
    let rlt_tail = old_rlt_abs.map(|off| bntx_bytes[off..].to_vec());

    // 3. Build output starting with everything up to `first_mip_off`
    let mut out = bntx_bytes[..first_mip_off].to_vec();
    let mut new_mip_tables: Vec<Vec<u64>> = Vec::with_capacity(recs.len());
    let mut new_image_sizes: Vec<u32> = Vec::with_capacity(recs.len());

    for (idx, mips) in all_tex_mips.iter().enumerate() {
        let align_bytes = (recs[idx].alignment.max(512) as usize).next_power_of_two();
        let tex_start = align(out.len() - base_off, align_bytes) + base_off;
        if out.len() < tex_start {
            out.resize(tex_start, 0);
        }
        let mut mip_ptrs = Vec::with_capacity(mips.len());
        let first_m_pos = out.len();
        for (m, surf) in mips.iter().enumerate() {
            if m > 0 {
                let m_pos = align(out.len() - base_off, 512) + base_off;
                if out.len() < m_pos {
                    out.resize(m_pos, 0);
                }
            }
            let cur_rel = (out.len() - base_off) as u64;
            mip_ptrs.push(cur_rel);
            out.extend_from_slice(surf);
        }
        let total_img_sz = (out.len() - first_m_pos) as u32;
        new_mip_tables.push(mip_ptrs);
        new_image_sizes.push(total_img_sz);
    }

    // 4. Patch BRTI headers and mip pointer tables (matching parse_bntx_textures field offsets)
    for (idx, rec) in recs.iter().enumerate() {
        let bo = rec.brti_offset;
        if bo + 0x78 <= out.len() && &out[bo..bo + 4] == b"BRTI" {
            let mip_ptrs_rel =
                u64::from_le_bytes(out[bo + 0x70..bo + 0x78].try_into().unwrap()) as usize;
            let mip_ptrs_abs = base_off + mip_ptrs_rel;
            let new_ptrs = &new_mip_tables[idx];
            let cap_mips = new_ptrs.len().min(rec.mip_count as usize);
            let fallback_rel = (first_mip_off.saturating_sub(base_off)) as u64;
            let last_valid_ptr = new_ptrs
                .get(cap_mips.saturating_sub(1))
                .copied()
                .unwrap_or(fallback_rel);
            // Fill all rec.mip_count slots so _RLT relocation never points outside BRTD when mip count shrinks
            for m in 0..(rec.mip_count as usize) {
                let ptr = if m < cap_mips { new_ptrs[m] } else { last_valid_ptr };
                let slot = mip_ptrs_abs + m * 8;
                if slot + 8 <= out.len() {
                    out[slot..slot + 8].copy_from_slice(&ptr.to_le_bytes());
                }
            }
            out[bo + 0x50..bo + 0x54].copy_from_slice(&new_image_sizes[idx].to_le_bytes());

            if idx == target_idx {
                let new_mip_cnt = (cap_mips.max(1)) as u16;
                let new_surf_fmt = new_format.to_bntx_surface_format();
                let old_layout = u32::from_le_bytes(out[bo + 0x34..bo + 0x38].try_into().unwrap());
                let patched_layout = (old_layout & !7u32) | (new_block_height_log2 & 7);
                out[bo + 0x16..bo + 0x18].copy_from_slice(&new_mip_cnt.to_le_bytes());
                out[bo + 0x1C..bo + 0x20].copy_from_slice(&new_surf_fmt.to_le_bytes());
                out[bo + 0x24..bo + 0x28].copy_from_slice(&new_width.to_le_bytes());
                out[bo + 0x28..bo + 0x2C].copy_from_slice(&new_height.to_le_bytes());
                out[bo + 0x34..bo + 0x38].copy_from_slice(&patched_layout.to_le_bytes());
            }
        }
    }

    // 5. Align end of BRTD, update BRTD size, and re-attach _RLT relocation table
    if let Some(mut rlt) = rlt_tail {
        let new_rlt_rel = align(out.len() - base_off, 4096);
        let new_rlt_abs = base_off + new_rlt_rel;
        if out.len() < new_rlt_abs {
            out.resize(new_rlt_abs, 0);
        }
        if let Some(b_off) = brtd_off {
            let brtd_size = (new_rlt_abs - b_off) as u64;
            if b_off + 16 <= out.len() {
                out[b_off + 8..b_off + 16].copy_from_slice(&brtd_size.to_le_bytes());
            }
            if rlt.len() >= 0x38 {
                let sec1_pos = u32::from_le_bytes(rlt[0x30..0x34].try_into().unwrap()) as usize;
                if sec1_pos == b_off.saturating_sub(base_off) && new_rlt_rel >= sec1_pos {
                    let sec1_size = (new_rlt_rel - sec1_pos) as u32;
                    rlt[0x34..0x38].copy_from_slice(&sec1_size.to_le_bytes());
                }
            }
        }
        if rlt.len() >= 8 {
            let new_rlt_u32 = new_rlt_rel as u32;
            rlt[4..8].copy_from_slice(&new_rlt_u32.to_le_bytes());
        }
        out.extend_from_slice(&rlt);
        let total_bntx_len = (out.len() - base_off) as u32;
        if base_off + rlt_hdr_field_off + 8 <= out.len() {
            out[base_off + rlt_hdr_field_off..base_off + rlt_hdr_field_off + 4]
                .copy_from_slice(&(new_rlt_rel as u32).to_le_bytes());
            out[base_off + rlt_hdr_field_off + 4..base_off + rlt_hdr_field_off + 8]
                .copy_from_slice(&total_bntx_len.to_le_bytes());
        }
    } else if let Some(b_off) = brtd_off {
        let brtd_size = (out.len() - b_off) as u64;
        if b_off + 16 <= out.len() {
            out[b_off + 8..b_off + 16].copy_from_slice(&brtd_size.to_le_bytes());
        }
        let total_bntx_len = (out.len() - base_off) as u32;
        if base_off + 0x20 <= out.len() {
            out[base_off + 0x1C..base_off + 0x20].copy_from_slice(&total_bntx_len.to_le_bytes());
        }
    }

    Ok(out)
}

/// Replace sub-texture `sub_index` inside a `.bntx` container from either a raw `.dds` (lossless GOB swizzle)
/// or an `RgbaImage` encoded with `target_format` (supporting both in-place and relocating format/size changes).
pub fn replace_bntx_subtexture(
    bntx_bytes: &[u8],
    sub_index: usize,
    dds_or_img: std::result::Result<&[u8], &RgbaImage>,
    target_format: Option<TextureFormat>,
    generate_mips: bool,
    apply_solidify: bool,
    make_opaque: bool,
) -> Result<Vec<u8>> {
    replace_bntx_subtexture_with_options(
        bntx_bytes,
        sub_index,
        dds_or_img,
        target_format,
        generate_mips,
        apply_solidify,
        make_opaque,
        EncodeQuality::Normal,
        None,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn replace_bntx_subtexture_with_options(
    bntx_bytes: &[u8],
    sub_index: usize,
    dds_or_img: std::result::Result<&[u8], &RgbaImage>,
    target_format: Option<TextureFormat>,
    generate_mips: bool,
    apply_solidify: bool,
    make_opaque: bool,
    quality: EncodeQuality,
    nvtt_dll_path: Option<&Path>,
) -> Result<Vec<u8>> {
    let recs = parse_bntx_textures(bntx_bytes)?;
    let tex = recs.get(sub_index).unwrap_or(&recs[0]);

    match dds_or_img {
        Ok(dds_bytes) => {
            let dds_meta = inspect_texture_blob(dds_bytes)?;
            let fmt = target_format.unwrap_or(dds_meta.format);
            let (bw, bh) = fmt.block_dimensions();
            let bpb = fmt.bytes_per_block();
            let hb0 = ((dds_meta.height as usize) + bh - 1) / bh;
            let bh_log2 = if dds_meta.height == tex.height && fmt.block_dimensions() == tex.format.block_dimensions() {
                tex.block_height_log2
            } else {
                gob_swizzle::block_height_log2(hb0)
            };
            let bh_mip0 = 1usize << bh_log2;

            let header_size = if dds_bytes.len() >= 148 && &dds_bytes[84..88] == b"DX10" {
                148usize
            } else {
                128usize
            };
            if dds_bytes.len() < header_size {
                return Err(PgError::InvalidData("Truncated DDS header".into()));
            }
            let dds_payload = &dds_bytes[header_size..];
            let mip_cnt = (dds_meta.mip_count as usize).min(tex.mip_offsets.len().max(1));
            let mut swizzled_mips = Vec::with_capacity(mip_cnt);
            let mut cursor = 0usize;
            for m in 0..mip_cnt {
                let mip_w = (dds_meta.width >> m).max(1) as usize;
                let mip_h = (dds_meta.height >> m).max(1) as usize;
                let wb = (mip_w + bw - 1) / bw;
                let hb = (mip_h + bh - 1) / bh;
                let mip_bh = gob_swizzle::mip_block_height(hb, bh_mip0);
                let pitch = align(wb * bpb, 64);
                let surf_size = pitch * align(hb, mip_bh * 8);
                let linear_len = wb * hb * bpb;
                let mut linear = vec![0u8; linear_len];
                if cursor < dds_payload.len() {
                    let avail = (dds_payload.len() - cursor).min(linear_len);
                    linear[..avail].copy_from_slice(&dds_payload[cursor..cursor + avail]);
                    cursor += avail;
                }
                let mut swizzled = vec![0u8; surf_size];
                gob_swizzle::swizzle_volume(&linear, &mut swizzled, wb, hb, 1, bpb, mip_bh, 1, pitch);
                swizzled_mips.push(swizzled);
            }
            rebuild_bntx_container_subtexture(
                bntx_bytes,
                sub_index,
                fmt,
                dds_meta.width,
                dds_meta.height,
                bh_log2,
                &swizzled_mips,
            )
        }
        Err(base_img) => {
            let fmt = target_format.unwrap_or(tex.format);
            let (bw, bh) = fmt.block_dimensions();
            let bpb = fmt.bytes_per_block();
            let mut processed = if apply_solidify {
                solidify_rgba_image(base_img, 0)
            } else {
                base_img.clone()
            };
            if make_opaque {
                for p in processed.pixels_mut() {
                    p.0[3] = 255;
                }
            }
            let max_mips = if generate_mips {
                tex.mip_offsets.len().max(1) as u32
            } else {
                1
            };
            let mip_images = generate_mip_chain(&processed, max_mips);
            let hb0 = ((processed.height() as usize) + bh - 1) / bh;
            let bh_log2 = if processed.height() == tex.height
                && fmt.block_dimensions() == tex.format.block_dimensions()
            {
                tex.block_height_log2
            } else {
                gob_swizzle::block_height_log2(hb0)
            };
            let bh_mip0 = 1usize << bh_log2;

            let mut swizzled_mips = Vec::with_capacity(mip_images.len());
            for mip_img in &mip_images {
                let wb = ((mip_img.width() as usize) + bw - 1) / bw;
                let hb = ((mip_img.height() as usize) + bh - 1) / bh;
                let mip_bh = gob_swizzle::mip_block_height(hb, bh_mip0);
                let pitch = align(wb * bpb, 64);
                let surf_size = pitch * align(hb, mip_bh * 8);
                let linear = encode_linear_surface_with_quality(mip_img, fmt, quality, nvtt_dll_path);
                let mut swizzled = vec![0u8; surf_size];
                gob_swizzle::swizzle_volume(&linear, &mut swizzled, wb, hb, 1, bpb, mip_bh, 1, pitch);
                swizzled_mips.push(swizzled);
            }
            rebuild_bntx_container_subtexture(
                bntx_bytes,
                sub_index,
                fmt,
                processed.width(),
                processed.height(),
                bh_log2,
                &swizzled_mips,
            )
        }
    }
}

/// Inspect a `.dds`, `.gtx`, `.xt1`, or `.bntx` texture blob and extract its metadata.
pub fn inspect_texture_blob(data: &[u8]) -> Result<TextureMetadata> {
    if data.len() < 16 {
        return Err(PgError::InvalidData("Texture blob shorter than 16 bytes".into()));
    }
    match &data[0..4] {
        b"DDS " => {
            let mut r = BinReader::new(data, Endian::Little);
            r.seek(12)?;
            let height = r.read_u32()?;
            let width = r.read_u32()?;
            r.seek(24)?;
            let depth = r.read_u32()?.max(1);
            let mip_count = r.read_u32()?.max(1);
            r.seek(80)?;
            let pf_flags = r.read_u32()?;
            let fourcc = r.read_bytes(4)?;
            let format = if (pf_flags & 0x4) != 0 {
                match fourcc {
                    b"DXT1" => TextureFormat::Bc1,
                    b"DXT3" => TextureFormat::Bc2,
                    b"DXT5" => TextureFormat::Bc3,
                    b"ATI1" | b"BC4U" | b"BC4S" => TextureFormat::Bc4,
                    b"ATI2" | b"BC5U" | b"BC5S" => TextureFormat::Bc5,
                    b"DX10" if data.len() >= 148 => {
                        let dxgi = u32::from_le_bytes(data[128..132].try_into().unwrap());
                        TextureFormat::from_dxgi_format(dxgi)
                    }
                    _ => TextureFormat::Bc3,
                }
            } else {
                let r_mask = r.read_u32().unwrap_or(0);
                let _bpp = r_mask;
                let r_bit_mask = r.read_u32().unwrap_or(0);
                if r_bit_mask == 0x00FF_0000 {
                    TextureFormat::Bgra8
                } else {
                    TextureFormat::Rgba8
                }
            };
            let caps2 = if data.len() >= 116 {
                u32::from_le_bytes(data[112..116].try_into().unwrap())
            } else {
                0
            };
            let (dx10_misc, dx10_array_size) = if data.len() >= 148 && &data[84..88] == b"DX10" {
                (
                    u32::from_le_bytes(data[136..140].try_into().unwrap()),
                    u32::from_le_bytes(data[140..144].try_into().unwrap()).max(1),
                )
            } else {
                (0, 1)
            };
            let is_cube = (caps2 & 0x200) != 0 || (dx10_misc & 0x4) != 0;
            let dimension = if is_cube {
                TextureDimension::TexCube
            } else if depth > 1 || dx10_array_size > 1 {
                TextureDimension::TexAnimatedLayers
            } else {
                TextureDimension::Tex2D
            };
            let layer_count = match dimension {
                TextureDimension::TexCube => 6,
                TextureDimension::TexAnimatedLayers => depth.max(dx10_array_size).max(1),
                TextureDimension::Tex2D => 1,
            };
            Ok(TextureMetadata {
                container_ext: ".dds".into(),
                format,
                dimension,
                width,
                height,
                depth,
                layer_count,
                mip_count,
                data_size: data.len(),
                name: None,
            })
        }
        b"Gfx2" => {
            if data.len() < 0x40 + 0x9C {
                return Err(PgError::InvalidData("Truncated GTX container".into()));
            }
            let mut r = BinReader::new(&data[0x40..0x40 + 0x9C], Endian::Big);
            let dim = r.read_u32()?;
            let width = r.read_u32()?;
            let height = r.read_u32()?;
            let depth = r.read_u32()?.max(1);
            let mip_count = r.read_u32()?.max(1);
            let gx2_fmt = r.read_u32()?;
            let format = gx2_addrlib::gx2_format_to_texture_format(gx2_fmt);
            let dimension = if dim == 3 || depth == 6 {
                TextureDimension::TexCube
            } else if depth > 1 {
                TextureDimension::TexAnimatedLayers
            } else {
                TextureDimension::Tex2D
            };
            let layer_count = match dimension {
                TextureDimension::TexCube => 6,
                TextureDimension::TexAnimatedLayers => depth,
                TextureDimension::Tex2D => 1,
            };
            Ok(TextureMetadata {
                container_ext: ".gtx".into(),
                format,
                dimension,
                width,
                height,
                depth,
                layer_count,
                mip_count,
                data_size: data.len(),
                name: None,
            })
        }
        b"XT1\0" => {
            if data.len() < 0x38 {
                return Err(PgError::InvalidData("Truncated XT1 header".into()));
            }
            let mut r = BinReader::new(data, Endian::Little);
            r.seek(0x14)?;
            let mip_word = r.read_u32()?;
            let mip_count = (mip_word & 0xFFFF).max(1);
            let tex_type = r.read_u32()?;
            let xt1_fmt = r.read_u32()?;
            let width = r.read_u32()?;
            let height = r.read_u32()?;
            let depth = r.read_u32()?.max(1);
            let format = TextureFormat::from_xt1_code(xt1_fmt);
            let dimension = if tex_type == 3 || tex_type == 8 {
                TextureDimension::TexCube
            } else if tex_type == 2 || tex_type == 5 || depth > 1 {
                TextureDimension::TexAnimatedLayers
            } else {
                TextureDimension::Tex2D
            };
            let layer_count = match dimension {
                TextureDimension::TexCube => 6,
                TextureDimension::TexAnimatedLayers => depth,
                TextureDimension::Tex2D => 1,
            };
            Ok(TextureMetadata {
                container_ext: ".xt1".into(),
                format,
                dimension,
                width,
                height,
                depth,
                layer_count,
                mip_count,
                data_size: data.len(),
                name: None,
            })
        }
        _ if (data.len() >= 4 && &data[0..4] == b"BNTX")
            || (data.len() >= 84 && &data[80..84] == b"BNTX") =>
        {
            let recs = parse_bntx_textures(data)?;
            let first = &recs[0];
            let dimension = if first.surface_dim == 3 || first.array_length == 6 {
                TextureDimension::TexCube
            } else if first.depth > 1 || first.array_length > 1 {
                TextureDimension::TexAnimatedLayers
            } else {
                TextureDimension::Tex2D
            };
            let layer_count = match dimension {
                TextureDimension::TexCube => 6,
                TextureDimension::TexAnimatedLayers => first.depth.max(first.array_length).max(1),
                TextureDimension::Tex2D => 1,
            };
            Ok(TextureMetadata {
                container_ext: ".bntx".into(),
                format: first.format,
                dimension,
                width: first.width,
                height: first.height,
                depth: first.depth,
                layer_count,
                mip_count: first.mip_count as u32,
                data_size: data.len(),
                name: Some(first.name.clone()),
            })
        }
        _ => Err(PgError::InvalidData("Unrecognized texture magic".into())),
    }
}

/// Convert a Wii U `.gtx` texture into a standard `.dds` file using exact `GX2.cs` AddrLib deswizzling.
pub fn convert_gtx_to_dds(gtx_bytes: &[u8]) -> Result<Vec<u8>> {
    if gtx_bytes.len() < 0x40 + 0x9C + 0x20 {
        return Err(PgError::InvalidData(
            "GTX file too small to contain surface payload".into(),
        ));
    }
    let mut r = BinReader::new(&gtx_bytes[0x40..0x40 + 0x9C], Endian::Big);
    let _dim = r.read_u32()?;
    let width = r.read_u32()?;
    let height = r.read_u32()?;
    let depth = r.read_u32()?.max(1);
    let num_mips = r.read_u32()?.max(1);
    let gx2_fmt = r.read_u32()?;
    let aa = r.read_u32()?;
    let use_flags = r.read_u32()?;
    let data_length = r.read_u32()? as usize;
    let _data_ptr = r.read_u32()?;
    let mip_length = r.read_u32()? as usize;
    let _mip_ptr = r.read_u32()?;
    let tile_mode = r.read_u32()?;
    let swizzle_val = r.read_u32()?;
    let _alignment = r.read_u32()?;
    let pitch = r.read_u32()?;
    let mut mip_offsets = [0u32; 13];
    for slot in &mut mip_offsets {
        *slot = r.read_u32()?;
    }

    let fmt = gx2_addrlib::gx2_format_to_texture_format(gx2_fmt);
    let base_start = 0x40 + 0x9C + 0x20;
    let base_end = (base_start + data_length).min(gtx_bytes.len());
    let mip_blk_start = base_end + 0x20;
    let export_mips = if mip_length == 0 || mip_blk_start >= gtx_bytes.len() {
        1
    } else {
        num_mips.min(14)
    };

    let mut dds_payload = Vec::new();
    for m in 0..export_mips {
        let mip_w = (width >> m).max(1);
        let mip_h = (height >> m).max(1);
        let (swizzled_slice, mip_pitch) = if m == 0 {
            (&gtx_bytes[base_start..base_end], pitch)
        } else {
            let rel_off = if m == 1 {
                0usize
            } else {
                (mip_offsets[(m - 1) as usize] as usize).saturating_sub(data_length)
            };
            let start = (mip_blk_start + rel_off).min(gtx_bytes.len());
            (&gtx_bytes[start..], (pitch >> m).max(32))
        };
        let linear = gx2_addrlib::deswizzle_gx2_surface(
            mip_w,
            mip_h,
            depth,
            gx2_fmt,
            aa,
            use_flags,
            tile_mode,
            swizzle_val,
            mip_pitch,
            0,
            swizzled_slice,
        );
        dds_payload.extend_from_slice(&linear);
    }

    Ok(build_dds_file(width, height, export_mips, fmt, &dds_payload))
}

/// Decode linear block/pixel bytes of known `width`, `height`, and `TextureFormat` into an `RgbaImage`.
pub fn decode_linear_blocks_to_rgba8(
    linear: &[u8],
    width: u32,
    height: u32,
    format: TextureFormat,
) -> Result<RgbaImage> {
    let w = width.max(1);
    let h = height.max(1);
    let mut img: RgbaImage = ImageBuffer::new(w, h);
    let blocks_x = ((w as usize) + 3) / 4;
    let blocks_y = ((h as usize) + 3) / 4;

    match format {
        TextureFormat::AfL8Unorm | TextureFormat::R8Unorm => {
            for y in 0..h {
                for x in 0..w {
                    let idx = (y * w + x) as usize;
                    if idx < linear.len() {
                        let v = linear[idx];
                        img.put_pixel(x, y, image::Rgba([v, v, v, 255]));
                    }
                }
            }
        }
        TextureFormat::Rg8Unorm => {
            for y in 0..h {
                for x in 0..w {
                    let idx = ((y * w + x) * 2) as usize;
                    if idx + 2 <= linear.len() {
                        img.put_pixel(x, y, image::Rgba([linear[idx], linear[idx + 1], 0, 255]));
                    }
                }
            }
        }
        TextureFormat::Rgba8 | TextureFormat::Rgba8Srgb => {
            for y in 0..h {
                for x in 0..w {
                    let idx = ((y * w + x) * 4) as usize;
                    if idx + 4 <= linear.len() {
                        img.put_pixel(
                            x,
                            y,
                            image::Rgba([
                                linear[idx],
                                linear[idx + 1],
                                linear[idx + 2],
                                linear[idx + 3],
                            ]),
                        );
                    }
                }
            }
        }
        TextureFormat::Bgra8 | TextureFormat::Bgra8Srgb => {
            for y in 0..h {
                for x in 0..w {
                    let idx = ((y * w + x) * 4) as usize;
                    if idx + 4 <= linear.len() {
                        img.put_pixel(
                            x,
                            y,
                            image::Rgba([
                                linear[idx + 2],
                                linear[idx + 1],
                                linear[idx],
                                linear[idx + 3],
                            ]),
                        );
                    }
                }
            }
        }
        TextureFormat::Rgba16Float => {
            for y in 0..h {
                for x in 0..w {
                    let idx = ((y * w + x) * 8) as usize;
                    if idx + 8 <= linear.len() {
                        let r = F16(u16::from_le_bytes([linear[idx], linear[idx + 1]])).to_f32();
                        let g =
                            F16(u16::from_le_bytes([linear[idx + 2], linear[idx + 3]])).to_f32();
                        let b =
                            F16(u16::from_le_bytes([linear[idx + 4], linear[idx + 5]])).to_f32();
                        let a =
                            F16(u16::from_le_bytes([linear[idx + 6], linear[idx + 7]])).to_f32();
                        let tm = |v: f32| -> u8 {
                            if v.is_nan() || v <= 0.0 {
                                0
                            } else {
                                ((v / (1.0 + v)) * 255.0).round().clamp(0.0, 255.0) as u8
                            }
                        };
                        img.put_pixel(
                            x,
                            y,
                            image::Rgba([
                                tm(r),
                                tm(g),
                                tm(b),
                                (a * 255.0).round().clamp(0.0, 255.0) as u8,
                            ]),
                        );
                    }
                }
            }
        }
        TextureFormat::Bc1 | TextureFormat::Bc1Srgb => {
            for by in 0..blocks_y {
                for bx in 0..blocks_x {
                    let off = (by * blocks_x + bx) * 8;
                    if off + 8 <= linear.len() {
                        let block = decode_bc1_block(&linear[off..off + 8]);
                        blit_4x4(&mut img, bx * 4, by * 4, &block);
                    }
                }
            }
        }
        TextureFormat::Bc2 | TextureFormat::Bc2Srgb => {
            for by in 0..blocks_y {
                for bx in 0..blocks_x {
                    let off = (by * blocks_x + bx) * 16;
                    if off + 16 <= linear.len() {
                        let mut block = decode_bc1_block(&linear[off + 8..off + 16]);
                        let alpha_bits =
                            u64::from_le_bytes(linear[off..off + 8].try_into().unwrap());
                        for i in 0..16 {
                            let a4 = ((alpha_bits >> (i * 4)) & 0xF) as u8;
                            block[i][3] = a4 | (a4 << 4);
                        }
                        blit_4x4(&mut img, bx * 4, by * 4, &block);
                    }
                }
            }
        }
        TextureFormat::Bc4 => {
            for by in 0..blocks_y {
                for bx in 0..blocks_x {
                    let off = (by * blocks_x + bx) * 8;
                    if off + 8 <= linear.len() {
                        let reds = decode_bc4_block(&linear[off..off + 8]);
                        let mut block = [[0u8; 4]; 16];
                        for i in 0..16 {
                            block[i] = [reds[i], reds[i], reds[i], 255];
                        }
                        blit_4x4(&mut img, bx * 4, by * 4, &block);
                    }
                }
            }
        }
        TextureFormat::Bc5 => {
            for by in 0..blocks_y {
                for bx in 0..blocks_x {
                    let off = (by * blocks_x + bx) * 16;
                    if off + 16 <= linear.len() {
                        let reds = decode_bc4_block(&linear[off..off + 8]);
                        let greens = decode_bc4_block(&linear[off + 8..off + 16]);
                        let mut block = [[0u8; 4]; 16];
                        for i in 0..16 {
                            let nx = (reds[i] as f32 / 255.0) * 2.0 - 1.0;
                            let ny = (greens[i] as f32 / 255.0) * 2.0 - 1.0;
                            let nz = (1.0 - (nx * nx + ny * ny)).max(0.0).sqrt();
                            let b = (((nz + 1.0) * 0.5) * 255.0).clamp(0.0, 255.0) as u8;
                            block[i] = [reds[i], greens[i], b, 255];
                        }
                        blit_4x4(&mut img, bx * 4, by * 4, &block);
                    }
                }
            }
        }
        TextureFormat::Astc(bw, bh, _) => {
            return Ok(decode_astc_linear_to_rgba8(
                linear,
                w,
                h,
                bw as usize,
                bh as usize,
            ));
        }
        TextureFormat::Bc7 | TextureFormat::Bc7Srgb => {
            for by in 0..blocks_y {
                for bx in 0..blocks_x {
                    let off = (by * blocks_x + bx) * 16;
                    if off + 16 <= linear.len() {
                        let block = decode_bc7_block(&linear[off..off + 16]);
                        blit_4x4(&mut img, bx * 4, by * 4, &block);
                    }
                }
            }
        }
        TextureFormat::Bc6hUf16 | TextureFormat::Bc6hSf16 => {
            let signed = format == TextureFormat::Bc6hSf16;
            for by in 0..blocks_y {
                for bx in 0..blocks_x {
                    let off = (by * blocks_x + bx) * 16;
                    if off + 16 <= linear.len() {
                        let block = decode_bc6h_block(&linear[off..off + 16], signed);
                        blit_4x4(&mut img, bx * 4, by * 4, &block);
                    }
                }
            }
        }
        _ => {
            // BC3 / fallback
            for by in 0..blocks_y {
                for bx in 0..blocks_x {
                    let off = (by * blocks_x + bx) * 16;
                    if off + 16 <= linear.len() {
                        let mut block = decode_bc1_block(&linear[off + 8..off + 16]);
                        let alphas = decode_bc4_block(&linear[off..off + 8]);
                        for i in 0..16 {
                            block[i][3] = alphas[i];
                        }
                        blit_4x4(&mut img, bx * 4, by * 4, &block);
                    }
                }
            }
        }
    }

    Ok(img)
}

/// Decode a `.dds`, `.gtx`, `.xt1`, or `.bntx` texture blob at `mip_index` with optional channel
/// isolation (`"RGBA"`, `"RGB"`, `"R"`, `"G"`, `"B"`, `"A"`) and `opaque` override.
pub fn decode_texture_mip_filtered(
    blob: &[u8],
    mip_index: u32,
    channel: &str,
    opaque: bool,
) -> Result<(RgbaImage, u32, u32)> {
    if blob.len() < 16 {
        return Err(PgError::InvalidData("Texture blob too short".into()));
    }

    let (mut img, w, h) = if &blob[0..4] == b"XT1\0" {
        if blob.len() < 0x38 {
            return Err(PgError::InvalidData("Truncated XT1 header".into()));
        }
        let mut r = BinReader::new(blob, Endian::Little);
        r.seek(0x08)?;
        let stored_image_size = r.read_u64()?;
        let header_size = r.read_u32()? as usize;
        let mip_word = r.read_u32()?;
        let mip_count = (mip_word & 0xFFFF).max(1);
        let tex_type = r.read_u32()?;
        let xt1_fmt = r.read_u32()?;
        let width = r.read_u32()?;
        let height = r.read_u32()?;
        let depth = r.read_u32()?.max(1);
        let _tex_flag = r.read_u32()?;
        let layout_word = r.read_u32()?;
        let format = TextureFormat::from_xt1_code(xt1_fmt);

        let layout = compute_xt1_layout(
            width,
            height,
            depth,
            mip_count,
            tex_type,
            layout_word,
            format,
            stored_image_size,
        );
        let clamped_mip = (mip_index as usize).min(layout.mips.len().saturating_sub(1));
        let mip_info = &layout.mips[clamped_mip];
        let payload = if header_size <= blob.len() {
            &blob[header_size..]
        } else {
            &[]
        };

        let bpb = format.bytes_per_block();
        let mut linear = vec![0u8; mip_info.width_blocks * mip_info.height_blocks * bpb];
        if mip_info.offset < payload.len() {
            let avail = (payload.len() - mip_info.offset).min(mip_info.slice_surface_size);
            let mut surface = vec![0u8; mip_info.slice_surface_size];
            surface[..avail].copy_from_slice(&payload[mip_info.offset..mip_info.offset + avail]);
            gob_swizzle::deswizzle_volume(
                &surface,
                &mut linear,
                mip_info.width_blocks,
                mip_info.height_blocks,
                1,
                bpb,
                mip_info.block_height,
                1,
                mip_info.pitch,
            );
        }
        let decoded = decode_linear_blocks_to_rgba8(&linear, mip_info.width, mip_info.height, format)?;
        (decoded, mip_info.width, mip_info.height)
    } else if &blob[0..4] == b"Gfx2" {
        if blob.len() < 0x40 + 0x9C + 0x20 {
            return Err(PgError::InvalidData("Truncated GTX container".into()));
        }
        let mut r = BinReader::new(&blob[0x40..0x40 + 0x9C], Endian::Big);
        let _dim = r.read_u32()?;
        let width = r.read_u32()?;
        let height = r.read_u32()?;
        let depth = r.read_u32()?.max(1);
        let num_mips = r.read_u32()?.max(1);
        let gx2_fmt = r.read_u32()?;
        let aa = r.read_u32()?;
        let use_flags = r.read_u32()?;
        let data_length = r.read_u32()? as usize;
        let _data_ptr = r.read_u32()?;
        let mip_length = r.read_u32()? as usize;
        let _mip_ptr = r.read_u32()?;
        let tile_mode = r.read_u32()?;
        let swizzle_val = r.read_u32()?;
        let _alignment = r.read_u32()?;
        let pitch = r.read_u32()?;
        let mut mip_offsets = [0u32; 13];
        for slot in &mut mip_offsets {
            *slot = r.read_u32()?;
        }

        let format = gx2_addrlib::gx2_format_to_texture_format(gx2_fmt);
        let clamped_mip = mip_index.min(num_mips.saturating_sub(1));
        let mip_w = (width >> clamped_mip).max(1);
        let mip_h = (height >> clamped_mip).max(1);

        let base_start = 0x40 + 0x9C + 0x20;
        let base_end = (base_start + data_length).min(blob.len());
        let (swizzled_slice, mip_pitch) = if clamped_mip == 0 || mip_length == 0 {
            (&blob[base_start..base_end], pitch)
        } else {
            let mip_blk_start = base_end + 0x20;
            if mip_blk_start < blob.len() {
                let rel_off = if clamped_mip == 1 {
                    0usize
                } else {
                    (mip_offsets[(clamped_mip - 1) as usize] as usize)
                        .saturating_sub(data_length)
                };
                let start = (mip_blk_start + rel_off).min(blob.len());
                (&blob[start..], (pitch >> clamped_mip).max(32))
            } else {
                (&blob[base_start..base_end], pitch)
            }
        };

        let linear = gx2_addrlib::deswizzle_gx2_surface(
            mip_w,
            mip_h,
            depth,
            gx2_fmt,
            aa,
            use_flags,
            tile_mode,
            swizzle_val,
            mip_pitch,
            0,
            swizzled_slice,
        );
        let decoded = decode_linear_blocks_to_rgba8(&linear, mip_w, mip_h, format)?;
        (decoded, mip_w, mip_h)
    } else if &blob[0..4] == b"BNTX" || (blob.len() >= 84 && &blob[80..84] == b"BNTX") {
        let recs = parse_bntx_textures(blob)?;
        let tex = &recs[0];
        let clamped_mip = (mip_index as usize).min(tex.mip_offsets.len().saturating_sub(1));
        let mip_w = (tex.width >> clamped_mip).max(1);
        let mip_h = (tex.height >> clamped_mip).max(1);
        let (bw, bh) = tex.format.block_dimensions();
        let bpb = tex.format.bytes_per_block();
        let wb = ((mip_w as usize) + bw - 1) / bw;
        let hb = ((mip_h as usize) + bh - 1) / bh;
        let bh_mip0 = 1usize << tex.block_height_log2;
        let mip_bh = gob_swizzle::mip_block_height(hb, bh_mip0);
        let pitch = align(wb * bpb, 64);
        let surf_size = pitch * align(hb, mip_bh * 8);

        let mut linear = vec![0u8; wb * hb * bpb];
        if let Some(&mip_off) = tex.mip_offsets.get(clamped_mip) {
            if mip_off < blob.len() {
                let avail = (blob.len() - mip_off).min(surf_size);
                let mut surface = vec![0u8; surf_size];
                surface[..avail].copy_from_slice(&blob[mip_off..mip_off + avail]);
                gob_swizzle::deswizzle_volume(
                    &surface,
                    &mut linear,
                    wb,
                    hb,
                    1,
                    bpb,
                    mip_bh,
                    1,
                    pitch,
                );
            }
        }
        let decoded = decode_linear_blocks_to_rgba8(&linear, mip_w, mip_h, tex.format)?;
        (decoded, mip_w, mip_h)
    } else {
        // Standard DDS
        let meta = inspect_texture_blob(blob)?;
        let header_size = if blob.len() >= 148 && &blob[84..88] == b"DX10" {
            148
        } else {
            128
        };
        if blob.len() < header_size {
            return Err(PgError::InvalidData("DDS shorter than header".into()));
        }
        let payload = &blob[header_size..];
        let clamped_mip = mip_index.min(meta.mip_count.saturating_sub(1));
        let mut mip_offset = 0usize;
        let mut cur_w = meta.width.max(1);
        let mut cur_h = meta.height.max(1);
        let (bw, bh) = meta.format.block_dimensions();
        let bpb = meta.format.bytes_per_block();

        for _ in 0..clamped_mip {
            let bx = ((cur_w as usize) + bw - 1) / bw;
            let by = ((cur_h as usize) + bh - 1) / bh;
            let mip_bytes = bx.max(1) * by.max(1) * bpb;
            if mip_offset + mip_bytes <= payload.len() {
                mip_offset += mip_bytes;
                cur_w = (cur_w / 2).max(1);
                cur_h = (cur_h / 2).max(1);
            } else {
                break;
            }
        }
        let decoded =
            decode_linear_blocks_to_rgba8(&payload[mip_offset..], cur_w, cur_h, meta.format)?;
        (decoded, cur_w, cur_h)
    };

    if channel != "RGBA" || opaque {
        for p in img.pixels_mut() {
            let [r, g, b, a] = p.0;
            p.0 = match channel {
                "RGB" => [r, g, b, 255],
                "R" => [r, r, r, 255],
                "G" => [g, g, g, 255],
                "B" => [b, b, b, 255],
                "A" => [a, a, a, 255],
                _ => [r, g, b, if opaque { 255 } else { a }],
            };
        }
    }

    Ok((img, w, h))
}

/// Decode a specific `BRTI` sub-texture (`sub_index`) inside a `.bntx` container at `mip_index` and `layer_index`.
pub fn decode_bntx_subtexture_layer_mip_filtered(
    blob: &[u8],
    sub_index: usize,
    mip_index: u32,
    layer_index: u32,
    channel: &str,
    opaque: bool,
) -> Result<(RgbaImage, u32, u32, u32)> {
    let recs = parse_bntx_textures(blob)?;
    let tex = recs.get(sub_index).unwrap_or(&recs[0]);
    let layer_count = if tex.surface_dim == 3 || tex.array_length == 6 {
        6u32
    } else {
        tex.depth.max(tex.array_length).max(1)
    };
    let clamped_layer = (layer_index as usize).min((layer_count as usize).saturating_sub(1));
    let layer_stride = if layer_count > 1 && (tex.image_size as usize) >= (layer_count as usize) {
        (tex.image_size as usize) / (layer_count as usize)
    } else {
        0
    };

    let clamped_mip = (mip_index as usize).min(tex.mip_offsets.len().saturating_sub(1));
    let mip_w = (tex.width >> clamped_mip).max(1);
    let mip_h = (tex.height >> clamped_mip).max(1);
    let (bw, bh) = tex.format.block_dimensions();
    let bpb = tex.format.bytes_per_block();
    let wb = ((mip_w as usize) + bw - 1) / bw;
    let hb = ((mip_h as usize) + bh - 1) / bh;
    let bh_mip0 = 1usize << tex.block_height_log2;
    let mip_bh = gob_swizzle::mip_block_height(hb, bh_mip0);
    let pitch = align(wb * bpb, 64);
    let surf_size = pitch * align(hb, mip_bh * 8);

    let mut linear = vec![0u8; wb * hb * bpb];
    if let Some(&mip_off) = tex.mip_offsets.get(clamped_mip) {
        let slice_off = mip_off + clamped_layer * layer_stride;
        if slice_off < blob.len() {
            let avail = (blob.len() - slice_off).min(surf_size);
            let mut surface = vec![0u8; surf_size];
            surface[..avail].copy_from_slice(&blob[slice_off..slice_off + avail]);
            gob_swizzle::deswizzle_volume(
                &surface,
                &mut linear,
                wb,
                hb,
                1,
                bpb,
                mip_bh,
                1,
                pitch,
            );
        }
    }
    let mut img = decode_linear_blocks_to_rgba8(&linear, mip_w, mip_h, tex.format)?;
    apply_channel_and_opaque_filter(&mut img, channel, opaque);
    Ok((img, mip_w, mip_h, layer_count))
}

/// Decode a specific `BRTI` sub-texture (`sub_index`) inside a `.bntx` container at `mip_index`.
pub fn decode_bntx_subtexture_mip_filtered(
    blob: &[u8],
    sub_index: usize,
    mip_index: u32,
    channel: &str,
    opaque: bool,
) -> Result<(RgbaImage, u32, u32)> {
    let (img, w, h, _) =
        decode_bntx_subtexture_layer_mip_filtered(blob, sub_index, mip_index, 0, channel, opaque)?;
    Ok((img, w, h))
}

/// Decode base mip 0 of a texture blob into an `RgbaImage`.
pub fn decode_dds_to_rgba8(blob: &[u8]) -> Result<RgbaImage> {
    let (img, _, _) = decode_texture_mip_filtered(blob, 0, "RGBA", false)?;
    Ok(img)
}

/// Encode an `RgbaImage` into a `.dds` file (defaulting to BC3 or RGBA8, with optional solidify & mipmap chain).
pub fn encode_rgba8_to_dds(img: &RgbaImage) -> Vec<u8> {
    let (w, h) = img.dimensions();
    build_dds_file(w, h, 1, TextureFormat::Rgba8, img.as_raw())
}

/// Create an 8x8 solid dummy `.dds` or `.xt1` texture (`"white"`, `"black"`, or `"normal"` `(128, 128, 255)`).
pub fn build_dummy_dds(kind: &str) -> Vec<u8> {
    let rgba = match kind {
        "black" => [0u8, 0, 0, 255],
        "normal" => [128u8, 128, 255, 255],
        _ => [255u8, 255, 255, 255],
    };
    let mut img = RgbaImage::new(8, 8);
    for p in img.pixels_mut() {
        p.0 = rgba;
    }
    encode_rgba8_to_dds(&img)
}

fn blit_4x4(img: &mut RgbaImage, base_x: usize, base_y: usize, block: &[[u8; 4]; 16]) {
    let (w, h) = img.dimensions();
    for py in 0..4 {
        for px in 0..4 {
            let x = (base_x + px) as u32;
            let y = (base_y + py) as u32;
            if x < w && y < h {
                img.put_pixel(x, y, image::Rgba(block[py * 4 + px]));
            }
        }
    }
}

fn rgb565_to_rgb888(c: u16) -> [u8; 3] {
    let r = (((c >> 11) & 0x1F) * 255 + 15) / 31;
    let g = (((c >> 5) & 0x3F) * 255 + 31) / 63;
    let b = ((c & 0x1F) * 255 + 15) / 31;
    [r as u8, g as u8, b as u8]
}

fn decode_bc1_block(b: &[u8]) -> [[u8; 4]; 16] {
    let c0 = u16::from_le_bytes([b[0], b[1]]);
    let c1 = u16::from_le_bytes([b[2], b[3]]);
    let idx_bits = u32::from_le_bytes([b[4], b[5], b[6], b[7]]);

    let rgb0 = rgb565_to_rgb888(c0);
    let rgb1 = rgb565_to_rgb888(c1);
    let mut palette = [[0u8; 4]; 4];
    palette[0] = [rgb0[0], rgb0[1], rgb0[2], 255];
    palette[1] = [rgb1[0], rgb1[1], rgb1[2], 255];

    if c0 > c1 {
        for ch in 0..3 {
            palette[2][ch] = ((2 * (rgb0[ch] as u16) + (rgb1[ch] as u16)) / 3) as u8;
            palette[3][ch] = (((rgb0[ch] as u16) + 2 * (rgb1[ch] as u16)) / 3) as u8;
        }
        palette[2][3] = 255;
        palette[3][3] = 255;
    } else {
        for ch in 0..3 {
            palette[2][ch] = (((rgb0[ch] as u16) + (rgb1[ch] as u16)) / 2) as u8;
        }
        palette[2][3] = 255;
        palette[3] = [0, 0, 0, 0];
    }

    let mut out = [[0u8; 4]; 16];
    for i in 0..16 {
        let code = ((idx_bits >> (i * 2)) & 0x3) as usize;
        out[i] = palette[code];
    }
    out
}

fn decode_bc4_block(b: &[u8]) -> [u8; 16] {
    let a0 = b[0];
    let a1 = b[1];
    let mut lut = [0u8; 8];
    lut[0] = a0;
    lut[1] = a1;
    if a0 > a1 {
        for i in 1..7 {
            lut[i + 1] = (((7 - i) as u16 * (a0 as u16) + (i as u16) * (a1 as u16)) / 7) as u8;
        }
    } else {
        for i in 1..5 {
            lut[i + 1] = (((5 - i) as u16 * (a0 as u16) + (i as u16) * (a1 as u16)) / 5) as u8;
        }
        lut[6] = 0;
        lut[7] = 255;
    }

    let mut bits: u64 = 0;
    for i in 0..6 {
        bits |= (b[2 + i] as u64) << (8 * i);
    }
    let mut out = [0u8; 16];
    for i in 0..16 {
        let idx = ((bits >> (i * 3)) & 0x7) as usize;
        out[i] = lut[idx];
    }
    out
}


// ============================================================================
// QUALITY-AWARE ENCODERS, LOSSLESS DDS EXPORTERS, CUBEMAP & LAYER DECODERS,
// AND BATCH ASTC -> BC RECOMPRESSOR (XT1 & BNTX)
// ============================================================================

#[allow(clippy::too_many_arguments)]
pub fn encode_rgba_to_xt1_with_options(
    base_img: &RgbaImage,
    target_format: TextureFormat,
    mip_count_override: Option<u32>,
    generate_mips: bool,
    apply_solidify: bool,
    make_opaque: bool,
    quality: EncodeQuality,
    template_header: Option<&[u8]>,
    nvtt_dll_path: Option<&Path>,
) -> Vec<u8> {
    let mut processed = if apply_solidify {
        solidify_rgba_image(base_img, 0)
    } else {
        base_img.clone()
    };
    if make_opaque {
        for p in processed.pixels_mut() {
            p.0[3] = 255;
        }
    }

    let max_mips = if let Some(explicit_mips) = mip_count_override {
        explicit_mips.max(1)
    } else if generate_mips {
        (processed.width().max(processed.height()) as f32)
            .log2()
            .floor() as u32
            + 1
    } else {
        1
    };
    let mip_images = generate_mip_chain(&processed, max_mips);
    encode_mip_levels_to_xt1(&mip_images, target_format, quality, template_header, nvtt_dll_path)
}

/// Encode an explicit slice of pre-existing mip `RgbaImage` levels into a `.xt1` blob.
/// Used by both `encode_rgba_to_xt1_with_options` and `BatchRecompressor` (which reuses existing mip levels).
pub fn encode_mip_levels_to_xt1(
    mip_images: &[RgbaImage],
    target_format: TextureFormat,
    quality: EncodeQuality,
    template_header: Option<&[u8]>,
    nvtt_dll_path: Option<&Path>,
) -> Vec<u8> {
    let base = &mip_images[0];
    let mip_count = mip_images.len().max(1) as u32;
    let enc_format = target_format;

    let (_, bh) = enc_format.block_dimensions();
    let hb0 = ((base.height() as usize) + bh - 1) / bh;
    let blk_log2 = gob_swizzle::block_height_log2(hb0);

    let layout_res = compute_xt1_layout(
        base.width(),
        base.height(),
        1,
        mip_count,
        1,
        blk_log2,
        enc_format,
        0,
    );

    let bpb = enc_format.bytes_per_block();
    let mut payload = vec![0u8; layout_res.computed_image_size];
    for (m, mip_info) in layout_res.mips.iter().enumerate() {
        let linear = encode_linear_surface_with_quality(&mip_images[m], enc_format, quality, nvtt_dll_path);
        let mut swizzled = vec![0u8; mip_info.slice_surface_size];
        gob_swizzle::swizzle_volume(
            &linear,
            &mut swizzled,
            mip_info.width_blocks,
            mip_info.height_blocks,
            1,
            bpb,
            mip_info.block_height,
            1,
            mip_info.pitch,
        );
        if mip_info.offset + swizzled.len() <= payload.len() {
            payload[mip_info.offset..mip_info.offset + swizzled.len()].copy_from_slice(&swizzled);
        }
    }

    let mut hdr = [0u8; 0x38];
    if let Some(tpl) = template_header {
        if tpl.len() >= 0x38 {
            hdr.copy_from_slice(&tpl[0..0x38]);
        }
    } else {
        hdr[0..4].copy_from_slice(b"XT1\0");
        hdr[4..8].copy_from_slice(&0x01000101u32.to_le_bytes());
        hdr[0x34..0x38].copy_from_slice(&0x00010007u32.to_le_bytes());
    }
    hdr[0..4].copy_from_slice(b"XT1\0");
    hdr[0x08..0x10].copy_from_slice(&(payload.len() as u64).to_le_bytes());
    hdr[0x10..0x14].copy_from_slice(&0x38u32.to_le_bytes());
    let codec_tag: u16 = if enc_format == TextureFormat::AfL8Unorm {
        0x6661
    } else {
        0
    };
    let mip_word = (mip_count & 0xFFFF) | ((codec_tag as u32) << 16);
    hdr[0x14..0x18].copy_from_slice(&mip_word.to_le_bytes());
    hdr[0x18..0x1C].copy_from_slice(&1u32.to_le_bytes()); // T_2D
    hdr[0x1C..0x20].copy_from_slice(&enc_format.to_xt1_code().to_le_bytes());
    hdr[0x20..0x24].copy_from_slice(&base.width().to_le_bytes());
    hdr[0x24..0x28].copy_from_slice(&base.height().to_le_bytes());
    hdr[0x28..0x2C].copy_from_slice(&1u32.to_le_bytes());
    let prev_layout = u32::from_le_bytes(hdr[0x30..0x34].try_into().unwrap());
    let new_layout = (prev_layout & !0x407u32) | blk_log2;
    hdr[0x30..0x34].copy_from_slice(&new_layout.to_le_bytes());

    let mut full = Vec::with_capacity(0x38 + payload.len());
    full.extend_from_slice(&hdr);
    full.extend_from_slice(&payload);
    full
}

pub fn encode_rgba_to_dds_with_quality(
    base_img: &RgbaImage,
    target_format: TextureFormat,
    mip_count_override: Option<u32>,
    generate_mips: bool,
    apply_solidify: bool,
    make_opaque: bool,
    quality: EncodeQuality,
    nvtt_dll_path: Option<&Path>,
) -> Vec<u8> {
    let mut processed = if apply_solidify {
        solidify_rgba_image(base_img, 0)
    } else {
        base_img.clone()
    };
    if make_opaque {
        for p in processed.pixels_mut() {
            p.0[3] = 255;
        }
    }

    let max_mips = if let Some(explicit_mips) = mip_count_override {
        explicit_mips.max(1)
    } else if generate_mips {
        (processed.width().max(processed.height()) as f32)
            .log2()
            .floor() as u32
            + 1
    } else {
        1
    };
    let mip_images = generate_mip_chain(&processed, max_mips);
    let mip_count = mip_images.len() as u32;

    let mut payload = Vec::new();
    for mip_img in &mip_images {
        payload.extend_from_slice(&encode_linear_surface_with_quality(
            mip_img,
            target_format,
            quality,
            nvtt_dll_path,
        ));
    }
    build_dds_file(
        processed.width(),
        processed.height(),
        mip_count,
        target_format,
        &payload,
    )
}

/// Losslessly deswizzle an `.xt1` texture (all mip levels, and all 6 faces for cubemaps) into a
/// DX9/DX10 `.dds` file without decoding or re-encoding blocks (matching `XT1 Studio`'s `DdsExport.Build`).
pub fn deswizzle_xt1_to_dds(xt1_bytes: &[u8]) -> Result<Vec<u8>> {
    if xt1_bytes.len() < 0x38 || &xt1_bytes[0..4] != b"XT1\0" {
        return Err(PgError::InvalidData("Invalid XT1 header".into()));
    }
    let mut r = BinReader::new(xt1_bytes, Endian::Little);
    r.seek(0x08)?;
    let stored_image_size = r.read_u64()?;
    let header_size = r.read_u32()? as usize;
    let mip_word = r.read_u32()?;
    let mip_count = (mip_word & 0xFFFF).max(1);
    let tex_type = r.read_u32()?;
    let xt1_fmt = r.read_u32()?;
    let width = r.read_u32()?;
    let height = r.read_u32()?;
    let depth = r.read_u32()?.max(1);
    let _tex_flag = r.read_u32()?;
    let layout_word = r.read_u32()?;
    let format = TextureFormat::from_xt1_code(xt1_fmt);
    if matches!(format, TextureFormat::Unknown(_)) {
        return Err(PgError::InvalidData(format!("Unsupported XT1 format 0x{xt1_fmt:X} for DDS export")));
    }

    let layout = compute_xt1_layout(
        width,
        height,
        depth,
        mip_count,
        tex_type,
        layout_word,
        format,
        stored_image_size,
    );
    let payload = if header_size <= xt1_bytes.len() {
        &xt1_bytes[header_size..]
    } else {
        &[]
    };
    let bpb = format.bytes_per_block();
    let is_cube = tex_type == 3 || tex_type == 8;
    let is_3d = tex_type == 2;
    let face_count = if is_cube { 6 } else { 1 };

    let mut dds_payload = Vec::new();
    for face in 0..face_count {
        let face_base = if is_cube { face * layout.face_stride } else { 0 };
        for mip_info in &layout.mips {
            let linear_len = mip_info.width_blocks
                * mip_info.height_blocks
                * (if is_3d { mip_info.slice_count } else { 1 })
                * bpb;
            let mut linear = vec![0u8; linear_len];
            let mip_off = face_base + mip_info.offset;
            if is_3d && mip_info.block_depth > 1 {
                let total_surf = mip_info.slice_surface_size * mip_info.slice_count;
                let mut surface = vec![0u8; total_surf];
                if mip_off < payload.len() {
                    let avail = (payload.len() - mip_off).min(total_surf);
                    surface[..avail].copy_from_slice(&payload[mip_off..mip_off + avail]);
                }
                gob_swizzle::deswizzle_volume(
                    &surface,
                    &mut linear,
                    mip_info.width_blocks,
                    mip_info.height_blocks,
                    mip_info.slice_count,
                    bpb,
                    mip_info.block_height,
                    mip_info.block_depth,
                    mip_info.pitch,
                );
            } else {
                let slice_cnt = if is_3d { mip_info.slice_count } else { 1 };
                let single_linear = mip_info.width_blocks * mip_info.height_blocks * bpb;
                for s in 0..slice_cnt {
                    let s_off = mip_off + s * mip_info.slice_surface_size;
                    let mut surface = vec![0u8; mip_info.slice_surface_size];
                    if s_off < payload.len() {
                        let avail = (payload.len() - s_off).min(mip_info.slice_surface_size);
                        surface[..avail].copy_from_slice(&payload[s_off..s_off + avail]);
                    }
                    gob_swizzle::deswizzle_volume(
                        &surface,
                        &mut linear[s * single_linear..(s + 1) * single_linear],
                        mip_info.width_blocks,
                        mip_info.height_blocks,
                        1,
                        bpb,
                        mip_info.block_height,
                        1,
                        mip_info.pitch,
                    );
                }
            }
            dds_payload.extend_from_slice(&linear);
        }
    }

    Ok(build_dds_file_ex(
        width,
        height,
        if is_3d { depth } else { 1 },
        layout.mips.len() as u32,
        is_cube,
        format,
        &dds_payload,
    ))
}

/// Losslessly deswizzle sub-texture `sub_index` from a Switch `.bntx` container into a multi-mip `.dds`.
pub fn deswizzle_bntx_to_dds(bntx_bytes: &[u8], sub_index: usize) -> Result<Vec<u8>> {
    let recs = parse_bntx_textures(bntx_bytes)?;
    let tex = recs.get(sub_index).unwrap_or(&recs[0]);
    let (bw, bh) = tex.format.block_dimensions();
    let bpb = tex.format.bytes_per_block();
    let bh_mip0 = 1usize << tex.block_height_log2;
    let is_cube = tex.surface_dim == 3 || tex.array_length == 6;
    let face_count = if is_cube { 6usize } else { 1usize };
    let layer_stride = if face_count > 1 && (tex.image_size as usize) >= face_count {
        (tex.image_size as usize) / face_count
    } else {
        0
    };

    let mut dds_payload = Vec::new();
    for face in 0..face_count {
        let face_base = face * layer_stride;
        for (m, &mip_off) in tex.mip_offsets.iter().enumerate() {
            let mip_w = (tex.width >> m).max(1) as usize;
            let mip_h = (tex.height >> m).max(1) as usize;
            let wb = (mip_w + bw - 1) / bw;
            let hb = (mip_h + bh - 1) / bh;
            let mip_bh = gob_swizzle::mip_block_height(hb, bh_mip0);
            let pitch = align(wb * bpb, 64);
            let surf_size = pitch * align(hb, mip_bh * 8);
            let mut linear = vec![0u8; wb * hb * bpb];
            let slice_off = mip_off + face_base;
            if slice_off < bntx_bytes.len() {
                let avail = (bntx_bytes.len() - slice_off).min(surf_size);
                let mut surface = vec![0u8; surf_size];
                surface[..avail].copy_from_slice(&bntx_bytes[slice_off..slice_off + avail]);
                gob_swizzle::deswizzle_volume(&surface, &mut linear, wb, hb, 1, bpb, mip_bh, 1, pitch);
            }
            dds_payload.extend_from_slice(&linear);
        }
    }

    Ok(build_dds_file_ex(
        tex.width,
        tex.height,
        1,
        tex.mip_offsets.len().max(1) as u32,
        is_cube,
        tex.format,
        &dds_payload,
    ))
}

/// Export any `.dds`, `.xt1`, `.bntx`, or `.gtx` texture blob into a lossless `.dds` file.
pub fn export_texture_to_dds_lossless(blob: &[u8], sub_index: usize) -> Result<Vec<u8>> {
    if blob.starts_with(b"DDS ") {
        return Ok(blob.to_vec());
    }
    if blob.starts_with(b"XT1\0") {
        return deswizzle_xt1_to_dds(blob);
    }
    if blob.starts_with(b"Gfx2") {
        return convert_gtx_to_dds(blob);
    }
    if (blob.len() >= 4 && &blob[0..4] == b"BNTX")
        || (blob.len() >= 84 && &blob[80..84] == b"BNTX")
    {
        return deswizzle_bntx_to_dds(blob, sub_index);
    }
    let (img, _, _) = decode_texture_mip_filtered(blob, 0, "RGBA", false)?;
    Ok(encode_rgba8_to_dds(&img))
}

pub fn build_dds_file_ex(
    width: u32,
    height: u32,
    depth: u32,
    mip_count: u32,
    is_cubemap: bool,
    format: TextureFormat,
    payload: &[u8],
) -> Vec<u8> {
    if !is_cubemap && depth <= 1 {
        return build_dds_file(width, height, mip_count, format, payload);
    }
    let mut w = BinWriter::with_capacity(148 + payload.len(), Endian::Little);
    w.write_bytes(b"DDS ");
    w.write_u32(124);
    let (bw, bh) = format.block_dimensions();
    let is_block = bw > 1 || bh > 1;
    let flags = 0x1
        | 0x2
        | 0x4
        | 0x1000
        | if is_block { 0x80000 } else { 0x8 }
        | if mip_count > 1 { 0x20000 } else { 0 }
        | if depth > 1 { 0x800000 } else { 0 };
    w.write_u32(flags);
    w.write_u32(height);
    w.write_u32(width);

    let pitch_or_linear = if is_block {
        (((width as usize) + bw - 1) / bw)
            * (((height as usize) + bh - 1) / bh)
            * format.bytes_per_block()
    } else {
        (width as usize) * format.bytes_per_block()
    };
    w.write_u32(pitch_or_linear as u32);
    w.write_u32(depth.max(1));
    w.write_u32(mip_count.max(1));
    for _ in 0..11 {
        w.write_u32(0);
    }

    let dx10_code = match format {
        TextureFormat::Bc6hUf16
        | TextureFormat::Bc6hSf16
        | TextureFormat::Bc7
        | TextureFormat::Bc7Srgb
        | TextureFormat::Astc(_, _, _)
        | TextureFormat::Rgba16Float
        | TextureFormat::Rg8Unorm
        | TextureFormat::R8Unorm
        | TextureFormat::AfL8Unorm => format.to_dxgi_format(),
        _ => None,
    };

    w.write_u32(32);
    if dx10_code.is_some() {
        w.write_u32(0x4);
        w.write_bytes(b"DX10");
        for _ in 0..5 {
            w.write_u32(0);
        }
    } else {
        match format {
            TextureFormat::Bc1 | TextureFormat::Bc1Srgb => {
                w.write_u32(0x4);
                w.write_bytes(b"DXT1");
                for _ in 0..5 {
                    w.write_u32(0);
                }
            }
            TextureFormat::Bc2 | TextureFormat::Bc2Srgb => {
                w.write_u32(0x4);
                w.write_bytes(b"DXT3");
                for _ in 0..5 {
                    w.write_u32(0);
                }
            }
            TextureFormat::Bc3 | TextureFormat::Bc3Srgb => {
                w.write_u32(0x4);
                w.write_bytes(b"DXT5");
                for _ in 0..5 {
                    w.write_u32(0);
                }
            }
            TextureFormat::Bc4 => {
                w.write_u32(0x4);
                w.write_bytes(b"ATI1");
                for _ in 0..5 {
                    w.write_u32(0);
                }
            }
            TextureFormat::Bc5 => {
                w.write_u32(0x4);
                w.write_bytes(b"ATI2");
                for _ in 0..5 {
                    w.write_u32(0);
                }
            }
            _ => {
                w.write_u32(0x41);
                w.write_u32(0);
                w.write_u32(32);
                w.write_u32(0x0000_00FF);
                w.write_u32(0x0000_FF00);
                w.write_u32(0x00FF_0000);
                w.write_u32(0xFF00_0000);
            }
        }
    }

    let caps1 = 0x1000
        | if mip_count > 1 || is_cubemap { 0x8 } else { 0 }
        | if mip_count > 1 { 0x400000 } else { 0 };
    let caps2 = if is_cubemap {
        0xFE00
    } else if depth > 1 {
        0x200000
    } else {
        0
    };
    w.write_u32(caps1);
    w.write_u32(caps2);
    for _ in 0..3 {
        w.write_u32(0);
    }

    if let Some(dxgi) = dx10_code {
        w.write_u32(dxgi);
        w.write_u32(if depth > 1 { 4 } else { 3 });
        w.write_u32(if is_cubemap { 0x4 } else { 0 });
        w.write_u32(1);
        w.write_u32(0);
    }

    w.write_bytes(payload);
    w.into_inner()
}

/// Decode a specific `mip_index` and `layer_index` (animation slice or cubemap face) of any
/// `.xt1`, `.bntx`, `.gtx`, or `.dds` blob, returning `(RgbaImage, mip_width, mip_height, layer_count_at_mip)`.
pub fn decode_texture_layer_filtered(
    blob: &[u8],
    sub_index: usize,
    mip_index: u32,
    layer_index: u32,
    channel: &str,
    opaque: bool,
) -> Result<(RgbaImage, u32, u32, u32)> {
    if blob.len() < 16 {
        return Err(PgError::InvalidData("Texture blob too short".into()));
    }

    if &blob[0..4] == b"XT1\0" {
        if blob.len() < 0x38 {
            return Err(PgError::InvalidData("Truncated XT1 header".into()));
        }
        let mut r = BinReader::new(blob, Endian::Little);
        r.seek(0x08)?;
        let stored_image_size = r.read_u64()?;
        let header_size = r.read_u32()? as usize;
        let mip_word = r.read_u32()?;
        let mip_count = (mip_word & 0xFFFF).max(1);
        let tex_type = r.read_u32()?;
        let xt1_fmt = r.read_u32()?;
        let width = r.read_u32()?;
        let height = r.read_u32()?;
        let depth = r.read_u32()?.max(1);
        let _tex_flag = r.read_u32()?;
        let layout_word = r.read_u32()?;
        let format = TextureFormat::from_xt1_code(xt1_fmt);

        let layout = compute_xt1_layout(
            width,
            height,
            depth,
            mip_count,
            tex_type,
            layout_word,
            format,
            stored_image_size,
        );
        let clamped_mip = (mip_index as usize).min(layout.mips.len().saturating_sub(1));
        let mip_info = &layout.mips[clamped_mip];
        let payload = if header_size <= blob.len() {
            &blob[header_size..]
        } else {
            &[]
        };

        let is_cube = tex_type == 3 || tex_type == 8;
        let is_3d = tex_type == 2;
        let layer_count = if is_cube {
            6
        } else if is_3d {
            mip_info.slice_count.max(1) as u32
        } else {
            1
        };
        let clamped_layer = (layer_index as usize).min((layer_count as usize).saturating_sub(1));
        let bpb = format.bytes_per_block();
        let single_slice_bytes = mip_info.width_blocks * mip_info.height_blocks * bpb;
        let mut linear = vec![0u8; single_slice_bytes];

        if is_3d && mip_info.block_depth > 1 {
            let total_vol_bytes = single_slice_bytes * mip_info.slice_count;
            let mut vol_linear = vec![0u8; total_vol_bytes];
            let total_surf = mip_info.slice_surface_size * mip_info.slice_count;
            let mut surface = vec![0u8; total_surf];
            if mip_info.offset < payload.len() {
                let avail = (payload.len() - mip_info.offset).min(total_surf);
                surface[..avail].copy_from_slice(&payload[mip_info.offset..mip_info.offset + avail]);
            }
            gob_swizzle::deswizzle_volume(
                &surface,
                &mut vol_linear,
                mip_info.width_blocks,
                mip_info.height_blocks,
                mip_info.slice_count,
                bpb,
                mip_info.block_height,
                mip_info.block_depth,
                mip_info.pitch,
            );
            let slice_start = clamped_layer * single_slice_bytes;
            if slice_start + single_slice_bytes <= vol_linear.len() {
                linear.copy_from_slice(&vol_linear[slice_start..slice_start + single_slice_bytes]);
            }
        } else {
            let sub_offset = if is_cube {
                clamped_layer * layout.face_stride + mip_info.offset
            } else if is_3d {
                mip_info.offset + clamped_layer * mip_info.slice_surface_size
            } else {
                mip_info.offset
            };
            if sub_offset < payload.len() {
                let avail = (payload.len() - sub_offset).min(mip_info.slice_surface_size);
                let mut surface = vec![0u8; mip_info.slice_surface_size];
                surface[..avail].copy_from_slice(&payload[sub_offset..sub_offset + avail]);
                gob_swizzle::deswizzle_volume(
                    &surface,
                    &mut linear,
                    mip_info.width_blocks,
                    mip_info.height_blocks,
                    1,
                    bpb,
                    mip_info.block_height,
                    1,
                    mip_info.pitch,
                );
            }
        }

        let mut img = decode_linear_blocks_to_rgba8(&linear, mip_info.width, mip_info.height, format)?;
        apply_channel_and_opaque_filter(&mut img, channel, opaque);
        return Ok((img, mip_info.width, mip_info.height, layer_count));
    }

    if (blob.len() >= 4 && &blob[0..4] == b"BNTX")
        || (blob.len() >= 84 && &blob[80..84] == b"BNTX")
    {
        return decode_bntx_subtexture_layer_mip_filtered(
            blob,
            sub_index,
            mip_index,
            layer_index,
            channel,
            opaque,
        );
    }

    let meta = inspect_texture_blob(blob).ok();
    let layer_count = meta.as_ref().map(|m| m.layer_count.max(1)).unwrap_or(1);
    let (img, w, h) = decode_texture_mip_filtered(blob, mip_index, channel, opaque)?;
    Ok((img, w, h, layer_count))
}

fn apply_channel_and_opaque_filter(img: &mut RgbaImage, channel: &str, opaque: bool) {
    if channel != "RGBA" || opaque {
        for p in img.pixels_mut() {
            let [r, g, b, a] = p.0;
            p.0 = match channel {
                "RGB" => [r, g, b, 255],
                "R" => [r, r, r, 255],
                "G" => [g, g, g, 255],
                "B" => [b, b, b, 255],
                "A" => [a, a, a, 255],
                _ => [r, g, b, if opaque { 255 } else { a }],
            };
        }
    }
}

/// Compute scene-referred auto-exposure scale factor matching `Xt1.Core/Imaging/AutoExposure.cs`.
pub fn compute_auto_exposure_scale(faces: &[RgbaImage]) -> f32 {
    const MIDDLE_GREY: f64 = 0.18;
    const MIN_LUM: f64 = 1e-4;
    const MIN_SCALE: f32 = 1.0 / 32.0;
    const MAX_SCALE: f32 = 32.0;

    let mut log_sum = 0.0f64;
    let mut count = 0u64;
    for face in faces {
        for p in face.pixels() {
            let r = (p.0[0] as f64) / 255.0;
            let g = (p.0[1] as f64) / 255.0;
            let b = (p.0[2] as f64) / 255.0;
            let lum = (0.2126 * r + 0.7152 * g + 0.0722 * b).max(MIN_LUM);
            log_sum += lum.ln();
            count += 1;
        }
    }
    if count == 0 {
        return 1.0;
    }
    let log_avg = log_sum / (count as f64);
    let scale = (MIDDLE_GREY / log_avg.exp()) as f32;
    if scale.is_nan() || scale.is_infinite() {
        1.0
    } else {
        scale.clamp(MIN_SCALE, MAX_SCALE)
    }
}

/// Decode all 6 faces (`+X, -X, +Y, -Y, +Z, -Z`) of a cubemap at `mip_index`, returning
/// `(six_face_images, auto_exposure_scale)`.
pub fn decode_cubemap_six_faces(
    blob: &[u8],
    sub_index: usize,
    mip_index: u32,
    opaque: bool,
) -> Result<(Vec<RgbaImage>, f32)> {
    let mut faces = Vec::with_capacity(6);
    for face_idx in 0..6 {
        let (img, _, _, _) =
            decode_texture_layer_filtered(blob, sub_index, mip_index, face_idx, "RGBA", opaque)?;
        faces.push(img);
    }
    let scale = compute_auto_exposure_scale(&faces);
    Ok((faces, scale))
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct AstcRecompressConfig {
    pub opaque_bc7: bool,
    pub alpha_bc7: bool,
    pub opaque_quality: EncodeQuality,
    pub alpha_quality: EncodeQuality,
    pub downscale: bool,
    pub skip_downscale_without_mips: bool,
}

/// Recompress an `.xt1` or `.bntx` texture blob from `ASTC` to `BC1`/`BC3`/`BC7` in place,
/// reusing existing mip levels and optionally dropping mip 0 (or box-filtering single-mip images)
/// when `cfg.downscale` is enabled. Returns `Ok(Some((new_blob, log_msg)))` if converted, or `Ok(None)` if skipped.
pub fn recompress_astc_texture_blob_to_bc(
    blob: &[u8],
    sub_index: usize,
    cfg: &AstcRecompressConfig,
    nvtt_dll_path: Option<&Path>,
) -> Result<Option<(Vec<u8>, String)>> {
    if blob.starts_with(b"XT1\0") {
        let meta = inspect_texture_blob(blob)?;
        let TextureFormat::Astc(_, _, is_srgb) = meta.format else {
            return Ok(None);
        };
        if meta.dimension != TextureDimension::Tex2D {
            return Ok(None);
        }
        let mips = meta.mip_count.max(1);
        let mut levels = Vec::with_capacity(mips as usize);
        for m in 0..mips {
            let (img, _, _) = decode_texture_mip_filtered(blob, m, "RGBA", false)?;
            levels.push(img);
        }
        let has_alpha = levels[0].pixels().any(|p| p.0[3] < 255);
        let target_fmt = if has_alpha {
            if cfg.alpha_bc7 {
                if is_srgb { TextureFormat::Bc7Srgb } else { TextureFormat::Bc7 }
            } else if is_srgb {
                TextureFormat::Bc3Srgb
            } else {
                TextureFormat::Bc3
            }
        } else if cfg.opaque_bc7 {
            if is_srgb { TextureFormat::Bc7Srgb } else { TextureFormat::Bc7 }
        } else if is_srgb {
            TextureFormat::Bc1Srgb
        } else {
            TextureFormat::Bc1
        };
        let quality = if has_alpha {
            cfg.alpha_quality
        } else {
            cfg.opaque_quality
        };

        let working: Vec<RgbaImage> = if cfg.downscale {
            if levels.len() > 1 {
                levels[1..].to_vec()
            } else if !cfg.skip_downscale_without_mips {
                let chain = generate_mip_chain(&levels[0], 2);
                vec![chain.into_iter().last().unwrap()]
            } else {
                levels
            }
        } else {
            levels
        };

        let new_xt1 = encode_mip_levels_to_xt1(&working, target_fmt, quality, Some(blob), nvtt_dll_path);
        let msg = format!(
            "{} -> {} ({} quality, {} mip(s))",
            meta.format.display_name(),
            target_fmt.display_name(),
            quality.label(),
            working.len()
        );
        return Ok(Some((new_xt1, msg)));
    }

    if (blob.len() >= 4 && &blob[0..4] == b"BNTX")
        || (blob.len() >= 84 && &blob[80..84] == b"BNTX")
    {
        let recs = parse_bntx_textures(blob)?;
        let idx = sub_index.min(recs.len().saturating_sub(1));
        let rec = &recs[idx];
        let TextureFormat::Astc(_, _, is_srgb) = rec.format else {
            return Ok(None);
        };
        if rec.surface_dim == 3 || rec.depth > 1 || rec.array_length > 1 {
            return Ok(None);
        }
        let mips = rec.mip_offsets.len().max(1);
        let mut levels = Vec::with_capacity(mips);
        for m in 0..mips {
            let (img, _, _) =
                decode_bntx_subtexture_mip_filtered(blob, idx, m as u32, "RGBA", false)?;
            levels.push(img);
        }
        let has_alpha = levels[0].pixels().any(|p| p.0[3] < 255);
        let target_fmt = if has_alpha {
            if cfg.alpha_bc7 {
                if is_srgb { TextureFormat::Bc7Srgb } else { TextureFormat::Bc7 }
            } else if is_srgb {
                TextureFormat::Bc3Srgb
            } else {
                TextureFormat::Bc3
            }
        } else if cfg.opaque_bc7 {
            if is_srgb { TextureFormat::Bc7Srgb } else { TextureFormat::Bc7 }
        } else if is_srgb {
            TextureFormat::Bc1Srgb
        } else {
            TextureFormat::Bc1
        };
        let quality = if has_alpha {
            cfg.alpha_quality
        } else {
            cfg.opaque_quality
        };

        let working: Vec<RgbaImage> = if cfg.downscale {
            if levels.len() > 1 {
                levels[1..].to_vec()
            } else if !cfg.skip_downscale_without_mips {
                let chain = generate_mip_chain(&levels[0], 2);
                vec![chain.into_iter().last().unwrap()]
            } else {
                levels
            }
        } else {
            levels
        };

        let base_w = working[0].width();
        let base_h = working[0].height();
        let (bw, bh) = target_fmt.block_dimensions();
        let bpb = target_fmt.bytes_per_block();
        let hb0 = ((base_h as usize) + bh - 1) / bh;
        let bh_log2 = gob_swizzle::block_height_log2(hb0);
        let bh_mip0 = 1usize << bh_log2;

        let mut swizzled_mips = Vec::with_capacity(working.len());
        for mip_img in &working {
            let wb = ((mip_img.width() as usize) + bw - 1) / bw;
            let hb = ((mip_img.height() as usize) + bh - 1) / bh;
            let mip_bh = gob_swizzle::mip_block_height(hb, bh_mip0);
            let pitch = align(wb * bpb, 64);
            let surf_size = pitch * align(hb, mip_bh * 8);
            let linear =
                encode_linear_surface_with_quality(mip_img, target_fmt, quality, nvtt_dll_path);
            let mut swizzled = vec![0u8; surf_size];
            gob_swizzle::swizzle_volume(&linear, &mut swizzled, wb, hb, 1, bpb, mip_bh, 1, pitch);
            swizzled_mips.push(swizzled);
        }

        let new_bntx = rebuild_bntx_container_subtexture(
            blob,
            idx,
            target_fmt,
            base_w,
            base_h,
            bh_log2,
            &swizzled_mips,
        )?;
        let msg = format!(
            "{} ({}) -> {} ({} quality, {} mip(s))",
            rec.name,
            rec.format.display_name(),
            target_fmt.display_name(),
            quality.label(),
            working.len()
        );
        return Ok(Some((new_bntx, msg)));
    }

    Ok(None)
}
