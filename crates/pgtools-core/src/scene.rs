//! Scene export contract (`WmbSceneManifest`, binary geometry regions, `UidMap`, FNV-1a hashing,
//! and `WTX1` texture decoding/encoding) for the PGTools Studio 3D WMB/WMB3 workspace.

use crate::databases::{
    lookup_wmb3_vertex_layout_with_strides, lookup_wmb_vertex_layout, VertexFieldType,
};
use crate::io::{align, BinReader, Endian, MAX_ARCHIVE_ALLOC_BYTES};
use crate::shader_db::{decode_wmb_material_rich, list_core_pretextures};
use crate::texture_conv::{
    compute_xt1_layout, decode_bntx_subtexture_mip_filtered, decode_linear_blocks_to_rgba8,
    decode_texture_mip_filtered, gob_swizzle, inspect_texture_blob, TextureFormat,
};
use crate::wmb::{PackedNormal, WmbFile};
use crate::wmb3::Wmb3File;
use crate::{PgError, Result};
use image::RgbaImage;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const WTX1_MAGIC: u32 = 0x3158_5457; // "WTX1" in little-endian

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Region {
    pub offset: u32,
    pub byte_length: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub omitted: Option<bool>,
}

impl Region {
    pub fn included(offset: u32, byte_length: u32) -> Self {
        Self {
            offset,
            byte_length,
            omitted: None,
        }
    }

