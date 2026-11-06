//! `EXP` (Bayonetta 1) and `EXP2` (Bayonetta 2) skeletal expression / constraint solver,
//! layout builder, and Wii U <-> PC endian converter (`lib/bayonetta/exp.rb`).

use crate::io::{BinReader, BinWriter, Endian, PgHalf};
use crate::{PgError, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Exp1Operation {
    pub flags: u32,
    pub value: f32,
}

impl Exp1Operation {
    pub fn transform(&self, v: f32) -> Result<f32> {
        match self.flags {
            0x4 => Ok(v * self.value),
            0x20004 => Ok(v.abs() * self.value),
            0x1 => Ok(v + self.value),
            other => Err(PgError::Unsupported(format!(
                "Unknown EXP1 operation flag: 0x{other:X}"
            ))),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Exp1Entry {
    pub flags: u32,
    pub bone_index: i16,
    pub animation_track: i8,
    pub padding: i8,
    pub operations: Vec<Exp1Operation>,
}

impl Exp1Entry {
    pub fn byte_size(&self) -> usize {
        8 + self.operations.len() * 8
    }

    pub fn evaluate(&self, tracks: &[Vec<f32>], table: &BTreeMap<u16, u16>) -> Result<f32> {
        let local_idx = *table
            .get(&(self.bone_index as u16))
            .ok_or_else(|| PgError::InvalidData(format!("Bone {} missing in table", self.bone_index)))?
            as usize;
        let mut v = tracks
            .get(local_idx)
            .and_then(|t| t.get(self.animation_track as usize))
            .copied()
            .unwrap_or(0.0);
        for op in &self.operations {
            v = op.transform(v)?;
        }
        Ok(v)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExpSplinePoint {
    pub v: f32,
    pub p: f32,
    pub m0: f32,
    pub m1: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Exp1Interpolation {
    /// Type 2: Direct float points `(v, p, m0, m1)`.
    Direct(Vec<ExpSplinePoint>),
    /// Type 4: Quantized `[f32; 6]` header + `(f32 v, u16 dummy, u16 cp, u16 cm0, u16 cm1)`.
    QuantizedF32 {
        header: [f32; 6],
        keys: Vec<(f32, u16, [u16; 3])>,
    },
    /// Type 6: Quantized `[PgHalf; 6]` header + `[u8; 4]` relative `(v, cp, cm0, cm1)`.
    QuantizedPgHalf {
        header: [PgHalf; 6],
        keys: Vec<[u8; 4]>,
    },
}

impl Exp1Interpolation {
    pub fn byte_size(&self) -> usize {
        match self {
            Self::Direct(pts) => pts.len() * 16,
            Self::QuantizedF32 { keys, .. } => 24 + keys.len() * 12,
            Self::QuantizedPgHalf { keys, .. } => 12 + keys.len() * 4,
        }
    }

    pub fn to_points(&self) -> Vec<ExpSplinePoint> {
        match self {
            Self::Direct(pts) => pts.clone(),
            Self::QuantizedF32 { header, keys } => {
                let [p, dp, m0, dm0, m1, dm1] = *header;
                keys.iter()
                    .map(|&(v, _dummy, [cp, cm0, cm1])| ExpSplinePoint {
                        v,
                        p: p + (cp as f32) * dp,
                        m0: m0 + (cm0 as f32) * dm0,
                        m1: m1 + (cm1 as f32) * dm1,
                    })
                    .collect()
            }
            Self::QuantizedPgHalf { header, keys } => {
                let [p, dp, m0, dm0, m1, dm1] = header.map(|h| h.to_f32());
                let mut sum: u16 = 0;
                keys.iter()
                    .map(|&[dv, cp, cm0, cm1]| {
                        sum = sum.saturating_add(dv as u16);
                        ExpSplinePoint {
                            v: sum as f32,
                            p: p + (cp as f32) * dp,
                            m0: m0 + (cm0 as f32) * dm0,
                            m1: m1 + (cm1 as f32) * dm1,
                        }
                    })
                    .collect()
            }
        }
    }

    pub fn interpolate(&self, val: f32) -> f32 {
        let pts = self.to_points();
        if pts.is_empty() {
            return 0.0;
        }
        if val <= pts[0].v {
            return pts[0].p;
        }
        let last = pts.last().unwrap();
        if val >= last.v {
            return last.p;
        }
        for pair in pts.windows(2) {
            let l = &pair[0];
            let r = &pair[1];
            if val >= l.v && val <= r.v && (r.v - l.v).abs() > 1e-9 {
                let t = (val - l.v) / (r.v - l.v);
                let t2 = t * t;
                let t3 = t2 * t;
                return (2.0 * t3 - 3.0 * t2 + 1.0) * l.p
                    + (t3 - 2.0 * t2 + t) * l.m1
                    + (-2.0 * t3 + 3.0 * t2) * r.p
                    + (t3 - t2) * r.m0;
            }
        }
        last.p
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Exp1Record {
    pub u_a: i16,
    pub bone_index: i16,
    pub animation_track: i8,
    pub entry_type: i8,
    pub u_b: i8,
    pub interpolation_type: i8,
    pub num_points: i16,
    pub u_c: i16,
    pub offset: u32,
    pub offset_interpolation: u32,
    pub entry: Option<Exp1Entry>,
    pub interpolation: Option<Exp1Interpolation>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Exp2Operation {
    pub op_type: i8,
    pub info: i8,
    pub number: i16,
    pub value: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Exp2Record {
    pub bone_index: i16,
    pub animation_track: i8,
    pub padding: i8,
    pub num_operations: i16,
    pub unknown: i16,
    pub offset: u32,
    pub operations: Vec<Exp2Operation>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Exp2Interpolation {
    pub num_points: i16,
    pub padding: i16,
    pub offset: u32,
    pub points: Vec<ExpSplinePoint>,
}

impl Exp2Interpolation {
    pub fn evaluate(&self, val: f32) -> f32 {
        if self.points.is_empty() {
            return 0.0;
        }
        if val <= self.points[0].v {
            return self.points[0].p;
        }
        if let Some(last) = self.points.last() {
            if val >= last.v {
                return last.p;
            }
        }
        for pair in self.points.windows(2) {
            let l = &pair[0];
            let r = &pair[1];
            if val >= l.v && val <= r.v && (r.v - l.v).abs() > 1e-9 {
                let t = (val - l.v) / (r.v - l.v);
                let t2 = t * t;
                let t3 = t2 * t;
                return (2.0 * t3 - 3.0 * t2 + 1.0) * l.p
                    + (t3 - 2.0 * t2 + t) * l.m1
                    + (-2.0 * t3 + 3.0 * t2) * r.p
                    + (t3 - t2) * r.m0;
            }
        }
        0.0
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ExpFile {
    Bayo1 {
        endian: Endian,
        version: i32,
        offset_records: u32,
        records: Vec<Exp1Record>,
    },
    Bayo2 {
        endian: Endian,
        version: i32,
        offset_records: u32,
        offset_interpolations: u32,
        records: Vec<Exp2Record>,
        interpolations: Vec<Exp2Interpolation>,
    },
}

impl ExpFile {
    pub fn detect_endian(data: &[u8]) -> Result<Endian> {
        if data.len() < 16 {
            return Err(PgError::InvalidData("EXP file shorter than 16 bytes".into()));
        }
        if &data[0..4] != b"exp\0" {
            return Err(PgError::InvalidMagic {
                expected: "exp\\0",
                actual: data[0..4].to_vec(),
            });
        }
        let check = |endian: Endian| -> bool {
            let mut r = BinReader::new(data, endian);
            let _ = r.seek(8);
            let off_rec = r.read_i32().unwrap_or(-1);
            let num_rec = r.read_i32().unwrap_or(-1);
            num_rec >= 0 && off_rec > 0 && (off_rec as usize) < data.len()
        };
        match (check(Endian::Big), check(Endian::Little)) {
            (true, false) => Ok(Endian::Big),
            (false, true) => Ok(Endian::Little),
            _ => Ok(Endian::Little),
        }
    }

    pub fn from_bytes(data: &[u8]) -> Result<Self> {
        let endian = Self::detect_endian(data)?;
        let mut r = BinReader::new(data, endian);
        let _id = r.read_bytes(4)?;
        let version = r.read_i32()?;
        if version == 0x2011_0714 {
            Self::read_bayo2(data, endian)
        } else {
            Self::read_bayo1(data, endian)
        }
    }

    fn read_bayo1(data: &[u8], endian: Endian) -> Result<Self> {
        let mut r = BinReader::new(data, endian);
        let _id = r.read_bytes(4)?;
        let version = r.read_i32()?;
        let offset_records = r.read_u32()?;
        let num_records = r.read_u32()? as usize;

        r.seek(offset_records as usize)?;
        let mut records = Vec::with_capacity(num_records);
        for _ in 0..num_records {
            records.push(Exp1Record {
                u_a: r.read_i16()?,
                bone_index: r.read_i16()?,
                animation_track: r.read_i8()?,
                entry_type: r.read_i8()?,
                u_b: r.read_i8()?,
                interpolation_type: r.read_i8()?,
                num_points: r.read_i16()?,
                u_c: r.read_i16()?,
                offset: r.read_u32()?,
                offset_interpolation: r.read_u32()?,
                entry: None,
                interpolation: None,
            });
        }

        for rec in &mut records {
            if (1..=3).contains(&rec.entry_type) && rec.offset > 0 {
                r.seek(rec.offset as usize)?;
                let flags = r.read_u32()?;
                let bone_index = r.read_i16()?;
                let animation_track = r.read_i8()?;
                let padding = r.read_i8()?;
                let op_count = (rec.entry_type - 1) as usize;
                let mut operations = Vec::with_capacity(op_count);
                for _ in 0..op_count {
                    operations.push(Exp1Operation {
                        flags: r.read_u32()?,
                        value: r.read_f32()?,
                    });
                }
                rec.entry = Some(Exp1Entry {
                    flags,
                    bone_index,
                    animation_track,
                    padding,
                    operations,
                });
            }

            if rec.interpolation_type != -1 && rec.offset_interpolation > 0 {
                r.seek(rec.offset_interpolation as usize)?;
                let n = rec.num_points.max(0) as usize;
                let interp = match rec.interpolation_type {
                    2 => {
                        let mut pts = Vec::with_capacity(n);
                        for _ in 0..n {
                            pts.push(ExpSplinePoint {
                                v: r.read_f32()?,
                                p: r.read_f32()?,
                                m0: r.read_f32()?,
                                m1: r.read_f32()?,
                            });
                        }
                        Exp1Interpolation::Direct(pts)
                    }
                    4 => {
                        let header = [
                            r.read_f32()?,
                            r.read_f32()?,
                            r.read_f32()?,
                            r.read_f32()?,
                            r.read_f32()?,
                            r.read_f32()?,
                        ];
                        let mut keys = Vec::with_capacity(n);
                        for _ in 0..n {
                            let v = r.read_f32()?;
                            let dummy = r.read_u16()?;
                            keys.push((v, dummy, [r.read_u16()?, r.read_u16()?, r.read_u16()?]));
                        }
                        Exp1Interpolation::QuantizedF32 { header, keys }
                    }
                    6 => {
                        let header = [
                            PgHalf(r.read_u16()?),
                            PgHalf(r.read_u16()?),
                            PgHalf(r.read_u16()?),
                            PgHalf(r.read_u16()?),
                            PgHalf(r.read_u16()?),
                            PgHalf(r.read_u16()?),
                        ];
                        let mut keys = Vec::with_capacity(n);
                        for _ in 0..n {
                            let b = r.read_bytes(4)?;
                            keys.push([b[0], b[1], b[2], b[3]]);
                        }
                        Exp1Interpolation::QuantizedPgHalf { header, keys }
                    }
                    other => {
                        return Err(PgError::Unsupported(format!(
                            "Unknown EXP1 interpolation type: {other}"
                        )))
                    }
                };
                rec.interpolation = Some(interp);
            }
        }

        Ok(Self::Bayo1 {
            endian,
            version,
            offset_records,
            records,
        })
    }

    fn read_bayo2(data: &[u8], endian: Endian) -> Result<Self> {
        let mut r = BinReader::new(data, endian);
        let _id = r.read_bytes(4)?;
        let version = r.read_i32()?;
        let offset_records = r.read_u32()?;
        let num_records = r.read_u32()? as usize;
        let offset_interpolations = r.read_u32()?;
        let num_interpolations = r.read_u32()? as usize;

        r.seek(offset_records as usize)?;
        let mut records = Vec::with_capacity(num_records);
        for _ in 0..num_records {
            records.push(Exp2Record {
                bone_index: r.read_i16()?,
                animation_track: r.read_i8()?,
                padding: r.read_i8()?,
                num_operations: r.read_i16()?,
                unknown: r.read_i16()?,
                offset: r.read_u32()?,
                operations: Vec::new(),
            });
        }

        for (i, rec) in records.iter_mut().enumerate() {
            let op_off = (offset_records as usize) + 12 * i + (rec.offset as usize);
            r.seek(op_off)?;
            let n = rec.num_operations.max(0) as usize;
            for _ in 0..n {
                rec.operations.push(Exp2Operation {
                    op_type: r.read_i8()?,
                    info: r.read_i8()?,
                    number: r.read_i16()?,
                    value: r.read_f32()?,
                });
            }
        }

        r.seek(offset_interpolations as usize)?;
        let mut interpolations = Vec::with_capacity(num_interpolations);
        for _ in 0..num_interpolations {
            interpolations.push(Exp2Interpolation {
                num_points: r.read_i16()?,
                padding: r.read_i16()?,
                offset: r.read_u32()?,
                points: Vec::new(),
            });
        }

        for (i, interp) in interpolations.iter_mut().enumerate() {
            let pt_off =
                (offset_interpolations as usize) + 8 * i + (interp.offset as usize);
            r.seek(pt_off)?;
            let n = interp.num_points.max(0) as usize;
            for _ in 0..n {
                interp.points.push(ExpSplinePoint {
                    v: r.read_f32()?,
                    p: r.read_f32()?,
                    m0: r.read_f32()?,
                    m1: r.read_f32()?,
                });
            }
        }

        Ok(Self::Bayo2 {
            endian,
            version,
            offset_records,
            offset_interpolations,
            records,
            interpolations,
        })
    }

    pub fn endian(&self) -> Endian {
        match self {
            Self::Bayo1 { endian, .. } | Self::Bayo2 { endian, .. } => *endian,
        }
    }

    pub fn add_entries_bayo1(&mut self, add_counts: &BTreeMap<i8, usize>) -> Result<()> {
        if let Self::Bayo1 { records, .. } = self {
            for (&entry_type, &count) in add_counts {
                if !(1..=3).contains(&entry_type) {
                    return Err(PgError::InvalidData(format!(
                        "Invalid EXP1 entry type {entry_type} (must be 1..=3)"
                    )));
                }
                for _ in 0..count {
                    let op_count = (entry_type - 1) as usize;
                    let entry = Exp1Entry {
                        flags: 0x8000_0001,
                        bone_index: 0,
                        animation_track: 0,
                        padding: 0,
                        operations: vec![
                            Exp1Operation {
                                flags: 0,
                                value: 0.0
                            };
                            op_count
                        ],
                    };
                    let insert_pos = records.len().saturating_sub(1);
                    records.insert(
                        insert_pos,
                        Exp1Record {
                            u_a: 0,
                            bone_index: 0,
                            animation_track: 0,
                            entry_type,
                            u_b: 0,
                            interpolation_type: -1,
                            num_points: 0,
                            u_c: -1,
                            offset: 0,
                            offset_interpolation: 0,
                            entry: Some(entry),
                            interpolation: None,
                        },
                    );
                }
            }
        }
        Ok(())
    }

    pub fn to_bytes(&self, output_endian: Endian) -> Vec<u8> {
        let mut w = BinWriter::new(output_endian);
        match self {
            Self::Bayo1 {
                version,
                offset_records,
                records,
                ..
            } => {
                w.write_bytes(b"exp\0");
                w.write_i32(*version);
                w.write_u32(*offset_records);
                w.write_u32(records.len() as u32);

                let base_rec = *offset_records as usize;
                let mut current_offset = base_rec + records.len() * 20;

                w.seek(base_rec);
                let mut rec_offsets = Vec::with_capacity(records.len());
                for r in records {
                    let entry_off = if r.entry.is_some() {
                        let o = if r.offset > 0 {
                            r.offset as usize
                        } else {
                            let tmp = current_offset;
                            current_offset += r.entry.as_ref().unwrap().byte_size();
                            tmp
                        };
                        o as u32
                    } else {
                        0
                    };
                    let interp_off = if r.interpolation.is_some() {
                        let o = if r.offset_interpolation > 0 {
                            r.offset_interpolation as usize
                        } else {
                            let tmp = current_offset;
                            current_offset += r.interpolation.as_ref().unwrap().byte_size();
                            tmp
                        };
                        o as u32
                    } else {
                        0
                    };
                    rec_offsets.push((entry_off, interp_off));
                    w.write_i16(r.u_a);
                    w.write_i16(r.bone_index);
                    w.write_i8(r.animation_track);
                    w.write_i8(r.entry_type);
                    w.write_i8(r.u_b);
                    w.write_i8(r.interpolation_type);
                    w.write_i16(r.num_points);
                    w.write_i16(r.u_c);
                    w.write_u32(entry_off);
                    w.write_u32(interp_off);
                }

                for (i, r) in records.iter().enumerate() {
                    let (entry_off, interp_off) = rec_offsets[i];
                    if let Some(ref e) = r.entry {
                        w.seek(entry_off as usize);
                        w.write_u32(e.flags);
                        w.write_i16(e.bone_index);
                        w.write_i8(e.animation_track);
                        w.write_i8(e.padding);
                        for op in &e.operations {
                            w.write_u32(op.flags);
                            w.write_f32(op.value);
                        }
                    }
                    if let Some(ref interp) = r.interpolation {
                        w.seek(interp_off as usize);
                        match interp {
                            Exp1Interpolation::Direct(pts) => {
                                for pt in pts {
                                    w.write_f32(pt.v);
                                    w.write_f32(pt.p);
                                    w.write_f32(pt.m0);
                                    w.write_f32(pt.m1);
                                }
                            }
                            Exp1Interpolation::QuantizedF32 { header, keys } => {
                                for &h in header {
                                    w.write_f32(h);
                                }
                                for &(v, dummy, [cp, cm0, cm1]) in keys {
                                    w.write_f32(v);
                                    w.write_u16(dummy);
                                    w.write_u16(cp);
                                    w.write_u16(cm0);
                                    w.write_u16(cm1);
                                }
                            }
                            Exp1Interpolation::QuantizedPgHalf { header, keys } => {
                                for h in header {
                                    w.write_u16(h.0);
                                }
                                for k in keys {
                                    w.write_bytes(k);
                                }
                            }
                        }
                    }
                }
            }
            Self::Bayo2 {
                version,
                offset_records,
                offset_interpolations,
                records,
                interpolations,
                ..
            } => {
                w.write_bytes(b"exp\0");
                w.write_i32(*version);
                w.write_u32(*offset_records);
                w.write_u32(records.len() as u32);
                w.write_u32(*offset_interpolations);
                w.write_u32(interpolations.len() as u32);

                let base_rec = *offset_records as usize;
                w.seek(base_rec);
                for r in records {
                    w.write_i16(r.bone_index);
                    w.write_i8(r.animation_track);
                    w.write_i8(r.padding);
                    w.write_i16(r.operations.len() as i16);
                    w.write_i16(r.unknown);
                    w.write_u32(r.offset);
                }
                for (i, r) in records.iter().enumerate() {
                    let op_off = base_rec + 12 * i + (r.offset as usize);
                    w.seek(op_off);
                    for op in &r.operations {
                        w.write_i8(op.op_type);
                        w.write_i8(op.info);
                        w.write_i16(op.number);
                        w.write_f32(op.value);
                    }
                }

                let base_interp = *offset_interpolations as usize;
                w.seek(base_interp);
                for interp in interpolations {
                    w.write_i16(interp.points.len() as i16);
                    w.write_i16(interp.padding);
                    w.write_u32(interp.offset);
                }
                for (i, interp) in interpolations.iter().enumerate() {
                    let pt_off = base_interp + 8 * i + (interp.offset as usize);
                    w.seek(pt_off);
                    for pt in &interp.points {
                        w.write_f32(pt.v);
                        w.write_f32(pt.p);
                        w.write_f32(pt.m0);
                        w.write_f32(pt.m1);
                    }
                }
            }
        }
        w.into_inner()
    }
}
