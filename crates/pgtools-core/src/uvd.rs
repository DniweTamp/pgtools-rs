//! PlatinumGames `UVD` UV atlas / sprite-sheet definition parser, serializer,
//! CRC32 ID helpers, per-group sprite extractor, and deterministic pure-Rust
//! power-of-two rectangle packer.
//!
//! Format specification:
//! - 16-byte header (`0x10`):
//!   - `0x00`: `group_count: u32`
//!   - `0x04`: `item_count: u32`
//!   - `0x08`: `item_offset: u32` (normally `0x10`)
//!   - `0x0C`: `group_offset: u32` (`0x10 + 96 * item_count`)
//! - `Item` record = 96 bytes (`0x60`):
//!   - `0x00..0x40`: 64-byte null-padded UTF-8 name (`<= 63` characters + NUL)
//!   - `0x40`: `id: u32` (full 32-bit CRC32 of the name)
//!   - `0x44`: `group_id: u32` (`CRC32(group_name) & 0x7FFF_FFFF`)
//!   - `0x48`: `x: f32` (pixel X offset in sheet)
//!   - `0x4C`: `y: f32` (pixel Y offset in sheet)
//!   - `0x50`: `w: f32` (sprite width in pixels)
//!   - `0x54`: `h: f32` (sprite height in pixels)
//!   - `0x58`: `inv_w: f32` (`1.0 / sheet_width` as exact IEEE 754 `f32`)
//!   - `0x5C`: `inv_h: f32` (`1.0 / sheet_height` as exact IEEE 754 `f32`)
//! - `Group` record = 36 bytes (`0x24`):
//!   - `0x00..0x20`: 32-byte null-padded UTF-8 name (`<= 31` characters + NUL)
//!   - `0x20`: `id: u32` (`CRC32(name) & 0x7FFF_FFFF`, high bit cleared)

use crate::io::{checked_count, validate_single_entry_name, BinReader, BinWriter, Endian};
use crate::{PgError, Result};
use image::{Rgba, RgbaImage};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

/// Maximum allowed characters in a UVD Group name (32-byte field with NUL terminator).
pub const MAX_UVD_GROUP_NAME_LEN: usize = 31;

/// Maximum allowed characters in a UVD Item name (64-byte field with NUL terminator).
pub const MAX_UVD_ITEM_NAME_LEN: usize = 63;

/// Default inter-sprite padding in pixels when packing a UVD group sprite sheet.
pub const DEFAULT_UVD_PADDING: u32 = 4;

/// Default maximum power-of-two dimension (width or height) when packing a UVD group sprite sheet.
pub const DEFAULT_UVD_MAX_SIZE: u32 = 4096;

/// Size in bytes of the UVD file header.
pub const UVD_HEADER_SIZE: usize = 16;

/// Size in bytes of a single UVD Item record.
pub const UVD_ITEM_RECORD_SIZE: usize = 96;

/// Size in bytes of a single UVD Group record.
pub const UVD_GROUP_RECORD_SIZE: usize = 36;

/// Compute the full 32-bit CRC32 of a PlatinumGames UVD item name.
/// Uses ASCII-lowercased bytes to match PlatinumGames' case-insensitive CRC32 (`strcrc32i`)
/// and the existing texture editor CRC helper (identical to raw CRC32 for lowercase names).
#[inline]
pub fn compute_uvd_item_id(name: &str) -> u32 {
    let trimmed = name.trim_end_matches('\0').trim();
    crc32fast::hash(trimmed.to_ascii_lowercase().as_bytes())
}

/// Compute the 31-bit masked CRC32 (`CRC32(name) & 0x7FFF_FFFF`, high bit cleared)
/// used for UVD Group IDs and WTA/WTB texture IDs.
#[inline]
pub fn compute_uvd_group_id(name: &str) -> u32 {
    compute_uvd_item_id(name) & 0x7FFF_FFFF
}

/// Validate a UVD Group name (`1..=31` bytes, valid single path/entry segment, no NUL bytes).
pub fn validate_uvd_group_name(raw_name: &str) -> Result<String> {
    let trimmed = raw_name.trim_end_matches('\0').trim();
    if trimmed.is_empty() {
        return Err(PgError::InvalidData("UVD group name cannot be empty".into()));
    }
    if trimmed.len() > MAX_UVD_GROUP_NAME_LEN {
        return Err(PgError::InvalidData(format!(
            "UVD group name '{}' is {} bytes (maximum allowed is {} characters)",
            trimmed,
            trimmed.len(),
            MAX_UVD_GROUP_NAME_LEN
        )));
    }
    validate_single_entry_name(trimmed)
}

/// Validate a UVD Item name (`1..=63` bytes, valid single path/entry segment, no NUL bytes).
pub fn validate_uvd_item_name(raw_name: &str) -> Result<String> {
    let trimmed = raw_name.trim_end_matches('\0').trim();
    if trimmed.is_empty() {
        return Err(PgError::InvalidData("UVD item name cannot be empty".into()));
    }
    if trimmed.len() > MAX_UVD_ITEM_NAME_LEN {
        return Err(PgError::InvalidData(format!(
            "UVD item name '{}' is {} bytes (maximum allowed is {} characters)",
            trimmed,
            trimmed.len(),
            MAX_UVD_ITEM_NAME_LEN
        )));
    }
    validate_single_entry_name(trimmed)
}

/// A single sprite entry (96 bytes on disk) belonging to a `UvdGroup`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UvdItem {
    pub name: String,
    pub id: u32,
    pub group_id: u32,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub inv_w: f32,
    pub inv_h: f32,
}

