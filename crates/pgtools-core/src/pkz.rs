//! PlatinumGames `.pkz` archive reader, decompressor, and repacker.
//!
//! Matches both `PKZ-Explorer` (`PkzArchive.cs`, `PkzWriter.cs`, `CompressionCodec.cs`, `Oodle.cs`,
//! `OodleLocator.cs` for Bayonetta 3 `pkzl` v2 with 32-byte descriptors) and `bayonetta_tools`
//! (`lib/bayonetta/pkz.rb` for Astral Chain `PKZL`/`PKZB` v1 with 40-byte descriptors).
//!
//! Supports:
//! - Uncompressed entries (`None` / `""`)
//! - Native pure-Rust `zstd` compression and decompression (`ZStandard`, `Zstd`, `zstd`) with levels `1..=22`
//! - Dynamic user-supplied or application-directory `OodleKraken` DLL/SO (`oo2core_*.dll` / `liboo2core.so`)
//!   with both `OodleLZ_Decompress` and `OodleLZ_Compress` (levels `1..=9` and 100-byte seek-chunk options)
//! - Built-in pure-Rust open-source Oodle Kraken (`ooz`) fallback decompressor when `oo2core_*.dll`
//!   is not available on disk
//! - PKZ Explorer's 90% worthwhile compression threshold (`comp.len() < raw.len() * 0.90`, with optional
//!   `force_compression` override), storing entries uncompressed (`None`) when compression does not
//!   shrink the file by at least 10%
//! - Recursive directory enumeration (`build_from_dir`) preserving relative virtual paths with `/` separators.

use std::collections::HashMap;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::io::{
    align, checked_count, sanitize_entry_path, validate_single_entry_name, BinReader, BinWriter,
    Endian, MAX_ARCHIVE_ALLOC_BYTES,
};
use crate::oodle_fallback::decompress_oodle_fallback;
use crate::{PgError, Result};

/// Default ZStandard compression level, matching `CompressionCodec.DefaultZstdLevel`.
pub const DEFAULT_ZSTD_LEVEL: i32 = 3;
/// Minimum ZStandard compression level.
pub const MIN_ZSTD_LEVEL: i32 = 1;
/// Maximum ZStandard compression level.
pub const MAX_ZSTD_LEVEL: i32 = 22;

/// Default Oodle Kraken compression level (`Normal` = 4), matching `Oodle.DefaultKrakenLevel`.
pub const DEFAULT_OODLE_LEVEL: i32 = 4;
/// Minimum Oodle Kraken compression level (`SuperFast` = 1).
pub const MIN_OODLE_LEVEL: i32 = 1;
/// Maximum Oodle Kraken compression level (`Optimal5` = 9).
pub const MAX_OODLE_LEVEL: i32 = 9;

/// Worthwhile compression ratio numerator (`0.90` = 9 / 10).
/// Entries whose compressed size is `>= 90%` of the uncompressed size are stored as `None`
/// unless `force_compression` is enabled.
pub const COMPRESSION_WORTHWHILE_NUM: u128 = 9;
pub const COMPRESSION_WORTHWHILE_DEN: u128 = 10;

/// Return the default compression level for a codec name (`ZStandard` -> 3, `OodleKraken` -> 4, `None` -> 0).
pub fn default_compression_level(codec: &str) -> i32 {
    let low = codec.trim().to_ascii_lowercase();
    if low.contains("oodle") || low.contains("kraken") {
        DEFAULT_OODLE_LEVEL
    } else if low == "none" || low.is_empty() {
        0
    } else {
        DEFAULT_ZSTD_LEVEL
    }
}

/// Clamp a requested compression level into the valid range for `codec`.
pub fn clamp_compression_level(codec: &str, level: i32) -> i32 {
    let low = codec.trim().to_ascii_lowercase();
    if low.contains("oodle") || low.contains("kraken") {
        level.clamp(MIN_OODLE_LEVEL, MAX_OODLE_LEVEL)
    } else if low == "none" || low.is_empty() {
        0
    } else {
        level.clamp(MIN_ZSTD_LEVEL, MAX_ZSTD_LEVEL)
    }
}

/// Returns true when `compressed_len` shrinks `raw_len` below the 90% threshold (or when `force` is set).
#[inline]
pub fn is_compression_worthwhile(compressed_len: usize, raw_len: usize, force: bool) -> bool {
    if raw_len == 0 {
        return false;
    }
    if force {
        return true;
    }
    (compressed_len as u128) * COMPRESSION_WORTHWHILE_DEN
        < (raw_len as u128) * COMPRESSION_WORTHWHILE_NUM
}

