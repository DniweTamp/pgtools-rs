//! Skeletal bone representations and the 3-level `BoneIndexTranslateTable` trie
//! used across WMB1, WMB2, and WMB3 models (`lib/bayonetta/bone.rb` & `wmb.rb`).

use crate::io::{BinReader, BinWriter};
use crate::Result;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct Vec3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

impl Vec3 {
    pub const ZERO: Self = Self { x: 0.0, y: 0.0, z: 0.0 };
    pub const ONE: Self = Self { x: 1.0, y: 1.0, z: 1.0 };

    #[inline]
    pub const fn new(x: f32, y: f32, z: f32) -> Self {
        Self { x, y, z }
    }

    pub fn read(r: &mut BinReader) -> Result<Self> {
        Ok(Self {
            x: r.read_f32()?,
            y: r.read_f32()?,
            z: r.read_f32()?,
        })
    }

    pub fn write(&self, w: &mut BinWriter) {
        w.write_f32(self.x);
        w.write_f32(self.y);
        w.write_f32(self.z);
    }

    pub fn to_array(self) -> [f32; 3] {
        [self.x, self.y, self.z]
    }

    pub fn sub(self, other: Self) -> Self {
        Self::new(self.x - other.x, self.y - other.y, self.z - other.z)
    }

    pub fn add(self, other: Self) -> Self {
        Self::new(self.x + other.x, self.y + other.y, self.z + other.z)
    }

    pub fn scale(self, s: f32) -> Self {
        Self::new(self.x * s, self.y * s, self.z * s)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BoneNode {
    pub index: usize,
    pub parent_index: i16,
    pub position: Vec3,
    pub relative_position: Vec3,
    pub global_index: i16,
    pub symmetric: i16,
    pub flag: i8,
}

impl BoneNode {
    pub fn depth(&self, all_bones: &[BoneNode]) -> usize {
        let mut d = 0;
        let mut curr = self.parent_index;
        while curr >= 0 && (curr as usize) < all_bones.len() && d < all_bones.len() {
            d += 1;
            curr = all_bones[curr as usize].parent_index;
        }
        d
    }

    pub fn ancestors(&self, all_bones: &[BoneNode]) -> Vec<usize> {
        let mut res = Vec::new();
        let mut curr = self.parent_index;
        while curr >= 0 && (curr as usize) < all_bones.len() && res.len() < all_bones.len() {
            res.push(curr as usize);
            curr = all_bones[curr as usize].parent_index;
        }
        res
    }

    pub fn distance(&self, other: &BoneNode, all_bones_self: &[BoneNode], all_bones_other: &[BoneNode]) -> (f32, usize) {
        let dx = self.position.x - other.position.x;
        let dy = self.position.y - other.position.y;
        let dz = self.position.z - other.position.z;
        let dist = (dx * dx + dy * dy + dz * dz).sqrt();
        let d_self = self.depth(all_bones_self);
        let d_other = other.depth(all_bones_other);
        (dist, d_self.abs_diff(d_other))
    }
}

/// 3-level 16-way radix trie mapping global 12-bit bone IDs (`0x000..=0xFFF`)
/// to local model bone indices (`BoneIndexTranslateTable` in `wmb.rb`).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct BoneIndexTranslateTable {
    /// Global ID -> Local Bone Index
    pub table: BTreeMap<u16, u16>,
}

impl BoneIndexTranslateTable {
    pub fn new() -> Self {
        Self {
            table: BTreeMap::new(),
        }
    }