impl UvdItem {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        name: impl Into<String>,
        group_id: u32,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        tex_w: u32,
        tex_h: u32,
    ) -> Self {
        let name_str = name.into();
        let id = compute_uvd_item_id(&name_str);
        let inv_w = if tex_w > 0 { 1.0f32 / (tex_w as f32) } else { 0.0 };
        let inv_h = if tex_h > 0 { 1.0f32 / (tex_h as f32) } else { 0.0 };
        Self {
            name: name_str,
            id,
            group_id,
            x,
            y,
            w,
            h,
            inv_w,
            inv_h,
        }
    }

    /// Inferred texture sheet width in pixels (`round(1.0 / inv_w)`).
    #[inline]
    pub fn tex_width(&self) -> u32 {
        if self.inv_w.is_finite() && self.inv_w > 0.0 {
            (1.0f32 / self.inv_w).round().max(1.0) as u32
        } else {
            0
        }
    }

    /// Inferred texture sheet height in pixels (`round(1.0 / inv_h)`).
    #[inline]
    pub fn tex_height(&self) -> u32 {
        if self.inv_h.is_finite() && self.inv_h > 0.0 {
            (1.0f32 / self.inv_h).round().max(1.0) as u32
        } else {
            0
        }
    }

    /// Update `inv_w = 1.0 / new_width` and `inv_h = 1.0 / new_height` as exact IEEE 754 `f32` values.
    #[inline]
    pub fn set_texture_dimensions(&mut self, new_width: u32, new_height: u32) {
        self.inv_w = 1.0f32 / (new_width.max(1) as f32);
        self.inv_h = 1.0f32 / (new_height.max(1) as f32);
    }
}

/// An independent sprite-sheet group (36-byte group header + ordered list of `UvdItem`s).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UvdGroup {
    pub name: String,
    pub id: u32,
    pub items: Vec<UvdItem>,
}

impl UvdGroup {
    pub fn new(name: impl Into<String>) -> Self {
        let name_str = name.into();
        let id = compute_uvd_group_id(&name_str);
        Self {
            name: name_str,
            id,
            items: Vec::new(),
        }
    }

    /// Infer the sprite sheet `(width, height)` from the group's items (`1.0 / inv_w`, `1.0 / inv_h`),
    /// falling back to the next power-of-two bounding box of the items when empty or zero.
    pub fn inferred_dimensions(&self) -> Option<(u32, u32)> {
        if let Some(first) = self.items.first() {
            let tw = first.tex_width();
            let th = first.tex_height();
            if tw > 0 && th > 0 {
                return Some((tw, th));
            }
        }
        if self.items.is_empty() {
            return None;
        }
        let max_x = self
            .items
            .iter()
            .map(|it| (it.x + it.w).ceil().max(1.0) as u32)
            .max()
            .unwrap_or(4);
        let max_y = self
            .items
            .iter()
            .map(|it| (it.y + it.h).ceil().max(1.0) as u32)
            .max()
            .unwrap_or(4);
        Some((
            max_x.next_power_of_two().max(4),
            max_y.next_power_of_two().max(4),
        ))
    }

    /// Update `inv_w` and `inv_h` on all items in this group to `1.0 / width` and `1.0 / height`.
    pub fn set_texture_dimensions(&mut self, new_width: u32, new_height: u32) {
        let inv_w = 1.0f32 / (new_width.max(1) as f32);
        let inv_h = 1.0f32 / (new_height.max(1) as f32);
        for item in &mut self.items {
            item.inv_w = inv_w;
            item.inv_h = inv_h;
        }
    }
}

/// Complete PlatinumGames `.uvd` UV atlas / sprite-sheet definition file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UvdFile {
    pub endian: Endian,
    pub groups: Vec<UvdGroup>,
}

impl UvdFile {
    pub fn new(endian: Endian) -> Self {
        Self {
            endian,
            groups: Vec::new(),
        }
    }

    /// Total number of items across all groups.
    pub fn total_items(&self) -> usize {
        self.groups.iter().map(|g| g.items.len()).sum()
    }

    /// Detect whether `bytes` has a valid UVD header in either Little-Endian or Big-Endian.
    pub fn detect_uvd_endian(bytes: &[u8]) -> Result<Endian> {
        if bytes.len() < UVD_HEADER_SIZE {
            return Err(PgError::InvalidData(
                "UVD buffer shorter than 16-byte header".into(),
            ));
        }

        let check_header = |endian: Endian| -> bool {
            let mut r = BinReader::new(bytes, endian);
            let Ok(gc) = r.read_u32() else { return false };
            let Ok(ic) = r.read_u32() else { return false };
            let Ok(ip) = r.read_u32() else { return false };
            let Ok(gp) = r.read_u32() else { return false };

            let gc = gc as usize;
            let ic = ic as usize;
            let ip = ip as usize;
            let gp = gp as usize;

            if gc > 65536 || ic > 262144 {
                return false;
            }
            let valid_ip = ip == UVD_HEADER_SIZE || (ic == 0 && ip == 0);
            if !valid_ip {
                return false;
            }
            let Some(expected_gp) = ic
                .checked_mul(UVD_ITEM_RECORD_SIZE)
                .and_then(|v| v.checked_add(UVD_HEADER_SIZE))
            else {
                return false;
            };
            if gp != expected_gp {
                return false;
            }
            let Some(expected_end) = gc
                .checked_mul(UVD_GROUP_RECORD_SIZE)
                .and_then(|v| v.checked_add(gp))
            else {
                return false;
            };
            expected_end <= bytes.len() && bytes.len().saturating_sub(expected_end) < 64
        };

        if check_header(Endian::Little) {
            return Ok(Endian::Little);
        }
        if check_header(Endian::Big) {
            return Ok(Endian::Big);
        }
        Err(PgError::InvalidData(
            "Invalid UVD header offsets or counts".into(),
        ))
    }

    #[inline]
    pub fn is_likely_uvd(bytes: &[u8]) -> bool {
        Self::detect_uvd_endian(bytes).is_ok()
    }