/// Normalize a virtual path inside a `.pkz` archive:
/// - Converts `\` to `/`
/// - Strips leading/trailing `/`
/// - Validates each path segment via `validate_single_entry_name` (rejecting `.`, `..`, colons, reserved device names)
pub fn normalize_pkz_virtual_path(raw_path: &str) -> Result<String> {
    let normalized = raw_path.trim().replace('\\', "/");
    let mut segments: Vec<String> = Vec::new();
    for seg in normalized.split('/') {
        let trimmed = seg.trim();
        if trimmed.is_empty() {
            continue;
        }
        let valid = validate_single_entry_name(trimmed)?;
        segments.push(valid);
    }
    if segments.is_empty() {
        return Err(PgError::InvalidData(
            "PKZ virtual path cannot be empty".to_string(),
        ));
    }
    Ok(segments.join("/"))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PkzFileDescriptor {
    pub name: String,
    pub compression: String,
    pub size: u64,
    pub offset: u64,
    pub compressed_size: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PkzArchive {
    pub endian: Endian,
    /// True if header magic was lowercase `pkzl` (Bayonetta 3), false if `PKZL`/`PKZB` (Astral Chain).
    pub is_lowercase_magic: bool,
    /// True if file descriptors are 32 bytes (`u32` name/compression offsets, Bayonetta 3 `0x20`),
    /// false if 40 bytes (`u64` name/compression offsets, Astral Chain `0x28`).
    pub is_32byte_descriptors: bool,
    pub unknown: u32,
    pub descriptors: Vec<PkzFileDescriptor>,
}

/// Optional Oodle Kraken dynamic library wrapper (`oo2core_9_win64.dll`, `oo2core_8_win64.dll`, etc.).
pub struct OodleLibrary {
    lib: libloading::Library,
    pub resolved_path: PathBuf,
}

impl OodleLibrary {
    /// Locate an `oo2core*.dll` / `liboo2core.so` on disk safely:
    /// 1. Explicit path supplied by caller/settings (must be an existing file)
    /// 2. Application executable directory (`oo2core*.dll`)
    /// Never searches the current working directory (CWD) or `PATH` to prevent DLL planting.
    pub fn find_dll_path(explicit_path: Option<&Path>) -> Option<PathBuf> {
        if let Some(p) = explicit_path {
            if !p.as_os_str().is_empty() && p.is_file() {
                return Some(fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf()));
            }
        }

        let mut search_dirs: Vec<PathBuf> = Vec::new();
        if let Ok(exe) = std::env::current_exe() {
            if let Some(parent) = exe.parent() {
                search_dirs.push(parent.to_path_buf());
            }
        }

        let fixed_candidates = [
            "oo2core_9_win64.dll",
            "oo2core_8_win64.dll",
            "oo2core_7_win64.dll",
            "oo2core_6_win64.dll",
            "oo2core_5_win64.dll",
            "oo2core_4_win64.dll",
            "liboo2core.so",
            "liboo2core.dylib",
        ];

        for dir in &search_dirs {
            for cand in &fixed_candidates {
                let candidate = dir.join(cand);
                if candidate.is_file() {
                    return Some(fs::canonicalize(&candidate).unwrap_or(candidate));
                }
            }
            if let Ok(rd) = fs::read_dir(dir) {
                let mut matches: Vec<PathBuf> = rd
                    .filter_map(|e| e.ok())
                    .map(|e| e.path())
                    .filter(|p| {
                        p.is_file()
                            && p.file_name()
                                .and_then(|n| n.to_str())
                                .map(|s| {
                                    let low = s.to_ascii_lowercase();
                                    low.starts_with("oo2core")
                                        && (low.ends_with(".dll")
                                            || low.ends_with(".so")
                                            || low.ends_with(".dylib"))
                                })
                                .unwrap_or(false)
                    })
                    .collect();
                matches.sort();
                if let Some(best) = matches.pop() {
                    return Some(fs::canonicalize(&best).unwrap_or(best));
                }
            }
        }

        None
    }

    /// Load a user-supplied or application-directory Oodle shared library by absolute path only.
    pub fn load(explicit_path: Option<&Path>) -> Result<Self> {
        if let Some(found) = Self::find_dll_path(explicit_path) {
            let abs_found = fs::canonicalize(&found).unwrap_or(found);
            let lib = unsafe { libloading::Library::new(&abs_found) }.map_err(|e| {
                PgError::Unsupported(format!(
                    "Failed to load Oodle library {}: {e}",
                    abs_found.display()
                ))
            })?;
            return Ok(Self {
                lib,
                resolved_path: abs_found,
            });
        }

        Err(PgError::Unsupported(
            "Oodle Kraken library (oo2core_*_win64.dll) not found.".to_string(),
        ))
    }

    /// True when the loaded library exports `OodleLZ_Decompress`.
    pub fn can_decompress(&self) -> bool {
        unsafe {
            self.lib
                .get::<*const std::ffi::c_void>(b"OodleLZ_Decompress\0")
                .is_ok()
                || self
                    .lib
                    .get::<*const std::ffi::c_void>(b"_OodleLZ_Decompress@56\0")
                    .is_ok()
        }
    }

    /// True when the loaded library exports `OodleLZ_Compress`.
    pub fn can_compress(&self) -> bool {
        unsafe {
            self.lib
                .get::<*const std::ffi::c_void>(b"OodleLZ_Compress\0")
                .is_ok()
                || self
                    .lib
                    .get::<*const std::ffi::c_void>(b"_OodleLZ_Compress@40\0")
                    .is_ok()
        }
    }

    pub fn decompress(&self, compressed: &[u8], uncompressed_size: usize) -> Result<Vec<u8>> {
        if uncompressed_size == 0 {
            return Ok(Vec::new());
        }
        if uncompressed_size > MAX_ARCHIVE_ALLOC_BYTES {
            return Err(PgError::InvalidData(format!(
                "Oodle uncompressed size {uncompressed_size} exceeds safety limit ({MAX_ARCHIVE_ALLOC_BYTES} bytes)"
            )));
        }
        if compressed.len() == uncompressed_size {
            return Ok(compressed.to_vec());
        }
        type OodleLzDecompress = unsafe extern "C" fn(
            comp_buf: *const u8,
            comp_buf_size: isize,
            raw_buf: *mut u8,
            raw_len: isize,
            fuzz_safe: i32,
            check_crc: i32,
            verbosity: i32,
            dec_buf_base: *mut u8,
            dec_buf_size: isize,
            fp_callback: *mut std::ffi::c_void,
            callback_user_data: *mut std::ffi::c_void,
            decoder_memory: *mut u8,
            decoder_memory_size: isize,
            thread_phase: i32,
        ) -> isize;

        let func: libloading::Symbol<OodleLzDecompress> = unsafe {
            self.lib
                .get(b"OodleLZ_Decompress\0")
                .or_else(|_| self.lib.get(b"_OodleLZ_Decompress@56\0"))
                .map_err(|e| {
                    PgError::Unsupported(format!("OodleLZ_Decompress symbol not found: {e}"))
                })?
        };

        let mut out = vec![0u8; uncompressed_size];
        let decoded = unsafe {
            func(
                compressed.as_ptr(),
                compressed.len() as isize,
                out.as_mut_ptr(),
                uncompressed_size as isize,
                1,
                0,
                0,
                std::ptr::null_mut(),
                0,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                0,
                3,
            )
        };

        if decoded != uncompressed_size as isize {
            return Err(PgError::InvalidData(format!(
                "OodleLZ_Decompress failed: expected {uncompressed_size} bytes, got {decoded}"
            )));
        }

        Ok(out)
    }

    /// Decompress using `oo2core_*.dll` if available, or fall back to the built-in pure-Rust
    /// open-source Oodle Kraken (`ooz`) decoder.
    pub fn decompress_with_fallback(
        explicit_path: Option<&Path>,
        compressed: &[u8],
        uncompressed_size: usize,
    ) -> Result<Vec<u8>> {
        if uncompressed_size > MAX_ARCHIVE_ALLOC_BYTES {
            return Err(PgError::InvalidData(format!(
                "Oodle uncompressed size {uncompressed_size} exceeds safety limit ({MAX_ARCHIVE_ALLOC_BYTES} bytes)"
            )));
        }
        if let Ok(lib) = Self::load(explicit_path) {
            if let Ok(out) = lib.decompress(compressed, uncompressed_size) {
                return Ok(out);
            }
        }
        decompress_oodle_fallback(compressed, uncompressed_size)
    }

    /// Compress `raw` using `OodleLZ_Compress` (Kraken = 8, Default level = 4).
    pub fn compress(&self, raw: &[u8]) -> Result<Vec<u8>> {
        self.compress_with_level(raw, DEFAULT_OODLE_LEVEL)
    }

    /// Compress `raw` using `OodleLZ_Compress` (Kraken = 8) at `level` (`1..=9`),
    /// passing the 100-byte options struct (`seekChunkReset = 1`, `seekChunkLen = 0x40000`)
    /// matching `Oodle.cs` in `PKZ-Explorer`.
    pub fn compress_with_level(&self, raw: &[u8], level: i32) -> Result<Vec<u8>> {
        if raw.is_empty() {
            return Ok(Vec::new());
        }
        type OodleLzCompress = unsafe extern "C" fn(
            compressor: i32,
            raw_buf: *const u8,
            raw_len: isize,
            comp_buf: *mut u8,
            level: i32,
            p_options: *const std::ffi::c_void,
            dictionary_base: *const u8,
            lrm: *const std::ffi::c_void,
            scratch_mem: *mut u8,
            scratch_size: isize,
        ) -> isize;

        let func: libloading::Symbol<OodleLzCompress> = unsafe {
            self.lib
                .get(b"OodleLZ_Compress\0")
                .or_else(|_| self.lib.get(b"_OodleLZ_Compress@40\0"))
                .map_err(|e| {
                    PgError::Unsupported(format!("OodleLZ_Compress symbol not found: {e}"))
                })?
        };

        let clamped_level = clamp_compression_level("OodleKraken", level);
        let mut options = [0u8; 100];
        options[8..12].copy_from_slice(&1i32.to_le_bytes()); // seekChunkReset = 1
        options[12..16].copy_from_slice(&0x40000i32.to_le_bytes()); // seekChunkLen = 0x40000

        let bound = raw.len() + 65536usize.max(274 * ((raw.len() + 0x3FFFF) / 0x40000) + 64);
        let mut comp = vec![0u8; bound];
        let written = unsafe {
            func(
                8, // OodleFormat.Kraken
                raw.as_ptr(),
                raw.len() as isize,
                comp.as_mut_ptr(),
                clamped_level,
                options.as_ptr() as *const std::ffi::c_void,
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null_mut(),
                0,
            )
        };
        if written <= 0 {
            return Err(PgError::InvalidData(format!(
                "OodleLZ_Compress failed with return code {written}"
            )));
        }
        comp.truncate(written as usize);
        Ok(comp)
    }
}

#[derive(Debug, Clone, Copy)]
struct RawFileDescriptor {
    name_offset: u64,
    compression_offset: u64,
    size: u64,
    offset: u64,
    compressed_size: u64,
}

impl PkzArchive {
    /// Parse a `.pkz` archive header and file table, supporting both Bayonetta 3 (`pkzl`, 32-byte
    /// descriptors) and Astral Chain (`PKZL`/`PKZB`, 40-byte descriptors).
    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < 32 {
            return Err(PgError::InvalidData("PKZ buffer shorter than 32 bytes".into()));
        }
        let magic = &bytes[0..4];
        let (endian, is_lowercase_magic) = match magic {
            b"pkzl" => (Endian::Little, true),
            b"PKZL" => (Endian::Little, false),
            b"PKZB" => (Endian::Big, false),
            _ => {
                return Err(PgError::InvalidMagic {
                    expected: "pkzl, PKZL, or PKZB",
                    actual: magic.to_vec(),
                })
            }
        };

        let mut r = BinReader::new(bytes, endian);
        r.seek(4)?;
        let unknown = r.read_u32()?;
        let _total_size = r.read_u64()?;
        let num_files = r.read_u32()? as usize;
        let field_14 = r.read_u32()? as usize;
        let file_name_table_length = r.read_u64()? as usize;

        let is_32byte_descriptors = if field_14 == 0x28 {
            false
        } else if is_lowercase_magic || unknown == 0x0002_0000 {
            true
        } else {
            let str_base_32 = 0x20usize.saturating_add(num_files.saturating_mul(32));
            let str_base_40 = 0x20usize.saturating_add(num_files.saturating_mul(40));
            str_base_40.saturating_add(file_name_table_length) > bytes.len()
                && str_base_32.saturating_add(file_name_table_length) <= bytes.len()
        };

        let offset_file_descriptors = 0x20usize;
        let desc_stride = if is_32byte_descriptors { 32 } else { 40 };
        checked_count(
            num_files,
            desc_stride,
            bytes.len().saturating_sub(offset_file_descriptors),
            "PKZ descriptors",
        )?;
        r.seek(offset_file_descriptors)?;
        let mut raw_descriptors = Vec::with_capacity(num_files);
        for _ in 0..num_files {
            if is_32byte_descriptors {
                raw_descriptors.push(RawFileDescriptor {
                    name_offset: r.read_u32()? as u64,
                    compression_offset: r.read_u32()? as u64,
                    size: r.read_u64()?,
                    offset: r.read_u64()?,
                    compressed_size: r.read_u64()?,
                });
            } else {
                raw_descriptors.push(RawFileDescriptor {
                    name_offset: r.read_u64()?,
                    compression_offset: r.read_u64()?,
                    size: r.read_u64()?,
                    offset: r.read_u64()?,
                    compressed_size: r.read_u64()?,
                });
            }
        }

        let string_table_base = r.pos();
        let mut descriptors = Vec::with_capacity(num_files);
        for desc in raw_descriptors {
            let name = string_table_base
                .checked_add(desc.name_offset as usize)
                .and_then(|off| r.read_cstring_at(off).ok())
                .unwrap_or_default();
            let compression = string_table_base
                .checked_add(desc.compression_offset as usize)
                .and_then(|off| r.read_cstring_at(off).ok())
                .unwrap_or_else(|| "None".to_string());
            descriptors.push(PkzFileDescriptor {
                name,
                compression,
                size: desc.size,
                offset: desc.offset,
                compressed_size: desc.compressed_size,
            });
        }

        Ok(Self {
            endian,
            is_lowercase_magic,
            is_32byte_descriptors,
            unknown,
            descriptors,
        })
    }

    /// Returns true if any entry in the archive uses OodleKraken compression.
    pub fn uses_oodle(&self) -> bool {
        self.descriptors.iter().any(|d| {
            d.compression.eq_ignore_ascii_case("OodleKraken") && d.compressed_size != d.size
        })
    }

    /// Return the raw stored compressed slice for entry `index`.
    pub fn raw_entry_slice<'a>(&self, archive_bytes: &'a [u8], index: usize) -> Result<&'a [u8]> {
        let desc = self
            .descriptors
            .get(index)
            .ok_or_else(|| PgError::InvalidData(format!("PKZ entry index {index} out of bounds")))?;
        let start = desc.offset as usize;
        let end = start.saturating_add(desc.compressed_size as usize);
        if end > archive_bytes.len() {
            return Err(PgError::InvalidData(format!(
                "PKZ entry {} slice ({}..{}) out of bounds (len {})",
                desc.name,
                start,
                end,
                archive_bytes.len()
            )));
        }
        Ok(&archive_bytes[start..end])
    }

    /// Extract and decompress a single entry by index from the `.pkz` archive bytes.
    /// Automatically uses the pure-Rust `ooz` Oodle Kraken fallback when `oo2core_*.dll` is absent.
    pub fn extract_entry(
        &self,
        archive_bytes: &[u8],
        index: usize,
        oodle_dll: Option<&Path>,
    ) -> Result<Vec<u8>> {
        let oodle_loader = OodleLibrary::load(oodle_dll).ok();
        self.extract_entry_with_lib(archive_bytes, index, oodle_loader.as_ref())
    }

    /// Extract and decompress a single entry using a pre-loaded `OodleLibrary` handle (if any).
    pub fn extract_entry_with_lib(
        &self,
        archive_bytes: &[u8],
        index: usize,
        oodle_lib: Option<&OodleLibrary>,
    ) -> Result<Vec<u8>> {
        let desc = self
            .descriptors
            .get(index)
            .ok_or_else(|| PgError::InvalidData(format!("PKZ entry index {index} out of bounds")))?;
        if desc.size > MAX_ARCHIVE_ALLOC_BYTES as u64 {
            return Err(PgError::InvalidData(format!(
                "PKZ entry {} uncompressed size ({}) exceeds safety limit ({MAX_ARCHIVE_ALLOC_BYTES})",
                desc.name, desc.size
            )));
        }
        let raw_slice = self.raw_entry_slice(archive_bytes, index)?;
        let comp_lower = desc.compression.to_ascii_lowercase();
        if comp_lower.is_empty() || comp_lower == "none" {
            return Ok(raw_slice.to_vec());
        }
        if comp_lower == "zstandard" || comp_lower == "zstd" {
            if desc.compressed_size == desc.size && !raw_slice.starts_with(&[0x28, 0xB5, 0x2F, 0xFD])
            {
                return Ok(raw_slice.to_vec());
            }
            let max_out = (desc.size as usize)
                .max(1024)
                .min(MAX_ARCHIVE_ALLOC_BYTES);
            let decoder = zstd::stream::read::Decoder::new(raw_slice).map_err(|e| {
                PgError::InvalidData(format!("Zstd init error on {}: {e}", desc.name))
            })?;
            let mut limited = decoder.take((max_out as u64) + 1);
            let mut out = Vec::with_capacity((desc.size as usize).min(16 * 1024 * 1024));
            limited.read_to_end(&mut out).map_err(|e| {
                PgError::InvalidData(format!("Zstd decompression error on {}: {e}", desc.name))
            })?;
            if out.len() > max_out {
                return Err(PgError::InvalidData(format!(
                    "Zstd decompression on {} exceeded expected size ({max_out} bytes)",
                    desc.name
                )));
            }
            return Ok(out);
        }
        if comp_lower == "oodlekraken" || comp_lower == "oodle" || comp_lower == "kraken" {
            if desc.compressed_size == desc.size {
                return Ok(raw_slice.to_vec());
            }
            let target_size = desc.size as usize;
            if let Some(lib) = oodle_lib {
                if let Ok(out) = lib.decompress(raw_slice, target_size) {
                    return Ok(out);
                }
            }
            return decompress_oodle_fallback(raw_slice, target_size);
        }
        Err(PgError::Unsupported(format!(
            "Unsupported PKZ compression '{}' on {}",
            desc.compression, desc.name
        )))
    }

    /// Extract all entries from the `.pkz` archive into `dest_dir`.
    pub fn extract_all(
        &self,
        archive_bytes: &[u8],
        dest_dir: &Path,
        oodle_dll: Option<&Path>,
    ) -> Result<Vec<PathBuf>> {
        fs::create_dir_all(dest_dir)?;
        let mut extracted = Vec::with_capacity(self.descriptors.len());
        let oodle_loader: Option<OodleLibrary> = OodleLibrary::load(oodle_dll).ok();

        for (i, desc) in self.descriptors.iter().enumerate() {
            let data = self.extract_entry_with_lib(archive_bytes, i, oodle_loader.as_ref())?;
            let fallback = format!("entry_{i}.bin");
            let out_path = sanitize_entry_path(dest_dir, &desc.name, &fallback)?;
            if let Some(parent) = out_path.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(&out_path, data)?;
            extracted.push(out_path);
        }

        Ok(extracted)
    }

    /// Repack an existing `PkzArchive` while preserving untouched compressed slices verbatim
    /// (even if Oodle DLL is unavailable!) and compressing modified/added entries with Oodle
    /// (if DLL is available and requested) or falling back to `ZStandard` / `Zstd` / `None`.
    pub fn repack_with_replacements(
        &self,
        original_archive_bytes: &[u8],
        replacements: &HashMap<usize, Vec<u8>>,
        added_files: &[(String, Vec<u8>)],
        removed_indices: &[usize],
        reorder_map: Option<&[usize]>,
        preferred_compression: Option<&str>,
        oodle_dll: Option<&Path>,
    ) -> Result<Vec<u8>> {
        self.repack_with_options(
            original_archive_bytes,
            replacements,
            added_files,
            removed_indices,
            reorder_map,
            preferred_compression,
            None,
            None,
            oodle_dll,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn repack_with_options(
        &self,
        original_archive_bytes: &[u8],
        replacements: &HashMap<usize, Vec<u8>>,
        added_files: &[(String, Vec<u8>)],
        removed_indices: &[usize],
        reorder_map: Option<&[usize]>,
        preferred_compression: Option<&str>,
        compression_overrides: Option<&HashMap<usize, String>>,
        renamed_entries: Option<&HashMap<usize, String>>,
        oodle_dll: Option<&Path>,
    ) -> Result<Vec<u8>> {
        self.repack_with_full_options(
            original_archive_bytes,
            replacements,
            added_files,
            removed_indices,
            reorder_map,
            preferred_compression,
            compression_overrides,
            renamed_entries,
            None,
            false,
            oodle_dll,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn repack_with_full_options(
        &self,
        original_archive_bytes: &[u8],
        replacements: &HashMap<usize, Vec<u8>>,
        added_files: &[(String, Vec<u8>)],
        removed_indices: &[usize],
        reorder_map: Option<&[usize]>,
        preferred_compression: Option<&str>,
        compression_overrides: Option<&HashMap<usize, String>>,
        renamed_entries: Option<&HashMap<usize, String>>,
        compression_level: Option<i32>,
        force_compression: bool,
        oodle_dll: Option<&Path>,
    ) -> Result<Vec<u8>> {
        let default_zstd_label = if self.is_lowercase_magic {
            "ZStandard"
        } else {
            "Zstd"
        };
        let oodle_lib = OodleLibrary::load(oodle_dll).ok();

        let mut raw_items: Vec<(String, String, u64, Vec<u8>)> = Vec::new();
        let indices: Vec<usize> = if let Some(order) = reorder_map {
            order.to_vec()
        } else {
            (0..self.descriptors.len()).collect()
        };

        for idx in indices {
            if removed_indices.contains(&idx) {
                continue;
            }
            let Some(desc) = self.descriptors.get(idx) else {
                continue;
            };
            let entry_name = renamed_entries
                .and_then(|m| m.get(&idx))
                .cloned()
                .unwrap_or_else(|| desc.name.clone());
            let entry_comp_override = compression_overrides
                .and_then(|m| m.get(&idx).map(|s| s.as_str()))
                .or(preferred_compression);

            if let Some(new_raw) = replacements.get(&idx) {
                let target_comp = entry_comp_override.unwrap_or(desc.compression.as_str());
                let (actual_comp, payload) = Self::compress_payload(
                    new_raw,
                    target_comp,
                    compression_level,
                    force_compression,
                    default_zstd_label,
                    oodle_lib.as_ref(),
                )?;
                raw_items.push((entry_name, actual_comp, new_raw.len() as u64, payload));
            } else if let Some(override_codec) = entry_comp_override {
                let level_changed = compression_level.is_some();
                if !override_codec.eq_ignore_ascii_case(&desc.compression) || level_changed {
                    let raw_bytes =
                        self.extract_entry_with_lib(original_archive_bytes, idx, oodle_lib.as_ref())?;
                    let (actual_comp, payload) = Self::compress_payload(
                        &raw_bytes,
                        override_codec,
                        compression_level,
                        force_compression,
                        default_zstd_label,
                        oodle_lib.as_ref(),
                    )?;
                    raw_items.push((entry_name, actual_comp, raw_bytes.len() as u64, payload));
                } else {
                    let start = desc.offset as usize;
                    let end = (start + desc.compressed_size as usize).min(original_archive_bytes.len());
                    let slice = if start < end {
                        original_archive_bytes[start..end].to_vec()
                    } else {
                        Vec::new()
                    };
                    raw_items.push((entry_name, desc.compression.clone(), desc.size, slice));
                }
            } else {
                let start = desc.offset as usize;
                let end = (start + desc.compressed_size as usize).min(original_archive_bytes.len());
                let slice = if start < end {
                    original_archive_bytes[start..end].to_vec()
                } else {
                    Vec::new()
                };
                raw_items.push((entry_name, desc.compression.clone(), desc.size, slice));
            }
        }

        for (name, raw) in added_files {
            let target_comp = preferred_compression.unwrap_or(default_zstd_label);
            let (actual_comp, payload) = Self::compress_payload(
                raw,
                target_comp,
                compression_level,
                force_compression,
                default_zstd_label,
                oodle_lib.as_ref(),
            )?;
            raw_items.push((name.clone(), actual_comp, raw.len() as u64, payload));
        }

        Ok(self.pack_raw_slices(&raw_items))
    }

    /// Lay out pre-compressed `(name, compression_label, uncompressed_size, compressed_payload)` slices
    /// into a `.pkz` archive preserving this archive's header format and alignment.
    pub fn pack_raw_slices(&self, items: &[(String, String, u64, Vec<u8>)]) -> Vec<u8> {
        let num_files = items.len();
        let offset_file_descriptors = 0x20usize;
        let entry_byte_size = if self.is_32byte_descriptors { 32 } else { 40 };
        let descriptors_byte_len = num_files * entry_byte_size;

        let mut string_table: Vec<u8> = Vec::new();
        let mut string_offsets: HashMap<String, u64> = HashMap::new();

        fn intern_str(
            s: &str,
            string_table: &mut Vec<u8>,
            string_offsets: &mut HashMap<String, u64>,
        ) -> u64 {
            if let Some(&off) = string_offsets.get(s) {
                return off;
            }
            let off = string_table.len() as u64;
            string_table.extend_from_slice(s.as_bytes());
            string_table.push(0);
            string_offsets.insert(s.to_string(), off);
            off
        }

        let mut name_offsets = Vec::with_capacity(num_files);
        let mut comp_offsets = Vec::with_capacity(num_files);
        for (_, comp, _, _) in items {
            comp_offsets.push(intern_str(comp, &mut string_table, &mut string_offsets));
        }
        for (name, _, _, _) in items {
            name_offsets.push(intern_str(name, &mut string_table, &mut string_offsets));
        }

        let align_boundary = if self.is_lowercase_magic { 8 } else { 64 };
        let aligned_str_len = align(string_table.len(), align_boundary);
        string_table.resize(aligned_str_len, 0);

        let data_alignment = if self.is_lowercase_magic { 0x40 } else { 64 };
        let mut data_offset = align(
            offset_file_descriptors + descriptors_byte_len + string_table.len(),
            data_alignment,
        );

        let mut raw_descs = Vec::with_capacity(num_files);
        for (i, (_, _, uncomp_sz, payload)) in items.iter().enumerate() {
            let comp_len = payload.len();
            raw_descs.push(RawFileDescriptor {
                name_offset: name_offsets[i],
                compression_offset: comp_offsets[i],
                size: *uncomp_sz,
                offset: data_offset as u64,
                compressed_size: comp_len as u64,
            });
            data_offset = align(data_offset + comp_len, data_alignment);
        }

        let mut w = BinWriter::new(self.endian);
        if self.is_lowercase_magic {
            w.write_bytes(b"pkzl");
        } else if self.endian.is_big() {
            w.write_bytes(b"PKZB");
        } else {
            w.write_bytes(b"PKZL");
        }
        w.write_u32(self.unknown);
        w.write_u64(data_offset as u64);
        w.write_u32(num_files as u32);
        w.write_u32(if self.is_32byte_descriptors {
            0x20
        } else {
            offset_file_descriptors as u32
        });
        w.write_u64(string_table.len() as u64);

        for desc in &raw_descs {
            if self.is_32byte_descriptors {
                w.write_u32(desc.name_offset as u32);
                w.write_u32(desc.compression_offset as u32);
            } else {
                w.write_u64(desc.name_offset);
                w.write_u64(desc.compression_offset);
            }
            w.write_u64(desc.size);
            w.write_u64(desc.offset);
            w.write_u64(desc.compressed_size);
        }

        w.write_bytes(&string_table);
        w.align_to(data_alignment);

        for (desc, (_, _, _, payload)) in raw_descs.iter().zip(items.iter()) {
            w.seek(desc.offset as usize);
            w.write_bytes(payload);
            w.align_to(data_alignment);
        }

        w.into_inner()
    }

    pub fn compress_payload(
        raw: &[u8],
        requested_codec: &str,
        level: Option<i32>,
        force_compression: bool,
        default_zstd_label: &str,
        oodle_lib: Option<&OodleLibrary>,
    ) -> Result<(String, Vec<u8>)> {
        let low = requested_codec.trim().to_ascii_lowercase();
        if raw.is_empty() || low.is_empty() || low == "none" || low == "uncompressed" {
            return Ok(("None".to_string(), raw.to_vec()));
        }
        if low == "oodlekraken" || low == "oodle" || low == "kraken" || low == "oodle kraken" {
            let oodle_lvl = clamp_compression_level(
                "OodleKraken",
                level.unwrap_or(DEFAULT_OODLE_LEVEL),
            );
            if let Some(oodle) = oodle_lib {
                if let Ok(comp) = oodle.compress_with_level(raw, oodle_lvl) {
                    if is_compression_worthwhile(comp.len(), raw.len(), force_compression) {
                        return Ok(("OodleKraken".to_string(), comp));
                    } else {
                        return Ok(("None".to_string(), raw.to_vec()));
                    }
                }
            }
            let zstd_lvl = clamp_compression_level(
                "ZStandard",
                level.unwrap_or(DEFAULT_ZSTD_LEVEL),
            );
            let zstd_bytes = zstd::stream::encode_all(raw, zstd_lvl)
                .map_err(|e| PgError::InvalidData(format!("Zstd encode error: {e}")))?;
            if is_compression_worthwhile(zstd_bytes.len(), raw.len(), force_compression) {
                return Ok((default_zstd_label.to_string(), zstd_bytes));
            } else {
                return Ok(("None".to_string(), raw.to_vec()));
            }
        }
        let label = if requested_codec.eq_ignore_ascii_case("ZStandard") {
            "ZStandard"
        } else if requested_codec.eq_ignore_ascii_case("Zstd") {
            "Zstd"
        } else {
            default_zstd_label
        };
        let zstd_lvl = clamp_compression_level(
            "ZStandard",
            level.unwrap_or(DEFAULT_ZSTD_LEVEL),
        );
        let zstd_bytes = zstd::stream::encode_all(raw, zstd_lvl)
            .map_err(|e| PgError::InvalidData(format!("Zstd encode error: {e}")))?;
        if is_compression_worthwhile(zstd_bytes.len(), raw.len(), force_compression) {
            Ok((label.to_string(), zstd_bytes))
        } else {
            Ok(("None".to_string(), raw.to_vec()))
        }
    }

    /// Recursively collect all files under `source_dir`, returning `(virtual_path, raw_bytes)`
    /// sorted case-insensitively by virtual path (`pl/pl0000.dat`), matching `PkzFolderBuilder.cs`.
    pub fn collect_dir_files_recursive(source_dir: &Path) -> Result<Vec<(String, Vec<u8>)>> {
        if !source_dir.is_dir() {
            return Err(PgError::InvalidData(format!(
                "PKZ source folder does not exist: {}",
                source_dir.display()
            )));
        }
        let mut collected: Vec<(String, PathBuf)> = Vec::new();
        fn walk(base: &Path, cur: &Path, out: &mut Vec<(String, PathBuf)>) -> Result<()> {
            for entry in fs::read_dir(cur)? {
                let entry = entry?;
                let ft = entry.file_type()?;
                if ft.is_symlink() {
                    continue;
                }
                let path = entry.path();
                if ft.is_dir() {
                    walk(base, &path, out)?;
                } else if ft.is_file() {
                    let rel = path.strip_prefix(base).map_err(|e| {
                        PgError::InvalidData(format!("Failed to compute relative path: {e}"))
                    })?;
                    let rel_str = rel
                        .components()
                        .filter_map(|c| match c {
                            std::path::Component::Normal(s) => s.to_str(),
                            _ => None,
                        })
                        .collect::<Vec<_>>()
                        .join("/");
                    let norm = normalize_pkz_virtual_path(&rel_str)?;
                    out.push((norm, path));
                }
            }
            Ok(())
        }
        walk(source_dir, source_dir, &mut collected)?;
        collected.sort_by(|a, b| {
            a.0.to_ascii_lowercase()
                .cmp(&b.0.to_ascii_lowercase())
                .then_with(|| a.0.cmp(&b.0))
        });

        let mut files = Vec::with_capacity(collected.len());
        for (vpath, disk_path) in collected {
            let bytes = fs::read(&disk_path)?;
            files.push((vpath, bytes));
        }
        Ok(files)
    }

    /// Build a `.pkz` archive recursively from `source_dir` using the requested codec, level,
    /// and 90% worthwhile compression rule (`PkzFolderBuilder.Build` parity).
    pub fn build_from_dir(
        source_dir: &Path,
        codec: &str,
        level: i32,
        force_compression: bool,
        oodle_dll: Option<&Path>,
    ) -> Result<Vec<u8>> {
        let files = Self::collect_dir_files_recursive(source_dir)?;
        if files.is_empty() {
            return Err(PgError::InvalidData(
                "The folder has no files to pack into a PKZ archive".into(),
            ));
        }
        Self::build_with_options(&files, codec, Some(level), force_compression, oodle_dll)
    }

    /// Build a `.pkz` archive from `(filename, raw_bytes)` pairs with explicit codec, level, and threshold options.
    pub fn build_with_options(
        files: &[(String, Vec<u8>)],
        codec: &str,
        level: Option<i32>,
        force_compression: bool,
        oodle_dll: Option<&Path>,
    ) -> Result<Vec<u8>> {
        let req_low = codec.trim().to_ascii_lowercase();
        if (req_low == "oodlekraken" || req_low == "oodle" || req_low == "kraken" || req_low == "oodle kraken")
            && !OodleLibrary::load(oodle_dll)
                .map(|l| l.can_compress())
                .unwrap_or(false)
        {
            return Err(PgError::Unsupported(
                "OODLE_DLL_MISSING: oo2core_*_win64.dll with OodleLZ_Compress is required to compress with OodleKraken.".into(),
            ));
        }
        let dummy = Self {
            endian: Endian::Little,
            is_lowercase_magic: true,
            is_32byte_descriptors: true,
            unknown: 0x0002_0000,
            descriptors: Vec::new(),
        };
        dummy.repack_with_full_options(
            &[],
            &HashMap::new(),
            files,
            &[],
            None,
            Some(codec),
            None,
            None,
            level,
            force_compression,
            oodle_dll,
        )
    }

    /// Build a `.pkz` archive from `(filename, raw_bytes)` pairs using ZStandard/Zstd or uncompressed storage.
    pub fn build(files: &[(String, Vec<u8>)], prefer_zstd: bool) -> Result<Vec<u8>> {
        let codec = if prefer_zstd { "ZStandard" } else { "None" };
        Self::build_with_options(files, codec, None, false, None)
    }
}