    pub fn read(r: &mut BinReader) -> Result<Self> {
        let mut first_level = [0i16; 16];
        for slot in &mut first_level {
            *slot = r.read_i16()?;
        }

        let mut second_levels: Vec<[i16; 16]> = Vec::new();
        for &o in &first_level {
            if o != -1 {
                let mut block = [0i16; 16];
                for slot in &mut block {
                    *slot = r.read_i16()?;
                }
                second_levels.push(block);
            }
        }

        let mut third_levels: Vec<[i16; 16]> = Vec::new();
        for l2 in &second_levels {
            for &o in l2 {
                if o != -1 {
                    let mut block = [0i16; 16];
                    for slot in &mut block {
                        *slot = r.read_i16()?;
                    }
                    third_levels.push(block);
                }
            }
        }

        let mut flat: Vec<i16> = Vec::with_capacity(
            16 + second_levels.len() * 16 + third_levels.len() * 16,
        );
        flat.extend_from_slice(&first_level);
        for l2 in &second_levels {
            flat.extend_from_slice(l2);
        }
        for l3 in &third_levels {
            flat.extend_from_slice(l3);
        }

        let mut table = BTreeMap::new();
        for global_id in 0x000u16..=0xFFFu16 {
            let i1 = ((global_id & 0xF00) >> 8) as usize;
            let idx1 = *flat.get(i1).unwrap_or(&-1);
            if idx1 == -1 {
                continue;
            }
            let i2 = (idx1 as usize) + (((global_id & 0x0F0) >> 4) as usize);
            let idx2 = *flat.get(i2).unwrap_or(&-1);
            if idx2 == -1 {
                continue;
            }
            let i3 = (idx2 as usize) + ((global_id & 0x00F) as usize);
            let local_idx = *flat.get(i3).unwrap_or(&0xFFF);
            if local_idx != 0xFFF && local_idx >= 0 {
                table.insert(global_id, local_idx as u16);
            }
        }

        Ok(Self { table })
    }

    pub fn encode_blocks(&self) -> (Vec<i16>, usize) {
        let keys: Vec<u16> = self.table.keys().copied().collect();
        let mut first_table: Vec<Vec<u16>> = Vec::with_capacity(16);
        for i in 0..16u16 {
            let lower = i * 0x100;
            let upper = (i + 1) * 0x100;
            first_table.push(
                keys.iter()
                    .copied()
                    .filter(|&k| k >= lower && k < upper)
                    .collect(),
            );
        }

        let mut off: i16 = 0;
        let mut first_offsets = [0i16; 16];
        for (i, bucket) in first_table.iter().enumerate() {
            if bucket.is_empty() {
                first_offsets[i] = -1;
            } else {
                off += 0x10;
                first_offsets[i] = off;
            }
        }

        let mut second_tables: Vec<Vec<Vec<u16>>> = Vec::new();
        for bucket in first_table.iter().filter(|b| !b.is_empty()) {
            let mut sub: Vec<Vec<u16>> = Vec::with_capacity(16);
            for i in 0..16u16 {
                let lower = i * 0x10;
                let upper = (i + 1) * 0x10;
                sub.push(
                    bucket
                        .iter()
                        .copied()
                        .filter(|&k| (k & 0xFF) >= lower && (k & 0xFF) < upper)
                        .collect(),
                );
            }
            second_tables.push(sub);
        }

        let mut second_levels: Vec<[i16; 16]> = Vec::with_capacity(second_tables.len());
        for st in &second_tables {
            let mut block = [0i16; 16];
            for (i, ee) in st.iter().enumerate() {
                if ee.is_empty() {
                    block[i] = -1;
                } else {
                    off += 0x10;
                    block[i] = off;
                }
            }
            second_levels.push(block);
        }

        let mut third_levels: Vec<[i16; 16]> = Vec::new();
        for st in &second_tables {
            for ee in st.iter().filter(|ee| !ee.is_empty()) {
                let mut block = [0x0FFFi16; 16];
                for &k in ee {
                    if let Some(&local_id) = self.table.get(&k) {
                        block[(k & 0xF) as usize] = local_id as i16;
                    }
                }
                third_levels.push(block);
            }
        }

        let mut flat = Vec::with_capacity(
            16 + second_levels.len() * 16 + third_levels.len() * 16,
        );
        flat.extend_from_slice(&first_offsets);
        for b in &second_levels {
            flat.extend_from_slice(b);
        }
        for b in &third_levels {
            flat.extend_from_slice(b);
        }
        let byte_size = flat.len() * 2;
        (flat, byte_size)
    }

    pub fn byte_size(&self) -> usize {
        self.encode_blocks().1
    }

    pub fn write(&self, w: &mut BinWriter) {
        let (flat, _) = self.encode_blocks();
        for v in flat {
            w.write_i16(v);
        }
    }

    pub fn invert(&self) -> BTreeMap<u16, u16> {
        self.table.iter().map(|(&global, &local)| (local, global)).collect()
    }
}
