use crate::common::{AssetTreeNode, DocSnapshot, DocumentOutline, PendingUvdSheetInfo};
use pgtools_core::{
    dat::DatFile,
    eff::EffFile,
    io::sanitize_entry_path,
    pkz::{normalize_pkz_virtual_path, OodleLibrary, PkzArchive},
    scr::ScrFile,
    uvd::UvdFile,
    wmb::WmbFile,
    wmb3::Wmb3File,
    wtb::WtbFile,
};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub fn is_supported_extension(ext: &str) -> bool {
    matches!(
        ext.to_ascii_lowercase().as_str(),
        "dat"
            | "dtt"
            | "evn"
            | "eff"
            | "idd"
            | "pkz"
            | "scr"
            | "wtb"
            | "wta"
            | "wtp"
            | "wmb"
            | "mdb"
            | "dds"
            | "gtx"
            | "xt1"
            | "bntx"
            | "bxm"
            | "xml"
            | "gad"
            | "clp"
            | "clh"
            | "clw"
            | "mot"
            | "exp"
            | "est"
            | "cos"
            | "att"
            | "obi"
            | "msq"
            | "col"
            | "fls"
            | "fla"
            | "cta"
            | "lit"
            | "csv"
            | "uvd"
    )
}

pub(crate) fn is_bntx_blob(bytes: &[u8]) -> bool {
    (bytes.len() >= 4 && &bytes[0..4] == b"BNTX")
        || (bytes.len() >= 84 && &bytes[80..84] == b"BNTX")
}

pub(crate) fn parse_eff_entry_id(fname: &str) -> u32 {
    let stem = fname.split('.').next().unwrap_or("");
    if let Some(hex) = stem.strip_prefix("0x").or_else(|| stem.strip_prefix("0X")) {
        u32::from_str_radix(hex, 16).unwrap_or(0)
    } else {
        stem.parse::<u32>()
            .or_else(|_| u32::from_str_radix(stem, 16))
            .unwrap_or(0)
    }
}

pub(crate) fn find_pkz_peer_dtt_index(pkz: &PkzArchive, entry_name: &str) -> Option<usize> {
    let norm = entry_name.replace('\\', "/");
    let trimmed = norm.trim_start_matches('/');
    if !trimmed.to_ascii_lowercase().ends_with(".dat") || trimmed.len() < 4 {
        return None;
    }
    let stem = &trimmed[..trimmed.len() - 4];
    let target_dtt = format!("{stem}.dtt");
    pkz.descriptors.iter().position(|d| {
        d.name
            .replace('\\', "/")
            .trim_start_matches('/')
            .eq_ignore_ascii_case(&target_dtt)
    })
}

pub(crate) fn find_pkz_peer_dat_index(pkz: &PkzArchive, entry_name: &str) -> Option<usize> {
    let norm = entry_name.replace('\\', "/");
    let trimmed = norm.trim_start_matches('/');
    if !trimmed.to_ascii_lowercase().ends_with(".dtt") || trimmed.len() < 4 {
        return None;
    }
    let stem = &trimmed[..trimmed.len() - 4];
    let target_dat = format!("{stem}.dat");
    pkz.descriptors.iter().position(|d| {
        d.name
            .replace('\\', "/")
            .trim_start_matches('/')
            .eq_ignore_ascii_case(&target_dat)
    })
}

/// Compare a current `PkzArchive` against its saved baseline bytes (if any).
/// Returns `(entry_index -> is_modified, total_staged_change_count)`.
pub(crate) fn compute_pkz_modified_map(
    cur_bytes: &[u8],
    cur_pkz: &PkzArchive,
    base_bytes: Option<&[u8]>,
) -> (HashMap<usize, bool>, usize) {
    let Some(bb) = base_bytes else {
        return (HashMap::new(), 0);
    };
    let Ok(base_pkz) = PkzArchive::from_bytes(bb) else {
        return (HashMap::new(), 0);
    };

    let mut base_by_key: HashMap<String, (usize, &pgtools_core::pkz::PkzFileDescriptor)> =
        HashMap::with_capacity(base_pkz.descriptors.len());
    for (bi, bd) in base_pkz.descriptors.iter().enumerate() {
        let key = bd
            .name
            .replace('\\', "/")
            .trim_start_matches('/')
            .to_ascii_lowercase();
        base_by_key.entry(key).or_insert((bi, bd));
    }

    let mut seen_base_keys: HashSet<String> = HashSet::with_capacity(cur_pkz.descriptors.len());
    let mut mod_map: HashMap<usize, bool> = HashMap::with_capacity(cur_pkz.descriptors.len());
    let mut modified_count = 0usize;

    for (ci, cd) in cur_pkz.descriptors.iter().enumerate() {
        let key = cd
            .name
            .replace('\\', "/")
            .trim_start_matches('/')
            .to_ascii_lowercase();
        seen_base_keys.insert(key.clone());

        let is_mod = match base_by_key.get(&key) {
            None => true,
            Some(&(bi, bd)) => {
                let cur_slice = cur_pkz.raw_entry_slice(cur_bytes, ci).unwrap_or(&[]);
                let base_slice = base_pkz.raw_entry_slice(bb, bi).unwrap_or(&[]);
                cd.name != bd.name
                    || !cd.compression.eq_ignore_ascii_case(&bd.compression)
                    || cd.size != bd.size
                    || cd.compressed_size != bd.compressed_size
                    || cur_slice != base_slice
            }
        };
        if is_mod {
            modified_count += 1;
        }
        mod_map.insert(ci, is_mod);
    }

    let deleted_count = base_by_key
        .keys()
        .filter(|k| !seen_base_keys.contains(*k))
        .count();

    (mod_map, modified_count + deleted_count)
}

pub(crate) fn detect_file_format_tag(name: &str, bytes: &[u8]) -> &'static str {
    let ext = Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if ext == "csv" {
        return "CSV";
    }
    if bytes.len() >= 4 {
        match &bytes[0..4] {
            b"DAT\0" | b"\0TAD" => return "DAT",
            b"pkzl" | b"PKZL" | b"PKZB" => return "PKZ",
            b"EF2\0" | b"IDD\0" => return "EFF",
            b"WTB\0" | b"\0BTW" => return "WTB",
            b"WMB\0" | b"\0BMW" => return "WMB",
            b"WMB3" | b"3BMW" => return "WMB3",
            b"SCR\0" | b"\0RCS" => return "SCR",
            b"BXM\0" | b"XML\0" => return "BXM",
            b"MOT\0" | b"\0TOM" | b"mot\0" | b"\0tom" => return "MOT",
            b"EXP\0" | b"\0PXE" | b"exp\0" => return "EXP",
            b"EST\0" | b"\0TSE" => return "EST",
            b"FLS\0" => return "FLS",
            b"XAD\0" => return "XAD",
            b"LIT\0" => return "LIT",
            b"DDS " => return "DDS",
            b"Gfx2" => return "GTX",
            b"XT1\0" => return "XT1",
            b"BNTX" => return "BNTX",
            _ => {}
        }
    }
    if bytes.len() >= 2 && &bytes[0..2] == b"JK" && ext == "mdb" {
        return "MDB";
    }
    if bytes.len() >= 84 && &bytes[80..84] == b"BNTX" {
        return "BNTX";
    }
    match ext.as_str() {
        "dat" | "dtt" | "evn" => "DAT",
        "pkz" => "PKZ",
        "eff" | "idd" => "EFF",
        "wtb" | "wta" => "WTB",
        "wtp" => {
            if bytes.len() >= 4 && &bytes[0..4] == b"BNTX" {
                "BNTX"
            } else {
                "WTP"
            }
        }
        "wmb" => "WMB",
        "mdb" => "MDB",
        "scr" => "SCR",
        "bxm" | "xml" | "gad" => "BXM",
        "clp" => "CLP",
        "clh" => "CLH",
        "clw" => "CLW",
        "mot" => "MOT",
        "exp" => "EXP",
        "est" => "EST",
        "cos" => "COS",
        "att" => "ATT",
        "obi" => "OBI",
        "msq" => "MSQ",
        "col" => "COL",
        "fls" => "FLS",
        "fla" | "cta" => "XAD",
        "lit" => "LIT",
        "csv" => "CSV",
        "uvd" => "UVD",
        _ => "BIN",
    }
}

