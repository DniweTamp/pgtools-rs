//! Additional PlatinumGames binary formats ported from the 010 Editor `.bt` templates
//! in `binary_templates/`:
//! - `EST` (*Bayonetta 1 / 2 / NieR:Automata* particle effect records inside `.eff` — `Bayonetta est.bt`, `Bayonetta 2 est.bt`)
//! - `COS` (*Bayonetta 3* costume color palettes — `Bayonetta 3 cos.bt`)
//! - `SEQ` (*Bayonetta* speed/attack/sound/flag sequences — `Bayonetta seq base.bt`)
//! - `FLG` (*Bayonetta* mesh visibility/substruct flags — `Bayonetta flg base.bt`)
//! - `OBI` (*Bayonetta* object placement instances — `Bayonetta obi base.bt`)
//! - `ATT` (*Bayonetta* bone attachment points — `Bayonetta att base.bt`)
//! - `MSQ` (*Bayonetta 2* subtitle/speech sequence timings — `Bayonetta 2 msq base.bt`)
//! - `COL` (*Bayonetta* / *NieR:Automata* spatial collision mesh header & groups — `Bayonetta col base.bt`)

use crate::bone::Vec3;
use crate::io::{BinReader, BinWriter, Endian};
use crate::{PgError, Result};
use serde::{Deserialize, Serialize};

/// Editable particle effect record parsed from a `.est` file (standalone or nested inside `.eff` inside `.dat`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EstRecord {
    pub index: usize,
    pub group_tag: String,
    pub byte_offset: usize,
    pub byte_size: usize,
    pub enabled_flag: i16,
    pub attach_bone: i16,
    pub offset: Vec3,
    pub scale: f32,
    pub color_rgba: [f32; 4],
    pub speed_x: f32,
    pub speed_y: f32,
    pub speed_z: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EstFile {
    pub endian: Endian,
    pub version_label: String,
    pub records: Vec<EstRecord>,
}

impl EstFile {
    pub fn from_bytes(data: &[u8]) -> Result<Self> {
        if data.len() < 16 {
            return Err(PgError::InvalidData("EST file shorter than 16 bytes".into()));
        }

        // Check for Bayonetta 2 / NieR:Automata `EST\0` or `\0TSE` header
        let magic = &data[0..4];
        if magic == b"EST\0" || magic == b"\0TSE" {
            let endian = if magic == b"EST\0" {
                Endian::Little
            } else {
                Endian::Big
            };
            let mut r = BinReader::new(data, endian);
            let _id = r.read_bytes(4)?;
            let record_count = r.read_u32()? as usize;
            let type_offsets_offset = r.read_u32()? as usize;
            let type_data_offset = r.read_u32()? as usize;
            let type_names_offset = r.read_u32()? as usize;
            let type_number = r.read_u32()? as usize;

            let mut tags = Vec::with_capacity(type_number);
            if type_names_offset + type_number * 4 <= data.len() {
                r.seek(type_names_offset)?;
                for _ in 0..type_number {
                    let b = r.read_bytes(4)?;
                    let mut s: String = b
                        .iter()
                        .filter(|&&c| c.is_ascii_graphic() || c == b' ')
                        .map(|&c| c as char)
                        .collect();
                    if endian.is_little() {
                        s = s.chars().rev().collect();
                    }
                    tags.push(s.trim().to_string());
                }
            }

            // Read descriptor table at `type_offsets_offset`: `record_count * type_number` entries of 8 bytes (u16 u_a, u16 size, u32 offset)
            let mut records = Vec::new();
            for rec_idx in 0..record_count.min(256) {
                for (t_idx, tag) in tags.iter().enumerate() {
                    let desc_pos = type_offsets_offset + (rec_idx * type_number + t_idx) * 8;
                    if desc_pos + 8 > data.len() {
                        break;
                    }
                    r.seek(desc_pos)?;
                    let _u_a = r.read_u16()?;
                    let sz = r.read_u16()? as usize;
                    let rel_off = r.read_u32()? as usize;
                    if sz == 0 {
                        continue;
                    }
                    let abs_off = type_data_offset + rel_off;
                    if abs_off + sz > data.len() {
                        continue;
                    }
                    if tag == "MOVE" && sz >= 0xAC {
                        r.seek(abs_off + 4)?;
                        let ox = r.read_f32()?;
                        let oy = r.read_f32()?;
                        let oz = r.read_f32()?;
                        r.seek(abs_off + 0x50)?;
                        let sx = r.read_f32().unwrap_or(0.0);
                        let sy = r.read_f32().unwrap_or(0.0);
                        let sz_v = r.read_f32().unwrap_or(0.0);
                        r.seek(abs_off + 0x90)?;
                        let scale = r.read_f32().unwrap_or(1.0);
                        r.seek(abs_off + 0x9C)?;
                        let red = r.read_f32().unwrap_or(1.0);
                        let green = r.read_f32().unwrap_or(1.0);
                        let blue = r.read_f32().unwrap_or(1.0);
                        let alpha = r.read_f32().unwrap_or(1.0);
                        records.push(EstRecord {
                            index: rec_idx,
                            group_tag: "MOVE".to_string(),
                            byte_offset: abs_off,
                            byte_size: sz,
                            enabled_flag: 1,
                            attach_bone: 0,
                            offset: Vec3::new(ox, oy, oz),
                            scale,
                            color_rgba: [red, green, blue, alpha],
                            speed_x: sx,
                            speed_y: sy,
                            speed_z: sz_v,
                        });
                    } else if tag == "PART" && sz >= 8 {
                        r.seek(abs_off)?;
                        let u_a = r.read_i16()?;
                        let anchor_bone = r.read_i16()?;
                        records.push(EstRecord {
                            index: rec_idx,
                            group_tag: "PART".to_string(),
                            byte_offset: abs_off,
                            byte_size: sz,
                            enabled_flag: u_a,
                            attach_bone: anchor_bone,
                            offset: Vec3::default(),
                            scale: 1.0,
                            color_rgba: [1.0, 1.0, 1.0, 1.0],
                            speed_x: 0.0,
                            speed_y: 0.0,
                            speed_z: 0.0,
                        });
                    } else {
                        records.push(EstRecord {
                            index: rec_idx,
                            group_tag: tag.clone(),
                            byte_offset: abs_off,
                            byte_size: sz,
                            enabled_flag: 1,
                            attach_bone: 0,
                            offset: Vec3::default(),
                            scale: 1.0,
                            color_rgba: [1.0, 1.0, 1.0, 1.0],
                            speed_x: 0.0,
                            speed_y: 0.0,
                            speed_z: 0.0,
                        });
                    }
                }
            }

            return Ok(Self {
                endian,
                version_label: "Bayonetta 2 / NieR:Automata EST".to_string(),
                records,
            });
        }

        // Bayonetta 1 EST (`int32 datumNumber`, followed by `datumNumber` u32 offsets to 704-byte (`0x2C0`) effect structs)
        let n_le = i32::from_le_bytes(data[0..4].try_into().unwrap());
        let endian = if (1..=512).contains(&n_le) {
            Endian::Little
        } else {
            Endian::Big
        };
        let mut r = BinReader::new(data, endian);
        let count = (r.read_i32()?.max(0) as usize).min(512);
        let mut offsets = Vec::with_capacity(count);
        for _ in 0..count {
            offsets.push(r.read_u32()? as usize);
        }

        let mut records = Vec::new();
        for (i, &off) in offsets.iter().enumerate() {
            if off == 0 || off + 0xAC > data.len() {
                continue;
            }
            r.seek(off)?;
            let enabled_flag = r.read_i16()?;
            let _life = r.read_i16()?;
            let attach_bone = r.read_i16()?;
            r.seek(off + 0x24)?;
            let ox = r.read_f32()?;
            let oy = r.read_f32()?;
            let oz = r.read_f32()?;
            r.seek(off + 0x6C)?;
            let sx = r.read_f32().unwrap_or(0.0);
            let sy = r.read_f32().unwrap_or(0.0);
            let sz_v = r.read_f32().unwrap_or(0.0);
            r.seek(off + 0x90)?;
            let scale = r.read_f32().unwrap_or(1.0);
            r.seek(off + 0x9C)?;
            let red = r.read_f32().unwrap_or(1.0);
            let green = r.read_f32().unwrap_or(1.0);
            let blue = r.read_f32().unwrap_or(1.0);
            let alpha = r.read_f32().unwrap_or(1.0);
            records.push(EstRecord {
                index: i,
                group_tag: "EFFECT".to_string(),
                byte_offset: off,
                byte_size: (data.len() - off).min(0x2C0),
                enabled_flag,
                attach_bone,
                offset: Vec3::new(ox, oy, oz),
                scale,
                color_rgba: [red, green, blue, alpha],
                speed_x: sx,
                speed_y: sy,
                speed_z: sz_v,
            });
        }

        Ok(Self {
            endian,
            version_label: "Bayonetta 1 EST".to_string(),
            records,
        })
    }

