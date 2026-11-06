//! `SCR` (Bayonetta 1) and `SCR2` (Bayonetta 2) stage model/texture container
//! parser, extractor, and repacker (`lib/bayonetta/scr.rb`).

use crate::io::{align, BinReader, BinWriter, Endian};
use crate::{PgError, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScrModelMetadata {
    pub name: String,
    pub transform: [f32; 9],
    /// 42 `i16` values for Bayonetta 1 (`SCRFile`), 18 `i16` values for Bayonetta 2 (`SCR2File`).
    pub unknown_shorts: Vec<i16>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScrFile {
    pub is_bayo2: bool,
    pub endian: Endian,
    pub unknown_header: u32,
    pub models_metadata: Vec<ScrModelMetadata>,
    #[serde(skip)]
    pub models: Vec<Vec<u8>>,
    #[serde(skip)]
    pub textures: Option<Vec<u8>>,
}

impl ScrFile {
    pub fn is_bayo2_format(data: &[u8]) -> Result<bool> {
        if data.len() < 8 {
            return Err(PgError::InvalidData("SCR file shorter than 8 bytes".into()));
        }
        if &data[0..4] != b"SCR\0" {
            return Err(PgError::InvalidMagic {
                expected: "SCR\\0",
                actual: data[0..4].to_vec(),
            });
        }
        let a = u16::from_le_bytes([data[4], data[5]]);
        let b = u16::from_le_bytes([data[6], data[7]]);
        Ok(a > 0 && b > 0)
    }

    pub fn from_bytes(data: &[u8]) -> Result<Self> {
        let is_bayo2 = Self::is_bayo2_format(data)?;
        if is_bayo2 {
            Self::read_bayo2(data)
        } else {
            Self::read_bayo1(data)
        }
    }

    fn read_bayo2(data: &[u8]) -> Result<Self> {
        let check = |endian: Endian| -> bool {
            let mut r = BinReader::new(data, endian);
            let _ = r.seek(4);
            let unknown = r.read_i16().unwrap_or(-1);
            let num_models = r.read_i16().unwrap_or(-1);
            let off_offsets = r.read_i32().unwrap_or(-1);
            unknown >= 0
                && num_models >= 0
                && (num_models as usize) * 0x8C + 0x10 < data.len()
                && off_offsets >= 0
                && (off_offsets as usize) < data.len()
        };
        let endian = match (check(Endian::Big), check(Endian::Little)) {
            (true, false) => Endian::Big,
            (false, true) => Endian::Little,
            _ => Endian::Little,
        };

        let mut r = BinReader::new(data, endian);
        let _id = r.read_bytes(4)?;
        let unknown = r.read_u16()? as u32;
        let num_models = r.read_u16()? as usize;
        let offset_offsets_models = r.read_u32()? as usize;

        r.seek(offset_offsets_models)?;
        let mut offsets_models_meta = Vec::with_capacity(num_models);
        for _ in 0..num_models {
            offsets_models_meta.push(r.read_u32()? as usize);
        }

        let mut offsets_models = Vec::with_capacity(num_models);
        let mut models_metadata = Vec::with_capacity(num_models);
        for &meta_off in &offsets_models_meta {
            r.seek(meta_off)?;
            let offset = r.read_u32()? as usize;
            let name = r.read_fixed_string(64)?;
            let mut transform = [0.0f32; 9];
            for slot in &mut transform {
                *slot = r.read_f32()?;
            }
            let mut unknown_shorts = Vec::with_capacity(18);
            for _ in 0..18 {
                unknown_shorts.push(r.read_i16()?);
            }
            offsets_models.push(offset);
            models_metadata.push(ScrModelMetadata {
                name,
                transform,
                unknown_shorts,
            });
        }

        let mut models = Vec::with_capacity(num_models);
        for i in 0..num_models {
            let start = offsets_models[i];
            let end = if i + 1 == num_models {
                data.len()
            } else {
                offsets_models_meta[i + 1]
            };
            let size = end.saturating_sub(start);
            models.push(r.slice_at(start, size)?.to_vec());
        }

        Ok(Self {
            is_bayo2: true,
            endian,
            unknown_header: unknown,
            models_metadata,
            models,
            textures: None,
        })
    }

    fn read_bayo1(data: &[u8]) -> Result<Self> {
        let check = |endian: Endian| -> bool {
            let mut r = BinReader::new(data, endian);
            let _ = r.seek(4);
            let num_models = r.read_i32().unwrap_or(-1);
            let off_tex = r.read_i32().unwrap_or(-1);
            num_models >= 0
                && (num_models as usize) * 0x8C + 0x10 < data.len()
                && off_tex >= 0
                && (off_tex as usize) <= data.len()
        };
        let endian = match (check(Endian::Big), check(Endian::Little)) {
            (true, false) => Endian::Big,
            (false, true) => Endian::Little,
            _ => Endian::Little,
        };

        let mut r = BinReader::new(data, endian);
        let _id = r.read_bytes(4)?;
        let num_models = r.read_u32()? as usize;
        let offset_texture = r.read_u32()? as usize;
        let unknown_header = r.read_u32()?;

        let mut offsets_models = Vec::with_capacity(num_models);
        let mut models_metadata = Vec::with_capacity(num_models);
        for _ in 0..num_models {
            let pos = r.pos();
            let name = r.read_fixed_string(16)?;
            let rel_offset = r.read_u32()? as usize;
            let mut transform = [0.0f32; 9];
            for slot in &mut transform {
                *slot = r.read_f32()?;
            }
            let mut unknown_shorts = Vec::with_capacity(42);
            for _ in 0..42 {
                unknown_shorts.push(r.read_i16()?);
            }
            offsets_models.push(pos + rel_offset);
            models_metadata.push(ScrModelMetadata {
                name,
                transform,
                unknown_shorts,
            });
        }

        let mut models = Vec::with_capacity(num_models);
        for i in 0..num_models {
            let start = offsets_models[i];
            let end = if i + 1 == num_models {
                offset_texture
            } else {
                offsets_models[i + 1]
            };
            let size = end.saturating_sub(start);
            models.push(r.slice_at(start, size)?.to_vec());
        }

        let textures = if offset_texture > 0 && offset_texture < data.len() {
            Some(r.slice_at(offset_texture, data.len() - offset_texture)?.to_vec())
        } else {
            None
        };

        Ok(Self {
            is_bayo2: false,
            endian,
            unknown_header,
            models_metadata,
            models,
            textures,
        })
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        if self.is_bayo2 {
            self.write_bayo2()
        } else {
            self.write_bayo1()
        }
    }

    fn write_bayo2(&self) -> Vec<u8> {
        let num_models = self.models.len();
        let offset_offsets_models = 0x10usize;
        let mut current_offset = offset_offsets_models + 4 * num_models;
        let mut offsets_models_meta = Vec::with_capacity(num_models);
        let mut offsets_models = Vec::with_capacity(num_models);

        for m in &self.models {
            current_offset = align(current_offset, 0x20);
            offsets_models_meta.push(current_offset);
            current_offset += 0x8C;
            current_offset = align(current_offset, 0x80);
            offsets_models.push(current_offset);
            current_offset += m.len();
        }
        let total_size = align(current_offset, 0x20);

        let mut w = BinWriter::with_capacity(total_size, self.endian);
        w.write_bytes(b"SCR\0");
        w.write_u16(self.unknown_header as u16);
        w.write_u16(num_models as u16);
        w.write_u32(offset_offsets_models as u32);

        w.seek(offset_offsets_models);
        for &meta_off in &offsets_models_meta {
            w.write_u32(meta_off as u32);
        }

        for i in 0..num_models {
            w.seek(offsets_models_meta[i]);
            w.write_u32(offsets_models[i] as u32);
            let meta = &self.models_metadata[i];
            w.write_fixed_string(&meta.name, 64);
            for &f in &meta.transform {
                w.write_f32(f);
            }
            for j in 0..18 {
                let s = meta.unknown_shorts.get(j).copied().unwrap_or(0);
                w.write_i16(s);
            }
            w.seek(offsets_models[i]);
            w.write_bytes(&self.models[i]);
        }

        w.seek(total_size);
        w.into_inner()
    }

    fn write_bayo1(&self) -> Vec<u8> {
        let num_models = self.models.len();
        let mut current_offset = 0x10 + 0x8C * num_models;
        let mut offsets_models = Vec::with_capacity(num_models);
        for m in &self.models {
            let tmp = align(current_offset, 0x20);
            current_offset = tmp + m.len();
            offsets_models.push(tmp);
        }
        let offset_texture = align(current_offset, 0x1000);
        let tex_bytes = self.textures.as_deref().unwrap_or(&[]);
        let total_size = offset_texture + tex_bytes.len();

        let mut w = BinWriter::with_capacity(total_size, self.endian);
        w.write_bytes(b"SCR\0");
        w.write_u32(num_models as u32);
        w.write_u32(offset_texture as u32);
        w.write_u32(self.unknown_header);

        for i in 0..num_models {
            let pos = w.pos();
            let meta = &self.models_metadata[i];
            w.write_fixed_string(&meta.name, 16);
            w.write_u32((offsets_models[i] - pos) as u32);
            for &f in &meta.transform {
                w.write_f32(f);
            }
            for j in 0..42 {
                let s = meta.unknown_shorts.get(j).copied().unwrap_or(0);
                w.write_i16(s);
            }
        }

        for (i, &off) in offsets_models.iter().enumerate() {
            w.seek(off);
            w.write_bytes(&self.models[i]);
        }

        if !tex_bytes.is_empty() {
            w.seek(offset_texture);
            w.write_bytes(tex_bytes);
        }

        w.seek(total_size);
        w.into_inner()
    }

    pub fn extract_to_dir(&self, output_dir: &Path) -> Result<()> {
        fs::create_dir_all(output_dir)?;
        for (i, m) in self.models.iter().enumerate() {
            fs::write(output_dir.join(format!("{:03}.wmb", i)), m)?;
        }
        if let Some(ref tex) = self.textures {
            fs::write(output_dir.join("000.wtb"), tex)?;
        }
        let meta_dir = output_dir.join(".metadata");
        fs::create_dir_all(&meta_dir)?;
        fs::write(
            meta_dir.join("models_metadata.yaml"),
            serde_yaml::to_string(&self.models_metadata)
                .map_err(|e| PgError::Serialization(e.to_string()))?,
        )?;
        fs::write(
            meta_dir.join("big.yaml"),
            serde_yaml::to_string(&self.endian.is_big())
                .map_err(|e| PgError::Serialization(e.to_string()))?,
        )?;
        fs::write(
            meta_dir.join("bayo2.yaml"),
            serde_yaml::to_string(&self.is_bayo2)
                .map_err(|e| PgError::Serialization(e.to_string()))?,
        )?;
        fs::write(
            meta_dir.join("unknown.yaml"),
            serde_yaml::to_string(&self.unknown_header)
                .map_err(|e| PgError::Serialization(e.to_string()))?,
        )?;
        Ok(())
    }
}