/// Match `.wta` in `dat` with `.wtp` in `dat` or `dtt` using `XT1 Studio` `BankDocument::MatchPayload` logic:
/// 1. Same filename stem (e.g. `pl0000.wta` <-> `pl0000.wtp`)
/// 2. Single `.wta` and single `.wtp` across the pair
/// 3. Same ordinal among `.wta` and `.wtp` entries
pub(crate) fn find_paired_wtp_bytes(
    wta_name: &str,
    wta_ordinal: usize,
    total_wtas: usize,
    dat_entries: &[pgtools_core::dat::DatEntry],
    dtt_entries: Option<&[pgtools_core::dat::DatEntry]>,
) -> Option<Vec<u8>> {
    let wta_stem = Path::new(wta_name)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("");

    let mut all_wtps: Vec<(&str, &[u8])> = Vec::new();
    for e in dat_entries {
        if e.extension.eq_ignore_ascii_case("wtp") {
            all_wtps.push((&e.name, &e.data));
        }
    }
    if let Some(dtt) = dtt_entries {
        for e in dtt {
            if e.extension.eq_ignore_ascii_case("wtp") {
                all_wtps.push((&e.name, &e.data));
            }
        }
    }

    if all_wtps.is_empty() {
        return None;
    }

    for (name, data) in &all_wtps {
        let stem = Path::new(name)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("");
        if !wta_stem.is_empty() && stem.eq_ignore_ascii_case(wta_stem) {
            return Some(data.to_vec());
        }
    }

    if total_wtas == 1 && all_wtps.len() == 1 {
        return Some(all_wtps[0].1.to_vec());
    }

    all_wtps.get(wta_ordinal).map(|(_, d)| d.to_vec())
}

/// Build the recursive tree outline for any bytes buffer at `id_prefix`.
pub(crate) fn build_nodes_for_blob(
    id_prefix: &str,
    name: &str,
    bytes: &[u8],
    sibling_wtp: Option<&[u8]>,
    sibling_dtt_bytes: Option<&[u8]>,
    oodle_dll: Option<&Path>,
    depth: usize,
) -> AssetTreeNode {
    build_nodes_for_blob_with_baseline(
        id_prefix,
        name,
        bytes,
        None,
        sibling_wtp,
        sibling_dtt_bytes,
        oodle_dll,
        depth,
    )
}