    /// Parse a `.uvd` binary buffer (automatically detecting Little-Endian vs Big-Endian).
    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        let endian = Self::detect_uvd_endian(bytes)?;
        let mut r = BinReader::new(bytes, endian);
        let group_count = r.read_u32()? as usize;
        let item_count = r.read_u32()? as usize;
        let item_offset = r.read_u32()? as usize;
        let group_offset = r.read_u32()? as usize;

        checked_count(
            group_count,
            UVD_GROUP_RECORD_SIZE,
            bytes.len().saturating_sub(group_offset),
            "UVD groups",
        )?;
        if item_count > 0 {
            checked_count(
                item_count,
                UVD_ITEM_RECORD_SIZE,
                bytes.len().saturating_sub(item_offset),
                "UVD items",
            )?;
        }

        r.seek(group_offset)?;
        let mut groups = Vec::with_capacity(group_count);
        let mut group_idx_by_id: BTreeMap<u32, usize> = BTreeMap::new();
        for gi in 0..group_count {
            let name = r.read_fixed_string(32)?;
            let id = r.read_u32()?;
            group_idx_by_id.entry(id).or_insert(gi);
            groups.push(UvdGroup {
                name,
                id,
                items: Vec::new(),
            });
        }

        if item_count > 0 {
            r.seek(item_offset)?;
            for _ in 0..item_count {
                let name = r.read_fixed_string(64)?;
                let id = r.read_u32()?;
                let group_id = r.read_u32()?;
                let x = r.read_f32()?;
                let y = r.read_f32()?;
                let w = r.read_f32()?;
                let h = r.read_f32()?;
                let inv_w = r.read_f32()?;
                let inv_h = r.read_f32()?;

                let item = UvdItem {
                    name,
                    id,
                    group_id,
                    x,
                    y,
                    w,
                    h,
                    inv_w,
                    inv_h,
                };
                if let Some(&g_idx) = group_idx_by_id.get(&group_id) {
                    groups[g_idx].items.push(item);
                } else {
                    let new_idx = groups.len();
                    group_idx_by_id.insert(group_id, new_idx);
                    groups.push(UvdGroup {
                        name: format!("group_0x{group_id:08X}"),
                        id: group_id,
                        items: vec![item],
                    });
                }
            }
        }

