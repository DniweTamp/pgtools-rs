use crate::common::DocSnapshot;
use crate::session::{atomic_write_file, lock_clipboard, lock_session, AppState};
use crate::vfs::{
    detect_file_format_tag, find_paired_wtp_bytes, find_pkz_peer_dat_index,
    find_pkz_peer_dtt_index, is_bntx_blob, resolve_subblob, split_container_and_leaf,
};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use pgtools_core::{
    dat::DatFile,
    demo::build_demo_dat,
    pkz::{OodleLibrary, PkzArchive},
    scene::{
        build_wmb3_scene, build_wmb_scene, extract_texture_for_wtx2, pack_scene_blob,
        TextureRefDesc, UidMap,
    },
    scene_ops::{copy_wmb3_to_clipboard, copy_wmb_to_clipboard, ClipboardStatus},
    shader_db::{
        convert_material_between_games, decode_wmb_material_rich, list_available_shaders,
        list_core_pretextures, CorePretextureInfo, ShaderOptionDesc,
    },
    texture_conv::{decode_bntx_subtexture_mip_filtered, decode_texture_mip_filtered},
    wmb::{DecodedWmbMaterial, WmbFile},
    wmb3::{Wmb3File, Wmb3Material, Wmb3NamedParameter, Wmb3TextureSlotView},
    wtb::WtbFile,
};
use serde::{Deserialize, Serialize};
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

    let candidate_roots = collect_candidate_dat_roots(snap, &container_parts, oodle_dll);

    for (_prefix, dat_bytes, _peer_dtt) in candidate_roots {
        if let Ok(dat) = DatFile::from_bytes(dat_bytes.as_slice()) {
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

/// Resolve the parent DAT/DTT container roots (with optional peer DTT bytes for `.wta`+`.wtp`
/// pairing) for a model node, including `.dat`/`.dtt` sibling pairs stored inside a `.pkz` archive.
fn collect_candidate_dat_roots(
    snap: &DocSnapshot,
    container_parts: &[&str],
    oodle_dll: Option<&Path>,
) -> Vec<(String, Arc<Vec<u8>>, Option<Arc<Vec<u8>>>)> {
    let mut candidate_roots: Vec<(String, Arc<Vec<u8>>, Option<Arc<Vec<u8>>>)> = Vec::new();
    if let DocSnapshot::PairedDatDtt { dat, dtt } = snap {
        candidate_roots.push(("root:dat".to_string(), dat.clone(), Some(dtt.clone())));
        candidate_roots.push(("root:dtt".to_string(), dtt.clone(), None));
        return candidate_roots;
    }
    if container_parts.is_empty() {
        return candidate_roots;
    }

    let parent_parts = &container_parts[..container_parts.len() - 1];
    let parent_id = parent_parts.join("/");

    // If the parent container is an entry inside a PKZ archive (e.g. `pkz:4/dat:0`),
    // also resolve its sibling `.dtt` (or `.dat`) inside the same PKZ so `.wta`+`.wtp`
    // and `.wtb`/`.bntx` banks in the peer archive are loaded automatically.
    if let Some(pkz_idx) = parent_parts
        .last()
        .and_then(|p| p.strip_prefix("pkz:"))
        .and_then(|s| s.parse::<usize>().ok())
    {
        let grandparent_parts = &parent_parts[..parent_parts.len() - 1];
        let grandparent_id = grandparent_parts.join("/");

        let borrowed_pkz: Option<Arc<Vec<u8>>> = if grandparent_id.is_empty() {
            match snap {
                DocSnapshot::SingleFile(b) => Some(b.clone()),
                _ => None,
            }
        } else {
            resolve_subblob(snap, &grandparent_id, oodle_dll)
                .ok()
                .map(|(_, bytes, _, _)| Arc::new(bytes))
        };

        if let Some(pkz_bytes) = borrowed_pkz {
            if let Ok(pkz) = PkzArchive::from_bytes(pkz_bytes.as_slice()) {
                if let Some(desc) = pkz.descriptors.get(pkz_idx) {
                    let oodle_lib = OodleLibrary::load(oodle_dll).ok();
                    if let Ok(parent_bytes) =
                        pkz.extract_entry_with_lib(pkz_bytes.as_slice(), pkz_idx, oodle_lib.as_ref())
                    {
                        let parent_arc = Arc::new(parent_bytes);
                        let make_pkz_node_id = |entry_idx: usize| -> String {
                            if grandparent_id.is_empty() {
                                format!("pkz:{entry_idx}")
                            } else {
                                format!("{grandparent_id}/pkz:{entry_idx}")
                            }
                        };

                        if let Some(dtt_idx) = find_pkz_peer_dtt_index(&pkz, &desc.name) {
                            if let Ok(dtt_bytes) = pkz.extract_entry_with_lib(
                                pkz_bytes.as_slice(),
                                dtt_idx,
                                oodle_lib.as_ref(),
                            ) {
                                let dtt_arc = Arc::new(dtt_bytes);
                                candidate_roots.push((
                                    parent_id,
                                    parent_arc,
                                    Some(dtt_arc.clone()),
                                ));
                                candidate_roots.push((make_pkz_node_id(dtt_idx), dtt_arc, None));
                                return candidate_roots;
                            }
                        } else if let Some(dat_idx) = find_pkz_peer_dat_index(&pkz, &desc.name) {
                            if let Ok(dat_bytes) = pkz.extract_entry_with_lib(
                                pkz_bytes.as_slice(),
                                dat_idx,
                                oodle_lib.as_ref(),
                            ) {
                                let dat_arc = Arc::new(dat_bytes);
                                candidate_roots.push((
                                    make_pkz_node_id(dat_idx),
                                    dat_arc,
                                    Some(parent_arc.clone()),
                                ));
                                candidate_roots.push((parent_id, parent_arc, None));
                                return candidate_roots;
                            }
                        }

                        candidate_roots.push((parent_id, parent_arc, None));
                        return candidate_roots;
                    }
                }
            }
        }
    }

    if let Ok((_, parent_bytes, _, _)) = resolve_subblob(snap, &parent_id, oodle_dll) {
        candidate_roots.push((parent_id, Arc::new(parent_bytes), None));
    }
    candidate_roots
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

    // 2. Model inside DAT / Paired DAT+DTT / PKZ (zero-copy Arc borrows for PairedDatDtt & root PKZ)
    let candidate_roots = collect_candidate_dat_roots(snap, &container_parts, oodle_dll);

    for (prefix, dat_bytes, peer_dtt) in candidate_roots {
        if let Ok(dat) = DatFile::from_bytes(dat_bytes.as_slice()) {
            let parsed_dtt = peer_dtt.as_ref().and_then(|b| DatFile::from_bytes(b.as_slice()).ok());
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

/// Build `TextureRefDesc` metadata entries from already-parsed paired WTB banks with ZERO CPU pixel decoding.
fn build_bank_texture_refs_from_banks(banks: &[(String, Arc<WtbFile>)]) -> Vec<TextureRefDesc> {
    let mut out = Vec::new();
    for (_bank_node_id, wtb) in banks {
        for e in &wtb.entries {
            let tid = e.texture_id.unwrap_or(e.index as u32);
            let (fmt_name, w, h, meta_name) = e
                .metadata
                .as_ref()
                .map(|m| (m.format.display_name(), m.width, m.height, m.name.clone()))
                .unwrap_or_else(|| ("TEX".to_string(), 0, 0, None));
            let name = meta_name.unwrap_or_else(|| format!("tex{}", e.index));
            let content_hash = if !e.content_hash.is_empty() {
                e.content_hash.clone()
            } else {
                format!("{:08x}:{:x}", tid, e.data.len())
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
                thumbnail_data_url: None,
            });
        }
    }
    out
}

#[tauri::command]
pub async fn get_wmb_scene(
    node_id: String,
    known_geometry_hashes: Option<Vec<String>>,
    state: State<'_, AppState>,
) -> Result<tauri::ipc::Response, String> {
    let (snap, doc_name, oodle_path, structure_version, mut uid_map, model_key, cached_banks) = {
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
        let cached_banks = sess.paired_wtb_cache.get(&model_key).cloned();
        (
            snap,
            doc_name,
            oodle_path,
            structure_version,
            uid_map,
            model_key,
            cached_banks,
        )
    };

    let model_key_bg = model_key.clone();
    let node_id_bg = node_id.clone();
    let (wire_bytes, updated_uid_map, parsed_banks) = tauri::async_runtime::spawn_blocking(move || {
        catch_unwind(AssertUnwindSafe(|| -> Result<_, String> {
            let oodle_opt = if oodle_path.is_empty() {
                None
            } else {
                Some(Path::new(oodle_path.as_str()))
            };
            let (sub_name, sub_bytes, _, _) = resolve_subblob(&snap, &model_key_bg, oodle_opt)?;
            let file_name = if sub_name == "root" || sub_name.is_empty() {
                doc_name
            } else {
                sub_name
            };

            let banks_arc: Arc<Vec<(String, Arc<WtbFile>)>> = match cached_banks {
                Some(b) => b,
                None => Arc::new(
                    collect_paired_wtb_banks(&snap, &node_id_bg, oodle_opt)
                        .into_iter()
                        .map(|(k, w)| (k, Arc::new(w)))
                        .collect(),
                ),
            };
            let bank_textures = build_bank_texture_refs_from_banks(&banks_arc);
            let known = known_geometry_hashes.unwrap_or_default();
            let tag = detect_file_format_tag(&file_name, &sub_bytes);

            let (manifest, binary) = if tag == "WMB3" {
                let wmb3 = Wmb3File::from_bytes(&sub_bytes).map_err(|e| e.to_string())?;
                build_wmb3_scene(
                    &wmb3,
                    &mut uid_map,
                    &node_id_bg,
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
                    &node_id_bg,
                    &file_name,
                    structure_version,
                    bank_textures,
                    &known,
                )
            } else {
                return Err(format!(
                    "Node '{node_id_bg}' is not a WMB or WMB3 model (detected {tag})"
                ));
            };

            let wire_bytes = pack_scene_blob(&manifest, &binary).map_err(|e| e.to_string())?;
            Ok((wire_bytes, uid_map, banks_arc))
        }))
        .map_err(|_| "Internal panic while building WMB scene".to_string())?
    })
    .await
    .map_err(|e| format!("WMB scene worker failed: {e}"))??;

    {
        let mut sess = lock_session(&state);
        sess.paired_wtb_cache.insert(model_key.clone(), parsed_banks);
        sess.uid_maps.insert(model_key.clone(), updated_uid_map.clone());
        sess.committed_uid_maps
            .entry(model_key)
            .or_insert(updated_uid_map);
    }

    Ok(tauri::ipc::Response::new(wire_bytes))
}

#[tauri::command]
pub async fn get_wmb_texture(
    node_id: String,
    texture_id: u32,
    max_dim: u32,
    state: State<'_, AppState>,
) -> Result<tauri::ipc::Response, String> {
    let model_key = canonical_model_key(&node_id);
    let (snap, oodle_path, cached_banks) = {
        let sess = lock_session(&state);
        let snap = sess.current.clone().ok_or("No active document loaded")?;
        let oodle_path = sess.settings.oodle_dll_path.clone();
        let cached_banks = sess.paired_wtb_cache.get(&model_key).cloned();
        (snap, oodle_path, cached_banks)
    };

    let (banks_arc, newly_parsed) = if let Some(b) = cached_banks {
        (b, false)
    } else {
        let snap_c = snap.clone();
        let nid_c = node_id.clone();
        let oodle_c = oodle_path.clone();
        let parsed = tauri::async_runtime::spawn_blocking(move || {
            let oodle_opt = if oodle_c.is_empty() {
                None
            } else {
                Some(Path::new(oodle_c.as_str()))
            };
            Arc::new(
                collect_paired_wtb_banks(&snap_c, &nid_c, oodle_opt)
                    .into_iter()
                    .map(|(k, w)| (k, Arc::new(w)))
                    .collect::<Vec<_>>(),
            )
        })
        .await
        .map_err(|e| format!("Bank parse worker failed: {e}"))?;
        (parsed, true)
    };

    if newly_parsed {
        let mut sess = lock_session(&state);
        sess.paired_wtb_cache.insert(model_key.clone(), banks_arc.clone());
    }

    // Locate entry in O(1) without cloning the entire WtbFile
    let mut matched: Option<(Arc<WtbFile>, usize)> = None;
    for (_bank_id, wtb) in banks_arc.iter() {
        if let Some(pos) = wtb
            .entries
            .iter()
            .position(|e| e.texture_id.unwrap_or(e.index as u32) == texture_id)
        {
            matched = Some((wtb.clone(), pos));
            break;
        }
    }
    if matched.is_none() {
        for (_bank_id, wtb) in banks_arc.iter() {
            if (texture_id as usize) < wtb.entries.len() {
                matched = Some((wtb.clone(), texture_id as usize));
                break;
            }
        }
    }

    let (wtb_arc, entry_pos) = matched.ok_or_else(|| {
        format!("Texture 0x{texture_id:08X} not found in paired bank for '{node_id}'")
    })?;

    let cache_key = {
        let e = &wtb_arc.entries[entry_pos];
        let tag = if !e.content_hash.is_empty() {
            e.content_hash.clone()
        } else {
            format!("{:x}", e.data.len())
        };
        (format!("{model_key}:{tag}"), texture_id, max_dim)
    };

    {
        let sess = lock_session(&state);
        if let Some(cached) = sess.wtx1_cache.get(&cache_key) {
            return Ok(tauri::ipc::Response::new(cached.clone()));
        }
    }

    let wtx2 = tauri::async_runtime::spawn_blocking(move || {
        catch_unwind(AssertUnwindSafe(|| -> Result<Vec<u8>, String> {
            let entry = &wtb_arc.entries[entry_pos];
            extract_texture_for_wtx2(&entry.data, entry.index, entry.flags, max_dim)
                .map_err(|e| format!("Failed to extract texture 0x{texture_id:08X}: {e}"))
        }))
        .map_err(|_| format!("Internal panic extracting texture 0x{texture_id:08X}"))?
    })
    .await
    .map_err(|e| format!("Texture worker failed: {e}"))??;

    {
        let mut sess = lock_session(&state);
        sess.insert_wtx1_cached(cache_key, wtx2.clone());
    }
    Ok(tauri::ipc::Response::new(wtx2))
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
        extract_texture_for_wtx2(&entry.data, entry.index, entry.flags, max_dim)
            .map_err(|e| format!("Failed to extract pre_texture slot {slot_idx}: {e}"))
    }))
    .map_err(|_| format!("Internal panic extracting pre_texture slot {slot_idx}"))??;

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
fn collect_model_bank_textures(
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