#[allow(clippy::too_many_arguments)]
fn build_nodes_for_blob_with_baseline(
    id_prefix: &str,
    name: &str,
    bytes: &[u8],
    baseline_bytes: Option<&[u8]>,
    sibling_wtp: Option<&[u8]>,
    sibling_dtt_bytes: Option<&[u8]>,
    oodle_dll: Option<&Path>,
    depth: usize,
) -> AssetTreeNode {
    let tag = detect_file_format_tag(name, bytes);
    let mut children = Vec::new();
    let mut node_modified = false;

    if depth < 6 {
        match tag {
            "DAT" => {
                if let Ok(dat) = DatFile::from_bytes(bytes) {
                    let base_dat = baseline_bytes.and_then(|bb| DatFile::from_bytes(bb).ok());
                    let parsed_dtt = sibling_dtt_bytes.and_then(|b| DatFile::from_bytes(b).ok());
                    let dtt_slice = parsed_dtt.as_ref().map(|d| d.entries.as_slice());
                    let total_wtas = dat
                        .entries
                        .iter()
                        .filter(|e| e.extension.eq_ignore_ascii_case("wta"))
                        .count();
                    let mut wta_ord = 0usize;

                    for (idx, entry) in dat.entries.iter().enumerate() {
                        let child_id = if id_prefix.is_empty() {
                            format!("dat:{idx}")
                        } else {
                            format!("{id_prefix}/dat:{idx}")
                        };
                        let ext_low = entry.extension.to_ascii_lowercase();
                        let pair_wtp = if ext_low == "wta" {
                            let matched = find_paired_wtp_bytes(
                                &entry.name,
                                wta_ord,
                                total_wtas,
                                &dat.entries,
                                dtt_slice,
                            );
                            wta_ord += 1;
                            matched
                        } else {
                            None
                        };
                        let child_base = base_dat
                            .as_ref()
                            .and_then(|bd| bd.entries.get(idx))
                            .map(|be| be.data.as_slice());
                        let child_node = build_nodes_for_blob_with_baseline(
                            &child_id,
                            &entry.name,
                            &entry.data,
                            child_base,
                            pair_wtp.as_deref(),
                            None,
                            oodle_dll,
                            depth + 1,
                        );
                        node_modified |= child_node.modified;
                        children.push(child_node);
                    }
                }
            }
            "PKZ" => {
                if let Ok(pkz) = PkzArchive::from_bytes(bytes) {
                    let (pkz_mod_map, pkz_staged_total) =
                        compute_pkz_modified_map(bytes, &pkz, baseline_bytes);
                    node_modified = pkz_staged_total > 0;
                    let oodle_lib = OodleLibrary::load(oodle_dll).ok();
                    let mut unpacked_cache: HashMap<usize, Vec<u8>> = HashMap::new();
                    for (idx, desc) in pkz.descriptors.iter().enumerate() {
                        let ext = Path::new(&desc.name)
                            .extension()
                            .and_then(|e| e.to_str())
                            .unwrap_or("")
                            .to_ascii_lowercase();
                        // Only decompress nested containers that expand into sub-trees in the Asset Browser
                        let expands_in_tree = matches!(
                            ext.as_str(),
                            "dat" | "dtt" | "evn" | "pkz" | "eff" | "idd" | "scr" | "wtb" | "wta" | "wtp" | "wmb" | "bntx" | "uvd"
                        );
                        if expands_in_tree {
                            if let Ok(u) = pkz.extract_entry_with_lib(bytes, idx, oodle_lib.as_ref()) {
                                unpacked_cache.insert(idx, u);
                            }
                        }
                    }

                    fn insert_into_pkz_folder(
                        nodes: &mut Vec<AssetTreeNode>,
                        folder_parts: &[&str],
                        accum_prefix: &str,
                        id_prefix: &str,
                        leaf_node: AssetTreeNode,
                    ) {
                        if folder_parts.is_empty() {
                            nodes.push(leaf_node);
                            return;
                        }
                        let folder_name = folder_parts[0];
                        let next_accum = if accum_prefix.is_empty() {
                            folder_name.to_string()
                        } else {
                            format!("{accum_prefix}/{folder_name}")
                        };
                        let folder_id = if id_prefix.is_empty() {
                            format!("pkzfolder:{next_accum}")
                        } else {
                            format!("{id_prefix}/pkzfolder:{next_accum}")
                        };
                        let pos = nodes.iter().position(|n| n.id == folder_id);
                        let folder_node = if let Some(idx) = pos {
                            &mut nodes[idx]
                        } else {
                            let new_idx = nodes.len();
                            nodes.push(AssetTreeNode {
                                id: folder_id,
                                label: folder_name.to_string(),
                                kind: "PKZ_FOLDER".into(),
                                format_badge: "DIR".into(),
                                size: 0,
                                modified: false,
                                compression: None,
                                children: Vec::new(),
                            });
                            &mut nodes[new_idx]
                        };
                        folder_node.size += leaf_node.size;
                        folder_node.modified |= leaf_node.modified;
                        insert_into_pkz_folder(
                            &mut folder_node.children,
                            &folder_parts[1..],
                            &next_accum,
                            id_prefix,
                            leaf_node,
                        );
                    }

                    for (idx, desc) in pkz.descriptors.iter().enumerate() {
                        let child_id = if id_prefix.is_empty() {
                            format!("pkz:{idx}")
                        } else {
                            format!("{id_prefix}/pkz:{idx}")
                        };
                        let norm_path = desc.name.replace('\\', "/");
                        let clean_path = norm_path.trim_start_matches('/');
                        let segs: Vec<&str> = clean_path.split('/').filter(|s| !s.is_empty()).collect();
                        let base_name = segs.last().copied().unwrap_or(desc.name.as_str());
                        let folder_segs = if segs.len() > 1 { &segs[..segs.len() - 1] } else { &[] };
                        let is_entry_mod = pkz_mod_map.get(&idx).copied().unwrap_or(false);

                        let mut leaf_node = if let Some(unpacked) = unpacked_cache.get(&idx) {
                            let peer_dtt = if desc.name.to_ascii_lowercase().ends_with(".dat") {
                                let stem = &desc.name[..desc.name.len() - 4];
                                let target_dtt = format!("{stem}.dtt");
                                pkz.descriptors
                                    .iter()
                                    .position(|d| d.name.eq_ignore_ascii_case(&target_dtt))
                                    .and_then(|d_idx| unpacked_cache.get(&d_idx).map(|v| v.as_slice()))
                            } else {
                                None
                            };
                            build_nodes_for_blob(
                                &child_id,
                                base_name,
                                unpacked,
                                None,
                                peer_dtt,
                                oodle_dll,
                                depth + 1,
                            )
                        } else {
                            let tag = detect_file_format_tag(base_name, &[]);
                            AssetTreeNode {
                                id: child_id,
                                label: base_name.to_string(),
                                kind: tag.to_string(),
                                format_badge: tag.to_string(),
                                size: desc.size as usize,
                                modified: is_entry_mod,
                                compression: Some(desc.compression.clone()),
                                children: Vec::new(),
                            }
                        };
                        leaf_node.modified = is_entry_mod;
                        leaf_node.compression = Some(desc.compression.clone());
                        insert_into_pkz_folder(&mut children, folder_segs, "", id_prefix, leaf_node);
                    }
                }
            }
            "EFF" => {
                if let Ok(eff) = EffFile::from_bytes(bytes) {
                    for (d_idx, dir) in eff.directories.iter().enumerate() {
                        let dir_id = if id_prefix.is_empty() {
                            format!("effdir:{d_idx}")
                        } else {
                            format!("{id_prefix}/effdir:{d_idx}")
                        };
                        let mut dir_children = Vec::new();
                        for (f_idx, f) in dir.files.iter().enumerate() {
                            let fid = format!("{dir_id}/efffile:{f_idx}");
                            dir_children.push(build_nodes_for_blob(
                                &fid,
                                &f.file_name,
                                &f.data,
                                None,
                                None,
                                oodle_dll,
                                depth + 1,
                            ));
                        }
                        children.push(AssetTreeNode {
                            id: dir_id,
                            label: format!("{} (#{})", dir.name, dir.dir_id),
                            kind: "EFF_DIR".into(),
                            format_badge: "DIR".into(),
                            size: dir.files.iter().map(|x| x.data.len()).sum(),
                            modified: false,
                            compression: None,
                            children: dir_children,
                        });
                    }
                }
            }
            "SCR" => {
                if let Ok(scr) = ScrFile::from_bytes(bytes) {
                    for (m_idx, m_bytes) in scr.models.iter().enumerate() {
                        let mid = if id_prefix.is_empty() {
                            format!("scrmdl:{m_idx}")
                        } else {
                            format!("{id_prefix}/scrmdl:{m_idx}")
                        };
                        let mname = scr
                            .models_metadata
                            .get(m_idx)
                            .map(|m| format!("{:03}_{}.wmb", m_idx, m.name))
                            .unwrap_or_else(|| format!("{m_idx:03}.wmb"));
                        children.push(build_nodes_for_blob(
                            &mid,
                            &mname,
                            m_bytes,
                            None,
                            None,
                            oodle_dll,
                            depth + 1,
                        ));
                    }
                    if let Some(ref tex_bytes) = scr.textures {
                        let tid = if id_prefix.is_empty() {
                            "scrtex:0".to_string()
                        } else {
                            format!("{id_prefix}/scrtex:0")
                        };
                        children.push(build_nodes_for_blob(
                            &tid,
                            "000.wtb",
                            tex_bytes,
                            None,
                            None,
                            oodle_dll,
                            depth + 1,
                        ));
                    }
                }
            }
            "WTB" | "BNTX" => {
                if let Ok(wtb) = WtbFile::from_bytes(bytes, sibling_wtp) {
                    for e in &wtb.entries {
                        let tid = if id_prefix.is_empty() {
                            format!("tex:{}", e.index)
                        } else {
                            format!("{id_prefix}/tex:{}", e.index)
                        };
                        let name_or_hash = e
                            .metadata
                            .as_ref()
                            .and_then(|m| m.name.as_ref())
                            .map(|n| format!("_{n}"))
                            .or_else(|| e.texture_id.map(|id| format!("_0x{id:08X}")))
                            .unwrap_or_default();
                        let fmt_badge = e
                            .metadata
                            .as_ref()
                            .map(|m| m.format.display_name())
                            .unwrap_or_else(|| e.extension.trim_start_matches('.').to_uppercase());
                        children.push(AssetTreeNode {
                            id: tid,
                            label: format!("tex{}{}{}", e.index, name_or_hash, e.extension),
                            kind: "TEXTURE".into(),
                            format_badge: fmt_badge,
                            size: e.data.len(),
                            modified: false,
                            compression: None,
                            children: Vec::new(),
                        });
                    }
                }
            }
            "WMB" => {
                if let Ok(wmb) = WmbFile::from_bytes(bytes) {
                    let is_b2 = wmb.is_bayo2();
                    let base = if id_prefix.is_empty() {
                        "wmb".to_string()
                    } else {
                        format!("{id_prefix}/wmb")
                    };
                    children.push(AssetTreeNode {
                        id: format!("{base}:header"),
                        label: format!("Header (0x{:08X})", wmb.header.vertex_flags),
                        kind: "WMB_HEADER".into(),
                        format_badge: "HDR".into(),
                        size: 128,
                        modified: false,
                        compression: None,
                        children: Vec::new(),
                    });
                    children.push(AssetTreeNode {
                        id: format!("{base}:vertexes"),
                        label: format!("Vertexes ({} verts)", wmb.vertexes.len()),
                        kind: "WMB_VERTEXES".into(),
                        format_badge: "VTX".into(),
                        size: wmb.vertexes.len() * 32,
                        modified: false,
                        compression: None,
                        children: Vec::new(),
                    });
                    children.push(AssetTreeNode {
                        id: format!("{base}:bones"),
                        label: format!("Bones ({} bones)", wmb.bone_hierarchy.len()),
                        kind: "WMB_BONES".into(),
                        format_badge: "SKEL".into(),
                        size: wmb.bone_hierarchy.len() * 16,
                        modified: false,
                        compression: None,
                        children: Vec::new(),
                    });
                    let mat_children: Vec<AssetTreeNode> = wmb
                        .materials
                        .iter()
                        .enumerate()
                        .map(|(mi, m)| {
                            let dec = m.decode_with_hint(mi, is_b2);
                            AssetTreeNode {
                                id: format!("{base}:mat:{mi}"),
                                label: format!(
                                    "Material #{mi} ({})",
                                    dec.shader_name.as_deref().unwrap_or("Unknown")
                                ),
                                kind: "WMB_MATERIAL".into(),
                                format_badge: format!("0x{:02X}", m.mat_type),
                                size: m.byte_size(),
                                modified: false,
                                compression: None,
                                children: Vec::new(),
                            }
                        })
                        .collect();
                    children.push(AssetTreeNode {
                        id: format!("{base}:materials"),
                        label: format!("Materials ({})", wmb.materials.len()),
                        kind: "WMB_MATERIALS".into(),
                        format_badge: "MAT".into(),
                        size: wmb.materials.iter().map(|m| m.byte_size()).sum(),
                        modified: false,
                        compression: None,
                        children: mat_children,
                    });
                    let mesh_children: Vec<AssetTreeNode> = wmb
                        .meshes
                        .iter()
                        .enumerate()
                        .map(|(mesh_idx, mesh)| {
                            let batch_nodes: Vec<AssetTreeNode> = mesh
                                .batches
                                .iter()
                                .enumerate()
                                .map(|(bi, b)| {
                                    let eff_mat = b.header.effective_material_id(is_b2);
                                    let shadow_tag = if b.header.is_shadow_proxy(is_b2) {
                                        " [SHADOW]"
                                    } else {
                                        ""
                                    };
                                    AssetTreeNode {
                                        id: format!("{base}:mesh:{mesh_idx}:batch:{bi}"),
                                        label: format!(
                                            "Batch #{bi} (mat {eff_mat}, {} idx){shadow_tag}",
                                            b.indices.len()
                                        ),
                                        kind: "WMB_BATCH".into(),
                                        format_badge: if b.header.is_shadow_proxy(is_b2) {
                                            "SHADOW".into()
                                        } else {
                                            "BATCH".into()
                                        },
                                        size: b.byte_size(),
                                        modified: false,
                                        compression: None,
                                        children: Vec::new(),
                                    }
                                })
                                .collect();
                            AssetTreeNode {
                                id: format!("{base}:mesh:{mesh_idx}"),
                                label: format!(
                                    "Mesh #{mesh_idx} {} ({} batches)",
                                    mesh.name,
                                    mesh.batches.len()
                                ),
                                kind: "WMB_MESH".into(),
                                format_badge: "MESH".into(),
                                size: mesh.byte_size(),
                                modified: false,
                                compression: None,
                                children: batch_nodes,
                            }
                        })
                        .collect();
                    children.push(AssetTreeNode {
                        id: format!("{base}:meshes"),
                        label: format!("Meshes ({})", wmb.meshes.len()),
                        kind: "WMB_MESHES".into(),
                        format_badge: "GEO".into(),
                        size: wmb.meshes.iter().map(|m| m.byte_size()).sum(),
                        modified: false,
                        compression: None,
                        children: mesh_children,
                    });
                }
            }
            "WMB3" => {
                if let Ok(wmb3) = Wmb3File::from_bytes(bytes) {
                    let base = if id_prefix.is_empty() {
                        "wmb3".to_string()
                    } else {
                        format!("{id_prefix}/wmb3")
                    };
                    children.push(AssetTreeNode {
                        id: format!("{base}:header"),
                        label: format!("Header (v0x{:08X})", wmb3.version),
                        kind: "WMB3_HEADER".into(),
                        format_badge: "WMB3".into(),
                        size: 144,
                        modified: false,
                        compression: None,
                        children: Vec::new(),
                    });
                    children.push(AssetTreeNode {
                        id: format!("{base}:bones"),
                        label: format!("Bones ({})", wmb3.bones.len()),
                        kind: "WMB3_BONES".into(),
                        format_badge: "SKEL".into(),
                        size: wmb3.bones.len() * 88,
                        modified: false,
                        compression: None,
                        children: Vec::new(),
                    });
                    let mat_children: Vec<AssetTreeNode> = wmb3
                        .materials
                        .iter()
                        .enumerate()
                        .map(|(mi, m)| AssetTreeNode {
                            id: format!("{base}:mat:{mi}"),
                            label: format!("Material #{mi} {} ({})", m.name, m.shader_name),
                            kind: "WMB3_MATERIAL".into(),
                            format_badge: "MAT3".into(),
                            size: 48,
                            modified: false,
                            compression: None,
                            children: Vec::new(),
                        })
                        .collect();
                    children.push(AssetTreeNode {
                        id: format!("{base}:materials"),
                        label: format!("Materials ({})", wmb3.materials.len()),
                        kind: "WMB3_MATERIALS".into(),
                        format_badge: "MAT3".into(),
                        size: wmb3.materials.len() * 48,
                        modified: false,
                        compression: None,
                        children: mat_children,
                    });
                    children.push(AssetTreeNode {
                        id: format!("{base}:meshes"),
                        label: format!(
                            "Meshes ({}) & Batches ({})",
                            wmb3.meshes.len(),
                            wmb3.batches.len()
                        ),
                        kind: "WMB3_MESHES".into(),
                        format_badge: "GEO3".into(),
                        size: wmb3.batches.len() * 28,
                        modified: false,
                        compression: None,
                        children: Vec::new(),
                    });
                }
            }
            "UVD" => {
                if let Ok(uvd) = UvdFile::from_bytes(bytes) {
                    for (g_idx, grp) in uvd.groups.iter().enumerate() {
                        let gid = if id_prefix.is_empty() {
                            format!("uvdgrp:{g_idx}")
                        } else {
                            format!("{id_prefix}/uvdgrp:{g_idx}")
                        };
                        let item_children: Vec<AssetTreeNode> = grp
                            .items
                            .iter()
                            .enumerate()
                            .map(|(i_idx, it)| AssetTreeNode {
                                id: format!("{gid}/uvditem:{i_idx}"),
                                label: format!(
                                    "{} ({}×{})",
                                    it.name,
                                    it.w.round() as u32,
                                    it.h.round() as u32
                                ),
                                kind: "UVD_ITEM".into(),
                                format_badge: format!("0x{:08X}", it.id),
                                size: 96,
                                modified: false,
                                compression: None,
                                children: Vec::new(),
                            })
                            .collect();
                        children.push(AssetTreeNode {
                            id: gid,
                            label: format!("{} ({})", grp.name, grp.items.len()),
                            kind: "UVD_GROUP".into(),
                            format_badge: format!("0x{:08X}", grp.id),
                            size: 36 + grp.items.len() * 96,
                            modified: false,
                            compression: None,
                            children: item_children,
                        });
                    }
                }
            }
            _ => {}
        }
    }

    let node_id = if id_prefix.is_empty() {
        "root".to_string()
    } else {
        id_prefix.to_string()
    };

    AssetTreeNode {
        id: node_id,
        label: name.to_string(),
        kind: tag.to_string(),
        format_badge: tag.to_string(),
        size: bytes.len(),
        modified: node_modified,
        compression: None,
        children,
    }
}