        Ok(Self { endian, groups })
    }

    /// Serialize `UvdFile` to binary bytes using `self.endian`.
    ///
    /// Layout matches PlatinumGames' binary UVD builder:
    /// - 16-byte header (`group_count`, `item_count`, `0x10`, `0x10 + 96 * item_count`)
    /// - Item records at `0x10`, grouped by ascending `group.id` and ordered within each group
    ///   by `group.items` slice order (produces 100% binary-identical round-trips on all real UVDs
    ///   while preserving intra-group item ordering)
    /// - Group records at `0x10 + 96 * item_count` in `self.groups` slice order.
    pub fn to_bytes(&self) -> Vec<u8> {
        self.to_bytes_with_endian(self.endian)
    }

    pub fn to_bytes_with_endian(&self, endian: Endian) -> Vec<u8> {
        let group_count = self.groups.len();
        let item_count = self.total_items();
        let item_offset = UVD_HEADER_SIZE;
        let group_offset = UVD_HEADER_SIZE + item_count * UVD_ITEM_RECORD_SIZE;
        let total_size = group_offset + group_count * UVD_GROUP_RECORD_SIZE;

        let mut w = BinWriter::with_capacity(total_size, endian);
        w.write_u32(group_count as u32);
        w.write_u32(item_count as u32);
        w.write_u32(item_offset as u32);
        w.write_u32(group_offset as u32);

        let mut groups_by_id: Vec<&UvdGroup> = self.groups.iter().collect();
        groups_by_id.sort_by_key(|g| g.id);

        w.seek(item_offset);
        for g in groups_by_id {
            for item in &g.items {
                w.write_fixed_string(&item.name, 64);
                w.write_u32(item.id);
                w.write_u32(item.group_id);
                w.write_f32(item.x);
                w.write_f32(item.y);
                w.write_f32(item.w);
                w.write_f32(item.h);
                w.write_f32(item.inv_w);
                w.write_f32(item.inv_h);
            }
        }

        w.seek(group_offset);
        for g in &self.groups {
            w.write_fixed_string(&g.name, 32);
            w.write_u32(g.id);
        }

        w.into_inner()
    }

    /// Find a group index by name (case-insensitive) or by hex/decimal ID string.
    pub fn find_group_index(&self, selector: &str) -> Option<usize> {
        let trimmed = selector.trim();
        if let Some(pos) = self
            .groups
            .iter()
            .position(|g| g.name.eq_ignore_ascii_case(trimmed))
        {
            return Some(pos);
        }
        let parsed_id = if let Some(hex) = trimmed
            .strip_prefix("0x")
            .or_else(|| trimmed.strip_prefix("0X"))
        {
            u32::from_str_radix(hex, 16).ok()
        } else {
            trimmed.parse::<u32>().ok()
        };
        if let Some(tid) = parsed_id {
            return self.groups.iter().position(|g| g.id == tid);
        }
        None
    }

    /// Check if `candidate_name` (via its item CRC32 or group CRC32) collides with any existing
    /// Group ID or Item ID in this `UvdFile`, optionally ignoring a specific group or item being renamed.
    pub fn check_name_crc_collision(
        &self,
        candidate_name: &str,
        is_group: bool,
        ignore_group_idx: Option<usize>,
        ignore_item: Option<(usize, usize)>,
    ) -> Option<String> {
        let item_crc = compute_uvd_item_id(candidate_name);
        let group_crc = compute_uvd_group_id(candidate_name);
        let target_crc = if is_group { group_crc } else { item_crc };

        for (gi, g) in self.groups.iter().enumerate() {
            if ignore_group_idx != Some(gi)
                && (g.id == target_crc
                    || g.id == group_crc
                    || g.id == item_crc
                    || g.name.eq_ignore_ascii_case(candidate_name))
            {
                return Some(format!(
                    "CRC32/ID collision with existing Group '{}' (0x{:08X})",
                    g.name, g.id
                ));
            }
            for (ii, it) in g.items.iter().enumerate() {
                if ignore_item != Some((gi, ii))
                    && (it.id == target_crc
                        || it.id == item_crc
                        || it.id == group_crc
                        || it.name.eq_ignore_ascii_case(candidate_name))
                {
                    return Some(format!(
                        "CRC32/ID collision with existing Item '{}' (0x{:08X}) in group '{}'",
                        it.name, it.id, g.name
                    ));
                }
            }
        }
        None
    }

    /// Generate a unique name by appending `_1`, `_2`, ... if `base_name` collides with an existing
    /// Group or Item ID, respecting the 31-char (group) or 63-char (item) length limit.
    pub fn auto_suffix_unique_name(&self, base_name: &str, is_group: bool) -> Result<String> {
        fn truncate_utf8_to_bytes(s: &str, max_bytes: usize) -> &str {
            if s.len() <= max_bytes {
                return s;
            }
            let mut end = max_bytes;
            while end > 0 && !s.is_char_boundary(end) {
                end -= 1;
            }
            &s[..end]
        }

        let max_len = if is_group {
            MAX_UVD_GROUP_NAME_LEN
        } else {
            MAX_UVD_ITEM_NAME_LEN
        };
        let clean = validate_single_entry_name(base_name)?;
        let truncated_base = truncate_utf8_to_bytes(&clean, max_len);
        if self
            .check_name_crc_collision(truncated_base, is_group, None, None)
            .is_none()
        {
            return Ok(truncated_base.to_string());
        }
        for suffix_num in 1..10000u32 {
            let suffix = format!("_{suffix_num}");
            let keep_len = max_len.saturating_sub(suffix.len());
            let prefix = truncate_utf8_to_bytes(truncated_base, keep_len);
            let candidate = format!("{prefix}{suffix}");
            if self
                .check_name_crc_collision(&candidate, is_group, None, None)
                .is_none()
            {
                return Ok(candidate);
            }
        }
        Err(PgError::InvalidData(format!(
            "Could not find unique non-colliding UVD name for '{base_name}'"
        )))
    }

    /// Serialize to a human-readable `.uvd.json` string.
    pub fn to_json_pretty(&self) -> Result<String> {
        serde_json::to_string_pretty(self).map_err(|e| PgError::Serialization(e.to_string()))
    }

    /// Parse from `.uvd.json` (supports both native `UvdFile` JSON and `UVD_tool.py` legacy JSON).
    pub fn from_json_str(json_str: &str) -> Result<Self> {
        if let Ok(uvd) = serde_json::from_str::<UvdFile>(json_str) {
            return Ok(uvd);
        }
        // Fallback parser for legacy `UVD_tool.py` JSON format
        let val: serde_json::Value =
            serde_json::from_str(json_str).map_err(|e| PgError::Serialization(e.to_string()))?;
        let endian = match val.get("Endianness").and_then(|v| v.as_str()) {
            Some("big") | Some("Big") => Endian::Big,
            _ => Endian::Little,
        };
        let groups_obj = val
            .get("Groups")
            .and_then(|v| v.as_object())
            .ok_or_else(|| PgError::InvalidData("Missing 'Groups' object in UVD JSON".into()))?;
        let group_order: Vec<u32> = if let Some(arr) = val.get("Group order").and_then(|v| v.as_array()) {
            arr.iter()
                .filter_map(|v| v.as_u64().map(|n| n as u32))
                .collect()
        } else {
            groups_obj
                .keys()
                .filter_map(|k| k.parse::<u32>().ok())
                .collect()
        };

        let mut groups = Vec::with_capacity(group_order.len());
        for gid in group_order {
            let g_val = groups_obj
                .get(&gid.to_string())
                .ok_or_else(|| PgError::InvalidData(format!("Missing group {gid} in UVD JSON")))?;
            let g_name = g_val
                .get("Name")
                .and_then(|v| v.as_str())
                .unwrap_or("group")
                .to_string();
            let mut items = Vec::new();
            if let Some(items_obj) = g_val.get("Items").and_then(|v| v.as_object()) {
                for (iid_str, it_val) in items_obj {
                    let iid = iid_str.parse::<u32>().unwrap_or(0);
                    let iname = it_val
                        .get("Name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("item")
                        .to_string();
                    let x = it_val
                        .get("X Position in File")
                        .and_then(|v| v.as_f64())
                        .unwrap_or(0.0) as f32;
                    let y = it_val
                        .get("Y Position in File")
                        .and_then(|v| v.as_f64())
                        .unwrap_or(0.0) as f32;
                    let w = it_val
                        .get("Item Size X")
                        .and_then(|v| v.as_f64())
                        .unwrap_or(1.0) as f32;
                    let h = it_val
                        .get("Item Size Y")
                        .and_then(|v| v.as_f64())
                        .unwrap_or(1.0) as f32;
                    let tex_w = it_val
                        .get("Tex Size W")
                        .and_then(|v| v.as_f64())
                        .unwrap_or(256.0) as f32;
                    let tex_h = it_val
                        .get("Tex Size H")
                        .and_then(|v| v.as_f64())
                        .unwrap_or(256.0) as f32;
                    items.push(UvdItem {
                        name: iname,
                        id: iid,
                        group_id: gid,
                        x,
                        y,
                        w,
                        h,
                        inv_w: if tex_w > 0.0 { 1.0f32 / tex_w } else { 0.0 },
                        inv_h: if tex_h > 0.0 { 1.0f32 / tex_h } else { 0.0 },
                    });
                }
            }
            items.sort_by_key(|it| it.id);
            groups.push(UvdGroup {
                name: g_name,
                id: gid,
                items,
            });
        }

        Ok(Self { endian, groups })
    }
}

