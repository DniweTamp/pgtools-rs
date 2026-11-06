//! `EFF` and `IDD` multi-directory effect container parser, extractor, and repacker
//! (1:1 port of `lib/bayonetta/eff.rb`, `eff_idd_extractor.rb`, and `eff_idd_creator.rb`).

use crate::io::{align, BinReader, BinWriter, Endian};
use crate::{PgError, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

pub const KNOWN_STRUCTURED_DIRS: &[&str] = &["TEX", "MOD", "EST", "IDT"];
pub const KNOWN_RAW_DIRS: &[&str] = &["SAD", "SST", "TUV"];

pub fn eff_dir_alignment(dir_name: &str) -> usize {
    match dir_name.trim_end_matches('\0') {
        "TEX" | "MOD" => 0x1000,
        "EST" | "IDT" => 0x10,
        _ => 0x10,
    }
}

pub fn eff_dir_extension(dir_name: &str) -> String {
    match dir_name.trim_end_matches('\0') {
        "TEX" => "wtb".to_string(),
        "MOD" => "dat".to_string(),
        other => other.to_ascii_lowercase(),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EffSubFile {
    pub id: u32,
    pub file_name: String,
    #[serde(skip)]
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EffDirectory {
    pub dir_id: u32,
    pub name: String,
    pub files: Vec<EffSubFile>,
}

impl EffDirectory {
    pub fn is_structured(&self) -> bool {
        let trimmed = self.name.trim_end_matches('\0');
        KNOWN_STRUCTURED_DIRS.contains(&trimmed)
    }

    pub fn from_bytes(dir_id: u32, data: &[u8], endian: Endian) -> Result<Self> {
        if data.len() < 4 {
            return Err(PgError::InvalidData("EFF directory shorter than 4 bytes".into()));
        }
        let mut r = BinReader::new(data, endian);
        let name = r.read_fixed_string(4)?;
        let ext = eff_dir_extension(&name);

        if KNOWN_RAW_DIRS.contains(&name.as_str()) {
            return Ok(Self {
                dir_id,
                name,
                files: vec![EffSubFile {
                    id: 0,
                    file_name: format!("{:010}.{}", 0, ext),
                    data: data.to_vec(),
                }],
            });
        }

        if !KNOWN_STRUCTURED_DIRS.contains(&name.as_str()) {
            return Err(PgError::InvalidData(format!(
                "Unknown EFF directory tag: {name:?}"
            )));
        }

        let file_number = r.read_u32()? as usize;
        let mut raw_infos: Vec<(u32, usize)> = Vec::with_capacity(file_number);
        for _ in 0..file_number {
            let id = r.read_u32()?;
            let offset = r.read_u32()? as usize;
            raw_infos.push((id, offset));
        }

        let mut sorted = raw_infos.clone();
        sorted.sort_by_key(|&(_, off)| off);

        let mut offset_to_size = std::collections::HashMap::new();
        for (i, &(_, offset)) in sorted.iter().enumerate() {
            let size = if let Some(&(_, next_off)) = sorted.get(i + 1) {
                next_off.saturating_sub(offset)
            } else {
                data.len().saturating_sub(offset)
            };
            offset_to_size.insert(offset, size);
        }

        let mut files = Vec::with_capacity(file_number);
        for (id, offset) in raw_infos {
            let size = *offset_to_size.get(&offset).unwrap_or(&0);
            let payload = if size > 0 && offset < data.len() {
                r.slice_at(offset, size.min(data.len() - offset))?.to_vec()
            } else {
                Vec::new()
            };
            files.push(EffSubFile {
                id,
                file_name: format!("{:010}.{}", id, ext),
                data: payload,
            });
        }

        Ok(Self {
            dir_id,
            name,
            files,
        })
    }

    pub fn to_bytes(&self, endian: Endian) -> Result<Vec<u8>> {
        if KNOWN_RAW_DIRS.contains(&self.name.as_str()) {
            if self.files.len() != 1 {
                return Err(PgError::InvalidData(format!(
                    "Raw EFF directory {} must contain exactly 1 file",
                    self.name
                )));
            }
            return Ok(self.files[0].data.clone());
        }

        let alignment = eff_dir_alignment(&self.name);
        let mut current_offset = align(8 + self.files.len() * 8, alignment);
        let mut offsets = Vec::with_capacity(self.files.len());
        for f in &self.files {
            offsets.push(current_offset);
            current_offset = align(current_offset + f.data.len(), alignment);
        }

        let mut w = BinWriter::with_capacity(current_offset, endian);
        w.write_fixed_string(&self.name, 4);
        w.write_u32(self.files.len() as u32);
        for (i, f) in self.files.iter().enumerate() {
            w.write_u32(f.id);
            w.write_u32(offsets[i] as u32);
        }
        for (i, f) in self.files.iter().enumerate() {
            w.seek(offsets[i]);
            w.write_bytes(&f.data);
        }
        w.seek(current_offset);
        Ok(w.into_inner())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EffFile {
    pub id: String,
    pub endian: Endian,
    pub directories: Vec<EffDirectory>,
}

impl EffFile {
    pub fn detect_endian(data: &[u8]) -> Result<Endian> {
        if data.len() < 8 {
            return Err(PgError::InvalidData("EFF/IDD file shorter than 8 bytes".into()));
        }
        let magic = &data[0..4];
        if magic != b"EF2\0" && magic != b"IDD\0" {
            return Err(PgError::InvalidMagic {
                expected: "EF2\\0 or IDD\\0",
                actual: magic.to_vec(),
            });
        }
        let check = |endian: Endian| -> bool {
            let mut r = BinReader::new(data, endian);
            let _ = r.seek(4);
            let mut dir_num = r.read_i32().unwrap_or(-1);
            if magic == b"IDD\0" {
                dir_num += 1;
            }
            if dir_num <= 0 || (dir_num as usize) * 8 + 8 >= data.len() {
                return false;
            }
            for _ in 0..dir_num {
                let idx = r.read_i32().unwrap_or(-1);
                let off = r.read_i32().unwrap_or(-1);
                if idx < 0 || off <= 0 || (off as usize) >= data.len() {
                    return false;
                }
            }
            true
        };
        let big = check(Endian::Big);
        let little = check(Endian::Little);
        match (big, little) {
            (true, false) => Ok(Endian::Big),
            (false, true) => Ok(Endian::Little),
            _ => Err(PgError::InvalidData("Ambiguous EFF/IDD endianness".into())),
        }
    }

    pub fn from_bytes(data: &[u8]) -> Result<Self> {
        let endian = Self::detect_endian(data)?;
        let mut r = BinReader::new(data, endian);
        let id = r.read_fixed_string(4)?;
        let mut directory_number = r.read_u32()? as usize;
        if id == "IDD" {
            directory_number += 1;
        }

        let mut dir_infos = Vec::with_capacity(directory_number);
        for _ in 0..directory_number {
            let dir_id = r.read_u32()?;
            let offset = r.read_u32()? as usize;
            dir_infos.push((dir_id, offset));
        }

        let mut sorted = dir_infos.clone();
        sorted.sort_by_key(|&(_, off)| off);
        let mut offset_to_size = std::collections::HashMap::new();
        for (i, &(_, offset)) in sorted.iter().enumerate() {
            let size = if let Some(&(_, next_off)) = sorted.get(i + 1) {
                next_off.saturating_sub(offset)
            } else {
                data.len().saturating_sub(offset)
            };
            offset_to_size.insert(offset, size);
        }

        let mut directories = Vec::with_capacity(directory_number);
        for (dir_id, offset) in dir_infos {
            let size = *offset_to_size.get(&offset).unwrap_or(&0);
            let slice = r.slice_at(offset, size)?;
            directories.push(EffDirectory::from_bytes(dir_id, slice, endian)?);
        }

        Ok(Self {
            id,
            endian,
            directories,
        })
    }

    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        let mut serialized_dirs = Vec::with_capacity(self.directories.len());
        let mut current_offset = 0x1000usize;
        let mut offsets = Vec::with_capacity(self.directories.len());

        for d in &self.directories {
            offsets.push(current_offset);
            let bytes = d.to_bytes(self.endian)?;
            current_offset = align(current_offset + bytes.len(), 0x1000);
            serialized_dirs.push(bytes);
        }

        let mut w = BinWriter::with_capacity(current_offset, self.endian);
        w.write_fixed_string(&self.id, 4);
        let header_count = if self.id == "IDD" {
            self.directories.len().saturating_sub(1) as u32
        } else {
            self.directories.len() as u32
        };
        w.write_u32(header_count);

        for (i, d) in self.directories.iter().enumerate() {
            w.write_u32(d.dir_id);
            w.write_u32(offsets[i] as u32);
        }

        for (i, bytes) in serialized_dirs.iter().enumerate() {
            w.seek(offsets[i]);
            w.write_bytes(bytes);
        }

        w.seek(current_offset);
        Ok(w.into_inner())
    }

    pub fn extract_to_dir(&self, output_dir: &Path, original_ext: &str) -> Result<()> {
        fs::create_dir_all(output_dir)?;
        let mut layout: Vec<(u32, String)> = Vec::with_capacity(self.directories.len());
        for d in &self.directories {
            let sub_dir = output_dir.join(&d.name);
            fs::create_dir_all(&sub_dir)?;
            for f in &d.files {
                fs::write(sub_dir.join(&f.file_name), &f.data)?;
            }
            layout.push((d.dir_id, d.name.clone()));
        }

        let meta_dir = output_dir.join(".metadata");
        fs::create_dir_all(&meta_dir)?;
        let id_with_null = format!("{}\0", self.id);
        fs::write(
            meta_dir.join("id.yaml"),
            serde_yaml::to_string(&id_with_null)
                .map_err(|e| PgError::Serialization(e.to_string()))?,
        )?;
        fs::write(
            meta_dir.join("layout.yaml"),
            serde_yaml::to_string(&layout).map_err(|e| PgError::Serialization(e.to_string()))?,
        )?;
        fs::write(
            meta_dir.join("extension.yaml"),
            serde_yaml::to_string(&original_ext)
                .map_err(|e| PgError::Serialization(e.to_string()))?,
        )?;
        fs::write(
            meta_dir.join("big.yaml"),
            serde_yaml::to_string(&self.endian.is_big())
                .map_err(|e| PgError::Serialization(e.to_string()))?,
        )?;
        Ok(())
    }

    pub fn create_from_dir(input_dir: &Path) -> Result<(Self, String)> {
        let meta_dir = input_dir.join(".metadata");
        let id_raw: String = serde_yaml::from_str(&fs::read_to_string(meta_dir.join("id.yaml"))?)
            .map_err(|e| PgError::Serialization(e.to_string()))?;
        let id = id_raw.trim_end_matches('\0').to_string();
        let big: bool = serde_yaml::from_str(&fs::read_to_string(meta_dir.join("big.yaml"))?)
            .map_err(|e| PgError::Serialization(e.to_string()))?;
        let extension: String =
            serde_yaml::from_str(&fs::read_to_string(meta_dir.join("extension.yaml"))?)
                .map_err(|e| PgError::Serialization(e.to_string()))?;
        let layout: Vec<(u32, String)> =
            serde_yaml::from_str(&fs::read_to_string(meta_dir.join("layout.yaml"))?)
                .map_err(|e| PgError::Serialization(e.to_string()))?;

        let mut directories = Vec::with_capacity(layout.len());
        for (dir_id, dname) in layout {
            let sub_dir = input_dir.join(&dname);
            let mut fnames: Vec<String> = fs::read_dir(&sub_dir)?
                .filter_map(|e| e.ok())
                .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect();
            fnames.sort();

            let mut files = Vec::with_capacity(fnames.len());
            for fname in fnames {
                let stem = Path::new(&fname)
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("0");
                let file_id = stem.parse::<u32>().unwrap_or(0);
                let data = fs::read(sub_dir.join(&fname))?;
                files.push(EffSubFile {
                    id: file_id,
                    file_name: fname,
                    data,
                });
            }
            directories.push(EffDirectory {
                dir_id,
                name: dname,
                files,
            });
        }

        let endian = if big { Endian::Big } else { Endian::Little };
        Ok((
            Self {
                id,
                endian,
                directories,
            },
            extension,
        ))
    }
}
