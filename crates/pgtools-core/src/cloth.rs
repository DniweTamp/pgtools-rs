//! Cloth physics & collision parameter formats: `CLP` (Cloth parameters), `CLH` (Cloth capsule collisions),
//! and `CLW` (Cloth wind parameters), including Bayonetta 2 `_clp.bxm` / `_clh.bxm` / `_clw.bxm` conversion,
//! bone remapping, and Wii U <-> PC endian swapping (`lib/bayonetta/clp.rb`, `clh.rb`, `clw.rb`).

use crate::bone::Vec3;
use crate::bxm::{BxmFile, BxmXmlNode};
use crate::io::{BinReader, BinWriter, Endian};
use crate::{PgError, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

fn parse_bxm_vec3(s: &str) -> Vec3 {
    let mut parts = s
        .split_whitespace()
        .filter_map(|p| p.parse::<f32>().ok());
    Vec3 {
        x: parts.next().unwrap_or(0.0),
        y: parts.next().unwrap_or(0.0),
        z: parts.next().unwrap_or(0.0),
    }
}

fn find_child_text<'a>(node: &'a BxmXmlNode, tag: &str) -> &'a str {
    node.children
        .iter()
        .find(|c| c.name == tag)
        .and_then(|c| c.value.as_deref())
        .unwrap_or("0")
}

fn find_descendants<'a>(node: &'a BxmXmlNode, tag: &str, out: &mut Vec<&'a BxmXmlNode>) {
    if node.name == tag {
        out.push(node);
    }
    for c in &node.children {
        find_descendants(c, tag, out);
    }
}

fn detect_cloth_endian(data: &[u8]) -> Result<Endian> {
    if data.len() < 4 {
        return Err(PgError::InvalidData("Cloth file shorter than 4 bytes".into()));
    }
    let big_num = i32::from_be_bytes(data[0..4].try_into().unwrap());
    let lit_num = i32::from_le_bytes(data[0..4].try_into().unwrap());
    let big_ok = (0..256).contains(&big_num);
    let lit_ok = (0..256).contains(&lit_num);
    match (big_ok, lit_ok) {
        (true, false) => Ok(Endian::Big),
        (false, true) => Ok(Endian::Little),
        (true, true) => Ok(Endian::Little),
        _ => Err(PgError::InvalidData("Invalid entry count in cloth header".into())),
    }
}