    /// Apply edited `EstRecord` values directly onto the original `.est` binary buffer, preserving all
    /// unknown shader/curve fields bit-for-bit.
    pub fn apply_to_bytes(&self, original: &[u8]) -> Vec<u8> {
        let mut out = original.to_vec();
        let write_i16 = |buf: &mut [u8], pos: usize, val: i16, endian: Endian| {
            if pos + 2 <= buf.len() {
                let bytes = match endian {
                    Endian::Little => val.to_le_bytes(),
                    Endian::Big => val.to_be_bytes(),
                };
                buf[pos..pos + 2].copy_from_slice(&bytes);
            }
        };
        let write_f32 = |buf: &mut [u8], pos: usize, val: f32, endian: Endian| {
            if pos + 4 <= buf.len() {
                let bytes = match endian {
                    Endian::Little => val.to_le_bytes(),
                    Endian::Big => val.to_be_bytes(),
                };
                buf[pos..pos + 4].copy_from_slice(&bytes);
            }
        };

        for rec in &self.records {
            let off = rec.byte_offset;
            match rec.group_tag.as_str() {
                "MOVE" => {
                    write_f32(&mut out, off + 4, rec.offset.x, self.endian);
                    write_f32(&mut out, off + 8, rec.offset.y, self.endian);
                    write_f32(&mut out, off + 12, rec.offset.z, self.endian);
                    write_f32(&mut out, off + 0x50, rec.speed_x, self.endian);
                    write_f32(&mut out, off + 0x54, rec.speed_y, self.endian);
                    write_f32(&mut out, off + 0x58, rec.speed_z, self.endian);
                    write_f32(&mut out, off + 0x90, rec.scale, self.endian);
                    write_f32(&mut out, off + 0x9C, rec.color_rgba[0], self.endian);
                    write_f32(&mut out, off + 0xA0, rec.color_rgba[1], self.endian);
                    write_f32(&mut out, off + 0xA4, rec.color_rgba[2], self.endian);
                    write_f32(&mut out, off + 0xA8, rec.color_rgba[3], self.endian);
                }
                "PART" => {
                    write_i16(&mut out, off, rec.enabled_flag, self.endian);
                    write_i16(&mut out, off + 2, rec.attach_bone, self.endian);
                }
                "EFFECT" => {
                    write_i16(&mut out, off, rec.enabled_flag, self.endian);
                    write_i16(&mut out, off + 4, rec.attach_bone, self.endian);
                    write_f32(&mut out, off + 0x24, rec.offset.x, self.endian);
                    write_f32(&mut out, off + 0x28, rec.offset.y, self.endian);
                    write_f32(&mut out, off + 0x2C, rec.offset.z, self.endian);
                    write_f32(&mut out, off + 0x6C, rec.speed_x, self.endian);
                    write_f32(&mut out, off + 0x70, rec.speed_y, self.endian);
                    write_f32(&mut out, off + 0x74, rec.speed_z, self.endian);
                    write_f32(&mut out, off + 0x90, rec.scale, self.endian);
                    write_f32(&mut out, off + 0x9C, rec.color_rgba[0], self.endian);
                    write_f32(&mut out, off + 0xA0, rec.color_rgba[1], self.endian);
                    write_f32(&mut out, off + 0xA4, rec.color_rgba[2], self.endian);
                    write_f32(&mut out, off + 0xA8, rec.color_rgba[3], self.endian);
                }
                _ => {}
            }
        }
        out
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CosEntry {
    pub name: String,
    pub colors: [[f32; 3]; 7],
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CosFile {
    pub version: u32,
    pub entries: Vec<CosEntry>,
}

impl CosFile {
    pub fn from_bytes(data: &[u8]) -> Result<Self> {
        let mut r = BinReader::new(data, Endian::Little);
        let size = r.read_u32()?;
        let version = r.read_u32()?;
        let offset_entries = r.read_u32()? as usize;
        let num_entries = r.read_u32()? as usize;

        let (actual_offset, actual_count) = if size != 0 && num_entries != 0 {
            (offset_entries, num_entries)
        } else {
            r.seek(offset_entries)?;
            let _size2 = r.read_u32()?;
            let _ver2 = r.read_u32()?;
            let count2 = r.read_u32()? as usize;
            let off2 = r.read_u32()? as usize;
            (offset_entries + off2, count2)
        };

        r.seek(actual_offset)?;
        let mut entries = Vec::with_capacity(actual_count);
        for _ in 0..actual_count {
            let mut colors = [[0.0f32; 3]; 7];
            for rgb in &mut colors {
                rgb[0] = r.read_f32()?;
                rgb[1] = r.read_f32()?;
                rgb[2] = r.read_f32()?;
            }
            let name = r.read_fixed_string(0x40)?;
            entries.push(CosEntry { name, colors });
        }
        Ok(Self { version, entries })
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let offset_entries = 16usize;
        let total_size = offset_entries + self.entries.len() * (7 * 12 + 0x40);
        let mut w = BinWriter::with_capacity(total_size, Endian::Little);
        w.write_u32(total_size as u32);
        w.write_u32(self.version);
        w.write_u32(offset_entries as u32);
        w.write_u32(self.entries.len() as u32);
        for e in &self.entries {
            for rgb in &e.colors {
                w.write_f32(rgb[0]);
                w.write_f32(rgb[1]);
                w.write_f32(rgb[2]);
            }
            w.write_fixed_string(&e.name, 0x40);
        }
        w.into_inner()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AttPoint {
    pub source_bone: i32,
    pub target_bone: i32,
    pub option: i32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AttFile {
    pub endian: Endian,
    pub points: Vec<AttPoint>,
}

impl AttFile {
    pub fn from_bytes(data: &[u8]) -> Result<Self> {
        if data.len() < 4 {
            return Err(PgError::InvalidData("ATT file shorter than 4 bytes".into()));
        }
        let n_le = i32::from_le_bytes(data[0..4].try_into().unwrap());
        let endian = if (0..4096).contains(&n_le) {
            Endian::Little
        } else {
            Endian::Big
        };
        let mut r = BinReader::new(data, endian);
        let count = r.read_i32()?.max(0) as usize;
        let mut points = Vec::with_capacity(count);
        for _ in 0..count {
            points.push(AttPoint {
                source_bone: r.read_i32()?,
                target_bone: r.read_i32()?,
                option: r.read_i32()?,
            });
        }
        Ok(Self { endian, points })
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut w = BinWriter::with_capacity(4 + self.points.len() * 12, self.endian);
        w.write_i32(self.points.len() as i32);
        for p in &self.points {
            w.write_i32(p.source_bone);
            w.write_i32(p.target_bone);
            w.write_i32(p.option);
        }
        w.into_inner()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ObiEntry {
    pub object_id: u32,
    pub object_name: String,
    pub flags: i32,
    pub position: Vec3,
    pub orientation: Vec3,
    pub scale: Vec3,
    pub difficulty_selector: u8,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ObiFile {
    pub endian: Endian,
    pub entries: Vec<ObiEntry>,
}

impl ObiFile {
    pub fn format_object_id(id: u32) -> String {
        let prefix = match id >> 16 {
            0 => "sc",
            1 => "pl",
            2 => "em",
            3 => "wp",
            4 => "bg",
            5 => "gm",
            6 => "et",
            7 => "ef",
            9 => "it",
            10 => "id",
            11 => "es",
            12 => "bm",
            13 => "ic",
            16 => "ct",
            _ => "uk",
        };
        format!("{}{:04x}", prefix, id & 0xFFFF)
    }

    pub fn from_bytes(data: &[u8]) -> Result<Self> {
        if data.len() < 32 {
            return Err(PgError::InvalidData("OBI file shorter than 32-byte header".into()));
        }
        let n_le = i32::from_le_bytes(data[4..8].try_into().unwrap());
        let endian = if (0..10000).contains(&n_le) {
            Endian::Little
        } else {
            Endian::Big
        };
        let mut r = BinReader::new(data, endian);
        let _id = r.read_bytes(4)?;
        let num_entries = r.read_i32()?.max(0) as usize;
        let _u_a = r.read_f32()?;
        let _u_b = r.read_f32()?;
        let _u_off = r.read_u32()?;
        let _ = r.read_bytes(12)?;

        let mut entries = Vec::with_capacity(num_entries);
        for _ in 0..num_entries {
            let object_id = r.read_u32()?;
            let _u_a = r.read_i32()?;
            let flags = r.read_i32()?;
            let _u_b = r.read_i32()?;
            let position = Vec3::read(&mut r)?;
            let orientation = Vec3::read(&mut r)?;
            let scale = Vec3::read(&mut r)?;
            let _u_c = r.read_i32()?;
            let difficulty_selector = r.read_u8()?;
            let _rest = r.read_bytes(3 + 8 + 4 + 56)?;
            entries.push(ObiEntry {
                object_id,
                object_name: Self::format_object_id(object_id),
                flags,
                position,
                orientation,
                scale,
                difficulty_selector,
            });
        }
        Ok(Self { endian, entries })
    }

    pub fn apply_to_bytes(&self, original: &[u8]) -> Vec<u8> {
        let mut out = original.to_vec();
        let write_u32 = |buf: &mut [u8], pos: usize, val: u32, endian: Endian| {
            if pos + 4 <= buf.len() {
                let b = match endian {
                    Endian::Little => val.to_le_bytes(),
                    Endian::Big => val.to_be_bytes(),
                };
                buf[pos..pos + 4].copy_from_slice(&b);
            }
        };
        let write_i32 = |buf: &mut [u8], pos: usize, val: i32, endian: Endian| {
            if pos + 4 <= buf.len() {
                let b = match endian {
                    Endian::Little => val.to_le_bytes(),
                    Endian::Big => val.to_be_bytes(),
                };
                buf[pos..pos + 4].copy_from_slice(&b);
            }
        };
        let write_f32 = |buf: &mut [u8], pos: usize, val: f32, endian: Endian| {
            if pos + 4 <= buf.len() {
                let b = match endian {
                    Endian::Little => val.to_le_bytes(),
                    Endian::Big => val.to_be_bytes(),
                };
                buf[pos..pos + 4].copy_from_slice(&b);
            }
        };
        for (i, e) in self.entries.iter().enumerate() {
            let base = 32 + i * 128;
            if base + 128 > out.len() {
                break;
            }
            write_u32(&mut out, base, e.object_id, self.endian);
            write_i32(&mut out, base + 8, e.flags, self.endian);
            write_f32(&mut out, base + 16, e.position.x, self.endian);
            write_f32(&mut out, base + 20, e.position.y, self.endian);
            write_f32(&mut out, base + 24, e.position.z, self.endian);
            write_f32(&mut out, base + 28, e.orientation.x, self.endian);
            write_f32(&mut out, base + 32, e.orientation.y, self.endian);
            write_f32(&mut out, base + 36, e.orientation.z, self.endian);
            write_f32(&mut out, base + 40, e.scale.x, self.endian);
            write_f32(&mut out, base + 44, e.scale.y, self.endian);
            write_f32(&mut out, base + 48, e.scale.z, self.endian);
            out[base + 56] = e.difficulty_selector;
        }
        out
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MsqEvent {
    pub name: String,
    pub id: u32,
    pub event_type: u32,
    pub line_number_per_group: u32,
    pub event_duration: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MsqFile {
    pub endian: Endian,
    pub events: Vec<MsqEvent>,
}

impl MsqFile {
    pub fn from_bytes(data: &[u8]) -> Result<Self> {
        if data.len() < 8 {
            return Err(PgError::InvalidData("MSQ file shorter than 8 bytes".into()));
        }
        let n_le = u32::from_le_bytes(data[0..4].try_into().unwrap());
        let endian = if n_le < 10000 { Endian::Little } else { Endian::Big };
        let mut r = BinReader::new(data, endian);
        let count = r.read_u32()? as usize;
        let _offset = r.read_u32()?;
        let mut events = Vec::with_capacity(count);
        for _ in 0..count {
            let name = r.read_fixed_string(0x20)?;
            let id = r.read_u32()?;
            let event_type = r.read_u32()?;
            let line_number_per_group = r.read_u32()?;
            let _last_group_stop = r.read_u32()?;
            let event_duration = r.read_u32()?;
            let _groups = r.read_bytes(24)?;
            events.push(MsqEvent {
                name,
                id,
                event_type,
                line_number_per_group,
                event_duration,
            });
        }
        Ok(Self { endian, events })
    }

    pub fn apply_to_bytes(&self, original: &[u8]) -> Vec<u8> {
        let mut out = original.to_vec();
        let write_u32 = |buf: &mut [u8], pos: usize, val: u32, endian: Endian| {
            if pos + 4 <= buf.len() {
                let b = match endian {
                    Endian::Little => val.to_le_bytes(),
                    Endian::Big => val.to_be_bytes(),
                };
                buf[pos..pos + 4].copy_from_slice(&b);
            }
        };
        for (i, ev) in self.events.iter().enumerate() {
            let base = 8 + i * 76;
            if base + 76 > out.len() {
                break;
            }
            let mut name_buf = [0u8; 0x20];
            let bytes = ev.name.as_bytes();
            let copy_len = bytes.len().min(0x1F);
            name_buf[..copy_len].copy_from_slice(&bytes[..copy_len]);
            out[base..base + 0x20].copy_from_slice(&name_buf);
            write_u32(&mut out, base + 0x20, ev.id, self.endian);
            write_u32(&mut out, base + 0x24, ev.event_type, self.endian);
            write_u32(&mut out, base + 0x28, ev.line_number_per_group, self.endian);
            write_u32(&mut out, base + 0x30, ev.event_duration, self.endian);
        }
        out
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ColGroup {
    pub index: u8,
    pub name: String,
    pub flags: u32,
    pub flags2: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ColFileSummary {
    pub endian: Endian,
    #[serde(default)]
    pub base_offset: usize,
    #[serde(default)]
    pub offset_groups: usize,
    pub num_groups: i32,
    pub num_entries: i32,
    pub num_vertices: i32,
    pub num_indices: i32,
    pub groups: Vec<ColGroup>,
}

impl ColFileSummary {
    pub fn from_bytes(data: &[u8]) -> Result<Self> {
        if data.len() < 0x50 {
            return Err(PgError::InvalidData("COL file too small".into()));
        }
        let mut base = 0usize;
        if &data[0..4] == b"col\0" {
            let off_le = u32::from_le_bytes(data[8..12].try_into().unwrap()) as usize;
            let off_be = u32::from_be_bytes(data[8..12].try_into().unwrap()) as usize;
            let table_off = if off_le < data.len() { off_le } else { off_be };
            if table_off + 4 <= data.len() {
                let first_le =
                    u32::from_le_bytes(data[table_off..table_off + 4].try_into().unwrap()) as usize;
                let first_be =
                    u32::from_be_bytes(data[table_off..table_off + 4].try_into().unwrap()) as usize;
                base = if first_le < data.len() { first_le } else { first_be };
            }
        }
        let mut r = BinReader::new(&data[base..], Endian::Little);
        let _id = r.read_bytes(4)?;
        let _version = r.read_f32()?;
        let _depth = r.read_i16()?;
        let _num_nodes = r.read_i16()?;
        let _u_c = r.read_bytes(4)?;
        let num_groups = r.read_i32()?;
        let num_entries = r.read_i32()?;
        let num_vertices = r.read_i32()?;
        let num_indices = r.read_i32()?;
        let offset_groups = r.read_i32()? as usize;

        let mut groups = Vec::new();
        if num_groups > 0 && offset_groups + (num_groups as usize) * 48 <= r.len() {
            r.seek(offset_groups)?;
            for _ in 0..(num_groups as usize) {
                let index = r.read_u8()?;
                let name = r.read_fixed_string(0x1B)?;
                let flags = r.read_u32()?;
                let flags2 = r.read_u32()?;
                let _u_d = r.read_bytes(0x0C)?;
                groups.push(ColGroup {
                    index,
                    name,
                    flags,
                    flags2,
                });
            }
        }

        Ok(Self {
            endian: Endian::Little,
            base_offset: base,
            offset_groups,
            num_groups,
            num_entries,
            num_vertices,
            num_indices,
            groups,
        })
    }

    pub fn apply_to_bytes(&self, original: &[u8]) -> Vec<u8> {
        let mut out = original.to_vec();
        let grp_start = self.base_offset + self.offset_groups;
        for (i, g) in self.groups.iter().enumerate() {
            let pos = grp_start + i * 48;
            if pos + 48 > out.len() {
                break;
            }
            out[pos] = g.index;
            let mut nbuf = [0u8; 0x1B];
            let nb = g.name.as_bytes();
            let cl = nb.len().min(0x1A);
            nbuf[..cl].copy_from_slice(&nb[..cl]);
            out[pos + 1..pos + 0x1C].copy_from_slice(&nbuf);
            out[pos + 0x1C..pos + 0x20].copy_from_slice(&g.flags.to_le_bytes());
            out[pos + 0x20..pos + 0x24].copy_from_slice(&g.flags2.to_le_bytes());
        }
        out
    }
}

// ============================================================================
// STAGE LIGHTING FORMATS (.fls, .fla, .cta, .lit) from Bayonetta2Blender2Bayonetta
// ============================================================================

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FlsFogRecord {
    pub fog_color: [f32; 4],
    pub fog_near: f32,
    pub fog_far: f32,
    pub scr_color: [f32; 4],
    pub flag: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FlsFilterRecord {
    pub no: u32,
    pub role: String,
    pub byte_offset: usize,
    pub flag: [u32; 4],
    pub point_num: u32,
    pub saido: [f32; 4],
    pub color: [f32; 4],
    pub bg_color: [f32; 4],
    pub glare_color: [f32; 4],
    pub add_color: [f32; 4],
    pub kido_base: f32,
    pub kido_range: f32,
    pub kido_min: f32,
    pub kido_max: f32,
    pub saido_range: f32,
    pub kido_spd: f32,
    pub glare_range: f32,
    pub glare_range_alpha_mul: f32,
    pub shadow_light_no: i32,
    pub dof_back_far: f32,
    pub dof_back_near: f32,
    pub dof_front_far: f32,
    pub dof_front_near: f32,
    pub dof_pow: f32,
    pub dof_type: u32,
    pub fogs: Vec<FlsFogRecord>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FlsFile {
    pub filters: Vec<FlsFilterRecord>,
}

impl FlsFile {
    pub fn filter_role(no: u32) -> &'static str {
        match no {
            0 => "Room default (outside every area)",
            40 => "Night (cFilterManager::setNightFilter)",
            100 => "Witch Time / battle",
            1..=39 => "Area filter (on top of filter 0)",
            41..=99 => "Night area filter",
            _ => "Witch Time area filter",
        }
    }

    pub fn from_bytes(data: &[u8]) -> Result<Self> {
        if data.len() < 8 || &data[0..3] != b"FLS" {
            return Err(PgError::InvalidMagic {
                expected: "FLS",
                actual: data.get(0..4).unwrap_or(&[]).to_vec(),
            });
        }
        let mut r = BinReader::new(data, Endian::Little);
        let _magic = r.read_u32()?;
        let num = r.read_u32()? as usize;
        if num > 4096 || 8 + 8 * num > data.len() {
            return Err(PgError::InvalidData("Implausible FLS filter count".into()));
        }
        let mut table = Vec::with_capacity(num);
        for _ in 0..num {
            table.push((r.read_u32()?, r.read_u32()? as usize));
        }
        let mut filters = Vec::with_capacity(num);
        for (no, off) in table {
            if off + 2244 > data.len() {
                continue;
            }
            r.seek(off)?;
            let flag = [r.read_u32()?, r.read_u32()?, r.read_u32()?, r.read_u32()?];
            let point_num = r.read_u32()?;
            r.seek(off + 36)?;
            let read4 = |r: &mut BinReader| -> Result<[f32; 4]> {
                Ok([r.read_f32()?, r.read_f32()?, r.read_f32()?, r.read_f32()?])
            };
            let saido = read4(&mut r)?;
            let color = read4(&mut r)?;
            let bg_color = read4(&mut r)?;
            let glare_color = read4(&mut r)?;
            let add_color = read4(&mut r)?;
            let mut fogs = Vec::with_capacity(16);
            for j in 0..16 {
                let fo = off + 148 + 108 * j;
                r.seek(fo)?;
                let fog_color = read4(&mut r)?;
                let fog_near = r.read_f32()?;
                let fog_far = r.read_f32()?;
                let scr_color = read4(&mut r)?;
                let fflag = r.read_u32()?;
                fogs.push(FlsFogRecord {
                    fog_color,
                    fog_near,
                    fog_far,
                    scr_color,
                    flag: fflag,
                });
            }
            r.seek(off + 1876)?;
            let kido_base = r.read_f32()?;
            let kido_range = r.read_f32()?;
            let kido_min = r.read_f32()?;
            let kido_max = r.read_f32()?;
            let saido_range = r.read_f32()?;
            let kido_spd = r.read_f32()?;
            let glare_range = r.read_f32()?;
            let glare_range_alpha_mul = r.read_f32()?;
            let shadow_light_no = r.read_i32()?;
            let dof_back_far = r.read_f32()?;
            let dof_back_near = r.read_f32()?;
            let dof_front_far = r.read_f32()?;
            let dof_front_near = r.read_f32()?;
            let dof_pow = r.read_f32()?;
            let dof_type = r.read_u32()?;
            filters.push(FlsFilterRecord {
                no,
                role: Self::filter_role(no).to_string(),
                byte_offset: off,
                flag,
                point_num,
                saido,
                color,
                bg_color,
                glare_color,
                add_color,
                kido_base,
                kido_range,
                kido_min,
                kido_max,
                saido_range,
                kido_spd,
                glare_range,
                glare_range_alpha_mul,
                shadow_light_no,
                dof_back_far,
                dof_back_near,
                dof_front_far,
                dof_front_near,
                dof_pow,
                dof_type,
                fogs,
            });
        }
        Ok(Self { filters })
    }

    pub fn apply_to_bytes(&self, original: &[u8]) -> Vec<u8> {
        let mut out = original.to_vec();
        let wf = |buf: &mut [u8], pos: usize, v: f32| {
            if pos + 4 <= buf.len() {
                buf[pos..pos + 4].copy_from_slice(&v.to_le_bytes());
            }
        };
        let wu = |buf: &mut [u8], pos: usize, v: u32| {
            if pos + 4 <= buf.len() {
                buf[pos..pos + 4].copy_from_slice(&v.to_le_bytes());
            }
        };
        let wf4 = |buf: &mut [u8], pos: usize, v: [f32; 4]| {
            for k in 0..4 {
                wf(buf, pos + k * 4, v[k]);
            }
        };
        for f in &self.filters {
            let off = f.byte_offset;
            if off + 2244 > out.len() {
                continue;
            }
            for k in 0..4 {
                wu(&mut out, off + k * 4, f.flag[k]);
            }
            wu(&mut out, off + 16, f.point_num);
            wf4(&mut out, off + 36, f.saido);
            wf4(&mut out, off + 52, f.color);
            wf4(&mut out, off + 68, f.bg_color);
            wf4(&mut out, off + 84, f.glare_color);
            wf4(&mut out, off + 100, f.add_color);
            for (j, fog) in f.fogs.iter().take(16).enumerate() {
                let fo = off + 148 + 108 * j;
                wf4(&mut out, fo, fog.fog_color);
                wf(&mut out, fo + 16, fog.fog_near);
                wf(&mut out, fo + 20, fog.fog_far);
                wf4(&mut out, fo + 24, fog.scr_color);
                wu(&mut out, fo + 40, fog.flag);
            }
            wf(&mut out, off + 1876, f.kido_base);
            wf(&mut out, off + 1880, f.kido_range);
            wf(&mut out, off + 1884, f.kido_min);
            wf(&mut out, off + 1888, f.kido_max);
            wf(&mut out, off + 1892, f.saido_range);
            wf(&mut out, off + 1896, f.kido_spd);
            wf(&mut out, off + 1900, f.glare_range);
            wf(&mut out, off + 1904, f.glare_range_alpha_mul);
            wu(&mut out, off + 1908, f.shadow_light_no as u32);
            wf(&mut out, off + 1912, f.dof_back_far);
            wf(&mut out, off + 1916, f.dof_back_near);
            wf(&mut out, off + 1920, f.dof_front_far);
            wf(&mut out, off + 1924, f.dof_front_near);
            wf(&mut out, off + 1928, f.dof_pow);
            wu(&mut out, off + 1932, f.dof_type);
        }
        out
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct XadAreaEntry {
    pub index: usize,
    pub byte_offset: usize,
    pub shape_offset: usize,
    pub no: u16,
    pub area_type: u16,
    pub room_no: u16,
    pub flt_no: u16,
    pub in_time: u16,
    pub out_time: u16,
    pub flag: u32,
    pub shape_kind: String,
    pub center: [f32; 3],
    pub top_y: f32,
    pub bottom_y: f32,
    pub radius: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct XadFile {
    pub entries: Vec<XadAreaEntry>,
}

impl XadFile {
    pub fn from_bytes(data: &[u8]) -> Result<Self> {
        if data.len() < 16 || &data[0..3] != b"XAD" {
            return Err(PgError::InvalidMagic {
                expected: "XAD",
                actual: data.get(0..4).unwrap_or(&[]).to_vec(),
            });
        }
        let mut r = BinReader::new(data, Endian::Little);
        let _magic = r.read_u32()?;
        let num = r.read_u32()? as usize;
        let fix = (r.read_u32()? as usize).max(0x60);
        let mut entries = Vec::with_capacity(num);
        for i in 0..num {
            let eo = 16 + fix * i;
            if eo + 20 > data.len() {
                break;
            }
            r.seek(eo)?;
            let at_off = r.read_u32()? as usize;
            let no = r.read_u16()?;
            let area_type = r.read_u16()?;
            let room_no = r.read_u16()?;
            let flt_no = r.read_u16()?;
            let in_time = r.read_u16()?;
            let out_time = r.read_u16()?;
            let flag = r.read_u32()?;
            let so = if at_off > 0 { eo + at_off } else { 0 };
            let mut shape_kind = "none".to_string();
            let mut center = [0.0f32; 3];
            let mut top_y = 0.0f32;
            let mut bottom_y = 0.0f32;
            let mut radius = 0.0f32;
            if so > 0 && so + 0x3C <= data.len() {
                let stype = data[so];
                r.seek(so + 0x0C)?;
                radius = r.read_f32()?;
                center = [r.read_f32()?, r.read_f32()?, r.read_f32()?];
                if stype == 2 && so + 0x58 <= data.len() {
                    shape_kind = "prism".to_string();
                    r.seek(so + 0x50)?;
                    top_y = r.read_f32()?;
                    bottom_y = r.read_f32()?;
                } else if stype == 1 {
                    shape_kind = "cylinder".to_string();
                    r.seek(so + 0x30)?;
                    radius = r.read_f32()?;
                    top_y = r.read_f32()?;
                    bottom_y = r.read_f32()?;
                }
            }
            entries.push(XadAreaEntry {
                index: i,
                byte_offset: eo,
                shape_offset: so,
                no,
                area_type,
                room_no,
                flt_no,
                in_time,
                out_time,
                flag,
                shape_kind,
                center,
                top_y,
                bottom_y,
                radius,
            });
        }
        Ok(Self { entries })
    }

    pub fn apply_to_bytes(&self, original: &[u8]) -> Vec<u8> {
        let mut out = original.to_vec();
        for e in &self.entries {
            let eo = e.byte_offset;
            if eo + 20 > out.len() {
                continue;
            }
            out[eo + 4..eo + 6].copy_from_slice(&e.no.to_le_bytes());
            out[eo + 6..eo + 8].copy_from_slice(&e.area_type.to_le_bytes());
            out[eo + 8..eo + 10].copy_from_slice(&e.room_no.to_le_bytes());
            out[eo + 10..eo + 12].copy_from_slice(&e.flt_no.to_le_bytes());
            out[eo + 12..eo + 14].copy_from_slice(&e.in_time.to_le_bytes());
            out[eo + 14..eo + 16].copy_from_slice(&e.out_time.to_le_bytes());
            out[eo + 16..eo + 20].copy_from_slice(&e.flag.to_le_bytes());
            let so = e.shape_offset;
            if so > 0 && so + 0x3C <= out.len() {
                out[so + 0x10..so + 0x14].copy_from_slice(&e.center[0].to_le_bytes());
                out[so + 0x14..so + 0x18].copy_from_slice(&e.center[1].to_le_bytes());
                out[so + 0x18..so + 0x1C].copy_from_slice(&e.center[2].to_le_bytes());
                if e.shape_kind == "prism" && so + 0x58 <= out.len() {
                    out[so + 0x0C..so + 0x10].copy_from_slice(&e.radius.to_le_bytes());
                    out[so + 0x50..so + 0x54].copy_from_slice(&e.top_y.to_le_bytes());
                    out[so + 0x54..so + 0x58].copy_from_slice(&e.bottom_y.to_le_bytes());
                } else if e.shape_kind == "cylinder" {
                    out[so + 0x30..so + 0x34].copy_from_slice(&e.radius.to_le_bytes());
                    out[so + 0x34..so + 0x38].copy_from_slice(&e.top_y.to_le_bytes());
                    out[so + 0x38..so + 0x3C].copy_from_slice(&e.bottom_y.to_le_bytes());
                }
            }
        }
        out
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LitLightEntry {
    pub index: usize,
    pub byte_offset: usize,
    pub is_shadow_light: bool,
    pub flag: u32,
    pub no: u32,
    pub priority: i32,
    pub light_type: u32,
    pub distance: f32,
    pub pos: [f32; 4],
    pub rot_or_ang: [f32; 3],
    pub color: [f32; 4],
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LitFile {
    pub version: u32,
    pub lights: Vec<LitLightEntry>,
}

impl LitFile {
    pub fn from_bytes(data: &[u8]) -> Result<Self> {
        if data.len() < 16 || &data[0..3] != b"LIT" {
            return Err(PgError::InvalidMagic {
                expected: "LIT",
                actual: data.get(0..4).unwrap_or(&[]).to_vec(),
            });
        }
        let mut r = BinReader::new(data, Endian::Little);
        let _magic = r.read_u32()?;
        let num = r.read_u32()? as usize;
        let version = r.read_u32()?;
        let nshadow = r.read_u32()? as usize;
        let mut lights = Vec::with_capacity(num + nshadow);
        let mut o = 16usize;
        for i in 0..num {
            if o + 192 > data.len() {
                break;
            }
            r.seek(o)?;
            let flag = r.read_u32()?;
            let no = r.read_u32()?;
            let priority = r.read_i32()?;
            let _hit = r.read_u32()?;
            let pos = [r.read_f32()?, r.read_f32()?, r.read_f32()?, r.read_f32()?];
            let color = [r.read_f32()?, r.read_f32()?, r.read_f32()?, r.read_f32()?];
            let distance = r.read_f32()?;
            let light_type = r.read_u32()?;
            r.seek(o + 72)?;
            let rot_or_ang = [r.read_f32()?, r.read_f32()?, r.read_f32()?];
            lights.push(LitLightEntry {
                index: i,
                byte_offset: o,
                is_shadow_light: false,
                flag,
                no,
                priority,
                light_type,
                distance,
                pos,
                rot_or_ang,
                color,
            });
            o += 192;
        }
        for i in 0..nshadow {
            if o + 192 > data.len() {
                break;
            }
            r.seek(o)?;
            let flag = r.read_u32()?;
            let no = r.read_u32()?;
            let distance = r.read_f32()?;
            let light_type = r.read_u32()?;
            let pos = [r.read_f32()?, r.read_f32()?, r.read_f32()?, r.read_f32()?];
            let ang = [r.read_f32()?, r.read_f32()?, r.read_f32()?, r.read_f32()?];
            let color = [r.read_f32()?, r.read_f32()?, r.read_f32()?, r.read_f32()?];
            lights.push(LitLightEntry {
                index: num + i,
                byte_offset: o,
                is_shadow_light: true,
                flag,
                no,
                priority: 0,
                light_type,
                distance,
                pos,
                rot_or_ang: [ang[0], ang[1], ang[2]],
                color,
            });
            o += 192;
        }
        Ok(Self { version, lights })
    }

    pub fn apply_to_bytes(&self, original: &[u8]) -> Vec<u8> {
        let mut out = original.to_vec();
        for l in &self.lights {
            let o = l.byte_offset;
            if o + 192 > out.len() {
                continue;
            }
            out[o..o + 4].copy_from_slice(&l.flag.to_le_bytes());
            out[o + 4..o + 8].copy_from_slice(&l.no.to_le_bytes());
            if !l.is_shadow_light {
                out[o + 8..o + 12].copy_from_slice(&l.priority.to_le_bytes());
                for k in 0..4 {
                    out[o + 16 + k * 4..o + 20 + k * 4].copy_from_slice(&l.pos[k].to_le_bytes());
                    out[o + 32 + k * 4..o + 36 + k * 4].copy_from_slice(&l.color[k].to_le_bytes());
                }
                out[o + 48..o + 52].copy_from_slice(&l.distance.to_le_bytes());
                out[o + 52..o + 56].copy_from_slice(&l.light_type.to_le_bytes());
                for k in 0..3 {
                    out[o + 72 + k * 4..o + 76 + k * 4]
                        .copy_from_slice(&l.rot_or_ang[k].to_le_bytes());
                }
            } else {
                out[o + 8..o + 12].copy_from_slice(&l.distance.to_le_bytes());
                out[o + 12..o + 16].copy_from_slice(&l.light_type.to_le_bytes());
                for k in 0..4 {
                    out[o + 16 + k * 4..o + 20 + k * 4].copy_from_slice(&l.pos[k].to_le_bytes());
                    out[o + 48 + k * 4..o + 52 + k * 4].copy_from_slice(&l.color[k].to_le_bytes());
                }
                for k in 0..3 {
                    out[o + 32 + k * 4..o + 36 + k * 4]
                        .copy_from_slice(&l.rot_or_ang[k].to_le_bytes());
                }
            }
        }
        out
    }
}

// ============================================================================
// MADWORLD / BAYONETTA PROTOTYPE .MDB MODEL FORMAT (from wmb/mdb/mdb_importer.py)
// ============================================================================

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MdbMeshSummary {
    pub index: usize,
    pub byte_offset: usize,
    pub name: String,
    pub flags: u8,
    pub index_buffer_count: u32,
    pub total_triangles: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MdbFile {
    pub vertex_flags: u16,
    pub num_positions: u32,
    pub num_normals: u32,
    pub num_uvs: u32,
    pub num_colors: u32,
    pub num_bones: u32,
    pub num_materials: u32,
    pub position_divisor_bit: u16,
    pub uv_divisor_bit: u16,
    pub meshes: Vec<MdbMeshSummary>,
}

impl MdbFile {
    pub fn from_bytes(data: &[u8]) -> Result<Self> {
        if data.len() < 0x58 || &data[0..2] != b"JK" {
            return Err(PgError::InvalidMagic {
                expected: "JK (MDB)",
                actual: data.get(0..4).unwrap_or(&[]).to_vec(),
            });
        }
        let mut r = BinReader::new(data, Endian::Big);
        let _magic = r.read_u16()?;
        let vertex_flags = r.read_u16()?;
        let num_positions = r.read_u32()?;
        let _pos_off = r.read_u32()?;
        let num_normals = r.read_u32()?;
        let _nrm_off = r.read_u32()?;
        let num_uvs = r.read_u32()?;
        let _uv_off = r.read_u32()?;
        let num_colors = r.read_u32()?;
        let _col_off = r.read_u32()?;
        let _num_bp = r.read_u32()?;
        let _bp_off = r.read_u32()?;
        let num_bones = r.read_u32()?;
        let _parents_off = r.read_u32()?;
        let _bone_rel_off = r.read_u32()?;
        let _bone_pos_off = r.read_u32()?;
        let num_materials = r.read_u32()?;
        let _mat_off = r.read_u32()?;
        let _unk0 = r.read_u32()?;
        let meshes_offset = r.read_u32()? as usize;
        let position_divisor_bit = r.read_u16()?;
        let uv_divisor_bit = r.read_u16()?;
        let _unk1 = r.read_u32()?;
        let _unk2 = r.read_u32()?;
        let _unk3 = r.read_u32()?;
        let _bone_name_off = r.read_u32()?;
        let _bp_ref_off = r.read_u32()?;
        let num_meshes = r.read_u16()? as usize;

        let mut meshes = Vec::with_capacity(num_meshes);
        let mut cur = meshes_offset;
        for i in 0..num_meshes {
            if cur + 64 > data.len() {
                break;
            }
            r.seek(cur)?;
            let next_off = r.read_u32()? as usize;
            let _u0 = r.read_u8()?;
            let flags = r.read_u8()?;
            r.seek(cur + 24)?;
            let ib_count = r.read_u32()?;
            r.seek(cur + 32)?;
            let (name, hdr_size) = if (vertex_flags & 0x200) != 0 {
                (r.read_fixed_string(0x3C).unwrap_or_default(), 96usize)
            } else {
                (r.read_fixed_string(0x20).unwrap_or_default(), 64usize)
            };
            let ib_pos = (cur + hdr_size + 31) & !31;
            let mut total_tris = 0usize;
            if ib_pos < data.len() {
                r.seek(ib_pos)?;
                for _ in 0..ib_count {
                    if r.pos() + 3 > data.len() {
                        break;
                    }
                    let _ib_type = r.read_u8().unwrap_or(0);
                    let cnt = r.read_u16().unwrap_or(0) as usize;
                    total_tris += cnt / 3;
                    let mut stride = 2usize;
                    if num_normals > 0 && (flags & 0x4) == 0 {
                        stride += 2;
                    }
                    if num_colors > 0 && (flags & 0x1) == 0 {
                        stride += 2;
                    }
                    if num_uvs > 0 && (flags & 0x8) == 0 {
                        stride += 2;
                    }
                    let next_ib = r.pos() + cnt * stride;
                    if next_ib > data.len() {
                        break;
                    }
                    let _ = r.seek(next_ib);
                }
            }
            meshes.push(MdbMeshSummary {
                index: i,
                byte_offset: cur,
                name,
                flags,
                index_buffer_count: ib_count,
                total_triangles: total_tris,
            });
            if next_off == 0 {
                break;
            }
            cur += next_off;
        }

        Ok(Self {
            vertex_flags,
            num_positions,
            num_normals,
            num_uvs,
            num_colors,
            num_bones,
            num_materials,
            position_divisor_bit,
            uv_divisor_bit,
            meshes,
        })
    }

    pub fn apply_to_bytes(&self, original: &[u8]) -> Vec<u8> {
        let mut out = original.to_vec();
        let name_len = if (self.vertex_flags & 0x200) != 0 {
            0x3C
        } else {
            0x20
        };
        for m in &self.meshes {
            let cur = m.byte_offset;
            if cur + 32 + name_len > out.len() {
                continue;
            }
            out[cur + 5] = m.flags;
            let mut nbuf = vec![0u8; name_len];
            let nb = m.name.as_bytes();
            let cl = nb.len().min(name_len - 1);
            nbuf[..cl].copy_from_slice(&nb[..cl]);
            out[cur + 32..cur + 32 + name_len].copy_from_slice(&nbuf);
        }
        out
    }

    pub fn export_obj(data: &[u8]) -> Result<String> {
        let mdb = Self::from_bytes(data)?;
        let mut r = BinReader::new(data, Endian::Big);
        r.seek(8)?;
        let pos_off = r.read_u32()? as usize;
        let _num_nrm = r.read_u32()?;
        let nrm_off = r.read_u32()? as usize;
        let pos_div = (1u32 << (mdb.position_divisor_bit.min(15))) as f32;

        let mut out = String::from("# Exported by pgtools-rs (MDB)
");
        r.seek(pos_off)?;
        for _ in 0..mdb.num_positions {
            let x = (r.read_i16()? as f32) / pos_div;
            let y = (r.read_i16()? as f32) / pos_div;
            let z = (r.read_i16()? as f32) / pos_div;
            let _ = r.read_i16()?;
            out.push_str(&format!("v {x:.6} {y:.6} {z:.6}
"));
        }
        if mdb.num_normals > 0 && nrm_off < data.len() {
            r.seek(nrm_off)?;
            for _ in 0..mdb.num_normals {
                let nx = (r.read_i8().unwrap_or(0) as f32) / 64.0;
                let ny = (r.read_i8().unwrap_or(0) as f32) / 64.0;
                let nz = (r.read_i8().unwrap_or(0) as f32) / 64.0;
                out.push_str(&format!("vn {nx:.6} {ny:.6} {nz:.6}
"));
            }
        }
        let hdr_size = if (mdb.vertex_flags & 0x200) != 0 { 96 } else { 64 };
        for m in &mdb.meshes {
            out.push_str(&format!("g mesh_{}_{}
", m.index, m.name));
            let ib_pos = (m.byte_offset + hdr_size + 31) & !31;
            if ib_pos >= data.len() {
                continue;
            }
            r.seek(ib_pos)?;
            for _ in 0..m.index_buffer_count {
                if r.pos() + 3 > data.len() {
                    break;
                }
                let _ib_type = r.read_u8().unwrap_or(0);
                let cnt = r.read_u16().unwrap_or(0) as usize;
                let mut inds = Vec::with_capacity(cnt);
                for _ in 0..cnt {
                    let p_idx = r.read_u16().unwrap_or(0) as usize + 1;
                    let n_idx = if mdb.num_normals > 0 && (m.flags & 0x4) == 0 {
                        r.read_u16().unwrap_or(0) as usize + 1
                    } else {
                        1
                    };
                    if mdb.num_colors > 0 && (m.flags & 0x1) == 0 {
                        let _ = r.read_u16();
                    }
                    if mdb.num_uvs > 0 && (m.flags & 0x8) == 0 {
                        let _ = r.read_u16();
                    }
                    inds.push((p_idx, n_idx));
                }
                for tri in inds.chunks_exact(3) {
                    out.push_str(&format!(
                        "f {}//{} {}//{} {}//{}
",
                        tri[0].0, tri[0].1, tri[2].0, tri[2].1, tri[1].0, tri[1].1
                    ));
                }
            }
        }
        Ok(out)
    }
}
