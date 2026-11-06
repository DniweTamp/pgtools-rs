//! Endian-aware binary reader/writer, alignment helpers, IEEE 754 `f16`,
//! PlatinumGames custom `PgHalf` (1-sign, 6-exponent, 9-mantissa, bias 47),
//! safe archive path sanitization, and bounded allocation checks.

use crate::{PgError, Result};
use serde::{Deserialize, Serialize};
use std::path::{Component, Path, PathBuf};

/// Maximum single allocation / decompression size (512 MiB) when parsing untrusted containers.
pub const MAX_ARCHIVE_ALLOC_BYTES: usize = 512 * 1024 * 1024;

/// Validate that `count * min_elem_bytes` does not overflow, fits within `available_bytes`,
/// and does not exceed `MAX_ARCHIVE_ALLOC_BYTES`.
pub fn checked_count(
    count: usize,
    min_elem_bytes: usize,
    available_bytes: usize,
    context: &str,
) -> Result<usize> {
    let step = min_elem_bytes.max(1);
    let total = count.checked_mul(step).ok_or_else(|| {
        PgError::InvalidData(format!("{context}: count {count} * {step} overflow"))
    })?;
    if total > available_bytes {
        return Err(PgError::InvalidData(format!(
            "{context}: count {count} ({total} bytes) exceeds available buffer ({available_bytes} bytes)"
        )));
    }
    if total > MAX_ARCHIVE_ALLOC_BYTES {
        return Err(PgError::InvalidData(format!(
            "{context}: allocation size {total} bytes exceeds safety limit ({MAX_ARCHIVE_ALLOC_BYTES} bytes)"
        )));
    }
    Ok(count)
}

/// Returns true if `seg` (with or without extension) matches a Windows reserved device name.
pub fn is_windows_reserved_name(seg: &str) -> bool {
    let stem = seg
        .split('.')
        .next()
        .unwrap_or(seg)
        .trim_matches([' ', '.']);
    let upper = stem.to_ascii_uppercase();
    matches!(
        upper.as_str(),
        "CON"
            | "PRN"
            | "AUX"
            | "NUL"
            | "COM0"
            | "COM1"
            | "COM2"
            | "COM3"
            | "COM4"
            | "COM5"
            | "COM6"
            | "COM7"
            | "COM8"
            | "COM9"
            | "COM¹"
            | "COM²"
            | "COM³"
            | "LPT0"
            | "LPT1"
            | "LPT2"
            | "LPT3"
            | "LPT4"
            | "LPT5"
            | "LPT6"
            | "LPT7"
            | "LPT8"
            | "LPT9"
            | "LPT¹"
            | "LPT²"
            | "LPT³"
    )
}

/// Validate a single path component (e.g., for file manager `rename` or `new_folder`).
/// Rejects empty strings, `.`, `..`, path separators, drive/ADS colons, control chars,
/// Windows reserved device names, and trailing dots/spaces.
pub fn validate_single_entry_name(raw_name: &str) -> Result<String> {
    let trimmed_nul = raw_name.trim_end_matches('\0');
    if trimmed_nul.contains('\0') {
        return Err(PgError::InvalidData("Entry name contains interior NUL byte".into()));
    }
    let clean = trimmed_nul.trim().trim_end_matches(['.', ' ']);
    if clean.is_empty() || clean == "." || clean == ".." {
        return Err(PgError::InvalidData("Entry name cannot be empty, '.', or '..'".into()));
    }
    if clean.chars().any(|c| {
        c.is_control() || matches!(c, '/' | '\\' | ':' | '<' | '>' | '"' | '|' | '?' | '*')
    }) {
        return Err(PgError::InvalidData(format!(
            "Entry name '{clean}' contains invalid path characters"
        )));
    }
    if is_windows_reserved_name(clean) {
        return Err(PgError::InvalidData(format!(
            "Entry name '{clean}' is a reserved system device name"
        )));
    }
    let mut comps = Path::new(clean).components();
    match (comps.next(), comps.next()) {
        (Some(Component::Normal(_)), None) => Ok(clean.to_string()),
        _ => Err(PgError::InvalidData(format!(
            "Entry name '{clean}' must be a single normal path segment"
        ))),
    }
}