/// Resolve a hierarchical `node_id` path (e.g. `dat:2/effdir:0/efffile:1`, `root:dtt/dat:0/tex:2`, `pkz:0/dat:1/wmb:mat:0`)
/// to `(subfile_name, subfile_bytes, optional_sibling_wtp_bytes, leaf_selector)`.
pub(crate) fn resolve_subblob(
    snap: &DocSnapshot,
    node_id: &str,
    oodle_dll: Option<&Path>,
) -> Result<(String, Vec<u8>, Option<Vec<u8>>, String), String> {
    let (mut cur_bytes, mut cur_wtp, mut cur_dtt_peer): (Vec<u8>, Option<Vec<u8>>, Option<std::sync::Arc<Vec<u8>>>) = match snap {
        DocSnapshot::SingleFile(b) => (b.as_ref().clone(), None, None),
        DocSnapshot::SplitWtaWtp { wta, wtp } => {
            (wta.as_ref().clone(), Some(wtp.as_ref().clone()), None)
        }
        DocSnapshot::PairedDatDtt { dat, dtt } => {
            if node_id.starts_with("root:dtt") {
                (Vec::new(), None, None)
            } else {
                (dat.as_ref().clone(), None, Some(dtt.clone()))
            }
        }
    };
    let mut cur_name = String::from("root");

    if node_id == "root" || node_id.is_empty() || node_id == "root:dat" {
        return Ok((cur_name, cur_bytes, cur_wtp, String::new()));
    }
    if node_id == "root:dtt" {
        if let DocSnapshot::PairedDatDtt { dtt, .. } = snap {
            return Ok(("dtt".to_string(), dtt.as_ref().clone(), None, String::new()));
        }
    }

    let parts: Vec<&str> = node_id.split('/').collect();
    let mut idx = 0usize;
    while idx < parts.len() {
        let part = parts[idx];
        if part == "root:dat" {
            if let DocSnapshot::PairedDatDtt { dat, dtt } = snap {
                cur_bytes = dat.as_ref().clone();
                cur_dtt_peer = Some(dtt.clone());
                cur_name = "dat".to_string();
            }
            idx += 1;
            continue;
        }
        if part == "root:dtt" {
            if let DocSnapshot::PairedDatDtt { dtt, .. } = snap {
                cur_bytes = dtt.as_ref().clone();
                cur_dtt_peer = None;
                cur_name = "dtt".to_string();
            }
            idx += 1;
            continue;
        }
        if let Some(dat_idx_str) = part.strip_prefix("dat:") {
            let entry_idx: usize = dat_idx_str.parse().map_err(|_| "Invalid dat index")?;
            let dat = DatFile::from_bytes(&cur_bytes).map_err(|e| e.to_string())?;
            let parsed_dtt = cur_dtt_peer
                .as_ref()
                .and_then(|b| DatFile::from_bytes(b).ok());
            let dtt_slice = parsed_dtt.as_ref().map(|d| d.entries.as_slice());

            let total_wtas = dat
                .entries
                .iter()
                .filter(|e| e.extension.eq_ignore_ascii_case("wta"))
                .count();
            let wta_ord = dat.entries[..entry_idx.min(dat.entries.len())]
                .iter()
                .filter(|e| e.extension.eq_ignore_ascii_case("wta"))
                .count();

            let entry = dat
                .entries
                .get(entry_idx)
                .ok_or_else(|| format!("DAT entry #{entry_idx} out of range"))?;
            cur_name = entry.name.clone();
            cur_wtp = if entry.extension.eq_ignore_ascii_case("wta") {
                find_paired_wtp_bytes(&entry.name, wta_ord, total_wtas, &dat.entries, dtt_slice)
            } else {
                None
            };
            cur_bytes = entry.data.clone();
            cur_dtt_peer = None;
            idx += 1;
            continue;
        }
        if let Some(pkz_idx_str) = part.strip_prefix("pkz:") {
            let entry_idx: usize = pkz_idx_str.parse().map_err(|_| "Invalid pkz index")?;
            let pkz = PkzArchive::from_bytes(&cur_bytes).map_err(|e| e.to_string())?;
            let oodle_lib = OodleLibrary::load(oodle_dll).ok();
            let desc = pkz
                .descriptors
                .get(entry_idx)
                .ok_or_else(|| format!("PKZ entry #{entry_idx} out of range"))?;
            let peer_dtt = find_pkz_peer_dtt_index(&pkz, &desc.name).and_then(|d_idx| {
                pkz.extract_entry_with_lib(&cur_bytes, d_idx, oodle_lib.as_ref())
                    .ok()
            });
            cur_name = desc.name.clone();
            cur_bytes = pkz
                .extract_entry_with_lib(&cur_bytes, entry_idx, oodle_lib.as_ref())
                .map_err(|e| e.to_string())?;
            cur_wtp = None;
            cur_dtt_peer = peer_dtt.map(std::sync::Arc::new);
            idx += 1;
            continue;
        }
        if let Some(eff_dir_str) = part.strip_prefix("effdir:") {
            let d_idx: usize = eff_dir_str.parse().map_err(|_| "Invalid effdir index")?;
            let eff = EffFile::from_bytes(&cur_bytes).map_err(|e| e.to_string())?;
            let dir = eff
                .directories
                .get(d_idx)
                .ok_or_else(|| format!("EFF directory #{d_idx} out of range"))?;
            if idx + 1 < parts.len() {
                if let Some(eff_file_str) = parts[idx + 1].strip_prefix("efffile:") {
                    let f_idx: usize = eff_file_str.parse().map_err(|_| "Invalid efffile index")?;
                    let file_entry = dir
                        .files
                        .get(f_idx)
                        .ok_or_else(|| format!("EFF file #{f_idx} out of range"))?;
                    cur_name = file_entry.file_name.clone();
                    cur_bytes = file_entry.data.clone();
                    cur_wtp = None;
                    cur_dtt_peer = None;
                    idx += 2;
                    continue;
                }
            }
            let remaining = parts[idx..].join("/");
            return Ok((cur_name, cur_bytes, cur_wtp, remaining));
        }
        if let Some(scr_mdl_str) = part.strip_prefix("scrmdl:") {
            let m_idx: usize = scr_mdl_str.parse().map_err(|_| "Invalid scr model index")?;
            let scr = ScrFile::from_bytes(&cur_bytes).map_err(|e| e.to_string())?;
            cur_bytes = scr
                .models
                .get(m_idx)
                .cloned()
                .ok_or_else(|| "SCR model index out of bounds".to_string())?;
            cur_name = format!("{m_idx:03}.wmb");
            cur_wtp = None;
            cur_dtt_peer = None;
            idx += 1;
            continue;
        }
        if part == "scrtex:0" {
            let scr = ScrFile::from_bytes(&cur_bytes).map_err(|e| e.to_string())?;
            cur_bytes = scr
                .textures
                .ok_or_else(|| "SCR has no embedded WTB".to_string())?;
            cur_name = "000.wtb".to_string();
            cur_wtp = None;
            cur_dtt_peer = None;
            idx += 1;
            continue;
        }

        let remaining = parts[idx..].join("/");
        return Ok((cur_name, cur_bytes, cur_wtp, remaining));
    }

    Ok((cur_name, cur_bytes, cur_wtp, String::new()))
}

