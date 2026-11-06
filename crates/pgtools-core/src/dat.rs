//! `DAT` / `DTT` / `EVN` / `EFF` archive container parser, hash-map builder,
//! recursive listing (`dat_ls`), extractor, and creator (`lib/bayonetta/dat.rb`).

use crate::io::{align, checked_count, sanitize_entry_path, BinReader, BinWriter, Endian};
use crate::{PgError, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

pub fn dat_alignment_for_ext(ext: &str) -> usize {
    match ext.trim_end_matches('\0').to_ascii_lowercase().as_str() {
        "dat" => 0x2000,
        "wmb" | "wtb" | "wtp" | "exp" | "eff" | "sdx" => 0x1000,
        "wta" | "sop" | "bxm" => 0x40,
        _ => 0x10,
    }
}

/// PlatinumGames CRC32-based case-insensitive filename hash used in `DAT` hash tables.
pub fn compute_dat_crc32(name: &str, pre_hash_shift: u32) -> u32 {
    let lower = name.to_ascii_lowercase();
    let crc = crc32fast::hash(lower.as_bytes());
    let _ = pre_hash_shift;
    crc & 0x7FFF_FFFF
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DatHashMap {
    pub pre_hash_shift: u32,
    /// One hash per file in file index order (matches `HashMap#get` in `dat.rb`).
    pub hashes: Vec<u32>,
}

impl DatHashMap {
    pub fn from_file_names(names: &[String]) -> Self {
        let count = names.len().max(1);
        let log2 = (count as f64).log2().ceil() as u32;
        let pre_hash_shift = 31u32.saturating_sub(log2.clamp(1, 16));
        let hashes = names
            .iter()
            .map(|n| compute_dat_crc32(n, pre_hash_shift))
            .collect();
        Self {
            pre_hash_shift,
            hashes,
        }
    }

    pub fn read(r: &mut BinReader, base_offset: usize, num_files: usize) -> Result<Self> {
        r.seek(base_offset)?;
        let pre_hash_shift = r.read_u32()?;
        let offset_bucket_ranks = r.read_u32()? as usize;
        let offset_hashes = r.read_u32()? as usize;
        let offset_file_indices = r.read_u32()? as usize;

        let num_buckets = if pre_hash_shift <= 31 {
            1usize << (31 - pre_hash_shift)
        } else {
            0
        };

        let ranks_pos = base_offset.checked_add(offset_bucket_ranks).ok_or_else(|| {
            PgError::InvalidData("DAT bucket_ranks offset overflow".into())
        })?;
        checked_count(
            num_buckets,
            2,
            r.len().saturating_sub(ranks_pos),
            "DAT hash_map bucket_ranks",
        )?;
        r.seek(ranks_pos)?;
        for _ in 0..num_buckets {
            let _ = r.read_i16()?;
        }

        let hashes_pos = base_offset.checked_add(offset_hashes).ok_or_else(|| {
            PgError::InvalidData("DAT hashes offset overflow".into())
        })?;
        checked_count(
            num_files,
            4,
            r.len().saturating_sub(hashes_pos),
            "DAT hash_map hashes",
        )?;
        r.seek(hashes_pos)?;
        let mut raw_hashes = Vec::with_capacity(num_files);
        for _ in 0..num_files {
            raw_hashes.push(r.read_u32()?);
        }

        let indices_pos = base_offset.checked_add(offset_file_indices).ok_or_else(|| {
            PgError::InvalidData("DAT file_indices offset overflow".into())
        })?;
        checked_count(
            num_files,
            2,
            r.len().saturating_sub(indices_pos),
            "DAT hash_map file_indices",
        )?;
        r.seek(indices_pos)?;
        let mut file_indices = Vec::with_capacity(num_files);
        for _ in 0..num_files {
            file_indices.push(r.read_u16()?);
        }

        let mut pairs: Vec<(u16, u32)> = file_indices.into_iter().zip(raw_hashes).collect();
        pairs.sort_by_key(|&(idx, _)| idx);
        let hashes = pairs.into_iter().map(|(_, h)| h).collect();

        Ok(Self {
            pre_hash_shift,
            hashes,
        })
    }

    pub fn serialize(&self, endian: Endian) -> Vec<u8> {
        let mut w = BinWriter::with_capacity(self.byte_size(), endian);
        self.write_to(&mut w);
        w.into_inner()
    }

    pub fn write_to(&self, w: &mut BinWriter) {
        let bit_shift = self.pre_hash_shift.min(31);
        let num_files = self.hashes.len();
        let num_buckets = 1usize << (31 - bit_shift);

        let mut buckets: BTreeMap<usize, Vec<(u32, u16)>> = BTreeMap::new();
        for (i, &h) in self.hashes.iter().enumerate() {
            let bucket_index = (h >> bit_shift) as usize;
            buckets.entry(bucket_index).or_default().push((h, i as u16));
        }

        let mut bucket_ranks: Vec<i16> = Vec::with_capacity(num_buckets);
        let mut sorted_hashes: Vec<u32> = Vec::with_capacity(num_files);
        let mut file_indices: Vec<u16> = Vec::with_capacity(num_files);
        let mut bucket_rank: i16 = 0;

        for i in 0..num_buckets {
            if let Some(items) = buckets.get(&i) {
                bucket_ranks.push(bucket_rank);
                bucket_rank = bucket_rank.saturating_add(items.len() as i16);
                for &(h, ind) in items {
                    sorted_hashes.push(h);
                    file_indices.push(ind);
                }
            } else {
                bucket_ranks.push(-1);
            }
        }

        let offset_bucket_ranks = 0x10u32;
        let offset_hashes = offset_bucket_ranks + (num_buckets as u32) * 2;
        let offset_file_indices = offset_hashes + (num_files as u32) * 4;

        w.write_u32(bit_shift);
        w.write_u32(offset_bucket_ranks);
        w.write_u32(offset_hashes);
        w.write_u32(offset_file_indices);

        for r in bucket_ranks {
            w.write_i16(r);
        }
        for h in sorted_hashes {
            w.write_u32(h);
        }
        for idx in file_indices {
            w.write_u16(idx);
        }
    }

    pub fn byte_size(&self) -> usize {
        let bit_shift = self.pre_hash_shift.min(31);
        let num_buckets = 1usize << (31 - bit_shift);
        let num_files = self.hashes.len();
        0x10 + num_buckets * 2 + num_files * 4 + num_files * 2
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatEntry {
    pub name: String,
    pub extension: String,
    #[serde(skip)]
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatFile {
    pub endian: Endian,
    pub entries: Vec<DatEntry>,
    pub hash_map: Option<DatHashMap>,
}

impl DatFile {
    pub fn new(endian: Endian) -> Self {
        Self {
            endian,
            entries: Vec::new(),
            hash_map: None,
        }
    }

    pub fn detect_endian(data: &[u8]) -> Result<Endian> {
        if data.len() < 0x20 {
            return Err(PgError::InvalidData("DAT file shorter than 32-byte header".into()));
        }
        if &data[0..4] != b"DAT\0" {
            return Err(PgError::InvalidMagic {
                expected: "DAT\\0",
                actual: data[0..4].to_vec(),
            });
        }
        let check = |endian: Endian| -> bool {
            let mut r = BinReader::new(data, endian);
            let _ = r.seek(8);
            let off_offsets = r.read_u32().unwrap_or(u32::MAX) as usize;
            let _off_exts = r.read_u32().unwrap_or(u32::MAX) as usize;
            let off_names = r.read_u32().unwrap_or(u32::MAX) as usize;
            let off_sizes = r.read_u32().unwrap_or(u32::MAX) as usize;
            let off_hash = r.read_u32().unwrap_or(u32::MAX) as usize;
            off_offsets < data.len()
                && off_names < data.len()
                && off_sizes < data.len()
                && off_hash < data.len()
        };
        let big = check(Endian::Big);
        let little = check(Endian::Little);
        match (big, little) {
            (true, false) => Ok(Endian::Big),
            (false, true) => Ok(Endian::Little),
            (false, false) if data.len() == 0x20 => Ok(Endian::Little),
            _ => Err(PgError::InvalidData(
                "Ambiguous or corrupted DAT header offsets".into(),
            )),
        }
    }

    pub fn from_bytes(data: &[u8]) -> Result<Self> {
        let endian = Self::detect_endian(data)?;
        let mut r = BinReader::new(data, endian);
        let _id = r.read_bytes(4)?;
        let num_files = r.read_u32()? as usize;
        let offset_file_offsets = r.read_u32()? as usize;
        let offset_file_extensions = r.read_u32()? as usize;
        let offset_file_names = r.read_u32()? as usize;
        let offset_file_sizes = r.read_u32()? as usize;
        let offset_hash_map = r.read_u32()? as usize;

        if num_files == 0 {
            return Ok(Self::new(endian));
        }

        checked_count(
            num_files,
            4,
            data.len().saturating_sub(offset_file_offsets),
            "DAT file_offsets",
        )?;
        r.seek(offset_file_offsets)?;
        let mut file_offsets = Vec::with_capacity(num_files);
        for _ in 0..num_files {
            file_offsets.push(r.read_u32()? as usize);
        }

        checked_count(
            num_files,
            4,
            data.len().saturating_sub(offset_file_extensions),
            "DAT file_extensions",
        )?;
        r.seek(offset_file_extensions)?;
        let mut file_extensions = Vec::with_capacity(num_files);
        for _ in 0..num_files {
            file_extensions.push(r.read_fixed_string(4)?);
        }

        r.seek(offset_file_names)?;
        let file_name_length = r.read_u32()? as usize;
        if file_name_length == 0 || file_name_length > 4096 {
            return Err(PgError::InvalidData(format!(
                "Invalid DAT file_name_length: {file_name_length}"
            )));
        }
        checked_count(
            num_files,
            file_name_length,
            data.len().saturating_sub(offset_file_names.saturating_add(4)),
            "DAT file_names",
        )?;
        let mut file_names = Vec::with_capacity(num_files);
        for i in 0..num_files {
            r.seek(offset_file_names + 4 + i * file_name_length)?;
            file_names.push(r.read_fixed_string(file_name_length)?);
        }

        checked_count(
            num_files,
            4,
            data.len().saturating_sub(offset_file_sizes),
            "DAT file_sizes",
        )?;
        r.seek(offset_file_sizes)?;
        let mut file_sizes = Vec::with_capacity(num_files);
        for _ in 0..num_files {
            file_sizes.push(r.read_u32()? as usize);
        }

        let hash_map = if offset_hash_map != 0 && offset_hash_map < data.len() {
            DatHashMap::read(&mut r, offset_hash_map, num_files).ok()
        } else {
            None
        };

        let mut entries = Vec::with_capacity(num_files);
        for i in 0..num_files {
            let off = file_offsets[i];
            let sz = file_sizes[i];
            let payload = if sz > 0 && off > 0 {
                r.slice_at(off, sz)?.to_vec()
            } else {
                Vec::new()
            };
            entries.push(DatEntry {
                name: file_names[i].clone(),
                extension: file_extensions[i].clone(),
                data: payload,
            });
        }

        Ok(Self {
            endian,
            entries,
            hash_map,
        })
    }

    pub fn push(&mut self, name: impl Into<String>, data: Vec<u8>) -> Result<()> {
        let name = name.into();
        let ext = Path::new(&name)
            .extension()
            .and_then(|e| e.to_str())
            .ok_or_else(|| PgError::InvalidData(format!("Missing extension on DAT entry: {name}")))?
            .to_string();
        self.entries.push(DatEntry {
            name,
            extension: ext,
            data,
        });
        if self.hash_map.is_some() {
            self.hash_map = Some(DatHashMap::from_file_names(&self.layout()));
        }
        Ok(())
    }

    pub fn rebuild_hash_map(&mut self) {
        if self.hash_map.is_some() {
            self.hash_map = Some(DatHashMap::from_file_names(&self.layout()));
        }
    }

    pub fn layout(&self) -> Vec<String> {
        self.entries.iter().map(|e| e.name.clone()).collect()
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let num_files = self.entries.len();
        let offset_file_offsets = 0x20usize;
        let offset_file_extensions = offset_file_offsets + 4 * num_files;
        let offset_file_names = offset_file_extensions + 4 * num_files;
        let max_name_len = self
            .entries
            .iter()
            .map(|e| e.name.len() + 1)
            .max()
            .unwrap_or(1);
        let offset_file_sizes = align(offset_file_names + 4 + max_name_len * num_files, 4);

        let synced_hm = self
            .hash_map
            .as_ref()
            .map(|_| DatHashMap::from_file_names(&self.layout()));

        let (offset_hash_map, mut files_offset) = if let Some(ref hm) = synced_hm {
            let ohm = offset_file_sizes + 4 * num_files;
            (ohm, ohm + hm.byte_size())
        } else {
            (0usize, offset_file_sizes + 4 * num_files)
        };

        let mut file_offsets = Vec::with_capacity(num_files);
        for entry in &self.entries {
            if !entry.data.is_empty() {
                let al = dat_alignment_for_ext(&entry.extension);
                let tmp = align(files_offset, al);
                files_offset = align(tmp + entry.data.len(), al);
                file_offsets.push(tmp as u32);
            } else {
                file_offsets.push(0);
            }
        }

        let total_size = align(files_offset, 0x1000);
        let mut w = BinWriter::with_capacity(total_size, self.endian);

        w.write_bytes(b"DAT\0");
        w.write_u32(num_files as u32);
        w.write_u32(offset_file_offsets as u32);
        w.write_u32(offset_file_extensions as u32);
        w.write_u32(offset_file_names as u32);
        w.write_u32(offset_file_sizes as u32);
        w.write_u32(offset_hash_map as u32);

        w.seek(offset_file_offsets);
        for &off in &file_offsets {
            w.write_u32(off);
        }

        w.seek(offset_file_extensions);
        for entry in &self.entries {
            w.write_fixed_string(&entry.extension, 4);
        }

        w.seek(offset_file_names);
        w.write_u32(max_name_len as u32);
        for (i, entry) in self.entries.iter().enumerate() {
            w.seek(offset_file_names + 4 + i * max_name_len);
            w.write_fixed_string(&entry.name, max_name_len);
        }

        w.seek(offset_file_sizes);
        for entry in &self.entries {
            w.write_u32(entry.data.len() as u32);
        }

        if let Some(ref hm) = synced_hm {
            w.seek(offset_hash_map);
            hm.write_to(&mut w);
        }

        for (i, entry) in self.entries.iter().enumerate() {
            if !entry.data.is_empty() {
                w.seek(file_offsets[i] as usize);
                w.write_bytes(&entry.data);
            }
        }

        w.seek(total_size);
        w.into_inner()
    }

    /// Extract a `.dat` / `.dtt` / `.evn` / `.eff` archive to `output_dir`, writing `.metadata/*.yaml`
    /// 100% compatible with the original Ruby `dat_extractor` and `dat_creator`.
    pub fn extract_to_dir(&self, output_dir: &Path, original_ext: &str) -> Result<Vec<String>> {
        fs::create_dir_all(output_dir)?;
        let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
        for e in &self.entries {
            *counts.entry(&e.name).or_default() += 1;
        }
        let mut warnings = Vec::new();
        for (name, c) in counts {
            if c > 1 {
                warnings.push(format!("Duplicate entry in DAT: {name} ({c} occurrences)"));
            }
        }

        for entry in &self.entries {
            let dest = sanitize_entry_path(output_dir, &entry.name, "unnamed.bin")?;
            if let Some(parent) = dest.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(dest, &entry.data)?;
        }

        let meta_dir = output_dir.join(".metadata");
        fs::create_dir_all(&meta_dir)?;
        fs::write(
            meta_dir.join("layout.yaml"),
            serde_yaml::to_string(&self.layout()).map_err(|e| PgError::Serialization(e.to_string()))?,
        )?;
        fs::write(
            meta_dir.join("extension.yaml"),
            serde_yaml::to_string(&original_ext).map_err(|e| PgError::Serialization(e.to_string()))?,
        )?;
        fs::write(
            meta_dir.join("big.yaml"),
            serde_yaml::to_string(&self.endian.is_big())
                .map_err(|e| PgError::Serialization(e.to_string()))?,
        )?;
        if let Some(ref hm) = self.hash_map {
            fs::write(
                meta_dir.join("hash_map.yaml"),
                serde_yaml::to_string(hm).map_err(|e| PgError::Serialization(e.to_string()))?,
            )?;
        }

        Ok(warnings)
    }

    /// Pack a directory back into a `DatFile` (reading `.metadata/*.yaml` if present, or auto-building layout + hash_map).
    pub fn create_from_dir(input_dir: &Path) -> Result<(Self, String)> {
        let meta_dir = input_dir.join(".metadata");
        let big: bool = if meta_dir.join("big.yaml").exists() {
            let s = fs::read_to_string(meta_dir.join("big.yaml"))?;
            serde_yaml::from_str(&s).unwrap_or(false)
        } else {
            false
        };
        let extension: String = if meta_dir.join("extension.yaml").exists() {
            let s = fs::read_to_string(meta_dir.join("extension.yaml"))?;
            serde_yaml::from_str(&s).unwrap_or_else(|_| ".dat".to_string())
        } else {
            ".dat".to_string()
        };

        let layout: Vec<String> = if meta_dir.join("layout.yaml").exists() {
            let s = fs::read_to_string(meta_dir.join("layout.yaml"))?;
            serde_yaml::from_str(&s).map_err(|e| PgError::Serialization(e.to_string()))?
        } else {
            let mut found = Vec::new();
            for entry in fs::read_dir(input_dir)? {
                let entry = entry?;
                let fname = entry.file_name().to_string_lossy().into_owned();
                if fname.starts_with('.') || fname == "dat_output" {
                    continue;
                }
                if entry.file_type()?.is_file() {
                    found.push(fname);
                }
            }
            found.sort();
            found
        };

        let endian = if big { Endian::Big } else { Endian::Little };
        let mut dat = Self::new(endian);
        for fname in &layout {
            let fpath = sanitize_entry_path(input_dir, fname, "unnamed.bin")?;
            let bytes = fs::read(&fpath)?;
            dat.push(fname.clone(), bytes)?;
        }

        if meta_dir.join("hash_map.yaml").exists() {
            let s = fs::read_to_string(meta_dir.join("hash_map.yaml"))?;
            // Support both Ruby `:pre_hash_shift` keys and standard `pre_hash_shift` keys
            let normalized = s
                .replace(":pre_hash_shift:", "pre_hash_shift:")
                .replace(":hashes:", "hashes:");
            if let Ok(hm) = serde_yaml::from_str::<DatHashMap>(&normalized) {
                if hm.hashes.len() == layout.len() {
                    dat.hash_map = Some(hm);
                } else {
                    dat.hash_map = Some(DatHashMap::from_file_names(&layout));
                }
            } else {
                dat.hash_map = Some(DatHashMap::from_file_names(&layout));
            }
        } else {
            dat.hash_map = Some(DatHashMap::from_file_names(&layout));
        }

        Ok((dat, extension))
    }

    /// Recursively list files inside a `.dat`/`.dtt` byte buffer (including nested `.dat`/`.dtt` archives).
    pub fn list_recursive(prefix: &str, data: &[u8], out: &mut Vec<String>) -> Result<()> {
        let dat = Self::from_bytes(data)?;
        for entry in &dat.entries {
            let ext = entry.extension.to_ascii_lowercase();
            if (ext == "dat" || ext == "dtt") && !entry.data.is_empty() {
                let nested_prefix = format!("{}:{}", prefix, entry.name);
                let _ = Self::list_recursive(&nested_prefix, &entry.data, out);
            }
        }
        for entry in &dat.entries {
            out.push(format!("{}:{}", prefix, entry.name));
        }
        Ok(())
    }
}