// ============================================================================
// PER-GROUP SPRITE EXTRACTOR & DETERMINISTIC PURE-RUST RECTANGLE PACKER
// ============================================================================

/// Crop a single `UvdItem`'s rectangle `(x, y, w, h)` from its group sprite sheet `sheet`.
pub fn crop_sprite_from_sheet(sheet: &RgbaImage, item: &UvdItem) -> RgbaImage {
    let x0 = item.x.round() as i64;
    let y0 = item.y.round() as i64;
    let w = item.w.round().max(1.0) as u32;
    let h = item.h.round().max(1.0) as u32;

    let mut out = RgbaImage::from_pixel(w, h, Rgba([0, 0, 0, 0]));
    let sheet_w = sheet.width() as i64;
    let sheet_h = sheet.height() as i64;

    for dy in 0..h {
        let sy = y0 + (dy as i64);
        if sy < 0 || sy >= sheet_h {
            continue;
        }
        for dx in 0..w {
            let sx = x0 + (dx as i64);
            if sx < 0 || sx >= sheet_w {
                continue;
            }
            let p = *sheet.get_pixel(sx as u32, sy as u32);
            out.put_pixel(dx, dy, p);
        }
    }
    out
}

/// Extract all sprites in `group` from `sheet` in `group.items` order as `(item_name, cropped_rgba)`.
pub fn extract_group_sprites(group: &UvdGroup, sheet: &RgbaImage) -> Vec<(String, RgbaImage)> {
    group
        .items
        .iter()
        .map(|item| (item.name.clone(), crop_sprite_from_sheet(sheet, item)))
        .collect()
}