/// Sanitize an untrusted archive entry path against `base_dir`, preventing Windows drive prefixes,
/// NTFS alternate data streams (`:`), reserved device names, and `..` directory traversal.
pub fn sanitize_entry_path(
    base_dir: &Path,
    raw_name: &str,
    fallback_name: &str,
) -> Result<PathBuf> {
    let trimmed_nul = raw_name.trim_end_matches('\0');
    if trimmed_nul.contains('\0') {
        return Err(PgError::InvalidData(
            "Archive entry path contains interior NUL byte".into(),
        ));
    }

    let mut rel_path = PathBuf::new();
    for seg in trimmed_nul.split(['/', '\\']) {
        if seg.is_empty() || seg == "." {
            continue;
        }
        if seg == ".." {
            return Err(PgError::InvalidData(format!(
                "Archive entry '{trimmed_nul}' contains '..' path traversal"
            )));
        }
        let clean_seg = validate_single_entry_name(seg)?;
        rel_path.push(clean_seg);
    }

    if rel_path.as_os_str().is_empty() {
        let fb = validate_single_entry_name(fallback_name)?;
        rel_path.push(fb);
    }

    for comp in rel_path.components() {
        if !matches!(comp, Component::Normal(_)) {
            return Err(PgError::InvalidData(format!(
                "Archive entry '{trimmed_nul}' resolved to non-normal path component"
            )));
        }
    }

    let joined = base_dir.join(&rel_path);
    if !joined.starts_with(base_dir) {
        return Err(PgError::InvalidData(format!(
            "Archive entry '{trimmed_nul}' escaped destination directory"
        )));
    }
    Ok(joined)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Endian {
    Little,
    Big,
}

impl Endian {
    #[inline]
    pub const fn is_big(self) -> bool {
        matches!(self, Endian::Big)
    }

    #[inline]
    pub const fn is_little(self) -> bool {
        matches!(self, Endian::Little)
    }

    #[inline]
    pub const fn swap(self) -> Self {
        match self {
            Endian::Little => Endian::Big,
            Endian::Big => Endian::Little,
        }
    }
}

/// Align `val` upward to the next multiple of `alignment` (matches `Bayonetta::Alignment#align`).
#[inline]
pub const fn align(val: usize, alignment: usize) -> usize {
    if alignment == 0 {
        return val;
    }
    let rem = val % alignment;
    if rem > 0 {
        val + (alignment - rem)
    } else {
        val
    }
}

/// Align `u64` offset upward to the next multiple of `alignment`.
#[inline]
pub const fn align_u64(val: u64, alignment: u64) -> u64 {
    if alignment == 0 {
        return val;
    }
    let rem = val % alignment;
    if rem > 0 {
        val + (alignment - rem)
    } else {
        val
    }
}

/// PlatinumGames custom 16-bit floating point (`pghalf` in `libbin` and `.bt` templates):
/// 1 sign bit (`0x8000`), 6 exponent bits (`0x7E00`), 9 significand bits (`0x01FF`), exponent bias = 47.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct PgHalf(pub u16);

impl PgHalf {
    pub const ZERO: Self = Self(0);

    pub fn to_f32(self) -> f32 {
        let ui = self.0 as u32;
        let sign = (ui & 0x8000) != 0;
        let exp = (ui & 0x7E00) >> 9;
        let sig = ui & 0x01FF;

        if exp == 0x3F {
            if sig != 0 {
                return f32::NAN;
            } else if sign {
                return f32::NEG_INFINITY;
            } else {
                return f32::INFINITY;
            }
        }

        let frac = (sig as f64) / 512.0;
        let val = if exp != 0 {
            let sexp = (exp as i32) - 47;
            (1.0 + frac) * 2.0_f64.powi(sexp)
        } else if sig != 0 {
            frac * 2.0_f64.powi(-46)
        } else {
            0.0
        };

        if sign {
            -(val as f32)
        } else {
            val as f32
        }
    }

    pub fn from_f32(v: f32) -> Self {
        if v.is_nan() {
            return Self(0x7FFF);
        }
        let sign: u16 = if v.is_sign_negative() { 0x8000 } else { 0 };
        let abs = v.abs() as f64;
        if abs == 0.0 {
            return Self(sign);
        }
        if abs.is_infinite() {
            return Self(sign | 0x7E00);
        }
        let log2 = abs.log2().floor() as i32;
        let mut biased_exp = log2 + 47;
        if biased_exp >= 63 {
            return Self(sign | 0x7E00);
        }
        if biased_exp <= 0 {
            // Subnormal
            let frac = abs / 2.0_f64.powi(-46);
            let rounded = (frac * 512.0).round() as i32;
            if rounded >= 512 {
                return Self(sign | (1u16 << 9));
            }
            let sig = rounded.clamp(0, 511) as u16;
            return Self(sign | sig);
        }
        let norm = abs / 2.0_f64.powi(log2) - 1.0;
        let mut sig_i32 = (norm * 512.0).round() as i32;
        if sig_i32 >= 512 {
            sig_i32 = 0;
            biased_exp += 1;
            if biased_exp >= 63 {
                return Self(sign | 0x7E00);
            }
        }
        let sig = sig_i32.clamp(0, 511) as u16;
        Self(sign | ((biased_exp as u16) << 9) | sig)
    }
}

/// Standard IEEE 754 16-bit half-precision float (1-sign, 5-exponent, 10-mantissa, bias 15).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct F16(pub u16);

impl F16 {
    pub fn to_f32(self) -> f32 {
        let h = self.0 as u32;
        let sign = (h & 0x8000) << 16;
        let exp = (h >> 10) & 0x1F;
        let mant = h & 0x03FF;

        let bits = if exp == 0 {
            if mant == 0 {
                sign
            } else {
                let mut m = mant;
                let mut e = 127 - 14;
                while (m & 0x0400) == 0 {
                    m <<= 1;
                    e -= 1;
                }
                m &= 0x03FF;
                sign | ((e as u32) << 23) | (m << 13)
            }
        } else if exp == 31 {
            sign | 0x7F80_0000 | (mant << 13)
        } else {
            sign | ((exp + 127 - 15) << 23) | (mant << 13)
        };
        f32::from_bits(bits)
    }

    pub fn from_f32(v: f32) -> Self {
        let bits = v.to_bits();
        let sign = ((bits >> 16) & 0x8000) as u16;
        let exp = ((bits >> 23) & 0xFF) as i32 - 127 + 15;
        let mant = bits & 0x007F_FFFF;

        if exp <= 0 {
            if exp < -10 {
                return Self(sign);
            }
            let m = (mant | 0x0080_0000) >> (1 - exp + 13);
            Self(sign | (m as u16))
        } else if exp >= 31 {
            Self(sign | 0x7C00 | if v.is_nan() { 0x0200 } else { 0 })
        } else {
            Self(sign | ((exp as u16) << 10) | ((mant >> 13) as u16))
        }
    }
}

/// Fast in-memory endian-aware binary reader over a byte slice.
#[derive(Debug, Clone)]
pub struct BinReader<'a> {
    data: &'a [u8],
    pos: usize,
    pub endian: Endian,
}

