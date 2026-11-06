//! Tauri IPC commands and helpers for the PGTools Studio 3D WMB/WMB3 Workspace (Phase 1):
//! `get_wmb_scene`, `get_wmb_texture`, `get_pre_texture`, `resolve_legacy_node`,
//! `create_demo_model`, `wmb_clipboard_copy`, and `wmb_clipboard_status`.

use crate::{
    atomic_write_file, detect_file_format_tag, find_paired_wtp_bytes, lock_clipboard, lock_session,
    resolve_subblob, split_container_and_leaf, AppState, DocSnapshot,
};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use pgtools_core::{
    dat::DatFile,
    demo::build_demo_dat,
    scene::{
        build_wmb3_scene, build_wmb_scene, decode_texture_for_wtx1, encode_wtx1_blob,
        fnv1a_64_hex, pack_scene_blob, TextureRefDesc, UidMap,
    },
    scene_ops::{copy_wmb3_to_clipboard, copy_wmb_to_clipboard, ClipboardStatus},
    texture_conv::{decode_bntx_subtexture_mip_filtered, decode_texture_mip_filtered},
    wmb::WmbFile,
    wmb3::Wmb3File,
    wtb::WtbFile,
};
use std::collections::HashMap;
use std::io::Cursor;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::Path;
use std::sync::Arc;
use std::time::UNIX_EPOCH;
use tauri::State;

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
                    let is_bntx = (e.data.len() >= 4 && &e.data[0..4] == b"BNTX")
                        || (e.data.len() >= 84 && &e.data[80..84] == b"BNTX");
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
    // [wmb-studio BEGIN phase2]
    if trimmed == "textures" {
        let banks = collect_paired_wtb_banks(&snap, &node_id, oodle_opt);
        if let Some((bank_id, _)) = banks.first() {
            return Ok(format!("{bank_id}/tex:0"));
        }
        return Err("No paired texture bank found for this model".to_string());
    }
    // [wmb-studio END]

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