/// Extract all sprites in `group` to `<base_out_dir>/<group.name>/<item.name>.png`.
/// Returns the created group subfolder path and number of PNG files written.
pub fn extract_group_to_dir(
    group: &UvdGroup,
    sheet: &RgbaImage,
    base_out_dir: &Path,
) -> Result<(PathBuf, usize)> {
    let clean_group = validate_single_entry_name(&group.name)
        .unwrap_or_else(|_| format!("group_0x{:08X}", group.id));
    let group_dir = base_out_dir.join(clean_group);
    fs::create_dir_all(&group_dir)?;

    let mut written = 0usize;
    for item in &group.items {
        let clean_item = validate_single_entry_name(&item.name)
            .unwrap_or_else(|_| format!("item_0x{:08X}", item.id));
        let sprite = crop_sprite_from_sheet(sheet, item);
        let out_file = group_dir.join(format!("{clean_item}.png"));
        sprite
            .save(&out_file)
            .map_err(|e| PgError::InvalidData(format!("Failed to write {}: {e}", out_file.display())))?;
        written += 1;
    }
    Ok((group_dir, written))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FreeRect {
    x: u32,
    y: u32,
    w: u32,
    h: u32,
}

/// Result of packing a single UVD Group into a power-of-two RGBA sprite sheet.
#[derive(Debug, Clone)]
pub struct PackedGroupResult {
    pub sheet: RgbaImage,
    pub width: u32,
    pub height: u32,
    pub group: UvdGroup,
}

/// Try to place `sprites` (given as `(w, h)` pairs) in `order` inside a `(pw, ph)` canvas
/// with minimum inter-sprite gap `padding` (and `0` outer border requirement) using deterministic
/// MaxRects (Top-Left `(y, x)` primary, Best Short Side Fit secondary, no rotation).
fn try_pack_on_canvas(
    dims: &[(u32, u32)],
    order: &[usize],
    pw: u32,
    ph: u32,
    padding: u32,
) -> Option<Vec<(u32, u32)>> {
    let cw = pw.saturating_add(padding);
    let ch = ph.saturating_add(padding);
    let mut free = vec![FreeRect {
        x: 0,
        y: 0,
        w: cw,
        h: ch,
    }];
    let mut placed = vec![(0u32, 0u32); dims.len()];

    for &idx in order {
        let (orig_w, orig_h) = dims[idx];
        let sw = orig_w.saturating_add(padding);
        let sh = orig_h.saturating_add(padding);

        let mut best_score: Option<(u32, u32, u32, u32)> = None;
        let mut best_pos: Option<(u32, u32)> = None;

        for r in &free {
            if r.w >= sw && r.h >= sh {
                let rem_w = r.w - sw;
                let rem_h = r.h - sh;
                let short_side = rem_w.min(rem_h);
                let long_side = rem_w.max(rem_h);
                let score = (r.y, r.x, short_side, long_side);
                if best_score.map_or(true, |cur| score < cur) {
                    best_score = Some(score);
                    best_pos = Some((r.x, r.y));
                }
            }
        }

        let (bx, by) = best_pos?;
        placed[idx] = (bx, by);

        // Split intersecting free rectangles against `[bx, bx + sw) x [by, by + sh)`
        let mut next_free = Vec::with_capacity(free.len() + 4);
        for r in free {
            if bx >= r.x + r.w || bx + sw <= r.x || by >= r.y + r.h || by + sh <= r.y {
                next_free.push(r);
            } else {
                if bx > r.x {
                    next_free.push(FreeRect {
                        x: r.x,
                        y: r.y,
                        w: bx - r.x,
                        h: r.h,
                    });
                }
                if bx + sw < r.x + r.w {
                    next_free.push(FreeRect {
                        x: bx + sw,
                        y: r.y,
                        w: (r.x + r.w) - (bx + sw),
                        h: r.h,
                    });
                }
                if by > r.y {
                    next_free.push(FreeRect {
                        x: r.x,
                        y: r.y,
                        w: r.w,
                        h: by - r.y,
                    });
                }
                if by + sh < r.y + r.h {
                    next_free.push(FreeRect {
                        x: r.x,
                        y: by + sh,
                        w: r.w,
                        h: (r.y + r.h) - (by + sh),
                    });
                }
            }
        }

        // Prune redundant rectangles contained within another free rectangle
        let mut pruned = Vec::with_capacity(next_free.len());
        for (i, r1) in next_free.iter().enumerate() {
            if r1.w == 0 || r1.h == 0 {
                continue;
            }
            let mut is_contained = false;
            for (j, r2) in next_free.iter().enumerate() {
                if i != j
                    && r1.x >= r2.x
                    && r1.y >= r2.y
                    && r1.x + r1.w <= r2.x + r2.w
                    && r1.y + r1.h <= r2.y + r2.h
                    && (*r1 != *r2 || i > j)
                {
                    is_contained = true;
                    break;
                }
            }
            if !is_contained {
                pruned.push(*r1);
            }
        }
        free = pruned;
    }

    Some(placed)
}

/// Pure-Rust deterministic power-of-two rectangle packer for a single UVD Group.
///
/// Rules enforced:
/// - Packing unit is always a single `UvdGroup`.
/// - Power-of-two output dimensions `(width, height)` (`4..=max_size`, rectangular allowed).
/// - No sprite rotation.
/// - Configurable inter-sprite `padding` (default `4` px) and `max_size` (default `4096`).
/// - Automatically chooses the next suitable power-of-two size when the group no longer fits.
/// - Writes `inv_w = 1.0 / new_width` and `inv_h = 1.0 / new_height` as exact IEEE 754 `f32`
///   for every item in the group.
pub fn pack_group_sprites(
    group_name: &str,
    group_id: u32,
    sprites: &[(String, u32, RgbaImage)],
    padding: u32,
    max_size: u32,
    preferred_dims: Option<(u32, u32)>,
) -> Result<PackedGroupResult> {
    let eff_max = max_size.max(4).next_power_of_two();

    if sprites.is_empty() {
        let (w, h) = preferred_dims
            .map(|(pw, ph)| {
                (
                    pw.max(4).next_power_of_two().min(eff_max),
                    ph.max(4).next_power_of_two().min(eff_max),
                )
            })
            .unwrap_or((4, 4));
        return Ok(PackedGroupResult {
            sheet: RgbaImage::from_pixel(w, h, Rgba([0, 0, 0, 0])),
            width: w,
            height: h,
            group: UvdGroup {
                name: group_name.to_string(),
                id: group_id,
                items: Vec::new(),
            },
        });
    }

    let mut dims = Vec::with_capacity(sprites.len());
    let mut max_w = 1u32;
    let mut max_h = 1u32;
    let mut total_area = 0u64;

    for (name, _, img) in sprites {
        let w = img.width().max(1);
        let h = img.height().max(1);
        if w > eff_max || h > eff_max {
            return Err(PgError::InvalidData(format!(
                "Sprite '{}' ({}x{}) in group '{}' exceeds maximum atlas size {}x{}",
                name, w, h, group_name, eff_max, eff_max
            )));
        }
        max_w = max_w.max(w);
        max_h = max_h.max(h);
        total_area += (w as u64) * (h as u64);
        dims.push((w, h));
    }

    // Deterministic placement order: tallest first, then widest, then original group item index
    let mut order: Vec<usize> = (0..sprites.len()).collect();
    order.sort_by(|&a, &b| {
        dims[b]
            .1
            .cmp(&dims[a].1)
            .then_with(|| dims[b].0.cmp(&dims[a].0))
            .then_with(|| a.cmp(&b))
    });

    let mut powers = Vec::new();
    let mut p = 4u32;
    while p <= eff_max {
        powers.push(p);
        if let Some(next_p) = p.checked_mul(2) {
            p = next_p;
        } else {
            break;
        }
    }

    let mut candidates: Vec<(u64, u32, u8, i64, u32, u32)> = Vec::new();
    for &pw in &powers {
        if pw < max_w {
            continue;
        }
        for &ph in &powers {
            if ph < max_h {
                continue;
            }
            let area = (pw as u64) * (ph as u64);
            if area < total_area {
                continue;
            }
            let aspect = pw.max(ph) / pw.min(ph).max(1);
            let matches_pref = if preferred_dims == Some((pw, ph)) {
                0u8
            } else {
                1u8
            };
            candidates.push((area, aspect, matches_pref, -(pw as i64), pw, ph));
        }
    }
    candidates.sort_unstable();

    for (_, _, _, _, pw, ph) in candidates {
        if let Some(placed) = try_pack_on_canvas(&dims, &order, pw, ph, padding) {
            let mut sheet = RgbaImage::from_pixel(pw, ph, Rgba([0, 0, 0, 0]));
            let inv_w = 1.0f32 / (pw as f32);
            let inv_h = 1.0f32 / (ph as f32);
            let mut items = Vec::with_capacity(sprites.len());

            for (idx, (name, item_id, img)) in sprites.iter().enumerate() {
                let (bx, by) = placed[idx];
                let (sw, sh) = dims[idx];
                for y in 0..sh {
                    for x in 0..sw {
                        let px = *img.get_pixel(x, y);
                        sheet.put_pixel(bx + x, by + y, px);
                    }
                }
                items.push(UvdItem {
                    name: name.clone(),
                    id: *item_id,
                    group_id,
                    x: bx as f32,
                    y: by as f32,
                    w: sw as f32,
                    h: sh as f32,
                    inv_w,
                    inv_h,
                });
            }

            return Ok(PackedGroupResult {
                sheet,
                width: pw,
                height: ph,
                group: UvdGroup {
                    name: group_name.to_string(),
                    id: group_id,
                    items,
                },
            });
        }
    }

    Err(PgError::InvalidData(format!(
        "Group '{}' ({} sprites) does not fit within max power-of-two size {}x{} (padding {}px)",
        group_name,
        sprites.len(),
        eff_max,
        eff_max,
        padding
    )))
}

/// Repack a single group in `uvd` from a directory of `.png` files (`group_dir`).
/// Preserves existing item IDs for matching item names, computes `compute_uvd_item_id` for new items,
/// validates item name lengths (`<= 63`), and leaves all other groups in `uvd` untouched.
pub fn repack_uvd_group_from_dir(
    uvd: &mut UvdFile,
    group_dir: &Path,
    explicit_group_name: Option<&str>,
    padding: u32,
    max_size: u32,
) -> Result<(usize, PackedGroupResult)> {
    let inferred_name = explicit_group_name
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .or_else(|| {
            group_dir
                .file_name()
                .and_then(|s| s.to_str())
                .map(|s| s.to_string())
        })
        .ok_or_else(|| PgError::InvalidData("Could not determine group name from directory".into()))?;

    // If `group_dir/<inferred_name>` exists as a subdirectory, allow passing the parent extract root too.
    let actual_dir = if group_dir.join(&inferred_name).is_dir() {
        group_dir.join(&inferred_name)
    } else {
        group_dir.to_path_buf()
    };

    let group_idx = match uvd.find_group_index(&inferred_name) {
        Some(idx) => idx,
        None => {
            let valid_gname = validate_uvd_group_name(&inferred_name)?;
            let new_gid = compute_uvd_group_id(&valid_gname);
            uvd.groups.push(UvdGroup {
                name: valid_gname,
                id: new_gid,
                items: Vec::new(),
            });
            uvd.groups.len() - 1
        }
    };

    let existing_group = &uvd.groups[group_idx];
    let group_name = existing_group.name.clone();
    let group_id = existing_group.id;
    let preferred_dims = existing_group.inferred_dimensions();

    // Map existing item names -> (original_order_index, existing_id)
    let mut existing_by_name: BTreeMap<String, (usize, u32, String)> = BTreeMap::new();
    for (idx, it) in existing_group.items.iter().enumerate() {
        existing_by_name.insert(it.name.to_ascii_lowercase(), (idx, it.id, it.name.clone()));
    }

    let mut png_files: Vec<PathBuf> = fs::read_dir(&actual_dir)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.is_file()
                && p.extension()
                    .and_then(|e| e.to_str())
                    .map(|e| e.eq_ignore_ascii_case("png"))
                    .unwrap_or(false)
        })
        .collect();
    png_files.sort();

    if png_files.is_empty() {
        return Err(PgError::InvalidData(format!(
            "No .png sprites found in '{}'",
            actual_dir.display()
        )));
    }

    // Clear target group items while checking so removed items do not trigger false collisions,
    // while keeping target group.id active so new items are checked against group_id and all other groups.
    let saved_prev_items = std::mem::take(&mut uvd.groups[group_idx].items);

    let mut loaded_sprites: Vec<(usize, String, u32, RgbaImage)> =
        Vec::with_capacity(png_files.len());
    for (disk_ord, p) in png_files.into_iter().enumerate() {
        let stem = match p.file_stem().and_then(|s| s.to_str()) {
            Some(s) => s,
            None => {
                uvd.groups[group_idx].items = saved_prev_items;
                return Err(PgError::InvalidData(format!("Invalid PNG filename: {}", p.display())));
            }
        };
        let valid_stem = match validate_uvd_item_name(stem) {
            Ok(v) => v,
            Err(e) => {
                uvd.groups[group_idx].items = saved_prev_items;
                return Err(e);
            }
        };
        let raw_bytes = match fs::read(&p) {
            Ok(b) => b,
            Err(e) => {
                uvd.groups[group_idx].items = saved_prev_items;
                return Err(PgError::Io(e));
            }
        };
        let img = match image::load_from_memory(&raw_bytes) {
            Ok(im) => im.to_rgba8(),
            Err(e) => {
                uvd.groups[group_idx].items = saved_prev_items;
                return Err(PgError::InvalidData(format!("Failed to decode {}: {e}", p.display())));
            }
        };

        let (ord, final_name, final_id) = if let Some(&(orig_idx, orig_id, ref orig_name)) =
            existing_by_name.get(&valid_stem.to_ascii_lowercase())
        {
            (orig_idx, orig_name.clone(), orig_id)
        } else {
            if let Some(collision) =
                uvd.check_name_crc_collision(&valid_stem, false, None, None)
            {
                uvd.groups[group_idx].items = saved_prev_items;
                return Err(PgError::InvalidData(collision));
            }
            let new_id = compute_uvd_item_id(&valid_stem);
            let new_grp_crc = compute_uvd_group_id(&valid_stem);
            for (_, prev_name, prev_id, _) in &loaded_sprites {
                if *prev_id == new_id
                    || *prev_id == new_grp_crc
                    || prev_name.eq_ignore_ascii_case(&valid_stem)
                {
                    uvd.groups[group_idx].items = saved_prev_items;
                    return Err(PgError::InvalidData(format!(
                        "CRC32/ID collision between new sprites '{}' and '{}' (0x{:08X})",
                        valid_stem, prev_name, prev_id
                    )));
                }
            }
            (existing_by_name.len() + disk_ord, valid_stem, new_id)
        };
        loaded_sprites.push((ord, final_name, final_id, img));
    }

    // Preserve original group item order for existing sprites, appending new sprites in deterministic filename order
    loaded_sprites.sort_by_key(|(ord, _, _, _)| *ord);
    let pack_inputs: Vec<(String, u32, RgbaImage)> = loaded_sprites
        .into_iter()
        .map(|(_, name, id, img)| (name, id, img))
        .collect();

    let packed = pack_group_sprites(
        &group_name,
        group_id,
        &pack_inputs,
        padding,
        max_size,
        preferred_dims,
    )?;
    uvd.groups[group_idx] = packed.group.clone();

    Ok((group_idx, packed))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uvd_crc32_helpers_match_known_sample_ids() {
        // Verified against ui_core.dat / core.uvd & core.wta
        assert_eq!(compute_uvd_group_id("core_sys_btn"), 0x79C0_96FA);
        assert_eq!(compute_uvd_group_id("core_beast_icon_small"), 0x4829_60F9);
        assert_eq!(compute_uvd_group_id("core_punchthrough"), 0x73AC_A11A);
        assert_eq!(compute_uvd_group_id("core_main"), 0x0DCD_0E51);
        assert_eq!(compute_uvd_group_id("core_accessory_icon"), 0x0291_8C4F);

        // Item IDs use full 32-bit CRC32
        assert_eq!(compute_uvd_item_id("accessory_icon_08"), 0x0315_8924);
        assert_eq!(compute_uvd_item_id("accessory_icon_04"), 0x0AA3_C50F);
    }

    #[test]
    fn uvd_name_length_validation_enforces_limits() {
        assert!(validate_uvd_group_name("valid_group_name").is_ok());
        assert!(validate_uvd_group_name(&"a".repeat(31)).is_ok());
        assert!(validate_uvd_group_name(&"a".repeat(32)).is_err());

        assert!(validate_uvd_item_name("valid_item_name").is_ok());
        assert!(validate_uvd_item_name(&"b".repeat(63)).is_ok());
        assert!(validate_uvd_item_name(&"b".repeat(64)).is_err());
    }

    #[test]
    fn uvd_binary_round_trip_both_endians() {
        let mut uvd = UvdFile::new(Endian::Little);
        let mut g1 = UvdGroup::new("test_grp_a");
        g1.items.push(UvdItem::new("spr_01", g1.id, 0.0, 0.0, 64.0, 32.0, 256, 128));
        g1.items.push(UvdItem::new("spr_02", g1.id, 68.0, 0.0, 64.0, 32.0, 256, 128));
        g1.items.sort_by_key(|it| it.id);
        uvd.groups.push(g1);

        let le_bytes = uvd.to_bytes_with_endian(Endian::Little);
        let parsed_le = UvdFile::from_bytes(&le_bytes).expect("parse LE UVD");
        assert_eq!(parsed_le.endian, Endian::Little);
        assert_eq!(parsed_le.to_bytes(), le_bytes);

        let be_bytes = uvd.to_bytes_with_endian(Endian::Big);
        let parsed_be = UvdFile::from_bytes(&be_bytes).expect("parse BE UVD");
        assert_eq!(parsed_be.endian, Endian::Big);
        assert_eq!(parsed_be.to_bytes(), be_bytes);
    }

    #[test]
    fn uvd_extract_repack_extract_round_trip_pixel_identical() {
        let gname = "hud_icons";
        let gid = compute_uvd_group_id(gname);

        let mut spr1 = RgbaImage::new(48, 32);
        for (x, y, p) in spr1.enumerate_pixels_mut() {
            *p = Rgba([(x * 5) as u8, (y * 7) as u8, 180, 255]);
        }
        let mut spr2 = RgbaImage::new(80, 64);
        for (x, y, p) in spr2.enumerate_pixels_mut() {
            *p = Rgba([40, (x * 3) as u8, (y * 4) as u8, if (x + y) % 2 == 0 { 255 } else { 90 }]);
        }
        let mut spr3 = RgbaImage::new(32, 96);
        for (x, y, p) in spr3.enumerate_pixels_mut() {
            *p = Rgba([220, (x * 6) as u8, (y * 2) as u8, 200]);
        }

        let inputs = vec![
            ("icon_alpha".to_string(), compute_uvd_item_id("icon_alpha"), spr1.clone()),
            ("icon_beta".to_string(), compute_uvd_item_id("icon_beta"), spr2.clone()),
            ("icon_gamma".to_string(), compute_uvd_item_id("icon_gamma"), spr3.clone()),
        ];

        // Pack 1
        let packed1 = pack_group_sprites(gname, gid, &inputs, 4, 4096, None)
            .expect("pack1");
        assert!(packed1.width.is_power_of_two());
        assert!(packed1.height.is_power_of_two());
        for it in &packed1.group.items {
            assert_eq!(it.inv_w.to_bits(), (1.0f32 / (packed1.width as f32)).to_bits());
            assert_eq!(it.inv_h.to_bits(), (1.0f32 / (packed1.height as f32)).to_bits());
        }

        // Extract 1
        let extracted1 = extract_group_sprites(&packed1.group, &packed1.sheet);
        assert_eq!(extracted1.len(), 3);
        assert_eq!(extracted1[0].1, spr1);
        assert_eq!(extracted1[1].1, spr2);
        assert_eq!(extracted1[2].1, spr3);

        // Repack from extracted1 and Extract 2
        let repack_inputs: Vec<(String, u32, RgbaImage)> = extracted1
            .into_iter()
            .zip(packed1.group.items.iter())
            .map(|((n, img), it)| (n, it.id, img))
            .collect();
        let packed2 = pack_group_sprites(
            gname,
            gid,
            &repack_inputs,
            4,
            4096,
            Some((packed1.width, packed1.height)),
        )
        .expect("pack2");
        let extracted2 = extract_group_sprites(&packed2.group, &packed2.sheet);
        assert_eq!(extracted2[0].1, spr1);
        assert_eq!(extracted2[1].1, spr2);
        assert_eq!(extracted2[2].1, spr3);
    }
}