impl<'a> BinReader<'a> {
    pub fn new(data: &'a [u8], endian: Endian) -> Self {
        Self { data, pos: 0, endian }
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.data.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    #[inline]
    pub fn pos(&self) -> usize {
        self.pos
    }

    #[inline]
    pub fn data(&self) -> &'a [u8] {
        self.data
    }

    #[inline]
    pub fn seek(&mut self, offset: usize) -> Result<()> {
        if offset > self.data.len() {
            return Err(PgError::InvalidData(format!(
                "Seek out of bounds: 0x{:X} > 0x{:X}",
                offset,
                self.data.len()
            )));
        }
        self.pos = offset;
        Ok(())
    }

    pub fn read_bytes(&mut self, count: usize) -> Result<&'a [u8]> {
        let end = self.pos.checked_add(count).ok_or_else(|| {
            PgError::InvalidData("Read overflow".to_string())
        })?;
        if end > self.data.len() {
            return Err(PgError::InvalidData(format!(
                "Unexpected EOF reading {} bytes at 0x{:X} (len 0x{:X})",
                count,
                self.pos,
                self.data.len()
            )));
        }
        let slice = &self.data[self.pos..end];
        self.pos = end;
        Ok(slice)
    }

    pub fn slice_at(&self, offset: usize, count: usize) -> Result<&'a [u8]> {
        let end = offset.checked_add(count).ok_or_else(|| {
            PgError::InvalidData("Slice overflow".to_string())
        })?;
        if end > self.data.len() {
            return Err(PgError::InvalidData(format!(
                "Slice out of bounds: 0x{:X}..0x{:X} (len 0x{:X})",
                offset,
                end,
                self.data.len()
            )));
        }
        Ok(&self.data[offset..end])
    }

    #[inline]
    pub fn read_u8(&mut self) -> Result<u8> {
        Ok(self.read_bytes(1)?[0])
    }

    #[inline]
    pub fn read_i8(&mut self) -> Result<i8> {
        Ok(self.read_u8()? as i8)
    }

    #[inline]
    pub fn read_u16(&mut self) -> Result<u16> {
        let b = self.read_bytes(2)?;
        let arr = [b[0], b[1]];
        Ok(match self.endian {
            Endian::Little => u16::from_le_bytes(arr),
            Endian::Big => u16::from_be_bytes(arr),
        })
    }

    #[inline]
    pub fn read_i16(&mut self) -> Result<i16> {
        Ok(self.read_u16()? as i16)
    }

    #[inline]
    pub fn read_u32(&mut self) -> Result<u32> {
        let b = self.read_bytes(4)?;
        let arr = [b[0], b[1], b[2], b[3]];
        Ok(match self.endian {
            Endian::Little => u32::from_le_bytes(arr),
            Endian::Big => u32::from_be_bytes(arr),
        })
    }

    #[inline]
    pub fn read_i32(&mut self) -> Result<i32> {
        Ok(self.read_u32()? as i32)
    }

    #[inline]
    pub fn read_u64(&mut self) -> Result<u64> {
        let b = self.read_bytes(8)?;
        let arr = [b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]];
        Ok(match self.endian {
            Endian::Little => u64::from_le_bytes(arr),
            Endian::Big => u64::from_be_bytes(arr),
        })
    }

    #[inline]
    pub fn read_f32(&mut self) -> Result<f32> {
        Ok(f32::from_bits(self.read_u32()?))
    }

    #[inline]
    pub fn read_f16(&mut self) -> Result<f32> {
        Ok(F16(self.read_u16()?).to_f32())
    }

    #[inline]
    pub fn read_pghalf(&mut self) -> Result<f32> {
        Ok(PgHalf(self.read_u16()?).to_f32())
    }

    pub fn read_fixed_string(&mut self, len: usize) -> Result<String> {
        let raw = self.read_bytes(len)?;
        let end = raw.iter().position(|&b| b == 0).unwrap_or(len);
        Ok(String::from_utf8_lossy(&raw[..end]).into_owned())
    }

    pub fn read_cstring_at(&self, offset: usize) -> Result<String> {
        if offset >= self.data.len() {
            return Err(PgError::InvalidData(format!(
                "CString offset out of bounds: 0x{:X}",
                offset
            )));
        }
        let rem = &self.data[offset..];
        let end = rem.iter().position(|&b| b == 0).unwrap_or(rem.len());
        Ok(String::from_utf8_lossy(&rem[..end]).into_owned())
    }
}