fn remap_bone_id(map: &BTreeMap<i32, i32>, no: i16) -> Result<i16> {
    if no == 4095 || no == -1 {
        return Ok(no);
    }
    map.get(&(no as i32))
        .map(|&v| v as i16)
        .ok_or_else(|| PgError::InvalidData(format!("No bone found in remap table for {no}")))
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClpHeader {
    pub num: i32,
    pub limit_spring_rate: f32,
    pub spd_rate: f32,
    pub stretchy: f32,
    pub bundle_num: i16,
    pub bundle_num_2: i16,
    pub thick: f32,
    pub gravity_vec: Vec3,
    pub gravity_parts_no: i32,
    pub first_bundle_rate: f32,
    pub wind_vec: Vec3,
    pub wind_parts_no: i32,
    pub wind_offset: Vec3,
    pub wind_sin: f32,
    pub hit_adjust_rate: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClpClothEntry {
    pub no: i16,
    pub no_up: i16,
    pub no_down: i16,
    pub no_side: i16,
    pub no_poly: i16,
    pub no_fix: i16,
    pub rot_limit: f32,
    pub offset: Vec3,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClpFile {
    pub endian: Endian,
    pub header: ClpHeader,
    pub cloth: Vec<ClpClothEntry>,
}

impl ClpFile {
    pub fn from_bytes(data: &[u8]) -> Result<Self> {
        let endian = detect_cloth_endian(data)?;
        let mut r = BinReader::new(data, endian);
        let num = r.read_i32()?;
        let header = ClpHeader {
            num,
            limit_spring_rate: r.read_f32()?,
            spd_rate: r.read_f32()?,
            stretchy: r.read_f32()?,
            bundle_num: r.read_i16()?,
            bundle_num_2: r.read_i16()?,
            thick: r.read_f32()?,
            gravity_vec: Vec3::read(&mut r)?,
            gravity_parts_no: r.read_i32()?,
            first_bundle_rate: r.read_f32()?,
            wind_vec: Vec3::read(&mut r)?,
            wind_parts_no: r.read_i32()?,
            wind_offset: Vec3::read(&mut r)?,
            wind_sin: r.read_f32()?,
            hit_adjust_rate: r.read_f32()?,
        };
        let mut cloth = Vec::with_capacity(num.max(0) as usize);
        for _ in 0..(num.max(0) as usize) {
            cloth.push(ClpClothEntry {
                no: r.read_i16()?,
                no_up: r.read_i16()?,
                no_down: r.read_i16()?,
                no_side: r.read_i16()?,
                no_poly: r.read_i16()?,
                no_fix: r.read_i16()?,
                rot_limit: r.read_f32()?,
                offset: Vec3::read(&mut r)?,
            });
        }
        Ok(Self {
            endian,
            header,
            cloth,
        })
    }

    pub fn from_bxm(bxm_bytes: &[u8]) -> Result<Self> {
        let bxm = BxmFile::from_bytes(bxm_bytes)?;
        let root = bxm
            .root
            .as_ref()
            .ok_or_else(|| PgError::InvalidData("Empty BXM document".into()))?;
        let mut headers = Vec::new();
        find_descendants(root, "CLOTH_HEADER", &mut headers);
        let h = headers
            .first()
            .ok_or_else(|| PgError::InvalidData("Missing CLOTH_HEADER in _clp.bxm".into()))?;

        let mut wks = Vec::new();
        find_descendants(root, "CLOTH_WK", &mut wks);

        let header = ClpHeader {
            num: find_child_text(h, "m_Num").parse().unwrap_or(wks.len() as i32),
            limit_spring_rate: find_child_text(h, "m_LimitSpringRate").parse().unwrap_or(0.0),
            spd_rate: find_child_text(h, "m_SpdRate").parse().unwrap_or(0.0),
            stretchy: find_child_text(h, "m_Stretchy").parse().unwrap_or(0.0),
            bundle_num: find_child_text(h, "m_BundleNum").parse().unwrap_or(0),
            bundle_num_2: find_child_text(h, "m_BundleNum2").parse().unwrap_or(0),
            thick: find_child_text(h, "m_Thick").parse().unwrap_or(0.0),
            gravity_vec: parse_bxm_vec3(find_child_text(h, "m_GravityVec")),
            gravity_parts_no: find_child_text(h, "m_GravityPartsNo").parse().unwrap_or(0),
            first_bundle_rate: find_child_text(h, "m_FirstBundleRate").parse().unwrap_or(0.0),
            wind_vec: parse_bxm_vec3(find_child_text(h, "m_WindVec")),
            wind_parts_no: find_child_text(h, "m_WindPartsNo").parse().unwrap_or(0),
            wind_offset: parse_bxm_vec3(find_child_text(h, "m_WindOffset")),
            wind_sin: find_child_text(h, "m_WindSin").parse().unwrap_or(0.0),
            hit_adjust_rate: find_child_text(h, "m_HitAdjustRate").parse().unwrap_or(0.0),
        };

        let cloth = wks
            .into_iter()
            .map(|c| ClpClothEntry {
                no: find_child_text(c, "no").parse().unwrap_or(4095),
                no_up: find_child_text(c, "noUp").parse().unwrap_or(4095),
                no_down: find_child_text(c, "noDown").parse().unwrap_or(4095),
                no_side: find_child_text(c, "noSide").parse().unwrap_or(4095),
                no_poly: find_child_text(c, "noPoly").parse().unwrap_or(4095),
                no_fix: find_child_text(c, "noFix").parse().unwrap_or(4095),
                rot_limit: find_child_text(c, "rotLimit").parse().unwrap_or(0.0),
                offset: parse_bxm_vec3(find_child_text(c, "offset")),
            })
            .collect();

        Ok(Self {
            endian: Endian::Little,
            header,
            cloth,
        })
    }

    pub fn remap(&mut self, map: &BTreeMap<i32, i32>, poly: bool, fix: bool) -> Result<()> {
        if self.header.gravity_parts_no != -1 && self.header.gravity_parts_no != 4095 {
            if let Some(&mapped) = map.get(&self.header.gravity_parts_no) {
                self.header.gravity_parts_no = mapped;
            }
        }
        if self.header.wind_parts_no != -1 && self.header.wind_parts_no != 4095 {
            if let Some(&mapped) = map.get(&self.header.wind_parts_no) {
                self.header.wind_parts_no = mapped;
            }
        }
        for c in &mut self.cloth {
            c.no = remap_bone_id(map, c.no)?;
            c.no_up = remap_bone_id(map, c.no_up)?;
            c.no_down = remap_bone_id(map, c.no_down)?;
            c.no_side = remap_bone_id(map, c.no_side)?;
            c.no_poly = if poly { remap_bone_id(map, c.no_poly)? } else { 4095 };
            c.no_fix = if fix { remap_bone_id(map, c.no_fix)? } else { 4095 };
        }
        Ok(())
    }

    pub fn to_bytes(&self, endian: Endian) -> Vec<u8> {
        let mut w = BinWriter::new(endian);
        w.write_i32(self.cloth.len() as i32);
        w.write_f32(self.header.limit_spring_rate);
        w.write_f32(self.header.spd_rate);
        w.write_f32(self.header.stretchy);
        w.write_i16(self.header.bundle_num);
        w.write_i16(self.header.bundle_num_2);
        w.write_f32(self.header.thick);
        self.header.gravity_vec.write(&mut w);
        w.write_i32(self.header.gravity_parts_no);
        w.write_f32(self.header.first_bundle_rate);
        self.header.wind_vec.write(&mut w);
        w.write_i32(self.header.wind_parts_no);
        self.header.wind_offset.write(&mut w);
        w.write_f32(self.header.wind_sin);
        w.write_f32(self.header.hit_adjust_rate);

        for c in &self.cloth {
            w.write_i16(c.no);
            w.write_i16(c.no_up);
            w.write_i16(c.no_down);
            w.write_i16(c.no_side);
            w.write_i16(c.no_poly);
            w.write_i16(c.no_fix);
            w.write_f32(c.rot_limit);
            c.offset.write(&mut w);
        }
        w.into_inner()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClhEntry {
    #[serde(alias = "no1")]
    pub p1: i16,
    #[serde(alias = "no2")]
    pub p2: i16,
    pub weight: f32,
    #[serde(alias = "r")]
    pub radius: f32,
    pub offset1: Vec3,
    pub offset2: Vec3,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClhFile {
    pub endian: Endian,
    #[serde(alias = "cloth_wk")]
    pub entries: Vec<ClhEntry>,
}

impl ClhFile {
    pub fn from_bytes(data: &[u8]) -> Result<Self> {
        let endian = detect_cloth_endian(data)?;
        let mut r = BinReader::new(data, endian);
        let count = r.read_i32()?.max(0) as usize;
        let mut entries = Vec::with_capacity(count);
        for _ in 0..count {
            entries.push(ClhEntry {
                p1: r.read_i16()?,
                p2: r.read_i16()?,
                weight: r.read_f32()?,
                radius: r.read_f32()?,
                offset1: Vec3::read(&mut r)?,
                offset2: Vec3::read(&mut r)?,
            });
        }
        Ok(Self { endian, entries })
    }

    pub fn from_bxm(bxm_bytes: &[u8]) -> Result<Self> {
        let bxm = BxmFile::from_bytes(bxm_bytes)?;
        let root = bxm
            .root
            .as_ref()
            .ok_or_else(|| PgError::InvalidData("Empty BXM document".into()))?;
        let mut wks = Vec::new();
        find_descendants(root, "CLOTH_AT_WK", &mut wks);

        let entries = wks
            .into_iter()
            .map(|c| ClhEntry {
                p1: find_child_text(c, "p1").parse().unwrap_or(4095),
                p2: find_child_text(c, "p2").parse().unwrap_or(4095),
                weight: find_child_text(c, "weight").parse().unwrap_or(0.0),
                radius: find_child_text(c, "radius").parse().unwrap_or(0.0),
                offset1: parse_bxm_vec3(find_child_text(c, "offset1")),
                offset2: parse_bxm_vec3(find_child_text(c, "offset2")),
            })
            .collect();

        Ok(Self {
            endian: Endian::Little,
            entries,
        })
    }

    pub fn remap(&mut self, map: &BTreeMap<i32, i32>) -> Result<()> {
        for e in &mut self.entries {
            e.p1 = remap_bone_id(map, e.p1)?;
            e.p2 = remap_bone_id(map, e.p2)?;
        }
        Ok(())
    }

    pub fn to_bytes(&self, endian: Endian) -> Vec<u8> {
        let mut w = BinWriter::new(endian);
        w.write_i32(self.entries.len() as i32);
        for e in &self.entries {
            w.write_i16(e.p1);
            w.write_i16(e.p2);
            w.write_f32(e.weight);
            w.write_f32(e.radius);
            e.offset1.write(&mut w);
            e.offset2.write(&mut w);
        }
        w.into_inner()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClwEntry {
    #[serde(alias = "ent_type")]
    pub wind_type: i32,
    #[serde(alias = "no")]
    pub parts_no: i32,
    pub offset: Vec3,
    pub offset2: Vec3,
    pub radius: f32,
    pub power: f32,
    pub timer: f32,
    pub swing_rate: f32,
    pub swing_spd: f32,
    pub delay_max: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClwFile {
    pub endian: Endian,
    #[serde(alias = "cloth_wind_wk")]
    pub entries: Vec<ClwEntry>,
}

impl ClwFile {
    pub fn from_bytes(data: &[u8]) -> Result<Self> {
        let endian = detect_cloth_endian(data)?;
        let mut r = BinReader::new(data, endian);
        let count = r.read_i32()?.max(0) as usize;
        let mut entries = Vec::with_capacity(count);
        for _ in 0..count {
            entries.push(ClwEntry {
                wind_type: r.read_i32()?,
                parts_no: r.read_i32()?,
                offset: Vec3::read(&mut r)?,
                offset2: Vec3::read(&mut r)?,
                radius: r.read_f32()?,
                power: r.read_f32()?,
                timer: r.read_f32()?,
                swing_rate: r.read_f32()?,
                swing_spd: r.read_f32()?,
                delay_max: r.read_f32()?,
            });
        }
        Ok(Self { endian, entries })
    }

    pub fn from_bxm(bxm_bytes: &[u8]) -> Result<Self> {
        let bxm = BxmFile::from_bytes(bxm_bytes)?;
        let root = bxm
            .root
            .as_ref()
            .ok_or_else(|| PgError::InvalidData("Empty BXM document".into()))?;
        let mut wks = Vec::new();
        find_descendants(root, "CLOTH_WIND_WK", &mut wks);

        let entries = wks
            .into_iter()
            .map(|c| ClwEntry {
                wind_type: find_child_text(c, "WindType").parse().unwrap_or(0),
                parts_no: find_child_text(c, "PartsNo").parse().unwrap_or(-1),
                offset: parse_bxm_vec3(find_child_text(c, "Offset")),
                offset2: parse_bxm_vec3(find_child_text(c, "Offset2")),
                radius: find_child_text(c, "Radius").parse().unwrap_or(0.0),
                power: find_child_text(c, "Power").parse().unwrap_or(0.0),
                timer: find_child_text(c, "Timer").parse().unwrap_or(0.0),
                swing_rate: find_child_text(c, "SwingRate").parse().unwrap_or(0.0),
                swing_spd: find_child_text(c, "SwingSpd").parse().unwrap_or(0.0),
                delay_max: find_child_text(c, "DelayMax").parse().unwrap_or(0.0),
            })
            .collect();

        Ok(Self {
            endian: Endian::Little,
            entries,
        })
    }

    pub fn remap(&mut self, map: &BTreeMap<i32, i32>) -> Result<()> {
        for e in &mut self.entries {
            if e.parts_no != -1 && e.parts_no != 4095 {
                e.parts_no = *map.get(&e.parts_no).ok_or_else(|| {
                    PgError::InvalidData(format!("No bone found in remap table for {}", e.parts_no))
                })?;
            }
        }
        Ok(())
    }

    pub fn to_bytes(&self, endian: Endian) -> Vec<u8> {
        let mut w = BinWriter::new(endian);
        w.write_i32(self.entries.len() as i32);
        for e in &self.entries {
            w.write_i32(e.wind_type);
            w.write_i32(e.parts_no);
            e.offset.write(&mut w);
            e.offset2.write(&mut w);
            w.write_f32(e.radius);
            w.write_f32(e.power);
            w.write_f32(e.timer);
            w.write_f32(e.swing_rate);
            w.write_f32(e.swing_spd);
            w.write_f32(e.delay_max);
        }
        w.into_inner()
    }
}
