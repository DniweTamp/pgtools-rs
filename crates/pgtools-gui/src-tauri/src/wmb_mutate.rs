use crate::common::{DocSnapshot, DocumentOutline};
use crate::session::{
    build_outline_from_session, lock_clipboard, lock_session, record_mutation_with_key, AppState,
    StudioSession,
};
use crate::vfs::{resolve_subblob, split_container_and_leaf, write_back_subblob_with_align};
use crate::wmb_scene::{collect_paired_wtb_banks, collect_paired_wtb_texture_ids, validate_bone_parent};
use pgtools_core::{
    databases::lookup_wmb_vertex_layout,
    scene::UidMap,
    scene_ops::{
        apply_wmb3_scene_op, apply_wmb_scene_op, enforce_shadow_last, sync_wmb3_invariants,
        WmbClipboardData,
    },
    shader_db::{decode_wmb_material_rich, switch_wmb3_material_shader, switch_wmb_material_shader},
    texture_hash::{remap_wmb3_texture_hashes, remap_wmb_texture_hashes},
    wmb::{DecodedWmbMaterial, WmbFile},
    wmb3::{Wmb3File, Wmb3Material},
    wtb::WtbFile,
};
use std::fs;
use std::path::Path;
use tauri::State;

#[tauri::command]
pub fn mutate_wmb_node(
    node_id: String,
    operation: String,
    payload: serde_json::Value,
    state: State<'_, AppState>,
) -> Result<DocumentOutline, String> {
    let clipboard_opt = if operation == "paste_items" {
        lock_clipboard(&state).clone()
    } else {
        None
    };
    let mut sess = lock_session(&state);
    let snap = sess.current.clone().ok_or("No active document loaded")?;
    let oodle_path = sess.settings.oodle_dll_path.clone();
    let oodle_opt = if oodle_path.is_empty() {
        None
    } else {
        Some(Path::new(oodle_path.as_str()))
    };
    let (container_parts, _) = split_container_and_leaf(&node_id);
    let container_id = container_parts.join("/");
    let (_, sub_bytes, _, _) = resolve_subblob(&snap, &container_id, oodle_opt)?;

    // Work on a cloned UID map and structure flag; commit only after all fallible steps succeed (#5, #22)
    let orig_uid_map = sess
        .uid_maps
        .get(&container_id)
        .cloned()
        .unwrap_or_else(UidMap::new);
    let working_uid_map = orig_uid_map.clone();

    // Full WMB3 mutation & binary re-serialization (`Wmb3File::to_bytes`)!
    if sub_bytes.starts_with(b"WMB3") || sub_bytes.starts_with(b"3BMW") {
        mutate_wmb3_node(
            &mut sess,
            &snap,
            &node_id,
            &container_id,
            &container_parts,
            &sub_bytes,
            &operation,
            payload,
            oodle_opt,
            clipboard_opt.as_ref(),
            orig_uid_map,
            working_uid_map,
        )
    } else {
        mutate_wmb1_node(
            &mut sess,
            &snap,
            &node_id,
            &container_id,
            &container_parts,
            &sub_bytes,
            &operation,
            payload,
            oodle_opt,
            clipboard_opt.as_ref(),
            orig_uid_map,
            working_uid_map,
        )
    }
}