/// Random-access endian-aware binary writer backed by a growable `Vec<u8>`.
#[derive(Debug, Clone)]
pub struct BinWriter {
    buf: Vec<u8>,
    pos: usize,
    pub endian: Endian,
}

impl BinWriter {
    pub fn new(endian: Endian) -> Self {
        Self {
            buf: Vec::new(),
            pos: 0,
            endian,
        }
    }

    pub fn with_capacity(cap: usize, endian: Endian) -> Self {
        Self {
            buf: vec![0u8; cap],
            pos: 0,
            endian,
        }
    }

    pub fn into_inner(self) -> Vec<u8> {
        self.buf
    }

    pub fn as_slice(&self) -> &[u8] {
        &self.buf
    }

    #[inline]
    pub fn pos(&self) -> usize {
        self.pos
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.buf.len()
    }

    pub fn seek(&mut self, offset: usize) {
        if offset > self.buf.len() {
            self.buf.resize(offset, 0);
        }
        self.pos = offset;
    }

    pub fn align_to(&mut self, alignment: usize) {
        let target = align(self.pos, alignment);
        self.seek(target);
    }

    pub fn write_bytes(&mut self, bytes: &[u8]) {
        let end = self.pos + bytes.len();
        if end > self.buf.len() {
            self.buf.resize(end, 0);
        }
        self.buf[self.pos..end].copy_from_slice(bytes);
        self.pos = end;
    }

