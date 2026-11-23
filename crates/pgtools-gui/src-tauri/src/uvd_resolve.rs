use crate::common::DocSnapshot;
use crate::vfs::{detect_file_format_tag, is_bntx_blob, resolve_subblob, split_container_and_leaf};
use pgtools_core::{dat::DatFile, uvd::UvdFile, wtb::WtbFile};
use std::path::Path;

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

fn resolve_uvd_group_to_wta_tex_node(
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

pub(crate) fn resolve_effective_texture_node(
    snap: &DocSnapshot,
    node_id: &str,
    oodle_opt: Option<&Path>,
) -> String {
    resolve_uvd_group_to_wta_tex_node(snap, node_id, oodle_opt)
        .unwrap_or_else(|| node_id.to_string())
}
