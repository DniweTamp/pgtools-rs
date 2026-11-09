use crate::session::{
    atomic_write_file, lock_clipboard, lock_session, record_mutation_with_key, AppState, DocSnapshot,
};
use crate::vfs::{
    build_outline_from_session, detect_file_format_tag, find_paired_wtp_bytes, is_bntx_blob,
    resolve_subblob, split_container_and_leaf, write_back_subblob_with_align, DocumentOutline,
};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use pgtools_core::{
    dat::DatFile,
    databases::lookup_wmb_vertex_layout,
    demo::build_demo_dat,
    scene::{
        build_wmb3_scene, build_wmb_scene, decode_texture_for_wtx1, encode_wtx1_blob,
        fnv1a_64_hex, pack_scene_blob, TextureRefDesc, UidMap,
    },
    scene_ops::{
        apply_wmb3_scene_op, apply_wmb_scene_op, copy_wmb3_to_clipboard, copy_wmb_to_clipboard,
        enforce_shadow_last, sync_wmb3_invariants, ClipboardStatus,
    },
    shader_db::{
        convert_material_between_games, decode_wmb_material_rich, list_available_shaders,
        list_core_pretextures, switch_wmb3_material_shader, switch_wmb_material_shader,
        CorePretextureInfo, ShaderOptionDesc,
    },
    texture_conv::{decode_bntx_subtexture_mip_filtered, decode_texture_mip_filtered},
    wmb::{DecodedWmbMaterial, WmbFile},
    wmb3::{Wmb3File, Wmb3Material, Wmb3NamedParameter, Wmb3TextureSlotView},
    wtb::WtbFile,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::io::Cursor;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::Path;
use std::sync::Arc;
use std::time::UNIX_EPOCH;
use tauri::State;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MaterialBankTextureOption {
    pub index: usize,
    pub texture_id: u32,
    pub texture_id_hex: String,
    pub label: String,
    pub format_name: String,
    pub width: u32,
    pub height: u32,
    pub node_id: String,
    pub thumbnail_data_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MaterialPickerOption {
    pub index: usize,
    pub label: String,
    pub shader_name: String,
    pub badge: String,
    pub node_id: String,
    pub first_tex_id_hex: Option<String>,
    pub thumbnail_data_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WmbSectionView {
    pub node_id: String,
    pub section: String,
    pub title: String,
    pub breadcrumb: String,
    pub spec_line: String,
    pub validation_msg: String,
    pub is_wmb3: bool,
    pub is_bayo2: bool,
    pub endian: String,
    pub data: serde_json::Value,
    pub material_decoded: Option<DecodedWmbMaterial>,
    pub wmb3_material: Option<Wmb3Material>,
    pub wmb3_named_params: Vec<Wmb3NamedParameter>,
    pub wmb3_texture_views: Vec<Wmb3TextureSlotView>,
    pub bank_textures: Vec<MaterialBankTextureOption>,
    pub core_pretextures: Vec<CorePretextureInfo>,
    pub available_shaders: Vec<ShaderOptionDesc>,
    pub material_options: Vec<MaterialPickerOption>,
}

pub(crate) fn canonical_model_key(node_id: &str) -> String {
    let (container_parts, _) = split_container_and_leaf(node_id);
    container_parts.join("/")
}

/// Fast path for `paste_items`: read only texture IDs from paired texture banks without parsing/cloning payloads.
pub(crate) fn collect_paired_wtb_texture_ids(
    snap: &DocSnapshot,
    model_node_id: &str,
    oodle_dll: Option<&Path>,
) -> Vec<u32> {
    let mut ids = Vec::new();
    let (container_parts, _) = split_container_and_leaf(model_node_id);

    if let Some(last) = container_parts.last() {
        if last.starts_with("scrmdl:") {
            let parent_parts = &container_parts[..container_parts.len() - 1];
            let scr_tex_id = if parent_parts.is_empty() {
                "scrtex:0".to_string()
            } else {
                format!("{}/scrtex:0", parent_parts.join("/"))
            };
            if let Ok((_, wtb_bytes, _, _)) = resolve_subblob(snap, &scr_tex_id, oodle_dll) {
                if let Ok(bank_ids) = WtbFile::read_texture_ids_only(&wtb_bytes) {
                    ids.extend(bank_ids);
                }
            }
            return ids;
        }
    }

    let mut candidate_roots: Vec<Vec<u8>> = Vec::new();
    if let DocSnapshot::PairedDatDtt { dat, dtt } = snap {
        candidate_roots.push(dat.as_ref().clone());
        candidate_roots.push(dtt.as_ref().clone());
    } else if !container_parts.is_empty() {
        let parent_parts = &container_parts[..container_parts.len() - 1];
        let parent_id = parent_parts.join("/");
        if let Ok((_, parent_bytes, _, _)) = resolve_subblob(snap, &parent_id, oodle_dll) {
            candidate_roots.push(parent_bytes);
        }
    }

    for dat_bytes in candidate_roots {
        if let Ok(dat) = DatFile::from_bytes(&dat_bytes) {
            for e in &dat.entries {
                let ext = e.extension.to_ascii_lowercase();
                if ext == "wtb" || ext == "wta" || ext == "bntx" {
                    if let Ok(bank_ids) = WtbFile::read_texture_ids_only(&e.data) {
                        ids.extend(bank_ids);
                    }
                }
            }
        }
    }

    ids
}

/// Discover all sibling texture banks (`.wtb`, `.wta`+`.wtp`, `.bntx`) for `model_node_id`.
/// Returns `(bank_node_id, wtb_file)` pairs.
pub(crate) fn collect_paired_wtb_banks(
    snap: &DocSnapshot,
    model_node_id: &str,
    oodle_dll: Option<&Path>,
) -> Vec<(String, WtbFile)> {
    let mut banks = Vec::new();
    let (container_parts, _) = split_container_and_leaf(model_node_id);

    // 1. Model inside SCR (`.../scrmdl:X`)
    if let Some(last) = container_parts.last() {
        if last.starts_with("scrmdl:") {
            let parent_parts = &container_parts[..container_parts.len() - 1];
            let scr_tex_id = if parent_parts.is_empty() {
                "scrtex:0".to_string()
            } else {
                format!("{}/scrtex:0", parent_parts.join("/"))
            };
            if let Ok((_, wtb_bytes, _, _)) = resolve_subblob(snap, &scr_tex_id, oodle_dll) {
                if let Ok(wtb) = WtbFile::from_bytes(&wtb_bytes, None) {
                    banks.push((scr_tex_id, wtb));
                }
            }
            return banks;
        }
    }

    // 2. Model inside DAT / Paired DAT+DTT / PKZ
    let mut candidate_roots: Vec<(String, Vec<u8>, Option<Vec<u8>>)> = Vec::new();
    if let DocSnapshot::PairedDatDtt { dat, dtt } = snap {
        candidate_roots.push((
            "root:dat".to_string(),
            dat.as_ref().clone(),
            Some(dtt.as_ref().clone()),
        ));
        candidate_roots.push(("root:dtt".to_string(), dtt.as_ref().clone(), None));
    } else if !container_parts.is_empty() {
        let parent_parts = &container_parts[..container_parts.len() - 1];
        let parent_id = parent_parts.join("/");
        if let Ok((_, parent_bytes, _, _)) = resolve_subblob(snap, &parent_id, oodle_dll) {
            candidate_roots.push((parent_id, parent_bytes, None));
        }
    }

    for (prefix, dat_bytes, peer_dtt) in candidate_roots {
        if let Ok(dat) = DatFile::from_bytes(&dat_bytes) {
            let parsed_dtt = peer_dtt.as_ref().and_then(|b| DatFile::from_bytes(b).ok());
            let dtt_slice = parsed_dtt.as_ref().map(|d| d.entries.as_slice());
            let total_wtas = dat
                .entries
                .iter()
                .filter(|e| e.extension.eq_ignore_ascii_case("wta"))
                .count();
            let mut wta_ord = 0usize;

            for (idx, e) in dat.entries.iter().enumerate() {
                let ext = e.extension.to_ascii_lowercase();
                if ext == "wtb" || ext == "wta" || ext == "bntx" {
                    let wtp = if ext == "wta" {
                        let m = find_paired_wtp_bytes(
                            &e.name,
                            wta_ord,
                            total_wtas,
                            &dat.entries,
                            dtt_slice,
                        );
                        wta_ord += 1;
                        m
                    } else {
                        None
                    };
                    let bank_node_id = if prefix.is_empty() {
                        format!("dat:{idx}")
                    } else {
                        format!("{prefix}/dat:{idx}")
                    };
                    if let Ok(wtb) = WtbFile::from_bytes(&e.data, wtp.as_deref()) {
                        banks.push((bank_node_id, wtb));
                    }
                }
            }
        }
    }

    banks
}

/// Build `TextureRefDesc` entries for EVERY texture in the paired bank(s) outside the session lock,
/// returning `(texture_refs, newly_decoded_thumbnails)`.
fn build_bank_texture_refs_unlocked(
    existing_thumb_cache: &HashMap<String, (String, Option<String>)>,
    snap: &DocSnapshot,
    model_node_id: &str,
    oodle_dll: Option<&Path>,
) -> (Vec<TextureRefDesc>, Vec<(String, (String, Option<String>))>) {
    let banks = collect_paired_wtb_banks(snap, model_node_id, oodle_dll);
    let mut out = Vec::new();
    let mut new_thumbs = Vec::new();

    for (_bank_node_id, wtb) in banks {
        for e in &wtb.entries {
            let tid = e.texture_id.unwrap_or(e.index as u32);
            let (fmt_name, w, h, meta_name) = e
                .metadata
                .as_ref()
                .map(|m| (m.format.display_name(), m.width, m.height, m.name.clone()))
                .unwrap_or_else(|| ("TEX".to_string(), 0, 0, None));
            let name = meta_name.unwrap_or_else(|| format!("tex{}", e.index));

            let blob_key = if !e.content_hash.is_empty() {
                e.content_hash.clone()
            } else {
                let mut key_hasher = pgtools_core::scene::Fnv1a64::new();
                key_hasher.update(&(e.index as u32).to_le_bytes());
                key_hasher.update(&e.data);
                key_hasher.finish_hex()
            };

            let (content_hash, thumb_url) =
                if let Some(cached) = existing_thumb_cache.get(&blob_key) {
                    cached.clone()
                } else {
                    let is_bntx = is_bntx_blob(&e.data);
                    let decoded_res = if is_bntx {
                        decode_bntx_subtexture_mip_filtered(&e.data, e.index, 0, "RGBA", false)
                    } else {
                        decode_texture_mip_filtered(&e.data, 0, "RGBA", false)
                    };
                    let pair = match decoded_res {
                        Ok((img, _, _)) => {
                            let c_hash = fnv1a_64_hex(img.as_raw());
                            let small = if img.width() > 96 || img.height() > 96 {
                                image::imageops::thumbnail(&img, 96, 96)
                            } else {
                                img
                            };
                            let mut png_buf = Cursor::new(Vec::new());
                            let url = if small.write_to(&mut png_buf, image::ImageFormat::Png).is_ok()
                            {
                                Some(format!(
                                    "data:image/png;base64,{}",
                                    BASE64.encode(png_buf.into_inner())
                                ))
                            } else {
                                None
                            };
                            (c_hash, url)
                        }
                        Err(_) => (blob_key.clone(), None),
                    };
                    new_thumbs.push((blob_key, pair.clone()));
                    pair
                };

            out.push(TextureRefDesc {
                texture_id: tid,
                id_hex: format!("0x{tid:08X}"),
                name,
                width: w,
                height: h,
                format_name: fmt_name,
                content_hash,
                used_by_material_ids: Vec::new(),
                thumbnail_data_url: thumb_url,
            });
        }
    }

    (out, new_thumbs)
}

#[tauri::command]
pub fn get_wmb_scene(
    node_id: String,
    known_geometry_hashes: Option<Vec<String>>,
    state: State<'_, AppState>,
) -> Result<tauri::ipc::Response, String> {
    let (snap, doc_name, oodle_path, structure_version, mut uid_map, thumb_snapshot, model_key) = {
        let sess = lock_session(&state);
        let snap = sess.current.clone().ok_or("No active document loaded")?;
        let doc_name = sess
            .doc_path
            .as_ref()
            .and_then(|p| p.file_name())
            .and_then(|s| s.to_str())
            .unwrap_or("model.wmb")
            .to_string();
        let oodle_path = sess.settings.oodle_dll_path.clone();
        let structure_version = sess.structure_version.max(1);
        let model_key = canonical_model_key(&node_id);
        let uid_map = sess.uid_maps.get(&model_key).cloned().unwrap_or_else(UidMap::new);
        let thumb_snapshot = sess.tex_thumb_cache.clone();
        (
            snap,
            doc_name,
            oodle_path,
            structure_version,
            uid_map,
            thumb_snapshot,
            model_key,
        )
    };

    let work_res = catch_unwind(AssertUnwindSafe(|| -> Result<_, String> {
        let oodle_opt = if oodle_path.is_empty() {
            None
        } else {
            Some(Path::new(oodle_path.as_str()))
        };
        let (sub_name, sub_bytes, _, _) = resolve_subblob(&snap, &model_key, oodle_opt)?;
        let file_name = if sub_name == "root" || sub_name.is_empty() {
            doc_name
        } else {
            sub_name
        };

        let (bank_textures, new_thumbs) =
            build_bank_texture_refs_unlocked(&thumb_snapshot, &snap, &node_id, oodle_opt);
        let known = known_geometry_hashes.unwrap_or_default();
        let tag = detect_file_format_tag(&file_name, &sub_bytes);

        let (manifest, binary) = if tag == "WMB3" {
            let wmb3 = Wmb3File::from_bytes(&sub_bytes).map_err(|e| e.to_string())?;
            build_wmb3_scene(
                &wmb3,
                &mut uid_map,
                &node_id,
                &file_name,
                structure_version,
                bank_textures,
                &known,
            )
        } else if tag == "WMB" {
            let wmb = WmbFile::from_bytes(&sub_bytes).map_err(|e| e.to_string())?;
            build_wmb_scene(
                &wmb,
                &mut uid_map,
                &node_id,
                &file_name,
                structure_version,
                bank_textures,
                &known,
            )
        } else {
            return Err(format!(
                "Node '{node_id}' is not a WMB or WMB3 model (detected {tag})"
            ));
        };

        let wire_bytes = pack_scene_blob(&manifest, &binary).map_err(|e| e.to_string())?;
        Ok((wire_bytes, uid_map, new_thumbs))
    }))
    .map_err(|_| "Internal panic while building WMB scene".to_string())?;

    let (wire_bytes, updated_uid_map, new_thumbs) = work_res?;

    {
        let mut sess = lock_session(&state);
        for (k, v) in new_thumbs {
            sess.insert_thumb_cached(k, v);
        }
        sess.uid_maps.insert(model_key.clone(), updated_uid_map.clone());
        sess.committed_uid_maps
            .entry(model_key)
            .or_insert(updated_uid_map);
    }

    Ok(tauri::ipc::Response::new(wire_bytes))
}

#[tauri::command]
pub fn get_wmb_texture(
    node_id: String,
    texture_id: u32,
    max_dim: u32,
    state: State<'_, AppState>,
) -> Result<tauri::ipc::Response, String> {
    let (snap, oodle_path) = {
        let sess = lock_session(&state);
        let snap = sess.current.clone().ok_or("No active document loaded")?;
        let oodle_path = sess.settings.oodle_dll_path.clone();
        (snap, oodle_path)
    };
    let oodle_opt = if oodle_path.is_empty() {
        None
    } else {
        Some(Path::new(oodle_path.as_str()))
    };

    let model_key = canonical_model_key(&node_id);
    let banks = collect_paired_wtb_banks(&snap, &node_id, oodle_opt);
    let mut matched_entry = None;

    for (_bank_id, wtb) in &banks {
        for e in &wtb.entries {
            let tid = e.texture_id.unwrap_or(e.index as u32);
            if tid == texture_id {
                matched_entry = Some(e.clone());
                break;
            }
        }
        if matched_entry.is_some() {
            break;
        }
    }
    if matched_entry.is_none() {
        for (_bank_id, wtb) in &banks {
            if let Some(e) = wtb.entries.get(texture_id as usize) {
                matched_entry = Some(e.clone());
                break;
            }
        }
    }

    let entry = matched_entry.ok_or_else(|| {
        format!("Texture 0x{texture_id:08X} not found in paired bank for '{node_id}'")
    })?;

    let blob_hash = if !entry.content_hash.is_empty() {
        entry.content_hash.clone()
    } else {
        fnv1a_64_hex(&entry.data)
    };
    let cache_key = (format!("{model_key}:{blob_hash}"), texture_id, max_dim);

    {
        let sess = lock_session(&state);
        if let Some(cached) = sess.wtx1_cache.get(&cache_key) {
            return Ok(tauri::ipc::Response::new(cached.clone()));
        }
    }

    let wtx1 = catch_unwind(AssertUnwindSafe(|| -> Result<Vec<u8>, String> {
        let (w, h, is_cube, rgba) =
            decode_texture_for_wtx1(&entry.data, entry.index, entry.flags, max_dim).map_err(|e| {
                format!("Failed to decode texture 0x{texture_id:08X}: {e}")
            })?;
        Ok(encode_wtx1_blob(w, h, is_cube, &rgba))
    }))
    .map_err(|_| format!("Internal panic decoding texture 0x{texture_id:08X}"))??;

    {
        let mut sess = lock_session(&state);
        sess.insert_wtx1_cached(cache_key, wtx1.clone());
    }
    Ok(tauri::ipc::Response::new(wtx1))
}

#[tauri::command]
pub fn get_pre_texture(
    game: String,
    index: u32,
    max_dim: u32,
    state: State<'_, AppState>,
) -> Result<tauri::ipc::Response, String> {
    let g = game.trim().to_ascii_lowercase();
    let core_path_str = {
        let sess = lock_session(&state);
        match g.as_str() {
            "b1" | "wmb1" => sess.settings.core_dat_b1.clone(),
            "b2" | "wmb2" => sess.settings.core_dat_b2.clone(),
            "b3" | "wmb3" => sess.settings.core_dat_b3.clone(),
            _ => String::new(),
        }
    };

    if core_path_str.trim().is_empty() {
        return Err(format!("core.dat path is not configured for game '{game}'"));
    }

    let mtime_ms = std::fs::metadata(&core_path_str)
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);

    let cache_key = (
        format!("pre:{g}:{core_path_str}:{mtime_ms}"),
        index,
        max_dim,
    );

    let cached_wtb_opt = {
        let sess = lock_session(&state);
        if let Some(cached) = sess.wtx1_cache.get(&cache_key) {
            return Ok(tauri::ipc::Response::new(cached.clone()));
        }
        sess.pre_wtb_cache
            .get(&core_path_str)
            .and_then(|(cached_mtime, wtb_arc)| {
                if *cached_mtime == mtime_ms {
                    Some(wtb_arc.clone())
                } else {
                    None
                }
            })
    };

    let wtb_arc = if let Some(existing) = cached_wtb_opt {
        existing
    } else {
        let raw = std::fs::read(&core_path_str)
            .map_err(|e| format!("Failed to read core.dat at '{core_path_str}': {e}"))?;

        let parsed_wtb = if raw.starts_with(b"WTB\0") || raw.starts_with(b"\0BTW") {
            WtbFile::from_bytes(&raw, None)
                .map_err(|e| format!("Invalid pre_texture WTB: {e}"))?
        } else {
            let dat = DatFile::from_bytes(&raw)
                .map_err(|e| format!("Invalid core.dat archive: {e}"))?;
            let pre_entry = dat
                .entries
                .iter()
                .find(|e| {
                    let base = e
                        .name
                        .trim_matches('\0')
                        .rsplit(['/', '\\'])
                        .next()
                        .unwrap_or(&e.name)
                        .to_ascii_lowercase();
                    base == "pre_texture.wtb" || base == "pre_texture.wta"
                })
                .or_else(|| {
                    dat.entries
                        .iter()
                        .find(|e| e.extension.eq_ignore_ascii_case("wtb"))
                })
                .ok_or_else(|| {
                    "pre_texture.wtb not found inside configured core.dat".to_string()
                })?;
            let wtp_opt = if pre_entry.extension.eq_ignore_ascii_case("wta") {
                dat.entries
                    .iter()
                    .find(|e| {
                        let base = e
                            .name
                            .trim_matches('\0')
                            .rsplit(['/', '\\'])
                            .next()
                            .unwrap_or(&e.name)
                            .to_ascii_lowercase();
                        base == "pre_texture.wtp"
                    })
                    .or_else(|| {
                        dat.entries
                            .iter()
                            .find(|e| e.extension.eq_ignore_ascii_case("wtp"))
                    })
                    .map(|e| e.data.as_slice())
            } else {
                None
            };
            WtbFile::from_bytes(&pre_entry.data, wtp_opt)
                .map_err(|e| format!("Failed to parse pre_texture.wtb: {e}"))?
        };

        let arc = Arc::new(parsed_wtb);
        {
            let mut sess = lock_session(&state);
            sess.pre_wtb_cache
                .insert(core_path_str.clone(), (mtime_ms, arc.clone()));
        }
        arc
    };

    let slot_idx = (index & 0xFFFF) as usize;
    let entry = wtb_arc
        .entries
        .get(slot_idx)
        .or_else(|| wtb_arc.entries.iter().find(|e| e.texture_id == Some(index)))
        .ok_or_else(|| format!("pre_texture slot {slot_idx} (0x{index:08X}) out of range"))?;

    let wtx1 = catch_unwind(AssertUnwindSafe(|| -> Result<Vec<u8>, String> {
        let (w, h, is_cube, rgba) =
            decode_texture_for_wtx1(&entry.data, entry.index, entry.flags, max_dim).map_err(|e| {
                format!("Failed to decode pre_texture slot {slot_idx}: {e}")
            })?;
        Ok(encode_wtx1_blob(w, h, is_cube, &rgba))
    }))
    .map_err(|_| format!("Internal panic decoding pre_texture slot {slot_idx}"))??;

    {
        let mut sess = lock_session(&state);
        sess.insert_wtx1_cached(cache_key, wtx1.clone());
    }

    Ok(tauri::ipc::Response::new(wtx1))
}

#[tauri::command]
pub fn resolve_legacy_node(
    node_id: String,
    item_id: String,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let mut sess = lock_session(&state);
    let snap = sess.current.clone().ok_or("No active document loaded")?;
    let oodle_path = sess.settings.oodle_dll_path.clone();
    let oodle_opt = if oodle_path.is_empty() {
        None
    } else {
        Some(Path::new(oodle_path.as_str()))
    };

    let (container_parts, _) = split_container_and_leaf(&node_id);
    let model_key = container_parts.join("/");
    let (sub_name, sub_bytes, _, _) = resolve_subblob(&snap, &model_key, oodle_opt)?;
    let tag = detect_file_format_tag(&sub_name, &sub_bytes);
    let is_wmb3 = tag == "WMB3";
    let wmb_tag = if is_wmb3 { "wmb3" } else { "wmb" };
    let base_prefix = if model_key.is_empty() {
        wmb_tag.to_string()
    } else {
        format!("{model_key}/{wmb_tag}")
    };

    let trimmed = item_id.trim();
    if trimmed.is_empty() || trimmed == "root" || trimmed == "header" {
        return Ok(format!("{base_prefix}:header"));
    }
    if trimmed == "materials" || trimmed == "meshes" || trimmed == "bones" || trimmed == "vertexes" {
        return Ok(format!("{base_prefix}:{trimmed}"));
    }
    if trimmed == "textures" {
        let banks = collect_paired_wtb_banks(&snap, &node_id, oodle_opt);
        if let Some((bank_id, _)) = banks.first() {
            return Ok(format!("{bank_id}/tex:0"));
        }
        return Err("No paired texture bank found for this model".to_string());
    }

    // Texture resolution (`tex:<textureId>`) -> resolves to `<bank_node_id>/tex:<entry_index>`
    if let Some(tex_str) = trimmed.strip_prefix("tex:") {
        let tid = if let Some(hex) = tex_str.strip_prefix("0x").or_else(|| tex_str.strip_prefix("0X")) {
            u32::from_str_radix(hex, 16).map_err(|e| e.to_string())?
        } else {
            tex_str.parse::<u32>().map_err(|e| e.to_string())?
        };
        let banks = collect_paired_wtb_banks(&snap, &node_id, oodle_opt);
        for (bank_id, wtb) in &banks {
            for e in &wtb.entries {
                if e.texture_id.unwrap_or(e.index as u32) == tid {
                    return Ok(format!("{bank_id}/tex:{}", e.index));
                }
            }
        }
        for (bank_id, wtb) in &banks {
            if (tid as usize) < wtb.entries.len() {
                return Ok(format!("{bank_id}/tex:{tid}"));
            }
        }
        return Err(format!("Texture {trimmed} not found in paired bank"));
    }

    let mut uid_map = sess.uid_maps.get(&model_key).cloned().unwrap_or_else(UidMap::new);
    let resolved = if is_wmb3 {
        let wmb3 = Wmb3File::from_bytes(&sub_bytes).map_err(|e| e.to_string())?;
        uid_map.ensure_wmb3(&wmb3);
        if trimmed.starts_with("mat:") {
            let mi = uid_map
                .material_index_by_id(trimmed)
                .ok_or_else(|| format!("Material '{trimmed}' not found"))?;
            Ok(format!("{base_prefix}:mat:{mi}"))
        } else if trimmed.starts_with("batch:") {
            let bi = uid_map
                .wmb3_batch_index_by_id(trimmed)
                .ok_or_else(|| format!("Batch '{trimmed}' not found"))?;
            Ok(format!("{base_prefix}:batch:{bi}"))
        } else if trimmed.starts_with("mesh:") {
            let mi = uid_map
                .mesh_index_by_id(trimmed)
                .ok_or_else(|| format!("Mesh group '{trimmed}' not found"))?;
            Ok(format!("{base_prefix}:mesh:{mi}"))
        } else if trimmed.starts_with("lod:") {
            let (mi, lvl) = uid_map
                .parse_lod_id(trimmed)
                .ok_or_else(|| format!("LOD '{trimmed}' not found"))?;
            Ok(format!("{base_prefix}:lod:{mi}:{lvl}"))
        } else if let Some(bone_str) = trimmed.strip_prefix("bone:") {
            let gid: i32 = bone_str
                .parse()
                .map_err(|_| format!("Invalid bone id '{trimmed}'"))?;
            let bi = wmb3
                .bones
                .iter()
                .position(|b| b.id as i32 == gid)
                .or_else(|| {
                    if gid >= 0 && (gid as usize) < wmb3.bones.len() {
                        Some(gid as usize)
                    } else {
                        None
                    }
                })
                .ok_or_else(|| format!("Bone '{trimmed}' not found"))?;
            Ok(format!("{base_prefix}:bone:{bi}"))
        } else {
            Err(format!("Unrecognized scene item id '{trimmed}'"))
        }
    } else {
        let wmb = WmbFile::from_bytes(&sub_bytes).map_err(|e| e.to_string())?;
        uid_map.ensure_wmb(&wmb);
        if trimmed.starts_with("mat:") {
            let mi = uid_map
                .material_index_by_id(trimmed)
                .ok_or_else(|| format!("Material '{trimmed}' not found"))?;
            Ok(format!("{base_prefix}:mat:{mi}"))
        } else if trimmed.starts_with("batch:") {
            let (mi, bi) = uid_map
                .wmb_batch_pos_by_id(trimmed)
                .ok_or_else(|| format!("Batch '{trimmed}' not found"))?;
            Ok(format!("{base_prefix}:mesh:{mi}:batch:{bi}"))
        } else if trimmed.starts_with("mesh:") {
            let mi = uid_map
                .mesh_index_by_id(trimmed)
                .ok_or_else(|| format!("Mesh group '{trimmed}' not found"))?;
            Ok(format!("{base_prefix}:mesh:{mi}"))
        } else if let Some(bone_str) = trimmed.strip_prefix("bone:") {
            let gid: i32 = bone_str
                .parse()
                .map_err(|_| format!("Invalid bone id '{trimmed}'"))?;
            let bones = wmb.dump_bones();
            let bi = bones
                .iter()
                .position(|b| b.global_index as i32 == gid)
                .or_else(|| {
                    if gid >= 0 && (gid as usize) < bones.len() {
                        Some(gid as usize)
                    } else {
                        None
                    }
                })
                .ok_or_else(|| format!("Bone '{trimmed}' not found"))?;
            Ok(format!("{base_prefix}:bone:{bi}"))
        } else {
            Err(format!("Unrecognized scene item id '{trimmed}'"))
        }
    };

    let out = resolved?;
    sess.uid_maps.insert(model_key.clone(), uid_map.clone());
    sess.committed_uid_maps.entry(model_key).or_insert(uid_map);
    Ok(out)
}

#[tauri::command]
pub fn create_demo_model(game: String) -> Result<String, String> {
    let clean_game = match game.trim().to_ascii_lowercase().as_str() {
        "b1" | "bayo1" | "wmb1" => "b1",
        "b2" | "bayo2" | "wmb2" => "b2",
        "b3" | "bayo3" | "wmb3" => "b3",
        other => return Err(format!("Unsupported demo model game '{other}'")),
    };
    let dat_bytes = build_demo_dat(clean_game).map_err(|e| e.to_string())?;
    let demo_dir = std::env::temp_dir().join(format!("pgtools-studio-{}", std::process::id()));
    std::fs::create_dir_all(&demo_dir).map_err(|e| e.to_string())?;
    let out_path = demo_dir.join(format!("pgtools-demo-{clean_game}.dat"));
    atomic_write_file(&out_path, &dat_bytes)?;
    Ok(out_path.to_string_lossy().into_owned())
}

#[tauri::command]
pub fn wmb_clipboard_copy(
    node_id: String,
    items: Vec<String>,
    state: State<'_, AppState>,
) -> Result<ClipboardStatus, String> {
    let clip_data = {
        let mut sess = lock_session(&state);
        let snap = sess.current.clone().ok_or("No active document loaded")?;
        let oodle_path = sess.settings.oodle_dll_path.clone();
        let oodle_opt = if oodle_path.is_empty() {
            None
        } else {
            Some(Path::new(oodle_path.as_str()))
        };
        let model_key = canonical_model_key(&node_id);
        let (sub_name, sub_bytes, _, _) = resolve_subblob(&snap, &model_key, oodle_opt)?;
        let tag = detect_file_format_tag(&sub_name, &sub_bytes);
        let mut uid_map = sess.uid_maps.get(&model_key).cloned().unwrap_or_else(UidMap::new);

        let res = if tag == "WMB3" {
            let wmb3 = Wmb3File::from_bytes(&sub_bytes).map_err(|e| e.to_string())?;
            copy_wmb3_to_clipboard(&wmb3, &mut uid_map, &items)
        } else if tag == "WMB" {
            let wmb = WmbFile::from_bytes(&sub_bytes).map_err(|e| e.to_string())?;
            copy_wmb_to_clipboard(&wmb, &mut uid_map, &items)
        } else {
            Err(format!("Node '{node_id}' is not a WMB or WMB3 model"))
        }?;
        sess.uid_maps.insert(model_key.clone(), uid_map.clone());
        sess.committed_uid_maps.entry(model_key).or_insert(uid_map);
        res
    };

    let status = clip_data.status();
    *lock_clipboard(&state) = Some(clip_data);
    Ok(status)
}

#[tauri::command]
pub fn wmb_clipboard_status(state: State<'_, AppState>) -> Result<Option<ClipboardStatus>, String> {
    let guard = lock_clipboard(&state);
    Ok(guard.as_ref().map(|c| c.status()))
}

/// Discover all textures in sibling `.wtb` / `.wta` / `.bntx` banks for a model inside a `.dat`/`.dtt`/`.scr`
/// and generate small thumbnail previews for referenced textures.
pub(crate) fn collect_model_bank_textures(
    snap: &DocSnapshot,
    model_node_id: &str,
    referenced_ids: &[u32],
    is_b1_indices: bool,
    oodle_dll: Option<&Path>,
) -> Vec<MaterialBankTextureOption> {
    let banks = collect_paired_wtb_banks(snap, model_node_id, oodle_dll);
    let mut out = Vec::new();
    for (bank_node_id, wtb) in banks {
        for e in &wtb.entries {
            let tid = e.texture_id.unwrap_or(e.index as u32);
            let (fmt_name, w, h) = e
                .metadata
                .as_ref()
                .map(|m| (m.format.display_name(), m.width, m.height))
                .unwrap_or_else(|| ("TEX".to_string(), 0, 0));
            let is_ref = if is_b1_indices {
                referenced_ids.contains(&(e.index as u32))
            } else {
                referenced_ids.contains(&tid) || referenced_ids.contains(&(e.index as u32))
            };
            let thumb = if is_ref || e.index < 64 {
                let decoded_res = if is_bntx_blob(&e.data) {
                    decode_bntx_subtexture_mip_filtered(&e.data, e.index, 0, "RGBA", false)
                } else {
                    decode_texture_mip_filtered(&e.data, 0, "RGBA", false)
                };
                decoded_res.ok().and_then(|(img, _, _)| {
                    let small = if img.width() > 96 || img.height() > 96 {
                        image::imageops::thumbnail(&img, 96, 96)
                    } else {
                        img
                    };
                    let mut buf = Cursor::new(Vec::new());
                    if small.write_to(&mut buf, image::ImageFormat::Png).is_ok() {
                        Some(format!(
                            "data:image/png;base64,{}",
                            BASE64.encode(buf.into_inner())
                        ))
                    } else {
                        None
                    }
                })
            } else {
                None
            };
            let tex_node_id = format!("{bank_node_id}/tex:{}", e.index);
            out.push(MaterialBankTextureOption {
                index: e.index,
                texture_id: tid,
                texture_id_hex: format!("0x{tid:08X}"),
                label: format!("tex{} (0x{tid:08X}) — {fmt_name} {w}×{h}", e.index),
                format_name: fmt_name,
                width: w,
                height: h,
                node_id: tex_node_id,
                thumbnail_data_url: thumb,
            });
        }
    }
    out
}


fn build_wmb1_material_options(
    snap: &DocSnapshot,
    node_id: &str,
    wmb: &WmbFile,
    is_b2: bool,
    oodle_dll: Option<&Path>,
) -> (Vec<MaterialPickerOption>, Vec<MaterialBankTextureOption>) {
    let (container_parts, _) = split_container_and_leaf(node_id);
    let base_prefix = if container_parts.is_empty() {
        "wmb".to_string()
    } else {
        format!("{}/wmb", container_parts.join("/"))
    };
    let mut all_ref_ids = Vec::new();
    let mut decoded_mats = Vec::with_capacity(wmb.materials.len());
    for (mi, m) in wmb.materials.iter().enumerate() {
        let dec = decode_wmb_material_rich(m, mi, is_b2);
        for f in &dec.fields {
            if let Some(tid) = f.sampler_tex_id {
                if !all_ref_ids.contains(&tid) {
                    all_ref_ids.push(tid);
                }
            }
        }
        decoded_mats.push((mi, m.mat_type, dec));
    }
    let bank_textures = collect_model_bank_textures(snap, node_id, &all_ref_ids, !is_b2, oodle_dll);
    let mut mat_options = Vec::with_capacity(decoded_mats.len());
    for (mi, mat_type, dec) in decoded_mats {
        let first_tid = dec.fields.iter().find_map(|f| f.sampler_tex_id);
        let matched_bank = first_tid.and_then(|tid| {
            if !is_b2 {
                bank_textures.iter().find(|bt| bt.index as u32 == tid || bt.texture_id == tid)
            } else {
                bank_textures.iter().find(|bt| bt.texture_id == tid || bt.index as u32 == tid)
            }
        });
        let shader_s = dec.shader_name.clone().unwrap_or_else(|| "Unknown".to_string());
        mat_options.push(MaterialPickerOption {
            index: mi,
            label: format!("Material #{mi} ({shader_s})"),
            shader_name: shader_s,
            badge: format!("0x{mat_type:02X}"),
            node_id: format!("{base_prefix}:mat:{mi}"),
            first_tex_id_hex: first_tid.map(|t| format!("0x{t:08X}")),
            thumbnail_data_url: matched_bank.and_then(|b| b.thumbnail_data_url.clone()),
        });
    }
    (mat_options, bank_textures)
}

pub(crate) fn build_wmb1_section_view(
    snap: &DocSnapshot,
    node_id: &str,
    model_name: &str,
    wmb: &WmbFile,
    section_key: &str,
    b2_override: Option<bool>,
    oodle_dll: Option<&Path>,
) -> WmbSectionView {
    let is_b2 = b2_override.unwrap_or_else(|| wmb.is_bayo2());
    let endian_str = format!("{:?}", wmb.endian);
    let spec_line = format!(
        "{} | {endian_str} endian | {} verts | {} bones | {} materials | {} meshes",
        if is_b2 {
            "WMB2 (Bayonetta 2)"
        } else {
            "WMB1 (Bayonetta 1)"
        },
        wmb.vertexes.len(),
        wmb.bone_hierarchy.len(),
        wmb.materials.len(),
        wmb.meshes.len()
    );
    let needs_material_options = section_key == "materials"
        || section_key.starts_with("mat:")
        || section_key == "meshes"
        || section_key.starts_with("mesh:");
    let material_options = if needs_material_options {
        build_wmb1_material_options(snap, node_id, wmb, is_b2, oodle_dll).0
    } else {
        Vec::new()
    };

    if let Some(mat_idx_str) = section_key.strip_prefix("mat:") {
        if let Ok(mi) = mat_idx_str.parse::<usize>() {
            if let Some(mat) = wmb.materials.get(mi) {
                let dec = decode_wmb_material_rich(mat, mi, is_b2);
                let ref_ids: Vec<u32> = dec
                    .fields
                    .iter()
                    .filter_map(|f| f.sampler_tex_id)
                    .collect();
                let bank_textures =
                    collect_model_bank_textures(snap, node_id, &ref_ids, !dec.is_bayo2, oodle_dll);
                let game_key = if dec.is_bayo2 { "b2" } else { "b1" };
                return WmbSectionView {
                    node_id: node_id.into(),
                    section: format!("mat:{mi}"),
                    title: format!("Material #{mi}"),
                    breadcrumb: format!("{model_name} / Materials / #{mi}"),
                    spec_line,
                    validation_msg: format!(
                        "Shader {} (Type 0x{:02X}) · {} bytes",
                        dec.shader_name.as_deref().unwrap_or("Unknown"),
                        mat.mat_type,
                        mat.byte_size()
                    ),
                    is_wmb3: false,
                    is_bayo2: dec.is_bayo2,
                    endian: endian_str,
                    data: serde_json::to_value(&dec).unwrap_or_default(),
                    material_decoded: Some(dec),
                    wmb3_material: None,
                    wmb3_named_params: Vec::new(),
                    wmb3_texture_views: Vec::new(),
                    bank_textures,
                    core_pretextures: list_core_pretextures(game_key),
                    available_shaders: list_available_shaders(game_key),
                    material_options: material_options.clone(),
                };
            }
        }
    }

    if let Some(rest) = section_key.strip_prefix("mesh:") {
        let parts: Vec<&str> = rest.split(":batch:").collect();
        if let Ok(mesh_idx) = parts[0].parse::<usize>() {
            if let Some(mesh) = wmb.meshes.get(mesh_idx) {
                if parts.len() == 2 {
                    if let Ok(bi) = parts[1].parse::<usize>() {
                        if let Some(batch) = mesh.batches.get(bi) {
                            return WmbSectionView {
                                node_id: node_id.into(),
                                section: format!("mesh:{mesh_idx}:batch:{bi}"),
                                title: format!("Mesh #{mesh_idx} ({}) / Batch #{bi}", mesh.name),
                                breadcrumb: format!(
                                    "{model_name} / Meshes / #{mesh_idx} / Batch #{bi}"
                                ),
                                spec_line,
                                validation_msg: format!(
                                    "Material #{} · {} indices · Bone refs: {}{}",
                                    batch.header.effective_material_id(is_b2),
                                    batch.indices.len(),
                                    batch.bone_refs.len(),
                                    if batch.header.is_shadow_proxy(is_b2) {
                                        " · Shadow Proxy (u_e1/u_e2)"
                                    } else {
                                        ""
                                    }
                                ),
                                is_wmb3: false,
                                is_bayo2: is_b2,
                                endian: endian_str,
                                data: serde_json::json!({
                                    "mesh_index": mesh_idx,
                                    "mesh_name": mesh.name,
                                    "batch_index": bi,
                                    "batch": batch,
                                }),
                                material_decoded: None,
                                wmb3_material: None,
                                wmb3_named_params: Vec::new(),
                                wmb3_texture_views: Vec::new(),
                                bank_textures: Vec::new(),
                                core_pretextures: Vec::new(),
                                available_shaders: Vec::new(),
                                material_options: material_options.clone(),
                            };
                        }
                    }
                }
                return WmbSectionView {
                    node_id: node_id.into(),
                    section: format!("mesh:{mesh_idx}"),
                    title: format!("Mesh #{mesh_idx} ({})", mesh.name),
                    breadcrumb: format!("{model_name} / Meshes / #{mesh_idx}"),
                    spec_line,
                    validation_msg: format!(
                        "{} batches in mesh '{}'",
                        mesh.batches.len(),
                        mesh.name
                    ),
                    is_wmb3: false,
                    is_bayo2: is_b2,
                    endian: endian_str,
                    data: serde_json::to_value(mesh).unwrap_or_default(),
                    material_decoded: None,
                    wmb3_material: None,
                    wmb3_named_params: Vec::new(),
                    wmb3_texture_views: Vec::new(),
                    bank_textures: Vec::new(),
                    core_pretextures: Vec::new(),
                    available_shaders: Vec::new(),
                    material_options: material_options.clone(),
                };
            }
        }
    }

    if let Some(bone_idx_str) = section_key.strip_prefix("bone:") {
        if let Ok(bi) = bone_idx_str.parse::<usize>() {
            let dumped = wmb.dump_bones();
            if let Some(b) = dumped.get(bi) {
                return WmbSectionView {
                    node_id: node_id.into(),
                    section: format!("bone:{bi}"),
                    title: format!("Bone #{} (global {})", bi, b.global_index),
                    breadcrumb: format!("{model_name} / Bones / #{bi}"),
                    spec_line,
                    validation_msg: format!(
                        "Local #{bi} · Global {} · Parent {}",
                        b.global_index, b.parent_index
                    ),
                    is_wmb3: false,
                    is_bayo2: is_b2,
                    endian: endian_str,
                    data: serde_json::json!({
                        "index": b.index,
                        "global_id": b.global_index,
                        "parent_index": b.parent_index,
                        "relative_position": [b.relative_position.x, b.relative_position.y, b.relative_position.z],
                        "position": [b.position.x, b.position.y, b.position.z],
                        "symmetric": b.symmetric,
                        "flag": b.flag,
                    }),
                    material_decoded: None,
                    wmb3_material: None,
                    wmb3_named_params: Vec::new(),
                    wmb3_texture_views: Vec::new(),
                    bank_textures: Vec::new(),
                    core_pretextures: Vec::new(),
                    available_shaders: Vec::new(),
                    material_options: material_options.clone(),
                };
            }
        }
    }

    match section_key {
        "bones" => {
            let bones: Vec<serde_json::Value> = (0..wmb.bone_hierarchy.len())
                .map(|i| {
                    let parent = wmb.bone_hierarchy[i];
                    let rel = wmb.bone_relative_positions.get(i).copied().unwrap_or_default();
                    let pos = wmb.bone_positions.get(i).copied().unwrap_or_default();
                    let global_id = wmb
                        .bone_index_translate_table
                        .invert()
                        .get(&(i as u16))
                        .copied()
                        .unwrap_or(i as u16);
                    serde_json::json!({
                        "index": i,
                        "global_id": global_id,
                        "parent_index": parent,
                        "relative_position": [rel.x, rel.y, rel.z],
                        "position": [pos.x, pos.y, pos.z],
                    })
                })
                .collect();
            WmbSectionView {
                node_id: node_id.into(),
                section: "bones".into(),
                title: format!("Skeleton Hierarchy ({} bones)", bones.len()),
                breadcrumb: format!("{model_name} / Bones"),
                spec_line,
                validation_msg: format!(
                    "{} bones · {} global translate entries",
                    bones.len(),
                    wmb.bone_index_translate_table.table.len()
                ),
                is_wmb3: false,
                is_bayo2: is_b2,
                endian: endian_str,
                data: serde_json::json!({
                    "bones": bones,
                    "translate_table": wmb.bone_index_translate_table.table,
                }),
                material_decoded: None,
                wmb3_material: None,
                wmb3_named_params: Vec::new(),
                wmb3_texture_views: Vec::new(),
                bank_textures: Vec::new(),
                core_pretextures: Vec::new(),
                available_shaders: Vec::new(),
                material_options: material_options.clone(),
            }
        }
        "vertexes" => {
            let preview_verts: Vec<_> = wmb.vertexes.iter().take(250).collect();
            WmbSectionView {
                node_id: node_id.into(),
                section: "vertexes".into(),
                title: format!("Vertex Buffer ({} total)", wmb.vertexes.len()),
                breadcrumb: format!("{model_name} / Vertexes"),
                spec_line,
                validation_msg: format!(
                    "Vertex flags 0x{:08X} · showing first {} of {} vertices",
                    wmb.header.vertex_flags,
                    preview_verts.len(),
                    wmb.vertexes.len()
                ),
                is_wmb3: false,
                is_bayo2: is_b2,
                endian: endian_str,
                data: serde_json::to_value(preview_verts).unwrap_or_default(),
                material_decoded: None,
                wmb3_material: None,
                wmb3_named_params: Vec::new(),
                wmb3_texture_views: Vec::new(),
                bank_textures: Vec::new(),
                core_pretextures: Vec::new(),
                available_shaders: Vec::new(),
                material_options: material_options.clone(),
            }
        }
        "materials" => {
            let mats: Vec<DecodedWmbMaterial> = wmb
                .materials
                .iter()
                .enumerate()
                .map(|(i, m)| decode_wmb_material_rich(m, i, is_b2))
                .collect();
            let first_dec = mats.first().cloned();
            let ref_ids: Vec<u32> = first_dec
                .as_ref()
                .map(|d| d.fields.iter().filter_map(|f| f.sampler_tex_id).collect())
                .unwrap_or_default();
            let game_key = if is_b2 { "b2" } else { "b1" };
            let bank_textures =
                collect_model_bank_textures(snap, node_id, &ref_ids, !is_b2, oodle_dll);
            WmbSectionView {
                node_id: node_id.into(),
                section: "materials".into(),
                title: format!("Materials Table ({})", mats.len()),
                breadcrumb: format!("{model_name} / Materials"),
                spec_line,
                validation_msg: format!(
                    "{} material definitions decoded against shader database",
                    mats.len()
                ),
                is_wmb3: false,
                is_bayo2: is_b2,
                endian: endian_str,
                data: serde_json::to_value(&mats).unwrap_or_default(),
                material_decoded: first_dec,
                wmb3_material: None,
                wmb3_named_params: Vec::new(),
                wmb3_texture_views: Vec::new(),
                bank_textures,
                core_pretextures: list_core_pretextures(game_key),
                available_shaders: list_available_shaders(game_key),
                material_options: material_options.clone(),
            }
        }
        "meshes" => WmbSectionView {
            node_id: node_id.into(),
            section: "meshes".into(),
            title: format!("Meshes Table ({})", wmb.meshes.len()),
            breadcrumb: format!("{model_name} / Meshes"),
            spec_line,
            validation_msg: format!("{} meshes parsed", wmb.meshes.len()),
            is_wmb3: false,
            is_bayo2: is_b2,
            endian: endian_str,
            data: serde_json::to_value(&wmb.meshes).unwrap_or_default(),
            material_decoded: None,
            wmb3_material: None,
            wmb3_named_params: Vec::new(),
            wmb3_texture_views: Vec::new(),
            bank_textures: Vec::new(),
            core_pretextures: Vec::new(),
            available_shaders: Vec::new(),
            material_options: material_options.clone(),
        },
        _ => WmbSectionView {
            node_id: node_id.into(),
            section: "header".into(),
            title: format!("WMB Header ({model_name})"),
            breadcrumb: format!("{model_name} / Header"),
            spec_line,
            validation_msg:
                "Offsets managed automatically on save — no manual offset calculation needed."
                    .into(),
            is_wmb3: false,
            is_bayo2: is_b2,
            endian: endian_str,
            data: serde_json::to_value(&wmb.header).unwrap_or_default(),
            material_decoded: None,
            wmb3_material: None,
            wmb3_named_params: Vec::new(),
            wmb3_texture_views: Vec::new(),
            bank_textures: Vec::new(),
            core_pretextures: Vec::new(),
            available_shaders: Vec::new(),
            material_options: material_options.clone(),
        },
    }
}

fn build_wmb3_material_options(
    snap: &DocSnapshot,
    node_id: &str,
    wmb3: &Wmb3File,
    oodle_dll: Option<&Path>,
) -> (Vec<MaterialPickerOption>, Vec<MaterialBankTextureOption>) {
    let (container_parts, _) = split_container_and_leaf(node_id);
    let base_prefix = if container_parts.is_empty() {
        "wmb3".to_string()
    } else {
        format!("{}/wmb3", container_parts.join("/"))
    };
    let mut all_ref_ids = Vec::new();
    for m in &wmb3.materials {
        for (_, tid) in &m.textures {
            if !all_ref_ids.contains(tid) {
                all_ref_ids.push(*tid);
            }
        }
    }
    let bank_textures = collect_model_bank_textures(snap, node_id, &all_ref_ids, false, oodle_dll);
    let mut mat_options = Vec::with_capacity(wmb3.materials.len());
    for (mi, m) in wmb3.materials.iter().enumerate() {
        let first_tid = m
            .textures
            .iter()
            .find(|(sname, _)| sname.contains("Albedo") || sname.contains("Color") || sname == "g_Texture0")
            .or_else(|| m.textures.first())
            .map(|(_, id)| *id);
        let matched_bank = first_tid.and_then(|tid| {
            bank_textures
                .iter()
                .find(|bt| bt.texture_id == tid || bt.index as u32 == tid)
        });
        mat_options.push(MaterialPickerOption {
            index: mi,
            label: format!("Material #{mi} {} ({})", m.name, m.shader_name),
            shader_name: m.shader_name.clone(),
            badge: m.technique_name.clone(),
            node_id: format!("{base_prefix}:mat:{mi}"),
            first_tex_id_hex: first_tid.map(|t| format!("0x{t:08X}")),
            thumbnail_data_url: matched_bank.and_then(|b| b.thumbnail_data_url.clone()),
        });
    }
    (mat_options, bank_textures)
}

pub(crate) fn build_wmb3_section_view(
    snap: &DocSnapshot,
    node_id: &str,
    model_name: &str,
    wmb3: &Wmb3File,
    section_key: &str,
    oodle_dll: Option<&Path>,
) -> WmbSectionView {
    let endian_str = "Little".to_string();
    let needs_material_options = section_key == "materials"
        || section_key.starts_with("mat:")
        || section_key == "meshes"
        || section_key.starts_with("mesh:");
    let material_options = if needs_material_options {
        build_wmb3_material_options(snap, node_id, wmb3, oodle_dll).0
    } else {
        Vec::new()
    };

    let total_verts: usize = wmb3.vertex_groups.iter().map(|vg| vg.vertexes.len()).sum();
    let game_label = if wmb3.version == 0x2020_0824 {
        "WMB3 (Bayonetta 3)"
    } else {
        "WMB3 (NieR:Automata / Astral Chain / MGRR)"
    };
    let spec_line = format!(
        "{game_label} | {endian_str} endian | {total_verts} verts | {} bones | {} materials | {} meshes",
        wmb3.bones.len(),
        wmb3.materials.len(),
        wmb3.meshes.len()
    );

    if let Some(mat_idx_str) = section_key.strip_prefix("mat:") {
        if let Ok(mi) = mat_idx_str.parse::<usize>() {
            if let Some(mat) = wmb3.materials.get(mi) {
                let ref_ids: Vec<u32> = mat.textures.iter().map(|&(_, id)| id).collect();
                let bank_textures =
                    collect_model_bank_textures(snap, node_id, &ref_ids, false, oodle_dll);
                let named_params = mat.named_parameters();
                let tex_views = mat.texture_views();
                return WmbSectionView {
                    node_id: node_id.into(),
                    section: format!("mat:{mi}"),
                    title: format!("WMB3 Material #{mi} ({})", mat.name),
                    breadcrumb: format!("{model_name} / Materials / #{mi}"),
                    spec_line,
                    validation_msg: format!(
                        "Shader {} · Technique {} · {} textures · {} parameters",
                        mat.shader_name,
                        mat.technique_name,
                        mat.textures.len(),
                        named_params.len() + mat.variables.len()
                    ),
                    is_wmb3: true,
                    is_bayo2: false,
                    endian: endian_str,
                    data: serde_json::to_value(mat).unwrap_or_default(),
                    material_decoded: None,
                    wmb3_material: Some(mat.clone()),
                    wmb3_named_params: named_params,
                    wmb3_texture_views: tex_views,
                    bank_textures,
                    core_pretextures: list_core_pretextures("b3"),
                    available_shaders: list_available_shaders("b3"),
                    material_options: material_options.clone(),
                };
            }
        }
    }

    if let Some(mesh_idx_str) = section_key.strip_prefix("mesh:") {
        if let Ok(mi) = mesh_idx_str.parse::<usize>() {
            if let Some(mesh) = wmb3.meshes.get(mi) {
                return WmbSectionView {
                    node_id: node_id.into(),
                    section: format!("mesh:{mi}"),
                    title: format!("WMB3 Mesh #{mi} ({})", mesh.name),
                    breadcrumb: format!("{model_name} / Meshes / #{mi}"),
                    spec_line,
                    validation_msg: format!(
                        "Mesh {} · {} materials · {} bones",
                        mesh.name,
                        mesh.materials.len(),
                        mesh.bone_indices.len()
                    ),
                    is_wmb3: true,
                    is_bayo2: false,
                    endian: endian_str,
                    data: serde_json::json!({
                        "mesh_index": mi,
                        "mesh": mesh,
                        "meshes": wmb3.meshes,
                        "batches": wmb3.batches,
                        "lods": wmb3.lods,
                        "bone_sets": wmb3.bone_sets,
                    }),
                    material_decoded: None,
                    wmb3_material: None,
                    wmb3_named_params: Vec::new(),
                    wmb3_texture_views: Vec::new(),
                    bank_textures: Vec::new(),
                    core_pretextures: Vec::new(),
                    available_shaders: Vec::new(),
                    material_options: material_options.clone(),
                };
            }
        }
    }

    if let Some(lod_rest) = section_key.strip_prefix("lod:") {
        let parts: Vec<&str> = lod_rest.split(':').collect();
        let (mesh_opt, lvl_opt) = if parts.len() == 2 {
            (parts[0].parse::<usize>().ok(), parts[1].parse::<i32>().ok())
        } else {
            (None, parts[0].parse::<i32>().ok())
        };
        if let Some(lvl) = lvl_opt {
            if let Some((li, lod)) = wmb3
                .lods
                .iter()
                .enumerate()
                .find(|(_, l)| l.lod_level == lvl)
            {
                return WmbSectionView {
                    node_id: node_id.into(),
                    section: format!("lod:{lod_rest}"),
                    title: format!("WMB3 LOD {} ({})", lod.lod_level, lod.name),
                    breadcrumb: format!("{model_name} / LOD {}", lod.lod_level),
                    spec_line,
                    validation_msg: format!(
                        "LOD #{li} {} (level {}) · {} batches",
                        lod.name,
                        lod.lod_level,
                        lod.batch_infos.len()
                    ),
                    is_wmb3: true,
                    is_bayo2: false,
                    endian: endian_str,
                    data: serde_json::json!({
                        "lod_index": li,
                        "mesh_index": mesh_opt,
                        "lod": lod,
                        "meshes": wmb3.meshes,
                        "batches": wmb3.batches,
                        "lods": wmb3.lods,
                    }),
                    material_decoded: None,
                    wmb3_material: None,
                    wmb3_named_params: Vec::new(),
                    wmb3_texture_views: Vec::new(),
                    bank_textures: Vec::new(),
                    core_pretextures: Vec::new(),
                    available_shaders: Vec::new(),
                    material_options: material_options.clone(),
                };
            }
        }
    }

    if let Some(batch_idx_str) = section_key.strip_prefix("batch:") {
        if let Ok(bi) = batch_idx_str.parse::<usize>() {
            if let Some(batch) = wmb3.batches.get(bi) {
                let mut batch_info_val = serde_json::Value::Null;
                let mut lod_level = 0i32;
                for lod in &wmb3.lods {
                    let start = lod.batch_start as usize;
                    let end = start + lod.batch_infos.len();
                    if bi >= start && bi < end {
                        batch_info_val =
                            serde_json::to_value(&lod.batch_infos[bi - start]).unwrap_or_default();
                        lod_level = lod.lod_level;
                        break;
                    }
                }
                return WmbSectionView {
                    node_id: node_id.into(),
                    section: format!("batch:{bi}"),
                    title: format!("WMB3 Batch #{bi} (LOD {lod_level})"),
                    breadcrumb: format!("{model_name} / Batches / #{bi}"),
                    spec_line,
                    validation_msg: format!(
                        "VG #{} · {} tris ({} idx)",
                        batch.vertex_group_index, batch.num_primitives, batch.num_indices
                    ),
                    is_wmb3: true,
                    is_bayo2: false,
                    endian: endian_str,
                    data: serde_json::json!({
                        "batch_index": bi,
                        "batch": batch,
                        "batch_info": batch_info_val,
                        "meshes": wmb3.meshes,
                        "batches": wmb3.batches,
                        "lods": wmb3.lods,
                        "bone_sets": wmb3.bone_sets,
                    }),
                    material_decoded: None,
                    wmb3_material: None,
                    wmb3_named_params: Vec::new(),
                    wmb3_texture_views: Vec::new(),
                    bank_textures: Vec::new(),
                    core_pretextures: Vec::new(),
                    available_shaders: Vec::new(),
                    material_options: material_options.clone(),
                };
            }
        }
    }

    if let Some(bone_idx_str) = section_key.strip_prefix("bone:") {
        if let Ok(bi) = bone_idx_str.parse::<usize>() {
            if let Some(b) = wmb3.bones.get(bi) {
                return WmbSectionView {
                    node_id: node_id.into(),
                    section: format!("bone:{bi}"),
                    title: format!("WMB3 Bone #{bi} (global {})", b.id),
                    breadcrumb: format!("{model_name} / Bones / #{bi}"),
                    spec_line,
                    validation_msg: format!("Global ID {} · Parent {}", b.id, b.parent_index),
                    is_wmb3: true,
                    is_bayo2: false,
                    endian: endian_str,
                    data: serde_json::json!({
                        "index": bi,
                        "global_id": b.id,
                        "parent_index": b.parent_index,
                        "local_position": [b.local_position.x, b.local_position.y, b.local_position.z],
                        "local_rotation": [b.local_rotation.x, b.local_rotation.y, b.local_rotation.z],
                        "local_scale": [b.local_scale.x, b.local_scale.y, b.local_scale.z],
                        "position": [b.position.x, b.position.y, b.position.z],
                        "rotation": [b.rotation.x, b.rotation.y, b.rotation.z],
                        "scale": [b.scale.x, b.scale.y, b.scale.z],
                        "t_position": [b.t_position.x, b.t_position.y, b.t_position.z],
                        "bones": wmb3.bones,
                    }),
                    material_decoded: None,
                    wmb3_material: None,
                    wmb3_named_params: Vec::new(),
                    wmb3_texture_views: Vec::new(),
                    bank_textures: Vec::new(),
                    core_pretextures: Vec::new(),
                    available_shaders: Vec::new(),
                    material_options: material_options.clone(),
                };
            }
        }
    }

    match section_key {
        "bones" => WmbSectionView {
            node_id: node_id.into(),
            section: "bones".into(),
            title: format!("WMB3 Skeleton ({} bones)", wmb3.bones.len()),
            breadcrumb: format!("{model_name} / Bones"),
            spec_line,
            validation_msg: format!("{} bones with T-pose transforms", wmb3.bones.len()),
            is_wmb3: true,
            is_bayo2: false,
            endian: endian_str,
            data: serde_json::to_value(&wmb3.bones).unwrap_or_default(),
            material_decoded: None,
            wmb3_material: None,
            wmb3_named_params: Vec::new(),
            wmb3_texture_views: Vec::new(),
            bank_textures: Vec::new(),
            core_pretextures: Vec::new(),
            available_shaders: Vec::new(),
            material_options: material_options.clone(),
        },
        "materials" => {
            let first_mat = wmb3.materials.first().cloned();
            let ref_ids: Vec<u32> = first_mat
                .as_ref()
                .map(|m| m.textures.iter().map(|&(_, id)| id).collect())
                .unwrap_or_default();
            let bank_textures =
                collect_model_bank_textures(snap, node_id, &ref_ids, false, oodle_dll);
            let named_params = first_mat
                .as_ref()
                .map(|m| m.named_parameters())
                .unwrap_or_default();
            let tex_views = first_mat
                .as_ref()
                .map(|m| m.texture_views())
                .unwrap_or_default();
            WmbSectionView {
                node_id: node_id.into(),
                section: "materials".into(),
                title: format!("WMB3 Materials ({})", wmb3.materials.len()),
                breadcrumb: format!("{model_name} / Materials"),
                spec_line,
                validation_msg: format!("{} named shader materials", wmb3.materials.len()),
                is_wmb3: true,
                is_bayo2: false,
                endian: endian_str,
                data: serde_json::to_value(&wmb3.materials).unwrap_or_default(),
                material_decoded: None,
                wmb3_material: first_mat,
                wmb3_named_params: named_params,
                wmb3_texture_views: tex_views,
                bank_textures,
                core_pretextures: list_core_pretextures("b3"),
                available_shaders: list_available_shaders("b3"),
                material_options: material_options.clone(),
            }
        }
        "meshes" => WmbSectionView {
            node_id: node_id.into(),
            section: "meshes".into(),
            title: format!(
                "WMB3 Meshes ({}) & Batches ({})",
                wmb3.meshes.len(),
                wmb3.batches.len()
            ),
            breadcrumb: format!("{model_name} / Meshes"),
            spec_line,
            validation_msg: format!(
                "{} meshes · {} batches · {} LODs",
                wmb3.meshes.len(),
                wmb3.batches.len(),
                wmb3.lods.len()
            ),
            is_wmb3: true,
            is_bayo2: false,
            endian: endian_str,
            data: serde_json::json!({
                "meshes": wmb3.meshes,
                "batches": wmb3.batches,
                "lods": wmb3.lods,
                "bone_sets": wmb3.bone_sets,
            }),
            material_decoded: None,
            wmb3_material: None,
            wmb3_named_params: Vec::new(),
            wmb3_texture_views: Vec::new(),
            bank_textures: Vec::new(),
            core_pretextures: Vec::new(),
            available_shaders: Vec::new(),
            material_options: material_options.clone(),
        },
        _ => WmbSectionView {
            node_id: node_id.into(),
            section: "header".into(),
            title: format!("WMB3 Header ({model_name})"),
            breadcrumb: format!("{model_name} / Header"),
            spec_line,
            validation_msg: format!("Version 0x{:08X} · Flags 0x{:04X}", wmb3.version, wmb3.flags),
            is_wmb3: true,
            is_bayo2: false,
            endian: endian_str,
            data: serde_json::json!({
                "version": format!("0x{:08X}", wmb3.version),
                "u_a": wmb3.u_a,
                "flags": wmb3.flags,
                "u_c": wmb3.u_c,
                "bounding_box": wmb3.bounding_box,
                "vertex_groups_count": wmb3.vertex_groups.len(),
                "batches_count": wmb3.batches.len(),
                "lods_count": wmb3.lods.len(),
            }),
            material_decoded: None,
            wmb3_material: None,
            wmb3_named_params: Vec::new(),
            wmb3_texture_views: Vec::new(),
            bank_textures: Vec::new(),
            core_pretextures: Vec::new(),
            available_shaders: Vec::new(),
            material_options: material_options.clone(),
        },
    }
}

#[tauri::command]
pub fn convert_material_action(
    src_game: String,
    dst_game: String,
    material_payload: serde_json::Value,
    target_shader: Option<String>,
) -> Result<serde_json::Value, String> {
    let wmb_mat: Option<DecodedWmbMaterial> = if src_game == "b1" || src_game == "b2" {
        serde_json::from_value(material_payload.clone()).ok()
    } else {
        None
    };
    let wmb3_mat: Option<Wmb3Material> = if src_game == "b3" {
        serde_json::from_value(material_payload).ok()
    } else {
        None
    };
    let (converted, report) = convert_material_between_games(
        &src_game,
        &dst_game,
        wmb_mat.as_ref(),
        wmb3_mat.as_ref(),
        target_shader.as_deref(),
    )?;
    Ok(serde_json::json!({
        "converted": converted,
        "report": report,
    }))
}

pub(crate) fn validate_bone_parent(
    num_bones: usize,
    bone_idx: usize,
    new_parent: i64,
    get_parent: impl Fn(usize) -> i16,
) -> Result<i16, String> {
    if new_parent < -1 || new_parent >= num_bones as i64 {
        return Err(format!(
            "Parent bone index {new_parent} is out of range (-1..{})",
            num_bones.saturating_sub(1)
        ));
    }
    if new_parent == bone_idx as i64 {
        return Err(format!("Bone #{bone_idx} cannot be its own parent"));
    }
    let mut cur = new_parent;
    let mut steps = 0usize;
    while cur >= 0 {
        if cur as usize == bone_idx {
            return Err(format!(
                "Cannot reparent bone #{bone_idx} to its own descendant #{new_parent} (cycle detected)"
            ));
        }
        if cur as usize >= num_bones || steps > num_bones {
            return Err("Cycle detected in bone hierarchy".into());
        }
        cur = get_parent(cur as usize) as i64;
        steps += 1;
    }
    Ok(new_parent as i16)
}

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
    let mut working_uid_map = orig_uid_map.clone();
    let mut bump_structure = false;
    let mut coalesce_key: Option<String> = None;

    // Full WMB3 mutation & binary re-serialization (`Wmb3File::to_bytes`)!
    if sub_bytes.starts_with(b"WMB3") || sub_bytes.starts_with(b"3BMW") {
        let mut wmb3 = Wmb3File::from_bytes(&sub_bytes).map_err(|e| e.to_string())?;
        let mut init_committed = orig_uid_map;
        init_committed.ensure_wmb3(&wmb3);
        working_uid_map.ensure_wmb3(&wmb3);

        let desc = match operation.as_str() {
            "export_obj" => {
                let out_path = payload
                    .get("path")
                    .and_then(|v| v.as_str())
                    .ok_or("Missing OBJ export path")?;
                fs::write(out_path, wmb3.export_obj()).map_err(|e| e.to_string())?;
                return build_outline_from_session(&sess);
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
                        &snap, &node_id, oodle_opt,
                    ))
                } else {
                    None
                };
                let outcome = apply_wmb3_scene_op(
                    &mut wmb3,
                    &mut working_uid_map,
                    &operation,
                    &payload,
                    clipboard_opt.as_ref(),
                    bank_tex_ids_opt.as_deref(),
                )?;
                if outcome.structural {
                    bump_structure = true;
                }
                outcome.description
            }
            other => return Err(format!("Unsupported WMB3 operation: {other}")),
        };
        sync_wmb3_invariants(&mut wmb3);
        working_uid_map.ensure_wmb3(&wmb3);
        let new_wmb3_bytes = wmb3.to_bytes().map_err(|e| e.to_string())?;
        let _ = Wmb3File::from_bytes(&new_wmb3_bytes)
            .map_err(|e| format!("WMB3 round-trip verification failed: {e}"))?;
        let dat_align = sess.settings.effective_dat_alignment();
        let next_snap =
            write_back_subblob_with_align(&snap, &container_parts, new_wmb3_bytes, None, oodle_opt, dat_align)?;

        sess.committed_uid_maps
            .entry(container_id.clone())
            .or_insert(init_committed);
        sess.uid_maps.insert(container_id, working_uid_map);
        if bump_structure {
            sess.structure_version = sess.structure_version.max(1) + 1;
        }
        record_mutation_with_key(&mut sess, desc, next_snap, coalesce_key);
        return build_outline_from_session(&sess);
    }

    let mut wmb = WmbFile::from_bytes(&sub_bytes).map_err(|e| e.to_string())?;
    let mut target_endian = wmb.endian;
    let is_b2 = wmb.is_bayo2();
    let mut init_committed = orig_uid_map;
    init_committed.ensure_wmb(&wmb);
    working_uid_map.ensure_wmb(&wmb);

    let desc = match operation.as_str() {
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
            return build_outline_from_session(&sess);
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
                    &snap, &node_id, oodle_opt,
                ))
            } else {
                None
            };
            let outcome = apply_wmb_scene_op(
                &mut wmb,
                &mut working_uid_map,
                &operation,
                &payload,
                clipboard_opt.as_ref(),
                bank_tex_ids_opt.as_deref(),
            )?;
            if outcome.structural {
                bump_structure = true;
            }
            outcome.description
        }
        other => return Err(format!("Unknown WMB operation: {other}")),
    };

    enforce_shadow_last(&mut wmb, &mut working_uid_map);
    let new_wmb_bytes = wmb.to_bytes(target_endian).map_err(|e| e.to_string())?;
    let _ = WmbFile::from_bytes(&new_wmb_bytes)
        .map_err(|e| format!("WMB round-trip verification failed: {e}"))?;
    let dat_align = sess.settings.effective_dat_alignment();
    let next_snap = write_back_subblob_with_align(&snap, &container_parts, new_wmb_bytes, None, oodle_opt, dat_align)?;

    sess.committed_uid_maps
        .entry(container_id.clone())
        .or_insert(init_committed);
    sess.uid_maps.insert(container_id, working_uid_map);
    if bump_structure {
        sess.structure_version = sess.structure_version.max(1) + 1;
    }
    record_mutation_with_key(&mut sess, desc, next_snap, coalesce_key);
    build_outline_from_session(&sess)
}