pub(crate) fn split_container_and_leaf(node_id: &str) -> (Vec<&str>, &str) {
    if node_id == "root" || node_id.is_empty() {
        return (Vec::new(), "");
    }
    let parts: Vec<&str> = node_id.split('/').collect();
    let mut container_end = 0;
    let mut i = 0;
    while i < parts.len() {
        let p = parts[i];
        if p == "root:dat"
            || p == "root:dtt"
            || p.starts_with("dat:")
            || p.starts_with("pkz:")
            || p.starts_with("scrmdl:")
            || p == "scrtex:0"
        {
            container_end = i + 1;
            i += 1;
        } else if p.starts_with("effdir:") {
            if i + 1 < parts.len() && parts[i + 1].starts_with("efffile:") {
                container_end = i + 2;
                i += 2;
            } else {
                break;
            }
        } else {
            break;
        }
    }
    let container_parts = parts[..container_end].to_vec();
    let leaf = if container_end < parts.len() {
        let prefix_len: usize = parts[..container_end].iter().map(|s| s.len() + 1).sum();
        node_id.get(prefix_len..).unwrap_or(parts[container_end])
    } else {
        ""
    };
    (container_parts, leaf)
}

/// Update paired `.wtp` inside a `DatFile` (by stem match or ordinal) when a `.wta` was modified.
pub(crate) fn update_matching_wtp_in_dat(
    wta_name: &str,
    wta_ordinal: usize,
    dat: &mut DatFile,
    new_wtp: &[u8],
) -> bool {
    let wta_stem = Path::new(wta_name)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("");
    for e in &mut dat.entries {
        if e.extension.eq_ignore_ascii_case("wtp") {
            let stem = Path::new(&e.name)
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("");
            if !wta_stem.is_empty() && stem.eq_ignore_ascii_case(wta_stem) {
                e.data = new_wtp.to_vec();
                return true;
            }
        }
    }
    let mut wtp_indices = Vec::new();
    for (idx, e) in dat.entries.iter().enumerate() {
        if e.extension.eq_ignore_ascii_case("wtp") {
            wtp_indices.push(idx);
        }
    }
    if wtp_indices.len() == 1 {
        dat.entries[wtp_indices[0]].data = new_wtp.to_vec();
        return true;
    }
    if let Some(&target_idx) = wtp_indices.get(wta_ordinal) {
        dat.entries[target_idx].data = new_wtp.to_vec();
        return true;
    }
    false
}

/// Replace a modified child byte buffer back up the entire nested container hierarchy
/// (`PKZ <-> DAT/DTT <-> EFF/IDD <-> SCR <-> WTB/WMB/WMB3/EST/BXM/CSV`) and return the updated `DocSnapshot`.
#[cfg(test)]
pub(crate) fn write_back_subblob(
    snap: &DocSnapshot,
    container_parts: &[&str],
    new_bytes: Vec<u8>,
    new_wtp: Option<Vec<u8>>,
    oodle_dll: Option<&Path>,
) -> Result<DocSnapshot, String> {
    write_back_subblob_with_align(snap, container_parts, new_bytes, new_wtp, oodle_dll, None)
}

