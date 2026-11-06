//! `MOT` (Bayonetta 1) and `MOT2` (Bayonetta 2 / *Astral Chain* / *NieR:Automata*)
//! skeletal animation format parser, Hermite spline evaluator, bone remapper,
//! and Wii U <-> PC endian converter (`lib/bayonetta/mot.rb`).

use crate::io::{align, BinReader, BinWriter, Endian, PgHalf};
use crate::{PgError, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Evaluate cubic Hermite spline between `(start_frame, p0, m0)` and `(stop_frame, p1, m1)`.
#[inline]
pub fn hermite_interpolate(
    frame: usize,
    start_frame: usize,
    stop_frame: usize,
    p0: f32,
    p1: f32,
    m0: f32,
    m1: f32,
) -> f32 {
    if stop_frame <= start_frame {
        return p0;
    }
    let t = ((frame - start_frame) as f32) / ((stop_frame - start_frame) as f32);
    let t2 = t * t;
    let t3 = t2 * t;
    (2.0 * t3 - 3.0 * t2 + 1.0) * p0
        + (t3 - 2.0 * t2 + t) * m0
        + (-2.0 * t3 + 3.0 * t2) * p1
        + (t3 - t2) * m1
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SplineKnot {
    pub frame_index: u16,
    pub p: f32,
    pub m0: f32,
    pub m1: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum MotInterpolation {
    /// Type 1: Explicit float per keyframe (`MOT1` & `MOT2`).
    DirectValues(Vec<f32>),
    /// Type 2 (`MOT2`): Quantized linear `p + dp * cp` (`u16` keys).
    QuantizedU16 {
        p: f32,
        dp: f32,
        keys: Vec<u16>,
    },
    /// Type 3 (`MOT2`): Reduced-precision quantized linear `pghalf p, dp` + `u8` keys.
    QuantizedU8PgHalf {
        p: PgHalf,
        dp: PgHalf,
        keys: Vec<u8>,
    },
    /// Type 4 (`MOT2`): Direct Hermite spline knots (`u16 index, u16 dummy, f32 p, f32 m0, f32 m1`).
    HermiteDirect(Vec<SplineKnot>),
    /// Type 4 (`MOT1`) / Type 5 (`MOT2`): Quantized Hermite spline (`f32 p, dp, m0, dm0, m1, dm1` + `[u16; 4]` keys).
    HermiteQuantizedF32 {
        p: f32,
        dp: f32,
        m0: f32,
        dm0: f32,
        m1: f32,
        dm1: f32,
        keys: Vec<[u16; 4]>,
    },
    /// Type 6 (`MOT2`): Quantized Hermite spline (`PgHalf` header + `[u8; 4]` absolute frame keys).
    HermiteQuantizedPgHalfAbsU8 {
        header: [PgHalf; 6],
        keys: Vec<[u8; 4]>,
    },
    /// Type 6 (`MOT1`) / Type 7 (`MOT2`): Quantized Hermite spline (`PgHalf` header + `[u8; 4]` relative frame keys).
    HermiteQuantizedPgHalfRelU8 {
        header: [PgHalf; 6],
        keys: Vec<[u8; 4]>,
    },
    /// Type 7 (`MOT1`): Quantized Hermite spline (`PgHalf` header + `u16 index, u8 dummy, u8 cp, u8 cm0, u8 cm1`).
    HermiteQuantizedPgHalfMot1Type7 {
        header: [PgHalf; 6],
        keys: Vec<(u16, u8, [u8; 3])>,
    },
    /// Type 8 (`MOT2`): Quantized Hermite spline (`PgHalf` header + big-endian `u16 index, u8 cp, u8 cm0, u8 cm1`).
    HermiteQuantizedPgHalfMot2Type8 {
        header: [PgHalf; 6],
        keys: Vec<(u16, [u8; 3])>,
    },
}

impl MotInterpolation {
    pub fn to_knots(&self) -> Option<Vec<SplineKnot>> {
        match self {
            Self::HermiteDirect(knots) => Some(knots.clone()),
            Self::HermiteQuantizedF32 {
                p,
                dp,
                m0,
                dm0,
                m1,
                dm1,
                keys,
            } => Some(
                keys.iter()
                    .map(|&[idx, cp, cm0, cm1]| SplineKnot {
                        frame_index: idx,
                        p: p + (cp as f32) * dp,
                        m0: m0 + (cm0 as f32) * dm0,
                        m1: m1 + (cm1 as f32) * dm1,
                    })
                    .collect(),
            ),
            Self::HermiteQuantizedPgHalfAbsU8 { header, keys } => {
                let [p, dp, m0, dm0, m1, dm1] = header.map(|h| h.to_f32());
                Some(
                    keys.iter()
                        .map(|&[idx, cp, cm0, cm1]| SplineKnot {
                            frame_index: idx as u16,
                            p: p + (cp as f32) * dp,
                            m0: m0 + (cm0 as f32) * dm0,
                            m1: m1 + (cm1 as f32) * dm1,
                        })
                        .collect(),
                )
            }
            Self::HermiteQuantizedPgHalfRelU8 { header, keys } => {
                let [p, dp, m0, dm0, m1, dm1] = header.map(|h| h.to_f32());
                let mut accum: u16 = 0;
                Some(
                    keys.iter()
                        .map(|&[rel_idx, cp, cm0, cm1]| {
                            accum = accum.saturating_add(rel_idx as u16);
                            SplineKnot {
                                frame_index: accum,
                                p: p + (cp as f32) * dp,
                                m0: m0 + (cm0 as f32) * dm0,
                                m1: m1 + (cm1 as f32) * dm1,
                            }
                        })
                        .collect(),
                )
            }
            Self::HermiteQuantizedPgHalfMot1Type7 { header, keys } => {
                let [p, dp, m0, dm0, m1, dm1] = header.map(|h| h.to_f32());
                Some(
                    keys.iter()
                        .map(|&(idx, _dummy, [cp, cm0, cm1])| SplineKnot {
                            frame_index: idx,
                            p: p + (cp as f32) * dp,
                            m0: m0 + (cm0 as f32) * dm0,
                            m1: m1 + (cm1 as f32) * dm1,
                        })
                        .collect(),
                )
            }
            Self::HermiteQuantizedPgHalfMot2Type8 { header, keys } => {
                let [p, dp, m0, dm0, m1, dm1] = header.map(|h| h.to_f32());
                Some(
                    keys.iter()
                        .map(|&(idx, [cp, cm0, cm1])| SplineKnot {
                            frame_index: idx,
                            p: p + (cp as f32) * dp,
                            m0: m0 + (cm0 as f32) * dm0,
                            m1: m1 + (cm1 as f32) * dm1,
                        })
                        .collect(),
                )
            }
            _ => None,
        }
    }

    pub fn evaluate_frame(&self, frame_index: usize) -> f32 {
        match self {
            Self::DirectValues(keys) => keys
                .get(frame_index)
                .copied()
                .or_else(|| keys.last().copied())
                .unwrap_or(0.0),
            Self::QuantizedU16 { p, dp, keys } => {
                let cp = keys
                    .get(frame_index)
                    .copied()
                    .or_else(|| keys.last().copied())
                    .unwrap_or(0);
                p + (cp as f32) * dp
            }
            Self::QuantizedU8PgHalf { p, dp, keys } => {
                let cp = keys
                    .get(frame_index)
                    .copied()
                    .or_else(|| keys.last().copied())
                    .unwrap_or(0);
                p.to_f32() + (cp as f32) * dp.to_f32()
            }
            _ => {
                let knots = match self.to_knots() {
                    Some(k) if !k.is_empty() => k,
                    _ => return 0.0,
                };
                if frame_index <= knots[0].frame_index as usize {
                    return knots[0].p;
                }
                let last = knots.last().unwrap();
                if frame_index >= last.frame_index as usize {
                    return last.p;
                }
                for pair in knots.windows(2) {
                    let s = pair[0].frame_index as usize;
                    let e = pair[1].frame_index as usize;
                    if frame_index >= s && frame_index <= e {
                        return hermite_interpolate(
                            frame_index,
                            s,
                            e,
                            pair[0].p,
                            pair[1].p,
                            pair[0].m1,
                            pair[1].m0,
                        );
                    }
                }
                last.p
            }
        }
    }

    pub fn evaluate_all(&self, frame_count: usize) -> Vec<f32> {
        (0..frame_count).map(|f| self.evaluate_frame(f)).collect()
    }

    pub fn write(&self, w: &mut BinWriter) {
        match self {
            Self::DirectValues(keys) => {
                for &v in keys {
                    w.write_f32(v);
                }
            }
            Self::QuantizedU16 { p, dp, keys } => {
                w.write_f32(*p);
                w.write_f32(*dp);
                for &k in keys {
                    w.write_u16(k);
                }
            }
            Self::QuantizedU8PgHalf { p, dp, keys } => {
                w.write_u16(p.0);
                w.write_u16(dp.0);
                w.write_bytes(keys);
            }
            Self::HermiteDirect(knots) => {
                for k in knots {
                    w.write_u16(k.frame_index);
                    w.write_u16(0);
                    w.write_f32(k.p);
                    w.write_f32(k.m0);
                    w.write_f32(k.m1);
                }
            }
            Self::HermiteQuantizedF32 {
                p,
                dp,
                m0,
                dm0,
                m1,
                dm1,
                keys,
            } => {
                w.write_f32(*p);
                w.write_f32(*dp);
                w.write_f32(*m0);
                w.write_f32(*dm0);
                w.write_f32(*m1);
                w.write_f32(*dm1);
                for &[idx, cp, cm0, cm1] in keys {
                    w.write_u16(idx);
                    w.write_u16(cp);
                    w.write_u16(cm0);
                    w.write_u16(cm1);
                }
            }
            Self::HermiteQuantizedPgHalfAbsU8 { header, keys }
            | Self::HermiteQuantizedPgHalfRelU8 { header, keys } => {
                for h in header {
                    w.write_u16(h.0);
                }
                for k in keys {
                    w.write_bytes(k);
                }
            }
            Self::HermiteQuantizedPgHalfMot1Type7 { header, keys } => {
                for h in header {
                    w.write_u16(h.0);
                }
                for &(idx, dummy, [cp, cm0, cm1]) in keys {
                    w.write_u16(idx);
                    w.write_u8(dummy);
                    w.write_u8(cp);
                    w.write_u8(cm0);
                    w.write_u8(cm1);
                }
            }
            Self::HermiteQuantizedPgHalfMot2Type8 { header, keys } => {
                for h in header {
                    w.write_u16(h.0);
                }
                for &(idx, [cp, cm0, cm1]) in keys {
                    // Key8 stores `index_proxy` as 2 big-endian bytes (`S>`) regardless of file endianness
                    w.write_bytes(&idx.to_be_bytes());
                    w.write_u8(cp);
                    w.write_u8(cm0);
                    w.write_u8(cm1);
                }
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MotRecord {
    pub bone_index: i16,
    pub animation_track: i8,
    pub interpolation_type: i8,
    pub num_keys: i16,
    pub unknown: i16,
    /// Raw 32-bit field: if `interpolation_type == 0`, reinterpret bits as `f32`; otherwise offset.
    pub raw_offset_or_value: u32,
    pub interpolation: Option<MotInterpolation>,
}

impl MotRecord {
    #[inline]
    pub fn constant_value(&self) -> f32 {
        f32::from_bits(self.raw_offset_or_value)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DecodedMotion {
    pub flag: u16,
    pub frame_count: u16,
    /// `bone_index -> (animation_track -> values_per_frame)`
    pub tracks: BTreeMap<i16, BTreeMap<i8, Vec<f32>>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MotFile {
    pub is_bayo2: bool,
    pub endian: Endian,
    pub version: u32,
    pub flag: u16,
    pub frame_count: u16,
    pub offset_records: u32,
    pub unknown_b2: [i8; 4],
    pub name: String,
    pub records: Vec<MotRecord>,
    pub terminator: Option<MotRecord>,
}

impl MotFile {
    pub fn is_bayo2_format(data: &[u8]) -> Result<bool> {
        if data.len() < 16 {
            return Err(PgError::InvalidData("MOT file shorter than 16 bytes".into()));
        }
        if !matches!(&data[0..4], b"mot\0" | b"MOT\0" | b"\0tom" | b"\0TOM") {
            return Err(PgError::InvalidMagic {
                expected: "mot\\0",
                actual: data[0..4].to_vec(),
            });
        }
        let ver_le = u32::from_le_bytes(data[4..8].try_into().unwrap());
        let ver_be = u32::from_be_bytes(data[4..8].try_into().unwrap());
        Ok((0x2011_1109..=0x2030_1231).contains(&ver_le)
            || (0x2011_1109..=0x2030_1231).contains(&ver_be))
    }

    pub fn from_bytes(data: &[u8]) -> Result<Self> {
        if Self::is_bayo2_format(data)? {
            Self::read_bayo2(data)
        } else {
            Self::read_bayo1(data)
        }
    }

    fn read_bayo2(data: &[u8]) -> Result<Self> {
        let ver_be = u32::from_be_bytes(data[4..8].try_into().unwrap());
        let endian = if (0x2011_1109..=0x2030_1231).contains(&ver_be) {
            Endian::Big
        } else {
            Endian::Little
        };
        let mut r = BinReader::new(data, endian);
        let _id = r.read_bytes(4)?;
        let version = r.read_u32()?;
        let flag = r.read_u16()?;
        let frame_count = r.read_u16()?;
        let offset_records = r.read_u32()?;
        let num_records = r.read_u32()? as usize;
        let unknown_b2 = [r.read_i8()?, r.read_i8()?, r.read_i8()?, r.read_i8()?];
        let name_len = (offset_records as usize).saturating_sub(0x18).clamp(16, 64);
        let name = r.read_fixed_string(name_len)?;

        r.seek(offset_records as usize)?;
        let mut records = Vec::with_capacity(num_records);
        for _ in 0..num_records {
            records.push(MotRecord {
                bone_index: r.read_i16()?,
                animation_track: r.read_i8()?,
                interpolation_type: r.read_i8()?,
                num_keys: r.read_i16()?,
                unknown: r.read_i16()?,
                raw_offset_or_value: r.read_u32()?,
                interpolation: None,
            });
        }

        let terminator = if (offset_records as usize) + 12 * (num_records + 1) <= data.len() {
            r.seek((offset_records as usize) + 12 * num_records)?;
            Some(MotRecord {
                bone_index: r.read_i16()?,
                animation_track: r.read_i8()?,
                interpolation_type: r.read_i8()?,
                num_keys: r.read_i16()?,
                unknown: r.read_i16()?,
                raw_offset_or_value: r.read_u32()?,
                interpolation: None,
            })
        } else {
            None
        };

        for (i, rec) in records.iter_mut().enumerate() {
            if rec.interpolation_type <= 0 {
                continue;
            }
            let interp_offset =
                (offset_records as usize) + 12 * i + (rec.raw_offset_or_value as usize);
            r.seek(interp_offset)?;
            let n = rec.num_keys.max(0) as usize;
            let interp = match rec.interpolation_type {
                1 => {
                    let mut keys = Vec::with_capacity(n);
                    for _ in 0..n {
                        keys.push(r.read_f32()?);
                    }
                    MotInterpolation::DirectValues(keys)
                }
                2 => {
                    let p = r.read_f32()?;
                    let dp = r.read_f32()?;
                    let mut keys = Vec::with_capacity(n);
                    for _ in 0..n {
                        keys.push(r.read_u16()?);
                    }
                    MotInterpolation::QuantizedU16 { p, dp, keys }
                }
                3 => {
                    let p = PgHalf(r.read_u16()?);
                    let dp = PgHalf(r.read_u16()?);
                    let keys = r.read_bytes(n)?.to_vec();
                    MotInterpolation::QuantizedU8PgHalf { p, dp, keys }
                }
                4 => {
                    let mut knots = Vec::with_capacity(n);
                    for _ in 0..n {
                        let frame_index = r.read_u16()?;
                        let _dummy = r.read_u16()?;
                        knots.push(SplineKnot {
                            frame_index,
                            p: r.read_f32()?,
                            m0: r.read_f32()?,
                            m1: r.read_f32()?,
                        });
                    }
                    MotInterpolation::HermiteDirect(knots)
                }
                5 => {
                    let p = r.read_f32()?;
                    let dp = r.read_f32()?;
                    let m0 = r.read_f32()?;
                    let dm0 = r.read_f32()?;
                    let m1 = r.read_f32()?;
                    let dm1 = r.read_f32()?;
                    let mut keys = Vec::with_capacity(n);
                    for _ in 0..n {
                        keys.push([r.read_u16()?, r.read_u16()?, r.read_u16()?, r.read_u16()?]);
                    }
                    MotInterpolation::HermiteQuantizedF32 {
                        p,
                        dp,
                        m0,
                        dm0,
                        m1,
                        dm1,
                        keys,
                    }
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
                    MotInterpolation::HermiteQuantizedPgHalfAbsU8 { header, keys }
                }
                7 => {
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
                    MotInterpolation::HermiteQuantizedPgHalfRelU8 { header, keys }
                }
                8 => {
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
                        let b = r.read_bytes(5)?;
                        let idx = u16::from_be_bytes([b[0], b[1]]);
                        keys.push((idx, [b[2], b[3], b[4]]));
                    }
                    MotInterpolation::HermiteQuantizedPgHalfMot2Type8 { header, keys }
                }
                other => {
                    return Err(PgError::Unsupported(format!(
                        "Unknown MOT2 interpolation type: {other}"
                    )))
                }
            };
            rec.interpolation = Some(interp);
        }

        Ok(Self {
            is_bayo2: true,
            endian,
            version,
            flag,
            frame_count,
            offset_records,
            unknown_b2,
            name,
            records,
            terminator,
        })
    }

    fn read_bayo1(data: &[u8]) -> Result<Self> {
        let check = |endian: Endian| -> bool {
            let mut r = BinReader::new(data, endian);
            let _ = r.seek(8);
            let off_rec = r.read_i32().unwrap_or(-1);
            let num_rec = r.read_i32().unwrap_or(-1);
            num_rec >= 0
                && (num_rec as usize) * 12 < data.len()
                && off_rec > 0
                && (off_rec as usize) < data.len()
        };
        let endian = match (check(Endian::Big), check(Endian::Little)) {
            (true, false) => Endian::Big,
            (false, true) => Endian::Little,
            _ => Endian::Little,
        };

        let mut r = BinReader::new(data, endian);
        let _id = r.read_bytes(4)?;
        let flag = r.read_u16()?;
        let frame_count = r.read_u16()?;
        let offset_records = r.read_u32()?;
        let num_records = r.read_u32()? as usize;

        r.seek(offset_records as usize)?;
        let mut records = Vec::with_capacity(num_records);
        for _ in 0..num_records {
            records.push(MotRecord {
                bone_index: r.read_i16()?,
                animation_track: r.read_i8()?,
                interpolation_type: r.read_i8()?,
                num_keys: r.read_i16()?,
                unknown: r.read_i16()?,
                raw_offset_or_value: r.read_u32()?,
                interpolation: None,
            });
        }

        for rec in &mut records {
            if rec.interpolation_type <= 0 {
                continue;
            }
            let interp_offset = rec.raw_offset_or_value as usize;
            r.seek(interp_offset)?;
            let n = rec.num_keys.max(0) as usize;
            let interp = match rec.interpolation_type {
                1 => {
                    let mut keys = Vec::with_capacity(n);
                    for _ in 0..n {
                        keys.push(r.read_f32()?);
                    }
                    MotInterpolation::DirectValues(keys)
                }
                4 => {
                    let p = r.read_f32()?;
                    let dp = r.read_f32()?;
                    let m0 = r.read_f32()?;
                    let dm0 = r.read_f32()?;
                    let m1 = r.read_f32()?;
                    let dm1 = r.read_f32()?;
                    let mut keys = Vec::with_capacity(n);
                    for _ in 0..n {
                        keys.push([r.read_u16()?, r.read_u16()?, r.read_u16()?, r.read_u16()?]);
                    }
                    MotInterpolation::HermiteQuantizedF32 {
                        p,
                        dp,
                        m0,
                        dm0,
                        m1,
                        dm1,
                        keys,
                    }
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
                    MotInterpolation::HermiteQuantizedPgHalfRelU8 { header, keys }
                }
                7 => {
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
                        let idx = r.read_u16()?;
                        let dummy = r.read_u8()?;
                        let b = r.read_bytes(3)?;
                        keys.push((idx, dummy, [b[0], b[1], b[2]]));
                    }
                    MotInterpolation::HermiteQuantizedPgHalfMot1Type7 { header, keys }
                }
                other => {
                    return Err(PgError::Unsupported(format!(
                        "Unknown MOT1 interpolation type: {other}"
                    )))
                }
            };
            rec.interpolation = Some(interp);
        }

        Ok(Self {
            is_bayo2: false,
            endian,
            version: 0,
            flag,
            frame_count,
            offset_records,
            unknown_b2: [0; 4],
            name: String::new(),
            records,
            terminator: None,
        })
    }

    pub fn remap_bones(&mut self, map: &BTreeMap<i16, i16>) {
        for r in &mut self.records {
            if let Some(&new_bone) = map.get(&r.bone_index) {
                r.bone_index = new_bone;
            }
        }
    }

    pub fn decode_frame(&self, frame_index: usize) -> Result<BTreeMap<i16, BTreeMap<i8, f32>>> {
        if frame_index >= self.frame_count as usize {
            return Err(PgError::InvalidData(format!(
                "Invalid frame index {frame_index} (0..{})",
                self.frame_count
            )));
        }
        let mut tracks: BTreeMap<i16, BTreeMap<i8, f32>> = BTreeMap::new();
        for r in &self.records {
            if r.interpolation_type != -1 {
                let val = if r.interpolation_type == 0 {
                    r.constant_value()
                } else if let Some(ref interp) = r.interpolation {
                    interp.evaluate_frame(frame_index)
                } else {
                    0.0
                };
                tracks.entry(r.bone_index).or_default().insert(r.animation_track, val);
            }
        }
        Ok(tracks)
    }

    pub fn decode(&self) -> DecodedMotion {
        let fc = self.frame_count as usize;
        let mut tracks: BTreeMap<i16, BTreeMap<i8, Vec<f32>>> = BTreeMap::new();
        for r in &self.records {
            if r.interpolation_type != -1 {
                let vals = if r.interpolation_type == 0 {
                    vec![r.constant_value(); fc]
                } else if let Some(ref interp) = r.interpolation {
                    interp.evaluate_all(fc)
                } else {
                    vec![0.0; fc]
                };
                tracks
                    .entry(r.bone_index)
                    .or_default()
                    .insert(r.animation_track, vals);
            }
        }
        DecodedMotion {
            flag: self.flag,
            frame_count: self.frame_count,
            tracks,
        }
    }

    pub fn to_bytes(&self, output_endian: Endian) -> Vec<u8> {
        let mut w = BinWriter::new(output_endian);
        w.write_bytes(b"mot\0");
        let base_rec = if self.is_bayo2 {
            (self.offset_records as usize).max(0x2C)
        } else {
            (self.offset_records as usize).max(0x10)
        };
        if self.is_bayo2 {
            let ver = if self.version != 0 { self.version } else { 0x2012_0405 };
            w.write_u32(ver);
            w.write_u16(self.flag);
            w.write_u16(self.frame_count);
            w.write_u32(base_rec as u32);
            w.write_u32(self.records.len() as u32);
            for &b in &self.unknown_b2 {
                w.write_i8(b);
            }
            let name_len = base_rec.saturating_sub(0x18);
            w.write_fixed_string(&self.name, name_len);
        } else {
            w.write_u16(self.flag);
            w.write_u16(self.frame_count);
            w.write_u32(base_rec as u32);
            w.write_u32(self.records.len() as u32);
        }

        let term_count = if self.is_bayo2 || self.terminator.is_some() { 1 } else { 0 };
        let payload_start = align(base_rec + 12 * (self.records.len() + term_count), 4);
        w.seek(payload_start);

        let mut computed_offsets: Vec<u32> = Vec::with_capacity(self.records.len());
        for (i, r) in self.records.iter().enumerate() {
            if r.interpolation_type > 0 {
                if let Some(ref interp) = r.interpolation {
                    let aligned_pos = align(w.len(), 4);
                    w.seek(aligned_pos);
                    let rec_pos = base_rec + 12 * i;
                    let off_val = if self.is_bayo2 {
                        (aligned_pos - rec_pos) as u32
                    } else {
                        aligned_pos as u32
                    };
                    computed_offsets.push(off_val);
                    interp.write(&mut w);
                    continue;
                }
            }
            computed_offsets.push(r.raw_offset_or_value);
        }

        let end_pos = align(w.len(), 4);
        w.seek(base_rec);
        for (i, r) in self.records.iter().enumerate() {
            w.write_i16(r.bone_index);
            w.write_i8(r.animation_track);
            w.write_i8(r.interpolation_type);
            w.write_i16(r.num_keys);
            w.write_i16(r.unknown);
            w.write_u32(computed_offsets[i]);
        }
        if let Some(ref term) = self.terminator {
            w.write_i16(term.bone_index);
            w.write_i8(term.animation_track);
            w.write_i8(term.interpolation_type);
            w.write_i16(term.num_keys);
            w.write_i16(term.unknown);
            w.write_u32(term.raw_offset_or_value);
        } else if self.is_bayo2 {
            w.write_i16(0x7FFF);
            w.write_i8(0);
            w.write_i8(0);
            w.write_i16(0);
            w.write_i16(0);
            w.write_u32(0);
        }

        w.seek(end_pos);
        w.into_inner()
    }
}
