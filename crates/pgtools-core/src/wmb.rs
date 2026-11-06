//! `WMB` (Bayonetta 1 & Bayonetta 2) 3D model format parser, serializer, cleanup engine,
//! material decoder/editor, tangent recomputer, cross-platform converter (Wii U <-> PC <-> Switch),
//! and OBJ / glTF 2.0 exporter (`lib/bayonetta/wmb.rb`).

use crate::bone::{BoneIndexTranslateTable, BoneNode, Vec3};
use crate::databases::{
    lookup_material_type, lookup_wmb_vertex_layout, max_material_byte_size, MaterialParamType,
    VertexFieldType,
};
use crate::io::{align, checked_count, BinReader, BinWriter, Endian};
use crate::shader_db::lookup_b2_shader;
use crate::{PgError, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// Packed 4-byte vertex normal supporting both 8-bit signed `[0, nz, ny, nx]` (Bayonetta 1 PC)
/// and 10-10-10-2 signed bitfield (`unpack_wide` / `pack_wide` for Wii U, Switch, and Bayonetta 2).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PackedNormal {
    pub normal: [f32; 3],
    pub wide: bool,
    pub raw_bytes: [u8; 4],
}

impl Default for PackedNormal {
    fn default() -> Self {
        Self {
            normal: [0.0, 0.0, 1.0],
            wide: false,
            raw_bytes: [0, 127, 0, 0],
        }
    }
}

impl PackedNormal {
    fn normalize3(fx: f32, fy: f32, fz: f32) -> [f32; 3] {
        let nrm = (fx * fx + fy * fy + fz * fz).sqrt();
        if nrm == 0.0 {
            [0.0, 0.0, 0.0]
        } else {
            [fx / nrm, fy / nrm, fz / nrm]
        }
    }

    pub fn unpack_wide(v: u32) -> [f32; 3] {
        let mut nx = (v & 0x3FF) as i32;
        let mut ny = ((v >> 10) & 0x3FF) as i32;
        let mut nz = ((v >> 20) & 0x3FF) as i32;
        if (nx & 0x200) != 0 {
            nx -= 0x400;
        }
        if (ny & 0x200) != 0 {
            ny -= 0x400;
        }
        if (nz & 0x200) != 0 {
            nz -= 0x400;
        }
        let mag = 511.0f32;
        Self::normalize3((nx as f32) / mag, (ny as f32) / mag, (nz as f32) / mag)
    }

    pub fn pack_wide(normal: [f32; 3]) -> u32 {
        let mag = 511.0f32;
        let nx = ((normal[0] * mag) as i32).clamp(-512, 511) & 0x3FF;
        let ny = ((normal[1] * mag) as i32).clamp(-512, 511) & 0x3FF;
        let nz = ((normal[2] * mag) as i32).clamp(-512, 511) & 0x3FF;
        ((nz as u32) << 20) | ((ny as u32) << 10) | (nx as u32)
    }

    pub fn unpack_narrow(raw: [u8; 4]) -> [f32; 3] {
        let nz = raw[1] as i8;
        let ny = raw[2] as i8;
        let nx = raw[3] as i8;
        Self::normalize3((nx as f32) / 127.0, (ny as f32) / 127.0, (nz as f32) / 127.0)
    }

    pub fn pack_narrow(normal: [f32; 3]) -> [u8; 4] {
        let nx = ((normal[0] * 127.0) as i32).clamp(-128, 127) as i8 as u8;
        let ny = ((normal[1] * 127.0) as i32).clamp(-128, 127) as i8 as u8;
        let nz = ((normal[2] * 127.0) as i32).clamp(-128, 127) as i8 as u8;
        [0, nz, ny, nx]
    }

    pub fn read(r: &mut BinReader, is_bayo2: bool) -> Result<Self> {
        let b = r.read_bytes(4)?;
        let raw_bytes = [b[0], b[1], b[2], b[3]];
        let (normal, wide) = if r.endian.is_big() {
            (Self::unpack_wide(u32::from_be_bytes(raw_bytes)), true)
        } else if is_bayo2 {
            (Self::unpack_wide(u32::from_le_bytes(raw_bytes)), true)
        } else {
            (Self::unpack_narrow(raw_bytes), false)
        };
        Ok(Self {
            normal,
            wide,
            raw_bytes,
        })
    }