pub(crate) fn write_back_subblob_with_align(
    snap: &DocSnapshot,
    container_parts: &[&str],
    new_bytes: Vec<u8>,
    new_wtp: Option<Vec<u8>>,
    oodle_dll: Option<&Path>,
    dat_align: Option<usize>,
) -> Result<DocSnapshot, String> {
    if container_parts.is_empty() {
        return Ok(match (snap, new_wtp) {
            (DocSnapshot::SplitWtaWtp { .. }, Some(wtp)) => DocSnapshot::split(new_bytes, wtp),
            (DocSnapshot::PairedDatDtt { dtt, .. }, _) => DocSnapshot::PairedDatDtt {
                dat: Arc::new(new_bytes),
                dtt: dtt.clone(),
            },
            _ => DocSnapshot::single(new_bytes),
        });
    }

    let first = container_parts[0];
    let rest = &container_parts[1..];

    if first == "root:dat" {
        if let DocSnapshot::PairedDatDtt { dat, dtt } = snap {
            if rest.is_empty() {
                return Ok(DocSnapshot::PairedDatDtt {
                    dat: Arc::new(new_bytes),
                    dtt: dtt.clone(),
                });
            }
            if let Some(dat_idx_str) = rest[0].strip_prefix("dat:") {
                let entry_idx: usize = dat_idx_str.parse().map_err(|_| "Invalid dat index")?;
                let mut parsed_dat = DatFile::from_bytes(dat).map_err(|e| e.to_string())?;
                let mut parsed_dtt = DatFile::from_bytes(dtt).map_err(|e| e.to_string())?;
                let wta_ord = parsed_dat.entries[..entry_idx.min(parsed_dat.entries.len())]
                    .iter()
                    .filter(|e| e.extension.eq_ignore_ascii_case("wta"))
                    .count();
                let child_orig = parsed_dat
                    .entries
                    .get(entry_idx)
                    .ok_or("DAT index out of bounds")?
                    .data
                    .clone();
                let entry_name = parsed_dat.entries[entry_idx].name.clone();
                let updated_child = write_back_subblob_with_align(
                    &DocSnapshot::single(child_orig),
                    &rest[1..],
                    new_bytes.clone(),
                    new_wtp.clone(),
                    oodle_dll,
                    dat_align,
                )?;
                if let DocSnapshot::SingleFile(cb) = updated_child {
                    parsed_dat.entries[entry_idx].data = cb.as_ref().clone();
                }
                let mut dtt_modified = false;
                if rest.len() == 1 {
                    if let Some(ref wtp_bytes) = new_wtp {
                        if !update_matching_wtp_in_dat(
                            &entry_name,
                            wta_ord,
                            &mut parsed_dat,
                            wtp_bytes,
                        ) {
                            if update_matching_wtp_in_dat(
                                &entry_name,
                                wta_ord,
                                &mut parsed_dtt,
                                wtp_bytes,
                            ) {
                                dtt_modified = true;
                            } else if !wtp_bytes.is_empty() {
                                let wta_stem = Path::new(&entry_name)
                                    .file_stem()
                                    .and_then(|s| s.to_str())
                                    .unwrap_or("textures");
                                if parsed_dtt
                                    .push(format!("{wta_stem}.wtp"), wtp_bytes.clone())
                                    .is_ok()
                                {
                                    dtt_modified = true;
                                }
                            }
                        }
                    }
                }
                return Ok(DocSnapshot::PairedDatDtt {
                    dat: Arc::new(parsed_dat.to_bytes_with_alignment(dat_align)),
                    dtt: if dtt_modified {
                        Arc::new(parsed_dtt.to_bytes_with_alignment(dat_align))
                    } else {
                        dtt.clone()
                    },
                });
            }
        }
    }

    if first == "root:dtt" {
        if let DocSnapshot::PairedDatDtt { dat, dtt } = snap {
            if rest.is_empty() {
                return Ok(DocSnapshot::PairedDatDtt {
                    dat: dat.clone(),
                    dtt: Arc::new(new_bytes),
                });
            }
            let updated_dtt = write_back_subblob_with_align(
                &DocSnapshot::SingleFile(dtt.clone()),
                rest,
                new_bytes.clone(),
                new_wtp.clone(),
                oodle_dll,
                dat_align,
            )?;
            if let DocSnapshot::SingleFile(new_dtt_bytes) = updated_dtt {
                return Ok(DocSnapshot::PairedDatDtt {
                    dat: dat.clone(),
                    dtt: new_dtt_bytes,
                });
            }
        }
    }

    let root_bytes: &[u8] = match snap {
        DocSnapshot::SingleFile(b) => b.as_slice(),
        DocSnapshot::SplitWtaWtp { wta, .. } => wta.as_slice(),
        DocSnapshot::PairedDatDtt { dat, .. } => dat.as_slice(),
    };

    if let Some(dat_idx_str) = first.strip_prefix("dat:") {
        let entry_idx: usize = dat_idx_str.parse().map_err(|_| "Invalid dat index")?;
        let mut dat = DatFile::from_bytes(root_bytes).map_err(|e| e.to_string())?;
        let wta_ord = dat.entries[..entry_idx.min(dat.entries.len())]
            .iter()
            .filter(|e| e.extension.eq_ignore_ascii_case("wta"))
            .count();
        let entry_name = dat
            .entries
            .get(entry_idx)
            .ok_or("DAT index out of bounds")?
            .name
            .clone();
        let child_orig = dat.entries[entry_idx].data.clone();
        let updated_child = write_back_subblob_with_align(
            &DocSnapshot::single(child_orig),
            rest,
            new_bytes.clone(),
            new_wtp.clone(),
            oodle_dll,
            dat_align,
        )?;
        if let DocSnapshot::SingleFile(cb) = updated_child {
            dat.entries[entry_idx].data = cb.as_ref().clone();
        }
        let mut updated_dtt_bytes = None;
        if rest.is_empty() {
            if let Some(wtp_bytes) = new_wtp {
                if !update_matching_wtp_in_dat(&entry_name, wta_ord, &mut dat, &wtp_bytes) {
                    let mut handled_in_dtt = false;
                    if let DocSnapshot::PairedDatDtt { dtt, .. } = snap {
                        if let Ok(mut parsed_dtt) = DatFile::from_bytes(dtt) {
                            if update_matching_wtp_in_dat(
                                &entry_name,
                                wta_ord,
                                &mut parsed_dtt,
                                &wtp_bytes,
                            ) {
                                updated_dtt_bytes = Some(Arc::new(parsed_dtt.to_bytes_with_alignment(dat_align)));
                                handled_in_dtt = true;
                            } else if !wtp_bytes.is_empty() {
                                let wta_stem = Path::new(&entry_name)
                                    .file_stem()
                                    .and_then(|s| s.to_str())
                                    .unwrap_or("textures");
                                if parsed_dtt
                                    .push(format!("{wta_stem}.wtp"), wtp_bytes.clone())
                                    .is_ok()
                                {
                                    updated_dtt_bytes = Some(Arc::new(parsed_dtt.to_bytes_with_alignment(dat_align)));
                                    handled_in_dtt = true;
                                }
                            }
                        }
                    }
                    if !handled_in_dtt && !wtp_bytes.is_empty() {
                        let wta_stem = Path::new(&entry_name)
                            .file_stem()
                            .and_then(|s| s.to_str())
                            .unwrap_or("textures");
                        let wtp_name = format!("{wta_stem}.wtp");
                        dat.push(wtp_name.clone(), wtp_bytes)
                            .map_err(|e| format!("Failed to add {wtp_name} to DAT: {e}"))?;
                    }
                }
            }
        }
        return Ok(match snap {
            DocSnapshot::PairedDatDtt { dtt, .. } => DocSnapshot::PairedDatDtt {
                dat: Arc::new(dat.to_bytes_with_alignment(dat_align)),
                dtt: updated_dtt_bytes.unwrap_or_else(|| dtt.clone()),
            },
            _ => DocSnapshot::single(dat.to_bytes_with_alignment(dat_align)),
        });
    }

    if let Some(pkz_idx_str) = first.strip_prefix("pkz:") {
        let entry_idx: usize = pkz_idx_str.parse().map_err(|_| "Invalid pkz index")?;
        let pkz = PkzArchive::from_bytes(root_bytes).map_err(|e| e.to_string())?;
        let oodle_lib = OodleLibrary::load(oodle_dll).ok();
        let child_orig = pkz
            .extract_entry_with_lib(root_bytes, entry_idx, oodle_lib.as_ref())
            .map_err(|e| e.to_string())?;
        let entry_name = pkz
            .descriptors
            .get(entry_idx)
            .map(|d| d.name.clone())
            .unwrap_or_default();
        let peer_dtt_idx = find_pkz_peer_dtt_index(&pkz, &entry_name);

        if let Some(dtt_idx) = peer_dtt_idx {
            if let Ok(dtt_orig) =
                pkz.extract_entry_with_lib(root_bytes, dtt_idx, oodle_lib.as_ref())
            {
                let mut child_parts = Vec::with_capacity(rest.len() + 1);
                child_parts.push("root:dat");
                child_parts.extend_from_slice(rest);
                let paired_snap = DocSnapshot::paired(child_orig.clone(), dtt_orig.clone());
                let updated_paired = write_back_subblob_with_align(
                    &paired_snap,
                    &child_parts,
                    new_bytes.clone(),
                    new_wtp.clone(),
                    oodle_dll,
                    dat_align,
                )?;
                if let DocSnapshot::PairedDatDtt {
                    dat: new_dat,
                    dtt: new_dtt,
                } = updated_paired
                {
                    let mut replacements = HashMap::new();
                    replacements.insert(entry_idx, new_dat.as_ref().clone());
                    if new_dtt.as_slice() != dtt_orig.as_slice() {
                        replacements.insert(dtt_idx, new_dtt.as_ref().clone());
                    }
                    let repacked = pkz
                        .repack_with_replacements(
                            root_bytes,
                            &replacements,
                            &[],
                            &[],
                            None,
                            None,
                            oodle_dll,
                        )
                        .map_err(|e| e.to_string())?;
                    return Ok(DocSnapshot::single(repacked));
                }
            }
        }

        let updated_child = write_back_subblob_with_align(
            &DocSnapshot::single(child_orig),
            rest,
            new_bytes.clone(),
            new_wtp.clone(),
            oodle_dll,
            dat_align,
        )?;
        if let DocSnapshot::SingleFile(cb) = updated_child {
            let mut replacements = HashMap::new();
            replacements.insert(entry_idx, cb.as_ref().clone());
            let repacked = pkz
                .repack_with_replacements(
                    root_bytes,
                    &replacements,
                    &[],
                    &[],
                    None,
                    None,
                    oodle_dll,
                )
                .map_err(|e| e.to_string())?;
            return Ok(DocSnapshot::single(repacked));
        }
    }

    if let Some(eff_dir_str) = first.strip_prefix("effdir:") {
        let d_idx: usize = eff_dir_str.parse().map_err(|_| "Invalid effdir index")?;
        if let Some(second) = rest.first() {
            if let Some(eff_file_str) = second.strip_prefix("efffile:") {
                let f_idx: usize = eff_file_str.parse().map_err(|_| "Invalid efffile index")?;
                let mut eff = EffFile::from_bytes(root_bytes).map_err(|e| e.to_string())?;
                let child_orig = eff
                    .directories
                    .get(d_idx)
                    .and_then(|d| d.files.get(f_idx))
                    .ok_or("EFF entry out of bounds")?
                    .data
                    .clone();
                let updated_child = write_back_subblob_with_align(
                    &DocSnapshot::single(child_orig),
                    &rest[1..],
                    new_bytes.clone(),
                    new_wtp.clone(),
                    oodle_dll,
                    dat_align,
                )?;
                if let DocSnapshot::SingleFile(cb) = updated_child {
                    eff.directories[d_idx].files[f_idx].data = cb.as_ref().clone();
                }
                return Ok(DocSnapshot::single(
                    eff.to_bytes().map_err(|e| e.to_string())?,
                ));
            }
        }
    }

    if let Some(scr_mdl_str) = first.strip_prefix("scrmdl:") {
        let m_idx: usize = scr_mdl_str.parse().map_err(|_| "Invalid scr model index")?;
        let mut scr = ScrFile::from_bytes(root_bytes).map_err(|e| e.to_string())?;
        let child_orig = scr
            .models
            .get(m_idx)
            .ok_or("SCR model index out of bounds")?
            .clone();
        let updated_child = write_back_subblob_with_align(
            &DocSnapshot::single(child_orig),
            rest,
            new_bytes.clone(),
            None,
            oodle_dll,
            dat_align,
        )?;
        if let DocSnapshot::SingleFile(cb) = updated_child {
            scr.models[m_idx] = cb.as_ref().clone();
        }
        return Ok(DocSnapshot::single(scr.to_bytes()));
    }

    if first == "scrtex:0" {
        let mut scr = ScrFile::from_bytes(root_bytes).map_err(|e| e.to_string())?;
        scr.textures = Some(new_bytes);
        return Ok(DocSnapshot::single(scr.to_bytes()));
    }

    Err(format!(
        "Container write-back not supported for path segment '{first}'"
    ))
}