#[allow(clippy::too_many_arguments)]
fn mutate_wmb3_node(
    sess: &mut StudioSession,
    snap: &DocSnapshot,
    node_id: &str,
    container_id: &str,
    container_parts: &[&str],
    sub_bytes: &[u8],
    operation: &str,
    payload: serde_json::Value,
    oodle_opt: Option<&Path>,
    clipboard_opt: Option<&WmbClipboardData>,
    orig_uid_map: UidMap,
    mut working_uid_map: UidMap,
) -> Result<DocumentOutline, String> {
    let mut bump_structure = false;
    let mut coalesce_key: Option<String> = None;

    let mut wmb3 = Wmb3File::from_bytes(sub_bytes).map_err(|e| e.to_string())?;
    let mut init_committed = orig_uid_map;
    init_committed.ensure_wmb3(&wmb3);
    working_uid_map.ensure_wmb3(&wmb3);
    let mut modified_wtb_banks: Vec<(String, WtbFile)> = Vec::new();

    let desc = match operation {
        "export_obj" => {
            let out_path = payload
                .get("path")
                .and_then(|v| v.as_str())
                .ok_or("Missing OBJ export path")?;
            fs::write(out_path, wmb3.export_obj()).map_err(|e| e.to_string())?;
            return build_outline_from_session(sess);
        }
        "update_wmb3_material" => {
            let idx = payload
                .get("index")
                .and_then(|v| v.as_u64())
                .ok_or("Missing WMB3 material index")? as usize;
            let updated: Wmb3Material = serde_json::from_value(
                payload
                    .get("material")
                    .cloned()
                    .unwrap_or_else(|| payload.clone()),
            )
            .map_err(|e| e.to_string())?;
            let slot = wmb3
                .materials
                .get_mut(idx)
                .ok_or_else(|| format!("WMB3 Material index #{idx} out of bounds"))?;
            *slot = updated;
            format!("Update WMB3 Material #{idx}")
        }
        "switch_shader" => {
            let idx = payload
                .get("index")
                .and_then(|v| v.as_u64())
                .ok_or("Missing WMB3 material index")? as usize;
            let new_shader = payload
                .get("shader")
                .and_then(|v| v.as_str())
                .ok_or("Missing shader name")?;
            let mat = wmb3
                .materials
                .get_mut(idx)
                .ok_or("WMB3 Material index out of bounds")?;
            switch_wmb3_material_shader(mat, new_shader);
            format!("Switch WMB3 Material #{idx} Shader to {new_shader}")
        }
        "update_bone" => {
            let b_idx = payload
                .get("index")
                .and_then(|v| v.as_u64())
                .ok_or("Missing bone index")? as usize;
            if b_idx >= wmb3.bones.len() {
                return Err("WMB3 bone index out of bounds".into());
            }
            let validated_parent =
                if let Some(parent) = payload.get("parent_index").and_then(|v| v.as_i64()) {
                    Some(validate_bone_parent(
                        wmb3.bones.len(),
                        b_idx,
                        parent,
                        |i| wmb3.bones[i].parent_index,
                    )?)
                } else {
                    None
                };
            let parent_world_pos = {
                let eff_p = validated_parent.unwrap_or(wmb3.bones[b_idx].parent_index);
                if eff_p >= 0 && (eff_p as usize) < wmb3.bones.len() {
                    Some(wmb3.bones[eff_p as usize].position)
                } else {
                    None
                }
            };
            let bone = &mut wmb3.bones[b_idx];
            if let Some(valid_p) = validated_parent {
                bone.parent_index = valid_p;
                if let Some(pp) = parent_world_pos {
                    bone.local_position = pgtools_core::bone::Vec3::new(
                        bone.position.x - pp.x,
                        bone.position.y - pp.y,
                        bone.position.z - pp.z,
                    );
                } else {
                    bone.local_position = bone.position;
                }
                bump_structure = true;
            }
            if let Some(id_i64) = payload.get("global_id").and_then(|v| v.as_i64()) {
                let new_gid = i16::try_from(id_i64).map_err(|_| "Invalid bone global_id")?;
                let old_gid = bone.id;
                bone.id = new_gid;
                if old_gid >= 0 {
                    wmb3.bone_index_translate_table.table.remove(&(old_gid as u16));
                }
                if new_gid >= 0 {
                    wmb3.bone_index_translate_table
                        .table
                        .insert(new_gid as u16, b_idx as u16);
                }
                wmb3.raw_btt.clear();
            }
            if let Some(pos) = payload.get("position").and_then(|v| v.as_array()) {
                if pos.len() == 3 {
                    bone.position.x = pos[0].as_f64().unwrap_or(bone.position.x as f64) as f32;
                    bone.position.y = pos[1].as_f64().unwrap_or(bone.position.y as f64) as f32;
                    bone.position.z = pos[2].as_f64().unwrap_or(bone.position.z as f64) as f32;
                    if payload.get("relative_position").is_none() {
                        if let Some(pp) = parent_world_pos {
                            bone.local_position = pgtools_core::bone::Vec3::new(
                                bone.position.x - pp.x,
                                bone.position.y - pp.y,
                                bone.position.z - pp.z,
                            );
                        } else {
                            bone.local_position = bone.position;
                        }
                    }
                }
            }
            if let Some(rel) = payload.get("relative_position").and_then(|v| v.as_array()) {
                if rel.len() == 3 {
                    bone.local_position.x =
                        rel[0].as_f64().unwrap_or(bone.local_position.x as f64) as f32;
                    bone.local_position.y =
                        rel[1].as_f64().unwrap_or(bone.local_position.y as f64) as f32;
                    bone.local_position.z =
                        rel[2].as_f64().unwrap_or(bone.local_position.z as f64) as f32;
                    if payload.get("position").is_none() {
                        if let Some(pp) = parent_world_pos {
                            bone.position = pgtools_core::bone::Vec3::new(
                                pp.x + bone.local_position.x,
                                pp.y + bone.local_position.y,
                                pp.z + bone.local_position.z,
                            );
                        } else {
                            bone.position = bone.local_position;
                        }
                    }
                }
            }
            if let Some(rot) = payload.get("rotation").and_then(|v| v.as_array()) {
                if rot.len() == 3 {
                    bone.rotation.x = rot[0].as_f64().unwrap_or(bone.rotation.x as f64) as f32;
                    bone.rotation.y = rot[1].as_f64().unwrap_or(bone.rotation.y as f64) as f32;
                    bone.rotation.z = rot[2].as_f64().unwrap_or(bone.rotation.z as f64) as f32;
                    bone.local_rotation = bone.rotation;
                }
            }
            if let Some(scl) = payload.get("scale").and_then(|v| v.as_array()) {
                if scl.len() == 3 {
                    bone.scale.x = scl[0].as_f64().unwrap_or(bone.scale.x as f64) as f32;
                    bone.scale.y = scl[1].as_f64().unwrap_or(bone.scale.y as f64) as f32;
                    bone.scale.z = scl[2].as_f64().unwrap_or(bone.scale.z as f64) as f32;
                    bone.local_scale = bone.scale;
                }
            }
            format!("Update WMB3 Bone #{b_idx}")
        }
        "update_wmb3_batch" => {
            let bi = if let Some(bid_str) = payload
                .get("batch_id")
                .or_else(|| payload.get("id"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
            {
                working_uid_map
                    .wmb3_batch_index_by_id(bid_str)
                    .ok_or_else(|| format!("WMB3 batch '{bid_str}' not found"))?
            } else {
                payload
                    .get("batch_index")
                    .and_then(|v| v.as_u64())
                    .ok_or("Missing batch_id or batch_index")? as usize
            };
            if let Some(vgi) = payload.get("vertex_group_index").and_then(|v| v.as_u64()) {
                if (vgi as usize) >= wmb3.vertex_groups.len() {
                    return Err(format!("WMB3 vertex_group_index {vgi} out of bounds"));
                }
            }
            if let Some(bsi) = payload.get("bone_set_index").and_then(|v| v.as_i64()) {
                if bsi < -1 || (bsi >= 0 && (bsi as usize) >= wmb3.bone_sets.len()) {
                    return Err(format!(
                        "WMB3 bone_set_index {bsi} out of bounds (-1..{})",
                        wmb3.bone_sets.len()
                    ));
                }
            }
            if let Some(m_id) = payload.get("material_index").and_then(|v| v.as_u64()) {
                if (m_id as usize) >= wmb3.materials.len() {
                    return Err(format!("WMB3 material_index {m_id} out of bounds"));
                }
            }
            if let Some(mg_id) = payload
                .get("mesh_index")
                .or_else(|| payload.get("mesh_group_index"))
                .and_then(|v| v.as_u64())
            {
                if (mg_id as usize) >= wmb3.meshes.len() {
                    return Err(format!("WMB3 mesh_index {mg_id} out of bounds"));
                }
            }
            if let Some(cn) = payload.get("call_tree_node_index").and_then(|v| v.as_i64()) {
                if cn < -1 || (cn >= 0 && !wmb3.col_tree_nodes.is_empty() && (cn as usize) >= wmb3.col_tree_nodes.len()) {
                    return Err(format!(
                        "WMB3 call_tree_node_index {cn} out of bounds (-1..{})",
                        wmb3.col_tree_nodes.len()
                    ));
                }
            }
            let batch = wmb3
                .batches
                .get_mut(bi)
                .ok_or("WMB3 batch index out of bounds")?;
            if let Some(vgi) = payload.get("vertex_group_index").and_then(|v| v.as_u64()) {
                batch.vertex_group_index = vgi as u32;
            }
            if let Some(bsi) = payload.get("bone_set_index").and_then(|v| v.as_i64()) {
                batch.bone_set_index = bsi as i32;
            }
            if let Some(vs) = payload.get("vertex_start").and_then(|v| v.as_u64()) {
                batch.vertex_start = vs as u32;
            }
            if let Some(ist) = payload.get("index_start").and_then(|v| v.as_u64()) {
                batch.index_start = ist as u32;
            }
            if let Some(nv) = payload.get("num_vertexes").and_then(|v| v.as_u64()) {
                batch.num_vertexes = nv as u32;
            }
            if let Some(ni) = payload.get("num_indices").and_then(|v| v.as_u64()) {
                batch.num_indices = ni as u32;
            }
            if let Some(np) = payload.get("num_primitives").and_then(|v| v.as_u64()) {
                batch.num_primitives = np as u32;
            }

            let req_lod_idx = payload.get("lod_index").and_then(|v| v.as_u64()).map(|v| v as usize);
            let req_mat = payload.get("material_index").and_then(|v| v.as_u64()).map(|v| v as u32);
            let req_mesh = payload
                .get("mesh_index")
                .or_else(|| payload.get("mesh_group_index"))
                .and_then(|v| v.as_u64())
                .map(|v| v as u32);
            let req_lod_vgi = payload.get("lod_vertex_group_index").and_then(|v| v.as_u64()).map(|v| v as u32);
            let req_col_node = payload.get("call_tree_node_index").and_then(|v| v.as_i64()).map(|v| v as i32);
            let req_mesh_mat = payload.get("mesh_material_index").and_then(|v| v.as_u64()).map(|v| v as u32);
            let req_ub = payload.get("u_b").and_then(|v| v.as_i64()).map(|v| v as i32);

            for (li, lod) in wmb3.lods.iter_mut().enumerate() {
                if let Some(target_li) = req_lod_idx {
                    if li != target_li {
                        continue;
                    }
                }
                let start = lod.batch_start as usize;
                let end = start + lod.batch_infos.len();
                if bi >= start && bi < end {
                    let inf = &mut lod.batch_infos[bi - start];
                    if let Some(vgi) = req_lod_vgi {
                        inf.vertex_group_index = vgi;
                    } else if let Some(vgi) = payload.get("vertex_group_index").and_then(|v| v.as_u64()) {
                        inf.vertex_group_index = vgi as u32;
                    }
                    if let Some(m_id) = req_mat {
                        inf.material_index = m_id;
                    }
                    if let Some(mg_id) = req_mesh {
                        inf.mesh_index = mg_id;
                    }
                    if let Some(cn) = req_col_node {
                        inf.call_tree_node_index = cn;
                    }
                    if let Some(ub) = req_ub {
                        inf.u_b = ub;
                    }
                    if let Some(explicit_mm) = req_mesh_mat {
                        inf.mesh_material_index = explicit_mm;
                    } else if req_mat.is_some() || req_mesh.is_some() {
                        let target_mesh_idx = inf.mesh_index as usize;
                        let target_mat_u16 = inf.material_index as u16;
                        if let Some(mesh_grp) = wmb3.meshes.get_mut(target_mesh_idx) {
                            let local_mat_pos = if let Some(pos) =
                                mesh_grp.materials.iter().position(|&m| m == target_mat_u16)
                            {
                                pos
                            } else {
                                mesh_grp.materials.push(target_mat_u16);
                                mesh_grp.materials.len() - 1
                            };
                            inf.mesh_material_index = local_mat_pos as u32;
                        }
                    }
                }
            }
            format!("Update WMB3 Batch #{bi}")
        }
        "set_tpose" => {
            wmb3.set_tpose();
            "Bake WMB3 Skeleton & Vertices to T-Pose".to_string()
        }
        "recompute_relative_positions" => {
            let positions: Vec<pgtools_core::bone::Vec3> =
                wmb3.bones.iter().map(|b| b.position).collect();
            for (i, b) in wmb3.bones.iter_mut().enumerate() {
                if b.parent_index >= 0 && (b.parent_index as usize) < positions.len() {
                    let pp = positions[b.parent_index as usize];
                    b.local_position = pgtools_core::bone::Vec3::new(
                        positions[i].x - pp.x,
                        positions[i].y - pp.y,
                        positions[i].z - pp.z,
                    );
                } else {
                    b.local_position = positions[i];
                }
            }
            "Recompute WMB3 Relative Bone Positions".to_string()
        }
        "update_header" => {
            if let Some(fl) = payload.get("flags").and_then(|v| v.as_i64()) {
                if fl < i16::MIN as i64 || fl > u16::MAX as i64 {
                    return Err(format!("WMB3 header flags {fl} out of 16-bit range"));
                }
                wmb3.flags = fl as i16;
            } else if let Some(flags_s) = payload
                .get("vertex_flags_hex")
                .or_else(|| payload.get("flags_hex"))
                .and_then(|v| v.as_str())
            {
                let clean = flags_s.trim().trim_start_matches("0x").trim_start_matches("0X");
                let vf = u16::from_str_radix(clean, 16).map_err(|_| {
                    format!(
                        "Invalid WMB3 header flags '{flags_s}': expected 16-bit hex value (0x0000..0xFFFF)"
                    )
                })?;
                wmb3.flags = vf as i16;
            }
            if let Some(bb) = payload.get("bounding_box").and_then(|v| v.as_array()) {
                if bb.len() == 6 {
                    for i in 0..6 {
                        if let Some(fv) = bb[i].as_f64() {
                            wmb3.bounding_box[i] = fv as f32;
                        }
                    }
                }
            }
            "Update WMB3 Header fields".to_string()
        }
        "set_material_texture" => {
            let idx = payload
                .get("index")
                .and_then(|v| v.as_u64())
                .ok_or("Missing WMB3 material index")? as usize;
            let slot = payload
                .get("slot")
                .and_then(|v| v.as_u64())
                .ok_or("Missing WMB3 texture slot index")? as usize;
            let tid = payload.get("texture_id").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
            let mat = wmb3
                .materials
                .get_mut(idx)
                .ok_or_else(|| format!("WMB3 Material index #{idx} out of bounds"))?;
            let entry = mat
                .textures
                .get_mut(slot)
                .ok_or_else(|| format!("WMB3 Material #{idx} texture slot #{slot} out of bounds"))?;
            entry.1 = tid;
            format!("Set WMB3 Material #{idx} Slot #{slot} to 0x{tid:08X}")
        }
        "set_material_param" => {
            let idx = payload
                .get("index")
                .and_then(|v| v.as_u64())
                .ok_or("Missing WMB3 material index")? as usize;
            let key = payload.get("key").and_then(|v| v.as_str()).unwrap_or("");
            let vals: Vec<f32> = payload
                .get("values")
                .and_then(|v| v.as_array())
                .map(|arr| arr.iter().filter_map(|x| x.as_f64().map(|f| f as f32)).collect())
                .unwrap_or_default();
            let mat = wmb3.materials.get_mut(idx).ok_or("WMB3 Material index out of bounds")?;
            if let Some(rest) = key.strip_prefix("var_") {
                if let Some((vi_str, v_name)) = rest.split_once('_') {
                    if let (Ok(vi), Some(&v0)) = (vi_str.parse::<usize>(), vals.first()) {
                        if let Some(var_slot) = mat.variables.get_mut(vi) {
                            var_slot.1 = v0;
                        }
                        let nps = mat.named_parameters();
                        for np in nps {
                            if np.name == v_name {
                                if let Some(grp) = mat.parameter_groups.get_mut(np.group_index) {
                                    if let Some(p_slot) = grp.1.get_mut(np.param_index) {
                                        *p_slot = v0;
                                    }
                                }
                            }
                        }
                    }
                }
            } else {
                let parts: Vec<&str> = key.splitn(3, '_').collect();
                if parts.len() >= 2 {
                    if let (Ok(gi), Ok(pi), Some(&v0)) = (parts[0].parse::<usize>(), parts[1].parse::<usize>(), vals.first()) {
                        if let Some(grp) = mat.parameter_groups.get_mut(gi) {
                            if let Some(p_slot) = grp.1.get_mut(pi) {
                                *p_slot = v0;
                            }
                        }
                        if let Some(&param_name) = parts.get(2) {
                            for (vname, vval) in &mut mat.variables {
                                if vname == param_name {
                                    *vval = v0;
                                }
                            }
                        }
                    }
                }
            }
            coalesce_key = Some(format!("param:{container_id}:{idx}:{key}"));
            format!("Update WMB3 Material #{idx} Param {key}")
        }
        "add_bone" => {
            let parent_gid = payload.get("parent_global_id").and_then(|v| v.as_i64()).unwrap_or(-1);
            let parent_idx = if parent_gid >= 0 {
                wmb3.bones
                    .iter()
                    .position(|b| b.id as i64 == parent_gid)
                    .map(|i| i as i16)
                    .unwrap_or(-1)
            } else {
                -1
            };
            let next_gid = wmb3.bones.iter().map(|b| b.id).max().unwrap_or(0).saturating_add(1);
            let parent_pos = if parent_idx >= 0 {
                wmb3.bones.get(parent_idx as usize).map(|b| b.position).unwrap_or_default()
            } else {
                pgtools_core::bone::Vec3::default()
            };
            let new_local_idx = wmb3.bones.len() as u16;
            wmb3.bones.push(pgtools_core::wmb3::Wmb3Bone {
                id: next_gid,
                parent_index: parent_idx,
                local_position: pgtools_core::bone::Vec3::new(0.0, 0.1, 0.0),
                local_rotation: pgtools_core::bone::Vec3::default(),
                local_scale: pgtools_core::bone::Vec3::new(1.0, 1.0, 1.0),
                position: pgtools_core::bone::Vec3::new(parent_pos.x, parent_pos.y + 0.1, parent_pos.z),
                rotation: pgtools_core::bone::Vec3::default(),
                scale: pgtools_core::bone::Vec3::new(1.0, 1.0, 1.0),
                t_position: pgtools_core::bone::Vec3::new(parent_pos.x, parent_pos.y + 0.1, parent_pos.z),
            });
            if !wmb3.bone_index_translate_table.table.is_empty() && next_gid >= 0 {
                wmb3.bone_index_translate_table
                    .table
                    .insert(next_gid as u16, new_local_idx);
                wmb3.raw_btt.clear();
            }
            bump_structure = true;
            format!("Add WMB3 Bone (gid {next_gid})")
        }
        "delete_bone" => {
            if wmb3.bones.len() <= 1 {
                return Err("Cannot delete the only remaining bone".into());
            }
            let gid_i64 = payload
                .get("global_id")
                .and_then(|v| v.as_i64())
                .ok_or("Missing bone global_id")?;
            let gid = i16::try_from(gid_i64).map_err(|_| format!("Invalid bone global_id {gid_i64}"))?;
            let del_idx = wmb3
                .bones
                .iter()
                .position(|b| b.id == gid)
                .or_else(|| {
                    if gid >= 0 && (gid as usize) < wmb3.bones.len() {
                        Some(gid as usize)
                    } else {
                        None
                    }
                })
                .ok_or("WMB3 bone not found")?;

            // Check if any vertex in any batch is actively skinned to `del_idx` (#17)
            for b in &wmb3.batches {
                let Some(vg) = wmb3.vertex_groups.get(b.vertex_group_index as usize) else {
                    continue;
                };
                let bone_refs: Vec<usize> = if b.bone_set_index >= 0
                    && (b.bone_set_index as usize) < wmb3.bone_sets.len()
                {
                    wmb3.bone_sets[b.bone_set_index as usize]
                        .iter()
                        .map(|&bi| {
                            if !wmb3.bone_map.is_empty() {
                                *wmb3.bone_map.get(bi as usize).unwrap_or(&0) as usize
                            } else {
                                bi.max(0) as usize
                            }
                        })
                        .collect()
                } else if !wmb3.bone_map.is_empty() {
                    wmb3.bone_map.iter().map(|&bi| bi as usize).collect()
                } else {
                    (0..wmb3.bones.len()).collect()
                };
                if !bone_refs.contains(&del_idx) {
                    continue;
                }
                let start = (b.index_start as usize).min(vg.indices.len());
                let end = (start + b.num_indices as usize).min(vg.indices.len());
                for &raw_vi in &vg.indices[start..end] {
                    let vi = ((raw_vi + b.vertex_start) as usize).min(vg.vertexes.len().saturating_sub(1));
                    if let Some(v) = vg.vertexes.get(vi) {
                        if let Some(bi_info) = v.bone_infos {
                            for (local_bi, bw) in bi_info.active_pairs() {
                                if bw > 0 && bone_refs.get(local_bi as usize) == Some(&del_idx) {
                                    return Err(format!(
                                        "Cannot delete bone #{del_idx} (gid {gid}) because it is referenced by skinned mesh vertices"
                                    ));
                                }
                            }
                        }
                    }
                }
            }

            let del_parent = wmb3.bones[del_idx].parent_index;
            let fallback_bone = if del_parent >= 0 {
                if (del_parent as usize) > del_idx {
                    (del_parent - 1) as usize
                } else {
                    del_parent as usize
                }
            } else {
                0usize
            };

            wmb3.bones.remove(del_idx);
            for b in &mut wmb3.bones {
                if b.parent_index == del_idx as i16 {
                    b.parent_index = if del_parent > del_idx as i16 {
                        del_parent - 1
                    } else {
                        del_parent
                    };
                } else if b.parent_index > del_idx as i16 {
                    b.parent_index -= 1;
                }
            }

            // Remap bone_index_translate_table & clear stale raw_btt (#17)
            if gid >= 0 {
                wmb3.bone_index_translate_table.table.remove(&(gid as u16));
            }
            wmb3.bone_index_translate_table
                .table
                .retain(|_, &mut l| (l as usize) != del_idx);
            for l in wmb3.bone_index_translate_table.table.values_mut() {
                if (*l as usize) > del_idx {
                    *l -= 1;
                }
            }
            wmb3.raw_btt.clear();

            // Remap bone_map, bone_sets, and meshes[].bone_indices (#17)
            if !wmb3.bone_map.is_empty() {
                for bm in &mut wmb3.bone_map {
                    if (*bm as usize) == del_idx {
                        *bm = fallback_bone as u32;
                    } else if (*bm as usize) > del_idx {
                        *bm -= 1;
                    }
                }
            } else {
                for set in &mut wmb3.bone_sets {
                    for bi in set.iter_mut() {
                        if (*bi as usize) == del_idx {
                            *bi = fallback_bone as i16;
                        } else if (*bi as usize) > del_idx {
                            *bi -= 1;
                        }
                    }
                }
            }
            for mesh in &mut wmb3.meshes {
                for bi in &mut mesh.bone_indices {
                    if (*bi as usize) == del_idx {
                        *bi = fallback_bone as u16;
                    } else if (*bi as usize) > del_idx {
                        *bi -= 1;
                    }
                }
            }

            bump_structure = true;
            format!("Delete WMB3 Bone (gid {gid})")
        }
        "reorder_materials" => {
            let from_idx = payload.get("from_index").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
            let to_idx = payload.get("to_index").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
            if from_idx >= wmb3.materials.len() || wmb3.materials.is_empty() {
                return Err("WMB3 Material index out of bounds".into());
            }
            let moved_mat = wmb3.materials.remove(from_idx);
            let moved_uid = working_uid_map.material_uids.remove(from_idx);
            let ins = if to_idx > from_idx {
                to_idx.saturating_sub(1)
            } else {
                to_idx
            }
            .min(wmb3.materials.len());
            wmb3.materials.insert(ins, moved_mat);
            working_uid_map.material_uids.insert(ins, moved_uid);
            let remap_idx = |old: u32| -> u32 {
                let o = old as usize;
                if o == from_idx {
                    ins as u32
                } else if from_idx < ins && o > from_idx && o <= ins {
                    (o - 1) as u32
                } else if ins < from_idx && o >= ins && o < from_idx {
                    (o + 1) as u32
                } else {
                    old
                }
            };
            for lod in &mut wmb3.lods {
                for inf in &mut lod.batch_infos {
                    inf.material_index = remap_idx(inf.material_index);
                }
            }
            for pair in &mut wmb3.mesh_material_pairs {
                pair.1 = remap_idx(pair.1);
            }
            for m in &mut wmb3.meshes {
                for mat_ref in &mut m.materials {
                    *mat_ref = remap_idx(*mat_ref as u32) as u16;
                }
            }
            bump_structure = true;
            format!("Reorder WMB3 Material #{from_idx}")
        }
        "set_batches_material"
        | "move_items"
        | "delete_items"
        | "duplicate_items"
        | "reorder_mesh_groups"
        | "mesh_group_add"
        | "mesh_group_duplicate"
        | "mesh_group_delete"
        | "mesh_group_rename"
        | "update_wmb3_lod"
        | "material_add"
        | "material_duplicate"
        | "material_delete"
        | "material_rename"
        | "paste_items" => {
            let bank_tex_ids_opt = if operation == "paste_items" {
                Some(collect_paired_wtb_texture_ids(
                    &snap, node_id, oodle_opt,
                ))
            } else {
                None
            };
            let outcome = apply_wmb3_scene_op(
                &mut wmb3,
                &mut working_uid_map,
                &operation,
                &payload,
                clipboard_opt,
                bank_tex_ids_opt.as_deref(),
            )?;
            if outcome.structural {
                bump_structure = true;
            }
            outcome.description
        }
        "remap_texture_hashes" => {
            let mut banks = collect_paired_wtb_banks(snap, node_id, oodle_opt);
            let mut bank_refs: Vec<&mut WtbFile> =
                banks.iter_mut().map(|(_, b)| b).collect();
            let report = remap_wmb3_texture_hashes(&mut wmb3, &mut bank_refs, false);
            if report.remaps.is_empty() {
                let mut outline = build_outline_from_session(sess)?;
                outline.status_message = Some("All texture hashes are valid crc32 & 7f".into());
                return Ok(outline);
            }
            for idx in report.wtb_modified_indices {
                modified_wtb_banks.push(banks[idx].clone());
            }
            format!(
                "Remap {} invalid texture hash(es) (crc32 & 7f)",
                report.remaps.len()
            )
        }
        other => return Err(format!("Unsupported WMB3 operation: {other}")),
    };
    sync_wmb3_invariants(&mut wmb3);
    working_uid_map.ensure_wmb3(&wmb3);
    let new_wmb3_bytes = wmb3.to_bytes().map_err(|e| e.to_string())?;
    let _ = Wmb3File::from_bytes(&new_wmb3_bytes)
        .map_err(|e| format!("WMB3 round-trip verification failed: {e}"))?;
    let dat_align = sess.settings.effective_dat_alignment();
    let mut next_snap =
        write_back_subblob_with_align(snap, container_parts, new_wmb3_bytes, None, oodle_opt, dat_align)?;
    for (bank_node_id, wtb) in modified_wtb_banks {
        let (wtb_bytes, _) = wtb.to_bytes();
        let (bank_parts, _) = split_container_and_leaf(&bank_node_id);
        next_snap = write_back_subblob_with_align(
            &next_snap,
            &bank_parts,
            wtb_bytes,
            None,
            oodle_opt,
            dat_align,
        )?;
    }

    sess.committed_uid_maps
        .entry(container_id.to_string())
        .or_insert(init_committed);
    sess.uid_maps.insert(container_id.to_string(), working_uid_map);
    if bump_structure {
        sess.structure_version = sess.structure_version.max(1) + 1;
    }
    record_mutation_with_key(sess, desc, next_snap, coalesce_key);
    return build_outline_from_session(sess);
}

#[allow(clippy::too_many_arguments)]
fn mutate_wmb1_node(
    sess: &mut StudioSession,
    snap: &DocSnapshot,
    node_id: &str,
    container_id: &str,
    container_parts: &[&str],
    sub_bytes: &[u8],
    operation: &str,
    payload: serde_json::Value,
    oodle_opt: Option<&Path>,
    clipboard_opt: Option<&WmbClipboardData>,
    orig_uid_map: UidMap,
    mut working_uid_map: UidMap,
) -> Result<DocumentOutline, String> {
    let mut bump_structure = false;
    let mut coalesce_key: Option<String> = None;
    let mut wmb = WmbFile::from_bytes(sub_bytes).map_err(|e| e.to_string())?;

    let mut target_endian = wmb.endian;
    let is_b2 = wmb.is_bayo2();
    let mut init_committed = orig_uid_map;
    let mut modified_wtb_banks: Vec<(String, WtbFile)> = Vec::new();
    init_committed.ensure_wmb(&wmb);
    working_uid_map.ensure_wmb(&wmb);

    let desc = match operation {
        "update_material" => {
            let decoded: DecodedWmbMaterial =
                serde_json::from_value(payload).map_err(|e| e.to_string())?;
            let mat = wmb
                .materials
                .get_mut(decoded.index)
                .ok_or("Material index out of bounds")?;
            mat.apply_decoded(&decoded);
            format!("Update Material #{}", decoded.index)
        }
        "switch_shader" => {
            let idx = payload
                .get("index")
                .and_then(|v| v.as_u64())
                .ok_or("Missing material index")? as usize;
            let target = payload
                .get("shader")
                .and_then(|v| v.as_str())
                .ok_or("Missing shader/type")?;
            let use_b2 = payload
                .get("is_bayo2")
                .and_then(|v| v.as_bool())
                .unwrap_or(is_b2);
            let mat = wmb
                .materials
                .get_mut(idx)
                .ok_or("Material index out of bounds")?;
            let _ = switch_wmb_material_shader(mat, idx, use_b2, target);
            format!("Switch Material #{idx} to {target}")
        }
        "update_header" => {
            if payload.get("bounding_box").is_some() && payload.get("vertex_flags_hex").is_none() {
                return Err(
                    "WMB0-2 headers do not store a model-level bounding box (computed from mesh vertices)"
                        .into(),
                );
            }
            if let Some(flags_s) = payload.get("vertex_flags_hex").and_then(|v| v.as_str()) {
                let clean = flags_s.trim().trim_start_matches("0x").trim_start_matches("0X");
                let vf = u32::from_str_radix(clean, 16)
                    .map_err(|_| format!("Invalid hex vertex_flags '{flags_s}'"))?;
                let force = payload
                    .get("force")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                if !force
                    && lookup_wmb_vertex_layout(
                        vf,
                        wmb.header.vertex_ex_data_size,
                        wmb.header.vertex_ex_data,
                    )
                    .is_none()
                {
                    return Err(format!(
                        "Unsupported WMB vertex_flags 0x{vf:08X} for ex_data_size={} ex_data={}",
                        wmb.header.vertex_ex_data_size, wmb.header.vertex_ex_data
                    ));
                }
                wmb.header.vertex_flags = vf;
            }
            "Update WMB Header fields".to_string()
        }
        "update_bone" => {
            let b_idx = payload
                .get("index")
                .and_then(|v| v.as_u64())
                .ok_or("Missing bone index")? as usize;
            if b_idx >= wmb.bone_hierarchy.len() {
                return Err("Bone index out of bounds".into());
            }
            if let Some(parent) = payload.get("parent_index").and_then(|v| v.as_i64()) {
                let valid_p = validate_bone_parent(
                    wmb.bone_hierarchy.len(),
                    b_idx,
                    parent,
                    |i| wmb.bone_hierarchy[i],
                )?;
                wmb.bone_hierarchy[b_idx] = valid_p;
                bump_structure = true;
            }
            let parent_idx = wmb.bone_hierarchy[b_idx];
            let parent_world = if parent_idx >= 0 && (parent_idx as usize) < wmb.bone_positions.len() {
                Some(wmb.bone_positions[parent_idx as usize])
            } else {
                None
            };
            if payload.get("parent_index").is_some()
                && payload.get("position").is_none()
                && payload.get("relative_position").is_none()
            {
                if let (Some(&world_p), Some(rel_slot)) = (
                    wmb.bone_positions.get(b_idx),
                    wmb.bone_relative_positions.get_mut(b_idx),
                ) {
                    if let Some(pp) = parent_world {
                        rel_slot.x = world_p.x - pp.x;
                        rel_slot.y = world_p.y - pp.y;
                        rel_slot.z = world_p.z - pp.z;
                    } else {
                        *rel_slot = world_p;
                    }
                }
            }
            if let Some(pos) = payload.get("position").and_then(|v| v.as_array()) {
                if pos.len() == 3 {
                    if let Some(slot) = wmb.bone_positions.get_mut(b_idx) {
                        slot.x = pos[0].as_f64().unwrap_or(slot.x as f64) as f32;
                        slot.y = pos[1].as_f64().unwrap_or(slot.y as f64) as f32;
                        slot.z = pos[2].as_f64().unwrap_or(slot.z as f64) as f32;
                    }
                    if payload.get("relative_position").is_none() {
                        if let (Some(&world_p), Some(rel_slot)) = (
                            wmb.bone_positions.get(b_idx),
                            wmb.bone_relative_positions.get_mut(b_idx),
                        ) {
                            if let Some(pp) = parent_world {
                                rel_slot.x = world_p.x - pp.x;
                                rel_slot.y = world_p.y - pp.y;
                                rel_slot.z = world_p.z - pp.z;
                            } else {
                                *rel_slot = world_p;
                            }
                        }
                    }
                }
            }
            if let Some(rel) = payload.get("relative_position").and_then(|v| v.as_array()) {
                if rel.len() == 3 {
                    if let Some(slot) = wmb.bone_relative_positions.get_mut(b_idx) {
                        slot.x = rel[0].as_f64().unwrap_or(slot.x as f64) as f32;
                        slot.y = rel[1].as_f64().unwrap_or(slot.y as f64) as f32;
                        slot.z = rel[2].as_f64().unwrap_or(slot.z as f64) as f32;
                    }
                    if payload.get("position").is_none() {
                        if let (Some(&rel_p), Some(world_slot)) = (
                            wmb.bone_relative_positions.get(b_idx),
                            wmb.bone_positions.get_mut(b_idx),
                        ) {
                            if let Some(pp) = parent_world {
                                world_slot.x = pp.x + rel_p.x;
                                world_slot.y = pp.y + rel_p.y;
                                world_slot.z = pp.z + rel_p.z;
                            } else {
                                *world_slot = rel_p;
                            }
                        }
                    }
                }
            }
            format!("Update Bone #{b_idx}")
        }
        "update_batch" => {
            let (mesh_idx, batch_idx) = if let Some(bid_str) = payload
                .get("batch_item_id")
                .or_else(|| payload.get("id"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
            {
                working_uid_map
                    .wmb_batch_pos_by_id(bid_str)
                    .ok_or_else(|| format!("WMB batch '{bid_str}' not found"))?
            } else if let Some(bid_str) = payload
                .get("batch_id")
                .and_then(|v| v.as_str())
                .filter(|s| s.starts_with("batch:"))
            {
                working_uid_map
                    .wmb_batch_pos_by_id(bid_str)
                    .ok_or_else(|| format!("WMB batch '{bid_str}' not found"))?
            } else {
                let mi = payload
                    .get("mesh_index")
                    .and_then(|v| v.as_u64())
                    .ok_or("Missing mesh_index")? as usize;
                let bi = payload
                    .get("batch_index")
                    .and_then(|v| v.as_u64())
                    .ok_or("Missing batch_index")? as usize;
                (mi, bi)
            };
            if let Some(ue1) = payload.get("u_e1").and_then(|v| v.as_i64()) {
                if !(0..=255).contains(&ue1) {
                    return Err(format!("u_e1 value {ue1} out of u8 range (0..=255)"));
                }
            }
            if let Some(ue2) = payload.get("u_e2").and_then(|v| v.as_i64()) {
                if !(0..=255).contains(&ue2) {
                    return Err(format!("u_e2 value {ue2} out of u8 range (0..=255)"));
                }
            }
            if let Some(prim) = payload.get("primitive_type").and_then(|v| v.as_i64()) {
                if prim != 4 && prim != 5 {
                    return Err(format!(
                        "Invalid WMB primitive_type {prim}: expected 4 (Triangle List) or 5 (Triangle Strip)"
                    ));
                }
            }
            if let Some(fl_hex) = payload.get("flags_hex").and_then(|v| v.as_str()) {
                let clean = fl_hex.trim().trim_start_matches("0x").trim_start_matches("0X");
                if u16::from_str_radix(clean, 16).is_err() {
                    return Err(format!(
                        "Invalid batch flags_hex '{fl_hex}': expected 16-bit hex (0x0000..0xFFFF)"
                    ));
                }
            }
            let max_bones = wmb.bone_hierarchy.len();
            if let Some(br_arr) = payload.get("bone_refs").and_then(|v| v.as_array()) {
                for item in br_arr {
                    let Some(val) = item.as_i64() else {
                        return Err("Invalid bone_refs entry: expected integer".into());
                    };
                    if val < 0 || val > 255 {
                        return Err(format!("bone_refs entry {val} out of u8 range (0..=255)"));
                    }
                    if max_bones > 0 && (val as usize) >= max_bones {
                        return Err(format!(
                            "bone_refs entry {val} exceeds skeleton bone count ({max_bones})"
                        ));
                    }
                }
            }
            let mesh = wmb.meshes.get_mut(mesh_idx).ok_or("Mesh index out of bounds")?;
            let batch = mesh
                .batches
                .get_mut(batch_idx)
                .ok_or("Batch index out of bounds")?;

            // Effective material selection (from Material Picker popover)
            if let Some(mat_id) = payload.get("material_id").and_then(|v| v.as_u64()) {
                if is_b2 {
                    batch.header.ex_mat_id = mat_id as i16;
                } else {
                    batch.header.material_id = mat_id as u8;
                }
            }
            // Explicit raw header fields (from Batch Header editor)
            if let Some(raw_mat) = payload.get("raw_material_id").and_then(|v| v.as_u64()) {
                batch.header.material_id = raw_mat as u8;
            }
            if let Some(ex_mat) = payload.get("ex_mat_id").and_then(|v| v.as_i64()) {
                batch.header.ex_mat_id = ex_mat as i16;
            }
            if let Some(bid) = payload.get("batch_id").and_then(|v| v.as_i64()) {
                batch.header.batch_id = bid as i16;
            }
            if let Some(mid) = payload.get("mesh_id").and_then(|v| v.as_i64()) {
                batch.header.mesh_id = mid as i16;
            }
            if let Some(fl) = payload.get("flags").and_then(|v| v.as_u64()) {
                batch.header.flags = fl as u16;
            }
            if let Some(fl_hex) = payload.get("flags_hex").and_then(|v| v.as_str()) {
                let clean = fl_hex.trim().trim_start_matches("0x").trim_start_matches("0X");
                if let Ok(parsed) = u16::from_str_radix(clean, 16) {
                    batch.header.flags = parsed;
                }
            }
            if let Some(hbr) = payload.get("has_bone_refs").and_then(|v| v.as_u64()) {
                batch.header.has_bone_refs = hbr as u8;
            }
            if let Some(ue1) = payload.get("u_e1").and_then(|v| v.as_u64()) {
                batch.header.u_e1 = ue1 as u8;
            }
            if let Some(ue2) = payload.get("u_e2").and_then(|v| v.as_u64()) {
                batch.header.u_e2 = ue2 as u8;
            }
            if let Some(sh_proxy) = payload.get("shadow_proxy").and_then(|v| v.as_bool()) {
                batch.header.set_shadow_proxy(is_b2, sh_proxy);
            }
            if let Some(vs) = payload.get("vertex_start").and_then(|v| v.as_u64()) {
                batch.header.vertex_start = vs as u32;
            }
            if let Some(ve) = payload.get("vertex_end").and_then(|v| v.as_u64()) {
                batch.header.vertex_end = ve as u32;
            }
            if let Some(prim) = payload.get("primitive_type").and_then(|v| v.as_i64()) {
                batch.header.primitive_type = prim as i32;
            }
            if let Some(oi) = payload.get("offset_indices").and_then(|v| v.as_u64()) {
                batch.header.offset_indices = oi as u32;
            }
            if let Some(ni) = payload.get("num_indices").and_then(|v| v.as_i64()) {
                batch.header.num_indices = ni as i32;
            }
            if let Some(vo) = payload.get("vertex_offset").and_then(|v| v.as_i64()) {
                batch.header.vertex_offset = vo as i32;
            }
            if let Some(uf_arr) = payload.get("u_f").and_then(|v| v.as_array()) {
                for (i, item) in uf_arr.iter().take(7).enumerate() {
                    if let Some(val) = item.as_i64() {
                        batch.header.u_f[i] = val as i32;
                    }
                }
            }
            if let Some(br_arr) = payload.get("bone_refs").and_then(|v| v.as_array()) {
                batch.bone_refs = br_arr
                    .iter()
                    .filter_map(|v| v.as_u64().map(|n| n as u8))
                    .collect();
                batch.header.has_bone_refs = if batch.bone_refs.is_empty() { 0 } else { 1 };
            }
            if let Some(unb_arr) = payload.get("unknown_no_bones").and_then(|v| v.as_array()) {
                for (i, item) in unb_arr.iter().take(4).enumerate() {
                    if let Some(val) = item.as_f64() {
                        batch.unknown_no_bones[i] = val as f32;
                    }
                }
            }
            batch.header.offset_indices = batch.effective_indices_offset();
            format!("Update Mesh #{mesh_idx} Batch #{batch_idx}")
        }
        "toggle_wmb_mesh_shadow_proxy" => {
            let mesh_idx = payload
                .get("mesh_index")
                .and_then(|v| v.as_u64())
                .ok_or("Missing mesh_index")? as usize;
            let mesh = wmb.meshes.get_mut(mesh_idx).ok_or("Mesh index out of bounds")?;
            let target_enabled = payload
                .get("enabled")
                .and_then(|v| v.as_bool())
                .unwrap_or_else(|| !mesh.batches.iter().all(|b| b.header.is_shadow_proxy(is_b2)));
            for b in &mut mesh.batches {
                b.header.set_shadow_proxy(is_b2, target_enabled);
            }
            bump_structure = true;
            format!(
                "{} Shadow Proxy on Mesh #{mesh_idx} ({})",
                if target_enabled { "Enable" } else { "Disable" },
                mesh.name
            )
        }
        "move_wmb_batch" => {
            let from_mesh = payload
                .get("from_mesh_index")
                .and_then(|v| v.as_u64())
                .ok_or("Missing from_mesh_index")? as usize;
            let from_batch = payload
                .get("from_batch_index")
                .and_then(|v| v.as_u64())
                .ok_or("Missing from_batch_index")? as usize;
            let to_mesh = payload
                .get("to_mesh_index")
                .and_then(|v| v.as_u64())
                .ok_or("Missing to_mesh_index")? as usize;
            let to_batch_opt = payload
                .get("to_batch_index")
                .and_then(|v| v.as_u64())
                .map(|v| v as usize);

            if from_mesh >= wmb.meshes.len() || to_mesh >= wmb.meshes.len() {
                return Err("Mesh index out of bounds".into());
            }
            if from_batch >= wmb.meshes[from_mesh].batches.len() {
                return Err("Batch index out of bounds".into());
            }
            let mut moved = wmb.meshes[from_mesh].batches.remove(from_batch);
            let moved_uid = working_uid_map.wmb_batch_uids[from_mesh].remove(from_batch);
            moved.header.mesh_id = to_mesh as i16;
            let dst_batches = &mut wmb.meshes[to_mesh].batches;
            let ins_pos = to_batch_opt.unwrap_or(dst_batches.len()).min(dst_batches.len());
            dst_batches.insert(ins_pos, moved);
            working_uid_map.wmb_batch_uids[to_mesh].insert(ins_pos, moved_uid);

            for (mi, m) in wmb.meshes.iter_mut().enumerate() {
                for (bi, b) in m.batches.iter_mut().enumerate() {
                    b.header.mesh_id = mi as i16;
                    b.header.batch_id = bi as i16;
                }
            }
            bump_structure = true;
            format!("Move Batch #{from_batch} from Mesh #{from_mesh} to Mesh #{to_mesh}")
        }
        "duplicate_wmb_batch" => {
            let mesh_idx = payload
                .get("mesh_index")
                .and_then(|v| v.as_u64())
                .ok_or("Missing mesh_index")? as usize;
            let batch_idx = payload
                .get("batch_index")
                .and_then(|v| v.as_u64())
                .ok_or("Missing batch_index")? as usize;
            let mesh = wmb.meshes.get_mut(mesh_idx).ok_or("Mesh index out of bounds")?;
            let cloned = mesh
                .batches
                .get(batch_idx)
                .ok_or("Batch index out of bounds")?
                .clone();
            mesh.batches.insert(batch_idx + 1, cloned);
            let new_uid = working_uid_map.alloc();
            working_uid_map.wmb_batch_uids[mesh_idx].insert(batch_idx + 1, new_uid);
            for (bi, b) in mesh.batches.iter_mut().enumerate() {
                b.header.batch_id = bi as i16;
            }
            bump_structure = true;
            format!("Duplicate Mesh #{mesh_idx} Batch #{batch_idx}")
        }
        "delete_wmb_batch" => {
            let mesh_idx = payload
                .get("mesh_index")
                .and_then(|v| v.as_u64())
                .ok_or("Missing mesh_index")? as usize;
            let batch_idx = payload
                .get("batch_index")
                .and_then(|v| v.as_u64())
                .ok_or("Missing batch_index")? as usize;
            let mesh = wmb.meshes.get_mut(mesh_idx).ok_or("Mesh index out of bounds")?;
            if mesh.batches.len() <= 1 {
                return Err("Cannot delete the only remaining batch in a mesh".into());
            }
            if batch_idx >= mesh.batches.len() {
                return Err("Batch index out of bounds".into());
            }
            mesh.batches.remove(batch_idx);
            working_uid_map.wmb_batch_uids[mesh_idx].remove(batch_idx);
            for (bi, b) in mesh.batches.iter_mut().enumerate() {
                b.header.batch_id = bi as i16;
            }
            bump_structure = true;
            format!("Delete Mesh #{mesh_idx} Batch #{batch_idx}")
        }
        "cleanup_vertexes" => {
            wmb.cleanup_vertexes();
            bump_structure = true;
            "WMB Cleanup Unreferenced Vertexes".to_string()
        }
        "cleanup_materials" => {
            wmb.cleanup_materials();
            bump_structure = true;
            "WMB Cleanup Unused Materials".to_string()
        }
        "cleanup_material_sizes" => {
            wmb.cleanup_material_sizes().map_err(|e| e.to_string())?;
            "WMB Clamp Material Sizes to Shader Spec".to_string()
        }
        "maximize_material_sizes" => {
            wmb.maximize_material_sizes().map_err(|e| e.to_string())?;
            "WMB Maximize Material Sizes".to_string()
        }
        "remove_triangle_strips" => {
            wmb.remove_triangle_strips();
            "WMB Convert Triangle Strips to Triangles".to_string()
        }
        "normals_to_wide" => {
            wmb.normals_to_wide();
            "WMB Convert Normals to 10-10-10-2 Wide".to_string()
        }
        "check_normals" => {
            wmb.check_normals();
            "WMB Normalize & Check Vertex Normals".to_string()
        }
        "recompute_relative_positions" => {
            wmb.recompute_relative_positions();
            "WMB Recompute Relative Bone Positions".to_string()
        }
        "scale" => {
            let s = payload.get("scale").and_then(|v| v.as_f64()).unwrap_or(1.0) as f32;
            wmb.scale(s);
            format!("WMB Scale Model by {s}")
        }
        "swap_endian" => {
            target_endian = wmb.endian.swap();
            format!("WMB Convert Endianness to {target_endian:?}")
        }
        "export_obj" => {
            let out_path = payload
                .get("path")
                .and_then(|v| v.as_str())
                .ok_or("Missing OBJ export path")?;
            fs::write(out_path, wmb.export_obj()).map_err(|e| e.to_string())?;
            return build_outline_from_session(sess);
        }
        "set_material_texture" => {
            let idx = payload
                .get("index")
                .and_then(|v| v.as_u64())
                .ok_or("Missing material index")? as usize;
            let slot = payload
                .get("slot")
                .and_then(|v| v.as_u64())
                .ok_or("Missing texture slot index")? as usize;
            let tid = payload.get("texture_id").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
            let mat = wmb
                .materials
                .get_mut(idx)
                .ok_or_else(|| format!("Material index #{idx} out of bounds"))?;
            let mut dec = decode_wmb_material_rich(mat, idx, is_b2);
            let mut cur_slot = 0usize;
            let mut found = false;
            for f in &mut dec.fields {
                if f.sampler_tex_id.is_some() {
                    if cur_slot == slot {
                        f.sampler_tex_id = Some(tid);
                        found = true;
                        break;
                    }
                    cur_slot += 1;
                }
            }
            if !found {
                return Err(format!("Material #{idx} texture slot #{slot} out of bounds"));
            }
            mat.apply_decoded(&dec);
            format!("Set Material #{idx} Slot #{slot} to 0x{tid:08X}")
        }
        "set_material_param" => {
            let idx = payload
                .get("index")
                .and_then(|v| v.as_u64())
                .ok_or("Missing material index")? as usize;
            let key = payload.get("key").and_then(|v| v.as_str()).unwrap_or("");
            let vals: Vec<f32> = payload
                .get("values")
                .and_then(|v| v.as_array())
                .map(|arr| arr.iter().filter_map(|x| x.as_f64().map(|f| f as f32)).collect())
                .unwrap_or_default();
            let mat = wmb.materials.get_mut(idx).ok_or("Material index out of bounds")?;
            let mut dec = decode_wmb_material_rich(mat, idx, is_b2);
            for f in &mut dec.fields {
                if f.sampler_tex_id.is_none() && f.name.eq_ignore_ascii_case(key) {
                    let mut flts = f.float_values.unwrap_or([0.0; 4]);
                    for (k, &v) in vals.iter().take(4).enumerate() {
                        flts[k] = v;
                    }
                    f.float_values = Some(flts);
                    break;
                }
            }
            mat.apply_decoded(&dec);
            coalesce_key = Some(format!("param:{container_id}:{idx}:{key}"));
            format!("Update Material #{idx} Param {key}")
        }
        "add_bone" => {
            let parent_gid = payload.get("parent_global_id").and_then(|v| v.as_i64()).unwrap_or(-1) as i32;
            let inv = wmb.bone_index_translate_table.invert();
            let parent_idx = if parent_gid >= 0 {
                wmb.bone_index_translate_table
                    .table
                    .get(&(parent_gid as u16))
                    .copied()
                    .map(|l| l as i16)
                    .or_else(|| {
                        if (parent_gid as usize) < wmb.bone_hierarchy.len() {
                            Some(parent_gid as i16)
                        } else {
                            None
                        }
                    })
                    .unwrap_or(-1)
            } else {
                -1
            };
            let new_local = wmb.bone_hierarchy.len() as u16;
            let next_gid = inv.values().copied().max().unwrap_or(new_local).max(new_local) + 1;
            let parent_pos = if parent_idx >= 0 {
                wmb.bone_positions.get(parent_idx as usize).copied().unwrap_or_default()
            } else {
                pgtools_core::bone::Vec3::default()
            };
            wmb.bone_hierarchy.push(parent_idx);
            wmb.bone_relative_positions.push(pgtools_core::bone::Vec3::new(0.0, 0.1, 0.0));
            wmb.bone_positions.push(pgtools_core::bone::Vec3::new(parent_pos.x, parent_pos.y + 0.1, parent_pos.z));
            if !wmb.bone_symmetries.is_empty() {
                wmb.bone_symmetries.push(-1);
            }
            if !wmb.bone_flags.is_empty() {
                wmb.bone_flags.push(0);
            }
            wmb.bone_index_translate_table.table.insert(next_gid, new_local);
            wmb.header.num_bones = wmb.bone_hierarchy.len() as i32;
            bump_structure = true;
            format!("Add Bone #{new_local} (gid {next_gid})")
        }
        "delete_bone" => {
            if wmb.bone_hierarchy.len() <= 1 {
                return Err("Cannot delete the only remaining bone".into());
            }
            let gid = payload.get("global_id").and_then(|v| v.as_i64()).unwrap_or(-1) as i32;
            let del_idx = wmb
                .bone_index_translate_table
                .table
                .get(&(gid as u16))
                .copied()
                .map(|l| l as usize)
                .or_else(|| {
                    if gid >= 0 && (gid as usize) < wmb.bone_hierarchy.len() {
                        Some(gid as usize)
                    } else {
                        None
                    }
                })
                .ok_or("Bone not found")?;
            let del_parent = wmb.bone_hierarchy[del_idx];
            wmb.bone_hierarchy.remove(del_idx);
            if del_idx < wmb.bone_relative_positions.len() {
                wmb.bone_relative_positions.remove(del_idx);
            }
            if del_idx < wmb.bone_positions.len() {
                wmb.bone_positions.remove(del_idx);
            }
            if del_idx < wmb.bone_symmetries.len() {
                wmb.bone_symmetries.remove(del_idx);
            }
            if del_idx < wmb.bone_flags.len() {
                wmb.bone_flags.remove(del_idx);
            }
            for p in &mut wmb.bone_hierarchy {
                if *p == del_idx as i16 {
                    *p = if del_parent > del_idx as i16 {
                        del_parent - 1
                    } else {
                        del_parent
                    };
                } else if *p > del_idx as i16 {
                    *p -= 1;
                }
            }
            wmb.bone_index_translate_table.table.retain(|_, &mut l| (l as usize) != del_idx);
            for l in wmb.bone_index_translate_table.table.values_mut() {
                if (*l as usize) > del_idx {
                    *l -= 1;
                }
            }
            let fallback_u8 = del_parent.max(0) as u8;
            for m in &mut wmb.meshes {
                for b in &mut m.batches {
                    for br in &mut b.bone_refs {
                        if (*br as usize) == del_idx {
                            *br = if (fallback_u8 as usize) > del_idx {
                                fallback_u8.saturating_sub(1)
                            } else {
                                fallback_u8
                            };
                        } else if (*br as usize) > del_idx {
                            *br -= 1;
                        }
                    }
                }
            }
            wmb.header.num_bones = wmb.bone_hierarchy.len() as i32;
            bump_structure = true;
            format!("Delete Bone #{del_idx}")
        }
        "reorder_materials" => {
            let from_idx = payload.get("from_index").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
            let to_idx = payload.get("to_index").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
            if from_idx >= wmb.materials.len() || wmb.materials.is_empty() {
                return Err("Material index out of bounds".into());
            }
            let moved_mat = wmb.materials.remove(from_idx);
            let moved_uid = working_uid_map.material_uids.remove(from_idx);
            let ins = if to_idx > from_idx {
                to_idx.saturating_sub(1)
            } else {
                to_idx
            }
            .min(wmb.materials.len());
            wmb.materials.insert(ins, moved_mat);
            working_uid_map.material_uids.insert(ins, moved_uid);
            let remap_idx = |old: usize| -> usize {
                if old == from_idx {
                    ins
                } else if from_idx < ins && old > from_idx && old <= ins {
                    old - 1
                } else if ins < from_idx && old >= ins && old < from_idx {
                    old + 1
                } else {
                    old
                }
            };
            for m in &mut wmb.meshes {
                for b in &mut m.batches {
                    let cur = b.header.effective_material_id(is_b2);
                    let next_m = remap_idx(cur);
                    if is_b2 {
                        b.header.ex_mat_id = next_m as i16;
                    } else {
                        b.header.material_id = next_m as u8;
                    }
                }
            }
            bump_structure = true;
            format!("Reorder Material #{from_idx}")
        }
        "set_batches_material"
        | "move_items"
        | "delete_items"
        | "duplicate_items"
        | "reorder_mesh_groups"
        | "mesh_group_add"
        | "mesh_group_duplicate"
        | "mesh_group_delete"
        | "mesh_group_rename"
        | "material_add"
        | "material_duplicate"
        | "material_delete"
        | "material_rename"
        | "paste_items" => {
            let bank_tex_ids_opt = if operation == "paste_items" {
                Some(collect_paired_wtb_texture_ids(
                    &snap, node_id, oodle_opt,
                ))
            } else {
                None
            };
            let outcome = apply_wmb_scene_op(
                &mut wmb,
                &mut working_uid_map,
                &operation,
                &payload,
                clipboard_opt,
                bank_tex_ids_opt.as_deref(),
            )?;
            if outcome.structural {
                bump_structure = true;
            }
            outcome.description
        }
        "remap_texture_hashes" => {
            if !is_b2 {
                return Err("Bayonetta 1 WMBs use index-based textures and 0x8000000x pretextures; crc32 & 7f only applies to Bayonetta 2 and Bayonetta 3 / NieR.".to_string());
            }
            let mut banks = collect_paired_wtb_banks(snap, node_id, oodle_opt);
            let mut bank_refs: Vec<&mut WtbFile> =
                banks.iter_mut().map(|(_, b)| b).collect();
            let report = remap_wmb_texture_hashes(&mut wmb, &mut bank_refs, false);
            if report.remaps.is_empty() {
                let mut outline = build_outline_from_session(sess)?;
                outline.status_message = Some("All texture hashes are valid crc32 & 7f".into());
                return Ok(outline);
            }
            for idx in report.wtb_modified_indices {
                modified_wtb_banks.push(banks[idx].clone());
            }
            format!(
                "Remap {} invalid texture hash(es) (crc32 & 7f)",
                report.remaps.len()
            )
        }
        other => return Err(format!("Unknown WMB operation: {other}")),
    };

    enforce_shadow_last(&mut wmb, &mut working_uid_map);
    let new_wmb_bytes = wmb.to_bytes(target_endian).map_err(|e| e.to_string())?;
    let _ = WmbFile::from_bytes(&new_wmb_bytes)
        .map_err(|e| format!("WMB round-trip verification failed: {e}"))?;
    let dat_align = sess.settings.effective_dat_alignment();
    let mut next_snap = write_back_subblob_with_align(snap, container_parts, new_wmb_bytes, None, oodle_opt, dat_align)?;
    for (bank_node_id, wtb) in modified_wtb_banks {
        let (wtb_bytes, _) = wtb.to_bytes();
        let (bank_parts, _) = split_container_and_leaf(&bank_node_id);
        next_snap = write_back_subblob_with_align(
            &next_snap,
            &bank_parts,
            wtb_bytes,
            None,
            oodle_opt,
            dat_align,
        )?;
    }

    sess.committed_uid_maps
        .entry(container_id.to_string())
        .or_insert(init_committed);
    sess.uid_maps.insert(container_id.to_string(), working_uid_map);
    if bump_structure {
        sess.structure_version = sess.structure_version.max(1) + 1;
    }
    record_mutation_with_key(sess, desc, next_snap, coalesce_key);
    build_outline_from_session(sess)
}