    pub fn write(&self, w: &mut BinWriter, is_bayo2: bool) {
        let out = if w.endian.is_big() {
            Self::pack_wide(self.normal).to_be_bytes()
        } else if is_bayo2 || self.wide {
            Self::pack_wide(self.normal).to_le_bytes()
        } else {
            Self::pack_narrow(self.normal)
        };
        w.write_bytes(&out);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PackedTangents {
    pub data: u32,
}

impl Default for PackedTangents {
    fn default() -> Self {
        Self { data: 0xFF80_8080 }
    }
}

impl PackedTangents {
    pub fn to_xyzw(self) -> [f32; 4] {
        let x = (((self.data & 0xFF) as f32) - 127.0) / 127.0;
        let y = ((((self.data >> 8) & 0xFF) as f32) - 127.0) / 127.0;
        let z = ((((self.data >> 16) & 0xFF) as f32) - 127.0) / 127.0;
        let s = ((((self.data >> 24) & 0xFF) as f32) - 127.0) / 127.0;
        [x, y, z, s]
    }

    pub fn from_xyzw(x: f32, y: f32, z: f32, s: f32) -> Self {
        let nrm = (x * x + y * y + z * z).sqrt();
        let (nx, ny, nz) = if nrm > 0.0 {
            (x / nrm, y / nrm, z / nrm)
        } else {
            (0.0, 0.0, 0.0)
        };
        let pack_b = |v: f32| ((v * 127.0 + 127.0).round().clamp(0.0, 255.0)) as u32;
        let data = pack_b(nx) | (pack_b(ny) << 8) | (pack_b(nz) << 16) | (pack_b(s) << 24);
        Self { data }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct BoneInfos {
    pub indexes: [u8; 4],
    pub weights: [u8; 4],
}

impl BoneInfos {
    pub fn read(r: &mut BinReader, is_bayo2: bool) -> Result<Self> {
        let read_u32_ulist = |r: &mut BinReader| -> Result<[u8; 4]> {
            let eff_endian = if r.endian.is_big() && !is_bayo2 {
                Endian::Big
            } else {
                Endian::Little
            };
            let b = r.read_bytes(4)?;
            let raw = match eff_endian {
                Endian::Little => u32::from_le_bytes([b[0], b[1], b[2], b[3]]),
                Endian::Big => u32::from_be_bytes([b[0], b[1], b[2], b[3]]),
            };
            Ok([
                (raw & 0xFF) as u8,
                ((raw >> 8) & 0xFF) as u8,
                ((raw >> 16) & 0xFF) as u8,
                ((raw >> 24) & 0xFF) as u8,
            ])
        };
        Ok(Self {
            indexes: read_u32_ulist(r)?,
            weights: read_u32_ulist(r)?,
        })
    }

    pub fn write(&self, w: &mut BinWriter, is_bayo2: bool) {
        let write_u32_ulist = |w: &mut BinWriter, arr: [u8; 4]| {
            let raw = (arr[0] as u32)
                | ((arr[1] as u32) << 8)
                | ((arr[2] as u32) << 16)
                | ((arr[3] as u32) << 24);
            let bytes = if w.endian.is_big() && !is_bayo2 {
                raw.to_be_bytes()
            } else {
                raw.to_le_bytes()
            };
            w.write_bytes(&bytes);
        };
        write_u32_ulist(w, self.indexes);
        write_u32_ulist(w, self.weights);
    }

    pub fn active_pairs(&self) -> Vec<(u8, u8)> {
        (0..4)
            .filter_map(|i| {
                if self.weights[i] > 0 {
                    Some((self.indexes[i], self.weights[i]))
                } else {
                    None
                }
            })
            .collect()
    }

    pub fn set_active_pairs(&mut self, pairs: &[(u8, u8)]) {
        self.indexes = [0; 4];
        self.weights = [0; 4];
        for (i, &(bi, bw)) in pairs.iter().take(4).enumerate() {
            self.indexes[i] = bi;
            self.weights[i] = bw;
        }
    }
}

/// Unified vertex representation across all 26 WMB1/WMB2 and 10 WMB3 vertex layouts.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct WmbVertex {
    pub position: Option<Vec3>,
    pub position2: Option<Vec3>,
    pub normal: PackedNormal,
    pub float_normal: Option<Vec3>,
    pub half_normal: Option<[f32; 4]>,
    pub tangents: PackedTangents,
    pub mapping: [f32; 2],
    pub mapping2: Option<[f32; 2]>,
    pub mapping3: Option<[f32; 2]>,
    pub mapping4: Option<[f32; 2]>,
    pub mapping5: Option<[f32; 2]>,
    pub color: Option<[u8; 4]>,
    pub bone_infos: Option<BoneInfos>,
}

impl WmbVertex {
    pub fn read_field(
        &mut self,
        r: &mut BinReader,
        field_name: &str,
        field_type: VertexFieldType,
        is_bayo2: bool,
    ) -> Result<()> {
        match field_type {
            VertexFieldType::Position => {
                let v = Vec3::read(r)?;
                if field_name == "position2" {
                    self.position2 = Some(v);
                } else {
                    self.position = Some(v);
                }
            }
            VertexFieldType::Mapping => {
                let uv = [r.read_f16()?, r.read_f16()?];
                match field_name {
                    "mapping" => self.mapping = uv,
                    "mapping2" => self.mapping2 = Some(uv),
                    "mapping3" => self.mapping3 = Some(uv),
                    "mapping4" => self.mapping4 = Some(uv),
                    "mapping5" => self.mapping5 = Some(uv),
                    _ => self.mapping = uv,
                }
            }
            VertexFieldType::FloatMapping => {
                let uv = [r.read_f32()?, r.read_f32()?];
                if field_name == "mapping2" {
                    self.mapping2 = Some(uv);
                } else {
                    self.mapping = uv;
                }
            }
            VertexFieldType::Normal => {
                self.normal = PackedNormal::read(r, is_bayo2)?;
            }
            VertexFieldType::FloatNormal => {
                let fnrm = Vec3::read(r)?;
                self.normal.normal = [fnrm.x, fnrm.y, fnrm.z];
                self.float_normal = Some(fnrm);
            }
            VertexFieldType::HalfNormal => {
                let hn = [r.read_f16()?, r.read_f16()?, r.read_f16()?, r.read_f16()?];
                self.normal.normal = [hn[0], hn[1], hn[2]];
                self.half_normal = Some(hn);
            }
            VertexFieldType::Tangents => {
                let b = r.read_bytes(4)?;
                let raw = if r.endian.is_big() && !is_bayo2 {
                    u32::from_be_bytes([b[0], b[1], b[2], b[3]])
                } else {
                    u32::from_le_bytes([b[0], b[1], b[2], b[3]])
                };
                self.tangents = PackedTangents { data: raw };
            }
            VertexFieldType::BoneInfos => {
                self.bone_infos = Some(BoneInfos::read(r, is_bayo2)?);
            }
            VertexFieldType::Color => {
                let b = r.read_bytes(4)?;
                let raw = if r.endian.is_big() && !is_bayo2 {
                    u32::from_be_bytes([b[0], b[1], b[2], b[3]])
                } else {
                    u32::from_le_bytes([b[0], b[1], b[2], b[3]])
                };
                self.color = Some([
                    (raw & 0xFF) as u8,
                    ((raw >> 8) & 0xFF) as u8,
                    ((raw >> 16) & 0xFF) as u8,
                    ((raw >> 24) & 0xFF) as u8,
                ]);
            }
        }
        Ok(())
    }

    pub fn write_field(
        &self,
        w: &mut BinWriter,
        field_name: &str,
        field_type: VertexFieldType,
        is_bayo2: bool,
    ) {
        match field_type {
            VertexFieldType::Position => {
                let v = if field_name == "position2" {
                    self.position2.unwrap_or_default()
                } else {
                    self.position.unwrap_or_default()
                };
                v.write(w);
            }
            VertexFieldType::Mapping => {
                let uv = match field_name {
                    "mapping" => self.mapping,
                    "mapping2" => self.mapping2.unwrap_or(self.mapping),
                    "mapping3" => self.mapping3.unwrap_or([0.0, 0.0]),
                    "mapping4" => self.mapping4.unwrap_or([0.0, 0.0]),
                    "mapping5" => self.mapping5.unwrap_or([0.0, 0.0]),
                    _ => self.mapping,
                };
                w.write_f16(uv[0]);
                w.write_f16(uv[1]);
            }
            VertexFieldType::FloatMapping => {
                let uv = if field_name == "mapping2" {
                    self.mapping2.unwrap_or(self.mapping)
                } else {
                    self.mapping
                };
                w.write_f32(uv[0]);
                w.write_f32(uv[1]);
            }
            VertexFieldType::Normal => {
                self.normal.write(w, is_bayo2);
            }
            VertexFieldType::FloatNormal => {
                let n = self.float_normal.unwrap_or(Vec3 {
                    x: self.normal.normal[0],
                    y: self.normal.normal[1],
                    z: self.normal.normal[2],
                });
                n.write(w);
            }
            VertexFieldType::HalfNormal => {
                let hn = self.half_normal.unwrap_or([
                    self.normal.normal[0],
                    self.normal.normal[1],
                    self.normal.normal[2],
                    0.0,
                ]);
                for &c in &hn {
                    w.write_f16(c);
                }
            }
            VertexFieldType::Tangents => {
                let bytes = if w.endian.is_big() && !is_bayo2 {
                    self.tangents.data.to_be_bytes()
                } else {
                    self.tangents.data.to_le_bytes()
                };
                w.write_bytes(&bytes);
            }
            VertexFieldType::BoneInfos => {
                self.bone_infos
                    .unwrap_or_default()
                    .write(w, is_bayo2);
            }
            VertexFieldType::Color => {
                let c = self.color.unwrap_or([0xC0, 0xC0, 0xC0, 0xFF]);
                let raw = (c[0] as u32)
                    | ((c[1] as u32) << 8)
                    | ((c[2] as u32) << 16)
                    | ((c[3] as u32) << 24);
                let bytes = if w.endian.is_big() && !is_bayo2 {
                    raw.to_be_bytes()
                } else {
                    raw.to_le_bytes()
                };
                w.write_bytes(&bytes);
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WmbMaterial {
    pub mat_type: i16,
    pub flag: u16,
    pub shader_name: Option<String>,
    pub material_data: Vec<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DecodedMaterialParameter {
    pub name: String,
    pub param_type: MaterialParamType,
    pub sampler_tex_id: Option<u32>,
    pub float_values: Option<[f32; 4]>,
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub component_labels: Option<[String; 4]>,
    #[serde(default)]
    pub pretexture_name: Option<String>,
    #[serde(default)]
    pub is_color: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DecodedWmbMaterial {
    pub index: usize,
    pub mat_type: i16,
    pub flag: u16,
    #[serde(default)]
    pub is_bayo2: bool,
    pub shader_name: Option<String>,
    #[serde(default)]
    pub shader_family: Option<String>,
    #[serde(default)]
    pub shader_flags_notes: Vec<String>,
    pub expected_byte_size: Option<usize>,
    pub fields: Vec<DecodedMaterialParameter>,
    pub raw_dwords: Vec<u32>,
}

impl WmbMaterial {
    pub fn decode(&self, index: usize) -> DecodedWmbMaterial {
        crate::shader_db::decode_wmb_material_rich(self, index, self.shader_name.is_some())
    }

    pub fn decode_with_hint(&self, index: usize, is_bayo2: bool) -> DecodedWmbMaterial {
        crate::shader_db::decode_wmb_material_rich(self, index, is_bayo2)
    }

    pub fn apply_decoded(&mut self, decoded: &DecodedWmbMaterial) {
        self.mat_type = decoded.mat_type;
        self.flag = decoded.flag;
        if let Some(ref sh) = decoded.shader_name {
            if decoded.is_bayo2 || self.shader_name.is_some() {
                self.shader_name = Some(sh.clone());
            }
        }
        if !decoded.fields.is_empty() {
            // Check if this is a B2 shader with matrix_blocks
            let matrix_words = decoded
                .shader_name
                .as_deref()
                .and_then(crate::shader_db::lookup_b2_shader)
                .map(|(_, lay)| (lay.tex_count, 16 * lay.matrix_blocks))
                .unwrap_or((0, 0));

            let mut cursor = 0usize;
            let mut samplers_written = 0usize;
            for f in &decoded.fields {
                if let Some(tid) = f.sampler_tex_id {
                    if cursor >= self.material_data.len() {
                        self.material_data.resize(cursor + 1, 0);
                    }
                    self.material_data[cursor] = tid;
                    cursor += 1;
                    samplers_written += 1;
                    if matrix_words.1 > 0 && samplers_written == matrix_words.0 {
                        cursor += matrix_words.1;
                        if cursor > self.material_data.len() {
                            self.material_data.resize(cursor, 0);
                        }
                    }
                } else if let Some(flts) = f.float_values {
                    if cursor + 4 > self.material_data.len() {
                        self.material_data.resize(cursor + 4, 0);
                    }
                    for k in 0..4 {
                        self.material_data[cursor + k] = flts[k].to_bits();
                    }
                    cursor += 4;
                }
            }
        } else if !decoded.raw_dwords.is_empty() {
            self.material_data = decoded.raw_dwords.clone();
        }
    }

    pub fn byte_size(&self) -> usize {
        4 + self.material_data.len() * 4
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WmbBatchHeader {
    pub batch_id: i16,
    pub mesh_id: i16,
    pub flags: u16,
    pub ex_mat_id: i16,
    pub material_id: u8,
    pub has_bone_refs: u8,
    pub u_e1: u8,
    pub u_e2: u8,
    pub vertex_start: u32,
    pub vertex_end: u32,
    pub primitive_type: i32,
    pub offset_indices: u32,
    pub num_indices: i32,
    pub vertex_offset: i32,
    pub u_f: [i32; 7],
}

impl WmbBatchHeader {
    /// Returns the effective material index for this batch (`ex_mat_id` in Bayonetta 2, `material_id` in Bayonetta 1).
    pub fn effective_material_id(&self, is_bayo2: bool) -> usize {
        if is_bayo2 {
            self.ex_mat_id.max(0) as usize
        } else {
            self.material_id as usize
        }
    }

    /// Checks whether this batch is a shadow-caster proxy mesh (`_shadow`) using the Bayonetta 1/2 engine convention:
    /// - Bayonetta 1: `u_e1 == 32 && u_e2 == 15` (`0x20, 0x0F`)
    /// - Bayonetta 2: `u_e1 == 48` (`0x30`)
    pub fn is_shadow_proxy(&self, is_bayo2: bool) -> bool {
        if is_bayo2 {
            self.u_e1 == 48
        } else {
            self.u_e1 == 32 && self.u_e2 == 15
        }
    }

    /// Sets or clears the shadow-caster proxy mesh flags (`u_e1`, `u_e2`) for Bayonetta 1 or Bayonetta 2.
    pub fn set_shadow_proxy(&mut self, is_bayo2: bool, enabled: bool) {
        if enabled {
            if is_bayo2 {
                self.u_e1 = 48;
                self.u_e2 = 0;
            } else {
                self.u_e1 = 32;
                self.u_e2 = 15;
            }
        } else {
            self.u_e1 = 0;
            self.u_e2 = 0;
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WmbBatch {
    pub header: WmbBatchHeader,
    pub bone_refs: Vec<u8>,
    pub unknown_no_bones: [f32; 4],
    pub indices: Vec<u16>,
}

impl WmbBatch {
    pub fn min_indices_offset(&self) -> u32 {
        let payload = if self.header.has_bone_refs != 0 {
            4 + self.bone_refs.len()
        } else {
            16
        };
        align(0x44 + payload, 0x10) as u32
    }

    pub fn effective_indices_offset(&self) -> u32 {
        self.header.offset_indices.max(self.min_indices_offset())
    }

    pub fn byte_size(&self) -> usize {
        (self.effective_indices_offset() as usize) + self.indices.len() * 2
    }

    pub fn vertex_indices(&self) -> Vec<usize> {
        let vo = self.header.vertex_offset.max(0) as usize;
        self.indices.iter().map(|&i| (i as usize) + vo).collect()
    }

    pub fn triangles(&self) -> Vec<[usize; 3]> {
        let inds = self.vertex_indices();
        if self.header.primitive_type == 4 {
            inds.chunks_exact(3)
                .map(|c| [c[0], c[1], c[2]])
                .collect()
        } else {
            let mut tris = Vec::new();
            for (i, w) in inds.windows(3).enumerate() {
                if w[0] == w[1] || w[1] == w[2] || w[0] == w[2] {
                    continue;
                }
                if i % 2 == 0 {
                    tris.push([w[0], w[1], w[2]]);
                } else {
                    tris.push([w[1], w[0], w[2]]);
                }
            }
            tris
        }
    }

    pub fn set_triangles(&mut self, tris: &[[usize; 3]]) {
        self.header.primitive_type = 4;
        let flat: Vec<usize> = tris.iter().flat_map(|t| t.iter().copied()).collect();
        self.recompute_from_absolute_indices(&flat);
    }

    pub fn recompute_from_absolute_indices(&mut self, abs_indices: &[usize]) {
        self.header.num_indices = abs_indices.len() as i32;
        if abs_indices.is_empty() {
            self.indices.clear();
            return;
        }
        let min_v = *abs_indices.iter().min().unwrap();
        let max_v = *abs_indices.iter().max().unwrap();
        self.header.vertex_start = min_v as u32;
        self.header.vertex_end = (max_v + 1) as u32;
        if max_v > 0xFFFF {
            self.header.vertex_offset = min_v as i32;
            self.indices = abs_indices.iter().map(|&i| (i - min_v) as u16).collect();
        } else {
            self.header.vertex_offset = 0;
            self.indices = abs_indices.iter().map(|&i| i as u16).collect();
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WmbMesh {
    pub id: i16,
    pub u_a1: i16,
    pub u_a2: i16,
    pub offset_batch_offsets: u32,
    pub u_b: u32,
    pub u_c: [i32; 4],
    pub name: String,
    pub bounding_matrix: [f32; 12],
    pub batches: Vec<WmbBatch>,
}

impl WmbMesh {
    pub fn byte_size(&self) -> usize {
        let mut sz = self.offset_batch_offsets as usize;
        sz += self.batches.len() * 4;
        sz = align(sz, 0x20);
        for b in &self.batches {
            sz += b.byte_size();
            sz = align(sz, 0x20);
        }
        sz
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WmbHeader {
    pub u_a: i32,
    pub vertex_flags: u32,
    pub num_vertexes: i32,
    pub vertex_ex_data_size: i8,
    pub vertex_ex_data: i8,
    pub u_e: i16,
    pub offset_positions: u32,
    pub offset_vertexes: u32,
    pub offset_vertexes_ex_data: u32,
    pub u_g: [i32; 4],
    pub num_bones: i32,
    pub offset_bone_hierarchy: u32,
    pub offset_bone_relative_position: u32,
    pub offset_bone_position: u32,
    pub offset_bone_index_translate_table: u32,
    pub num_materials: i32,
    pub offset_materials_offsets: u32,
    pub offset_materials: u32,
    pub num_meshes: i32,
    pub offset_meshes_offsets: u32,
    pub offset_meshes: u32,
    pub u_k: i32,
    pub u_l: i32,
    pub offset_u_j: u32,
    pub offset_bone_symmetries: u32,
    pub offset_bone_flags: u32,
    pub offset_shader_names: u32,
    pub offset_tex_infos: u32,
    pub u_m: u32,
    pub u_n: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WmbFile {
    pub endian: Endian,
    pub header: WmbHeader,
    pub positions: Vec<Vec3>,
    pub vertexes: Vec<WmbVertex>,
    pub bone_hierarchy: Vec<i16>,
    pub bone_relative_positions: Vec<Vec3>,
    pub bone_positions: Vec<Vec3>,
    pub bone_index_translate_table: BoneIndexTranslateTable,
    pub u_j_raw: Option<Vec<u8>>,
    pub bone_symmetries: Vec<i16>,
    pub bone_flags: Vec<i8>,
    pub tex_infos: Vec<(u32, i32)>,
    pub materials: Vec<WmbMaterial>,
    pub meshes: Vec<WmbMesh>,
}

impl WmbFile {
    pub fn is_bayo2(&self) -> bool {
        self.header.offset_shader_names != 0 || self.header.offset_tex_infos != 0
    }

    pub fn from_bytes(data: &[u8]) -> Result<Self> {
        if data.len() < 0x80 {
            return Err(PgError::InvalidData("WMB file shorter than 128-byte header".into()));
        }
        let endian = match &data[0..4] {
            b"WMB\0" => Endian::Little,
            b"\0BMW" => Endian::Big,
            other => {
                return Err(PgError::InvalidMagic {
                    expected: "WMB\\0 or \\0BMW",
                    actual: other.to_vec(),
                })
            }
        };

        let mut r = BinReader::new(data, endian);
        let _id = r.read_u32()?;
        let header = WmbHeader {
            u_a: r.read_i32()?,
            vertex_flags: r.read_u32()?,
            num_vertexes: r.read_i32()?,
            vertex_ex_data_size: r.read_i8()?,
            vertex_ex_data: r.read_i8()?,
            u_e: r.read_i16()?,
            offset_positions: r.read_u32()?,
            offset_vertexes: r.read_u32()?,
            offset_vertexes_ex_data: r.read_u32()?,
            u_g: [r.read_i32()?, r.read_i32()?, r.read_i32()?, r.read_i32()?],
            num_bones: r.read_i32()?,
            offset_bone_hierarchy: r.read_u32()?,
            offset_bone_relative_position: r.read_u32()?,
            offset_bone_position: r.read_u32()?,
            offset_bone_index_translate_table: r.read_u32()?,
            num_materials: r.read_i32()?,
            offset_materials_offsets: r.read_u32()?,
            offset_materials: r.read_u32()?,
            num_meshes: r.read_i32()?,
            offset_meshes_offsets: r.read_u32()?,
            offset_meshes: r.read_u32()?,
            u_k: r.read_i32()?,
            u_l: r.read_i32()?,
            offset_u_j: r.read_u32()?,
            offset_bone_symmetries: r.read_u32()?,
            offset_bone_flags: r.read_u32()?,
            offset_shader_names: r.read_u32()?,
            offset_tex_infos: r.read_u32()?,
            u_m: r.read_u32()?,
            u_n: r.read_u32()?,
        };

        let is_bayo2 = header.offset_shader_names != 0 || header.offset_tex_infos != 0;
        let num_v = header.num_vertexes.max(0) as usize;

        let layout = lookup_wmb_vertex_layout(
            header.vertex_flags,
            header.vertex_ex_data_size,
            header.vertex_ex_data,
        )
        .ok_or_else(|| {
            PgError::Unsupported(format!(
                "Unknown WMB vertex layout: flags=0x{:08X}, ex_size={}, ex={}",
                header.vertex_flags, header.vertex_ex_data_size, header.vertex_ex_data
            ))
        })?;

        if num_v > 0 {
            if header.offset_positions != 0 {
                checked_count(
                    num_v,
                    12,
                    data.len().saturating_sub(header.offset_positions as usize),
                    "WMB positions",
                )?;
            }
            if header.offset_vertexes != 0 && !layout.primary_fields.is_empty() {
                let p_stride: usize = layout.primary_fields.iter().map(|f| f.field_type.byte_size()).sum();
                checked_count(
                    num_v,
                    p_stride.max(1),
                    data.len().saturating_sub(header.offset_vertexes as usize),
                    "WMB vertexes",
                )?;
            } else if header.offset_positions == 0 {
                return Err(PgError::InvalidData(
                    "WMB has non-zero num_vertexes with no position or vertex buffer".into(),
                ));
            }
        }

        let mut positions = Vec::new();
        if header.offset_positions != 0 && num_v > 0 {
            r.seek(header.offset_positions as usize)?;
            positions.reserve(num_v);
            for _ in 0..num_v {
                positions.push(Vec3::read(&mut r)?);
            }
        }

        let mut vertexes = vec![WmbVertex::default(); num_v];
        if header.offset_vertexes != 0 && !layout.primary_fields.is_empty() {
            r.seek(header.offset_vertexes as usize)?;
            for v in &mut vertexes {
                for f in layout.primary_fields {
                    v.read_field(&mut r, f.name, f.field_type, is_bayo2)?;
                }
            }
        }
        if header.offset_vertexes_ex_data != 0 && !layout.ex_fields.is_empty() {
            r.seek(header.offset_vertexes_ex_data as usize)?;
            for v in &mut vertexes {
                for f in layout.ex_fields {
                    v.read_field(&mut r, f.name, f.field_type, is_bayo2)?;
                }
            }
        }

        let num_bones = header.num_bones.max(0) as usize;
        if num_bones > 0 && header.offset_bone_hierarchy != 0 {
            checked_count(
                num_bones,
                2,
                data.len().saturating_sub(header.offset_bone_hierarchy as usize),
                "WMB bone_hierarchy",
            )?;
        }
        let mut bone_hierarchy = Vec::with_capacity(num_bones);
        if header.offset_bone_hierarchy != 0 {
            r.seek(header.offset_bone_hierarchy as usize)?;
            for _ in 0..num_bones {
                bone_hierarchy.push(r.read_i16()?);
            }
        }

        let mut bone_relative_positions = Vec::with_capacity(num_bones);
        if header.offset_bone_relative_position != 0 {
            r.seek(header.offset_bone_relative_position as usize)?;
            for _ in 0..num_bones {
                bone_relative_positions.push(Vec3::read(&mut r)?);
            }
        }

        let mut bone_positions = Vec::with_capacity(num_bones);
        if header.offset_bone_position != 0 {
            r.seek(header.offset_bone_position as usize)?;
            for _ in 0..num_bones {
                bone_positions.push(Vec3::read(&mut r)?);
            }
        }

        let bone_index_translate_table = if header.offset_bone_index_translate_table != 0 {
            r.seek(header.offset_bone_index_translate_table as usize)?;
            BoneIndexTranslateTable::read(&mut r)?
        } else {
            BoneIndexTranslateTable::new()
        };

        let u_j_raw = if header.offset_u_j != 0 && (header.offset_u_j as usize) + 40 <= data.len() {
            r.seek(header.offset_u_j as usize)?;
            let b = r.read_bytes(40)?;
            Some(b.to_vec())
        } else {
            None
        };

        let mut bone_symmetries = Vec::new();
        if header.offset_bone_symmetries != 0 {
            r.seek(header.offset_bone_symmetries as usize)?;
            for _ in 0..num_bones {
                bone_symmetries.push(r.read_i16()?);
            }
        }

        let mut bone_flags = Vec::new();
        if header.offset_bone_flags != 0 {
            r.seek(header.offset_bone_flags as usize)?;
            for _ in 0..num_bones {
                bone_flags.push(r.read_i8()?);
            }
        }

        let num_materials = header.num_materials.max(0) as usize;
        if num_materials > 0 && header.offset_materials_offsets != 0 {
            checked_count(
                num_materials,
                4,
                data.len().saturating_sub(header.offset_materials_offsets as usize),
                "WMB materials_offsets",
            )?;
        }
        let mut shader_names = Vec::new();
        if header.offset_shader_names != 0 && num_materials > 0 {
            checked_count(
                num_materials,
                16,
                data.len().saturating_sub(header.offset_shader_names as usize),
                "WMB shader_names",
            )?;
            r.seek(header.offset_shader_names as usize)?;
            for _ in 0..num_materials {
                shader_names.push(r.read_fixed_string(16)?);
            }
        }

        let mut tex_infos = Vec::new();
        if header.offset_tex_infos != 0 {
            r.seek(header.offset_tex_infos as usize)?;
            let count = r.read_i32()?.max(0) as usize;
            checked_count(count, 8, data.len().saturating_sub(r.pos()), "WMB tex_infos")?;
            for _ in 0..count {
                tex_infos.push((r.read_u32()?, r.read_i32()?));
            }
        }

        let mut materials_offsets = Vec::with_capacity(num_materials);
        if header.offset_materials_offsets != 0 {
            r.seek(header.offset_materials_offsets as usize)?;
            for _ in 0..num_materials {
                materials_offsets.push(r.read_u32()? as usize);
            }
        }

        let mut materials = Vec::with_capacity(num_materials);
        for i in 0..num_materials {
            let mat_pos = (header.offset_materials as usize) + materials_offsets[i];
            let next_pos = if i + 1 < num_materials {
                (header.offset_materials as usize) + materials_offsets[i + 1]
            } else {
                header.offset_meshes_offsets as usize
            };
            let raw_dword_count = next_pos.saturating_sub(mat_pos + 4) / 4;
            r.seek(mat_pos)?;
            let mat_type = r.read_i16()?;
            let flag = r.read_u16()?;
            let dword_count = if i + 1 == num_materials {
                if let Some(sh_name) = shader_names.get(i) {
                    if let Some((_, b2_layout)) = lookup_b2_shader(sh_name) {
                        let expected = b2_layout.tex_count
                            + b2_layout.mat_vec4 * 4
                            + b2_layout.matrix_blocks * 16;
                        if expected > 0 && raw_dword_count > expected && (raw_dword_count - expected) < 8 {
                            expected
                        } else {
                            raw_dword_count
                        }
                    } else {
                        raw_dword_count
                    }
                } else if let Some(spec) = lookup_material_type(mat_type) {
                    let expected = spec.byte_size.saturating_sub(4) / 4;
                    if expected > 0 && raw_dword_count > expected && (raw_dword_count - expected) < 8 {
                        expected
                    } else {
                        raw_dword_count
                    }
                } else {
                    raw_dword_count
                }
            } else {
                raw_dword_count
            };
            let mut material_data = Vec::with_capacity(dword_count);
            for _ in 0..dword_count {
                material_data.push(r.read_u32()?);
            }
            materials.push(WmbMaterial {
                mat_type,
                flag,
                shader_name: shader_names.get(i).cloned(),
                material_data,
            });
        }

        let num_meshes = header.num_meshes.max(0) as usize;
        if num_meshes > 0 && header.offset_meshes_offsets != 0 {
            checked_count(
                num_meshes,
                4,
                data.len().saturating_sub(header.offset_meshes_offsets as usize),
                "WMB meshes_offsets",
            )?;
        }
        let mut meshes_offsets = Vec::with_capacity(num_meshes);
        if header.offset_meshes_offsets != 0 {
            r.seek(header.offset_meshes_offsets as usize)?;
            for _ in 0..num_meshes {
                meshes_offsets.push(r.read_u32()? as usize);
            }
        }

        let mut meshes = Vec::with_capacity(num_meshes);
        for i in 0..num_meshes {
            let mesh_pos = (header.offset_meshes as usize) + meshes_offsets[i];
            r.seek(mesh_pos)?;
            let id = r.read_i16()?;
            let num_batch = r.read_i16()?.max(0) as usize;
            let u_a1 = r.read_i16()?;
            let u_a2 = r.read_i16()?;
            let offset_batch_offsets = r.read_u32()?;
            let u_b = r.read_u32()?;
            let u_c = [r.read_i32()?, r.read_i32()?, r.read_i32()?, r.read_i32()?];
            let name = r.read_fixed_string(32)?;
            let mut bounding_matrix = [0.0f32; 12];
            for slot in &mut bounding_matrix {
                *slot = r.read_f32()?;
            }

            let batch_table_pos = mesh_pos + (offset_batch_offsets as usize);
            checked_count(
                num_batch,
                4,
                data.len().saturating_sub(batch_table_pos),
                "WMB batch_offsets",
            )?;
            r.seek(batch_table_pos)?;
            let mut batch_offsets = Vec::with_capacity(num_batch);
            for _ in 0..num_batch {
                batch_offsets.push(r.read_u32()? as usize);
            }

            let mut batches = Vec::with_capacity(num_batch);
            for &boff in &batch_offsets {
                let bpos = batch_table_pos + boff;
                r.seek(bpos)?;
                let bheader = WmbBatchHeader {
                    batch_id: r.read_i16()?,
                    mesh_id: r.read_i16()?,
                    flags: r.read_u16()?,
                    ex_mat_id: r.read_i16()?,
                    material_id: r.read_u8()?,
                    has_bone_refs: r.read_u8()?,
                    u_e1: r.read_u8()?,
                    u_e2: r.read_u8()?,
                    vertex_start: r.read_u32()?,
                    vertex_end: r.read_u32()?,
                    primitive_type: r.read_i32()?,
                    offset_indices: r.read_u32()?,
                    num_indices: r.read_i32()?,
                    vertex_offset: r.read_i32()?,
                    u_f: [
                        r.read_i32()?,
                        r.read_i32()?,
                        r.read_i32()?,
                        r.read_i32()?,
                        r.read_i32()?,
                        r.read_i32()?,
                        r.read_i32()?,
                    ],
                };
                let mut bone_refs = Vec::new();
                let mut unknown_no_bones = [0.0f32; 4];
                if bheader.has_bone_refs != 0 {
                    let num_br = r.read_i32()?.max(0) as usize;
                    bone_refs = r.read_bytes(num_br)?.to_vec();
                } else {
                    for slot in &mut unknown_no_bones {
                        *slot = r.read_f32()?;
                    }
                }
                let ind_pos = bpos + (bheader.offset_indices as usize);
                r.seek(ind_pos)?;
                let num_ind = bheader.num_indices.max(0) as usize;
                checked_count(num_ind, 2, data.len().saturating_sub(ind_pos), "WMB batch indices")?;
                let mut indices = Vec::with_capacity(num_ind);
                for _ in 0..num_ind {
                    indices.push(r.read_u16()?);
                }
                batches.push(WmbBatch {
                    header: bheader,
                    bone_refs,
                    unknown_no_bones,
                    indices,
                });
            }

            meshes.push(WmbMesh {
                id,
                u_a1,
                u_a2,
                offset_batch_offsets,
                u_b,
                u_c,
                name,
                bounding_matrix,
                batches,
            });
        }

        Ok(Self {
            endian,
            header,
            positions,
            vertexes,
            bone_hierarchy,
            bone_relative_positions,
            bone_positions,
            bone_index_translate_table,
            u_j_raw,
            bone_symmetries,
            bone_flags,
            tex_infos,
            materials,
            meshes,
        })
    }

    pub fn normals_to_wide(&mut self) {
        for v in &mut self.vertexes {
            v.normal.wide = true;
        }
    }

    pub fn check_normals(&mut self) {
        if self.endian.is_big() {
            return;
        }
        let is_wide = self.vertexes.iter().any(|v| v.normal.raw_bytes[0] != 0);
        if is_wide {
            for v in &mut self.vertexes {
                v.normal.wide = true;
                v.normal.normal = PackedNormal::unpack_wide(u32::from_le_bytes(v.normal.raw_bytes));
            }
        }
    }

    pub fn cleanup_material_sizes(&mut self) -> Result<()> {
        if self.is_bayo2() {
            return Err(PgError::Unsupported(
                "cleanup_material_sizes is only supported for Bayonetta 1 WMB files".into(),
            ));
        }
        for m in &mut self.materials {
            if let Some(spec) = lookup_material_type(m.mat_type) {
                let dwords = (spec.byte_size.saturating_sub(4)) / 4;
                m.material_data.truncate(dwords);
            }
        }
        Ok(())
    }

    pub fn maximize_material_sizes(&mut self) -> Result<()> {
        if self.is_bayo2() {
            return Err(PgError::Unsupported(
                "maximize_material_sizes is only supported for Bayonetta 1 WMB files".into(),
            ));
        }
        let max_dwords = (max_material_byte_size().saturating_sub(4)) / 4;
        for m in &mut self.materials {
            if m.material_data.len() < max_dwords {
                m.material_data.resize(max_dwords, 0);
            }
        }
        Ok(())
    }

    pub fn cleanup_materials(&mut self) {
        let is_b2 = self.is_bayo2();
        let mut used = BTreeSet::new();
        for m in &self.meshes {
            for b in &m.batches {
                if is_b2 {
                    used.insert(b.header.ex_mat_id as usize);
                } else {
                    used.insert(b.header.material_id as usize);
                }
            }
        }
        let mut remap = BTreeMap::new();
        let mut new_mats = Vec::with_capacity(used.len());
        for (old_idx, mat) in self.materials.iter().enumerate() {
            if used.contains(&old_idx) {
                remap.insert(old_idx, new_mats.len());
                new_mats.push(mat.clone());
            }
        }
        self.materials = new_mats;
        self.header.num_materials = self.materials.len() as i32;
        for m in &mut self.meshes {
            for b in &mut m.batches {
                if is_b2 {
                    if let Some(&n) = remap.get(&(b.header.ex_mat_id as usize)) {
                        b.header.ex_mat_id = n as i16;
                    }
                } else if let Some(&n) = remap.get(&(b.header.material_id as usize)) {
                    b.header.material_id = n as u8;
                }
            }
        }
    }

    pub fn cleanup_vertexes(&mut self) {
        let mut used_set = BTreeSet::new();
        for m in &self.meshes {
            for b in &m.batches {
                for vi in b.vertex_indices() {
                    if vi < self.vertexes.len() {
                        used_set.insert(vi);
                    }
                }
            }
        }
        let used_vec: Vec<usize> = used_set.into_iter().collect();
        let mut v_map = BTreeMap::new();
        let mut new_verts = Vec::with_capacity(used_vec.len());
        let mut new_pos = Vec::new();
        for (new_idx, &old_idx) in used_vec.iter().enumerate() {
            v_map.insert(old_idx, new_idx);
            new_verts.push(self.vertexes[old_idx].clone());
            if !self.positions.is_empty() && old_idx < self.positions.len() {
                new_pos.push(self.positions[old_idx]);
            }
        }
        self.vertexes = new_verts;
        if !self.positions.is_empty() {
            self.positions = new_pos;
        }
        self.header.num_vertexes = self.vertexes.len() as i32;

        for m in &mut self.meshes {
            for b in &mut m.batches {
                let mapped: Vec<usize> = b
                    .vertex_indices()
                    .into_iter()
                    .filter_map(|old| v_map.get(&old).copied())
                    .collect();
                b.recompute_from_absolute_indices(&mapped);
            }
        }
    }

    pub fn remove_triangle_strips(&mut self) {
        for m in &mut self.meshes {
            for b in &mut m.batches {
                let tris = b.triangles();
                b.set_triangles(&tris);
            }
        }
    }

    pub fn scale(&mut self, s: f32) {
        for p in &mut self.positions {
            *p = p.scale(s);
        }
        for v in &mut self.vertexes {
            if let Some(ref mut p) = v.position {
                *p = p.scale(s);
            }
            if let Some(ref mut p2) = v.position2 {
                *p2 = p2.scale(s);
            }
        }
        for bp in &mut self.bone_positions {
            *bp = bp.scale(s);
        }
        self.recompute_relative_positions();
    }

    pub fn shift(&mut self, dx: f32, dy: f32, dz: f32) {
        let delta = Vec3::new(dx, dy, dz);
        for p in &mut self.positions {
            *p = p.add(delta);
        }
        for v in &mut self.vertexes {
            if let Some(ref mut p) = v.position {
                *p = p.add(delta);
            }
            if let Some(ref mut p2) = v.position2 {
                *p2 = p2.add(delta);
            }
        }
        for bp in &mut self.bone_positions {
            *bp = bp.add(delta);
        }
        self.recompute_relative_positions();
    }

    pub fn recompute_relative_positions(&mut self) {
        for i in 0..self.bone_positions.len() {
            let parent = self.bone_hierarchy.get(i).copied().unwrap_or(-1);
            let rel = if parent >= 0 && (parent as usize) < self.bone_positions.len() {
                self.bone_positions[i].sub(self.bone_positions[parent as usize])
            } else {
                self.bone_positions[i]
            };
            if i < self.bone_relative_positions.len() {
                self.bone_relative_positions[i] = rel;
            }
        }
    }

    pub fn dump_bones(&self) -> Vec<BoneNode> {
        let inv = self.bone_index_translate_table.invert();
        let n = self.bone_positions.len();
        (0..n)
            .map(|bi| BoneNode {
                index: bi,
                parent_index: self.bone_hierarchy.get(bi).copied().unwrap_or(-1),
                position: self.bone_positions[bi],
                relative_position: self
                    .bone_relative_positions
                    .get(bi)
                    .copied()
                    .unwrap_or_default(),
                global_index: inv.get(&(bi as u16)).map(|&g| g as i16).unwrap_or(-1),
                symmetric: self.bone_symmetries.get(bi).copied().unwrap_or(-1),
                flag: self.bone_flags.get(bi).copied().unwrap_or(5),
            })
            .collect()
    }

    /// Recompute all section offsets (`WMBFile#recompute_layout` in `wmb.rb`) and serialize to binary.
    pub fn to_bytes(&mut self, output_endian: Endian) -> Result<Vec<u8>> {
        let is_b2 = self.is_bayo2();
        let layout = lookup_wmb_vertex_layout(
            self.header.vertex_flags,
            self.header.vertex_ex_data_size,
            self.header.vertex_ex_data,
        )
        .ok_or_else(|| {
            PgError::Unsupported(format!(
                "Unknown WMB vertex layout: 0x{:08X}",
                self.header.vertex_flags
            ))
        })?;

        let vertex_size: usize = layout
            .primary_fields
            .iter()
            .map(|f| f.field_type.byte_size())
            .sum();
        let vertex_ex_size: usize = layout
            .ex_fields
            .iter()
            .map(|f| f.field_type.byte_size())
            .sum();

        let mut last_offset = if is_b2 { 0xC0usize } else { 0x80usize };
        let num_v = self.vertexes.len();
        self.header.num_vertexes = num_v as i32;

        if !self.positions.is_empty() {
            last_offset = align(last_offset, 0x20);
            self.header.offset_positions = last_offset as u32;
            last_offset += num_v * 12;
        }
        if vertex_size > 0 {
            last_offset = align(last_offset, 0x20);
            self.header.offset_vertexes = last_offset as u32;
            last_offset += num_v * vertex_size;
        }
        if vertex_ex_size > 0 {
            if is_b2 {
                last_offset += 0x20;
            }
            last_offset = align(last_offset, 0x20);
            self.header.offset_vertexes_ex_data = last_offset as u32;
            last_offset += num_v * vertex_ex_size;
        }

        let num_bones = self.bone_positions.len();
        self.header.num_bones = num_bones as i32;
        if num_bones > 0 {
            last_offset = align(last_offset, 0x20);
            self.header.offset_bone_hierarchy = last_offset as u32;
            last_offset += num_bones * 2;

            last_offset = align(last_offset, 0x20);
            self.header.offset_bone_relative_position = last_offset as u32;
            last_offset += num_bones * 12;

            last_offset = align(last_offset, 0x20);
            self.header.offset_bone_position = last_offset as u32;
            last_offset += num_bones * 12;

            last_offset = align(last_offset, 0x20);
            self.header.offset_bone_index_translate_table = last_offset as u32;
            last_offset += self.bone_index_translate_table.byte_size();
        }

        if self.u_j_raw.is_some() {
            last_offset = align(last_offset, 0x20);
            self.header.offset_u_j = last_offset as u32;
            last_offset += 40;
        }
        if !self.bone_symmetries.is_empty() {
            last_offset = align(last_offset, 0x20);
            self.header.offset_bone_symmetries = last_offset as u32;
            last_offset += num_bones * 2;
        }
        if !self.bone_flags.is_empty() {
            last_offset = align(last_offset, 0x20);
            self.header.offset_bone_flags = last_offset as u32;
            last_offset += num_bones;
        }
        if is_b2 && self.header.offset_shader_names > 0 {
            last_offset = align(last_offset, 0x20);
            self.header.offset_shader_names = last_offset as u32;
            last_offset += self.materials.len() * 16;
        }
        if is_b2 && self.header.offset_tex_infos > 0 {
            last_offset = align(last_offset, 0x20);
            self.header.offset_tex_infos = last_offset as u32;
            last_offset += 4 + self.tex_infos.len() * 8;
        }

        self.header.num_materials = self.materials.len() as i32;
        last_offset = align(last_offset, 0x20);
        self.header.offset_materials_offsets = last_offset as u32;

        let mut materials_offsets = Vec::with_capacity(self.materials.len());
        let mut mat_rel = 0usize;
        for m in &self.materials {
            materials_offsets.push(mat_rel as u32);
            mat_rel = align(mat_rel + m.byte_size(), 4);
        }
        last_offset += 4 * self.materials.len();
        last_offset = align(last_offset, 0x20);
        self.header.offset_materials = last_offset as u32;

        let total_mat_bytes: usize = mat_rel;
        let padded_end = align(last_offset + total_mat_bytes, 0x20);
        let pad_dwords = (padded_end - (last_offset + total_mat_bytes)) / 4;
        last_offset = padded_end;

        self.header.num_meshes = self.meshes.len() as i32;
        self.header.offset_meshes_offsets = last_offset as u32;
        let mut meshes_offsets = Vec::with_capacity(self.meshes.len());
        let mut mesh_rel = 0usize;
        for m in &self.meshes {
            meshes_offsets.push(mesh_rel as u32);
            mesh_rel = align(mesh_rel + m.byte_size(), 0x20);
        }
        last_offset += 4 * self.meshes.len();
        last_offset = align(last_offset, 0x20);
        self.header.offset_meshes = last_offset as u32;
        let total_size = align(last_offset + mesh_rel, 0x20);

        let mut w = BinWriter::with_capacity(total_size, output_endian);
        if output_endian.is_big() {
            w.write_bytes(b"\0BMW");
        } else {
            w.write_bytes(b"WMB\0");
        }
        let u_a = if self.endian != output_endian {
            if output_endian.is_big() { -1 } else { 0 }
        } else {
            self.header.u_a
        };
        w.write_i32(u_a);
        w.write_u32(self.header.vertex_flags);
        w.write_i32(self.header.num_vertexes);
        w.write_i8(self.header.vertex_ex_data_size);
        w.write_i8(self.header.vertex_ex_data);
        w.write_i16(self.header.u_e);
        w.write_u32(self.header.offset_positions);
        w.write_u32(self.header.offset_vertexes);
        w.write_u32(self.header.offset_vertexes_ex_data);
        for &g in &self.header.u_g {
            w.write_i32(g);
        }
        w.write_i32(self.header.num_bones);
        w.write_u32(self.header.offset_bone_hierarchy);
        w.write_u32(self.header.offset_bone_relative_position);
        w.write_u32(self.header.offset_bone_position);
        w.write_u32(self.header.offset_bone_index_translate_table);
        w.write_i32(self.header.num_materials);
        w.write_u32(self.header.offset_materials_offsets);
        w.write_u32(self.header.offset_materials);
        w.write_i32(self.header.num_meshes);
        w.write_u32(self.header.offset_meshes_offsets);
        w.write_u32(self.header.offset_meshes);
        w.write_i32(self.header.u_k);
        w.write_i32(self.header.u_l);
        w.write_u32(self.header.offset_u_j);
        w.write_u32(self.header.offset_bone_symmetries);
        w.write_u32(self.header.offset_bone_flags);
        w.write_u32(self.header.offset_shader_names);
        w.write_u32(self.header.offset_tex_infos);
        w.write_u32(self.header.u_m);
        w.write_u32(self.header.u_n);

        if self.header.offset_positions != 0 {
            w.seek(self.header.offset_positions as usize);
            for p in &self.positions {
                p.write(&mut w);
            }
        }
        if self.header.offset_vertexes != 0 {
            w.seek(self.header.offset_vertexes as usize);
            for v in &self.vertexes {
                for f in layout.primary_fields {
                    v.write_field(&mut w, f.name, f.field_type, is_b2);
                }
            }
        }
        if self.header.offset_vertexes_ex_data != 0 {
            w.seek(self.header.offset_vertexes_ex_data as usize);
            for v in &self.vertexes {
                for f in layout.ex_fields {
                    v.write_field(&mut w, f.name, f.field_type, is_b2);
                }
            }
        }
        if self.header.offset_bone_hierarchy != 0 {
            w.seek(self.header.offset_bone_hierarchy as usize);
            for &bh in &self.bone_hierarchy {
                w.write_i16(bh);
            }
        }
        if self.header.offset_bone_relative_position != 0 {
            w.seek(self.header.offset_bone_relative_position as usize);
            for p in &self.bone_relative_positions {
                p.write(&mut w);
            }
        }
        if self.header.offset_bone_position != 0 {
            w.seek(self.header.offset_bone_position as usize);
            for p in &self.bone_positions {
                p.write(&mut w);
            }
        }
        if self.header.offset_bone_index_translate_table != 0 {
            w.seek(self.header.offset_bone_index_translate_table as usize);
            self.bone_index_translate_table.write(&mut w);
        }
        if let Some(ref uj) = self.u_j_raw {
            w.seek(self.header.offset_u_j as usize);
            w.write_bytes(uj);
        }
        if self.header.offset_bone_symmetries != 0 {
            w.seek(self.header.offset_bone_symmetries as usize);
            for &bs in &self.bone_symmetries {
                w.write_i16(bs);
            }
        }
        if self.header.offset_bone_flags != 0 {
            w.seek(self.header.offset_bone_flags as usize);
            for &bf in &self.bone_flags {
                w.write_i8(bf);
            }
        }
        if self.header.offset_shader_names != 0 {
            w.seek(self.header.offset_shader_names as usize);
            for m in &self.materials {
                w.write_fixed_string(m.shader_name.as_deref().unwrap_or("dummy"), 16);
            }
        }
        if self.header.offset_tex_infos != 0 {
            w.seek(self.header.offset_tex_infos as usize);
            w.write_i32(self.tex_infos.len() as i32);
            for &(id, info) in &self.tex_infos {
                w.write_u32(id);
                w.write_i32(info);
            }
        }

        w.seek(self.header.offset_materials_offsets as usize);
        for &mo in &materials_offsets {
            w.write_u32(mo);
        }
        for (i, m) in self.materials.iter().enumerate() {
            w.seek((self.header.offset_materials as usize) + (materials_offsets[i] as usize));
            w.write_i16(m.mat_type);
            w.write_u16(m.flag);
            for &d in &m.material_data {
                w.write_u32(d);
            }
        }
        for _ in 0..pad_dwords {
            w.write_u32(0);
        }

        w.seek(self.header.offset_meshes_offsets as usize);
        for &mo in &meshes_offsets {
            w.write_u32(mo);
        }
        for (i, m) in self.meshes.iter().enumerate() {
            let mesh_pos = (self.header.offset_meshes as usize) + (meshes_offsets[i] as usize);
            w.seek(mesh_pos);
            w.write_i16(m.id);
            w.write_i16(m.batches.len() as i16);
            w.write_i16(m.u_a1);
            w.write_i16(m.u_a2);
            w.write_u32(m.offset_batch_offsets);
            w.write_u32(m.u_b);
            for &c in &m.u_c {
                w.write_i32(c);
            }
            w.write_fixed_string(&m.name, 32);
            for &f in &m.bounding_matrix {
                w.write_f32(f);
            }

            let batch_table_pos = mesh_pos + (m.offset_batch_offsets as usize);
            let mut mesh_rel = align((m.offset_batch_offsets as usize) + m.batches.len() * 4, 0x20);
            let mut batch_offsets = Vec::with_capacity(m.batches.len());
            for b in &m.batches {
                mesh_rel = align(mesh_rel, 0x20);
                let rel_off = mesh_rel - (m.offset_batch_offsets as usize);
                batch_offsets.push(rel_off as u32);
                mesh_rel += b.byte_size();
                mesh_rel = align(mesh_rel, 0x20);
            }

            w.seek(batch_table_pos);
            for &bo in &batch_offsets {
                w.write_u32(bo);
            }

            for (j, b) in m.batches.iter().enumerate() {
                let bpos = batch_table_pos + (batch_offsets[j] as usize);
                let eff_idx_off = b.effective_indices_offset();
                w.seek(bpos);
                w.write_i16(b.header.batch_id);
                w.write_i16(b.header.mesh_id);
                w.write_u16(b.header.flags);
                w.write_i16(b.header.ex_mat_id);
                w.write_u8(b.header.material_id);
                w.write_u8(b.header.has_bone_refs);
                w.write_u8(b.header.u_e1);
                w.write_u8(b.header.u_e2);
                w.write_u32(b.header.vertex_start);
                w.write_u32(b.header.vertex_end);
                w.write_i32(b.header.primitive_type);
                w.write_u32(eff_idx_off);
                w.write_i32(b.indices.len() as i32);
                w.write_i32(b.header.vertex_offset);
                for &f in &b.header.u_f {
                    w.write_i32(f);
                }
                if b.header.has_bone_refs != 0 {
                    w.write_i32(b.bone_refs.len() as i32);
                    w.write_bytes(&b.bone_refs);
                } else {
                    for &u in &b.unknown_no_bones {
                        w.write_f32(u);
                    }
                }
                w.seek(bpos + (eff_idx_off as usize));
                for &idx in &b.indices {
                    w.write_u16(idx);
                }
            }
        }

        w.seek(total_size);
        Ok(w.into_inner())
    }

    /// Export the WMB model to Wavefront `.obj` in pure Rust (zero Assimp dependency).
    pub fn export_obj(&self) -> String {
        let mut out = String::from("# Exported by pgtools-rs\n");
        for (i, v) in self.vertexes.iter().enumerate() {
            let p = v
                .position
                .or_else(|| self.positions.get(i).copied())
                .unwrap_or_default();
            out.push_str(&format!("v {:.6} {:.6} {:.6}\n", p.x, p.y, p.z));
        }
        for v in &self.vertexes {
            out.push_str(&format!(
                "vt {:.6} {:.6}\n",
                v.mapping[0],
                1.0 - v.mapping[1]
            ));
        }
        for v in &self.vertexes {
            let n = v.normal.normal;
            out.push_str(&format!("vn {:.6} {:.6} {:.6}\n", n[0], n[1], n[2]));
        }

        for m in &self.meshes {
            out.push_str(&format!("\no {}\n", m.name));
            for (bi, b) in m.batches.iter().enumerate() {
                out.push_str(&format!(
                    "g {}_{:02}_mat{:02}\n",
                    m.name,
                    bi,
                    b.header.effective_material_id(self.is_bayo2())
                ));
                for [i0, i1, i2] in b.triangles() {
                    let a = i0 + 1;
                    let b_idx = i1 + 1;
                    let c = i2 + 1;
                    out.push_str(&format!(
                        "f {a}/{a}/{a} {b_idx}/{b_idx}/{b_idx} {c}/{c}/{c}\n"
                    ));
                }
            }
        }
        out
    }
}