pub(crate) struct OutlineBuildContext<'a> {
    pub(crate) doc_path: Option<&'a Path>,
    pub(crate) dtt_path: Option<&'a Path>,
    pub(crate) wtp_path: Option<&'a Path>,
    pub(crate) current: Option<&'a DocSnapshot>,
    pub(crate) saved_snapshot: Option<&'a DocSnapshot>,
    pub(crate) oodle_path: Option<&'a Path>,
    pub(crate) dirty: bool,
    pub(crate) can_undo: bool,
    pub(crate) can_redo: bool,
    pub(crate) undo_label: Option<String>,
    pub(crate) redo_label: Option<String>,
    pub(crate) pending_uvd_encodes: Vec<PendingUvdSheetInfo>,
}

pub(crate) fn build_outline_from_parts(ctx: OutlineBuildContext<'_>) -> Result<DocumentOutline, String> {
    let path = ctx.doc_path.ok_or("No document currently loaded")?;
    let snap = ctx.current.ok_or("No document snapshot in memory")?;
    let oodle_path_opt = ctx.oodle_path;

    let file_name = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("document")
        .to_string();

    if let DocSnapshot::PairedDatDtt { dat, dtt } = snap {
        let dtt_name = ctx.dtt_path
            .and_then(|p| p.file_name())
            .and_then(|s| s.to_str())
            .unwrap_or("paired.dtt")
            .to_string();
        let dat_node = build_nodes_for_blob(
            "root:dat",
            &file_name,
            dat,
            None,
            Some(dtt),
            oodle_path_opt,
            0,
        );
        let dtt_node =
            build_nodes_for_blob("root:dtt", &dtt_name, dtt, None, None, oodle_path_opt, 0);
        let endian = if dat.len() >= 4 && &dat[0..4] == b"\0TAD" {
            "Big (Wii U / PS3 / X360)"
        } else {
            "Little (PC / Switch)"
        };
        let summary_right = format!(
            "Paired DAT+DTT · {} + {} files",
            dat_node.children.len(),
            dtt_node.children.len()
        );
        return Ok(DocumentOutline {
            path: path.to_string_lossy().into_owned(),
            paired_path: ctx.dtt_path
                .map(|p| p.to_string_lossy().into_owned()),
            file_name: format!("{file_name} + {dtt_name}"),
            root_format: "DAT+DTT".into(),
            endian: endian.into(),
            summary_right,
            dirty: ctx.dirty,
            can_undo: ctx.can_undo,
            can_redo: ctx.can_redo,
            undo_label: ctx.undo_label,
            redo_label: ctx.redo_label,
            tree: vec![dat_node, dtt_node],
            pending_uvd_encodes: ctx.pending_uvd_encodes,
            status_message: None,
        });
    }

    let (main_bytes, wtp_bytes) = match snap {
        DocSnapshot::SingleFile(b) => (b.as_slice(), None),
        DocSnapshot::SplitWtaWtp { wta, wtp } => (wta.as_slice(), Some(wtp.as_slice())),
        DocSnapshot::PairedDatDtt { dat, .. } => (dat.as_slice(), None),
    };

    let baseline_main_bytes: Option<&[u8]> = ctx.saved_snapshot.map(|s| match s {
        DocSnapshot::SingleFile(b) => b.as_slice(),
        DocSnapshot::SplitWtaWtp { wta, .. } => wta.as_slice(),
        DocSnapshot::PairedDatDtt { dat, .. } => dat.as_slice(),
    });

    let root_format = detect_file_format_tag(&file_name, main_bytes).to_string();
    let root_node = build_nodes_for_blob_with_baseline(
        "",
        &file_name,
        main_bytes,
        baseline_main_bytes,
        wtp_bytes,
        None,
        oodle_path_opt,
        0,
    );

    let endian = if main_bytes.len() >= 4 {
        match &main_bytes[0..4] {
            b"\0TAD" | b"PKZB" | b"\0BTW" | b"\0BMW" | b"3BMW" | b"\0RCS" | b"BXM\0" | b"XML\0"
            | b"\0TOM" | b"\0tom" | b"\0PXE" | b"Gfx2" | b"\0TSE" => "Big (Wii U / PS3 / X360)",
            _ => "Little (PC / Switch)",
        }
    } else {
        "Little"
    };

    let summary_right = match root_format.as_str() {
        "DAT" => format!("ModelDat · {} files", root_node.children.len()),
        "WTB" | "BNTX" => format!("TextureBank · {} textures", root_node.children.len()),
        "PKZ" => {
            if let Ok(pkz) = PkzArchive::from_bytes(main_bytes) {
                let (_, staged) = compute_pkz_modified_map(main_bytes, &pkz, baseline_main_bytes);
                if staged > 0 {
                    format!(
                        "PkzArchive · {} entries · {} staged change(s)",
                        pkz.descriptors.len(),
                        staged
                    )
                } else {
                    format!("PkzArchive · {} entries", pkz.descriptors.len())
                }
            } else {
                format!("PkzArchive · {} entries", root_node.children.len())
            }
        }
        "EFF" => format!("EffectContainer · {} dirs", root_node.children.len()),
        "WMB" | "WMB3" => format!("{root_format} · {} sections", root_node.children.len()),
        "UVD" => format!("UvdAtlas · {} groups", root_node.children.len()),
        _ => format!("{root_format} · {} B", main_bytes.len()),
    };

    Ok(DocumentOutline {
        path: path.to_string_lossy().into_owned(),
        paired_path: ctx.wtp_path
            .map(|p| p.to_string_lossy().into_owned()),
        file_name,
        root_format,
        endian: endian.into(),
        summary_right,
        dirty: ctx.dirty,
        can_undo: ctx.can_undo,
        can_redo: ctx.can_redo,
        undo_label: ctx.undo_label,
        redo_label: ctx.redo_label,
        tree: vec![root_node],
        pending_uvd_encodes: ctx.pending_uvd_encodes,
            status_message: None,
    })
}


/// Validate a user-supplied filesystem path used for export/import.
/// Requires a non-empty absolute path without `..` components; for writes the
/// parent directory must already exist.
pub(crate) fn validate_user_fs_path(path_str: &str, for_write: bool) -> Result<PathBuf, String> {
    let trimmed = path_str.trim();
    if trimmed.is_empty() {
        return Err("Empty filesystem path".into());
    }
    let p = PathBuf::from(trimmed);
    if !p.is_absolute() {
        return Err(format!("Path must be absolute: '{}'", p.display()));
    }
    for comp in p.components() {
        if matches!(comp, std::path::Component::ParentDir) {
            return Err(format!("Path must not contain '..': '{}'", p.display()));
        }
    }
    if for_write {
        let parent = p
            .parent()
            .ok_or_else(|| format!("Cannot write to filesystem root path '{}'", p.display()))?;
        if !parent.exists() {
            return Err(format!(
                "Parent directory does not exist: '{}'",
                parent.display()
            ));
        }
    } else if !p.is_file() {
        return Err(format!("Path is not an existing file: '{}'", p.display()));
    }
    Ok(p)
}

/// Collect disk files (and recursively any folders) into `(normalized_pkz_virtual_path, bytes)`
/// under `folder_prefix`.
pub(crate) fn collect_paths_for_pkz_add(
    paths: &[String],
    folder_prefix: &str,
) -> Result<Vec<(String, Vec<u8>)>, String> {
    let clean_prefix = folder_prefix.trim().trim_matches('/').replace('\\', "/");
    let mut items: Vec<(String, Vec<u8>)> = Vec::new();

    for p_str in paths {
        let trimmed = p_str.trim();
        let p = Path::new(trimmed);
        for comp in p.components() {
            if matches!(comp, std::path::Component::ParentDir) {
                return Err(format!("Path must not contain '..': '{}'", p.display()));
            }
        }
        if p.is_dir() {
            let sub_files =
                PkzArchive::collect_dir_files_recursive(p).map_err(|e| e.to_string())?;
            for (rel_vpath, bytes) in sub_files {
                let combined = if clean_prefix.is_empty() {
                    rel_vpath
                } else {
                    format!("{clean_prefix}/{rel_vpath}")
                };
                let norm = normalize_pkz_virtual_path(&combined).map_err(|e| e.to_string())?;
                items.push((norm, bytes));
            }
        } else if p.is_file() {
            let base_fname = p
                .file_name()
                .and_then(|s| s.to_str())
                .ok_or_else(|| format!("Invalid filename: '{}'", p.display()))?;
            let combined = if clean_prefix.is_empty() {
                base_fname.to_string()
            } else {
                format!("{clean_prefix}/{base_fname}")
            };
            let norm = normalize_pkz_virtual_path(&combined).map_err(|e| e.to_string())?;
            let bytes = fs::read(p).map_err(|e| e.to_string())?;
            items.push((norm, bytes));
        } else {
            return Err(format!("Path does not exist: '{}'", p.display()));
        }
    }

    Ok(items)
}

