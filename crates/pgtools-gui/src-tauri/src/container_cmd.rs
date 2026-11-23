use crate::common::DocumentOutline;
use crate::session::{build_outline_from_session, lock_session, record_mutation, AppState};
use crate::vfs::{
    collect_paths_for_pkz_add, compute_reorder_target, detect_file_format_tag,
    extract_pkz_entries_with_overwrite_check, parse_eff_entry_id,
    resolve_subblob, revert_pkz_subset, split_container_and_leaf, validate_user_fs_path,
    write_back_subblob_with_align,
};
use pgtools_core::{
    bxm::BxmFile,
    dat::DatFile,
    eff::{EffDirectory, EffFile, EffSubFile},
    io::{validate_single_entry_name, Endian},
    pkz::{
        clamp_compression_level, default_compression_level, normalize_pkz_virtual_path,
        OodleLibrary, PkzArchive,
    },
    scr::ScrFile,
    wtb::WtbFile,
};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use tauri::State;

#[tauri::command]
pub fn mutate_container_node(
    node_id: String,
    action: String,
    target_path: String,
    payload: Option<serde_json::Value>,
    state: State<'_, AppState>,
) -> Result<DocumentOutline, String> {
    let mut sess = lock_session(&state);
    let snap = sess.current.clone().ok_or("No active document loaded")?;
    let oodle_settings = sess.settings.clone();
    let oodle_opt = oodle_settings.oodle_path();
    let dat_align = oodle_settings.effective_dat_alignment();
    let payload_val = payload.unwrap_or(serde_json::Value::Null);

    match action.as_str() {
        "export_bxm_xml" => {
            let out = validate_user_fs_path(&target_path, true)?;
            let (_, sub_bytes, _, _) = resolve_subblob(&snap, &node_id, oodle_opt)?;
            let bxm = BxmFile::from_bytes(&sub_bytes).map_err(|e| e.to_string())?;
            fs::write(&out, bxm.to_xml_string()).map_err(|e| e.to_string())?;
            build_outline_from_session(&sess)
        }
        "import_bxm_xml" => {
            let src = validate_user_fs_path(&target_path, false)?;
            let (container_parts, _) = split_container_and_leaf(&node_id);
            let container_id = container_parts.join("/");
            let (_, sub_bytes, _, _) = resolve_subblob(&snap, &container_id, oodle_opt)?;
            let (orig_tag, orig_flags) = BxmFile::from_bytes(&sub_bytes)
                .map(|b| (b.tag, b.flags))
                .unwrap_or_else(|_| ("BXM".to_string(), 0));
            let xml_str = fs::read_to_string(&src).map_err(|e| e.to_string())?;
            let mut bxm = BxmFile::from_xml_str(&xml_str, &orig_tag).map_err(|e| e.to_string())?;
            bxm.flags = orig_flags;
            let new_bytes = bxm.to_bytes(Endian::Big);
            let next_snap = write_back_subblob_with_align(&snap, &container_parts, new_bytes, None, oodle_opt, dat_align)?;
            record_mutation(&mut sess, format!("Import XML into {node_id}"), next_snap);
            build_outline_from_session(&sess)
        }
        "set_pkz_compression" => {
            let (mut container_parts, leaf) = split_container_and_leaf(&node_id);
            let mut single_pkz_idx: Option<usize> = None;
            if let Some(last) = container_parts.last() {
                if let Some(idx_s) = last.strip_prefix("pkz:") {
                    if let Ok(parsed_idx) = idx_s.parse::<usize>() {
                        // Check if this pkz:X is a leaf file inside a parent PKZ rather than a nested PKZ container
                        let candidate_id = container_parts.join("/");
                        if let Ok((nm, b, _, _)) = resolve_subblob(&snap, &candidate_id, oodle_opt) {
                            if detect_file_format_tag(&nm, &b) != "PKZ" {
                                single_pkz_idx = Some(parsed_idx);
                                container_parts.pop();
                            }
                        }
                    }
                }
            }
            let container_id = container_parts.join("/");
            let (_, sub_bytes, _, _) = resolve_subblob(&snap, &container_id, oodle_opt)?;
            let pkz = PkzArchive::from_bytes(&sub_bytes).map_err(|e| e.to_string())?;
            let requested_codec = payload_val
                .get("compression")
                .and_then(|v| v.as_str())
                .unwrap_or(&target_path)
                .trim();
            let requested_level = payload_val
                .get("level")
                .and_then(|v| v.as_i64())
                .map(|v| clamp_compression_level(requested_codec, v as i32));
            let force_comp = payload_val
                .get("force_compression")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let allow_fallback = payload_val
                .get("allow_zstd_fallback")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            if (requested_codec.eq_ignore_ascii_case("OodleKraken")
                || requested_codec.eq_ignore_ascii_case("Oodle"))
                && !allow_fallback
                && !OodleLibrary::load(oodle_opt)
                    .map(|l| l.can_compress())
                    .unwrap_or(false)
            {
                return Err("OODLE_DLL_MISSING: oo2core_*_win64.dll with OodleLZ_Compress is required to compress with OodleKraken.".into());
            }

            let folder_prefix = payload_val
                .get("folder_prefix")
                .and_then(|v| v.as_str())
                .map(|s| s.trim_matches('/').to_string())
                .or_else(|| {
                    leaf.strip_prefix("pkzfolder:")
                        .map(|s| s.trim_matches('/').to_string())
                })
                .filter(|s| !s.is_empty());

            let entry_indices: Option<Vec<usize>> = payload_val
                .get("entry_indices")
                .and_then(|v| serde_json::from_value(v.clone()).ok())
                .or_else(|| single_pkz_idx.map(|i| vec![i]));

            let mut overrides = HashMap::new();
            if let Some(indices) = entry_indices {
                for idx in indices {
                    overrides.insert(idx, requested_codec.to_string());
                }
            } else if let Some(fp) = folder_prefix {
                let prefix_slash = format!("{fp}/");
                for (idx, d) in pkz.descriptors.iter().enumerate() {
                    let norm = d.name.replace('\\', "/");
                    let clean = norm.trim_start_matches('/');
                    if clean.eq_ignore_ascii_case(&fp)
                        || clean
                            .get(..prefix_slash.len())
                            .map(|p| p.eq_ignore_ascii_case(&prefix_slash))
                            .unwrap_or(false)
                    {
                        overrides.insert(idx, requested_codec.to_string());
                    }
                }
            } else {
                for idx in 0..pkz.descriptors.len() {
                    overrides.insert(idx, requested_codec.to_string());
                }
            }
            let count = overrides.len();
            let repacked = pkz
                .repack_with_full_options(
                    &sub_bytes,
                    &HashMap::new(),
                    &[],
                    &[],
                    None,
                    None,
                    Some(&overrides),
                    None,
                    requested_level,
                    force_comp,
                    oodle_opt,
                )
                .map_err(|e| e.to_string())?;
            let next_snap = write_back_subblob_with_align(&snap, &container_parts, repacked, None, oodle_opt, dat_align)?;
            if let Some(lvl) = requested_level {
                sess.settings.pkz_default_compression = requested_codec.to_string();
                sess.settings.pkz_default_level = lvl;
            }
            record_mutation(
                &mut sess,
                format!("Recompress {count} PKZ file(s) to {requested_codec}"),
                next_snap,
            );
            build_outline_from_session(&sess)
        }
        "revert_pkz_entries" => {
            let (mut container_parts, leaf) = split_container_and_leaf(&node_id);
            let mut single_pkz_idx: Option<usize> = None;
            if let Some(last) = container_parts.last() {
                if let Some(idx_s) = last.strip_prefix("pkz:") {
                    if let Ok(parsed_idx) = idx_s.parse::<usize>() {
                        let candidate_id = container_parts.join("/");
                        if let Ok((nm, b, _, _)) = resolve_subblob(&snap, &candidate_id, oodle_opt) {
                            if detect_file_format_tag(&nm, &b) != "PKZ" {
                                single_pkz_idx = Some(parsed_idx);
                                container_parts.pop();
                            }
                        }
                    }
                }
            }
            let container_id = container_parts.join("/");
            let (_, cur_bytes, _, _) = resolve_subblob(&snap, &container_id, oodle_opt)?;
            let base_snap = sess
                .saved_snapshot
                .as_ref()
                .ok_or("No saved baseline snapshot to revert to")?;
            let (_, base_bytes, _, _) = resolve_subblob(base_snap, &container_id, oodle_opt)?;

            let entry_indices: Option<Vec<usize>> = payload_val
                .get("entry_indices")
                .and_then(|v| serde_json::from_value(v.clone()).ok())
                .or_else(|| single_pkz_idx.map(|i| vec![i]));
            let folder_prefix = payload_val
                .get("folder_prefix")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .or_else(|| leaf.strip_prefix("pkzfolder:").map(|s| s.to_string()));

            let (reverted_bytes, count) = revert_pkz_subset(
                &cur_bytes,
                &base_bytes,
                entry_indices.as_deref(),
                folder_prefix.as_deref(),
            )?;
            let next_snap =
                write_back_subblob_with_align(&snap, &container_parts, reverted_bytes, None, oodle_opt, dat_align)?;
            sess.invalidate_texture_caches();
            record_mutation(
                &mut sess,
                format!("Revert {count} staged PKZ change(s)"),
                next_snap,
            );
            build_outline_from_session(&sess)
        }
        "extract_pkz_folder" | "extract_pkz_entries" => {
            let (mut container_parts, leaf) = split_container_and_leaf(&node_id);
            let mut single_pkz_idx: Option<usize> = None;
            if let Some(last) = container_parts.last() {
                if let Some(idx_s) = last.strip_prefix("pkz:") {
                    if let Ok(parsed_idx) = idx_s.parse::<usize>() {
                        let candidate_id = container_parts.join("/");
                        if let Ok((nm, b, _, _)) = resolve_subblob(&snap, &candidate_id, oodle_opt) {
                            if detect_file_format_tag(&nm, &b) != "PKZ" {
                                single_pkz_idx = Some(parsed_idx);
                                container_parts.pop();
                            }
                        }
                    }
                }
            }
            let container_id = container_parts.join("/");
            let (_, sub_bytes, _, _) = resolve_subblob(&snap, &container_id, oodle_opt)?;
            let pkz = PkzArchive::from_bytes(&sub_bytes).map_err(|e| e.to_string())?;
            let out_dir = PathBuf::from(&target_path);
            let overwrite_policy = payload_val.get("overwrite_policy").and_then(|v| v.as_str());

            let indices: Vec<usize> = if action == "extract_pkz_entries" {
                payload_val
                    .get("entry_indices")
                    .and_then(|v| serde_json::from_value::<Vec<usize>>(v.clone()).ok())
                    .or_else(|| single_pkz_idx.map(|i| vec![i]))
                    .unwrap_or_default()
            } else {
                let folder_prefix = payload_val
                    .get("folder_prefix")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
                    .or_else(|| leaf.strip_prefix("pkzfolder:").map(|s| s.to_string()))
                    .ok_or("Missing PKZ folder prefix")?;
                let clean_fp = folder_prefix.trim().trim_matches('/').replace('\\', "/");
                let prefix_slash = format!("{clean_fp}/");
                pkz.descriptors
                    .iter()
                    .enumerate()
                    .filter_map(|(idx, desc)| {
                        let norm = desc.name.replace('\\', "/");
                        let clean = norm.trim_start_matches('/');
                        if clean.eq_ignore_ascii_case(&clean_fp)
                            || clean
                                .get(..prefix_slash.len())
                                .map(|p| p.eq_ignore_ascii_case(&prefix_slash))
                                .unwrap_or(false)
                        {
                            Some(idx)
                        } else {
                            None
                        }
                    })
                    .collect()
            };

            extract_pkz_entries_with_overwrite_check(
                &pkz,
                &sub_bytes,
                &indices,
                &out_dir,
                overwrite_policy,
                oodle_opt,
            )?;
            build_outline_from_session(&sess)
        }
        "remove_pkz_folder" | "rename_pkz_folder" | "move_pkz_entry_to_folder" => {
            let (mut container_parts, leaf) = split_container_and_leaf(&node_id);
            let mut single_pkz_idx: Option<usize> = None;
            if action == "move_pkz_entry_to_folder" {
                if let Some(last) = container_parts.last() {
                    if let Some(idx_s) = last.strip_prefix("pkz:") {
                        single_pkz_idx = idx_s.parse().ok();
                        container_parts.pop();
                    }
                }
            }
            let container_id = container_parts.join("/");
            let (_, sub_bytes, _, _) = resolve_subblob(&snap, &container_id, oodle_opt)?;
            let pkz = PkzArchive::from_bytes(&sub_bytes).map_err(|e| e.to_string())?;
            let folder_prefix = payload_val
                .get("folder_prefix")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .or_else(|| leaf.strip_prefix("pkzfolder:").map(|s| s.to_string()))
                .unwrap_or_default();
            let clean_fp = folder_prefix.trim().trim_matches('/').replace('\\', "/");
            let prefix_slash = format!("{clean_fp}/");
            let collision_policy = payload_val.get("collision_policy").and_then(|v| v.as_str());

            let (repacked, desc) = if action == "remove_pkz_folder" {
                let mut removed = Vec::new();
                for (idx, d) in pkz.descriptors.iter().enumerate() {
                    let norm = d.name.replace('\\', "/");
                    let clean = norm.trim_start_matches('/');
                    if clean.eq_ignore_ascii_case(&clean_fp)
                        || clean
                            .get(..prefix_slash.len())
                            .map(|p| p.eq_ignore_ascii_case(&prefix_slash))
                            .unwrap_or(false)
                    {
                        removed.push(idx);
                    }
                }
                let r = pkz
                    .repack_with_options(
                        &sub_bytes,
                        &HashMap::new(),
                        &[],
                        &removed,
                        None,
                        None,
                        None,
                        None,
                        oodle_opt,
                    )
                    .map_err(|e| e.to_string())?;
                (
                    r,
                    format!("Remove PKZ folder '{clean_fp}' ({} files)", removed.len()),
                )
            } else if action == "rename_pkz_folder" {
                let raw_new_prefix = payload_val
                    .get("new_prefix")
                    .and_then(|v| v.as_str())
                    .unwrap_or(&target_path)
                    .trim()
                    .trim_matches('/');
                let new_prefix = if raw_new_prefix.is_empty() {
                    String::new()
                } else {
                    normalize_pkz_virtual_path(raw_new_prefix).map_err(|e| e.to_string())?
                };
                let mut renamed = HashMap::new();
                for (idx, d) in pkz.descriptors.iter().enumerate() {
                    let norm = d.name.replace('\\', "/");
                    let clean = norm.trim_start_matches('/');
                    if clean
                        .get(..prefix_slash.len())
                        .map(|p| p.eq_ignore_ascii_case(&prefix_slash))
                        .unwrap_or(false)
                    {
                        let rest = &clean[prefix_slash.len()..];
                        let updated = if new_prefix.is_empty() {
                            rest.to_string()
                        } else {
                            format!("{new_prefix}/{rest}")
                        };
                        renamed.insert(idx, updated);
                    }
                }

                // Check for collisions outside the renamed folder
                let mut colliding_indices: Vec<usize> = Vec::new();
                let mut colliding_names: Vec<String> = Vec::new();
                for (&src_idx, target_name) in &renamed {
                    for (other_idx, other_desc) in pkz.descriptors.iter().enumerate() {
                        if other_idx != src_idx
                            && !renamed.contains_key(&other_idx)
                            && other_desc.name.eq_ignore_ascii_case(target_name)
                        {
                            colliding_indices.push(other_idx);
                            colliding_names.push(target_name.to_string());
                        }
                    }
                }
                if !colliding_names.is_empty() && collision_policy.is_none() {
                    return Err(format!(
                        "PKZ_COLLISION:{}",
                        serde_json::json!({
                            "count": colliding_names.len(),
                            "paths": colliding_names,
                            "new_count": renamed.len().saturating_sub(colliding_names.len())
                        })
                    ));
                }
                if matches!(collision_policy, Some("skip")) {
                    for name in &colliding_names {
                        renamed.retain(|_, v| !v.eq_ignore_ascii_case(name));
                    }
                    colliding_indices.clear();
                }

                let r = pkz
                    .repack_with_options(
                        &sub_bytes,
                        &HashMap::new(),
                        &[],
                        &colliding_indices,
                        None,
                        None,
                        None,
                        Some(&renamed),
                        oodle_opt,
                    )
                    .map_err(|e| e.to_string())?;
                (
                    r,
                    format!(
                        "Rename PKZ folder '{clean_fp}' -> '{new_prefix}' ({} files)",
                        renamed.len()
                    ),
                )
            } else {
                let move_indices: Vec<usize> = payload_val
                    .get("entry_indices")
                    .and_then(|v| serde_json::from_value::<Vec<usize>>(v.clone()).ok())
                    .or_else(|| {
                        single_pkz_idx
                            .or_else(|| {
                                payload_val
                                    .get("entry_index")
                                    .and_then(|v| v.as_u64())
                                    .map(|v| v as usize)
                            })
                            .map(|i| vec![i])
                    })
                    .ok_or("Missing PKZ entry index to move")?;

                let raw_target_folder = payload_val
                    .get("target_folder")
                    .and_then(|v| v.as_str())
                    .unwrap_or(&target_path)
                    .trim()
                    .trim_matches('/');
                let target_folder = if raw_target_folder.is_empty() {
                    String::new()
                } else {
                    normalize_pkz_virtual_path(raw_target_folder).map_err(|e| e.to_string())?
                };

                let mut renamed = HashMap::new();
                let mut colliding_indices = Vec::new();
                let mut colliding_names = Vec::new();

                for &idx in &move_indices {
                    let old_name = pkz
                        .descriptors
                        .get(idx)
                        .ok_or("PKZ index out of bounds")?
                        .name
                        .clone();
                    let base_name = old_name
                        .replace('\\', "/")
                        .split('/')
                        .next_back()
                        .unwrap_or(&old_name)
                        .to_string();
                    let new_name = if target_folder.is_empty() {
                        base_name
                    } else {
                        format!("{target_folder}/{base_name}")
                    };
                    if new_name.eq_ignore_ascii_case(&old_name) {
                        continue;
                    }
                    for (other_idx, other_desc) in pkz.descriptors.iter().enumerate() {
                        if other_idx != idx
                            && !move_indices.contains(&other_idx)
                            && other_desc.name.eq_ignore_ascii_case(&new_name)
                        {
                            colliding_indices.push(other_idx);
                            colliding_names.push(new_name.clone());
                        }
                    }
                    renamed.insert(idx, new_name);
                }

                if !colliding_names.is_empty() && collision_policy.is_none() {
                    return Err(format!(
                        "PKZ_COLLISION:{}",
                        serde_json::json!({
                            "count": colliding_names.len(),
                            "paths": colliding_names,
                            "new_count": renamed.len().saturating_sub(colliding_names.len())
                        })
                    ));
                }
                if matches!(collision_policy, Some("skip")) {
                    for name in &colliding_names {
                        renamed.retain(|_, v| !v.eq_ignore_ascii_case(name));
                    }
                    colliding_indices.clear();
                }

                let r = pkz
                    .repack_with_options(
                        &sub_bytes,
                        &HashMap::new(),
                        &[],
                        &colliding_indices,
                        None,
                        None,
                        None,
                        Some(&renamed),
                        oodle_opt,
                    )
                    .map_err(|e| e.to_string())?;
                (
                    r,
                    format!(
                        "Move {} PKZ file(s) to '{}'",
                        renamed.len(),
                        if target_folder.is_empty() {
                            "(root)"
                        } else {
                            &target_folder
                        }
                    ),
                )
            };
            let next_snap = write_back_subblob_with_align(&snap, &container_parts, repacked, None, oodle_opt, dat_align)?;
            sess.invalidate_texture_caches();
            record_mutation(&mut sess, desc, next_snap);
            build_outline_from_session(&sess)
        }
        "export_subfile" => {
            let (_, sub_bytes, _, _) = resolve_subblob(&snap, &node_id, oodle_opt)?;
            fs::write(&target_path, sub_bytes).map_err(|e| e.to_string())?;
            build_outline_from_session(&sess)
        }
        "replace_subfile" => {
            let src_path = validate_user_fs_path(&target_path, false)?;
            let (mut container_parts, _) = split_container_and_leaf(&node_id);
            let new_bytes = fs::read(&src_path).map_err(|e| e.to_string())?;

            // If replacing an entry directly inside a PKZ with explicit compression/level options:
            if let Some(last) = container_parts.last() {
                if let Some(pkz_idx_s) = last.strip_prefix("pkz:") {
                    if let Ok(entry_idx) = pkz_idx_s.parse::<usize>() {
                        let parent_parts = &container_parts[..container_parts.len() - 1];
                        let parent_id = parent_parts.join("/");
                        let (_, parent_bytes, _, _) =
                            resolve_subblob(&snap, &parent_id, oodle_opt)?;
                        let pkz = PkzArchive::from_bytes(&parent_bytes).map_err(|e| e.to_string())?;
                        let requested_codec = payload_val
                            .get("compression")
                            .and_then(|v| v.as_str())
                            .map(|s| s.trim().to_string());
                        let codec_for_clamp = requested_codec
                            .as_deref()
                            .or_else(|| pkz.descriptors.get(entry_idx).map(|d| d.compression.as_str()))
                            .unwrap_or("ZStandard");
                        let requested_level = payload_val
                            .get("level")
                            .and_then(|v| v.as_i64())
                            .map(|v| clamp_compression_level(codec_for_clamp, v as i32));
                        let force_comp = payload_val
                            .get("force_compression")
                            .and_then(|v| v.as_bool())
                            .unwrap_or(false);
                        let allow_fallback = payload_val
                            .get("allow_zstd_fallback")
                            .and_then(|v| v.as_bool())
                            .unwrap_or(false);

                        if let Some(ref codec) = requested_codec {
                            if (codec.eq_ignore_ascii_case("OodleKraken")
                                || codec.eq_ignore_ascii_case("Oodle"))
                                && !allow_fallback
                                && !OodleLibrary::load(oodle_opt)
                                    .map(|l| l.can_compress())
                                    .unwrap_or(false)
                            {
                                return Err("OODLE_DLL_MISSING: oo2core_*_win64.dll with OodleLZ_Compress is required to compress with OodleKraken.".into());
                            }
                        }

                        let mut replacements = HashMap::new();
                        replacements.insert(entry_idx, new_bytes);
                        let mut overrides = HashMap::new();
                        if let Some(ref codec) = requested_codec {
                            overrides.insert(entry_idx, codec.clone());
                            sess.settings.pkz_default_compression = codec.clone();
                        }
                        if let Some(lvl) = requested_level {
                            sess.settings.pkz_default_level = lvl;
                        }
                        let entry_name = pkz
                            .descriptors
                            .get(entry_idx)
                            .map(|d| d.name.clone())
                            .unwrap_or_else(|| format!("pkz:{entry_idx}"));
                        let repacked = pkz
                            .repack_with_full_options(
                                &parent_bytes,
                                &replacements,
                                &[],
                                &[],
                                None,
                                None,
                                if overrides.is_empty() {
                                    None
                                } else {
                                    Some(&overrides)
                                },
                                None,
                                requested_level,
                                force_comp,
                                oodle_opt,
                            )
                            .map_err(|e| e.to_string())?;
                        container_parts.pop();
                        let next_snap = write_back_subblob_with_align(
                            &snap,
                            &container_parts,
                            repacked,
                            None,
                            oodle_opt,
                            dat_align,
                        )?;
                        sess.invalidate_texture_caches();
                        record_mutation(&mut sess, format!("Replace {entry_name}"), next_snap);
                        return build_outline_from_session(&sess);
                    }
                }
            }

            let next_snap =
                write_back_subblob_with_align(&snap, &container_parts, new_bytes, None, oodle_opt, dat_align)?;
            sess.invalidate_texture_caches();
            record_mutation(&mut sess, format!("Replace {node_id}"), next_snap);
            build_outline_from_session(&sess)
        }
        "extract_all" => {
            let (sub_name, sub_bytes, sub_wtp, _) = resolve_subblob(&snap, &node_id, oodle_opt)?;
            let out_dir = PathBuf::from(&target_path);
            fs::create_dir_all(&out_dir).map_err(|e| e.to_string())?;
            let tag = detect_file_format_tag(&sub_name, &sub_bytes);
            match tag {
                "PKZ" => {
                    let pkz = PkzArchive::from_bytes(&sub_bytes).map_err(|e| e.to_string())?;
                    let all_indices: Vec<usize> = (0..pkz.descriptors.len()).collect();
                    let overwrite_policy =
                        payload_val.get("overwrite_policy").and_then(|v| v.as_str());
                    extract_pkz_entries_with_overwrite_check(
                        &pkz,
                        &sub_bytes,
                        &all_indices,
                        &out_dir,
                        overwrite_policy,
                        oodle_opt,
                    )?;
                }
                "EFF" => {
                    let eff = EffFile::from_bytes(&sub_bytes).map_err(|e| e.to_string())?;
                    eff.extract_to_dir(&out_dir, ".eff")
                        .map_err(|e| e.to_string())?;
                }
                "SCR" => {
                    let scr = ScrFile::from_bytes(&sub_bytes).map_err(|e| e.to_string())?;
                    scr.extract_to_dir(&out_dir).map_err(|e| e.to_string())?;
                }
                "WTB" | "BNTX" => {
                    let wtb = WtbFile::from_bytes(&sub_bytes, sub_wtp.as_deref())
                        .map_err(|e| e.to_string())?;
                    wtb.extract_to_dir(&out_dir, "textures", ".wtb")
                        .map_err(|e| e.to_string())?;
                }
                _ => {
                    let dat = DatFile::from_bytes(&sub_bytes).map_err(|e| e.to_string())?;
                    dat.extract_to_dir(&out_dir, ".dat")
                        .map_err(|e| e.to_string())?;
                }
            }
            build_outline_from_session(&sess)
        }
        "add_files" => {
            let paths: Vec<String> = payload_val
                .get("paths")
                .and_then(|v| serde_json::from_value(v.clone()).ok())
                .unwrap_or_else(|| {
                    if !target_path.is_empty() {
                        vec![target_path.clone()]
                    } else {
                        Vec::new()
                    }
                });
            if paths.is_empty() {
                return Err("No files specified to add".into());
            }
            let (mut container_parts, leaf) = split_container_and_leaf(&node_id);
            let target_effdir: Option<usize> = leaf
                .strip_prefix("effdir:")
                .and_then(|s| s.parse().ok())
                .or_else(|| {
                    container_parts.last().and_then(|last| {
                        last.strip_prefix("efffile:").and_then(|_| {
                            container_parts
                                .get(container_parts.len().saturating_sub(2))
                                .and_then(|d| d.strip_prefix("effdir:"))
                                .and_then(|s| s.parse().ok())
                        })
                    })
                });
            if let Some(last) = container_parts.last() {
                if (last.starts_with("dat:") || last.starts_with("pkz:"))
                    && !node_id.ends_with("root:dat")
                    && !node_id.ends_with("root:dtt")
                {
                    let cur_id = container_parts.join("/");
                    if let Ok((nm, b, _, _)) = resolve_subblob(&snap, &cur_id, oodle_opt) {
                        let t = detect_file_format_tag(&nm, &b);
                        if !matches!(t, "DAT" | "PKZ" | "EFF") {
                            container_parts.pop();
                        }
                    }
                } else if last.starts_with("efffile:") {
                    container_parts.pop();
                    container_parts.pop();
                }
            }

            let container_id = container_parts.join("/");
            let (sub_name, sub_bytes, _, _) = resolve_subblob(&snap, &container_id, oodle_opt)?;
            let tag = detect_file_format_tag(&sub_name, &sub_bytes);

            let (new_container_bytes, desc) = match tag {
                "DAT" => {
                    let mut dat = DatFile::from_bytes(&sub_bytes).map_err(|e| e.to_string())?;
                    for p_str in &paths {
                        let p = Path::new(p_str);
                        let fname = p
                            .file_name()
                            .and_then(|s| s.to_str())
                            .unwrap_or("new_file.bin")
                            .to_string();
                        let data = fs::read(p).map_err(|e| e.to_string())?;
                        dat.push(fname, data).map_err(|e| e.to_string())?;
                    }
                    (
                        dat.to_bytes_with_alignment(dat_align),
                        format!("Add {} file(s) to DAT", paths.len()),
                    )
                }
                "PKZ" => {
                    let pkz = PkzArchive::from_bytes(&sub_bytes).map_err(|e| e.to_string())?;
                    let folder_prefix = payload_val
                        .get("folder_prefix")
                        .and_then(|v| v.as_str())
                        .map(|s| s.trim_matches('/').to_string())
                        .or_else(|| {
                            leaf.strip_prefix("pkzfolder:")
                                .map(|s| s.trim_matches('/').to_string())
                        })
                        .unwrap_or_default();
                    let requested_codec = payload_val
                        .get("compression")
                        .and_then(|v| v.as_str())
                        .map(|s| s.trim().to_string())
                        .unwrap_or_else(|| oodle_settings.pkz_default_compression.clone());
                    let requested_level = payload_val
                        .get("level")
                        .and_then(|v| v.as_i64())
                        .map(|v| clamp_compression_level(&requested_codec, v as i32))
                        .unwrap_or_else(|| {
                            if requested_codec
                                .eq_ignore_ascii_case(&oodle_settings.pkz_default_compression)
                            {
                                clamp_compression_level(
                                    &requested_codec,
                                    oodle_settings.pkz_default_level,
                                )
                            } else {
                                default_compression_level(&requested_codec)
                            }
                        });
                    let force_comp = payload_val
                        .get("force_compression")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false);
                    let allow_fallback = payload_val
                        .get("allow_zstd_fallback")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false);
                    let collision_policy =
                        payload_val.get("collision_policy").and_then(|v| v.as_str());

                    if (requested_codec.eq_ignore_ascii_case("OodleKraken")
                        || requested_codec.eq_ignore_ascii_case("Oodle"))
                        && !allow_fallback
                        && !OodleLibrary::load(oodle_opt)
                            .map(|l| l.can_compress())
                            .unwrap_or(false)
                    {
                        return Err("OODLE_DLL_MISSING: oo2core_*_win64.dll with OodleLZ_Compress is required to compress with OodleKraken.".into());
                    }

                    let raw_candidate_items = collect_paths_for_pkz_add(&paths, &folder_prefix)?;
                    // Deduplicate intra-batch items by lowercase virtual path (last item wins)
                    let mut dedup_pos: HashMap<String, usize> =
                        HashMap::with_capacity(raw_candidate_items.len());
                    let mut candidate_items: Vec<(String, Vec<u8>)> =
                        Vec::with_capacity(raw_candidate_items.len());
                    for (vpath, data) in raw_candidate_items {
                        let key = vpath.to_ascii_lowercase();
                        if let Some(&pos) = dedup_pos.get(&key) {
                            candidate_items[pos] = (vpath, data);
                        } else {
                            dedup_pos.insert(key, candidate_items.len());
                            candidate_items.push((vpath, data));
                        }
                    }

                    let mut existing_by_low: HashMap<String, usize> =
                        HashMap::with_capacity(pkz.descriptors.len());
                    for (idx, d) in pkz.descriptors.iter().enumerate() {
                        existing_by_low.insert(
                            d.name
                                .replace('\\', "/")
                                .trim_start_matches('/')
                                .to_ascii_lowercase(),
                            idx,
                        );
                    }

                    let mut colliding_paths: Vec<String> = Vec::new();
                    let mut new_count = 0usize;
                    for (vpath, _) in &candidate_items {
                        if existing_by_low.contains_key(&vpath.to_ascii_lowercase()) {
                            colliding_paths.push(vpath.clone());
                        } else {
                            new_count += 1;
                        }
                    }

                    if !colliding_paths.is_empty() && collision_policy.is_none() {
                        return Err(format!(
                            "PKZ_COLLISION:{}",
                            serde_json::json!({
                                "count": colliding_paths.len(),
                                "paths": colliding_paths,
                                "new_count": new_count
                            })
                        ));
                    }

                    let skip_colliding = matches!(collision_policy, Some("skip"));
                    let mut replacements: HashMap<usize, Vec<u8>> = HashMap::new();
                    let mut comp_overrides: HashMap<usize, String> = HashMap::new();
                    let mut renamed_entries: HashMap<usize, String> = HashMap::new();
                    let mut added: Vec<(String, Vec<u8>)> = Vec::new();

                    for (vpath, data) in candidate_items {
                        if let Some(&existing_idx) =
                            existing_by_low.get(&vpath.to_ascii_lowercase())
                        {
                            if skip_colliding {
                                continue;
                            }
                            replacements.insert(existing_idx, data);
                            comp_overrides.insert(existing_idx, requested_codec.clone());
                            renamed_entries.insert(existing_idx, vpath);
                        } else {
                            added.push((vpath, data));
                        }
                    }

                    let total_staged = replacements.len() + added.len();
                    sess.settings.pkz_default_compression = requested_codec.clone();
                    sess.settings.pkz_default_level = requested_level;

                    let repacked = pkz
                        .repack_with_full_options(
                            &sub_bytes,
                            &replacements,
                            &added,
                            &[],
                            None,
                            Some(&requested_codec),
                            if comp_overrides.is_empty() {
                                None
                            } else {
                                Some(&comp_overrides)
                            },
                            if renamed_entries.is_empty() {
                                None
                            } else {
                                Some(&renamed_entries)
                            },
                            Some(requested_level),
                            force_comp,
                            oodle_opt,
                        )
                        .map_err(|e| e.to_string())?;
                    (
                        repacked,
                        format!("Add {total_staged} file(s) to PKZ ({requested_codec})"),
                    )
                }
                "EFF" => {
                    let mut eff = EffFile::from_bytes(&sub_bytes).map_err(|e| e.to_string())?;
                    let d_idx = target_effdir.unwrap_or(0);
                    if eff.directories.is_empty() {
                        eff.directories.push(EffDirectory {
                            dir_id: 0,
                            name: "EST".to_string(),
                            files: Vec::new(),
                        });
                    }
                    let dir = eff
                        .directories
                        .get_mut(d_idx)
                        .ok_or("EFF directory out of range")?;
                    for p_str in &paths {
                        let p = Path::new(p_str);
                        let fname = p
                            .file_name()
                            .and_then(|s| s.to_str())
                            .unwrap_or("0000000000.est")
                            .to_string();
                        let id = parse_eff_entry_id(&fname);
                        let data = fs::read(p).map_err(|e| e.to_string())?;
                        dir.files.push(EffSubFile {
                            id,
                            file_name: fname,
                            data,
                        });
                    }
                    (
                        eff.to_bytes().map_err(|e| e.to_string())?,
                        format!("Add {} file(s) to EFF", paths.len()),
                    )
                }
                other => return Err(format!("Cannot add files to container of type {other}")),
            };

            let next_snap = write_back_subblob_with_align(
                &snap,
                &container_parts,
                new_container_bytes,
                None,
                oodle_opt,
                dat_align,
            )?;
            sess.invalidate_texture_caches();
            record_mutation(&mut sess, desc, next_snap);
            build_outline_from_session(&sess)
        }
        "remove_pkz_entries" => {
            let (container_parts, _) = split_container_and_leaf(&node_id);
            let container_id = container_parts.join("/");
            let (_, parent_bytes, _, _) = resolve_subblob(&snap, &container_id, oodle_opt)?;
            let pkz = PkzArchive::from_bytes(&parent_bytes).map_err(|e| e.to_string())?;
            let indices: Vec<usize> = payload_val
                .get("entry_indices")
                .and_then(|v| serde_json::from_value(v.clone()).ok())
                .ok_or("Missing entry_indices for remove_pkz_entries")?;
            let repacked = pkz
                .repack_with_replacements(
                    &parent_bytes,
                    &HashMap::new(),
                    &[],
                    &indices,
                    None,
                    None,
                    oodle_opt,
                )
                .map_err(|e| e.to_string())?;
            let next_snap =
                write_back_subblob_with_align(&snap, &container_parts, repacked, None, oodle_opt, dat_align)?;
            sess.invalidate_texture_caches();
            record_mutation(
                &mut sess,
                format!("Delete {} file(s) from PKZ", indices.len()),
                next_snap,
            );
            build_outline_from_session(&sess)
        }
        "remove_entry" | "rename_entry" | "reorder_entry" => {
            let (mut container_parts, _) = split_container_and_leaf(&node_id);
            if container_parts.is_empty() {
                return Err("Cannot remove or reorder the root container itself".into());
            }
            let last_seg = container_parts
                .pop()
                .ok_or("Cannot remove or reorder the root container itself")?;
            let (eff_dir_idx, eff_file_idx) =
                if let Some(f_str) = last_seg.strip_prefix("efffile:") {
                    let fi: usize = f_str.parse().map_err(|_| "Invalid efffile index")?;
                    let dir_seg = container_parts.pop().ok_or("Missing parent effdir")?;
                    let di: usize = dir_seg
                        .strip_prefix("effdir:")
                        .and_then(|s| s.parse().ok())
                        .ok_or("Invalid effdir index")?;
                    (Some(di), Some(fi))
                } else {
                    (None, None)
                };

            let parent_id = container_parts.join("/");
            let (_, parent_bytes, _, _) = resolve_subblob(&snap, &parent_id, oodle_opt)?;

            let (new_parent_bytes, desc) =
                if let (Some(di), Some(fi)) = (eff_dir_idx, eff_file_idx) {
                    let mut eff = EffFile::from_bytes(&parent_bytes).map_err(|e| e.to_string())?;
                    let dir = eff.directories.get_mut(di).ok_or("EFF dir out of bounds")?;
                    match action.as_str() {
                        "remove_entry" => {
                            let removed = dir.files.remove(fi);
                            (
                                eff.to_bytes().map_err(|e| e.to_string())?,
                                format!("Remove {} from EFF", removed.file_name),
                            )
                        }
                        "rename_entry" => {
                            let new_name = payload_val
                                .get("new_name")
                                .and_then(|v| v.as_str())
                                .unwrap_or(&target_path)
                                .trim()
                                .to_string();
                            let stem = new_name.split('.').next().unwrap_or("");
                            if let Some(parsed_id) = stem
                                .strip_prefix("0x")
                                .or_else(|| stem.strip_prefix("0X"))
                                .and_then(|h| u32::from_str_radix(h, 16).ok())
                                .or_else(|| stem.parse::<u32>().ok())
                                .or_else(|| u32::from_str_radix(stem, 16).ok())
                            {
                                dir.files[fi].id = parsed_id;
                            }
                            dir.files[fi].file_name = new_name.clone();
                            (
                                eff.to_bytes().map_err(|e| e.to_string())?,
                                format!("Rename EFF entry to {new_name}"),
                            )
                        }
                        "reorder_entry" => {
                            let to_idx = compute_reorder_target(fi, dir.files.len(), &payload_val)?;
                            let item = dir.files.remove(fi);
                            dir.files.insert(to_idx, item);
                            (
                                eff.to_bytes().map_err(|e| e.to_string())?,
                                format!("Reorder EFF entry #{fi} -> #{to_idx}"),
                            )
                        }
                        _ => unreachable!(),
                    }
                } else if let Some(dat_idx_str) = last_seg.strip_prefix("dat:") {
                    let idx: usize = dat_idx_str.parse().map_err(|_| "Invalid dat index")?;
                    let mut dat = DatFile::from_bytes(&parent_bytes).map_err(|e| e.to_string())?;
                    if idx >= dat.entries.len() {
                        return Err("DAT entry index out of range".into());
                    }
                    match action.as_str() {
                        "remove_entry" => {
                            let removed = dat.entries.remove(idx);
                            (
                                dat.to_bytes_with_alignment(dat_align),
                                format!("Remove {} from DAT", removed.name),
                            )
                        }
                        "rename_entry" => {
                            let new_name = payload_val
                                .get("new_name")
                                .and_then(|v| v.as_str())
                                .unwrap_or(&target_path)
                                .trim()
                                .to_string();
                            let ext = Path::new(&new_name)
                                .extension()
                                .and_then(|e| e.to_str())
                                .unwrap_or("")
                                .to_string();
                            dat.entries[idx].name = new_name.clone();
                            dat.entries[idx].extension = ext;
                            (
                                dat.to_bytes_with_alignment(dat_align),
                                format!("Rename DAT entry to {new_name}"),
                            )
                        }
                        "reorder_entry" => {
                            let to_idx =
                                compute_reorder_target(idx, dat.entries.len(), &payload_val)?;
                            let item = dat.entries.remove(idx);
                            dat.entries.insert(to_idx, item);
                            (
                                dat.to_bytes_with_alignment(dat_align),
                                format!("Reorder DAT entry #{idx} -> #{to_idx}"),
                            )
                        }
                        _ => unreachable!(),
                    }
                } else if let Some(pkz_idx_str) = last_seg.strip_prefix("pkz:") {
                    let idx: usize = pkz_idx_str.parse().map_err(|_| "Invalid pkz index")?;
                    let pkz = PkzArchive::from_bytes(&parent_bytes).map_err(|e| e.to_string())?;
                    if idx >= pkz.descriptors.len() {
                        return Err("PKZ entry index out of range".into());
                    }
                    match action.as_str() {
                        "remove_entry" => {
                            let name = pkz.descriptors[idx].name.clone();
                            let repacked = pkz
                                .repack_with_replacements(
                                    &parent_bytes,
                                    &HashMap::new(),
                                    &[],
                                    &[idx],
                                    None,
                                    None,
                                    oodle_opt,
                                )
                                .map_err(|e| e.to_string())?;
                            (repacked, format!("Remove {name} from PKZ"))
                        }
                        "rename_entry" => {
                            let raw_input = payload_val
                                .get("new_name")
                                .and_then(|v| v.as_str())
                                .unwrap_or(&target_path)
                                .trim();
                            let old_path = pkz.descriptors[idx].name.replace('\\', "/");
                            let new_full_path =
                                if !raw_input.contains('/') && !raw_input.contains('\\') {
                                    // Leaf-only rename: preserve parent directory segment (matching PKZ Explorer F2 rename)
                                    let leaf_valid = validate_single_entry_name(raw_input)
                                        .map_err(|e| e.to_string())?;
                                    if let Some(slash_pos) = old_path.rfind('/') {
                                        let dir_part = &old_path[..slash_pos];
                                        if dir_part.is_empty() {
                                            leaf_valid
                                        } else {
                                            format!("{dir_part}/{leaf_valid}")
                                        }
                                    } else {
                                        leaf_valid
                                    }
                                } else {
                                    normalize_pkz_virtual_path(raw_input)
                                        .map_err(|e| e.to_string())?
                                };

                            let collision_policy =
                                payload_val.get("collision_policy").and_then(|v| v.as_str());
                            let mut removed = Vec::new();
                            if !new_full_path.eq_ignore_ascii_case(&old_path) {
                                if let Some((other_idx, _)) = pkz
                                    .descriptors
                                    .iter()
                                    .enumerate()
                                    .find(|&(oi, d)| {
                                        oi != idx && d.name.eq_ignore_ascii_case(&new_full_path)
                                    })
                                {
                                    if collision_policy.is_none() {
                                        return Err(format!(
                                            "PKZ_COLLISION:{}",
                                            serde_json::json!({
                                                "count": 1,
                                                "paths": [new_full_path],
                                                "new_count": 0
                                            })
                                        ));
                                    }
                                    if matches!(collision_policy, Some("skip")) {
                                        return build_outline_from_session(&sess);
                                    }
                                    removed.push(other_idx);
                                }
                            }

                            let mut renamed = HashMap::new();
                            renamed.insert(idx, new_full_path.clone());
                            let repacked = pkz
                                .repack_with_options(
                                    &parent_bytes,
                                    &HashMap::new(),
                                    &[],
                                    &removed,
                                    None,
                                    None,
                                    None,
                                    Some(&renamed),
                                    oodle_opt,
                                )
                                .map_err(|e| e.to_string())?;
                            (repacked, format!("Rename PKZ entry to {new_full_path}"))
                        }
                        "reorder_entry" => {
                            let to_idx = compute_reorder_target(
                                idx,
                                pkz.descriptors.len(),
                                &payload_val,
                            )?;
                            let mut order: Vec<usize> = (0..pkz.descriptors.len()).collect();
                            let item = order.remove(idx);
                            order.insert(to_idx, item);
                            let repacked = pkz
                                .repack_with_replacements(
                                    &parent_bytes,
                                    &HashMap::new(),
                                    &[],
                                    &[],
                                    Some(&order),
                                    None,
                                    oodle_opt,
                                )
                                .map_err(|e| e.to_string())?;
                            (repacked, format!("Reorder PKZ entry #{idx} -> #{to_idx}"))
                        }
                        _ => unreachable!(),
                    }
                } else {
                    return Err(format!("Unsupported entry segment '{last_seg}' for {action}"));
                };

            let next_snap = write_back_subblob_with_align(
                &snap,
                &container_parts,
                new_parent_bytes,
                None,
                oodle_opt,
                dat_align,
            )?;
            sess.invalidate_texture_caches();
            record_mutation(&mut sess, desc, next_snap);
            build_outline_from_session(&sess)
        }
        _ => Err(format!("Unsupported container node action: {action}")),
    }
}