    pub fn omitted_region(byte_length: u32) -> Self {
        Self {
            offset: 0,
            byte_length,
            omitted: Some(true),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WmbSceneManifest {
    pub schema: u32,
    pub structure_version: u64,
    pub node_id: String,
    pub file_name: String,
    pub game: String,
    pub is_wmb3: bool,
    pub bounding_box: [f32; 6],
    pub lod_levels: Vec<i32>,
    pub mesh_groups: Vec<MeshGroupDesc>,
    pub batches: Vec<BatchDesc>,
    pub materials: Vec<MaterialDesc>,
    pub bones: Vec<BoneDesc>,
    pub textures: Vec<TextureRefDesc>,
    pub vertex_buffers: Vec<VertexBufferDesc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LodDesc {
    pub id: String,
    pub name: String,
    pub level: i32,
    pub batch_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MeshGroupDesc {
    pub id: String,
    pub index: usize,
    pub name: String,
    pub batch_ids: Vec<String>,
    pub lods: Vec<LodDesc>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchDesc {
    pub id: String,
    pub index: usize,
    pub mesh_id: String,
    pub lod_id: Option<String>,
    pub file_order: usize,
    pub material_index: usize,
    pub material_id: String,
    pub is_shadow_proxy: bool,
    pub vertex_buffer_index: usize,
    pub indices: Region,
    pub index_count: u32,
    pub vertex_count: u32,
    pub geometry_hash: String,
    pub bbox: [f32; 6],
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub has_dangling_material: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flags: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ex_mat_id: Option<i16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_material_id: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub has_bone_refs: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub u_e1: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub u_e2: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub primitive_type: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vertex_start: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vertex_end: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vertex_offset: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub u_f: Option<[i32; 7]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bone_refs: Option<Vec<u8>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unknown_no_bones: Option<[f32; 4]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bone_set_index: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub call_tree_node_index: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mesh_material_index: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub u_b: Option<i32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VertexBufferDesc {
    pub index: usize,
    pub vertex_count: u32,
    pub geometry_hash: String,
    pub position: Region,
    pub normal: Option<Region>,
    pub tangent: Option<Region>,
    pub uv0: Option<Region>,
    pub uv1: Option<Region>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uv2: Option<Region>,
    pub color: Option<Region>,
}

#[inline]
fn sanitize_uv_pair(uv: [f32; 2]) -> [f32; 2] {
    let u = if uv[0].is_finite() && uv[0].abs() < 1e4 { uv[0] } else { 0.0 };
    let v = if uv[1].is_finite() && uv[1].abs() < 1e4 { uv[1] } else { 0.0 };
    [u, v]
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MaterialParamDesc {
    pub key: String,
    pub name: String,
    pub values: Vec<f32>,
    pub is_color: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MaterialDesc {
    pub id: String,
    pub index: usize,
    pub name: String,
    pub game: String,
    pub shader_name: Option<String>,
    pub material_type: Option<i16>,
    pub technique: Option<String>,
    pub texture_slots: Vec<TextureSlotDesc>,
    pub params: Vec<MaterialParamDesc>,
    pub used_by_batch_ids: Vec<String>,
    pub thumbnail_data_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextureSlotDesc {
    pub slot: usize,
    pub name: String,
    pub role: String,
    pub texture_id: u32,
    pub texture_id_hex: String,
    pub global: bool,
    pub present: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextureRefDesc {
    pub texture_id: u32,
    pub id_hex: String,
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub format_name: String,
    pub content_hash: String,
    pub used_by_material_ids: Vec<String>,
    pub thumbnail_data_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BoneDesc {
    pub global_id: i32,
    pub local_id: Option<usize>,
    pub parent: i32,
    pub name: Option<String>,
    pub local_position: [f32; 3],
    pub world_position: [f32; 3],
    pub rotation: Option<[f32; 3]>,
    pub scale: Option<[f32; 3]>,
}

/// Session-stable UID tracker for a single WMB or WMB3 model node.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct UidMap {
    pub next_uid: u64,
    pub mesh_uids: Vec<u64>,
    pub material_uids: Vec<u64>,
    /// WMB0/2: `wmb_batch_uids[mesh_idx][batch_idx]`
    pub wmb_batch_uids: Vec<Vec<u64>>,
    /// WMB3: `wmb3_batch_uids[global_batch_idx]`
    pub wmb3_batch_uids: Vec<u64>,
}

impl UidMap {
    pub fn new() -> Self {
        Self {
            next_uid: 1,
            mesh_uids: Vec::new(),
            material_uids: Vec::new(),
            wmb_batch_uids: Vec::new(),
            wmb3_batch_uids: Vec::new(),
        }
    }

    pub fn alloc(&mut self) -> u64 {
        if self.next_uid == 0 {
            self.next_uid = 1;
        }
        let id = self.next_uid;
        self.next_uid += 1;
        id
    }

    pub fn ensure_wmb(&mut self, wmb: &WmbFile) {
        if self.next_uid == 0 {
            self.next_uid = 1;
        }
        while self.mesh_uids.len() < wmb.meshes.len() {
            let id = self.alloc();
            self.mesh_uids.push(id);
        }
        self.mesh_uids.truncate(wmb.meshes.len());

        while self.material_uids.len() < wmb.materials.len() {
            let id = self.alloc();
            self.material_uids.push(id);
        }
        self.material_uids.truncate(wmb.materials.len());

        while self.wmb_batch_uids.len() < wmb.meshes.len() {
            self.wmb_batch_uids.push(Vec::new());
        }
        self.wmb_batch_uids.truncate(wmb.meshes.len());
        for (mi, mesh) in wmb.meshes.iter().enumerate() {
            while self.wmb_batch_uids[mi].len() < mesh.batches.len() {
                let id = self.alloc();
                self.wmb_batch_uids[mi].push(id);
            }
            self.wmb_batch_uids[mi].truncate(mesh.batches.len());
        }
    }

    pub fn ensure_wmb3(&mut self, wmb3: &Wmb3File) {
        if self.next_uid == 0 {
            self.next_uid = 1;
        }
        while self.mesh_uids.len() < wmb3.meshes.len() {
            let id = self.alloc();
            self.mesh_uids.push(id);
        }
        self.mesh_uids.truncate(wmb3.meshes.len());

        while self.material_uids.len() < wmb3.materials.len() {
            let id = self.alloc();
            self.material_uids.push(id);
        }
        self.material_uids.truncate(wmb3.materials.len());

        while self.wmb3_batch_uids.len() < wmb3.batches.len() {
            let id = self.alloc();
            self.wmb3_batch_uids.push(id);
        }
        self.wmb3_batch_uids.truncate(wmb3.batches.len());
    }

    pub fn mesh_index_by_id(&self, id: &str) -> Option<usize> {
        let raw = id.trim().strip_prefix("mesh:").unwrap_or(id.trim());
        let uid: u64 = raw.parse().ok()?;
        self.mesh_uids.iter().position(|&u| u == uid)
    }

    pub fn material_index_by_id(&self, id: &str) -> Option<usize> {
        let raw = id.trim().strip_prefix("mat:").unwrap_or(id.trim());
        let uid: u64 = raw.parse().ok()?;
        self.material_uids.iter().position(|&u| u == uid)
    }

    pub fn wmb_batch_pos_by_id(&self, id: &str) -> Option<(usize, usize)> {
        let raw = id.trim().strip_prefix("batch:").unwrap_or(id.trim());
        let uid: u64 = raw.parse().ok()?;
        for (mi, row) in self.wmb_batch_uids.iter().enumerate() {
            if let Some(bi) = row.iter().position(|&u| u == uid) {
                return Some((mi, bi));
            }
        }
        None
    }

    pub fn wmb3_batch_index_by_id(&self, id: &str) -> Option<usize> {
        let raw = id.trim().strip_prefix("batch:").unwrap_or(id.trim());
        let uid: u64 = raw.parse().ok()?;
        self.wmb3_batch_uids.iter().position(|&u| u == uid)
    }

    /// Parses `"lod:<meshUid>/<level>"` or `"lod:mesh:<meshUid>/<level>"` into `(mesh_index, lod_level)`.
    pub fn parse_lod_id(&self, id: &str) -> Option<(usize, i32)> {
        let rest = id.trim().strip_prefix("lod:")?;
        let (mesh_part, level_part) = rest.rsplit_once('/')?;
        let level: i32 = level_part.parse().ok()?;
        let mesh_idx = self.mesh_index_by_id(mesh_part)?;
        Some((mesh_idx, level))
    }
}

pub struct Fnv1a64(u64);

impl Fnv1a64 {
    pub const OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
    pub const PRIME: u64 = 0x0000_0100_0000_01b3;

    pub fn new() -> Self {
        Self(Self::OFFSET_BASIS)
    }

    pub fn update(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 ^= b as u64;
            self.0 = self.0.wrapping_mul(Self::PRIME);
        }
    }

    pub fn finish_hex(&self) -> String {
        format!("{:016x}", self.0)
    }
}

impl Default for Fnv1a64 {
    fn default() -> Self {
        Self::new()
    }
}

pub fn fnv1a_64_hex(bytes: &[u8]) -> String {
    let mut h = Fnv1a64::new();
    h.update(bytes);
    h.finish_hex()
}

/// Maps sampler/slot names, DB roles, and pretexture labels into the canonical scene role set:
/// `"albedo" | "normal" | "spec" | "mask" | "env" | "lut" | "unknown"`.
pub fn classify_texture_role(
    slot_name: &str,
    db_role: Option<&str>,
    pretex_name: Option<&str>,
) -> String {
    fn match_one(s: &str) -> Option<&'static str> {
        let low = s.trim().to_ascii_lowercase();
        if low.is_empty() || low == "unknown" || low == "extra" || low == "texture" {
            return None;
        }
        if low.contains("normal") || low.contains("relief") || low.contains("bump") {
            return Some("normal");
        }
        if low.contains("lut")
            || low.contains("spec_pow")
            || low.contains("pre_pow")
            || low.contains("subsurface")
        {
            return Some("lut");
        }
        if low.contains("cube")
            || low.contains("env")
            || low.contains("irradiance")
            || low.contains("refhard")
            || low.contains("refsoft")
            || low.contains("spechard")
            || low.contains("specsoft")
            || low.contains("reflection")
        {
            return Some("env");
        }
        if low.contains("spec_mask") || low.contains("mask") || low.contains("lightmap") || low.contains("occlusion") || low.contains("blend") || low.contains("dither") || low.contains("alpha") || low.contains("opacity") {
            if low == "spec_mask" {
                return Some("spec");
            }
            return Some("mask");
        }
        if low.contains("spec") || low.contains("rss") {
            return Some("spec");
        }
        if low.contains("albedo")
            || low.contains("color")
            || low.contains("diffuse")
            || low.contains("base")
            || low.contains("white")
            || low.contains("black")
            || low.contains("grey")
            || low.contains("2ndmap")
            || low == "g_texture0"
        {
            return Some("albedo");
        }
        None
    }

    if let Some(r) = match_one(slot_name) {
        return r.to_string();
    }
    if let Some(dbr) = db_role {
        if let Some(r) = match_one(dbr) {
            return r.to_string();
        }
    }
    if let Some(pre) = pretex_name {
        if let Some(r) = match_one(pre) {
            return r.to_string();
        }
    }
    "unknown".to_string()
}

fn append_binary_region(binary: &mut Vec<u8>, bytes: &[u8], omit: bool) -> Region {
    let byte_len = bytes.len() as u32;
    if omit {
        return Region::omitted_region(byte_len);
    }
    let aligned = align(binary.len(), 4);
    binary.resize(aligned, 0);
    let offset = binary.len() as u32;
    binary.extend_from_slice(bytes);
    let end_aligned = align(binary.len(), 4);
    binary.resize(end_aligned, 0);
    Region::included(offset, byte_len)
}

fn compute_bbox_from_positions(positions: &[[f32; 3]]) -> [f32; 6] {
    if positions.is_empty() {
        return [0.0; 6];
    }
    let mut min_x = f32::INFINITY;
    let mut min_y = f32::INFINITY;
    let mut min_z = f32::INFINITY;
    let mut max_x = f32::NEG_INFINITY;
    let mut max_y = f32::NEG_INFINITY;
    let mut max_z = f32::NEG_INFINITY;
    for &[x, y, z] in positions {
        if x < min_x {
            min_x = x;
        }
        if y < min_y {
            min_y = y;
        }
        if z < min_z {
            min_z = z;
        }
        if x > max_x {
            max_x = x;
        }
        if y > max_y {
            max_y = y;
        }
        if z > max_z {
            max_z = z;
        }
    }
    if !min_x.is_finite() {
        [0.0; 6]
    } else {
        [min_x, min_y, min_z, max_x, max_y, max_z]
    }
}

/// Build the `WmbSceneManifest` and binary geometry region for a WMB0 / WMB1 / WMB2 model.
pub fn build_wmb_scene(
    wmb: &WmbFile,
    uid_map: &mut UidMap,
    node_id: &str,
    file_name: &str,
    structure_version: u64,
    mut bank_textures: Vec<TextureRefDesc>,
    known_geometry_hashes: &[String],
) -> (WmbSceneManifest, Vec<u8>) {
    uid_map.ensure_wmb(wmb);
    let is_b2 = wmb.is_bayo2();
    let game = if is_b2 { "b2" } else { "b1" };
    let known_set: BTreeSet<&str> = known_geometry_hashes.iter().map(|s| s.as_str()).collect();
    let mut binary = Vec::new();

    // 1. Build single global vertex buffer (index 0)
    let v_count = wmb.vertexes.len().max(wmb.positions.len());
    let layout_opt = lookup_wmb_vertex_layout(
        wmb.header.vertex_flags,
        wmb.header.vertex_ex_data_size,
        wmb.header.vertex_ex_data,
    );
    let has_uv1 = layout_opt
        .map(|l| {
            l.primary_fields
                .iter()
                .chain(l.ex_fields.iter())
                .any(|f| f.name == "mapping2")
        })
        .unwrap_or(false)
        || wmb.vertexes.iter().any(|v| v.mapping2.is_some());
    let has_color = layout_opt
        .map(|l| {
            l.primary_fields
                .iter()
                .chain(l.ex_fields.iter())
                .any(|f| f.field_type == VertexFieldType::Color)
        })
        .unwrap_or(false)
        || wmb.vertexes.iter().any(|v| v.color.is_some());

    let mut pos_bytes = Vec::with_capacity(v_count * 12);
    let mut nrm_bytes = Vec::with_capacity(v_count * 12);
    let mut tan_bytes = Vec::with_capacity(v_count * 16);
    let mut uv0_bytes = Vec::with_capacity(v_count * 8);
    let mut uv1_bytes = if has_uv1 {
        Vec::with_capacity(v_count * 8)
    } else {
        Vec::new()
    };
    let mut col_bytes = if has_color {
        Vec::with_capacity(v_count * 4)
    } else {
        Vec::new()
    };

    let mut all_positions = Vec::with_capacity(v_count);
    for i in 0..v_count {
        let v = wmb.vertexes.get(i);
        let p = v
            .and_then(|vx| vx.position)
            .or_else(|| wmb.positions.get(i).copied())
            .unwrap_or_default();
        all_positions.push([p.x, p.y, p.z]);
        pos_bytes.extend_from_slice(&p.x.to_le_bytes());
        pos_bytes.extend_from_slice(&p.y.to_le_bytes());
        pos_bytes.extend_from_slice(&p.z.to_le_bytes());

        let nrm = if let Some(vx) = v {
            if let Some(fnrm) = vx.float_normal {
                [fnrm.x, fnrm.y, fnrm.z]
            } else if let Some(hnrm) = vx.half_normal {
                [hnrm[0], hnrm[1], hnrm[2]]
            } else if vx.normal.normal == [0.0, 0.0, 0.0] && vx.normal.raw_bytes != [0, 0, 0, 0] {
                if wmb.endian.is_big() {
                    PackedNormal::unpack_wide(u32::from_be_bytes(vx.normal.raw_bytes))
                } else if is_b2 || vx.normal.wide {
                    PackedNormal::unpack_wide(u32::from_le_bytes(vx.normal.raw_bytes))
                } else {
                    PackedNormal::unpack_narrow(vx.normal.raw_bytes)
                }
            } else {
                vx.normal.normal
            }
        } else {
            [0.0, 0.0, 1.0]
        };
        nrm_bytes.extend_from_slice(&nrm[0].to_le_bytes());
        nrm_bytes.extend_from_slice(&nrm[1].to_le_bytes());
        nrm_bytes.extend_from_slice(&nrm[2].to_le_bytes());

        let tan = v
            .map(|vx| vx.tangents.to_xyzw())
            .unwrap_or([1.0, 0.0, 0.0, 1.0]);
        tan_bytes.extend_from_slice(&tan[0].to_le_bytes());
        tan_bytes.extend_from_slice(&tan[1].to_le_bytes());
        tan_bytes.extend_from_slice(&tan[2].to_le_bytes());
        tan_bytes.extend_from_slice(&tan[3].to_le_bytes());

        let uv0 = v.map(|vx| vx.mapping).unwrap_or([0.0, 0.0]);
        uv0_bytes.extend_from_slice(&uv0[0].to_le_bytes());
        uv0_bytes.extend_from_slice(&uv0[1].to_le_bytes());

        if has_uv1 {
            let uv1 = v.and_then(|vx| vx.mapping2).unwrap_or(uv0);
            uv1_bytes.extend_from_slice(&uv1[0].to_le_bytes());
            uv1_bytes.extend_from_slice(&uv1[1].to_le_bytes());
        }
        if has_color {
            let col = v.and_then(|vx| vx.color).unwrap_or([255, 255, 255, 255]);
            col_bytes.extend_from_slice(&col);
        }
    }

    let mut vb_hasher = Fnv1a64::new();
    vb_hasher.update(b"pos:");
    vb_hasher.update(&pos_bytes);
    vb_hasher.update(b"nrm:");
    vb_hasher.update(&nrm_bytes);
    vb_hasher.update(b"tan:");
    vb_hasher.update(&tan_bytes);
    vb_hasher.update(b"uv0:");
    vb_hasher.update(&uv0_bytes);
    if has_uv1 {
        vb_hasher.update(b"uv1:");
        vb_hasher.update(&uv1_bytes);
    }
    if has_color {
        vb_hasher.update(b"col:");
        vb_hasher.update(&col_bytes);
    }
    let vb_hash = vb_hasher.finish_hex();
    let omit_vb = known_set.contains(vb_hash.as_str());

    let pos_reg = append_binary_region(&mut binary, &pos_bytes, omit_vb);
    let nrm_reg = Some(append_binary_region(&mut binary, &nrm_bytes, omit_vb));
    let tan_reg = Some(append_binary_region(&mut binary, &tan_bytes, omit_vb));
    let uv0_reg = Some(append_binary_region(&mut binary, &uv0_bytes, omit_vb));
    let uv1_reg = if has_uv1 {
        Some(append_binary_region(&mut binary, &uv1_bytes, omit_vb))
    } else {
        None
    };
    let col_reg = if has_color {
        Some(append_binary_region(&mut binary, &col_bytes, omit_vb))
    } else {
        None
    };

    let vertex_buffers = vec![VertexBufferDesc {
        index: 0,
        vertex_count: v_count as u32,
        geometry_hash: vb_hash,
        position: pos_reg,
        normal: nrm_reg,
        tangent: tan_reg,
        uv0: uv0_reg,
        uv1: uv1_reg,
        uv2: None,
        color: col_reg,
    }];

    // 2. Build MeshGroups & Batches
    let mut mesh_groups = Vec::with_capacity(wmb.meshes.len());
    let mut batches = Vec::new();
    let mut mat_used_by_batches: BTreeMap<usize, Vec<String>> = BTreeMap::new();
    let mut non_shadow_positions = Vec::new();
    let mut global_batch_idx = 0usize;

    for (mi, mesh) in wmb.meshes.iter().enumerate() {
        let mesh_uid = uid_map.mesh_uids[mi];
        let mesh_id = format!("mesh:{mesh_uid}");
        let mut group_batch_ids = Vec::with_capacity(mesh.batches.len());

        for (bi, batch) in mesh.batches.iter().enumerate() {
            let batch_uid = uid_map.wmb_batch_uids[mi][bi];
            let batch_id = format!("batch:{batch_uid}");
            group_batch_ids.push(batch_id.clone());

            let raw_eff_mat = batch.header.effective_material_id(is_b2);
            let has_dangling_material =
                wmb.materials.is_empty() || raw_eff_mat >= wmb.materials.len();
            let mat_idx = raw_eff_mat.min(wmb.materials.len().saturating_sub(1));
            let mat_uid = uid_map.material_uids.get(mat_idx).copied().unwrap_or(0);
            let mat_id = format!("mat:{mat_uid}");
            mat_used_by_batches
                .entry(mat_idx)
                .or_default()
                .push(batch_id.clone());

            let is_shadow = batch.header.is_shadow_proxy(is_b2);
            let tris = batch.triangles();
            let mut idx_bytes = Vec::with_capacity(tris.len() * 12);
            let mut batch_positions = Vec::new();
            let mut unique_verts = BTreeSet::new();

            for [i0, i1, i2] in tris {
                if i0 == i1 || i1 == i2 || i0 == i2 {
                    continue;
                }
                if i0 >= v_count || i1 >= v_count || i2 >= v_count {
                    continue;
                }
                idx_bytes.extend_from_slice(&(i0 as u32).to_le_bytes());
                idx_bytes.extend_from_slice(&(i1 as u32).to_le_bytes());
                idx_bytes.extend_from_slice(&(i2 as u32).to_le_bytes());
                for v_idx in [i0, i1, i2] {
                    if unique_verts.insert(v_idx) {
                        if let Some(&p) = all_positions.get(v_idx) {
                            batch_positions.push(p);
                            if !is_shadow {
                                non_shadow_positions.push(p);
                            }
                        }
                    }
                }
            }

            let index_count = (idx_bytes.len() / 4) as u32;
            let vertex_count = unique_verts.len() as u32;
            let b_hash = fnv1a_64_hex(&idx_bytes);
            let omit_idx = known_set.contains(b_hash.as_str());
            let idx_reg = append_binary_region(&mut binary, &idx_bytes, omit_idx);
            let bbox = compute_bbox_from_positions(&batch_positions);

            batches.push(BatchDesc {
                id: batch_id,
                index: global_batch_idx,
                mesh_id: mesh_id.clone(),
                lod_id: None,
                file_order: global_batch_idx,
                material_index: mat_idx,
                material_id: mat_id,
                is_shadow_proxy: is_shadow,
                vertex_buffer_index: 0,
                indices: idx_reg,
                index_count,
                vertex_count,
                geometry_hash: b_hash,
                bbox,
                has_dangling_material,
                flags: Some(batch.header.flags),
                ex_mat_id: Some(batch.header.ex_mat_id),
                raw_material_id: Some(batch.header.material_id),
                has_bone_refs: Some(batch.header.has_bone_refs),
                u_e1: Some(batch.header.u_e1),
                u_e2: Some(batch.header.u_e2),
                primitive_type: Some(batch.header.primitive_type),
                vertex_start: Some(batch.header.vertex_start),
                vertex_end: Some(batch.header.vertex_end),
                vertex_offset: Some(batch.header.vertex_offset),
                u_f: Some(batch.header.u_f),
                bone_refs: Some(batch.bone_refs.clone()),
                unknown_no_bones: Some(batch.unknown_no_bones),
                bone_set_index: None,
                call_tree_node_index: None,
                mesh_material_index: None,
                u_b: None,
            });
            global_batch_idx += 1;
        }

        mesh_groups.push(MeshGroupDesc {
            id: mesh_id,
            index: mi,
            name: mesh.name.clone(),
            batch_ids: group_batch_ids,
            lods: Vec::new(),
        });
    }

    let model_bbox = if !non_shadow_positions.is_empty() {
        compute_bbox_from_positions(&non_shadow_positions)
    } else {
        compute_bbox_from_positions(&all_positions)
    };

    // 3. Build Materials & link with bank textures
    let core_pre = list_core_pretextures(game);
    let mut materials = Vec::with_capacity(wmb.materials.len());

    for (mi, mat) in wmb.materials.iter().enumerate() {
        let mat_uid = uid_map.material_uids[mi];
        let mat_id = format!("mat:{mat_uid}");
        let dec = decode_wmb_material_rich(mat, mi, is_b2);
        let mut texture_slots = Vec::new();
        let mut params = Vec::new();
        let mut first_thumb: Option<String> = None;

        for field in &dec.fields {
            if let Some(tid) = field.sampler_tex_id {
                let slot_idx = texture_slots.len();
                let is_b1_global = !is_b2 && (tid & 0x8000_0000) != 0;
                let pre_info = core_pre.iter().find(|p| p.id == tid);
                let db_role = field
                    .role
                    .as_deref()
                    .filter(|r| !r.trim().is_empty())
                    .or_else(|| field.description.as_deref())
                    .or_else(|| pre_info.map(|p| p.role.as_str()));
                let role = classify_texture_role(
                    &field.name,
                    db_role,
                    field.pretexture_name.as_deref(),
                );

                let matched_bank_idx = if !is_b2 && !is_b1_global {
                    bank_textures
                        .iter()
                        .position(|bt| bt.texture_id == tid)
                        .or_else(|| bank_textures.get(tid as usize).map(|_| tid as usize))
                } else {
                    bank_textures.iter().position(|bt| bt.texture_id == tid)
                };

                if let Some(b_idx) = matched_bank_idx {
                    if !bank_textures[b_idx].used_by_material_ids.contains(&mat_id) {
                        bank_textures[b_idx].used_by_material_ids.push(mat_id.clone());
                    }
                    if first_thumb.is_none() {
                        first_thumb = bank_textures[b_idx].thumbnail_data_url.clone();
                    }
                }

                let global = is_b1_global || (matched_bank_idx.is_none() && pre_info.is_some());
                let present = matched_bank_idx.is_some() || pre_info.is_some();

                texture_slots.push(TextureSlotDesc {
                    slot: slot_idx,
                    name: field.name.clone(),
                    role,
                    texture_id: tid,
                    texture_id_hex: format!("0x{tid:08X}"),
                    global,
                    present,
                });
            } else if let Some(vals) = field.float_values {
                params.push(MaterialParamDesc {
                    key: field.name.clone(),
                    name: field.name.clone(),
                    values: vals.to_vec(),
                    is_color: field.is_color,
                });
            }
        }

        materials.push(MaterialDesc {
            id: mat_id,
            index: mi,
            name: format!("Material {mi}"),
            game: game.to_string(),
            shader_name: dec.shader_name.clone(),
            material_type: Some(mat.mat_type),
            technique: None,
            texture_slots,
            params,
            used_by_batch_ids: mat_used_by_batches.remove(&mi).unwrap_or_default(),
            thumbnail_data_url: first_thumb,
        });
    }

    // 4. Build Bones
    let dumped_bones = wmb.dump_bones();
    let bones: Vec<BoneDesc> = dumped_bones
        .into_iter()
        .map(|b| {
            let gid = if b.global_index >= 0 {
                b.global_index as i32
            } else {
                b.index as i32
            };
            BoneDesc {
                global_id: gid,
                local_id: Some(b.index),
                parent: b.parent_index as i32,
                name: None,
                local_position: b.relative_position.to_array(),
                world_position: b.position.to_array(),
                rotation: None,
                scale: None,
            }
        })
        .collect();

    let manifest = WmbSceneManifest {
        schema: 1,
        structure_version,
        node_id: node_id.to_string(),
        file_name: file_name.to_string(),
        game: game.to_string(),
        is_wmb3: false,
        bounding_box: model_bbox,
        lod_levels: Vec::new(),
        mesh_groups,
        batches,
        materials,
        bones,
        textures: bank_textures,
        vertex_buffers,
    };

    (manifest, binary)
}

/// Build the `WmbSceneManifest` and binary geometry region for a WMB3 model.
pub fn build_wmb3_scene(
    wmb3: &Wmb3File,
    uid_map: &mut UidMap,
    node_id: &str,
    file_name: &str,
    structure_version: u64,
    mut bank_textures: Vec<TextureRefDesc>,
    known_geometry_hashes: &[String],
) -> (WmbSceneManifest, Vec<u8>) {
    uid_map.ensure_wmb3(wmb3);
    let known_set: BTreeSet<&str> = known_geometry_hashes.iter().map(|s| s.as_str()).collect();
    let mut binary = Vec::new();

    // 1. Build one VertexBufferDesc per Wmb3VertexGroup
    let mut vertex_buffers = Vec::with_capacity(wmb3.vertex_groups.len());
    let mut vg_positions: Vec<Vec<[f32; 3]>> = Vec::with_capacity(wmb3.vertex_groups.len());

    for (vgi, vg) in wmb3.vertex_groups.iter().enumerate() {
        let v_count = vg.vertexes.len();
        let layout_opt = lookup_wmb3_vertex_layout_with_strides(
            vg.vertex_flags,
            vg.vertex_size,
            vg.vertex_ex_data_size,
        );
        let has_uv1 = layout_opt
            .map(|l| {
                l.primary_fields
                    .iter()
                    .chain(l.ex_fields.iter())
                    .any(|f| f.name == "mapping2")
            })
            .unwrap_or(false)
            || vg.vertexes.iter().any(|v| v.mapping2.is_some());
        let has_uv2 = layout_opt
            .map(|l| {
                l.primary_fields
                    .iter()
                    .chain(l.ex_fields.iter())
                    .any(|f| f.name == "mapping3")
            })
            .unwrap_or(false)
            || vg.vertexes.iter().any(|v| v.mapping3.is_some());
        let has_color = layout_opt
            .map(|l| {
                l.primary_fields
                    .iter()
                    .chain(l.ex_fields.iter())
                    .any(|f| f.field_type == VertexFieldType::Color)
            })
            .unwrap_or(false)
            || vg.vertexes.iter().any(|v| v.color.is_some());

        let mut pos_bytes = Vec::with_capacity(v_count * 12);
        let mut nrm_bytes = Vec::with_capacity(v_count * 12);
        let mut tan_bytes = Vec::with_capacity(v_count * 16);
        let mut uv0_bytes = Vec::with_capacity(v_count * 8);
        let mut uv1_bytes = if has_uv1 {
            Vec::with_capacity(v_count * 8)
        } else {
            Vec::new()
        };
        let mut uv2_bytes = if has_uv2 {
            Vec::with_capacity(v_count * 8)
        } else {
            Vec::new()
        };
        let mut col_bytes = if has_color {
            Vec::with_capacity(v_count * 4)
        } else {
            Vec::new()
        };

        let mut positions_for_vg = Vec::with_capacity(v_count);
        for v in &vg.vertexes {
            let p = v.position.unwrap_or_default();
            positions_for_vg.push([p.x, p.y, p.z]);
            pos_bytes.extend_from_slice(&p.x.to_le_bytes());
            pos_bytes.extend_from_slice(&p.y.to_le_bytes());
            pos_bytes.extend_from_slice(&p.z.to_le_bytes());

            let nrm = if let Some(hn) = v.half_normal {
                [hn[0], hn[1], hn[2]]
            } else if let Some(fnrm) = v.float_normal {
                [fnrm.x, fnrm.y, fnrm.z]
            } else {
                v.normal.normal
            };
            nrm_bytes.extend_from_slice(&nrm[0].to_le_bytes());
            nrm_bytes.extend_from_slice(&nrm[1].to_le_bytes());
            nrm_bytes.extend_from_slice(&nrm[2].to_le_bytes());

            let tan = v.tangents.to_xyzw();
            tan_bytes.extend_from_slice(&tan[0].to_le_bytes());
            tan_bytes.extend_from_slice(&tan[1].to_le_bytes());
            tan_bytes.extend_from_slice(&tan[2].to_le_bytes());
            tan_bytes.extend_from_slice(&tan[3].to_le_bytes());

            let uv0 = sanitize_uv_pair(v.mapping);
            uv0_bytes.extend_from_slice(&uv0[0].to_le_bytes());
            uv0_bytes.extend_from_slice(&uv0[1].to_le_bytes());

            if has_uv1 {
                let uv1 = sanitize_uv_pair(v.mapping2.unwrap_or(uv0));
                uv1_bytes.extend_from_slice(&uv1[0].to_le_bytes());
                uv1_bytes.extend_from_slice(&uv1[1].to_le_bytes());
            }
            if has_uv2 {
                let uv2 = sanitize_uv_pair(v.mapping3.unwrap_or(uv0));
                uv2_bytes.extend_from_slice(&uv2[0].to_le_bytes());
                uv2_bytes.extend_from_slice(&uv2[1].to_le_bytes());
            }
            if has_color {
                let col = v.color.unwrap_or([255, 255, 255, 255]);
                col_bytes.extend_from_slice(&col);
            }
        }
        vg_positions.push(positions_for_vg);

        let mut vb_hasher = Fnv1a64::new();
        vb_hasher.update(b"pos:");
        vb_hasher.update(&pos_bytes);
        vb_hasher.update(b"nrm:");
        vb_hasher.update(&nrm_bytes);
        vb_hasher.update(b"tan:");
        vb_hasher.update(&tan_bytes);
        vb_hasher.update(b"uv0:");
        vb_hasher.update(&uv0_bytes);
        if has_uv1 {
            vb_hasher.update(b"uv1:");
            vb_hasher.update(&uv1_bytes);
        }
        if has_uv2 {
            vb_hasher.update(b"uv2:");
            vb_hasher.update(&uv2_bytes);
        }
        if has_color {
            vb_hasher.update(b"col:");
            vb_hasher.update(&col_bytes);
        }
        let vb_hash = vb_hasher.finish_hex();
        let omit_vb = known_set.contains(vb_hash.as_str());

        let pos_reg = append_binary_region(&mut binary, &pos_bytes, omit_vb);
        let nrm_reg = Some(append_binary_region(&mut binary, &nrm_bytes, omit_vb));
        let tan_reg = Some(append_binary_region(&mut binary, &tan_bytes, omit_vb));
        let uv0_reg = Some(append_binary_region(&mut binary, &uv0_bytes, omit_vb));
        let uv1_reg = if has_uv1 {
            Some(append_binary_region(&mut binary, &uv1_bytes, omit_vb))
        } else {
            None
        };
        let uv2_reg = if has_uv2 {
            Some(append_binary_region(&mut binary, &uv2_bytes, omit_vb))
        } else {
            None
        };
        let col_reg = if has_color {
            Some(append_binary_region(&mut binary, &col_bytes, omit_vb))
        } else {
            None
        };

        vertex_buffers.push(VertexBufferDesc {
            index: vgi,
            vertex_count: v_count as u32,
            geometry_hash: vb_hash,
            position: pos_reg,
            normal: nrm_reg,
            tangent: tan_reg,
            uv0: uv0_reg,
            uv1: uv1_reg,
            uv2: uv2_reg,
            color: col_reg,
        });
    }

    // 2. Derive MeshGroup -> LOD -> Batch mapping
    let mut lod_levels_set = BTreeSet::new();
    for lod in &wmb3.lods {
        lod_levels_set.insert(lod.lod_level);
    }
    let lod_levels: Vec<i32> = lod_levels_set.into_iter().collect();

    // Map each global batch index to (mesh_idx, mat_idx, dangling_mat, lod_level_and_name, call_tree_node_index, mesh_material_index, u_b)
    let mut batch_meta: Vec<(usize, usize, bool, Option<(i32, String)>, i32, u32, i32)> =
        vec![(0, 0, wmb3.materials.is_empty(), None, 0, 0, 0); wmb3.batches.len()];
    for lod in &wmb3.lods {
        let start = lod.batch_start as usize;
        for (i, inf) in lod.batch_infos.iter().enumerate() {
            let bi = start + i;
            if bi < batch_meta.len() {
                let m_idx = (inf.mesh_index as usize).min(wmb3.meshes.len().saturating_sub(1));
                let raw_mat_idx = inf.material_index as usize;
                let dangling = wmb3.materials.is_empty() || raw_mat_idx >= wmb3.materials.len();
                let mat_idx = raw_mat_idx.min(wmb3.materials.len().saturating_sub(1));
                batch_meta[bi] = (
                    m_idx,
                    mat_idx,
                    dangling,
                    Some((lod.lod_level, lod.name.clone())),
                    inf.call_tree_node_index,
                    inf.mesh_material_index,
                    inf.u_b,
                );
            }
        }
    }

    let mut mesh_lod_batches: BTreeMap<(usize, i32), Vec<String>> = BTreeMap::new();
    let mut mat_used_by_batches: BTreeMap<usize, Vec<String>> = BTreeMap::new();
    let mut batches = Vec::with_capacity(wmb3.batches.len());
    let mut all_model_positions = Vec::new();

    for (bi, b) in wmb3.batches.iter().enumerate() {
        let batch_uid = uid_map.wmb3_batch_uids[bi];
        let batch_id = format!("batch:{batch_uid}");
        let (
            mesh_idx,
            mat_idx,
            has_dangling_material,
            ref lod_opt,
            call_tree_node_index,
            mesh_material_index,
            u_b,
        ) = batch_meta[bi];
        let mesh_uid = uid_map.mesh_uids.get(mesh_idx).copied().unwrap_or(0);
        let mesh_id = format!("mesh:{mesh_uid}");
        let mat_uid = uid_map.material_uids.get(mat_idx).copied().unwrap_or(0);
        let mat_id = format!("mat:{mat_uid}");

        let lod_id = if let Some((lvl, _)) = lod_opt {
            mesh_lod_batches
                .entry((mesh_idx, *lvl))
                .or_default()
                .push(batch_id.clone());
            Some(format!("lod:{mesh_uid}/{lvl}"))
        } else {
            None
        };

        mat_used_by_batches
            .entry(mat_idx)
            .or_default()
            .push(batch_id.clone());

        let vgi = (b.vertex_group_index as usize).min(wmb3.vertex_groups.len().saturating_sub(1));
        let mut idx_bytes = Vec::new();
        let mut batch_positions = Vec::new();
        let mut unique_verts = BTreeSet::new();

        if let Some(vg) = wmb3.vertex_groups.get(vgi) {
            let v_len = vg.vertexes.len();
            let start = (b.index_start as usize).min(vg.indices.len());
            let end = (start + b.num_indices as usize).min(vg.indices.len());
            for tri in vg.indices[start..end].chunks_exact(3) {
                let resolve_vi = |raw: u32| -> usize {
                    let with_start = (raw as usize).saturating_add(b.vertex_start as usize);
                    if with_start < v_len {
                        with_start
                    } else {
                        raw as usize
                    }
                };
                let i0 = resolve_vi(tri[0]);
                let i1 = resolve_vi(tri[1]);
                let i2 = resolve_vi(tri[2]);
                if i0 == i1 || i1 == i2 || i0 == i2 {
                    continue;
                }
                if i0 >= v_len || i1 >= v_len || i2 >= v_len {
                    continue;
                }
                idx_bytes.extend_from_slice(&(i0 as u32).to_le_bytes());
                idx_bytes.extend_from_slice(&(i1 as u32).to_le_bytes());
                idx_bytes.extend_from_slice(&(i2 as u32).to_le_bytes());
                for v_idx in [i0, i1, i2] {
                    if unique_verts.insert(v_idx) {
                        if let Some(&p) = vg_positions.get(vgi).and_then(|vp| vp.get(v_idx)) {
                            batch_positions.push(p);
                            all_model_positions.push(p);
                        }
                    }
                }
            }
        }

        let index_count = (idx_bytes.len() / 4) as u32;
        let vertex_count = unique_verts.len() as u32;
        let b_hash = fnv1a_64_hex(&idx_bytes);
        let omit_idx = known_set.contains(b_hash.as_str());
        let idx_reg = append_binary_region(&mut binary, &idx_bytes, omit_idx);
        let bbox = compute_bbox_from_positions(&batch_positions);

        batches.push(BatchDesc {
            id: batch_id,
            index: bi,
            mesh_id,
            lod_id,
            file_order: bi,
            material_index: mat_idx,
            material_id: mat_id,
            is_shadow_proxy: false,
            vertex_buffer_index: vgi,
            indices: idx_reg,
            index_count,
            vertex_count,
            geometry_hash: b_hash,
            bbox,
            has_dangling_material,
            flags: None,
            ex_mat_id: None,
            raw_material_id: None,
            has_bone_refs: None,
            u_e1: None,
            u_e2: None,
            primitive_type: None,
            vertex_start: Some(b.vertex_start),
            vertex_end: None,
            vertex_offset: None,
            u_f: None,
            bone_refs: None,
            unknown_no_bones: None,
            bone_set_index: Some(b.bone_set_index),
            call_tree_node_index: Some(call_tree_node_index),
            mesh_material_index: Some(mesh_material_index),
            u_b: Some(u_b),
        });
    }

    let mut mesh_groups = Vec::with_capacity(wmb3.meshes.len());
    for (mi, mesh) in wmb3.meshes.iter().enumerate() {
        let mesh_uid = uid_map.mesh_uids[mi];
        let mesh_id = format!("mesh:{mesh_uid}");
        let mut lod_descs = Vec::with_capacity(wmb3.lods.len());
        for lod in &wmb3.lods {
            let b_ids = mesh_lod_batches
                .remove(&(mi, lod.lod_level))
                .unwrap_or_default();
            lod_descs.push(LodDesc {
                id: format!("lod:{mesh_uid}/{}", lod.lod_level),
                name: lod.name.clone(),
                level: lod.lod_level,
                batch_ids: b_ids,
            });
        }
        lod_descs.sort_by_key(|l| l.level);
        mesh_groups.push(MeshGroupDesc {
            id: mesh_id,
            index: mi,
            name: mesh.name.clone(),
            batch_ids: Vec::new(),
            lods: lod_descs,
        });
    }

    let model_bbox = if wmb3.bounding_box != [0.0; 6] {
        wmb3.bounding_box
    } else {
        compute_bbox_from_positions(&all_model_positions)
    };

    // 3. Build Materials
    let core_pre = list_core_pretextures("b3");
    let mut materials = Vec::with_capacity(wmb3.materials.len());

    for (mi, mat) in wmb3.materials.iter().enumerate() {
        let mat_uid = uid_map.material_uids[mi];
        let mat_id = format!("mat:{mat_uid}");
        let tex_views = mat.texture_views();
        let named_params = mat.named_parameters();
        let mut texture_slots = Vec::with_capacity(tex_views.len());
        let mut first_thumb: Option<String> = None;

        for tv in tex_views {
            let pre_info = core_pre.iter().find(|p| p.id == tv.texture_id);
            let role = classify_texture_role(
                &tv.name,
                pre_info.map(|p| p.role.as_str()),
                tv.pretexture_name.as_deref(),
            );
            let matched_bank_idx = bank_textures
                .iter()
                .position(|bt| bt.texture_id == tv.texture_id);
            if let Some(b_idx) = matched_bank_idx {
                if !bank_textures[b_idx].used_by_material_ids.contains(&mat_id) {
                    bank_textures[b_idx].used_by_material_ids.push(mat_id.clone());
                }
                if first_thumb.is_none() {
                    first_thumb = bank_textures[b_idx].thumbnail_data_url.clone();
                }
            }
            let global = matched_bank_idx.is_none() && pre_info.is_some();
            let present = matched_bank_idx.is_some() || pre_info.is_some();
            texture_slots.push(TextureSlotDesc {
                slot: tv.slot_index,
                name: tv.name,
                role,
                texture_id: tv.texture_id,
                texture_id_hex: tv.texture_id_hex,
                global,
                present,
            });
        }

        let mut params = Vec::with_capacity(named_params.len() + mat.variables.len());
        for np in named_params {
            let n_low = np.name.to_ascii_lowercase();
            let is_color = n_low.contains("color") || n_low.contains("tint");
            params.push(MaterialParamDesc {
                key: np.key,
                name: np.name,
                values: vec![np.value],
                is_color,
            });
        }
        for (vi, (vname, vval)) in mat.variables.iter().enumerate() {
            let n_low = vname.to_ascii_lowercase();
            params.push(MaterialParamDesc {
                key: format!("var_{vi}_{vname}"),
                name: vname.clone(),
                values: vec![*vval],
                is_color: n_low.contains("color") || n_low.contains("tint"),
            });
        }

        materials.push(MaterialDesc {
            id: mat_id,
            index: mi,
            name: mat.name.clone(),
            game: "b3".to_string(),
            shader_name: Some(mat.shader_name.clone()),
            material_type: None,
            technique: Some(mat.technique_name.clone()),
            texture_slots,
            params,
            used_by_batch_ids: mat_used_by_batches.remove(&mi).unwrap_or_default(),
            thumbnail_data_url: first_thumb,
        });
    }

    // 4. Build Bones
    let inv_btt = wmb3.bone_index_translate_table.invert();
    let bones: Vec<BoneDesc> = wmb3
        .bones
        .iter()
        .enumerate()
        .map(|(bi, b)| {
            let local_id = wmb3
                .bone_map
                .iter()
                .position(|&bm| bm as usize == bi)
                .or_else(|| Some(bi));
            let global_id = if b.id >= 0 {
                b.id as i32
            } else {
                inv_btt
                    .get(&(bi as u16))
                    .map(|&g| g as i32)
                    .unwrap_or(bi as i32)
            };
            BoneDesc {
                global_id,
                local_id,
                parent: b.parent_index as i32,
                name: None,
                local_position: b.local_position.to_array(),
                world_position: b.position.to_array(),
                rotation: Some(b.rotation.to_array()),
                scale: Some(b.scale.to_array()),
            }
        })
        .collect();

    let manifest = WmbSceneManifest {
        schema: 1,
        structure_version,
        node_id: node_id.to_string(),
        file_name: file_name.to_string(),
        game: "b3".to_string(),
        is_wmb3: true,
        bounding_box: model_bbox,
        lod_levels,
        mesh_groups,
        batches,
        materials,
        bones,
        textures: bank_textures,
        vertex_buffers,
    };

    (manifest, binary)
}

/// Pack `WmbSceneManifest` + `binary_region` into the Appendix E.1 wire format:
/// `u32 LE manifest_byte_length | UTF-8 JSON | 0-padding to 4B | binary_region`.
pub fn pack_scene_blob(manifest: &WmbSceneManifest, binary_region: &[u8]) -> Result<Vec<u8>> {
    let json_bytes =
        serde_json::to_vec(manifest).map_err(|e| PgError::Serialization(e.to_string()))?;
    let json_len = json_bytes.len() as u32;
    let padded_header_and_json = align(4 + json_bytes.len(), 4);
    let mut out = Vec::with_capacity(padded_header_and_json + binary_region.len());
    out.extend_from_slice(&json_len.to_le_bytes());
    out.extend_from_slice(&json_bytes);
    out.resize(padded_header_and_json, 0);
    out.extend_from_slice(binary_region);
    Ok(out)
}

/// Unpack a wire blob produced by `pack_scene_blob` back into `(WmbSceneManifest, Vec<u8>)`.
pub fn unpack_scene_blob(blob: &[u8]) -> Result<(WmbSceneManifest, Vec<u8>)> {
    if blob.len() < 4 {
        return Err(PgError::InvalidData("Scene blob shorter than 4 bytes".into()));
    }
    let json_len = u32::from_le_bytes([blob[0], blob[1], blob[2], blob[3]]) as usize;
    if 4 + json_len > blob.len() {
        return Err(PgError::InvalidData("Scene blob truncated inside manifest JSON".into()));
    }
    let manifest: WmbSceneManifest = serde_json::from_slice(&blob[4..4 + json_len])
        .map_err(|e| PgError::Serialization(e.to_string()))?;
    let bin_start = align(4 + json_len, 4).min(blob.len());
    let binary = blob[bin_start..].to_vec();
    Ok((manifest, binary))
}

fn downscale_rgba_if_needed(img: RgbaImage, max_dim: u32) -> RgbaImage {
    if max_dim > 0 && (img.width() > max_dim || img.height() > max_dim) {
        image::imageops::thumbnail(&img, max_dim, max_dim)
    } else {
        img
    }
}

/// Decode a 2D or cubemap texture blob into `(width, height, is_cube, rgba_bytes)` for `WTX1`.
pub fn decode_texture_for_wtx1(
    blob: &[u8],
    bntx_sub_index: usize,
    flags_hint: u32,
    max_dim: u32,
) -> Result<(u32, u32, bool, Vec<u8>)> {
    if blob.len() < 16 {
        return Err(PgError::InvalidData("Texture blob too short".into()));
    }

    // Check for cubemap in XT1
    if &blob[0..4] == b"XT1\0" && blob.len() >= 0x38 {
        let mut r = BinReader::new(blob, Endian::Little);
        r.seek(0x08)?;
        let stored_image_size = r.read_u64()?;
        let header_size = r.read_u32()? as usize;
        let mip_word = r.read_u32()?;
        let mip_count = (mip_word & 0xFFFF).max(1);
        let tex_type = r.read_u32()?;
        let xt1_fmt = r.read_u32()?;
        let width = r.read_u32()?;
        let height = r.read_u32()?;
        let depth = r.read_u32()?.max(1);
        if width == 0 || height == 0 || width > 16384 || height > 16384 {
            return Err(PgError::InvalidData(format!(
                "Invalid XT1 texture dimensions: {width}x{height}"
            )));
        }
        let _tex_flag = r.read_u32()?;
        let layout_word = r.read_u32()?;
        let format = TextureFormat::from_xt1_code(xt1_fmt);
        let layout = compute_xt1_layout(
            width,
            height,
            depth,
            mip_count,
            tex_type,
            layout_word,
            format,
            stored_image_size,
        );
        let is_cube = layout.face_count == 6 || (flags_hint & 0x0000_0200) != 0;
        if is_cube && !layout.mips.is_empty() {
            let mip0 = &layout.mips[0];
            let payload = if header_size <= blob.len() {
                &blob[header_size..]
            } else {
                &[]
            };
            let bpb = format.bytes_per_block();
            let linear_len = mip0
                .width_blocks
                .checked_mul(mip0.height_blocks)
                .and_then(|n| n.checked_mul(bpb))
                .ok_or_else(|| PgError::InvalidData("XT1 cubemap block size overflow".into()))?;
            if linear_len > MAX_ARCHIVE_ALLOC_BYTES / 6
                || mip0.slice_surface_size > MAX_ARCHIVE_ALLOC_BYTES / 6
            {
                return Err(PgError::InvalidData(
                    "XT1 cubemap surface size exceeds safety limit".into(),
                ));
            }
            let mut faces_rgba = Vec::new();
            let mut out_w = mip0.width;
            let mut out_h = mip0.height;
            for face in 0..6usize {
                let face_base = face.saturating_mul(layout.face_stride) + mip0.offset;
                let mut linear = vec![0u8; linear_len];
                if face_base < payload.len() {
                    let avail = (payload.len() - face_base).min(mip0.slice_surface_size);
                    let mut surface = vec![0u8; mip0.slice_surface_size];
                    surface[..avail].copy_from_slice(&payload[face_base..face_base + avail]);
                    gob_swizzle::deswizzle_volume(
                        &surface,
                        &mut linear,
                        mip0.width_blocks,
                        mip0.height_blocks,
                        1,
                        bpb,
                        mip0.block_height,
                        1,
                        mip0.pitch,
                    );
                }
                let decoded =
                    decode_linear_blocks_to_rgba8(&linear, mip0.width, mip0.height, format)?;
                let scaled = downscale_rgba_if_needed(decoded, max_dim);
                out_w = scaled.width();
                out_h = scaled.height();
                faces_rgba.extend_from_slice(scaled.as_raw());
            }
            return Ok((out_w, out_h, true, faces_rgba));
        }
    }

    // Check for cubemap in DDS (`DDSCAPS2_CUBEMAP = 0x200` at offset 112 or DX10 `miscFlag & 0x4` at offset 136)
    if &blob[0..4] == b"DDS " && blob.len() >= 128 {
        let caps2 = u32::from_le_bytes([blob[112], blob[113], blob[114], blob[115]]);
        let dx10_cube = blob.len() >= 148
            && &blob[84..88] == b"DX10"
            && (u32::from_le_bytes([blob[136], blob[137], blob[138], blob[139]]) & 0x4) != 0;
        if (caps2 & 0x200) != 0 || dx10_cube {
            let meta = inspect_texture_blob(blob)?;
            let header_size = if blob.len() >= 148 && &blob[84..88] == b"DX10" {
                148
            } else {
                128
            };
            let payload = &blob[header_size..];
            let (bw, bh) = meta.format.block_dimensions();
            let bpb = meta.format.bytes_per_block();
            let mut face_stride = 0usize;
            let mut cw = meta.width.max(1);
            let mut ch = meta.height.max(1);
            for _ in 0..meta.mip_count.max(1) {
                let bx = ((cw as usize) + bw - 1) / bw;
                let by = ((ch as usize) + bh - 1) / bh;
                face_stride += bx.max(1) * by.max(1) * bpb;
                cw = (cw / 2).max(1);
                ch = (ch / 2).max(1);
            }
            let mip0_bx = ((meta.width.max(1) as usize) + bw - 1) / bw;
            let mip0_by = ((meta.height.max(1) as usize) + bh - 1) / bh;
            let mip0_bytes = mip0_bx.max(1) * mip0_by.max(1) * bpb;

            let mut faces_rgba = Vec::new();
            let mut out_w = meta.width.max(1);
            let mut out_h = meta.height.max(1);
            if face_stride == 0 || payload.len() < 6usize.saturating_mul(face_stride) {
                return Err(PgError::InvalidData(format!(
                    "Truncated DDS cubemap payload: expected at least {} bytes for 6 faces, got {}",
                    6usize.saturating_mul(face_stride),
                    payload.len()
                )));
            }
            for face in 0..6usize {
                let face_off = face * face_stride;
                let end = face_off + mip0_bytes;
                if end > payload.len() {
                    return Err(PgError::InvalidData(format!(
                        "Truncated DDS cubemap face #{face}: need {end} bytes, got {}",
                        payload.len()
                    )));
                }
                let slice = &payload[face_off..end];
                let decoded =
                    decode_linear_blocks_to_rgba8(slice, meta.width.max(1), meta.height.max(1), meta.format)?;
                let scaled = downscale_rgba_if_needed(decoded, max_dim);
                out_w = scaled.width();
                out_h = scaled.height();
                faces_rgba.extend_from_slice(scaled.as_raw());
            }
            return Ok((out_w, out_h, true, faces_rgba));
        }
    }

    let is_bntx = (blob.len() >= 4 && &blob[0..4] == b"BNTX")
        || (blob.len() >= 84 && &blob[80..84] == b"BNTX");
    let (img, _, _) = if is_bntx {
        decode_bntx_subtexture_mip_filtered(blob, bntx_sub_index, 0, "RGBA", false)?
    } else {
        decode_texture_mip_filtered(blob, 0, "RGBA", false)?
    };
    let scaled = downscale_rgba_if_needed(img, max_dim);
    let w = scaled.width();
    let h = scaled.height();
    Ok((w, h, false, scaled.into_raw()))
}

/// Encode RGBA8 pixels into the Appendix E.3 `WTX1` binary response format.
pub fn encode_wtx1_blob(width: u32, height: u32, is_cube: bool, rgba: &[u8]) -> Vec<u8> {
    let flags: u32 = if is_cube { 1 } else { 0 };
    let mut out = Vec::with_capacity(16 + rgba.len());
    out.extend_from_slice(&WTX1_MAGIC.to_le_bytes());
    out.extend_from_slice(&width.to_le_bytes());
    out.extend_from_slice(&height.to_le_bytes());
    out.extend_from_slice(&flags.to_le_bytes());
    out.extend_from_slice(rgba);
    out
}