/// Extract a subset of PKZ entries to `out_dir`, checking for existing files on disk first.
/// If files exist and `overwrite_policy` (`"overwrite"` | `"skip"`) is not set, returns
/// `Err("EXTRACT_OVERWRITE:<count>")` so the UI can ask once for the batch.
pub(crate) fn extract_pkz_entries_with_overwrite_check(
    pkz: &PkzArchive,
    archive_bytes: &[u8],
    indices: &[usize],
    out_dir: &Path,
    overwrite_policy: Option<&str>,
    oodle_opt: Option<&Path>,
) -> Result<usize, String> {
    for comp in out_dir.components() {
        if matches!(comp, std::path::Component::ParentDir) {
            return Err(format!(
                "Destination folder must not contain '..': '{}'",
                out_dir.display()
            ));
        }
    }
    fs::create_dir_all(out_dir).map_err(|e| e.to_string())?;
    let mut planned: Vec<(usize, PathBuf)> = Vec::with_capacity(indices.len());
    for &idx in indices {
        let desc = pkz
            .descriptors
            .get(idx)
            .ok_or_else(|| format!("PKZ entry index {idx} out of range"))?;
        let norm = desc.name.replace('\\', "/");
        let clean = norm.trim_start_matches('/');
        let fallback = format!("entry_{idx}.bin");
        let dest_file =
            sanitize_entry_path(out_dir, clean, &fallback).map_err(|e| e.to_string())?;
        planned.push((idx, dest_file));
    }

    let existing_count = planned.iter().filter(|(_, dest)| dest.is_file()).count();
    if existing_count > 0 && overwrite_policy.is_none() {
        return Err(format!("EXTRACT_OVERWRITE:{existing_count}"));
    }

    let skip_existing = matches!(overwrite_policy, Some("skip"));
    let oodle_lib = OodleLibrary::load(oodle_opt).ok();
    let mut written = 0usize;
    for (idx, dest_file) in planned {
        if skip_existing && dest_file.is_file() {
            continue;
        }
        let data = pkz
            .extract_entry_with_lib(archive_bytes, idx, oodle_lib.as_ref())
            .map_err(|e| e.to_string())?;
        if let Some(p) = dest_file.parent() {
            fs::create_dir_all(p).map_err(|e| e.to_string())?;
        }
        fs::write(dest_file, data).map_err(|e| e.to_string())?;
        written += 1;
    }
    Ok(written)
}

/// Revert selected entries or a virtual folder prefix in `cur_bytes` back to their state in `base_bytes`.
pub(crate) fn revert_pkz_subset(
    cur_bytes: &[u8],
    base_bytes: &[u8],
    entry_indices: Option<&[usize]>,
    folder_prefix: Option<&str>,
) -> Result<(Vec<u8>, usize), String> {
    let cur_pkz = PkzArchive::from_bytes(cur_bytes).map_err(|e| e.to_string())?;
    let base_pkz = PkzArchive::from_bytes(base_bytes).map_err(|e| e.to_string())?;

    let clean_folder = folder_prefix
        .map(|s| s.trim().trim_matches('/').replace('\\', "/"))
        .filter(|s| !s.is_empty());

    // If neither specific indices nor folder prefix is specified, revert the entire PKZ
    if entry_indices.is_none() && clean_folder.is_none() {
        let (_, staged) = compute_pkz_modified_map(cur_bytes, &cur_pkz, Some(base_bytes));
        return Ok((base_bytes.to_vec(), staged.max(1)));
    }

    let matches_scope = |path: &str, idx_opt: Option<usize>| -> bool {
        if let Some(indices) = entry_indices {
            if let Some(i) = idx_opt {
                if indices.contains(&i) {
                    return true;
                }
            }
        }
        if let Some(ref fp) = clean_folder {
            let norm = path.replace('\\', "/");
            let clean = norm.trim_start_matches('/');
            let prefix_slash = format!("{fp}/");
            if clean.eq_ignore_ascii_case(fp)
                || clean
                    .get(..prefix_slash.len())
                    .map(|p| p.eq_ignore_ascii_case(&prefix_slash))
                    .unwrap_or(false)
            {
                return true;
            }
        }
        false
    };

    // Also include the lowercase names of any selected current indices so matching baseline entries are found
    let mut target_keys: HashSet<String> = HashSet::new();
    if let Some(indices) = entry_indices {
        for &idx in indices {
            if let Some(d) = cur_pkz.descriptors.get(idx) {
                target_keys.insert(
                    d.name
                        .replace('\\', "/")
                        .trim_start_matches('/')
                        .to_ascii_lowercase(),
                );
            }
        }
    }

    let mut base_by_key: HashMap<String, (usize, &pgtools_core::pkz::PkzFileDescriptor)> =
        HashMap::with_capacity(base_pkz.descriptors.len());
    for (bi, bd) in base_pkz.descriptors.iter().enumerate() {
        let key = bd
            .name
            .replace('\\', "/")
            .trim_start_matches('/')
            .to_ascii_lowercase();
        base_by_key.entry(key).or_insert((bi, bd));
    }

    let mut raw_items: Vec<(String, String, u64, Vec<u8>)> =
        Vec::with_capacity(cur_pkz.descriptors.len().max(base_pkz.descriptors.len()));
    let mut handled_base_keys: HashSet<String> = HashSet::new();
    let mut reverted_count = 0usize;

    for (ci, cd) in cur_pkz.descriptors.iter().enumerate() {
        let key = cd
            .name
            .replace('\\', "/")
            .trim_start_matches('/')
            .to_ascii_lowercase();
        let in_scope = matches_scope(&cd.name, Some(ci)) || target_keys.contains(&key);

        if !in_scope {
            let slice = cur_pkz
                .raw_entry_slice(cur_bytes, ci)
                .map_err(|e| e.to_string())?
                .to_vec();
            raw_items.push((cd.name.clone(), cd.compression.clone(), cd.size, slice));
            handled_base_keys.insert(key);
        } else if let Some(&(bi, bd)) = base_by_key.get(&key) {
            let base_slice = base_pkz
                .raw_entry_slice(base_bytes, bi)
                .map_err(|e| e.to_string())?
                .to_vec();
            raw_items.push((bd.name.clone(), bd.compression.clone(), bd.size, base_slice));
            handled_base_keys.insert(key);
            reverted_count += 1;
        } else {
            // Newly added entry in scope -> drop it on revert
            reverted_count += 1;
        }
    }

    // Restore any baseline entries in scope that were deleted in `cur_pkz`
    for (bi, bd) in base_pkz.descriptors.iter().enumerate() {
        let key = bd
            .name
            .replace('\\', "/")
            .trim_start_matches('/')
            .to_ascii_lowercase();
        if handled_base_keys.contains(&key) {
            continue;
        }
        if matches_scope(&bd.name, None) || target_keys.contains(&key) {
            let base_slice = base_pkz
                .raw_entry_slice(base_bytes, bi)
                .map_err(|e| e.to_string())?
                .to_vec();
            raw_items.push((bd.name.clone(), bd.compression.clone(), bd.size, base_slice));
            handled_base_keys.insert(key);
            reverted_count += 1;
        }
    }

    Ok((cur_pkz.pack_raw_slices(&raw_items), reverted_count))
}



pub(crate) fn compute_reorder_target(
    cur_idx: usize,
    len: usize,
    payload: &serde_json::Value,
) -> Result<usize, String> {
    if len <= 1 {
        return Err("Container has fewer than 2 entries".into());
    }
    if let Some(to) = payload.get("to_index").and_then(|v| v.as_u64()) {
        return Ok((to as usize).min(len - 1));
    }
    let dir = payload
        .get("direction")
        .and_then(|v| v.as_str())
        .unwrap_or("up");
    if dir == "up" {
        if cur_idx == 0 {
            return Err("Already at top".into());
        }
        Ok(cur_idx - 1)
    } else {
        if cur_idx + 1 >= len {
            return Err("Already at bottom".into());
        }
        Ok(cur_idx + 1)
    }
}
