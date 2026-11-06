//! Scene mutations (`move_items`, `delete_items`, `duplicate_items`, `reorder_mesh_groups`,
//! `mesh_group_*`, `update_wmb3_lod`, `material_*`, `paste_items`), shadow-last enforcement,
//! WMB3 cross-vertex-group vertex re-encoding, and cross-document clipboard operations.

use crate::bone::Vec3;
use crate::databases::{
    lookup_wmb3_vertex_layout_with_strides, lookup_wmb_vertex_layout, Wmb3VertexLayout,
    WmbVertexLayout,
};
use crate::io::{BinReader, BinWriter, Endian};
use crate::scene::UidMap;
use crate::shader_db::{
    decode_wmb_material_rich, list_core_pretextures, lookup_b3_shader_params,
    switch_wmb3_material_shader, switch_wmb_material_shader,
};
use crate::wmb::{BoneInfos, WmbBatch, WmbBatchHeader, WmbFile, WmbMaterial, WmbMesh, WmbVertex};
use crate::wmb3::{Wmb3Batch, Wmb3File, Wmb3LodBatchInfo, Wmb3Material, Wmb3Mesh};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClipboardStatus {
    pub game: String,
    pub kind: String, // "batches" | "materials"
    pub count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClipboardVertex {
    pub vertex: WmbVertex,
    /// Active `(global_bone_id, weight)` pairs resolved at copy time.
    pub global_bone_weights: Vec<(i32, u8)>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClipboardBatchItem {
    pub vertices: Vec<ClipboardVertex>,
    /// Local 0-based triangle indices into `vertices`.
    pub triangles: Vec<[u32; 3]>,
    /// Index into `WmbClipboardData::materials_wmb` or `materials_wmb3`.
    pub material_clipboard_idx: usize,
    pub is_shadow_proxy: bool,
    pub wmb_header_template: Option<WmbBatchHeader>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WmbClipboardData {
    pub game: String,
    pub kind: String, // "batches" | "materials"
    pub batches: Vec<ClipboardBatchItem>,
    pub materials_wmb: Vec<WmbMaterial>,
    pub materials_wmb3: Vec<Wmb3Material>,
}

impl WmbClipboardData {
    pub fn status(&self) -> ClipboardStatus {
        let count = if self.kind == "materials" {
            self.materials_wmb.len() + self.materials_wmb3.len()
        } else {
            self.batches.len()
        };
        ClipboardStatus {
            game: self.game.clone(),
            kind: self.kind.clone(),
            count,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MutationOutcome {
    pub description: String,
    pub warnings: Vec<String>,
    pub structural: bool,
}

impl MutationOutcome {
    pub fn new(description: impl Into<String>, structural: bool) -> Self {
        Self {
            description: description.into(),
            warnings: Vec::new(),
            structural,
        }
    }

    pub fn with_warnings(
        mut description: String,
        warnings: Vec<String>,
        structural: bool,
    ) -> Self {
        if !warnings.is_empty() {
            description = format!("{description} [Warning: {}]", warnings.join("; "));
        }
        Self {
            description,
            warnings,
            structural,
        }
    }
}

/// Stable partition of every mesh group in a WMB0/2 file so non-shadow batches come first
/// and shadow-proxy batches always sit at the bottom of their mesh group, keeping `uid_map` in sync.
pub fn enforce_shadow_last(wmb: &mut WmbFile, uid_map: &mut UidMap) {
    uid_map.ensure_wmb(wmb);
    let is_b2 = wmb.is_bayo2();
    for (mi, mesh) in wmb.meshes.iter_mut().enumerate() {
        let uids = &mut uid_map.wmb_batch_uids[mi];
        let mut non_shadow = Vec::new();
        let mut shadow = Vec::new();
        for (batch, uid) in mesh.batches.drain(..).zip(uids.drain(..)) {
            if batch.header.is_shadow_proxy(is_b2) {
                shadow.push((batch, uid));
            } else {
                non_shadow.push((batch, uid));
            }
        }
        for (b, u) in non_shadow.into_iter().chain(shadow.into_iter()) {
            mesh.batches.push(b);
            uids.push(u);
        }
    }
    reindex_wmb_batches_and_meshes(wmb);
}

/// Recompute `mesh.id`, `mesh.offset_batch_offsets`, `batch.header.mesh_id`, `batch.header.batch_id`,
/// and `batch.header.offset_indices` across all meshes and batches in a WMB0/2 file.
pub fn reindex_wmb_batches_and_meshes(wmb: &mut WmbFile) {
    wmb.header.num_meshes = wmb.meshes.len() as i32;
    wmb.header.num_materials = wmb.materials.len() as i32;
    wmb.header.num_vertexes = wmb.vertexes.len() as i32;
    for (mi, mesh) in wmb.meshes.iter_mut().enumerate() {
        mesh.id = mi as i16;
        if mesh.offset_batch_offsets < 0x80 {
            mesh.offset_batch_offsets = 0x80;
        }
        for (bi, batch) in mesh.batches.iter_mut().enumerate() {
            batch.header.mesh_id = mi as i16;
            batch.header.batch_id = bi as i16;
            batch.header.num_indices = batch.indices.len() as i32;
            batch.header.offset_indices = batch.effective_indices_offset();
        }
    }
}

/// Synchronize WMB3 `batch_start`, `meshes[mi].materials`, `mesh_material_pairs`, and `mesh_material_index`.
pub fn sync_wmb3_invariants(wmb3: &mut Wmb3File) {
    for vg in &mut wmb3.vertex_groups {
        vg.num_vertexes = vg.vertexes.len() as u32;
        vg.num_indices = vg.indices.len() as u32;
    }

    // Track which materials are actively referenced by batches in each mesh group (B-25)
    let mut active_mats_per_mesh: Vec<Vec<u16>> = vec![Vec::new(); wmb3.meshes.len()];
    let mut mesh_has_batches: Vec<bool> = vec![false; wmb3.meshes.len()];
    for lod in &mut wmb3.lods {
        for inf in &mut lod.batch_infos {
            if !wmb3.meshes.is_empty() && (inf.mesh_index as usize) >= wmb3.meshes.len() {
                inf.mesh_index = (wmb3.meshes.len() - 1) as u32;
            }
            if !wmb3.materials.is_empty() && (inf.material_index as usize) >= wmb3.materials.len() {
                inf.material_index = (wmb3.materials.len() - 1) as u32;
            }
            let mi = inf.mesh_index as usize;
            if mi < wmb3.meshes.len() {
                mesh_has_batches[mi] = true;
                let mat_u16 = inf.material_index as u16;
                if (mat_u16 as usize) < wmb3.materials.len()
                    && !active_mats_per_mesh[mi].contains(&mat_u16)
                {
                    active_mats_per_mesh[mi].push(mat_u16);
                }
            }
        }
    }

    let mut new_pairs: Vec<(u32, u32)> = Vec::new();
    for (mi, mesh) in wmb3.meshes.iter_mut().enumerate() {
        if mesh_has_batches[mi] {
            // Keep existing order for materials that are still referenced, then append any newly referenced ones
            let mut pruned = Vec::new();
            for &mat in &mesh.materials {
                if active_mats_per_mesh[mi].contains(&mat) && !pruned.contains(&mat) {
                    pruned.push(mat);
                }
            }
            for &mat in &active_mats_per_mesh[mi] {
                if !pruned.contains(&mat) {
                    pruned.push(mat);
                }
            }
            mesh.materials = pruned;
        } else {
            mesh.materials
                .retain(|&mat| (mat as usize) < wmb3.materials.len());
            let mut dedup = Vec::new();
            for &mat in &mesh.materials {
                if !dedup.contains(&mat) {
                    dedup.push(mat);
                }
            }
            mesh.materials = dedup;
        }
        for &mat in &mesh.materials {
            let pair = (mi as u32, mat as u32);
            if !new_pairs.contains(&pair) {
                new_pairs.push(pair);
            }
        }
    }

    let mut running_batch_start = 0u32;
    for lod in &mut wmb3.lods {
        lod.batch_start = running_batch_start;
        for (i, inf) in lod.batch_infos.iter_mut().enumerate() {
            let global_bi = (running_batch_start as usize) + i;
            if let Some(b) = wmb3.batches.get(global_bi) {
                inf.vertex_group_index = b.vertex_group_index;
            }
            let pair = (inf.mesh_index, inf.material_index);
            let pair_idx = if let Some(pos) = new_pairs.iter().position(|&p| p == pair) {
                pos
            } else {
                new_pairs.push(pair);
                new_pairs.len() - 1
            };
            inf.mesh_material_index = pair_idx as u32;
        }
        running_batch_start += lod.batch_infos.len() as u32;
    }

    wmb3.mesh_material_pairs = new_pairs;
}

fn roundtrip_vertex_wmb(v: &WmbVertex, layout: &WmbVertexLayout, is_b2: bool, endian: Endian) -> WmbVertex {
    let mut pw = BinWriter::new(endian);
    for f in layout.primary_fields {
        v.write_field(&mut pw, f.name, f.field_type, is_b2);
    }
    let mut exw = BinWriter::new(endian);
    for f in layout.ex_fields {
        v.write_field(&mut exw, f.name, f.field_type, is_b2);
    }
    let p_bytes = pw.into_inner();
    let ex_bytes = exw.into_inner();
    let mut out = WmbVertex::default();
    out.position = v.position;
    let mut pr = BinReader::new(&p_bytes, endian);
    for f in layout.primary_fields {
        let _ = out.read_field(&mut pr, f.name, f.field_type, is_b2);
    }
    let mut exr = BinReader::new(&ex_bytes, endian);
    for f in layout.ex_fields {
        let _ = out.read_field(&mut exr, f.name, f.field_type, is_b2);
    }
    out
}

fn roundtrip_vertex_wmb3(v: &WmbVertex, layout: &Wmb3VertexLayout) -> WmbVertex {
    let mut pw = BinWriter::new(Endian::Little);
    for f in layout.primary_fields {
        v.write_field(&mut pw, f.name, f.field_type, true);
    }
    let mut exw = BinWriter::new(Endian::Little);
    for f in layout.ex_fields {
        v.write_field(&mut exw, f.name, f.field_type, true);
    }
    let p_bytes = pw.into_inner();
    let ex_bytes = exw.into_inner();
    let mut out = WmbVertex::default();
    let mut pr = BinReader::new(&p_bytes, Endian::Little);
    for f in layout.primary_fields {
        let _ = out.read_field(&mut pr, f.name, f.field_type, true);
    }
    let mut exr = BinReader::new(&ex_bytes, Endian::Little);
    for f in layout.ex_fields {
        let _ = out.read_field(&mut exr, f.name, f.field_type, true);
    }
    out
}

fn wmb3_layout_has_field(layout: &Wmb3VertexLayout, name: &str) -> bool {
    layout
        .primary_fields
        .iter()
        .chain(layout.ex_fields.iter())
        .any(|f| f.name == name)
}

/// Re-encode a WMB3 batch from its current vertex group into `dst_vgi`, appending re-encoded
/// vertices and indices to `wmb3.vertex_groups[dst_vgi]`, updating `batch` and `info`, and
/// returning any dropped attribute names.
fn reencode_wmb3_batch_to_vg(
    wmb3: &mut Wmb3File,
    batch: &mut Wmb3Batch,
    info: &mut Wmb3LodBatchInfo,
    dst_vgi: usize,
) -> std::result::Result<Vec<String>, String> {
    let src_vgi = batch.vertex_group_index as usize;
    if src_vgi == dst_vgi {
        return Ok(Vec::new());
    }
    let src_vg = wmb3
        .vertex_groups
        .get(src_vgi)
        .ok_or_else(|| format!("Source vertex group #{src_vgi} out of bounds"))?;
    let dst_vg_meta = wmb3
        .vertex_groups
        .get(dst_vgi)
        .ok_or_else(|| format!("Target vertex group #{dst_vgi} out of bounds"))?;

    let dst_layout = lookup_wmb3_vertex_layout_with_strides(
        dst_vg_meta.vertex_flags,
        dst_vg_meta.vertex_size,
        dst_vg_meta.vertex_ex_data_size,
    )
    .ok_or_else(|| {
        format!(
            "Cannot re-encode batch into vertex group #{dst_vgi}: unknown layout (flags={}, size={}, ex={})",
            dst_vg_meta.vertex_flags, dst_vg_meta.vertex_size, dst_vg_meta.vertex_ex_data_size
        )
    })?;

    let src_layout_opt = lookup_wmb3_vertex_layout_with_strides(
        src_vg.vertex_flags,
        src_vg.vertex_size,
        src_vg.vertex_ex_data_size,
    );

    let v_len = src_vg.vertexes.len();
    let idx_start = (batch.index_start as usize).min(src_vg.indices.len());
    let idx_end = (idx_start + batch.num_indices as usize).min(src_vg.indices.len());
    let raw_slice = &src_vg.indices[idx_start..idx_end];

    let mut old_to_new: BTreeMap<usize, u32> = BTreeMap::new();
    let mut new_verts: Vec<WmbVertex> = Vec::new();
    let mut new_local_indices: Vec<u32> = Vec::with_capacity(raw_slice.len());

    for &raw_idx in raw_slice {
        let abs_vi = {
            let with_start = (raw_idx as usize).saturating_add(batch.vertex_start as usize);
            if with_start < v_len {
                with_start
            } else {
                raw_idx as usize
            }
        };
        if abs_vi >= v_len {
            return Err(format!(
                "Batch references vertex {abs_vi} out of bounds (len {v_len})"
            ));
        }
        let local_idx = if let Some(&existing) = old_to_new.get(&abs_vi) {
            existing
        } else {
            let next = new_verts.len() as u32;
            old_to_new.insert(abs_vi, next);
            let reencoded = roundtrip_vertex_wmb3(&src_vg.vertexes[abs_vi], dst_layout);
            new_verts.push(reencoded);
            next
        };
        new_local_indices.push(local_idx);
    }

    let mut dropped = Vec::new();
    for attr in ["bone_infos", "color", "mapping2", "mapping3", "mapping4", "mapping5"] {
        let src_has = src_layout_opt
            .map(|l| wmb3_layout_has_field(l, attr))
            .unwrap_or(false);
        if src_has && !wmb3_layout_has_field(dst_layout, attr) {
            dropped.push(attr.to_string());
        }
    }

    let dst_vg = &mut wmb3.vertex_groups[dst_vgi];
    let new_v_start = dst_vg.vertexes.len() as u32;
    let new_i_start = dst_vg.indices.len() as u32;
    let num_v = new_verts.len() as u32;
    let num_i = new_local_indices.len() as u32;
    dst_vg.vertexes.extend(new_verts);
    dst_vg.indices.extend(new_local_indices);
    dst_vg.num_vertexes = dst_vg.vertexes.len() as u32;
    dst_vg.num_indices = dst_vg.indices.len() as u32;

    batch.vertex_group_index = dst_vgi as u32;
    batch.vertex_start = new_v_start;
    batch.num_vertexes = num_v;
    batch.index_start = new_i_start;
    batch.num_indices = num_i;
    batch.num_primitives = num_i / 3;
    info.vertex_group_index = dst_vgi as u32;

    Ok(dropped)
}

fn parse_string_list(payload: &serde_json::Value, key: &str) -> Vec<String> {
    payload
        .get(key)
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|x| x.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default()
}

/// Apply one of the Appendix E.4 structural or material operations to a WMB0 / WMB1 / WMB2 file.
pub fn apply_wmb_scene_op(
    wmb: &mut WmbFile,
    uid_map: &mut UidMap,
    operation: &str,
    payload: &serde_json::Value,
    clipboard: Option<&WmbClipboardData>,
    bank_texture_ids: Option<&[u32]>,
) -> std::result::Result<MutationOutcome, String> {
    uid_map.ensure_wmb(wmb);
    let is_b2 = wmb.is_bayo2();
    let game = if is_b2 { "b2" } else { "b1" };

    let validate_wmb_mesh_name = |name: &str| -> std::result::Result<(), String> {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            return Err("Mesh group name cannot be empty".into());
        }
        if !trimmed.is_ascii() {
            return Err("WMB mesh group name must contain only ASCII characters".into());
        }
        if trimmed.len() > 31 {
            return Err(format!(
                "WMB mesh group name '{}' is {} bytes; maximum is 31 ASCII bytes",
                trimmed,
                trimmed.len()
            ));
        }
        Ok(())
    };

    match operation {
        "set_batches_material" => {
            let items = parse_string_list(payload, "items");
            if items.is_empty() {
                return Err("No items specified for set_batches_material".into());
            }
            let mat_idx = payload
                .get("material_index")
                .or_else(|| payload.get("materialIndex"))
                .and_then(|v| v.as_u64())
                .ok_or("Missing material_index for set_batches_material")? as usize;
            if mat_idx >= wmb.materials.len() {
                return Err(format!("Material index {mat_idx} out of bounds"));
            }
            if !is_b2 && mat_idx > u8::MAX as usize {
                return Err(format!(
                    "Bayonetta 1 WMB batch headers store material_id as u8 (max index 255), got #{mat_idx}"
                ));
            }
            let mut count = 0usize;
            for item_id in &items {
                let (mi, bi) = uid_map
                    .wmb_batch_pos_by_id(item_id)
                    .ok_or_else(|| format!("Batch '{item_id}' not found"))?;
                let batch = &mut wmb.meshes[mi].batches[bi];
                if is_b2 {
                    batch.header.ex_mat_id = mat_idx as i16;
                } else {
                    batch.header.material_id = mat_idx as u8;
                }
                count += 1;
            }
            Ok(MutationOutcome::new(
                format!("Assign Material #{mat_idx} to {count} batch(es)"),
                false,
            ))
        }
        "move_items" => {
            let items = parse_string_list(payload, "items");
            if items.is_empty() {
                return Err("No items specified for move_items".into());
            }
            let target = payload.get("target").unwrap_or(payload);
            let target_id = target
                .get("id")
                .and_then(|v| v.as_str())
                .ok_or("Missing target.id in move_items")?;
            let target_mesh_idx = uid_map
                .mesh_index_by_id(target_id)
                .or_else(|| uid_map.parse_lod_id(target_id).map(|(m, _)| m))
                .ok_or_else(|| format!("Target mesh '{target_id}' not found"))?;
            let req_index = target
                .get("index")
                .and_then(|v| v.as_u64())
                .map(|v| v as usize);

            // Locate all batches to move in the order listed in `items`
            let mut positions = Vec::with_capacity(items.len());
            let mut seen = BTreeSet::new();
            for item_id in &items {
                let (mi, bi) = uid_map
                    .wmb_batch_pos_by_id(item_id)
                    .ok_or_else(|| format!("Batch '{item_id}' not found"))?;
                if seen.insert((mi, bi)) {
                    positions.push((mi, bi));
                }
            }

            let mut removed_before_in_target = 0usize;
            if let Some(req_idx) = req_index {
                for &(mi, bi) in &positions {
                    if mi == target_mesh_idx && bi < req_idx {
                        removed_before_in_target += 1;
                    }
                }
            }

            // Extract batches while preserving the order in `positions`
            let mut extracted: Vec<(usize, WmbBatch, u64)> = Vec::with_capacity(positions.len());
            // Remove in reverse (mesh_idx, batch_idx) order so indices stay valid
            let mut sorted_pos: Vec<(usize, (usize, usize))> =
                positions.iter().copied().enumerate().collect();
            sorted_pos.sort_by(|a, b| b.1.cmp(&a.1));
            for (orig_order, (mi, bi)) in sorted_pos {
                let b = wmb.meshes[mi].batches.remove(bi);
                let u = uid_map.wmb_batch_uids[mi].remove(bi);
                extracted.push((orig_order, b, u));
            }
            extracted.sort_by_key(|&(orig_order, _, _)| orig_order);

            let dst_mesh = &mut wmb.meshes[target_mesh_idx];
            let first_shadow_pos = dst_mesh
                .batches
                .iter()
                .position(|b| b.header.is_shadow_proxy(is_b2))
                .unwrap_or(dst_mesh.batches.len());
            let raw_ins = req_index
                .map(|idx| idx.saturating_sub(removed_before_in_target))
                .unwrap_or(first_shadow_pos);
            let clamped_ins = raw_ins.min(first_shadow_pos).min(dst_mesh.batches.len());

            for (offset, (_, mut batch, uid)) in extracted.into_iter().enumerate() {
                batch.header.mesh_id = target_mesh_idx as i16;
                dst_mesh.batches.insert(clamped_ins + offset, batch);
                uid_map.wmb_batch_uids[target_mesh_idx].insert(clamped_ins + offset, uid);
            }

            let mesh_name = wmb.meshes[target_mesh_idx].name.clone();
            enforce_shadow_last(wmb, uid_map);
            Ok(MutationOutcome::new(
                format!("Move {} batch(es) to '{}'", items.len(), mesh_name),
                true,
            ))
        }
        "delete_items" => {
            let items = parse_string_list(payload, "items");
            if items.is_empty() {
                return Err("No items specified for delete_items".into());
            }
            let mut positions = Vec::new();
            for item_id in &items {
                let pos = uid_map
                    .wmb_batch_pos_by_id(item_id)
                    .ok_or_else(|| format!("Batch '{item_id}' not found"))?;
                if !positions.contains(&pos) {
                    positions.push(pos);
                }
            }
            let total_batches: usize = wmb.meshes.iter().map(|m| m.batches.len()).sum();
            if positions.len() >= total_batches {
                return Err("Cannot delete all batches in a WMB model".into());
            }
            positions.sort_by(|a, b| b.cmp(a));
            let count = positions.len();
            for (mi, bi) in positions {
                wmb.meshes[mi].batches.remove(bi);
                uid_map.wmb_batch_uids[mi].remove(bi);
            }
            enforce_shadow_last(wmb, uid_map);
            Ok(MutationOutcome::new(
                format!("Delete {count} batch(es)"),
                true,
            ))
        }
        "duplicate_items" => {
            let items = parse_string_list(payload, "items");
            if items.is_empty() {
                return Err("No items specified for duplicate_items".into());
            }
            let mut count = 0usize;
            for item_id in &items {
                let (mi, bi) = uid_map
                    .wmb_batch_pos_by_id(item_id)
                    .ok_or_else(|| format!("Batch '{item_id}' not found"))?;
                let cloned = wmb.meshes[mi].batches[bi].clone();
                let new_uid = uid_map.alloc();
                wmb.meshes[mi].batches.insert(bi + 1, cloned);
                uid_map.wmb_batch_uids[mi].insert(bi + 1, new_uid);
                count += 1;
            }
            enforce_shadow_last(wmb, uid_map);
            Ok(MutationOutcome::new(
                format!("Duplicate {count} batch(es)"),
                true,
            ))
        }
        "reorder_mesh_groups" => {
            let ids = parse_string_list(payload, "ids");
            if ids.is_empty() {
                return Err("No mesh group ids specified for reorder_mesh_groups".into());
            }
            let req_index = payload
                .get("index")
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as usize;

            let mut move_indices = Vec::new();
            for id in &ids {
                let mi = uid_map
                    .mesh_index_by_id(id)
                    .ok_or_else(|| format!("Mesh group '{id}' not found"))?;
                if !move_indices.contains(&mi) {
                    move_indices.push(mi);
                }
            }
            let removed_before = move_indices.iter().filter(|&&mi| mi < req_index).count();

            let mut moved_items = Vec::with_capacity(move_indices.len());
            let mut remaining_items = Vec::new();
            let old_meshes: Vec<_> = wmb.meshes.drain(..).collect();
            let old_mesh_uids: Vec<_> = uid_map.mesh_uids.drain(..).collect();
            let old_batch_uids: Vec<_> = uid_map.wmb_batch_uids.drain(..).collect();
            let mut indexed_all: Vec<Option<(WmbMesh, u64, Vec<u64>)>> = old_meshes
                .into_iter()
                .zip(old_mesh_uids)
                .zip(old_batch_uids)
                .map(|((m, mu), bu)| Some((m, mu, bu)))
                .collect();

            for &mi in &move_indices {
                if let Some(item) = indexed_all.get_mut(mi).and_then(|x| x.take()) {
                    moved_items.push(item);
                }
            }
            for item_opt in indexed_all {
                if let Some(item) = item_opt {
                    remaining_items.push(item);
                }
            }

            let ins_pos = req_index
                .saturating_sub(removed_before)
                .min(remaining_items.len());
            for (offset, item) in moved_items.into_iter().enumerate() {
                remaining_items.insert(ins_pos + offset, item);
            }
            for (m, mu, bu) in remaining_items {
                wmb.meshes.push(m);
                uid_map.mesh_uids.push(mu);
                uid_map.wmb_batch_uids.push(bu);
            }
            enforce_shadow_last(wmb, uid_map);
            Ok(MutationOutcome::new(
                format!("Reorder {} mesh group(s)", ids.len()),
                true,
            ))
        }
        "mesh_group_add" => {
            let name = payload
                .get("name")
                .and_then(|v| v.as_str())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| format!("mesh_{}", wmb.meshes.len()));
            validate_wmb_mesh_name(&name)?;
            let new_id = wmb.meshes.len() as i16;
            wmb.meshes.push(WmbMesh {
                id: new_id,
                u_a1: 0,
                u_a2: 0,
                offset_batch_offsets: 0x80,
                u_b: 0,
                u_c: [0; 4],
                name: name.clone(),
                bounding_matrix: [0.0; 12],
                batches: Vec::new(),
            });
            let new_uid = uid_map.alloc();
            uid_map.mesh_uids.push(new_uid);
            uid_map.wmb_batch_uids.push(Vec::new());
            reindex_wmb_batches_and_meshes(wmb);
            Ok(MutationOutcome::new(
                format!("Add mesh group '{name}'"),
                true,
            ))
        }
        "mesh_group_duplicate" => {
            let id = payload
                .get("id")
                .and_then(|v| v.as_str())
                .ok_or("Missing id for mesh_group_duplicate")?;
            let mi = uid_map
                .mesh_index_by_id(id)
                .ok_or_else(|| format!("Mesh group '{id}' not found"))?;
            let mut cloned = wmb.meshes[mi].clone();
            let base: String = cloned
                .name
                .chars()
                .filter(|c| c.is_ascii())
                .take(26)
                .collect();
            cloned.name = format!("{base}_copy");
            let new_mesh_uid = uid_map.alloc();
            let new_batch_uids: Vec<u64> =
                (0..cloned.batches.len()).map(|_| uid_map.alloc()).collect();
            wmb.meshes.insert(mi + 1, cloned);
            uid_map.mesh_uids.insert(mi + 1, new_mesh_uid);
            uid_map.wmb_batch_uids.insert(mi + 1, new_batch_uids);
            enforce_shadow_last(wmb, uid_map);
            Ok(MutationOutcome::new(
                format!("Duplicate mesh group #{}", mi),
                true,
            ))
        }
        "mesh_group_delete" => {
            let id = payload
                .get("id")
                .and_then(|v| v.as_str())
                .ok_or("Missing id for mesh_group_delete")?;
            let mi = uid_map
                .mesh_index_by_id(id)
                .ok_or_else(|| format!("Mesh group '{id}' not found"))?;
            if wmb.meshes.len() <= 1 {
                return Err("Cannot delete the only remaining mesh group".into());
            }
            let remaining_batches: usize = wmb
                .meshes
                .iter()
                .enumerate()
                .filter(|&(i, _)| i != mi)
                .map(|(_, m)| m.batches.len())
                .sum();
            if remaining_batches == 0 {
                return Err(
                    "Cannot delete mesh group because no batches would remain in the model".into(),
                );
            }
            let removed = wmb.meshes.remove(mi);
            uid_map.mesh_uids.remove(mi);
            uid_map.wmb_batch_uids.remove(mi);
            reindex_wmb_batches_and_meshes(wmb);
            Ok(MutationOutcome::new(
                format!("Delete mesh group '{}'", removed.name),
                true,
            ))
        }
        "mesh_group_rename" => {
            let id = payload
                .get("id")
                .and_then(|v| v.as_str())
                .ok_or("Missing id for mesh_group_rename")?;
            let name = payload
                .get("name")
                .and_then(|v| v.as_str())
                .ok_or("Missing name for mesh_group_rename")?
                .trim()
                .to_string();
            validate_wmb_mesh_name(&name)?;
            let mi = uid_map
                .mesh_index_by_id(id)
                .ok_or_else(|| format!("Mesh group '{id}' not found"))?;
            let old_name = std::mem::replace(&mut wmb.meshes[mi].name, name.clone());
            Ok(MutationOutcome::new(
                format!("Rename mesh group '{old_name}' to '{name}'"),
                true,
            ))
        }
        "material_add" => {
            if !is_b2 && wmb.materials.len() >= 256 {
                return Err(
                    "Bayonetta 1 WMB supports at most 256 materials (u8 material_id limit)".into(),
                );
            }
            let shader_key = payload
                .get("shaderKey")
                .or_else(|| payload.get("shader"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty());
            let new_idx = wmb.materials.len();
            let mut new_mat = if let Some(first) = wmb.materials.first() {
                first.clone()
            } else {
                WmbMaterial {
                    mat_type: if is_b2 { 0 } else { 110 },
                    flag: 0,
                    shader_name: if is_b2 {
                        Some("sis00_bxxxn".to_string())
                    } else {
                        None
                    },
                    material_data: vec![0; 48],
                }
            };
            if let Some(sk) = shader_key {
                let _ = switch_wmb_material_shader(&mut new_mat, new_idx, is_b2, sk);
            } else if wmb.materials.is_empty() {
                let default_key = if is_b2 { "sis00_bxxxn" } else { "110" };
                let _ = switch_wmb_material_shader(&mut new_mat, new_idx, is_b2, default_key);
            }
            wmb.materials.push(new_mat);
            let new_uid = uid_map.alloc();
            uid_map.material_uids.push(new_uid);
            wmb.header.num_materials = wmb.materials.len() as i32;
            Ok(MutationOutcome::new(
                format!("Add Material #{new_idx}"),
                true,
            ))
        }
        "material_duplicate" => {
            if !is_b2 && wmb.materials.len() >= 256 {
                return Err(
                    "Bayonetta 1 WMB supports at most 256 materials (u8 material_id limit)".into(),
                );
            }
            let id = payload
                .get("id")
                .and_then(|v| v.as_str())
                .ok_or("Missing id for material_duplicate")?;
            let mi = uid_map
                .material_index_by_id(id)
                .ok_or_else(|| format!("Material '{id}' not found"))?;
            let cloned = wmb.materials[mi].clone();
            wmb.materials.push(cloned);
            let new_uid = uid_map.alloc();
            uid_map.material_uids.push(new_uid);
            wmb.header.num_materials = wmb.materials.len() as i32;
            Ok(MutationOutcome::new(
                format!("Duplicate Material #{mi}"),
                true,
            ))
        }
        "material_delete" => {
            let id = payload
                .get("id")
                .and_then(|v| v.as_str())
                .ok_or("Missing id for material_delete")?;
            let del_idx = uid_map
                .material_index_by_id(id)
                .ok_or_else(|| format!("Material '{id}' not found"))?;
            if wmb.materials.len() <= 1 {
                return Err("Cannot delete the only remaining material".into());
            }
            let reassign_id_opt = payload
                .get("reassignToId")
                .or_else(|| payload.get("reassignTo"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty());
            let reassign_idx_opt = match reassign_id_opt {
                Some(rid) => Some(
                    uid_map
                        .material_index_by_id(rid)
                        .ok_or_else(|| format!("Reassign target material '{rid}' not found"))?,
                ),
                None => None,
            };
            if reassign_idx_opt == Some(del_idx) {
                return Err("Cannot reassign deleted material to itself".into());
            }

            let is_used = wmb.meshes.iter().any(|m| {
                m.batches
                    .iter()
                    .any(|b| b.header.effective_material_id(is_b2) == del_idx)
            });
            if is_used && reassign_idx_opt.is_none() {
                return Err(format!(
                    "Material #{del_idx} is still used by one or more batches and no reassignToId was provided"
                ));
            }

            for m in &mut wmb.meshes {
                for b in &mut m.batches {
                    let cur = b.header.effective_material_id(is_b2);
                    let next_mat = if cur == del_idx {
                        let r = reassign_idx_opt.unwrap_or(0);
                        if r > del_idx {
                            r - 1
                        } else {
                            r
                        }
                    } else if cur > del_idx {
                        cur - 1
                    } else {
                        cur
                    };
                    if is_b2 {
                        b.header.ex_mat_id = next_mat as i16;
                    } else {
                        b.header.material_id = next_mat as u8;
                    }
                }
            }

            wmb.materials.remove(del_idx);
            uid_map.material_uids.remove(del_idx);
            wmb.header.num_materials = wmb.materials.len() as i32;
            Ok(MutationOutcome::new(
                format!("Delete Material #{del_idx}"),
                true,
            ))
        }
        "material_rename" => {
            // WMB0/2 materials do not store a separate display name in binary, but succeed cleanly
            Ok(MutationOutcome::new("Rename WMB Material", false))
        }
        "paste_items" => {
            let clip = clipboard.ok_or("WMB clipboard is empty")?;
            if !clip.game.eq_ignore_ascii_case(game) {
                return Err(format!(
                    "Cannot paste clipboard items from game '{}' into a '{}' model",
                    clip.game, game
                ));
            }
            let target = payload.get("target").unwrap_or(payload);
            let explicit_target_kind = target.get("kind").and_then(|v| v.as_str());
            if let Some(tk) = explicit_target_kind {
                if tk == "materials" && clip.kind != "materials" && clip.materials_wmb.is_empty() {
                    return Err("Clipboard does not contain materials to paste".into());
                }
                if tk != "materials" && clip.kind == "materials" && clip.batches.is_empty() {
                    return Err(format!(
                        "Cannot paste clipboard materials into target kind '{tk}' (expected batches)"
                    ));
                }
            }
            let target_kind =
                explicit_target_kind.unwrap_or(if clip.kind == "materials" { "materials" } else { "mesh" });
            let dedup_materials = target
                .get("deduplicate")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);

            let mut warnings = Vec::new();
            let core_pre_ids: BTreeSet<u32> = list_core_pretextures(game)
                .into_iter()
                .map(|p| p.id)
                .collect();

            if clip.kind == "materials" || target_kind == "materials" {
                let mut pasted = 0usize;
                for src_mat in &clip.materials_wmb {
                    check_wmb_material_missing_textures(
                        src_mat,
                        is_b2,
                        bank_texture_ids,
                        &core_pre_ids,
                        &mut warnings,
                    );
                    let already_exists =
                        dedup_materials && wmb.materials.iter().any(|m| m == src_mat);
                    if !already_exists {
                        if !is_b2 && wmb.materials.len() >= 256 {
                            return Err(
                                "Bayonetta 1 WMB supports at most 256 materials (u8 material_id limit)"
                                    .into(),
                            );
                        }
                        wmb.materials.push(src_mat.clone());
                        let new_uid = uid_map.alloc();
                        uid_map.material_uids.push(new_uid);
                    }
                    pasted += 1;
                }
                wmb.header.num_materials = wmb.materials.len() as i32;
                return Ok(MutationOutcome::with_warnings(
                    format!("Paste {pasted} material(s)"),
                    warnings,
                    true,
                ));
            }

            // Paste batches into target mesh group
            if wmb.meshes.is_empty() {
                return Err("Target WMB has no mesh groups to paste into".into());
            }
            let target_mesh_idx = if let Some(id_str) = target
                .get("id")
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
            {
                uid_map
                    .mesh_index_by_id(id_str)
                    .or_else(|| uid_map.parse_lod_id(id_str).map(|(m, _)| m))
                    .ok_or_else(|| format!("Target mesh group '{id_str}' not found"))?
            } else {
                0
            };

            let dst_layout = lookup_wmb_vertex_layout(
                wmb.header.vertex_flags,
                wmb.header.vertex_ex_data_size,
                wmb.header.vertex_ex_data,
            )
            .ok_or("Target WMB has an unsupported vertex layout")?;

            // Dedupe or append referenced materials
            let mut mat_remap = Vec::with_capacity(clip.materials_wmb.len());
            for src_mat in &clip.materials_wmb {
                check_wmb_material_missing_textures(
                    src_mat,
                    is_b2,
                    bank_texture_ids,
                    &core_pre_ids,
                    &mut warnings,
                );
                if let Some(existing_idx) = wmb.materials.iter().position(|m| m == src_mat) {
                    mat_remap.push(existing_idx);
                } else {
                    if !is_b2 && wmb.materials.len() >= 256 {
                        return Err(
                            "Bayonetta 1 WMB supports at most 256 materials (u8 material_id limit)"
                                .into(),
                        );
                    }
                    let new_idx = wmb.materials.len();
                    wmb.materials.push(src_mat.clone());
                    let new_uid = uid_map.alloc();
                    uid_map.material_uids.push(new_uid);
                    mat_remap.push(new_idx);
                }
            }

            let mut dropped_bones = BTreeSet::new();
            let mut dropped_attrs = BTreeSet::new();
            let dst_has_field = |field_name: &str| -> bool {
                dst_layout
                    .primary_fields
                    .iter()
                    .chain(dst_layout.ex_fields.iter())
                    .any(|f| f.name == field_name)
            };
            let mut pasted_batches = Vec::with_capacity(clip.batches.len());

            for cb in &clip.batches {
                if !clip.materials_wmb.is_empty() && cb.material_clipboard_idx >= mat_remap.len() {
                    return Err(format!(
                        "Clipboard batch references out-of-range material_clipboard_idx {}",
                        cb.material_clipboard_idx
                    ));
                }
                let v_base = wmb.vertexes.len();
                let mut local_bone_palette: Vec<u8> = Vec::new();

                for cv in &cb.vertices {
                    let mut v = cv.vertex.clone();
                    if !cv.global_bone_weights.is_empty() {
                        let mut mapped_pairs = Vec::new();
                        for &(gid, weight) in &cv.global_bone_weights {
                            let target_local_opt = if gid >= 0 {
                                wmb.bone_index_translate_table
                                    .table
                                    .get(&(gid as u16))
                                    .copied()
                                    .map(|l| l as u8)
                                    .or_else(|| {
                                        if wmb.bone_index_translate_table.table.is_empty()
                                            && (gid as usize) < wmb.bone_positions.len()
                                        {
                                            Some(gid as u8)
                                        } else {
                                            None
                                        }
                                    })
                            } else {
                                None
                            };
                            if let Some(target_local) = target_local_opt {
                                let pal_idx = if let Some(pos) =
                                    local_bone_palette.iter().position(|&b| b == target_local)
                                {
                                    pos as u8
                                } else {
                                    if local_bone_palette.len() >= 256 {
                                        return Err(
                                            "WMB batch bone palette exceeds 256-joint u8 limit"
                                                .into(),
                                        );
                                    }
                                    local_bone_palette.push(target_local);
                                    (local_bone_palette.len() - 1) as u8
                                };
                                mapped_pairs.push((pal_idx, weight));
                            } else {
                                dropped_bones.insert(gid);
                            }
                        }
                        if mapped_pairs.is_empty() {
                            if local_bone_palette.is_empty() {
                                local_bone_palette.push(0);
                            }
                            mapped_pairs.push((0, 255));
                        }
                        let mut bi = BoneInfos::default();
                        bi.set_active_pairs(&mapped_pairs);
                        v.bone_infos = Some(bi);
                    }
                    if v.position2.is_some() && !dst_has_field("position2") {
                        dropped_attrs.insert("position2");
                    }
                    if v.color.is_some() && !dst_has_field("color") {
                        dropped_attrs.insert("color");
                    }
                    if v.mapping2.is_some() && !dst_has_field("mapping2") {
                        dropped_attrs.insert("mapping2");
                    }
                    let encoded = roundtrip_vertex_wmb(&v, dst_layout, is_b2, wmb.endian);
                    if !wmb.positions.is_empty() {
                        wmb.positions.push(encoded.position.unwrap_or_default());
                    }
                    wmb.vertexes.push(encoded);
                }

                let abs_tris: Vec<[usize; 3]> = cb
                    .triangles
                    .iter()
                    .map(|&[a, b, c]| [v_base + a as usize, v_base + b as usize, v_base + c as usize])
                    .collect();

                let dst_mat_idx = mat_remap
                    .get(cb.material_clipboard_idx)
                    .copied()
                    .unwrap_or(0);
                let mut hdr = cb.wmb_header_template.clone().unwrap_or(WmbBatchHeader {
                    batch_id: 0,
                    mesh_id: target_mesh_idx as i16,
                    flags: 0x8001,
                    ex_mat_id: if is_b2 { dst_mat_idx as i16 } else { 0 },
                    material_id: dst_mat_idx as u8,
                    has_bone_refs: if local_bone_palette.is_empty() { 0 } else { 1 },
                    u_e1: 0,
                    u_e2: 0,
                    vertex_start: 0,
                    vertex_end: 0,
                    primitive_type: 4,
                    offset_indices: 0x60,
                    num_indices: 0,
                    vertex_offset: 0,
                    u_f: [0; 7],
                });
                hdr.mesh_id = target_mesh_idx as i16;
                if is_b2 {
                    hdr.ex_mat_id = dst_mat_idx as i16;
                } else {
                    hdr.material_id = dst_mat_idx as u8;
                }
                hdr.set_shadow_proxy(is_b2, cb.is_shadow_proxy);
                hdr.has_bone_refs = if local_bone_palette.is_empty() { 0 } else { 1 };

                let mut new_batch = WmbBatch {
                    header: hdr,
                    bone_refs: local_bone_palette,
                    unknown_no_bones: [0.0; 4],
                    indices: Vec::new(),
                };
                new_batch.set_triangles(&abs_tris);
                new_batch.header.offset_indices = new_batch.effective_indices_offset();
                pasted_batches.push(new_batch);
            }

            if !dropped_bones.is_empty() {
                warnings.push(format!(
                    "Dropped influences for missing global bone IDs: {:?}",
                    dropped_bones
                ));
            }
            if !dropped_attrs.is_empty() {
                let attrs: Vec<&str> = dropped_attrs.into_iter().collect();
                warnings.push(format!(
                    "Target WMB vertex layout omitted attribute(s): {}",
                    attrs.join(", ")
                ));
            }

            let dst_mesh = &mut wmb.meshes[target_mesh_idx];
            let first_shadow = dst_mesh
                .batches
                .iter()
                .position(|b| b.header.is_shadow_proxy(is_b2))
                .unwrap_or(dst_mesh.batches.len());
            let req_ins = target
                .get("index")
                .and_then(|v| v.as_u64())
                .map(|v| v as usize)
                .unwrap_or(first_shadow)
                .min(first_shadow)
                .min(dst_mesh.batches.len());

            let count = pasted_batches.len();
            for (offset, b) in pasted_batches.into_iter().enumerate() {
                let new_uid = uid_map.alloc();
                dst_mesh.batches.insert(req_ins + offset, b);
                uid_map.wmb_batch_uids[target_mesh_idx].insert(req_ins + offset, new_uid);
            }

            enforce_shadow_last(wmb, uid_map);
            Ok(MutationOutcome::with_warnings(
                format!("Paste {count} batch(es)"),
                warnings,
                true,
            ))
        }
        other => Err(format!("Unsupported WMB scene operation: {other}")),
    }
}

fn check_wmb_material_missing_textures(
    mat: &WmbMaterial,
    is_b2: bool,
    bank_texture_ids: Option<&[u32]>,
    core_pre_ids: &BTreeSet<u32>,
    warnings: &mut Vec<String>,
) {
    let Some(bank_ids) = bank_texture_ids else {
        return;
    };
    let dec = decode_wmb_material_rich(mat, 0, is_b2);
    for f in &dec.fields {
        if let Some(tid) = f.sampler_tex_id {
            let in_pre = core_pre_ids.contains(&tid) || (!is_b2 && (tid & 0x8000_0000) != 0);
            let in_bank = if !is_b2 {
                bank_ids.contains(&tid) || (tid as usize) < bank_ids.len()
            } else {
                bank_ids.contains(&tid)
            };
            if !in_pre && !in_bank {
                let msg = format!("Referenced texture 0x{tid:08X} is absent from target bank");
                if !warnings.contains(&msg) {
                    warnings.push(msg);
                }
            }
        }
    }
}

fn check_wmb3_material_missing_textures(
    mat: &Wmb3Material,
    bank_texture_ids: Option<&[u32]>,
    core_pre_ids: &BTreeSet<u32>,
    warnings: &mut Vec<String>,
) {
    let Some(bank_ids) = bank_texture_ids else {
        return;
    };
    for &(_, tid) in &mat.textures {
        if !core_pre_ids.contains(&tid) && !bank_ids.contains(&tid) {
            let msg = format!("Referenced texture 0x{tid:08X} is absent from target bank");
            if !warnings.contains(&msg) {
                warnings.push(msg);
            }
        }
    }
}

/// Unpacked WMB3 LOD batch lists plus any trailing/uncovered batches not referenced by any LOD range (`B-5`).
struct UnpackedWmb3Lods {
    lod_lists: Vec<Vec<(Wmb3Batch, Wmb3LodBatchInfo, u64)>>,
    uncovered: Vec<(Wmb3Batch, u64)>,
}

/// Unpack WMB3 global batches + LOD batch_infos + UIDs into per-LOD lists of `(Wmb3Batch, Wmb3LodBatchInfo, u64)`
/// while preserving any batches not covered by a LOD range (`B-5`).
fn unpack_wmb3_lod_batches(
    wmb3: &Wmb3File,
    uid_map: &UidMap,
) -> UnpackedWmb3Lods {
    let mut out = Vec::with_capacity(wmb3.lods.len());
    let mut covered = vec![false; wmb3.batches.len()];
    for lod in &wmb3.lods {
        let start = lod.batch_start as usize;
        let mut list = Vec::with_capacity(lod.batch_infos.len());
        for (i, inf) in lod.batch_infos.iter().enumerate() {
            let bi = start + i;
            if bi < covered.len() && !covered[bi] {
                covered[bi] = true;
                if let (Some(b), Some(&u)) = (wmb3.batches.get(bi), uid_map.wmb3_batch_uids.get(bi)) {
                    list.push((b.clone(), inf.clone(), u));
                }
            }
        }
        out.push(list);
    }
    let mut uncovered = Vec::new();
    for (bi, is_cov) in covered.into_iter().enumerate() {
        if !is_cov {
            if let (Some(b), Some(&u)) = (wmb3.batches.get(bi), uid_map.wmb3_batch_uids.get(bi)) {
                uncovered.push((b.clone(), u));
            }
        }
    }
    UnpackedWmb3Lods {
        lod_lists: out,
        uncovered,
    }
}

/// Repack per-LOD lists of `(Wmb3Batch, Wmb3LodBatchInfo, u64)` and any uncovered batches back into `wmb3` and `uid_map`.
fn repack_wmb3_lod_batches(
    wmb3: &mut Wmb3File,
    uid_map: &mut UidMap,
    unpacked: UnpackedWmb3Lods,
) {
    let mut new_batches = Vec::new();
    let mut new_uids = Vec::new();
    for (li, list) in unpacked.lod_lists.into_iter().enumerate() {
        if let Some(lod) = wmb3.lods.get_mut(li) {
            lod.batch_start = new_batches.len() as u32;
            lod.batch_infos.clear();
            for (b, inf, u) in list {
                new_batches.push(b);
                lod.batch_infos.push(inf);
                new_uids.push(u);
            }
        }
    }
    for (b, u) in unpacked.uncovered {
        new_batches.push(b);
        new_uids.push(u);
    }
    wmb3.batches = new_batches;
    uid_map.wmb3_batch_uids = new_uids;
    sync_wmb3_invariants(wmb3);
}

/// Apply one of the Appendix E.4 structural or material operations to a WMB3 file.
pub fn apply_wmb3_scene_op(
    wmb3: &mut Wmb3File,
    uid_map: &mut UidMap,
    operation: &str,
    payload: &serde_json::Value,
    clipboard: Option<&WmbClipboardData>,
    bank_texture_ids: Option<&[u32]>,
) -> std::result::Result<MutationOutcome, String> {
    uid_map.ensure_wmb3(wmb3);

    match operation {
        "set_batches_material" => {
            let items = parse_string_list(payload, "items");
            if items.is_empty() {
                return Err("No items specified for set_batches_material".into());
            }
            let mat_idx = payload
                .get("material_index")
                .or_else(|| payload.get("materialIndex"))
                .and_then(|v| v.as_u64())
                .ok_or("Missing material_index for set_batches_material")? as usize;
            if mat_idx >= wmb3.materials.len() {
                return Err(format!("WMB3 Material index {mat_idx} out of bounds"));
            }
            let mut target_global_indices = BTreeSet::new();
            for item_id in &items {
                let bi = uid_map
                    .wmb3_batch_index_by_id(item_id)
                    .ok_or_else(|| format!("Batch '{item_id}' not found"))?;
                target_global_indices.insert(bi);
            }
            for lod in &mut wmb3.lods {
                let start = lod.batch_start as usize;
                for (i, inf) in lod.batch_infos.iter_mut().enumerate() {
                    if target_global_indices.contains(&(start + i)) {
                        inf.material_index = mat_idx as u32;
                    }
                }
            }
            sync_wmb3_invariants(wmb3);
            Ok(MutationOutcome::new(
                format!("Assign WMB3 Material #{mat_idx} to {} batch(es)", target_global_indices.len()),
                false,
            ))
        }
        "move_items" => {
            let items = parse_string_list(payload, "items");
            if items.is_empty() {
                return Err("No items specified for move_items".into());
            }
            let target = payload.get("target").unwrap_or(payload);
            let target_id = target
                .get("id")
                .and_then(|v| v.as_str())
                .ok_or("Missing target.id in move_items")?;
            let req_index = target
                .get("index")
                .and_then(|v| v.as_u64())
                .map(|v| v as usize);

            let (target_mesh_idx, target_lod_level_opt) =
                if let Some((m_idx, lvl)) = uid_map.parse_lod_id(target_id) {
                    (m_idx, Some(lvl))
                } else if let Some(m_idx) = uid_map.mesh_index_by_id(target_id) {
                    (m_idx, None)
                } else {
                    return Err(format!("Invalid move_items target '{target_id}'"));
                };

            let target_lod_idx = if let Some(lvl) = target_lod_level_opt {
                wmb3.lods
                    .iter()
                    .position(|l| l.lod_level == lvl)
                    .ok_or_else(|| format!("WMB3 LOD level {lvl} not found"))?
            } else {
                0
            };

            let mut unpacked = unpack_wmb3_lod_batches(wmb3, uid_map);
            let explicit_vgi_override = target
                .get("vertex_group_index")
                .or_else(|| target.get("vertexGroupIndex"))
                .and_then(|v| v.as_u64())
                .map(|v| v as usize);

            // Determine target LOD's preferred vertex group index BEFORE extracting
            let dst_vgi_opt: Option<usize> = unpacked
                .lod_lists
                .get(target_lod_idx)
                .and_then(|list| {
                    list.iter()
                        .find(|(_, inf, _)| inf.mesh_index as usize == target_mesh_idx)
                        .or_else(|| list.first())
                })
                .map(|(b, _, _)| b.vertex_group_index as usize);

            // Parse requested batch UIDs and compute original positions BEFORE any removal (B-3)
            let mut requested_uids = Vec::with_capacity(items.len());
            let mut seen_uids = BTreeSet::new();
            for item_id in &items {
                let raw_uid: u64 = item_id
                    .trim()
                    .strip_prefix("batch:")
                    .unwrap_or(item_id.trim())
                    .parse()
                    .map_err(|_| format!("Invalid batch id '{item_id}'"))?;
                if seen_uids.insert(raw_uid) {
                    requested_uids.push(raw_uid);
                }
            }

            let mut orig_mesh_local_by_uid: BTreeMap<u64, usize> = BTreeMap::new();
            if let Some(target_list) = unpacked.lod_lists.get(target_lod_idx) {
                let mut mesh_local_pos = 0usize;
                for (_, inf, u) in target_list {
                    if inf.mesh_index as usize == target_mesh_idx {
                        orig_mesh_local_by_uid.insert(*u, mesh_local_pos);
                        mesh_local_pos += 1;
                    }
                }
            }

            let mut removed_before_in_target_group = 0usize;
            if let Some(req_idx) = req_index {
                for &u in &requested_uids {
                    if let Some(&orig_local) = orig_mesh_local_by_uid.get(&u) {
                        if orig_local < req_idx {
                            removed_before_in_target_group += 1;
                        }
                    }
                }
            }

            // Now extract each batch in requested order
            let mut extracted = Vec::with_capacity(requested_uids.len());
            for raw_uid in requested_uids {
                let mut found = None;
                for (li, list) in unpacked.lod_lists.iter_mut().enumerate() {
                    if let Some(pos) = list.iter().position(|(_, _, u)| *u == raw_uid) {
                        found = Some((li, list.remove(pos)));
                        break;
                    }
                }
                let (src_li, item) =
                    found.ok_or_else(|| format!("Batch 'batch:{raw_uid}' not found in WMB3 LODs"))?;
                extracted.push((src_li, item));
            }

            let mut warnings = Vec::new();
            let mut prepared = Vec::with_capacity(extracted.len());
            for (src_li, (mut batch, mut info, uid)) in extracted {
                info.mesh_index = target_mesh_idx as u32;
                // Only re-encode vertex group when moving across different LODs or when an explicit override is passed (B-4)
                let dst_vgi = if let Some(explicit_vgi) = explicit_vgi_override {
                    explicit_vgi
                } else if src_li != target_lod_idx {
                    dst_vgi_opt.unwrap_or(batch.vertex_group_index as usize)
                } else {
                    batch.vertex_group_index as usize
                };
                if dst_vgi != batch.vertex_group_index as usize {
                    let dropped = reencode_wmb3_batch_to_vg(wmb3, &mut batch, &mut info, dst_vgi)?;
                    if !dropped.is_empty() {
                        let msg = format!(
                            "Re-encoded batch:{uid} into VG #{dst_vgi}, dropped: {}",
                            dropped.join(", ")
                        );
                        warnings.push(msg);
                    }
                }
                prepared.push((batch, info, uid));
            }

            // Insert into `unpacked.lod_lists[target_lod_idx]` at the requested position among `target_mesh_idx` batches
            let dst_list = &mut unpacked.lod_lists[target_lod_idx];
            let mesh_positions: Vec<usize> = dst_list
                .iter()
                .enumerate()
                .filter_map(|(idx, (_, inf, _))| {
                    if inf.mesh_index as usize == target_mesh_idx {
                        Some(idx)
                    } else {
                        None
                    }
                })
                .collect();

            let desired_local = req_index
                .map(|r| r.saturating_sub(removed_before_in_target_group))
                .unwrap_or(mesh_positions.len());

            let abs_insert_pos = if mesh_positions.is_empty() {
                dst_list.len()
            } else if desired_local < mesh_positions.len() {
                mesh_positions[desired_local]
            } else {
                mesh_positions.last().copied().unwrap() + 1
            };

            for (offset, item) in prepared.into_iter().enumerate() {
                dst_list.insert(abs_insert_pos + offset, item);
            }

            let mesh_name = wmb3
                .meshes
                .get(target_mesh_idx)
                .map(|m| m.name.clone())
                .unwrap_or_else(|| format!("mesh_{target_mesh_idx}"));
            let lod_lvl = wmb3
                .lods
                .get(target_lod_idx)
                .map(|l| l.lod_level)
                .unwrap_or(0);
            repack_wmb3_lod_batches(wmb3, uid_map, unpacked);

            Ok(MutationOutcome::with_warnings(
                format!(
                    "Move {} batch(es) to '{}' LOD {}",
                    items.len(),
                    mesh_name,
                    lod_lvl
                ),
                warnings,
                true,
            ))
        }
        "delete_items" => {
            let items = parse_string_list(payload, "items");
            if items.is_empty() {
                return Err("No items specified for delete_items".into());
            }
            let mut uids_to_delete = BTreeSet::new();
            for item_id in &items {
                let raw_uid: u64 = item_id
                    .trim()
                    .strip_prefix("batch:")
                    .unwrap_or(item_id.trim())
                    .parse()
                    .map_err(|_| format!("Invalid batch id '{item_id}'"))?;
                if !uid_map.wmb3_batch_uids.contains(&raw_uid) {
                    return Err(format!("Batch '{item_id}' not found"));
                }
                uids_to_delete.insert(raw_uid);
            }
            if uids_to_delete.len() >= wmb3.batches.len() {
                return Err("Cannot delete all batches in a WMB3 model".into());
            }
            let mut unpacked = unpack_wmb3_lod_batches(wmb3, uid_map);
            for list in &mut unpacked.lod_lists {
                list.retain(|(_, _, u)| !uids_to_delete.contains(u));
            }
            unpacked.uncovered.retain(|(_, u)| !uids_to_delete.contains(u));
            let count = uids_to_delete.len();
            repack_wmb3_lod_batches(wmb3, uid_map, unpacked);
            Ok(MutationOutcome::new(
                format!("Delete {count} batch(es)"),
                true,
            ))
        }
        "duplicate_items" => {
            let items = parse_string_list(payload, "items");
            if items.is_empty() {
                return Err("No items specified for duplicate_items".into());
            }
            let mut target_uids = Vec::new();
            for item_id in &items {
                let raw_uid: u64 = item_id
                    .trim()
                    .strip_prefix("batch:")
                    .unwrap_or(item_id.trim())
                    .parse()
                    .map_err(|_| format!("Invalid batch id '{item_id}'"))?;
                if !uid_map.wmb3_batch_uids.contains(&raw_uid) {
                    return Err(format!("Batch '{item_id}' not found"));
                }
                target_uids.push(raw_uid);
            }
            let mut unpacked = unpack_wmb3_lod_batches(wmb3, uid_map);
            let mut count = 0usize;
            for target_u in target_uids {
                for list in &mut unpacked.lod_lists {
                    if let Some(pos) = list.iter().position(|(_, _, u)| *u == target_u) {
                        let (b, inf, _) = list[pos].clone();
                        let new_uid = uid_map.alloc();
                        list.insert(pos + 1, (b, inf, new_uid));
                        count += 1;
                        break;
                    }
                }
            }
            repack_wmb3_lod_batches(wmb3, uid_map, unpacked);
            Ok(MutationOutcome::new(
                format!("Duplicate {count} batch(es)"),
                true,
            ))
        }
        "reorder_mesh_groups" => {
            let ids = parse_string_list(payload, "ids");
            if ids.is_empty() {
                return Err("No mesh group ids specified for reorder_mesh_groups".into());
            }
            let req_index = payload
                .get("index")
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as usize;

            let mut move_indices = Vec::new();
            for id in &ids {
                let mi = uid_map
                    .mesh_index_by_id(id)
                    .ok_or_else(|| format!("Mesh group '{id}' not found"))?;
                if !move_indices.contains(&mi) {
                    move_indices.push(mi);
                }
            }
            let removed_before = move_indices.iter().filter(|&&mi| mi < req_index).count();

            let old_meshes: Vec<_> = wmb3.meshes.drain(..).collect();
            let old_uids: Vec<_> = uid_map.mesh_uids.drain(..).collect();
            let mut indexed: Vec<Option<(usize, Wmb3Mesh, u64)>> = old_meshes
                .into_iter()
                .zip(old_uids)
                .enumerate()
                .map(|(orig_i, (m, u))| Some((orig_i, m, u)))
                .collect();

            let mut moved = Vec::new();
            for &mi in &move_indices {
                if let Some(item) = indexed.get_mut(mi).and_then(|x| x.take()) {
                    moved.push(item);
                }
            }
            let mut remaining: Vec<(usize, Wmb3Mesh, u64)> =
                indexed.into_iter().flatten().collect();
            let ins_pos = req_index
                .saturating_sub(removed_before)
                .min(remaining.len());
            for (offset, item) in moved.into_iter().enumerate() {
                remaining.insert(ins_pos + offset, item);
            }

            let mut old_to_new = BTreeMap::new();
            for (new_idx, (old_idx, m, u)) in remaining.into_iter().enumerate() {
                old_to_new.insert(old_idx as u32, new_idx as u32);
                wmb3.meshes.push(m);
                uid_map.mesh_uids.push(u);
            }

            for lod in &mut wmb3.lods {
                for inf in &mut lod.batch_infos {
                    if let Some(&new_m) = old_to_new.get(&inf.mesh_index) {
                        inf.mesh_index = new_m;
                    }
                }
            }
            for pair in &mut wmb3.mesh_material_pairs {
                if let Some(&new_m) = old_to_new.get(&pair.0) {
                    pair.0 = new_m;
                }
            }
            sync_wmb3_invariants(wmb3);
            Ok(MutationOutcome::new(
                format!("Reorder {} mesh group(s)", ids.len()),
                true,
            ))
        }
        "mesh_group_add" => {
            let name = payload
                .get("name")
                .and_then(|v| v.as_str())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| format!("mesh_{}", wmb3.meshes.len()));
            wmb3.meshes.push(Wmb3Mesh {
                name: name.clone(),
                bounding_box: wmb3.bounding_box,
                materials: Vec::new(),
                bone_indices: Vec::new(),
            });
            let new_uid = uid_map.alloc();
            uid_map.mesh_uids.push(new_uid);
            sync_wmb3_invariants(wmb3);
            Ok(MutationOutcome::new(
                format!("Add WMB3 mesh group '{name}'"),
                true,
            ))
        }
        "mesh_group_duplicate" => {
            let id = payload
                .get("id")
                .and_then(|v| v.as_str())
                .ok_or("Missing id for mesh_group_duplicate")?;
            let mi = uid_map
                .mesh_index_by_id(id)
                .ok_or_else(|| format!("Mesh group '{id}' not found"))?;
            let mut cloned = wmb3.meshes[mi].clone();
            cloned.name = format!("{}_copy", cloned.name);
            let new_mesh_idx = wmb3.meshes.len() as u32;
            wmb3.meshes.push(cloned);
            let new_mesh_uid = uid_map.alloc();
            uid_map.mesh_uids.push(new_mesh_uid);

            let mut unpacked = unpack_wmb3_lod_batches(wmb3, uid_map);
            for list in &mut unpacked.lod_lists {
                let mut dupes = Vec::new();
                for (b, inf, _) in list.iter() {
                    if inf.mesh_index as usize == mi {
                        let mut new_inf = inf.clone();
                        new_inf.mesh_index = new_mesh_idx;
                        let new_u = uid_map.alloc();
                        dupes.push((b.clone(), new_inf, new_u));
                    }
                }
                list.extend(dupes);
            }
            repack_wmb3_lod_batches(wmb3, uid_map, unpacked);
            Ok(MutationOutcome::new(
                format!("Duplicate WMB3 mesh group #{mi}"),
                true,
            ))
        }
        "mesh_group_delete" => {
            let id = payload
                .get("id")
                .and_then(|v| v.as_str())
                .ok_or("Missing id for mesh_group_delete")?;
            let mi = uid_map
                .mesh_index_by_id(id)
                .ok_or_else(|| format!("Mesh group '{id}' not found"))?;
            if wmb3.meshes.len() <= 1 {
                return Err("Cannot delete the only remaining mesh group".into());
            }
            let remaining_batches: usize = wmb3
                .lods
                .iter()
                .flat_map(|l| l.batch_infos.iter())
                .filter(|inf| inf.mesh_index as usize != mi)
                .count();
            if remaining_batches == 0 {
                return Err(
                    "Cannot delete WMB3 mesh group because no batches would remain in the model"
                        .into(),
                );
            }
            let removed = wmb3.meshes.remove(mi);
            uid_map.mesh_uids.remove(mi);

            let mut unpacked = unpack_wmb3_lod_batches(wmb3, uid_map);
            for list in &mut unpacked.lod_lists {
                list.retain(|(_, inf, _)| inf.mesh_index as usize != mi);
                for (_, inf, _) in list.iter_mut() {
                    if (inf.mesh_index as usize) > mi {
                        inf.mesh_index -= 1;
                    }
                }
            }
            wmb3.mesh_material_pairs
                .retain(|&(m, _)| m as usize != mi);
            for pair in &mut wmb3.mesh_material_pairs {
                if (pair.0 as usize) > mi {
                    pair.0 -= 1;
                }
            }
            repack_wmb3_lod_batches(wmb3, uid_map, unpacked);
            Ok(MutationOutcome::new(
                format!("Delete WMB3 mesh group '{}'", removed.name),
                true,
            ))
        }
        "mesh_group_rename" => {
            let id = payload
                .get("id")
                .and_then(|v| v.as_str())
                .ok_or("Missing id for mesh_group_rename")?;
            let name = payload
                .get("name")
                .and_then(|v| v.as_str())
                .ok_or("Missing name for mesh_group_rename")?
                .trim()
                .to_string();
            if name.is_empty() {
                return Err("Mesh group name cannot be empty".into());
            }
            let mi = uid_map
                .mesh_index_by_id(id)
                .ok_or_else(|| format!("Mesh group '{id}' not found"))?;
            let old_name = std::mem::replace(&mut wmb3.meshes[mi].name, name.clone());
            Ok(MutationOutcome::new(
                format!("Rename WMB3 mesh group '{old_name}' to '{name}'"),
                true,
            ))
        }
        "update_wmb3_lod" => {
            let id = payload
                .get("id")
                .and_then(|v| v.as_str())
                .ok_or("Missing id for update_wmb3_lod")?;
            let level_from_id = uid_map
                .parse_lod_id(id)
                .map(|(_, lvl)| lvl)
                .or_else(|| {
                    id.strip_prefix("lod:")
                        .and_then(|s| s.rsplit('/').next())
                        .and_then(|s| s.parse::<i32>().ok())
                })
                .ok_or_else(|| format!("Invalid LOD id '{id}'"))?;
            let lod_pos = wmb3
                .lods
                .iter()
                .position(|l| l.lod_level == level_from_id)
                .ok_or_else(|| format!("WMB3 LOD level {level_from_id} not found"))?;
            if let Some(new_lvl_i64) = payload.get("level").and_then(|v| v.as_i64()) {
                let new_lvl = i32::try_from(new_lvl_i64)
                    .map_err(|_| format!("Invalid WMB3 LOD level {new_lvl_i64} (out of i32 range)"))?;
                if new_lvl != level_from_id
                    && wmb3
                        .lods
                        .iter()
                        .enumerate()
                        .any(|(idx, l)| idx != lod_pos && l.lod_level == new_lvl)
                {
                    return Err(format!("WMB3 already has a LOD with level {new_lvl}"));
                }
                wmb3.lods[lod_pos].lod_level = new_lvl;
            }
            if let Some(new_name) = payload.get("name").and_then(|v| v.as_str()) {
                let trimmed = new_name.trim();
                if !trimmed.is_empty() {
                    wmb3.lods[lod_pos].name = trimmed.to_string();
                }
            }
            Ok(MutationOutcome::new(
                format!("Update WMB3 LOD (level {})", wmb3.lods[lod_pos].lod_level),
                true,
            ))
        }
        "material_add" => {
            let explicit_shader = payload
                .get("shaderKey")
                .or_else(|| payload.get("shader"))
                .and_then(|v| v.as_str())
                .map(|s| s.trim())
                .filter(|s| !s.is_empty());
            let shader_key = explicit_shader.unwrap_or("PBS00_XXXXX");
            if let Some(req_key) = explicit_shader {
                if req_key != "PBS00" && lookup_b3_shader_params(req_key).is_none() {
                    return Err(format!("Unknown WMB3 shader '{req_key}'"));
                }
            }
            let new_idx = wmb3.materials.len();
            let inherited_date = wmb3
                .materials
                .first()
                .map(|m| m.date)
                .unwrap_or([2026, 11, 2, 0]);
            let mut candidate_name = format!("Material_{new_idx}");
            let mut suffix = 2usize;
            while wmb3.materials.iter().any(|m| m.name == candidate_name) {
                candidate_name = format!("Material_{new_idx}_{suffix}");
                suffix += 1;
            }
            let mut new_mat = Wmb3Material {
                date: inherited_date,
                name: candidate_name,
                shader_name: shader_key.to_string(),
                technique_name: "Default".to_string(),
                u_a: 1,
                textures: Vec::new(),
                parameter_groups: Vec::new(),
                variables: Vec::new(),
            };
            let lookup_key = if lookup_b3_shader_params(shader_key).is_some() {
                shader_key
            } else {
                "PBS00_XXXXX"
            };
            switch_wmb3_material_shader(&mut new_mat, lookup_key);
            new_mat.shader_name = shader_key.to_string();
            wmb3.materials.push(new_mat);
            let new_uid = uid_map.alloc();
            uid_map.material_uids.push(new_uid);
            sync_wmb3_invariants(wmb3);
            Ok(MutationOutcome::new(
                format!("Add WMB3 Material #{new_idx} ({shader_key})"),
                true,
            ))
        }
        "material_duplicate" => {
            let id = payload
                .get("id")
                .and_then(|v| v.as_str())
                .ok_or("Missing id for material_duplicate")?;
            let mi = uid_map
                .material_index_by_id(id)
                .ok_or_else(|| format!("Material '{id}' not found"))?;
            let mut cloned = wmb3.materials[mi].clone();
            cloned.name = format!("{}_copy", cloned.name);
            wmb3.materials.push(cloned);
            let new_uid = uid_map.alloc();
            uid_map.material_uids.push(new_uid);
            sync_wmb3_invariants(wmb3);
            Ok(MutationOutcome::new(
                format!("Duplicate WMB3 Material #{mi}"),
                true,
            ))
        }
        "material_delete" => {
            let id = payload
                .get("id")
                .and_then(|v| v.as_str())
                .ok_or("Missing id for material_delete")?;
            let del_idx = uid_map
                .material_index_by_id(id)
                .ok_or_else(|| format!("Material '{id}' not found"))?;
            if wmb3.materials.len() <= 1 {
                return Err("Cannot delete the only remaining material".into());
            }
            let reassign_id_opt = payload
                .get("reassignToId")
                .or_else(|| payload.get("reassignTo"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty());
            let reassign_idx_opt = match reassign_id_opt {
                Some(rid) => Some(
                    uid_map
                        .material_index_by_id(rid)
                        .ok_or_else(|| format!("Reassign target material '{rid}' not found"))?,
                ),
                None => None,
            };
            if reassign_idx_opt == Some(del_idx) {
                return Err("Cannot reassign deleted material to itself".into());
            }

            let is_used = wmb3.lods.iter().any(|l| {
                l.batch_infos
                    .iter()
                    .any(|inf| inf.material_index as usize == del_idx)
            });
            if is_used && reassign_idx_opt.is_none() {
                return Err(format!(
                    "WMB3 Material #{del_idx} is still used by one or more batches and no reassignToId was provided"
                ));
            }

            let remap_mat_idx = |cur: usize| -> Option<usize> {
                if cur == del_idx {
                    reassign_idx_opt.map(|r| if r > del_idx { r - 1 } else { r })
                } else if cur > del_idx {
                    Some(cur - 1)
                } else {
                    Some(cur)
                }
            };

            for lod in &mut wmb3.lods {
                for inf in &mut lod.batch_infos {
                    if let Some(mapped) = remap_mat_idx(inf.material_index as usize) {
                        inf.material_index = mapped as u32;
                    }
                }
            }
            for mesh in &mut wmb3.meshes {
                mesh.materials = mesh
                    .materials
                    .iter()
                    .filter_map(|&m| remap_mat_idx(m as usize).map(|v| v as u16))
                    .collect();
            }
            wmb3.mesh_material_pairs = wmb3
                .mesh_material_pairs
                .iter()
                .filter_map(|&(m, mat)| remap_mat_idx(mat as usize).map(|v| (m, v as u32)))
                .collect();

            let removed = wmb3.materials.remove(del_idx);
            uid_map.material_uids.remove(del_idx);
            sync_wmb3_invariants(wmb3);
            Ok(MutationOutcome::new(
                format!("Delete WMB3 Material '{}' (#{del_idx})", removed.name),
                true,
            ))
        }
        "material_rename" => {
            let id = payload
                .get("id")
                .and_then(|v| v.as_str())
                .ok_or("Missing id for material_rename")?;
            let name = payload
                .get("name")
                .and_then(|v| v.as_str())
                .ok_or("Missing name for material_rename")?
                .trim()
                .to_string();
            if name.is_empty() {
                return Err("Material name cannot be empty".into());
            }
            let mi = uid_map
                .material_index_by_id(id)
                .ok_or_else(|| format!("Material '{id}' not found"))?;
            let old_name = std::mem::replace(&mut wmb3.materials[mi].name, name.clone());
            Ok(MutationOutcome::new(
                format!("Rename WMB3 Material '{old_name}' to '{name}'"),
                true,
            ))
        }
        "paste_items" => {
            let clip = clipboard.ok_or("WMB clipboard is empty")?;
            if !clip.game.eq_ignore_ascii_case("b3") {
                return Err(format!(
                    "Cannot paste clipboard items from game '{}' into a 'b3' model",
                    clip.game
                ));
            }
            let target = payload.get("target").unwrap_or(payload);
            let explicit_target_kind = target.get("kind").and_then(|v| v.as_str());
            if let Some(tk) = explicit_target_kind {
                if tk == "materials" && clip.kind != "materials" && clip.materials_wmb3.is_empty() {
                    return Err("Clipboard does not contain WMB3 materials to paste".into());
                }
                if tk != "materials" && clip.kind == "materials" && clip.batches.is_empty() {
                    return Err(format!(
                        "Cannot paste clipboard materials into target kind '{tk}' (expected batches)"
                    ));
                }
            }
            let target_kind =
                explicit_target_kind.unwrap_or(if clip.kind == "materials" { "materials" } else { "lod" });
            let dedup_materials = target
                .get("deduplicate")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);

            let mut warnings = Vec::new();
            let core_pre_ids: BTreeSet<u32> = list_core_pretextures("b3")
                .into_iter()
                .map(|p| p.id)
                .collect();

            if clip.kind == "materials" || target_kind == "materials" {
                let mut pasted = 0usize;
                for src_mat in &clip.materials_wmb3 {
                    check_wmb3_material_missing_textures(
                        src_mat,
                        bank_texture_ids,
                        &core_pre_ids,
                        &mut warnings,
                    );
                    if dedup_materials && wmb3.materials.iter().any(|existing| existing == src_mat) {
                        pasted += 1;
                        continue;
                    }
                    let mut m = src_mat.clone();
                    if wmb3.materials.iter().any(|existing| existing.name == m.name) {
                        let base = m.name.clone();
                        let mut candidate = format!("{base}_pasted");
                        let mut suffix = 2usize;
                        while wmb3.materials.iter().any(|existing| existing.name == candidate) {
                            candidate = format!("{base}_pasted_{suffix}");
                            suffix += 1;
                        }
                        m.name = candidate;
                    }
                    wmb3.materials.push(m);
                    let new_uid = uid_map.alloc();
                    uid_map.material_uids.push(new_uid);
                    pasted += 1;
                }
                sync_wmb3_invariants(wmb3);
                return Ok(MutationOutcome::with_warnings(
                    format!("Paste {pasted} WMB3 material(s)"),
                    warnings,
                    true,
                ));
            }

            // Paste batches into target LOD / mesh group
            if wmb3.lods.is_empty() || wmb3.vertex_groups.is_empty() {
                return Err("Target WMB3 has no LODs or vertex groups to paste into".into());
            }
            let (target_mesh_idx, target_lod_level_opt) = if let Some(id_str) = target
                .get("id")
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
            {
                uid_map
                    .parse_lod_id(id_str)
                    .map(|(m, l)| (m, Some(l)))
                    .or_else(|| uid_map.mesh_index_by_id(id_str).map(|m| (m, None)))
                    .ok_or_else(|| format!("Target WMB3 mesh/LOD '{id_str}' not found"))?
            } else {
                (0, None)
            };

            let target_lod_idx = if let Some(lvl) = target_lod_level_opt {
                wmb3.lods
                    .iter()
                    .position(|l| l.lod_level == lvl)
                    .ok_or_else(|| format!("Target WMB3 LOD level {lvl} not found"))?
            } else {
                0
            };

            let mut unpacked = unpack_wmb3_lod_batches(wmb3, uid_map);
            let dst_vgi = unpacked
                .lod_lists
                .get(target_lod_idx)
                .and_then(|list| {
                    list.iter()
                        .find(|(_, inf, _)| inf.mesh_index as usize == target_mesh_idx)
                        .or_else(|| list.first())
                })
                .map(|(b, _, _)| b.vertex_group_index as usize)
                .unwrap_or(0)
                .min(wmb3.vertex_groups.len() - 1);

            let dst_vg_meta = &wmb3.vertex_groups[dst_vgi];
            let dst_layout = lookup_wmb3_vertex_layout_with_strides(
                dst_vg_meta.vertex_flags,
                dst_vg_meta.vertex_size,
                dst_vg_meta.vertex_ex_data_size,
            )
            .ok_or("Target WMB3 vertex group has an unsupported vertex layout")?;

            // Dedupe or append referenced materials by deep equality
            let mut mat_remap = Vec::with_capacity(clip.materials_wmb3.len());
            for src_mat in &clip.materials_wmb3 {
                check_wmb3_material_missing_textures(
                    src_mat,
                    bank_texture_ids,
                    &core_pre_ids,
                    &mut warnings,
                );
                if let Some(existing_idx) = wmb3.materials.iter().position(|m| m == src_mat) {
                    mat_remap.push(existing_idx as u32);
                } else {
                    let new_idx = wmb3.materials.len() as u32;
                    wmb3.materials.push(src_mat.clone());
                    let new_uid = uid_map.alloc();
                    uid_map.material_uids.push(new_uid);
                    mat_remap.push(new_idx);
                }
            }

            let mut dropped_bones = BTreeSet::new();
            let mut dropped_attrs = BTreeSet::new();
            let mut pasted_tuples = Vec::with_capacity(clip.batches.len());

            for cb in &clip.batches {
                if !clip.materials_wmb3.is_empty() && cb.material_clipboard_idx >= mat_remap.len() {
                    return Err(format!(
                        "Clipboard batch references out-of-range material_clipboard_idx {}",
                        cb.material_clipboard_idx
                    ));
                }
                let mut new_verts = Vec::with_capacity(cb.vertices.len());
                for cv in &cb.vertices {
                    let mut v = cv.vertex.clone();
                    if !cv.global_bone_weights.is_empty() {
                        let mut mapped_pairs = Vec::new();
                        for &(gid, weight) in &cv.global_bone_weights {
                            if let Some(bone_arr_idx) =
                                wmb3.bones.iter().position(|b| b.id as i32 == gid)
                            {
                                let bm_pos = if let Some(pos) = wmb3
                                    .bone_map
                                    .iter()
                                    .position(|&bm| bm as usize == bone_arr_idx)
                                {
                                    pos
                                } else {
                                    if wmb3.bone_map.len() >= 256 {
                                        return Err(
                                            "WMB3 bone_map exceeds 256-entry u8 vertex bone index limit"
                                                .into(),
                                        );
                                    }
                                    wmb3.bone_map.push(bone_arr_idx as u32);
                                    wmb3.bone_map.len() - 1
                                };
                                let bm_idx = u8::try_from(bm_pos).map_err(|_| {
                                    format!("WMB3 bone_map index {bm_pos} exceeds u8 limit (255)")
                                })?;
                                mapped_pairs.push((bm_idx, weight));
                            } else {
                                dropped_bones.insert(gid);
                            }
                        }
                        if mapped_pairs.is_empty() {
                            mapped_pairs.push((0, 255));
                        }
                        let mut bi = BoneInfos::default();
                        bi.set_active_pairs(&mapped_pairs);
                        v.bone_infos = Some(bi);
                    }
                    if v.bone_infos.is_some() && !wmb3_layout_has_field(dst_layout, "bone_infos") {
                        dropped_attrs.insert("bone_infos");
                    }
                    if v.color.is_some() && !wmb3_layout_has_field(dst_layout, "color") {
                        dropped_attrs.insert("color");
                    }
                    if v.mapping2.is_some() && !wmb3_layout_has_field(dst_layout, "mapping2") {
                        dropped_attrs.insert("mapping2");
                    }
                    if v.mapping3.is_some() && !wmb3_layout_has_field(dst_layout, "mapping3") {
                        dropped_attrs.insert("mapping3");
                    }
                    new_verts.push(roundtrip_vertex_wmb3(&v, dst_layout));
                }

                let dst_vg = &mut wmb3.vertex_groups[dst_vgi];
                let v_start = dst_vg.vertexes.len() as u32;
                let i_start = dst_vg.indices.len() as u32;
                let num_v = new_verts.len() as u32;
                dst_vg.vertexes.extend(new_verts);
                for &[a, b, c] in &cb.triangles {
                    dst_vg.indices.push(a);
                    dst_vg.indices.push(b);
                    dst_vg.indices.push(c);
                }
                let num_i = (cb.triangles.len() * 3) as u32;
                dst_vg.num_vertexes = dst_vg.vertexes.len() as u32;
                dst_vg.num_indices = dst_vg.indices.len() as u32;

                let mat_idx = mat_remap
                    .get(cb.material_clipboard_idx)
                    .copied()
                    .unwrap_or(0);
                let new_batch = Wmb3Batch {
                    vertex_group_index: dst_vgi as u32,
                    bone_set_index: -1,
                    vertex_start: v_start,
                    index_start: i_start,
                    num_vertexes: num_v,
                    num_indices: num_i,
                    num_primitives: cb.triangles.len() as u32,
                };
                let new_inf = Wmb3LodBatchInfo {
                    vertex_group_index: dst_vgi as u32,
                    mesh_index: target_mesh_idx as u32,
                    material_index: mat_idx,
                    call_tree_node_index: -1,
                    mesh_material_index: 0,
                    u_b: 0,
                };
                let new_uid = uid_map.alloc();
                pasted_tuples.push((new_batch, new_inf, new_uid));
            }

            if !dropped_bones.is_empty() {
                warnings.push(format!(
                    "Dropped influences for missing global bone IDs: {:?}",
                    dropped_bones
                ));
            }
            if !dropped_attrs.is_empty() {
                let attrs: Vec<&str> = dropped_attrs.into_iter().collect();
                warnings.push(format!(
                    "Target WMB3 vertex group omitted attribute(s): {}",
                    attrs.join(", ")
                ));
            }

            let count = pasted_tuples.len();
            let req_index = target
                .get("index")
                .and_then(|v| v.as_u64())
                .map(|v| v as usize);
            let dst_list = &mut unpacked.lod_lists[target_lod_idx];
            if let Some(req_local) = req_index {
                let mesh_positions: Vec<usize> = dst_list
                    .iter()
                    .enumerate()
                    .filter_map(|(idx, (_, inf, _))| {
                        if inf.mesh_index as usize == target_mesh_idx {
                            Some(idx)
                        } else {
                            None
                        }
                    })
                    .collect();
                let abs_insert_pos = if mesh_positions.is_empty() {
                    dst_list.len()
                } else if req_local < mesh_positions.len() {
                    mesh_positions[req_local]
                } else {
                    mesh_positions.last().copied().unwrap() + 1
                };
                for (offset, item) in pasted_tuples.into_iter().enumerate() {
                    dst_list.insert(abs_insert_pos + offset, item);
                }
            } else {
                dst_list.extend(pasted_tuples);
            }
            repack_wmb3_lod_batches(wmb3, uid_map, unpacked);

            Ok(MutationOutcome::with_warnings(
                format!("Paste {count} WMB3 batch(es)"),
                warnings,
                true,
            ))
        }
        other => Err(format!("Unsupported WMB3 scene operation: {other}")),
    }
}

/// Copy batches or materials from a WMB0 / WMB1 / WMB2 model into a self-contained `WmbClipboardData`.
pub fn copy_wmb_to_clipboard(
    wmb: &WmbFile,
    uid_map: &mut UidMap,
    items: &[String],
) -> std::result::Result<WmbClipboardData, String> {
    if items.is_empty() {
        return Err("No items specified to copy".into());
    }
    uid_map.ensure_wmb(wmb);
    let is_b2 = wmb.is_bayo2();
    let game = if is_b2 { "b2" } else { "b1" }.to_string();

    if items.iter().all(|s| s.trim().starts_with("mat:")) {
        let mut mats = Vec::with_capacity(items.len());
        for id in items {
            let mi = uid_map
                .material_index_by_id(id)
                .ok_or_else(|| format!("Material '{id}' not found"))?;
            mats.push(wmb.materials[mi].clone());
        }
        return Ok(WmbClipboardData {
            game,
            kind: "materials".to_string(),
            batches: Vec::new(),
            materials_wmb: mats,
            materials_wmb3: Vec::new(),
        });
    }

    let inv_btt = wmb.bone_index_translate_table.invert();
    let mut copied_batches = Vec::with_capacity(items.len());
    let mut copied_mats: Vec<WmbMaterial> = Vec::new();

    for id in items {
        let (mi, bi) = uid_map
            .wmb_batch_pos_by_id(id)
            .ok_or_else(|| format!("Batch '{id}' not found"))?;
        let batch = &wmb.meshes[mi].batches[bi];
        let mat_idx = batch
            .header
            .effective_material_id(is_b2)
            .min(wmb.materials.len().saturating_sub(1));
        let mat = wmb
            .materials
            .get(mat_idx)
            .cloned()
            .unwrap_or_else(|| WmbMaterial {
                mat_type: 0,
                flag: 0,
                shader_name: None,
                material_data: Vec::new(),
            });
        let clip_mat_idx = if let Some(pos) = copied_mats.iter().position(|m| m == &mat) {
            pos
        } else {
            copied_mats.push(mat);
            copied_mats.len() - 1
        };

        let tris = batch.triangles();
        let mut old_to_new: BTreeMap<usize, u32> = BTreeMap::new();
        let mut clip_verts = Vec::new();
        let mut local_tris = Vec::with_capacity(tris.len());

        for [i0, i1, i2] in tris {
            let mut map_one = |abs_vi: usize| -> u32 {
                if let Some(&existing) = old_to_new.get(&abs_vi) {
                    return existing;
                }
                let next = clip_verts.len() as u32;
                old_to_new.insert(abs_vi, next);
                let mut v = wmb.vertexes.get(abs_vi).cloned().unwrap_or_default();
                if v.position.is_none() {
                    v.position = wmb.positions.get(abs_vi).copied().or(Some(Vec3::ZERO));
                }
                let mut global_bone_weights = Vec::new();
                if let Some(bi_info) = v.bone_infos {
                    for (pal_idx, w) in bi_info.active_pairs() {
                        let local_bone = batch
                            .bone_refs
                            .get(pal_idx as usize)
                            .copied()
                            .unwrap_or(pal_idx) as u16;
                        let gid = inv_btt
                            .get(&local_bone)
                            .copied()
                            .unwrap_or(local_bone) as i32;
                        global_bone_weights.push((gid, w));
                    }
                }
                clip_verts.push(ClipboardVertex {
                    vertex: v,
                    global_bone_weights,
                });
                next
            };
            let a = map_one(i0);
            let b = map_one(i1);
            let c = map_one(i2);
            local_tris.push([a, b, c]);
        }

        copied_batches.push(ClipboardBatchItem {
            vertices: clip_verts,
            triangles: local_tris,
            material_clipboard_idx: clip_mat_idx,
            is_shadow_proxy: batch.header.is_shadow_proxy(is_b2),
            wmb_header_template: Some(batch.header.clone()),
        });
    }

    Ok(WmbClipboardData {
        game,
        kind: "batches".to_string(),
        batches: copied_batches,
        materials_wmb: copied_mats,
        materials_wmb3: Vec::new(),
    })
}

/// Copy batches or materials from a WMB3 model into a self-contained `WmbClipboardData`.
pub fn copy_wmb3_to_clipboard(
    wmb3: &Wmb3File,
    uid_map: &mut UidMap,
    items: &[String],
) -> std::result::Result<WmbClipboardData, String> {
    if items.is_empty() {
        return Err("No items specified to copy".into());
    }
    uid_map.ensure_wmb3(wmb3);

    if items.iter().all(|s| s.trim().starts_with("mat:")) {
        let mut mats = Vec::with_capacity(items.len());
        for id in items {
            let mi = uid_map
                .material_index_by_id(id)
                .ok_or_else(|| format!("Material '{id}' not found"))?;
            mats.push(wmb3.materials[mi].clone());
        }
        return Ok(WmbClipboardData {
            game: "b3".to_string(),
            kind: "materials".to_string(),
            batches: Vec::new(),
            materials_wmb: Vec::new(),
            materials_wmb3: mats,
        });
    }

    let mut copied_batches = Vec::with_capacity(items.len());
    let mut copied_mats: Vec<Wmb3Material> = Vec::new();

    for id in items {
        let bi = uid_map
            .wmb3_batch_index_by_id(id)
            .ok_or_else(|| format!("Batch '{id}' not found"))?;
        let batch = &wmb3.batches[bi];
        let mut mat_idx = 0usize;
        for lod in &wmb3.lods {
            let start = lod.batch_start as usize;
            let end = start + lod.batch_infos.len();
            if bi >= start && bi < end {
                mat_idx = lod.batch_infos[bi - start].material_index as usize;
                break;
            }
        }
        let mat = wmb3
            .materials
            .get(mat_idx)
            .cloned()
            .unwrap_or_else(|| Wmb3Material {
                date: [2026, 10, 29, 0],
                name: "Default".to_string(),
                shader_name: "PBS00".to_string(),
                technique_name: "Default".to_string(),
                u_a: 1,
                textures: Vec::new(),
                parameter_groups: Vec::new(),
                variables: Vec::new(),
            });
        let clip_mat_idx = if let Some(pos) = copied_mats.iter().position(|m| m == &mat) {
            pos
        } else {
            copied_mats.push(mat);
            copied_mats.len() - 1
        };

        let vgi = (batch.vertex_group_index as usize).min(wmb3.vertex_groups.len().saturating_sub(1));
        let vg = wmb3
            .vertex_groups
            .get(vgi)
            .ok_or("WMB3 batch vertex group out of bounds")?;
        let v_len = vg.vertexes.len();
        let start = (batch.index_start as usize).min(vg.indices.len());
        let end = (start + batch.num_indices as usize).min(vg.indices.len());

        let bone_refs: Vec<usize> = if batch.bone_set_index >= 0
            && (batch.bone_set_index as usize) < wmb3.bone_sets.len()
        {
            wmb3.bone_sets[batch.bone_set_index as usize]
                .iter()
                .map(|&b_set_val| *wmb3.bone_map.get(b_set_val as usize).unwrap_or(&0) as usize)
                .collect()
        } else {
            wmb3.bone_map.iter().map(|&bm| bm as usize).collect()
        };

        let mut old_to_new: BTreeMap<usize, u32> = BTreeMap::new();
        let mut clip_verts = Vec::new();
        let mut local_tris = Vec::new();

        for tri in vg.indices[start..end].chunks_exact(3) {
            let resolve_vi = |raw: u32| -> usize {
                let with_start = (raw as usize).saturating_add(batch.vertex_start as usize);
                if with_start < v_len {
                    with_start
                } else {
                    raw as usize
                }
            };
            let i0 = resolve_vi(tri[0]);
            let i1 = resolve_vi(tri[1]);
            let i2 = resolve_vi(tri[2]);
            if i0 >= v_len || i1 >= v_len || i2 >= v_len {
                continue;
            }
            let mut map_one = |abs_vi: usize| -> u32 {
                if let Some(&existing) = old_to_new.get(&abs_vi) {
                    return existing;
                }
                let next = clip_verts.len() as u32;
                old_to_new.insert(abs_vi, next);
                let v = vg.vertexes[abs_vi].clone();
                let mut global_bone_weights = Vec::new();
                if let Some(bi_info) = v.bone_infos {
                    for (local_bi, w) in bi_info.active_pairs() {
                        let bone_arr_idx = bone_refs
                            .get(local_bi as usize)
                            .copied()
                            .unwrap_or(local_bi as usize);
                        let gid = wmb3
                            .bones
                            .get(bone_arr_idx)
                            .map(|b| b.id as i32)
                            .unwrap_or(bone_arr_idx as i32);
                        global_bone_weights.push((gid, w));
                    }
                }
                clip_verts.push(ClipboardVertex {
                    vertex: v,
                    global_bone_weights,
                });
                next
            };
            let a = map_one(i0);
            let b = map_one(i1);
            let c = map_one(i2);
            local_tris.push([a, b, c]);
        }

        copied_batches.push(ClipboardBatchItem {
            vertices: clip_verts,
            triangles: local_tris,
            material_clipboard_idx: clip_mat_idx,
            is_shadow_proxy: false,
            wmb_header_template: None,
        });
    }

    Ok(WmbClipboardData {
        game: "b3".to_string(),
        kind: "batches".to_string(),
        batches: copied_batches,
        materials_wmb: Vec::new(),
        materials_wmb3: copied_mats,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bone::BoneIndexTranslateTable;
    use crate::dat::DatFile;
    use crate::demo::build_demo_dat;
    use crate::scene::{build_wmb3_scene, build_wmb_scene, pack_scene_blob, unpack_scene_blob};
    use crate::wmb::{PackedNormal, PackedTangents, WmbHeader};
    use crate::wmb3::{Wmb3Bone, Wmb3Lod, Wmb3VertexGroup};

    fn make_synthetic_wmb0(is_b2: bool) -> WmbFile {
        // 2 meshes, 4 batches (including 1 shadow proxy), 2 materials, 3 bones
        let mut verts = Vec::new();
        for i in 0..12 {
            let mut bi = BoneInfos::default();
            bi.set_active_pairs(&[((i % 3) as u8, 255)]);
            verts.push(WmbVertex {
                position: Some(Vec3::new(i as f32 * 0.5, (i % 3) as f32, (i / 3) as f32)),
                position2: None,
                normal: PackedNormal {
                    normal: [0.0, 1.0, 0.0],
                    wide: is_b2,
                    raw_bytes: if is_b2 {
                        PackedNormal::pack_wide([0.0, 1.0, 0.0]).to_le_bytes()
                    } else {
                        PackedNormal::pack_narrow([0.0, 1.0, 0.0])
                    },
                },
                float_normal: None,
                half_normal: None,
                tangents: PackedTangents::default(),
                mapping: [0.25, 0.75],
                mapping2: None,
                mapping3: None,
                mapping4: None,
                mapping5: None,
                color: Some([255, 255, 255, 255]),
                bone_infos: Some(bi),
            });
        }

        let make_batch = |mesh_id: i16,
                          batch_id: i16,
                          mat_id: usize,
                          tri: [usize; 3],
                          shadow: bool|
         -> WmbBatch {
            let mut hdr = WmbBatchHeader {
                batch_id,
                mesh_id,
                flags: 0x8001,
                ex_mat_id: if is_b2 { mat_id as i16 } else { 0 },
                material_id: mat_id as u8,
                has_bone_refs: 1,
                u_e1: 0,
                u_e2: 0,
                vertex_start: 0,
                vertex_end: 0,
                primitive_type: 4,
                offset_indices: 0x60,
                num_indices: 0,
                vertex_offset: 0,
                u_f: [0; 7],
            };
            hdr.set_shadow_proxy(is_b2, shadow);
            let mut b = WmbBatch {
                header: hdr,
                bone_refs: vec![0, 1, 2],
                unknown_no_bones: [0.0; 4],
                indices: Vec::new(),
            };
            b.set_triangles(&[tri]);
            b.header.offset_indices = b.effective_indices_offset();
            b
        };

        let mut m0 = WmbMaterial {
            mat_type: 110,
            flag: 0,
            shader_name: if is_b2 {
                Some("sis00_bxxxn".to_string())
            } else {
                None
            },
            material_data: vec![0; 52],
        };
        let _ = switch_wmb_material_shader(
            &mut m0,
            0,
            is_b2,
            if is_b2 { "sis00_bxxxn" } else { "110" },
        );
        let mut m1 = WmbMaterial {
            mat_type: 68,
            flag: 0,
            shader_name: if is_b2 {
                Some("chr00_bxxxx".to_string())
            } else {
                None
            },
            material_data: vec![0; 52],
        };
        let _ = switch_wmb_material_shader(
            &mut m1,
            1,
            is_b2,
            if is_b2 { "chr00_bxxxx" } else { "68" },
        );

        let meshes = vec![
            WmbMesh {
                id: 0,
                u_a1: 0,
                u_a2: 0,
                offset_batch_offsets: 0x80,
                u_b: 0,
                u_c: [0; 4],
                name: "mesh_a".to_string(),
                bounding_matrix: [0.0; 12],
                batches: vec![
                    make_batch(0, 0, 0, [0, 1, 2], false),
                    make_batch(0, 1, 0, [3, 4, 5], true), // shadow proxy
                ],
            },
            WmbMesh {
                id: 1,
                u_a1: 0,
                u_a2: 0,
                offset_batch_offsets: 0x80,
                u_b: 0,
                u_c: [0; 4],
                name: "mesh_b".to_string(),
                bounding_matrix: [0.0; 12],
                batches: vec![
                    make_batch(1, 0, 1, [6, 7, 8], false),
                    make_batch(1, 1, 1, [9, 10, 11], false),
                ],
            },
        ];

        let mut btt = BoneIndexTranslateTable::new();
        btt.table.insert(10, 0);
        btt.table.insert(20, 1);
        btt.table.insert(30, 2);

        let mut wmb = WmbFile {
            endian: Endian::Little,
            header: WmbHeader {
                u_a: 0,
                vertex_flags: 0x4800_001F,
                num_vertexes: verts.len() as i32,
                vertex_ex_data_size: 1,
                vertex_ex_data: 1,
                u_e: 0,
                offset_positions: 0,
                offset_vertexes: 0,
                offset_vertexes_ex_data: 0,
                u_g: [0; 4],
                num_bones: 3,
                offset_bone_hierarchy: 0,
                offset_bone_relative_position: 0,
                offset_bone_position: 0,
                offset_bone_index_translate_table: 0,
                num_materials: 2,
                offset_materials_offsets: 0,
                offset_materials: 0,
                num_meshes: 2,
                offset_meshes_offsets: 0,
                offset_meshes: 0,
                u_k: 0,
                u_l: 0,
                offset_u_j: 0,
                offset_bone_symmetries: 0,
                offset_bone_flags: 0,
                offset_shader_names: if is_b2 { 1 } else { 0 },
                offset_tex_infos: if is_b2 { 1 } else { 0 },
                u_m: 0,
                u_n: 0,
            },
            positions: Vec::new(),
            vertexes: verts,
            bone_hierarchy: vec![-1, 0, 1],
            bone_relative_positions: vec![Vec3::ZERO; 3],
            bone_positions: vec![
                Vec3::new(0.0, 0.0, 0.0),
                Vec3::new(0.0, 1.0, 0.0),
                Vec3::new(0.0, 2.0, 0.0),
            ],
            bone_index_translate_table: btt,
            u_j_raw: None,
            bone_symmetries: vec![-1; 3],
            bone_flags: vec![5; 3],
            tex_infos: Vec::new(),
            materials: vec![m0, m1],
            meshes,
        };
        wmb.recompute_relative_positions();
        wmb
    }

    fn make_synthetic_wmb3() -> Wmb3File {
        // 2 meshes, 2 LODs, 3 batches in 2 vertex groups, 2 materials, 3 bones
        let make_verts = |count: usize, offset_x: f32| -> Vec<WmbVertex> {
            (0..count)
                .map(|i| {
                    let mut bi = BoneInfos::default();
                    bi.set_active_pairs(&[((i % 3) as u8, 255)]);
                    WmbVertex {
                        position: Some(Vec3::new(offset_x + i as f32, (i % 2) as f32, (i / 2) as f32)),
                        position2: None,
                        normal: PackedNormal {
                            normal: [0.0, 0.0, 1.0],
                            wide: true,
                            raw_bytes: PackedNormal::pack_wide([0.0, 0.0, 1.0]).to_le_bytes(),
                        },
                        float_normal: None,
                        half_normal: Some([0.0, 0.0, 1.0, 0.0]),
                        tangents: PackedTangents::default(),
                        mapping: [0.1 * i as f32, 0.2 * i as f32],
                        mapping2: Some([0.5, 0.5]),
                        mapping3: None,
                        mapping4: None,
                        mapping5: None,
                        color: Some([200, 200, 200, 255]),
                        bone_infos: Some(bi),
                    }
                })
                .collect()
        };

        // VG 0: flags=7 (has bone_infos + mapping2 + normal)
        // VG 1: flags=0 (position + tangents + mapping + normal, NO bone_infos / mapping2)
        let vg0 = Wmb3VertexGroup {
            offset_vertexes: 0,
            offset_vertexes_ex_data: 0,
            u_a: 0,
            u_b: 0,
            vertex_size: 28,
            vertex_ex_data_size: 12,
            u_c: 0,
            u_d: 0,
            num_vertexes: 6,
            vertex_flags: 7,
            offset_indices: 0,
            num_indices: 6,
            vertexes: make_verts(6, 0.0),
            indices: vec![0, 1, 2, 0, 1, 2],
            raw_vertex_bytes: Vec::new(),
            raw_ex_bytes: Vec::new(),
        };
        let vg1 = Wmb3VertexGroup {
            offset_vertexes: 0,
            offset_vertexes_ex_data: 0,
            u_a: 0,
            u_b: 0,
            vertex_size: 28,
            vertex_ex_data_size: 0,
            u_c: 0,
            u_d: 0,
            num_vertexes: 3,
            vertex_flags: 0,
            offset_indices: 0,
            num_indices: 3,
            vertexes: make_verts(3, 5.0),
            indices: vec![0, 1, 2],
            raw_vertex_bytes: Vec::new(),
            raw_ex_bytes: Vec::new(),
        };

        let batches = vec![
            Wmb3Batch {
                vertex_group_index: 0,
                bone_set_index: 0,
                vertex_start: 0,
                index_start: 0,
                num_vertexes: 3,
                num_indices: 3,
                num_primitives: 1,
            },
            Wmb3Batch {
                vertex_group_index: 0,
                bone_set_index: 0,
                vertex_start: 3,
                index_start: 3,
                num_vertexes: 3,
                num_indices: 3,
                num_primitives: 1,
            },
            Wmb3Batch {
                vertex_group_index: 1,
                bone_set_index: -1,
                vertex_start: 0,
                index_start: 0,
                num_vertexes: 3,
                num_indices: 3,
                num_primitives: 1,
            },
        ];

        let lods = vec![
            Wmb3Lod {
                name: "lod0".to_string(),
                lod_level: 0,
                batch_start: 0,
                batch_infos: vec![
                    Wmb3LodBatchInfo {
                        vertex_group_index: 0,
                        mesh_index: 0,
                        material_index: 0,
                        call_tree_node_index: -1,
                        mesh_material_index: 0,
                        u_b: 0,
                    },
                    Wmb3LodBatchInfo {
                        vertex_group_index: 0,
                        mesh_index: 1,
                        material_index: 1,
                        call_tree_node_index: -1,
                        mesh_material_index: 1,
                        u_b: 0,
                    },
                ],
            },
            Wmb3Lod {
                name: "lod1".to_string(),
                lod_level: 1,
                batch_start: 2,
                batch_infos: vec![Wmb3LodBatchInfo {
                    vertex_group_index: 1,
                    mesh_index: 1,
                    material_index: 1,
                    call_tree_node_index: -1,
                    mesh_material_index: 1,
                    u_b: 0,
                }],
            },
        ];

        let mut mat0 = Wmb3Material {
            date: [2026, 10, 29, 0],
            name: "mat0".to_string(),
            shader_name: "PBS00_XXXXX".to_string(),
            technique_name: "Default".to_string(),
            u_a: 1,
            textures: vec![("g_AlbedoMap".to_string(), 0x1111_2222)],
            parameter_groups: Vec::new(),
            variables: Vec::new(),
        };
        switch_wmb3_material_shader(&mut mat0, "PBS00_XXXXX");

        let mut mat1 = Wmb3Material {
            date: [2026, 10, 29, 0],
            name: "mat1".to_string(),
            shader_name: "PBS00_XXXXX".to_string(),
            technique_name: "Default".to_string(),
            u_a: 1,
            textures: vec![("g_AlbedoMap".to_string(), 0x3333_4444)],
            parameter_groups: Vec::new(),
            variables: Vec::new(),
        };
        switch_wmb3_material_shader(&mut mat1, "PBS00_XXXXX");

        let bones = vec![
            Wmb3Bone {
                id: 10,
                parent_index: -1,
                local_position: Vec3::ZERO,
                local_rotation: Vec3::ZERO,
                local_scale: Vec3::ONE,
                position: Vec3::ZERO,
                rotation: Vec3::ZERO,
                scale: Vec3::ONE,
                t_position: Vec3::ZERO,
            },
            Wmb3Bone {
                id: 20,
                parent_index: 0,
                local_position: Vec3::new(0.0, 1.0, 0.0),
                local_rotation: Vec3::ZERO,
                local_scale: Vec3::ONE,
                position: Vec3::new(0.0, 1.0, 0.0),
                rotation: Vec3::ZERO,
                scale: Vec3::ONE,
                t_position: Vec3::new(0.0, 1.0, 0.0),
            },
            Wmb3Bone {
                id: 30,
                parent_index: 1,
                local_position: Vec3::new(0.0, 1.0, 0.0),
                local_rotation: Vec3::ZERO,
                local_scale: Vec3::ONE,
                position: Vec3::new(0.0, 2.0, 0.0),
                rotation: Vec3::ZERO,
                scale: Vec3::ONE,
                t_position: Vec3::new(0.0, 2.0, 0.0),
            },
        ];

        Wmb3File {
            version: 0x2020_0824,
            u_a: 0,
            flags: 0,
            u_c: 0,
            bounding_box: [-1.0, -1.0, -1.0, 6.0, 3.0, 3.0],
            bones,
            bone_index_translate_table: BoneIndexTranslateTable::new(),
            raw_btt: Vec::new(),
            vertex_groups: vec![vg0, vg1],
            batches,
            lods,
            col_tree_nodes: Vec::new(),
            bone_map: vec![0, 1, 2],
            bone_sets: vec![vec![0, 1, 2]],
            materials: vec![mat0, mat1],
            meshes: vec![
                Wmb3Mesh {
                    name: "mesh0".to_string(),
                    bounding_box: [0.0; 6],
                    materials: vec![0],
                    bone_indices: vec![0, 1],
                },
                Wmb3Mesh {
                    name: "mesh1".to_string(),
                    bounding_box: [0.0; 6],
                    materials: vec![1],
                    bone_indices: vec![2],
                },
            ],
            mesh_material_pairs: vec![(0, 0), (1, 1)],
            world_data: Vec::new(),
            tail: [0, 0],
        }
    }

    #[test]
    fn test_scene_export_and_hash_stability() {
        let wmb = make_synthetic_wmb0(false);
        let mut uid_map = UidMap::new();
        let (m1, bin1) = build_wmb_scene(&wmb, &mut uid_map, "root", "test.wmb", 1, Vec::new(), &[]);
        let (m2, bin2) = build_wmb_scene(&wmb, &mut uid_map, "root", "test.wmb", 1, Vec::new(), &[]);
        assert_eq!(m1.mesh_groups.len(), 2);
        assert_eq!(m1.batches.len(), 4);
        assert_eq!(m1.materials.len(), 2);
        assert_eq!(m1.bones.len(), 3);
        assert!(m1.batches[1].is_shadow_proxy);
        assert!(!m1.batches[0].is_shadow_proxy);
        assert_eq!(m1.batches[0].file_order, 0);
        assert_eq!(m1.batches[3].file_order, 3);
        assert_eq!(m1.bones[0].global_id, 10);
        assert_eq!(m1.bones[2].global_id, 30);
        assert_eq!(m1.vertex_buffers[0].geometry_hash, m2.vertex_buffers[0].geometry_hash);
        assert_eq!(bin1, bin2);

        let blob = pack_scene_blob(&m1, &bin1).unwrap();
        let (unpacked_m, unpacked_bin) = unpack_scene_blob(&blob).unwrap();
        assert_eq!(unpacked_m.batches.len(), 4);
        assert_eq!(unpacked_bin, bin1);
    }

    #[test]
    fn test_wmb0_mutations_and_shadow_last() {
        let mut wmb = make_synthetic_wmb0(false);
        let mut uid_map = UidMap::new();
        uid_map.ensure_wmb(&wmb);

        let b2_id = format!("batch:{}", uid_map.wmb_batch_uids[1][0]);
        let mesh0_id = format!("mesh:{}", uid_map.mesh_uids[0]);

        // Move batch from mesh 1 to mesh 0 at index 2 (past the shadow batch -> must clamp before shadow batch!)
        let payload = serde_json::json!({
            "items": [b2_id],
            "target": { "kind": "mesh", "id": mesh0_id, "index": 2 }
        });
        apply_wmb_scene_op(&mut wmb, &mut uid_map, "move_items", &payload, None, None).unwrap();
        assert_eq!(wmb.meshes[0].batches.len(), 3);
        assert!(!wmb.meshes[0].batches[0].header.is_shadow_proxy(false));
        assert!(!wmb.meshes[0].batches[1].header.is_shadow_proxy(false));
        assert!(wmb.meshes[0].batches[2].header.is_shadow_proxy(false));

        // Round-trip check
        let bytes = wmb.to_bytes(Endian::Little).unwrap();
        let reparsed = WmbFile::from_bytes(&bytes).unwrap();
        assert_eq!(reparsed.meshes[0].batches.len(), 3);
    }

    #[test]
    fn test_wmb3_cross_lod_move_and_reencode() {
        let mut wmb3 = make_synthetic_wmb3();
        let mut uid_map = UidMap::new();
        uid_map.ensure_wmb3(&wmb3);

        let b0_uid = uid_map.wmb3_batch_uids[0];
        let b0_id = format!("batch:{b0_uid}");
        let mesh1_uid = uid_map.mesh_uids[1];
        let target_lod1 = format!("lod:{mesh1_uid}/1");

        // Move b0 (currently in VG 0, LOD 0) into mesh 1 LOD 1 (which uses VG 1 -> triggers re-encoding!)
        let payload = serde_json::json!({
            "items": [b0_id],
            "target": { "kind": "lod", "id": target_lod1, "index": 1 }
        });
        let outcome =
            apply_wmb3_scene_op(&mut wmb3, &mut uid_map, "move_items", &payload, None, None)
                .unwrap();
        assert!(!outcome.warnings.is_empty());
        assert_eq!(wmb3.lods[0].batch_infos.len(), 1);
        assert_eq!(wmb3.lods[1].batch_infos.len(), 2);

        // Verify UID was preserved on the moved batch
        let new_pos = uid_map.wmb3_batch_index_by_id(&b0_id).unwrap();
        assert_eq!(wmb3.batches[new_pos].vertex_group_index, 1);

        // Round-trip check
        let bytes = wmb3.to_bytes().unwrap();
        let reparsed = Wmb3File::from_bytes(&bytes).unwrap();
        assert_eq!(reparsed.batches.len(), 3);
    }

    #[test]
    fn test_demo_models_and_clipboard() {
        for game in ["b1", "b2", "b3"] {
            let dat_bytes = build_demo_dat(game).unwrap();
            let dat = DatFile::from_bytes(&dat_bytes).unwrap();
            assert_eq!(dat.entries.len(), 2);
            let wmb_entry = &dat.entries[0];
            if game == "b3" {
                let wmb3 = Wmb3File::from_bytes(&wmb_entry.data).unwrap();
                let mut uid_map = UidMap::new();
                let (manifest, _) =
                    build_wmb3_scene(&wmb3, &mut uid_map, "dat:0", "demo.wmb", 1, Vec::new(), &[]);
                assert_eq!(manifest.mesh_groups.len(), 2);
                assert_eq!(manifest.lod_levels, vec![0, 1]);
                assert_eq!(manifest.bones.len(), 4);
            } else {
                let wmb = WmbFile::from_bytes(&wmb_entry.data).unwrap();
                let mut uid_map = UidMap::new();
                let (manifest, _) =
                    build_wmb_scene(&wmb, &mut uid_map, "dat:0", "demo.wmb", 1, Vec::new(), &[]);
                assert_eq!(manifest.mesh_groups.len(), 2);
                assert_eq!(manifest.batches.len(), 5);
                assert_eq!(manifest.bones.len(), 4);
            }
        }

        // Clipboard copy from WMB0 and paste into second WMB0 of same game; reject different game
        let wmb_src = make_synthetic_wmb0(false);
        let mut src_uids = UidMap::new();
        src_uids.ensure_wmb(&wmb_src);
        let batch_id = format!("batch:{}", src_uids.wmb_batch_uids[0][0]);
        let clip = copy_wmb_to_clipboard(&wmb_src, &mut src_uids, &[batch_id]).unwrap();
        assert_eq!(clip.game, "b1");
        assert_eq!(clip.batches.len(), 1);

        let mut wmb_dst = make_synthetic_wmb0(false);
        let mut dst_uids = UidMap::new();
        dst_uids.ensure_wmb(&wmb_dst);
        let dst_mesh0 = format!("mesh:{}", dst_uids.mesh_uids[0]);
        let paste_payload = serde_json::json!({
            "target": { "kind": "mesh", "id": dst_mesh0, "index": 0 }
        });
        apply_wmb_scene_op(
            &mut wmb_dst,
            &mut dst_uids,
            "paste_items",
            &paste_payload,
            Some(&clip),
            None,
        )
        .unwrap();
        assert_eq!(wmb_dst.meshes[0].batches.len(), 3);

        // Reject pasting b1 clipboard into b2 or b3
        let mut wmb_b2 = make_synthetic_wmb0(true);
        let mut b2_uids = UidMap::new();
        assert!(apply_wmb_scene_op(
            &mut wmb_b2,
            &mut b2_uids,
            "paste_items",
            &paste_payload,
            Some(&clip),
            None,
        )
        .is_err());
    }

    #[test]
    fn test_mesh_reorder_material_delete_and_multi_move() {
        // 1. WMB0 multi-item move, mesh reorder, and material delete with remap
        let mut wmb = make_synthetic_wmb0(false);
        let mut uid_map = UidMap::new();
        uid_map.ensure_wmb(&wmb);

        let b2_id = format!("batch:{}", uid_map.wmb_batch_uids[1][0]);
        let b3_id = format!("batch:{}", uid_map.wmb_batch_uids[1][1]);
        let mesh0_id = format!("mesh:{}", uid_map.mesh_uids[0]);
        let mesh1_id = format!("mesh:{}", uid_map.mesh_uids[1]);

        // Multi-item move of both mesh 1 batches into mesh 0
        let move_payload = serde_json::json!({
            "items": [b2_id, b3_id],
            "target": { "kind": "mesh", "id": mesh0_id, "index": 0 }
        });
        apply_wmb_scene_op(&mut wmb, &mut uid_map, "move_items", &move_payload, None, None).unwrap();
        assert_eq!(wmb.meshes[0].batches.len(), 4);
        assert_eq!(wmb.meshes[1].batches.len(), 0);
        // Shadow batch is still last in mesh 0!
        assert!(wmb.meshes[0].batches[3].header.is_shadow_proxy(false));

        // Reorder mesh groups (move mesh 1 to index 0)
        let reorder_payload = serde_json::json!({
            "ids": [mesh1_id],
            "index": 0
        });
        apply_wmb_scene_op(&mut wmb, &mut uid_map, "reorder_mesh_groups", &reorder_payload, None, None).unwrap();
        assert_eq!(wmb.meshes[0].name, "mesh_b");
        assert_eq!(wmb.meshes[1].name, "mesh_a");

        // Material delete: mat 0 is used, so deleting without reassignToId must fail
        let mat0_id = format!("mat:{}", uid_map.material_uids[0]);
        let mat1_id = format!("mat:{}", uid_map.material_uids[1]);
        assert!(apply_wmb_scene_op(
            &mut wmb,
            &mut uid_map,
            "material_delete",
            &serde_json::json!({ "id": mat0_id }),
            None,
            None
        ).is_err());

        // Deleting mat 0 with reassignToId = mat1_id succeeds and shifts mat 1 -> index 0
        apply_wmb_scene_op(
            &mut wmb,
            &mut uid_map,
            "material_delete",
            &serde_json::json!({ "id": mat0_id, "reassignToId": mat1_id }),
            None,
            None,
        ).unwrap();
        assert_eq!(wmb.materials.len(), 1);
        for b in &wmb.meshes[1].batches {
            assert_eq!(b.header.effective_material_id(false), 0);
        }

        // 2. WMB3 mesh reorder and material delete with remap
        let mut wmb3 = make_synthetic_wmb3();
        let mut uid3 = UidMap::new();
        uid3.ensure_wmb3(&wmb3);
        let m3_1 = format!("mesh:{}", uid3.mesh_uids[1]);
        apply_wmb3_scene_op(
            &mut wmb3,
            &mut uid3,
            "reorder_mesh_groups",
            &serde_json::json!({ "ids": [m3_1], "index": 0 }),
            None,
            None,
        ).unwrap();
        assert_eq!(wmb3.meshes[0].name, "mesh1");
        assert_eq!(wmb3.meshes[1].name, "mesh0");

        let mat3_0 = format!("mat:{}", uid3.material_uids[0]);
        let mat3_1 = format!("mat:{}", uid3.material_uids[1]);
        apply_wmb3_scene_op(
            &mut wmb3,
            &mut uid3,
            "material_delete",
            &serde_json::json!({ "id": mat3_0, "reassignToId": mat3_1 }),
            None,
            None,
        ).unwrap();
        assert_eq!(wmb3.materials.len(), 1);
        let b3_bytes = wmb3.to_bytes().unwrap();
        let _ = Wmb3File::from_bytes(&b3_bytes).unwrap();
    }

    #[test]
    fn test_part2_b3_b5_b25_b27_b30_and_error_paths() {
        // 1. B-3 & B-4: WMB3 multi-item move spanning target drop index in same LOD preserves VG and order
        let mut wmb3 = make_synthetic_wmb3();
        // Add a 3rd batch in LOD 0 belonging to mesh 0 so mesh 0 has 3 batches in LOD 0
        wmb3.batches.insert(
            1,
            Wmb3Batch {
                vertex_group_index: 0,
                bone_set_index: 0,
                vertex_start: 0,
                index_start: 0,
                num_vertexes: 3,
                num_indices: 3,
                num_primitives: 1,
            },
        );
        wmb3.lods[0].batch_infos.insert(
            1,
            Wmb3LodBatchInfo {
                vertex_group_index: 0,
                mesh_index: 0,
                material_index: 0,
                call_tree_node_index: -1,
                mesh_material_index: 0,
                u_b: 0,
            },
        );
        wmb3.lods[0].batch_infos[2].mesh_index = 0;
        wmb3.lods[1].batch_start = 3;
        let mut uid_map = UidMap::new();
        uid_map.ensure_wmb3(&wmb3);

        let b0 = format!("batch:{}", uid_map.wmb3_batch_uids[0]);
        let b2 = format!("batch:{}", uid_map.wmb3_batch_uids[2]);
        let m0 = uid_map.mesh_uids[0];
        let lod0_id = format!("lod:{m0}/0");
        apply_wmb3_scene_op(
            &mut wmb3,
            &mut uid_map,
            "move_items",
            &serde_json::json!({
                "items": [b0, b2],
                "target": { "id": lod0_id, "index": 1 }
            }),
            None,
            None,
        )
        .unwrap();
        assert_eq!(wmb3.batches.len(), 4);

        // 2. B-5: Uncovered trailing batch in WMB3 is preserved across unpack/repack
        wmb3.batches.push(Wmb3Batch {
            vertex_group_index: 0,
            bone_set_index: -1,
            vertex_start: 0,
            index_start: 0,
            num_vertexes: 3,
            num_indices: 3,
            num_primitives: 1,
        });
        uid_map.ensure_wmb3(&wmb3);
        let unpacked = unpack_wmb3_lod_batches(&wmb3, &uid_map);
        assert_eq!(unpacked.uncovered.len(), 1);
        repack_wmb3_lod_batches(&mut wmb3, &mut uid_map, unpacked);
        assert_eq!(wmb3.batches.len(), 5);

        // 3. B-25: Deleting an unused material in WMB3 prunes stale mesh.materials & mesh_material_pairs
        let mut wmb3_del = make_synthetic_wmb3();
        // Make all LOD batch_infos use material 1, while mesh 0 still lists stale material 0
        for lod in &mut wmb3_del.lods {
            for inf in &mut lod.batch_infos {
                inf.material_index = 1;
            }
        }
        let mut uid_del = UidMap::new();
        uid_del.ensure_wmb3(&wmb3_del);
        let del_mat0_id = format!("mat:{}", uid_del.material_uids[0]);
        apply_wmb3_scene_op(
            &mut wmb3_del,
            &mut uid_del,
            "material_delete",
            &serde_json::json!({ "id": del_mat0_id }),
            None,
            None,
        )
        .unwrap();
        assert_eq!(wmb3_del.materials.len(), 1);
        assert_eq!(wmb3_del.meshes[0].materials, vec![0]);

        // 4. B-27: Reject duplicate LOD level in update_wmb3_lod
        let mesh0_uid = uid_del.mesh_uids[0];
        let dup_lod_res = apply_wmb3_scene_op(
            &mut wmb3_del,
            &mut uid_del,
            "update_wmb3_lod",
            &serde_json::json!({ "id": format!("lod:{mesh0_uid}/0"), "level": 1 }),
            None,
            None,
        );
        assert!(dup_lod_res.is_err());

        // 5. B-26 & B-30: WMB 31-byte ASCII mesh name check and last-batch mesh_group_delete check
        let mut wmb0 = make_synthetic_wmb0(false);
        let mut uid0 = UidMap::new();
        uid0.ensure_wmb(&wmb0);
        let too_long_name = "a".repeat(32);
        assert!(apply_wmb_scene_op(
            &mut wmb0,
            &mut uid0,
            "mesh_group_add",
            &serde_json::json!({ "name": too_long_name }),
            None,
            None,
        )
        .is_err());

        wmb0.meshes[1].batches.clear();
        uid0.wmb_batch_uids[1].clear();
        let m0_id = format!("mesh:{}", uid0.mesh_uids[0]);
        assert!(apply_wmb_scene_op(
            &mut wmb0,
            &mut uid0,
            "mesh_group_delete",
            &serde_json::json!({ "id": m0_id }),
            None,
            None,
        )
        .is_err());
    }
}
