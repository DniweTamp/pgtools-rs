use crate::session::{
    lock_session, record_mutation, AppState, DocSnapshot, PendingUvdSheetInfo, SaveEncodeOptions,
};
use crate::texture_cmd::{
    build_texture_node_view, encode_image_for_bank_entry, parse_requested_texture_format,
    TextureNodeView,
};
use crate::vfs::{
    build_outline_from_session, compute_reorder_target, detect_file_format_tag,
    is_bntx_blob, resolve_subblob, split_container_and_leaf,
    write_back_subblob_with_align, DocumentOutline,
};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use image::{Rgba, RgbaImage};
use pgtools_core::{
    dat::DatFile,
    io::validate_single_entry_name,
    texture_conv::{
        decode_bntx_subtexture_mip_filtered, decode_texture_mip_filtered, inspect_texture_blob,
        replace_bntx_subtexture_with_options, EncodeQuality,
        TextureFormat,
    },
    uvd::{
        compute_uvd_group_id, compute_uvd_item_id, crop_sprite_from_sheet,
        extract_group_to_dir, pack_group_sprites, validate_uvd_group_name, validate_uvd_item_name,
        UvdFile, UvdGroup, UvdItem,
    },
    wtb::{WtbFile, WtbGameProfile},
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tauri::State;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UvdGroupSummaryView {
    pub index: usize,
    pub node_id: String,
    pub name: String,
    pub id: u32,
    pub id_hex: String,
    pub item_count: usize,
    pub width: u32,
    pub height: u32,
    pub matched_wta_name: Option<String>,
    pub matched_wta_node_id: Option<String>,
    pub matched_tex_index: Option<usize>,
    pub has_pixel_payload: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UvdItemSummaryView {
    pub index: usize,
    pub node_id: String,
    pub name: String,
    pub id: u32,
    pub id_hex: String,
    pub group_id: u32,
    pub group_id_hex: String,
    pub group_name: String,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub inv_w: f32,
    pub inv_h: f32,
    pub tex_width: u32,
    pub tex_height: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UvdContextView {
    pub mode: String, // "uvd" | "group" | "item"
    pub uvd_node_id: String,
    pub uvd_name: String,
    pub endian: String,
    pub uvd_padding: u32,
    pub uvd_max_size: u32,
    pub group: Option<UvdGroupSummaryView>,
    pub item: Option<UvdItemSummaryView>,
    pub items: Vec<UvdItemSummaryView>,
    pub groups: Vec<UvdGroupSummaryView>,
    pub warning_banner: Option<String>,
}

/// Information about a located WTA/WTB texture bank inside the same parent container as a `.uvd`.
#[derive(Debug, Clone)]
pub(crate) struct SiblingWtaBank {
    pub container_parts: Vec<String>,
    pub container_id: String,
    pub wta_name: String,
    pub wtb: WtbFile,
    pub is_standalone_bntx: bool,
}

/// Parse `leaf` of a UVD node into `(Option<group_idx>, Option<item_idx>)`.
pub(crate) fn parse_uvd_leaf(leaf: &str) -> (Option<usize>, Option<usize>) {
    if leaf.is_empty() {
        return (None, None);
    }
    let mut grp_idx = None;
    let mut item_idx = None;
    for seg in leaf.split('/') {
        if let Some(g_str) = seg.strip_prefix("uvdgrp:") {
            grp_idx = g_str.parse::<usize>().ok();
        } else if let Some(i_str) = seg.strip_prefix("uvditem:") {
            item_idx = i_str.parse::<usize>().ok();
        }
    }
    (grp_idx, item_idx)
}

/// Discover all `.wta` / `.wtb` / `.bntx` texture banks in the same parent container as `uvd_container_parts`.
pub(crate) fn collect_sibling_wta_banks(
    snap: &DocSnapshot,
    uvd_container_parts: &[&str],
    oodle_opt: Option<&Path>,
) -> Vec<SiblingWtaBank> {
    let mut banks = Vec::new();
    if uvd_container_parts.is_empty() {
        return banks;
    }
    let parent_parts = &uvd_container_parts[..uvd_container_parts.len() - 1];
    let parent_id = parent_parts.join("/");
    let Ok((_, parent_bytes, _, _)) = resolve_subblob(snap, &parent_id, oodle_opt) else {
        return banks;
    };

    if let Ok(dat) = DatFile::from_bytes(&parent_bytes) {
        for (idx, entry) in dat.entries.iter().enumerate() {
            let tag = detect_file_format_tag(&entry.name, &entry.data);
            if matches!(tag, "WTB" | "BNTX") {
                let mut c_parts: Vec<String> =
                    parent_parts.iter().map(|s| (*s).to_string()).collect();
                c_parts.push(format!("dat:{idx}"));
                let cid = c_parts.join("/");
                if let Ok((wta_name, wta_bytes, wtp_bytes, _)) =
                    resolve_subblob(snap, &cid, oodle_opt)
                {
                    let is_bntx = is_bntx_blob(&wta_bytes);
                    if let Ok(wtb) = WtbFile::from_bytes(&wta_bytes, wtp_bytes.as_deref()) {
                        banks.push(SiblingWtaBank {
                            container_parts: c_parts,
                            container_id: cid,
                            wta_name,
                            wtb,
                            is_standalone_bntx: is_bntx,
                        });
                    }
                }
            }
        }
    }
    banks
}

/// Find the WTA bank and texture entry index whose `texture_id == target_id`.
pub(crate) fn find_wta_texture_by_id(
    banks: &[SiblingWtaBank],
    target_id: u32,
) -> Option<(usize, usize)> {
    for (bi, bank) in banks.iter().enumerate() {
        for (ei, entry) in bank.wtb.entries.iter().enumerate() {
            if entry.texture_id == Some(target_id) {
                return Some((bi, ei));
            }
        }
    }
    None
}

/// Find the first available WTA texture across `banks` (used as fallback when no matching ID exists).
pub(crate) fn find_first_available_wta_texture(banks: &[SiblingWtaBank]) -> Option<(usize, usize)> {
    for (bi, bank) in banks.iter().enumerate() {
        if !bank.wtb.entries.is_empty() {
            return Some((bi, 0));
        }
    }
    None
}

/// Render a clean wireframe/checkerboard atlas layout preview PNG for a UVD group when its WTA
/// texture payload is missing (e.g. header-only `ui_*.dat` without `.dtt`) or when no WTA matches.
pub(crate) fn render_uvd_group_wireframe_preview(
    group: &UvdGroup,
    sheet_w: u32,
    sheet_h: u32,
) -> Option<String> {
    let w = sheet_w.clamp(64, 1024);
    let h = sheet_h.clamp(64, 1024);
    let scale_x = (w as f32) / (sheet_w.max(1) as f32);
    let scale_y = (h as f32) / (sheet_h.max(1) as f32);

    let mut img = RgbaImage::from_pixel(w, h, Rgba([22, 26, 34, 255]));
    // Subtle 32px grid lines
    for y in 0..h {
        for x in 0..w {
            if x % 32 == 0 || y % 32 == 0 {
                img.put_pixel(x, y, Rgba([34, 40, 52, 255]));
            }
        }
    }

    for (idx, it) in group.items.iter().enumerate() {
        let x0 = ((it.x * scale_x).round().max(0.0) as u32).min(w.saturating_sub(1));
        let y0 = ((it.y * scale_y).round().max(0.0) as u32).min(h.saturating_sub(1));
        let x1 = (((it.x + it.w) * scale_x).round().max(1.0) as u32).min(w);
        let y1 = (((it.y + it.h) * scale_y).round().max(1.0) as u32).min(h);
        if x1 <= x0 || y1 <= y0 {
            continue;
        }
        let hue_r = (80 + (idx * 53) % 140) as u8;
        let hue_g = (110 + (idx * 89) % 120) as u8;
        let hue_b = (160 + (idx * 37) % 90) as u8;

        for py in y0..y1 {
            for px in x0..x1 {
                let is_border =
                    px == x0 || px + 1 == x1 || py == y0 || py + 1 == y1;
                if is_border {
                    img.put_pixel(px, py, Rgba([hue_r, hue_g, hue_b, 235]));
                } else {
                    img.put_pixel(px, py, Rgba([hue_r / 3, hue_g / 3, hue_b / 3, 140]));
                }
            }
        }
    }

    let mut png_buf = Cursor::new(Vec::new());
    if img.write_to(&mut png_buf, image::ImageFormat::Png).is_ok() {
        Some(format!(
            "data:image/png;base64,{}",
            BASE64.encode(png_buf.into_inner())
        ))
    } else {
        None
    }
}

/// If `node_id` points to a `uvdgrp:<g_idx>`, resolve it to its matched WTA texture node ID
/// (`<wta_container_id>/tex:<entry_idx>`) so existing Texture Editor actions work directly on the group's sheet.
pub(crate) fn resolve_uvd_group_to_wta_tex_node(
    snap: &DocSnapshot,
    node_id: &str,
    oodle_opt: Option<&Path>,
) -> Option<String> {
    let (container_parts, leaf) = split_container_and_leaf(node_id);
    let (Some(g_idx), None) = parse_uvd_leaf(leaf) else {
        return None;
    };
    let container_id = container_parts.join("/");
    let (_, sub_bytes, _, _) = resolve_subblob(snap, &container_id, oodle_opt).ok()?;
    let uvd = UvdFile::from_bytes(&sub_bytes).ok()?;
    let group = uvd.groups.get(g_idx)?;
    let banks = collect_sibling_wta_banks(snap, &container_parts, oodle_opt);
    let (bi, ei) = find_wta_texture_by_id(&banks, group.id)?;
    let bank = &banks[bi];
    if bank.container_id.is_empty() || bank.container_id == "root" {
        Some(format!("tex:{ei}"))
    } else {
        Some(format!("{}/tex:{ei}", bank.container_id))
    }
}

fn build_group_summary(
    uvd_node_id: &str,
    g_idx: usize,
    group: &UvdGroup,
    banks: &[SiblingWtaBank],
) -> UvdGroupSummaryView {
    let group_node_id = if uvd_node_id == "root" || uvd_node_id.is_empty() {
        format!("uvdgrp:{g_idx}")
    } else {
        format!("{uvd_node_id}/uvdgrp:{g_idx}")
    };
    let matched = find_wta_texture_by_id(banks, group.id);
    let (mut width, mut height) = group.inferred_dimensions().unwrap_or((256, 256));
    let mut matched_wta_name = None;
    let mut matched_wta_node_id = None;
    let mut matched_tex_index = None;
    let mut has_pixel_payload = false;

    if let Some((bi, ei)) = matched {
        let bank = &banks[bi];
        let entry = &bank.wtb.entries[ei];
        if let Some(ref meta) = entry.metadata {
            if meta.width > 0 && meta.height > 0 {
                width = meta.width;
                height = meta.height;
            }
        }
        matched_wta_name = Some(bank.wta_name.clone());
        matched_wta_node_id = Some(if bank.container_id.is_empty() {
            format!("tex:{ei}")
        } else {
            format!("{}/tex:{ei}", bank.container_id)
        });
        matched_tex_index = Some(ei);
        has_pixel_payload = entry.has_pixel_payload();
    }

    UvdGroupSummaryView {
        index: g_idx,
        node_id: group_node_id,
        name: group.name.clone(),
        id: group.id,
        id_hex: format!("0x{:08X}", group.id),
        item_count: group.items.len(),
        width,
        height,
        matched_wta_name,
        matched_wta_node_id,
        matched_tex_index,
        has_pixel_payload,
    }
}

fn build_item_summary(
    group_node_id: &str,
    group_name: &str,
    i_idx: usize,
    item: &UvdItem,
) -> UvdItemSummaryView {
    UvdItemSummaryView {
        index: i_idx,
        node_id: format!("{group_node_id}/uvditem:{i_idx}"),
        name: item.name.clone(),
        id: item.id,
        id_hex: format!("0x{:08X}", item.id),
        group_id: item.group_id,
        group_id_hex: format!("0x{:08X}", item.group_id),
        group_name: group_name.to_string(),
        x: item.x,
        y: item.y,
        w: item.w,
        h: item.h,
        inv_w: item.inv_w,
        inv_h: item.inv_h,
        tex_width: item.tex_width(),
        tex_height: item.tex_height(),
    }
}

/// Build the inspect payload when a `.uvd` node, `uvdgrp:<g_idx>` node, or `uvditem:<i_idx>` node is selected.
#[allow(clippy::too_many_arguments)]
pub(crate) fn build_uvd_inspect_payload(
    snap: &DocSnapshot,
    node_id: &str,
    doc_name: &str,
    display_name: &str,
    uvd_bytes: &[u8],
    leaf: &str,
    mip_index: u32,
    layer_index: u32,
    channel: &str,
    opaque: bool,
    uvd_padding: u32,
    uvd_max_size: u32,
    oodle_opt: Option<&Path>,
    nvtt_opt: Option<&Path>,
) -> Result<serde_json::Value, String> {
    let uvd = UvdFile::from_bytes(uvd_bytes).map_err(|e| e.to_string())?;
    let (container_parts, _) = split_container_and_leaf(node_id);
    let uvd_node_id = if container_parts.is_empty() {
        "root".to_string()
    } else {
        container_parts.join("/")
    };
    let banks = collect_sibling_wta_banks(snap, &container_parts, oodle_opt);

    let group_summaries: Vec<UvdGroupSummaryView> = uvd
        .groups
        .iter()
        .enumerate()
        .map(|(gi, g)| build_group_summary(&uvd_node_id, gi, g, &banks))
        .collect();

    let (grp_idx_opt, item_idx_opt) = parse_uvd_leaf(leaf);

    // 1. UVD Root Node (`leaf == ""`)
    let Some(g_idx) = grp_idx_opt else {
        let ctx = UvdContextView {
            mode: "uvd".to_string(),
            uvd_node_id: uvd_node_id.clone(),
            uvd_name: display_name.to_string(),
            endian: format!("{:?}", uvd.endian),
            uvd_padding,
            uvd_max_size,
            group: None,
            item: None,
            items: Vec::new(),
            groups: group_summaries,
            warning_banner: None,
        };
        return Ok(serde_json::json!({
            "view_type": "UVD",
            "node_id": node_id,
            "title": display_name,
            "spec_line": format!(
                "UVD Atlas · {:?} endian · {} groups · {} items · {} B",
                uvd.endian,
                uvd.groups.len(),
                uvd.total_items(),
                uvd_bytes.len()
            ),
            "uvd": ctx,
        }));
    };

    let group = uvd
        .groups
        .get(g_idx)
        .ok_or_else(|| format!("UVD Group #{g_idx} out of range"))?;
    let group_summary = group_summaries[g_idx].clone();
    let item_summaries: Vec<UvdItemSummaryView> = group
        .items
        .iter()
        .enumerate()
        .map(|(ii, it)| build_item_summary(&group_summary.node_id, &group.name, ii, it))
        .collect();

    // 2. UVD Group Selected (`uvdgrp:<g_idx>`)
    let Some(i_idx) = item_idx_opt else {
        let exact_match = find_wta_texture_by_id(&banks, group.id);
        let fallback_match = if exact_match.is_none() {
            find_first_available_wta_texture(&banks)
        } else {
            None
        };

        let mut warning_banner: Option<String> = None;
        let mut tex_view: TextureNodeView = if let Some((bi, ei)) = exact_match {
            let bank = &banks[bi];
            let entry = &bank.wtb.entries[ei];
            let mut v = build_texture_node_view(
                node_id,
                ei,
                &format!("{doc_name} / {display_name} / {}", group.name),
                &entry.data,
                entry.flags,
                entry.texture_id,
                &entry.extension,
                bank.wtb.game_profile,
                mip_index,
                layer_index,
                channel,
                opaque,
                nvtt_opt,
            );
            v.title = format!("Group '{}' (0x{:08X})", group.name, group.id);
            if !entry.has_pixel_payload() {
                let (gw, gh) = if v.width > 0 && v.height > 0 {
                    (v.width, v.height)
                } else {
                    group.inferred_dimensions().unwrap_or((256, 256))
                };
                v.width = gw;
                v.height = gh;
                v.preview_data_url = render_uvd_group_wireframe_preview(group, gw, gh);
                v.validation_ok = false;
                let msg = format!(
                    "Matched '{}' texture #{} (0x{:08X}), but texture payload is missing (header-only WTA without DTT/WTP pixel data). Replace, Add, or Repack to populate pixel data.",
                    bank.wta_name, ei, group.id
                );
                v.validation_msg = msg.clone();
                warning_banner = Some(msg);
            }
            v
        } else if let Some((bi, ei)) = fallback_match {
            let bank = &banks[bi];
            let entry = &bank.wtb.entries[ei];
            let mut v = build_texture_node_view(
                node_id,
                ei,
                &format!("{doc_name} / {display_name} / {}", group.name),
                &entry.data,
                entry.flags,
                entry.texture_id,
                &entry.extension,
                bank.wtb.game_profile,
                mip_index,
                layer_index,
                channel,
                opaque,
                nvtt_opt,
            );
            let (gw, gh) = group.inferred_dimensions().unwrap_or((256, 256));
            if !entry.has_pixel_payload() || v.preview_data_url.is_none() {
                v.preview_data_url = render_uvd_group_wireframe_preview(group, gw, gh);
            }
            v.title = format!("Group '{}' (0x{:08X}) [Unmatched]", group.name, group.id);
            v.validation_ok = false;
            let msg = format!(
                "Warning: No WTA texture with ID 0x{:08X} found for group '{}'. Showing placeholder / first available texture from '{}'.",
                group.id, group.name, bank.wta_name
            );
            v.validation_msg = msg.clone();
            warning_banner = Some(msg);
            v
        } else {
            let (gw, gh) = group.inferred_dimensions().unwrap_or((256, 256));
            let msg = format!(
                "Warning: No WTA texture bank or matching ID 0x{:08X} found for group '{}'. Showing UV wireframe placeholder.",
                group.id, group.name
            );
            warning_banner = Some(msg.clone());
            TextureNodeView {
                node_id: node_id.to_string(),
                index: g_idx,
                title: format!("Group '{}' (0x{:08X})", group.name, group.id),
                breadcrumb: format!("{doc_name} / {display_name} / {}", group.name),
                spec_line: format!(
                    "UVD Group | {} × {} | {} items | ID 0x{:08X}",
                    gw,
                    gh,
                    group.items.len(),
                    group.id
                ),
                validation_ok: false,
                validation_msg: msg,
                format_name: "Placeholder".to_string(),
                container_ext: ".xt1".to_string(),
                game_profile: WtbGameProfile::SwitchXt1.label().to_string(),
                has_confirmed_xt1_flags: true,
                dimension: "2D".to_string(),
                width: gw,
                height: gh,
                depth: 1,
                layer_count: 1,
                current_layer: 0,
                mip_count: 1,
                mips: vec![crate::texture_cmd::MipLevelDesc {
                    index: 0,
                    label: format!("mip 0 ({gw}x{gh})"),
                    width: gw,
                    height: gh,
                }],
                size_bytes: 0,
                flags_hex: "0x60000020".to_string(),
                table2_bytes_str: "20 00 00 60".to_string(),
                has_crc32_flag: true,
                srgb_flag: false,
                cubemap_flag: false,
                texture_id_hex: format!("0x{:08X}", group.id),
                embedded_name: Some(group.name.clone()),
                nvtt_status: "Built-in pure-Rust encoder active".to_string(),
                preview_data_url: render_uvd_group_wireframe_preview(group, gw, gh),
                gpu_direct: false,
                raw_header_hex: String::new(),
                uvd_mode: Some("group".to_string()),
            }
        };
        tex_view.uvd_mode = Some("group".to_string());

        let ctx = UvdContextView {
            mode: "group".to_string(),
            uvd_node_id,
            uvd_name: display_name.to_string(),
            endian: format!("{:?}", uvd.endian),
            uvd_padding,
            uvd_max_size,
            group: Some(group_summary),
            item: None,
            items: item_summaries,
            groups: group_summaries,
            warning_banner,
        };

        return Ok(serde_json::json!({
            "view_type": "TEXTURE",
            "texture": tex_view,
            "uvd": ctx,
        }));
    };

    // 3. UVD Item Selected (`uvdgrp:<g_idx>/uvditem:<i_idx>`)
    // Always crop from the texture whose ID equals `item.group_id` and hide normal texture controls.
    let item = group
        .items
        .get(i_idx)
        .ok_or_else(|| format!("UVD Item #{i_idx} out of range"))?;
    let item_summary = item_summaries[i_idx].clone();
    let matched_for_item = find_wta_texture_by_id(&banks, item.group_id);

    let crop_w = item.w.round().max(1.0) as u32;
    let crop_h = item.h.round().max(1.0) as u32;

    let (preview_data_url, validation_ok, validation_msg, fmt_name, ext_name, profile_name) =
        if let Some((bi, ei)) = matched_for_item {
            let bank = &banks[bi];
            let entry = &bank.wtb.entries[ei];
            let fmt_str = entry
                .metadata
                .as_ref()
                .map(|m| m.format.display_name())
                .unwrap_or_else(|| "Unknown".to_string());
            if entry.has_pixel_payload() {
                let decoded_sheet = if bank.is_standalone_bntx {
                    decode_bntx_subtexture_mip_filtered(&entry.data, ei, 0, "RGBA", false)
                } else {
                    decode_texture_mip_filtered(&entry.data, 0, "RGBA", false)
                };
                match decoded_sheet {
                    Ok((sheet_img, _, _)) => {
                        let cropped = crop_sprite_from_sheet(&sheet_img, item);
                        let mut png_buf = Cursor::new(Vec::new());
                        let url = if cropped.write_to(&mut png_buf, image::ImageFormat::Png).is_ok()
                        {
                            Some(format!(
                                "data:image/png;base64,{}",
                                BASE64.encode(png_buf.into_inner())
                            ))
                        } else {
                            None
                        };
                        (
                            url,
                            true,
                            format!(
                                "Cropped sprite '{}' ({}×{} at {}, {}) from '{}' texture #{} (0x{:08X}).",
                                item.name, crop_w, crop_h, item.x, item.y, bank.wta_name, ei, item.group_id
                            ),
                            fmt_str,
                            entry.extension.clone(),
                            bank.wtb.game_profile.label().to_string(),
                        )
                    }
                    Err(e) => (
                        None,
                        false,
                        format!("Failed to decode sheet 0x{:08X}: {e}", item.group_id),
                        fmt_str,
                        entry.extension.clone(),
                        bank.wtb.game_profile.label().to_string(),
                    ),
                }
            } else {
                (
                    None,
                    false,
                    format!(
                        "Texture payload missing for group '{}' (0x{:08X} in '{}'). Use 'Replace Sprite...' to populate pixel data.",
                        group.name, item.group_id, bank.wta_name
                    ),
                    fmt_str,
                    entry.extension.clone(),
                    bank.wtb.game_profile.label().to_string(),
                )
            }
        } else {
            (
                None,
                false,
                format!(
                    "No WTA texture found with ID 0x{:08X} (item.group_id) for sprite '{}'.",
                    item.group_id, item.name
                ),
                "Unmatched".to_string(),
                ".xt1".to_string(),
                WtbGameProfile::SwitchXt1.label().to_string(),
            )
        };

    let tex_view = TextureNodeView {
        node_id: node_id.to_string(),
        index: i_idx,
        title: format!("Sprite '{}'", item.name),
        breadcrumb: format!(
            "{doc_name} / {display_name} / {} / {}",
            group.name, item.name
        ),
        spec_line: format!(
            "Sprite {} × {} at ({}, {}) | ID 0x{:08X} | Group '{}' (0x{:08X})",
            crop_w, crop_h, item.x, item.y, item.id, group.name, item.group_id
        ),
        validation_ok,
        validation_msg: validation_msg.clone(),
        format_name: fmt_name,
        container_ext: ext_name,
        game_profile: profile_name,
        has_confirmed_xt1_flags: false,
        dimension: "Sprite".to_string(),
        width: crop_w,
        height: crop_h,
        depth: 1,
        layer_count: 1,
        current_layer: 0,
        mip_count: 1,
        mips: vec![crate::texture_cmd::MipLevelDesc {
            index: 0,
            label: format!("sprite ({crop_w}x{crop_h})"),
            width: crop_w,
            height: crop_h,
        }],
        size_bytes: 96,
        flags_hex: "0x00000000".to_string(),
        table2_bytes_str: String::new(),
        has_crc32_flag: true,
        srgb_flag: false,
        cubemap_flag: false,
        texture_id_hex: format!("0x{:08X}", item.id),
        embedded_name: Some(item.name.clone()),
        nvtt_status: String::new(),
        preview_data_url,
        gpu_direct: false,
        raw_header_hex: String::new(),
        uvd_mode: Some("item".to_string()),
    };

    let ctx = UvdContextView {
        mode: "item".to_string(),
        uvd_node_id,
        uvd_name: display_name.to_string(),
        endian: format!("{:?}", uvd.endian),
        uvd_padding,
        uvd_max_size,
        group: Some(group_summary),
        item: Some(item_summary),
        items: item_summaries,
        groups: group_summaries,
        warning_banner: if validation_ok {
            None
        } else {
            Some(validation_msg)
        },
    };

    Ok(serde_json::json!({
        "view_type": "TEXTURE",
        "texture": tex_view,
        "uvd": ctx,
    }))
}

/// Extract existing sprites for `group` from each item's matched WTA texture (`texture_id == item.group_id`,
/// preferring any lossless pending sheet in `pending_map` to avoid multi-generation block-compression loss),
/// or return transparent placeholder `RgbaImage`s of each item's `(w, h)` when payload is missing.
fn extract_or_init_group_sprites(
    group: &UvdGroup,
    banks: &[SiblingWtaBank],
    pending_map: &BTreeMap<(String, u32), PendingUvdSheetInfo>,
) -> Vec<(String, u32, RgbaImage)> {
    let mut decoded_cache: BTreeMap<u32, Option<Arc<RgbaImage>>> = BTreeMap::new();
    let mut get_sheet = |tex_id: u32| -> Option<Arc<RgbaImage>> {
        if let Some(cached) = decoded_cache.get(&tex_id) {
            return cached.clone();
        }
        let resolved = find_wta_texture_by_id(banks, tex_id).and_then(|(bi, ei)| {
            let bank = &banks[bi];
            if let Some(pending) = pending_map.get(&(bank.container_id.clone(), tex_id)) {
                if let Some(ref lossless) = pending.lossless_sheet {
                    return Some(lossless.clone());
                }
            }
            let entry = &bank.wtb.entries[ei];
            if !entry.has_pixel_payload() {
                return None;
            }
            let decoded = if bank.is_standalone_bntx {
                decode_bntx_subtexture_mip_filtered(&entry.data, ei, 0, "RGBA", false)
            } else {
                decode_texture_mip_filtered(&entry.data, 0, "RGBA", false)
            };
            decoded.ok().map(|(img, _, _)| Arc::new(img))
        });
        decoded_cache.insert(tex_id, resolved.clone());
        resolved
    };

    group
        .items
        .iter()
        .map(|it| {
            let target_tid = if it.group_id != 0 { it.group_id } else { group.id };
            if let Some(sheet_img) = get_sheet(target_tid) {
                (
                    it.name.clone(),
                    it.id,
                    crop_sprite_from_sheet(&sheet_img, it),
                )
            } else {
                let w = it.w.round().max(1.0) as u32;
                let h = it.h.round().max(1.0) as u32;
                (
                    it.name.clone(),
                    it.id,
                    RgbaImage::from_pixel(w, h, Rgba([0, 0, 0, 0])),
                )
            }
        })
        .collect()
}

/// Default `TextureFormat` for a `WtbGameProfile` when an entry's original format is not known.
pub(crate) fn default_format_for_profile(profile: WtbGameProfile) -> TextureFormat {
    match profile {
        WtbGameProfile::SwitchXt1 | WtbGameProfile::Bayo2SwitchBntx => {
            TextureFormat::Astc(4, 4, true)
        }
        _ => TextureFormat::Bc3,
    }
}

/// Encode a repacked `RgbaImage` sheet into its matched WTA texture entry (or create a new WTA entry
/// if `create_if_missing` is true), write back to `snap`, and record `PendingUvdSheetInfo` so the
/// save-time encode/recompression prompt can offer the original texture's format.
#[allow(clippy::too_many_arguments)]
fn apply_repacked_sheet_to_wta(
    snap: &DocSnapshot,
    uvd_container_parts: &[&str],
    uvd_name: &str,
    uvd: &UvdFile,
    group_name: &str,
    group_id: u32,
    sheet: &RgbaImage,
    create_if_missing: bool,
    oodle_opt: Option<&Path>,
    nvtt_opt: Option<&Path>,
    dat_align: Option<usize>,
    pending_map: &mut BTreeMap<(String, u32), PendingUvdSheetInfo>,
) -> Result<DocSnapshot, String> {
    let mut banks = collect_sibling_wta_banks(snap, uvd_container_parts, oodle_opt);

    if let Some((bi, ei)) = find_wta_texture_by_id(&banks, group_id) {
        let bank = &mut banks[bi];
        let game_profile = bank.wtb.game_profile;
        let entry = &mut bank.wtb.entries[ei];
        let orig_fmt = entry
            .metadata
            .as_ref()
            .map(|m| m.format)
            .unwrap_or_else(|| default_format_for_profile(game_profile));
        let orig_fmt_name = orig_fmt.display_name();
        let prev_bntx_name = entry.metadata.as_ref().and_then(|m| m.name.clone());

        let new_blob = if entry.extension == ".bntx" {
            replace_bntx_subtexture_with_options(
                &entry.data,
                ei,
                Err(sheet),
                Some(orig_fmt),
                false,
                true,
                false,
                EncodeQuality::Fastest,
                nvtt_opt,
            )
            .map_err(|e| e.to_string())?
        } else {
            encode_image_for_bank_entry(
                sheet,
                &entry.extension,
                orig_fmt,
                Some(1),
                false,
                true,
                false,
                EncodeQuality::Fastest,
                Some(&entry.data),
                nvtt_opt,
            )
        };

        entry.metadata = inspect_texture_blob(&new_blob).ok();
        if let Some(ref mut m) = entry.metadata {
            if m.name.is_none() {
                m.name = prev_bntx_name;
            }
        }
        entry.data = new_blob.clone();
        if game_profile == WtbGameProfile::SwitchXt1 {
            entry.sync_xt1_flags_from_metadata(game_profile);
        }

        let (new_wta, new_wtp) = if bank.is_standalone_bntx {
            (new_blob, None)
        } else {
            bank.wtb.to_bytes()
        };

        let c_refs: Vec<&str> = bank.container_parts.iter().map(|s| s.as_str()).collect();
        let next_snap = write_back_subblob_with_align(
            snap,
            &c_refs,
            new_wta,
            new_wtp,
            oodle_opt,
            dat_align,
        )?;

        pending_map.insert(
            (bank.container_id.clone(), group_id),
            PendingUvdSheetInfo {
                wta_container_id: bank.container_id.clone(),
                wta_name: bank.wta_name.clone(),
                texture_id: group_id,
                texture_id_hex: format!("0x{group_id:08X}"),
                group_name: group_name.to_string(),
                width: sheet.width(),
                height: sheet.height(),
                original_format: orig_fmt_name,
                game_profile: game_profile.label().to_string(),
                has_confirmed_xt1_flags: game_profile.has_confirmed_xt1_flags(),
                lossless_sheet: Some(Arc::new(sheet.clone())),
            },
        );
        return Ok(next_snap);
    }

    if !create_if_missing || uvd_container_parts.is_empty() {
        return Ok(snap.clone());
    }

    // Choose target WTA:
    // 1. Prefer the WTA that already contains matching texture IDs for this UVD's groups.
    // 2. If none, use the first WTA in the same DAT.
    // 3. If none exist, create `<basename>.wta`.
    let existing_gids: BTreeSet<u32> = uvd.groups.iter().map(|g| g.id).collect();
    let mut best_bank_idx: Option<usize> = None;
    let mut best_match_count = 0usize;
    for (bi, bank) in banks.iter().enumerate() {
        let matches = bank
            .wtb
            .entries
            .iter()
            .filter(|e| e.texture_id.map_or(false, |tid| existing_gids.contains(&tid)))
            .count();
        if matches > best_match_count {
            best_match_count = matches;
            best_bank_idx = Some(bi);
        }
    }
    if best_bank_idx.is_none() && !banks.is_empty() {
        best_bank_idx = Some(0);
    }

    if let Some(bi) = best_bank_idx {
        let bank = &mut banks[bi];
        let game_profile = bank.wtb.game_profile;
        let ext = bank
            .wtb
            .entries
            .first()
            .map(|e| e.extension.clone())
            .unwrap_or_else(|| match game_profile {
                WtbGameProfile::SwitchXt1 => ".xt1".to_string(),
                WtbGameProfile::Bayo2WiiUGtx => ".gtx".to_string(),
                WtbGameProfile::Bayo2SwitchBntx => ".bntx".to_string(),
                _ => ".dds".to_string(),
            });
        let orig_fmt = bank
            .wtb
            .entries
            .first()
            .and_then(|e| e.metadata.as_ref().map(|m| m.format))
            .unwrap_or_else(|| default_format_for_profile(game_profile));
        let template_hdr = bank.wtb.entries.first().map(|e| e.data.as_slice());

        let new_blob = encode_image_for_bank_entry(
            sheet,
            &ext,
            orig_fmt,
            Some(1),
            false,
            true,
            false,
            EncodeQuality::Fastest,
            template_hdr,
            nvtt_opt,
        );
        let default_flags = game_profile.default_entry_flags(bank.wtb.is_split_wta_wtp, true);
        bank.wtb
            .push(new_blob, default_flags, Some(group_id))
            .map_err(|e| e.to_string())?;
        if game_profile == WtbGameProfile::SwitchXt1 {
            if let Some(last) = bank.wtb.entries.last_mut() {
                last.sync_xt1_flags_from_metadata(game_profile);
            }
        }
        let (new_wta, new_wtp) = bank.wtb.to_bytes();
        let c_refs: Vec<&str> = bank.container_parts.iter().map(|s| s.as_str()).collect();
        let next_snap = write_back_subblob_with_align(
            snap,
            &c_refs,
            new_wta,
            new_wtp,
            oodle_opt,
            dat_align,
        )?;
        pending_map.insert(
            (bank.container_id.clone(), group_id),
            PendingUvdSheetInfo {
                wta_container_id: bank.container_id.clone(),
                wta_name: bank.wta_name.clone(),
                texture_id: group_id,
                texture_id_hex: format!("0x{group_id:08X}"),
                group_name: group_name.to_string(),
                width: sheet.width(),
                height: sheet.height(),
                original_format: orig_fmt.display_name(),
                game_profile: game_profile.label().to_string(),
                has_confirmed_xt1_flags: game_profile.has_confirmed_xt1_flags(),
                lossless_sheet: Some(Arc::new(sheet.clone())),
            },
        );
        return Ok(next_snap);
    }

    // No WTA exists in the parent DAT -> create `<basename>.wta` (and `<basename>.wtp`)
    let parent_parts = &uvd_container_parts[..uvd_container_parts.len() - 1];
    let parent_id = parent_parts.join("/");
    let (_, parent_bytes, _, _) = resolve_subblob(snap, &parent_id, oodle_opt)?;
    let mut dat = DatFile::from_bytes(&parent_bytes).map_err(|e| e.to_string())?;

    let base_stem = Path::new(uvd_name)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("textures");
    let new_wta_name = format!("{base_stem}.wta");
    let new_wtp_name = format!("{base_stem}.wtp");

    let profile = if dat.endian.is_big() {
        WtbGameProfile::Bayo2WiiUGtx
    } else {
        WtbGameProfile::SwitchXt1
    };
    let mut new_wtb = WtbFile::new_empty(true, profile);
    let default_fmt = default_format_for_profile(profile);
    let ext = if profile == WtbGameProfile::Bayo2WiiUGtx {
        ".gtx"
    } else {
        ".xt1"
    };
    let new_blob = encode_image_for_bank_entry(
        sheet,
        ext,
        default_fmt,
        Some(1),
        false,
        true,
        false,
        EncodeQuality::Fastest,
        None,
        nvtt_opt,
    );
    let flags = profile.default_entry_flags(true, true);
    new_wtb
        .push(new_blob, flags, Some(group_id))
        .map_err(|e| e.to_string())?;
    let (wta_bytes, wtp_bytes_opt) = new_wtb.to_bytes();

    dat.push(new_wta_name.clone(), wta_bytes)
        .map_err(|e| e.to_string())?;
    let new_wta_idx = dat.entries.len() - 1;
    if let Some(wtp_bytes) = wtp_bytes_opt {
        dat.push(new_wtp_name, wtp_bytes)
            .map_err(|e| e.to_string())?;
    }

    let mut new_wta_cparts: Vec<String> = parent_parts.iter().map(|s| (*s).to_string()).collect();
    new_wta_cparts.push(format!("dat:{new_wta_idx}"));
    let new_wta_cid = new_wta_cparts.join("/");

    let next_snap = write_back_subblob_with_align(
        snap,
        parent_parts,
        dat.to_bytes_with_alignment(dat_align),
        None,
        oodle_opt,
        dat_align,
    )?;

    pending_map.insert(
        (new_wta_cid.clone(), group_id),
        PendingUvdSheetInfo {
            wta_container_id: new_wta_cid,
            wta_name: new_wta_name,
            texture_id: group_id,
            texture_id_hex: format!("0x{group_id:08X}"),
            group_name: group_name.to_string(),
            width: sheet.width(),
            height: sheet.height(),
            original_format: default_fmt.display_name(),
            game_profile: profile.label().to_string(),
            has_confirmed_xt1_flags: profile.has_confirmed_xt1_flags(),
            lossless_sheet: Some(Arc::new(sheet.clone())),
        },
    );

    Ok(next_snap)
}

#[tauri::command]
pub fn mutate_uvd_node(
    node_id: String,
    action: String,
    payload: serde_json::Value,
    state: State<'_, AppState>,
) -> Result<DocumentOutline, String> {
    let mut sess = lock_session(&state);
    let snap = sess.current.clone().ok_or("No active document loaded")?;
    let oodle_path = sess.settings.oodle_dll_path.clone();
    let nvtt_path = sess.settings.nvtt_dll_path.clone();
    let uvd_padding = sess.settings.uvd_padding;
    let uvd_max_size = sess.settings.uvd_max_size;
    let dat_align = sess.settings.effective_dat_alignment();

    let oodle_opt = if oodle_path.is_empty() {
        None
    } else {
        Some(Path::new(oodle_path.as_str()))
    };
    let nvtt_opt = if nvtt_path.is_empty() {
        None
    } else {
        Some(Path::new(nvtt_path.as_str()))
    };

    let (container_parts, leaf) = split_container_and_leaf(&node_id);
    let container_id = container_parts.join("/");
    let (uvd_name, uvd_bytes, _, _) = resolve_subblob(&snap, &container_id, oodle_opt)?;
    let mut uvd = UvdFile::from_bytes(&uvd_bytes).map_err(|e| e.to_string())?;
    let (leaf_grp_idx, leaf_item_idx) = parse_uvd_leaf(leaf);

    let mut cur_snap = snap;
    let mut next_pending = sess.pending_uvd_sheets.clone();

    let desc = match action.as_str() {
        "add_group" => {
            let raw_name = payload
                .get("name")
                .and_then(|v| v.as_str())
                .ok_or("Missing group name")?;
            let auto_suffix = payload
                .get("auto_suffix")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let group_name = if auto_suffix {
                uvd.auto_suffix_unique_name(raw_name, true)
                    .map_err(|e| e.to_string())?
            } else {
                let valid = validate_uvd_group_name(raw_name).map_err(|e| e.to_string())?;
                if let Some(collision) = uvd.check_name_crc_collision(&valid, true, None, None) {
                    return Err(collision);
                }
                valid
            };
            let new_group_id = compute_uvd_group_id(&group_name);

            // Optional initial sprite file(s)
            let mut initial_paths: Vec<String> = payload
                .get("paths")
                .and_then(|v| serde_json::from_value(v.clone()).ok())
                .unwrap_or_default();
            if let Some(single_p) = payload.get("path").and_then(|v| v.as_str()) {
                if !single_p.trim().is_empty() && !initial_paths.contains(&single_p.to_string()) {
                    initial_paths.push(single_p.to_string());
                }
            }

            // Temporarily register the new group so collision checks and auto-suffixing account for
            // `new_group_id` as well as earlier sprites within `initial_paths`.
            uvd.groups.push(UvdGroup {
                name: group_name.clone(),
                id: new_group_id,
                items: Vec::new(),
            });
            let temp_gi = uvd.groups.len() - 1;

            let mut sprites: Vec<(String, u32, RgbaImage)> = Vec::new();
            for p_str in &initial_paths {
                let p = Path::new(p_str);
                let stem = p
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("sprite_01");
                let item_name = if let Some(explicit_item_name) =
                    payload.get("item_name").and_then(|v| v.as_str())
                {
                    if initial_paths.len() == 1 && !explicit_item_name.trim().is_empty() {
                        validate_uvd_item_name(explicit_item_name).map_err(|e| e.to_string())?
                    } else {
                        uvd.auto_suffix_unique_name(stem, false)
                            .map_err(|e| e.to_string())?
                    }
                } else {
                    uvd.auto_suffix_unique_name(stem, false)
                        .map_err(|e| e.to_string())?
                };
                if let Some(collision) = uvd.check_name_crc_collision(&item_name, false, None, None)
                {
                    return Err(collision);
                }
                let raw_img = fs::read(p).map_err(|e| format!("Failed to read {}: {e}", p.display()))?;
                let rgba = image::load_from_memory(&raw_img)
                    .map_err(|e| format!("Failed to decode {}: {e}", p.display()))?
                    .to_rgba8();
                let iid = compute_uvd_item_id(&item_name);
                uvd.groups[temp_gi].items.push(UvdItem::new(
                    item_name.clone(),
                    new_group_id,
                    0.0,
                    0.0,
                    rgba.width() as f32,
                    rgba.height() as f32,
                    256,
                    256,
                ));
                sprites.push((item_name, iid, rgba));
            }
            uvd.groups.pop();

            let packed = pack_group_sprites(
                &group_name,
                new_group_id,
                &sprites,
                uvd_padding,
                uvd_max_size,
                Some((256, 256)),
            )
            .map_err(|e| e.to_string())?;

            // Create new texture entry in preferred WTA before pushing the new group so existing group IDs determine WTA preference
            cur_snap = apply_repacked_sheet_to_wta(
                &cur_snap,
                &container_parts,
                &uvd_name,
                &uvd,
                &group_name,
                new_group_id,
                &packed.sheet,
                true,
                oodle_opt,
                nvtt_opt,
                dat_align,
                &mut next_pending,
            )?;

            uvd.groups.push(packed.group);
            format!(
                "Add UVD group '{}' (0x{:08X}, {} sprite(s))",
                group_name,
                new_group_id,
                sprites.len()
            )
        }
        "delete_groups" => {
            let mut indices: Vec<usize> = payload
                .get("group_indices")
                .and_then(|v| serde_json::from_value(v.clone()).ok())
                .unwrap_or_default();
            if indices.is_empty() {
                if let Some(gi) = leaf_grp_idx {
                    indices.push(gi);
                }
            }
            indices.sort_unstable();
            indices.dedup();
            if indices.is_empty() {
                return Err("No UVD group selected to delete".into());
            }
            for &gi in indices.iter().rev() {
                if gi >= uvd.groups.len() {
                    return Err(format!("UVD group index #{gi} out of range"));
                }
                uvd.groups.remove(gi);
            }
            // Note: WTA texture slots are NOT reordered or deleted when UVD groups are deleted.
            format!("Delete {} UVD group(s)", indices.len())
        }
        "rename_group" => {
            let gi = payload
                .get("group_index")
                .and_then(|v| v.as_u64())
                .map(|v| v as usize)
                .or(leaf_grp_idx)
                .ok_or("Missing UVD group index to rename")?;
            if gi >= uvd.groups.len() {
                return Err("UVD group index out of range".into());
            }
            let raw_name = payload
                .get("new_name")
                .and_then(|v| v.as_str())
                .ok_or("Missing new group name")?;
            let valid_name = validate_uvd_group_name(raw_name).map_err(|e| e.to_string())?;
            if let Some(collision) =
                uvd.check_name_crc_collision(&valid_name, true, Some(gi), None)
            {
                return Err(collision);
            }
            let old_name = uvd.groups[gi].name.clone();
            let old_id = uvd.groups[gi].id;
            let new_id = compute_uvd_group_id(&valid_name);

            uvd.groups[gi].name = valid_name.clone();
            uvd.groups[gi].id = new_id;
            for it in &mut uvd.groups[gi].items {
                it.group_id = new_id;
            }

            // Also update matching WTA texture entry's texture_id so group <-> texture ID link stays intact (without reordering WTA slots)
            let mut banks = collect_sibling_wta_banks(&cur_snap, &container_parts, oodle_opt);
            if let Some((bi, ei)) = find_wta_texture_by_id(&banks, old_id) {
                let bank = &mut banks[bi];
                bank.wtb.entries[ei].texture_id = Some(new_id);
                let (new_wta, new_wtp) = bank.wtb.to_bytes();
                let c_refs: Vec<&str> = bank.container_parts.iter().map(|s| s.as_str()).collect();
                cur_snap = write_back_subblob_with_align(
                    &cur_snap,
                    &c_refs,
                    new_wta,
                    new_wtp,
                    oodle_opt,
                    dat_align,
                )?;
                if let Some(mut pending) = next_pending.remove(&(bank.container_id.clone(), old_id))
                {
                    pending.texture_id = new_id;
                    pending.texture_id_hex = format!("0x{new_id:08X}");
                    pending.group_name = valid_name.clone();
                    next_pending.insert((bank.container_id.clone(), new_id), pending);
                }
            }

            format!(
                "Rename UVD group '{}' -> '{}' (0x{:08X} -> 0x{:08X}; warning: UID references to old ID may break)",
                old_name, valid_name, old_id, new_id
            )
        }
        "reorder_groups" => {
            let mut indices: Vec<usize> = payload
                .get("group_indices")
                .and_then(|v| serde_json::from_value(v.clone()).ok())
                .unwrap_or_default();
            if indices.is_empty() {
                if let Some(gi) = leaf_grp_idx {
                    indices.push(gi);
                }
            }
            indices.sort_unstable();
            indices.dedup();
            if indices.is_empty() {
                return Err("No UVD group selected to reorder".into());
            }
            if indices.len() == 1 {
                let gi = indices[0];
                let to_idx = compute_reorder_target(gi, uvd.groups.len(), &payload)?;
                let grp = uvd.groups.remove(gi);
                uvd.groups.insert(to_idx, grp);
                format!("Reorder UVD group #{gi} -> #{to_idx}")
            } else {
                let dir = payload
                    .get("direction")
                    .and_then(|v| v.as_str())
                    .unwrap_or("up");
                if dir == "up" {
                    if indices[0] == 0 {
                        return Err("Selected groups are already at top".into());
                    }
                    for &gi in &indices {
                        uvd.groups.swap(gi - 1, gi);
                    }
                } else {
                    if *indices.last().unwrap() + 1 >= uvd.groups.len() {
                        return Err("Selected groups are already at bottom".into());
                    }
                    for &gi in indices.iter().rev() {
                        uvd.groups.swap(gi, gi + 1);
                    }
                }
                format!("Reorder {} UVD groups ({dir})", indices.len())
            }
        }
        "add_item" => {
            let gi = payload
                .get("group_index")
                .and_then(|v| v.as_u64())
                .map(|v| v as usize)
                .or(leaf_grp_idx)
                .ok_or("Add sprite is only available on a UVD Group")?;
            if gi >= uvd.groups.len() {
                return Err("UVD group index out of range".into());
            }
            let raw_name = payload
                .get("name")
                .and_then(|v| v.as_str())
                .ok_or("Missing new sprite name")?;
            let auto_suffix = payload
                .get("auto_suffix")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let item_name = if auto_suffix {
                uvd.auto_suffix_unique_name(raw_name, false)
                    .map_err(|e| e.to_string())?
            } else {
                let valid = validate_uvd_item_name(raw_name).map_err(|e| e.to_string())?;
                if let Some(collision) = uvd.check_name_crc_collision(&valid, false, None, None) {
                    return Err(collision);
                }
                valid
            };
            let file_path = payload
                .get("path")
                .and_then(|v| v.as_str())
                .ok_or("Missing sprite image path")?;
            let raw_img = fs::read(file_path).map_err(|e| e.to_string())?;
            let new_rgba = image::load_from_memory(&raw_img)
                .map_err(|e| format!("Failed to decode sprite image: {e}"))?
                .to_rgba8();

            let banks = collect_sibling_wta_banks(&cur_snap, &container_parts, oodle_opt);
            let group = &uvd.groups[gi];
            let group_name = group.name.clone();
            let group_id = group.id;
            let preferred_dims = group.inferred_dimensions();
            let mut sprites = extract_or_init_group_sprites(group, &banks, &next_pending);
            let new_item_id = compute_uvd_item_id(&item_name);
            sprites.push((item_name.clone(), new_item_id, new_rgba));

            let packed = pack_group_sprites(
                &group_name,
                group_id,
                &sprites,
                uvd_padding,
                uvd_max_size,
                preferred_dims,
            )
            .map_err(|e| e.to_string())?;

            uvd.groups[gi] = packed.group;
            cur_snap = apply_repacked_sheet_to_wta(
                &cur_snap,
                &container_parts,
                &uvd_name,
                &uvd,
                &group_name,
                group_id,
                &packed.sheet,
                true,
                oodle_opt,
                nvtt_opt,
                dat_align,
                &mut next_pending,
            )?;

            format!(
                "Add sprite '{}' (0x{:08X}) to group '{}' ({}×{})",
                item_name, new_item_id, group_name, packed.width, packed.height
            )
        }
        "replace_item" => {
            let gi = payload
                .get("group_index")
                .and_then(|v| v.as_u64())
                .map(|v| v as usize)
                .or(leaf_grp_idx)
                .ok_or("Missing group index for replace_item")?;
            let ii = payload
                .get("item_index")
                .and_then(|v| v.as_u64())
                .map(|v| v as usize)
                .or(leaf_item_idx)
                .ok_or("Missing item index for replace_item")?;
            if gi >= uvd.groups.len() || ii >= uvd.groups[gi].items.len() {
                return Err("UVD group or item index out of range".into());
            }
            let file_path = payload
                .get("path")
                .and_then(|v| v.as_str())
                .ok_or("Missing replacement sprite file path")?;
            let raw_img = fs::read(file_path).map_err(|e| e.to_string())?;
            let new_rgba = image::load_from_memory(&raw_img)
                .map_err(|e| format!("Failed to decode replacement sprite: {e}"))?
                .to_rgba8();

            let banks = collect_sibling_wta_banks(&cur_snap, &container_parts, oodle_opt);
            let group = &uvd.groups[gi];
            let group_name = group.name.clone();
            let group_id = group.id;
            let item_name = group.items[ii].name.clone();
            let preferred_dims = group.inferred_dimensions();

            let mut sprites = extract_or_init_group_sprites(group, &banks, &next_pending);
            sprites[ii].2 = new_rgba;

            let packed = pack_group_sprites(
                &group_name,
                group_id,
                &sprites,
                uvd_padding,
                uvd_max_size,
                preferred_dims,
            )
            .map_err(|e| e.to_string())?;

            uvd.groups[gi] = packed.group;
            cur_snap = apply_repacked_sheet_to_wta(
                &cur_snap,
                &container_parts,
                &uvd_name,
                &uvd,
                &group_name,
                group_id,
                &packed.sheet,
                true,
                oodle_opt,
                nvtt_opt,
                dat_align,
                &mut next_pending,
            )?;

            format!(
                "Replace sprite '{}' in group '{}' (repacked to {}×{})",
                item_name, group_name, packed.width, packed.height
            )
        }
        "delete_items" => {
            // Support multi-select items across one or more groups (`items: [(group_idx, item_idx)]` or `item_indices` in `leaf_grp_idx`)
            let mut by_group: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
            if let Some(pairs) = payload
                .get("items")
                .and_then(|v| serde_json::from_value::<Vec<(usize, usize)>>(v.clone()).ok())
            {
                for (gi, ii) in pairs {
                    by_group.entry(gi).or_default().push(ii);
                }
            } else if let Some(gi) = payload
                .get("group_index")
                .and_then(|v| v.as_u64())
                .map(|v| v as usize)
                .or(leaf_grp_idx)
            {
                let mut i_list: Vec<usize> = payload
                    .get("item_indices")
                    .and_then(|v| serde_json::from_value(v.clone()).ok())
                    .unwrap_or_default();
                if i_list.is_empty() {
                    if let Some(ii) = leaf_item_idx {
                        i_list.push(ii);
                    }
                }
                if !i_list.is_empty() {
                    by_group.insert(gi, i_list);
                }
            }
            if by_group.is_empty() {
                return Err("No UVD sprite item(s) selected to delete".into());
            }

            let banks = collect_sibling_wta_banks(&cur_snap, &container_parts, oodle_opt);
            let mut total_deleted = 0usize;
            for (gi, mut item_indices) in by_group {
                if gi >= uvd.groups.len() {
                    return Err(format!("UVD group #{gi} out of range"));
                }
                item_indices.sort_unstable();
                item_indices.dedup();
                let group = &uvd.groups[gi];
                let group_name = group.name.clone();
                let group_id = group.id;
                let preferred_dims = group.inferred_dimensions();
                let mut sprites = extract_or_init_group_sprites(group, &banks, &next_pending);
                for &ii in item_indices.iter().rev() {
                    if ii >= sprites.len() {
                        return Err(format!("UVD item #{ii} out of range"));
                    }
                    sprites.remove(ii);
                    total_deleted += 1;
                }
                let packed = pack_group_sprites(
                    &group_name,
                    group_id,
                    &sprites,
                    uvd_padding,
                    uvd_max_size,
                    preferred_dims,
                )
                .map_err(|e| e.to_string())?;
                uvd.groups[gi] = packed.group;
                cur_snap = apply_repacked_sheet_to_wta(
                    &cur_snap,
                    &container_parts,
                    &uvd_name,
                    &uvd,
                    &group_name,
                    group_id,
                    &packed.sheet,
                    false,
                    oodle_opt,
                    nvtt_opt,
                    dat_align,
                    &mut next_pending,
                )?;
            }
            format!("Delete {total_deleted} UVD sprite(s) and repack affected group(s)")
        }
        "rename_item" => {
            let gi = payload
                .get("group_index")
                .and_then(|v| v.as_u64())
                .map(|v| v as usize)
                .or(leaf_grp_idx)
                .ok_or("Missing group index for rename_item")?;
            let ii = payload
                .get("item_index")
                .and_then(|v| v.as_u64())
                .map(|v| v as usize)
                .or(leaf_item_idx)
                .ok_or("Missing item index for rename_item")?;
            if gi >= uvd.groups.len() || ii >= uvd.groups[gi].items.len() {
                return Err("UVD group or item index out of range".into());
            }
            let raw_name = payload
                .get("new_name")
                .and_then(|v| v.as_str())
                .ok_or("Missing new sprite name")?;
            let valid_name = validate_uvd_item_name(raw_name).map_err(|e| e.to_string())?;
            if let Some(collision) =
                uvd.check_name_crc_collision(&valid_name, false, None, Some((gi, ii)))
            {
                return Err(collision);
            }

            let old_name = uvd.groups[gi].items[ii].name.clone();
            let old_id = uvd.groups[gi].items[ii].id;
            let new_id = compute_uvd_item_id(&valid_name);
            uvd.groups[gi].items[ii].name = valid_name.clone();
            uvd.groups[gi].items[ii].id = new_id;

            format!(
                "Rename sprite '{}' -> '{}' (0x{:08X} -> 0x{:08X}; warning: UID references to old ID may break)",
                old_name, valid_name, old_id, new_id
            )
        }
        "reorder_items" => {
            let gi = payload
                .get("group_index")
                .and_then(|v| v.as_u64())
                .map(|v| v as usize)
                .or(leaf_grp_idx)
                .ok_or("Missing group index for reorder_items")?;
            if gi >= uvd.groups.len() {
                return Err("UVD group index out of range".into());
            }
            let mut indices: Vec<usize> = payload
                .get("item_indices")
                .and_then(|v| serde_json::from_value(v.clone()).ok())
                .unwrap_or_default();
            if indices.is_empty() {
                if let Some(ii) = leaf_item_idx {
                    indices.push(ii);
                }
            }
            indices.sort_unstable();
            indices.dedup();
            if indices.is_empty() {
                return Err("No UVD item(s) selected to reorder".into());
            }

            let banks = collect_sibling_wta_banks(&cur_snap, &container_parts, oodle_opt);
            let group = &uvd.groups[gi];
            let group_name = group.name.clone();
            let group_id = group.id;
            let preferred_dims = group.inferred_dimensions();
            let mut sprites = extract_or_init_group_sprites(group, &banks, &next_pending);

            if indices.len() == 1 {
                let ii = indices[0];
                let to_idx = compute_reorder_target(ii, sprites.len(), &payload)?;
                let item = sprites.remove(ii);
                sprites.insert(to_idx, item);
            } else {
                let dir = payload
                    .get("direction")
                    .and_then(|v| v.as_str())
                    .unwrap_or("up");
                if dir == "up" {
                    if indices[0] == 0 {
                        return Err("Selected items are already at top".into());
                    }
                    for &ii in &indices {
                        sprites.swap(ii - 1, ii);
                    }
                } else {
                    if *indices.last().unwrap() + 1 >= sprites.len() {
                        return Err("Selected items are already at bottom".into());
                    }
                    for &ii in indices.iter().rev() {
                        sprites.swap(ii, ii + 1);
                    }
                }
            }

            let packed = pack_group_sprites(
                &group_name,
                group_id,
                &sprites,
                uvd_padding,
                uvd_max_size,
                preferred_dims,
            )
            .map_err(|e| e.to_string())?;
            uvd.groups[gi] = packed.group;
            cur_snap = apply_repacked_sheet_to_wta(
                &cur_snap,
                &container_parts,
                &uvd_name,
                &uvd,
                &group_name,
                group_id,
                &packed.sheet,
                false,
                oodle_opt,
                nvtt_opt,
                dat_align,
                &mut next_pending,
            )?;

            format!("Reorder {} sprite(s) in group '{}'", indices.len(), group_name)
        }
        "repack_group" => {
            let gi = payload
                .get("group_index")
                .and_then(|v| v.as_u64())
                .map(|v| v as usize)
                .or(leaf_grp_idx)
                .ok_or("Missing group index for repack_group")?;
            if gi >= uvd.groups.len() {
                return Err("UVD group index out of range".into());
            }
            let banks = collect_sibling_wta_banks(&cur_snap, &container_parts, oodle_opt);
            let group = &uvd.groups[gi];
            let group_name = group.name.clone();
            let group_id = group.id;
            let preferred_dims = group.inferred_dimensions();
            let sprites = extract_or_init_group_sprites(group, &banks, &next_pending);
            let packed = pack_group_sprites(
                &group_name,
                group_id,
                &sprites,
                uvd_padding,
                uvd_max_size,
                preferred_dims,
            )
            .map_err(|e| e.to_string())?;
            uvd.groups[gi] = packed.group;
            cur_snap = apply_repacked_sheet_to_wta(
                &cur_snap,
                &container_parts,
                &uvd_name,
                &uvd,
                &group_name,
                group_id,
                &packed.sheet,
                true,
                oodle_opt,
                nvtt_opt,
                dat_align,
                &mut next_pending,
            )?;
            format!(
                "Repack group '{}' ({}×{}, padding {}px)",
                group_name, packed.width, packed.height, uvd_padding
            )
        }
        "swap_endian" => {
            uvd.endian = uvd.endian.swap();
            format!("Swap UVD endianness to {:?}", uvd.endian)
        }
        other => return Err(format!("Unsupported UVD mutation action: {other}")),
    };

    let new_uvd_bytes = uvd.to_bytes();
    let final_snap = write_back_subblob_with_align(
        &cur_snap,
        &container_parts,
        new_uvd_bytes,
        None,
        oodle_opt,
        dat_align,
    )?;

    sess.pending_uvd_sheets = next_pending;
    sess.invalidate_texture_caches();
    record_mutation(&mut sess, desc, final_snap);
    build_outline_from_session(&sess)
}

#[tauri::command]
pub fn extract_uvd_action(
    node_ids: Vec<String>,
    dest_path: String,
    write_json: bool,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let sess = lock_session(&state);
    let snap = sess.current.as_ref().ok_or("No active document loaded")?;
    let oodle_opt = sess.settings.oodle_path();

    if node_ids.is_empty() {
        return Err("No UVD node selected to extract".into());
    }

    let first_id = &node_ids[0];
    let (container_parts, first_leaf) = split_container_and_leaf(first_id);
    let container_id = container_parts.join("/");
    let (uvd_name, uvd_bytes, _, _) = resolve_subblob(snap, &container_id, oodle_opt)?;
    let uvd = UvdFile::from_bytes(&uvd_bytes).map_err(|e| e.to_string())?;
    let banks = collect_sibling_wta_banks(snap, &container_parts, oodle_opt);

    let decode_sheet_by_id = |tex_id: u32, label: &str| -> Result<RgbaImage, String> {
        let Some((bi, ei)) = find_wta_texture_by_id(&banks, tex_id) else {
            return Err(format!(
                "Texture payload missing: no WTA texture with ID 0x{:08X} found for '{}'",
                tex_id, label
            ));
        };
        let bank = &banks[bi];
        if let Some(pending) = sess.pending_uvd_sheets.get(&(bank.container_id.clone(), tex_id)) {
            if let Some(ref lossless) = pending.lossless_sheet {
                return Ok((**lossless).clone());
            }
        }
        let entry = &bank.wtb.entries[ei];
        if !entry.has_pixel_payload() {
            return Err(format!(
                "Texture payload missing: matched WTA texture for '{}' (0x{:08X} in '{}') has no pixel data",
                label, tex_id, bank.wta_name
            ));
        }
        let decoded = if bank.is_standalone_bntx {
            decode_bntx_subtexture_mip_filtered(&entry.data, ei, 0, "RGBA", false)
        } else {
            decode_texture_mip_filtered(&entry.data, 0, "RGBA", false)
        };
        decoded
            .map(|(img, _, _)| img)
            .map_err(|e| format!("Failed to decode texture for '{}': {e}", label))
    };

    let (_, first_item_idx) = parse_uvd_leaf(first_leaf);

    // Single item extract to a `.png` file path (always crops from texture whose ID == item.group_id)
    if node_ids.len() == 1 && first_item_idx.is_some() && dest_path.to_ascii_lowercase().ends_with(".png") {
        let (Some(gi), Some(ii)) = parse_uvd_leaf(first_leaf) else {
            return Err("Invalid UVD item node ID".into());
        };
        let group = uvd.groups.get(gi).ok_or("UVD group out of range")?;
        let item = group.items.get(ii).ok_or("UVD item out of range")?;
        let sheet = decode_sheet_by_id(item.group_id, &item.name)?;
        let cropped = crop_sprite_from_sheet(&sheet, item);
        let out_file = PathBuf::from(&dest_path);
        if let Some(p) = out_file.parent() {
            fs::create_dir_all(p).map_err(|e| e.to_string())?;
        }
        cropped.save(&out_file).map_err(|e| e.to_string())?;
        return Ok(format!(
            "Extracted sprite '{}' ({}×{}) to {}",
            item.name,
            cropped.width(),
            cropped.height(),
            out_file.display()
        ));
    }

    let out_root = PathBuf::from(&dest_path);
    fs::create_dir_all(&out_root).map_err(|e| e.to_string())?;

    let mut groups_to_extract: BTreeSet<usize> = BTreeSet::new();
    let mut items_to_extract: BTreeMap<usize, BTreeSet<usize>> = BTreeMap::new();

    for nid in &node_ids {
        let (_, leaf) = split_container_and_leaf(nid);
        match parse_uvd_leaf(leaf) {
            (None, None) => {
                for gi in 0..uvd.groups.len() {
                    groups_to_extract.insert(gi);
                }
            }
            (Some(gi), None) => {
                groups_to_extract.insert(gi);
            }
            (Some(gi), Some(ii)) => {
                items_to_extract.entry(gi).or_default().insert(ii);
            }
            _ => {}
        }
    }

    let mut total_groups = 0usize;
    let mut total_sprites = 0usize;

    for &gi in &groups_to_extract {
        let group = uvd.groups.get(gi).ok_or("UVD group out of range")?;
        let sheet = decode_sheet_by_id(group.id, &group.name)?;
        let (_, count) = extract_group_to_dir(group, &sheet, &out_root).map_err(|e| e.to_string())?;
        total_groups += 1;
        total_sprites += count;
    }

    for (gi, item_set) in items_to_extract {
        if groups_to_extract.contains(&gi) {
            continue;
        }
        let group = uvd.groups.get(gi).ok_or("UVD group out of range")?;
        let clean_group = validate_single_entry_name(&group.name)
            .unwrap_or_else(|_| format!("group_0x{:08X}", group.id));
        let group_dir = out_root.join(clean_group);
        fs::create_dir_all(&group_dir).map_err(|e| e.to_string())?;
        let mut sheet_cache: BTreeMap<u32, RgbaImage> = BTreeMap::new();
        for ii in item_set {
            let item = group.items.get(ii).ok_or("UVD item out of range")?;
            if !sheet_cache.contains_key(&item.group_id) {
                let s = decode_sheet_by_id(item.group_id, &item.name)?;
                sheet_cache.insert(item.group_id, s);
            }
            let sheet = sheet_cache.get(&item.group_id).unwrap();
            let clean_item = validate_single_entry_name(&item.name)
                .unwrap_or_else(|_| format!("item_0x{:08X}", item.id));
            let sprite = crop_sprite_from_sheet(sheet, item);
            sprite
                .save(group_dir.join(format!("{clean_item}.png")))
                .map_err(|e| e.to_string())?;
            total_sprites += 1;
        }
        total_groups += 1;
    }

    if write_json {
        let uvd_stem = Path::new(&uvd_name)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("atlas");
        let json_path = out_root.join(format!("{uvd_stem}.uvd.json"));
        let json_str = uvd.to_json_pretty().map_err(|e| e.to_string())?;
        fs::write(&json_path, json_str).map_err(|e| e.to_string())?;
    }

    Ok(format!(
        "Extracted {total_sprites} sprite(s) across {total_groups} group(s){} to {}",
        if write_json { " (+ .uvd.json)" } else { "" },
        out_root.display()
    ))
}


/// Re-encode all pending UVD group textures in `pending_list` at save time using the user-selected
/// `SaveEncodeOptions` from the Save-Time Texture Encode modal.
pub(crate) fn reencode_pending_uvd_sheets_for_save(
    snap: &DocSnapshot,
    pending_list: &[PendingUvdSheetInfo],
    opts: &SaveEncodeOptions,
    oodle_opt: Option<&Path>,
    nvtt_opt: Option<&Path>,
    dat_align: Option<usize>,
) -> Result<DocSnapshot, String> {
    let mut cur_snap = snap.clone();
    let quality = EncodeQuality::from_str_name(&opts.quality);

    for item in pending_list {
        let (c_parts, _) = split_container_and_leaf(&item.wta_container_id);
        let cid = c_parts.join("/");
        let Ok((_, wta_bytes, wtp_bytes, _)) = resolve_subblob(&cur_snap, &cid, oodle_opt) else {
            continue;
        };
        let is_bntx = is_bntx_blob(&wta_bytes);
        let Ok(mut wtb) = WtbFile::from_bytes(&wta_bytes, wtp_bytes.as_deref()) else {
            continue;
        };
        let game_profile = wtb.game_profile;
        let Some(ei) = wtb
            .entries
            .iter()
            .position(|e| e.texture_id == Some(item.texture_id))
        else {
            continue;
        };
        let entry = &mut wtb.entries[ei];
        let sheet_img: RgbaImage = if let Some(ref lossless) = item.lossless_sheet {
            (**lossless).clone()
        } else {
            if !entry.has_pixel_payload() {
                continue;
            }
            let decoded = if is_bntx {
                decode_bntx_subtexture_mip_filtered(&entry.data, ei, 0, "RGBA", false)
            } else {
                decode_texture_mip_filtered(&entry.data, 0, "RGBA", false)
            };
            let Ok((img, _, _)) = decoded else {
                continue;
            };
            img
        };

        let fallback_fmt = parse_requested_texture_format(
            &item.original_format,
            default_format_for_profile(game_profile),
        );
        let target_fmt = parse_requested_texture_format(&opts.encode_format, fallback_fmt);
        let prev_bntx_name = entry.metadata.as_ref().and_then(|m| m.name.clone());

        let new_blob = if entry.extension == ".bntx" {
            replace_bntx_subtexture_with_options(
                &entry.data,
                ei,
                Err(&sheet_img),
                Some(target_fmt),
                opts.generate_mips,
                opts.solidify,
                opts.make_opaque,
                quality,
                nvtt_opt,
            )
            .map_err(|e| e.to_string())?
        } else {
            encode_image_for_bank_entry(
                &sheet_img,
                &entry.extension,
                target_fmt,
                if opts.generate_mips { None } else { Some(1) },
                opts.generate_mips,
                opts.solidify,
                opts.make_opaque,
                quality,
                Some(&entry.data),
                nvtt_opt,
            )
        };

        entry.metadata = inspect_texture_blob(&new_blob).ok();
        if let Some(ref mut m) = entry.metadata {
            if m.name.is_none() {
                m.name = prev_bntx_name;
            }
        }
        entry.data = new_blob.clone();
        if game_profile == WtbGameProfile::SwitchXt1 && opts.sync_flags {
            entry.sync_xt1_flags_from_metadata(game_profile);
        }

        let (new_wta, new_wtp) = if is_bntx {
            (new_blob, None)
        } else {
            wtb.to_bytes()
        };
        cur_snap = write_back_subblob_with_align(
            &cur_snap,
            &c_parts,
            new_wta,
            new_wtp,
            oodle_opt,
            dat_align,
        )?;
    }

    Ok(cur_snap)
}