    #[inline]
    pub fn write_u8(&mut self, v: u8) {
        self.write_bytes(&[v]);
    }

    #[inline]
    pub fn write_i8(&mut self, v: i8) {
        self.write_u8(v as u8);
    }

    #[inline]
    pub fn write_u16(&mut self, v: u16) {
        let b = match self.endian {
            Endian::Little => v.to_le_bytes(),
            Endian::Big => v.to_be_bytes(),
        };
        self.write_bytes(&b);
    }

    #[inline]
    pub fn write_i16(&mut self, v: i16) {
        self.write_u16(v as u16);
    }

    #[inline]
    pub fn write_u32(&mut self, v: u32) {
        let b = match self.endian {
            Endian::Little => v.to_le_bytes(),
            Endian::Big => v.to_be_bytes(),
        };
        self.write_bytes(&b);
    }

    #[inline]
    pub fn write_i32(&mut self, v: i32) {
        self.write_u32(v as u32);
    }

    #[inline]
    pub fn write_u64(&mut self, v: u64) {
        let b = match self.endian {
            Endian::Little => v.to_le_bytes(),
            Endian::Big => v.to_be_bytes(),
        };
        self.write_bytes(&b);
    }

    #[inline]
    pub fn write_f32(&mut self, v: f32) {
        self.write_u32(v.to_bits());
    }

    #[inline]
    pub fn write_f16(&mut self, v: f32) {
        self.write_u16(F16::from_f32(v).0);
    }

    #[inline]
    pub fn write_pghalf(&mut self, v: f32) {
        self.write_u16(PgHalf::from_f32(v).0);
    }

    pub fn write_fixed_string(&mut self, s: &str, len: usize) {
        let mut tmp = vec![0u8; len];
        let bytes = s.as_bytes();
        let copy_len = bytes.len().min(len);
        tmp[..copy_len].copy_from_slice(&bytes[..copy_len]);
        self.write_bytes(&tmp);
    }

    pub fn write_cstring(&mut self, s: &str) {
        self.write_bytes(s.as_bytes());
        self.write_u8(0);
    }
}
