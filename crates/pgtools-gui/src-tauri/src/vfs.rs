use crate::session::{
    lock_session, record_mutation, AppState, DocSnapshot, StudioSession,
};
use pgtools_core::{
    bxm::BxmFile,
    dat::DatFile,
    eff::{EffDirectory, EffFile, EffSubFile},
    io::{sanitize_entry_path, Endian},
    pkz::{OodleLibrary, PkzArchive},
    scr::ScrFile,
    wmb::WmbFile,
    wmb3::Wmb3File,
    wtb::WtbFile,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tauri::State;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssetTreeNode {
    pub id: String,
    pub label: String,
    pub kind: String,
    pub format_badge: String,
    pub size: usize,
    pub children: Vec<AssetTreeNode>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentOutline {
    pub path: String,
    pub paired_path: Option<String>,
    pub file_name: String,
    pub root_format: String,
    pub endian: String,
    pub summary_right: String,
    pub dirty: bool,
    pub can_undo: bool,
    pub can_redo: bool,
    pub undo_label: Option<String>,
    pub redo_label: Option<String>,
    pub tree: Vec<AssetTreeNode>,
}

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
    )
}

pub(crate) fn is_bntx_blob(bytes: &[u8]) -> bool {
    (bytes.len() >= 4 && &bytes[0..4] == b"BNTX")
        || (bytes.len() >= 84 && &bytes[80..84] == b"BNTX")
}

pub(crate) fn find_pkz_peer_dtt_index(pkz: &PkzArchive, entry_name: &str) -> Option<usize> {
    if !entry_name.to_ascii_lowercase().ends_with(".dat") || entry_name.len() < 4 {
        return None;
    }
    let stem = &entry_name[..entry_name.len() - 4];
    let target_dtt = format!("{stem}.dtt");
    pkz.descriptors
        .iter()
        .position(|d| d.name.eq_ignore_ascii_case(&target_dtt))
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
    let tag = detect_file_format_tag(name, bytes);
    let mut children = Vec::new();

    if depth < 6 {
        match tag {
            "DAT" => {
                if let Ok(dat) = DatFile::from_bytes(bytes) {
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
                        children.push(build_nodes_for_blob(
                            &child_id,
                            &entry.name,
                            &entry.data,
                            pair_wtp.as_deref(),
                            None,
                            oodle_dll,
                            depth + 1,
                        ));
                    }
                }
            }
            "PKZ" => {
                if let Ok(pkz) = PkzArchive::from_bytes(bytes) {
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
                            "dat" | "dtt" | "evn" | "eff" | "idd" | "scr" | "wtb" | "wta" | "wtp" | "wmb" | "bntx"
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
                            nodes.push(AssetTreeNode {
                                id: folder_id,
                                label: folder_name.to_string(),
                                kind: "PKZ_FOLDER".into(),
                                format_badge: "DIR".into(),
                                size: 0,
                                children: Vec::new(),
                            });
                            nodes.last_mut().unwrap()
                        };
                        folder_node.size += leaf_node.size;
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

                        let leaf_node = if let Some(unpacked) = unpacked_cache.get(&idx) {
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
                                children: Vec::new(),
                            }
                        };
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
                        children: Vec::new(),
                    });
                    children.push(AssetTreeNode {
                        id: format!("{base}:vertexes"),
                        label: format!("Vertexes ({} verts)", wmb.vertexes.len()),
                        kind: "WMB_VERTEXES".into(),
                        format_badge: "VTX".into(),
                        size: wmb.vertexes.len() * 32,
                        children: Vec::new(),
                    });
                    children.push(AssetTreeNode {
                        id: format!("{base}:bones"),
                        label: format!("Bones ({} bones)", wmb.bone_hierarchy.len()),
                        kind: "WMB_BONES".into(),
                        format_badge: "SKEL".into(),
                        size: wmb.bone_hierarchy.len() * 16,
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
                        children: Vec::new(),
                    });
                    children.push(AssetTreeNode {
                        id: format!("{base}:bones"),
                        label: format!("Bones ({})", wmb3.bones.len()),
                        kind: "WMB3_BONES".into(),
                        format_badge: "SKEL".into(),
                        size: wmb3.bones.len() * 88,
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
                            children: Vec::new(),
                        })
                        .collect();
                    children.push(AssetTreeNode {
                        id: format!("{base}:materials"),
                        label: format!("Materials ({})", wmb3.materials.len()),
                        kind: "WMB3_MATERIALS".into(),
                        format_badge: "MAT3".into(),
                        size: wmb3.materials.len() * 48,
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
                        children: Vec::new(),
                    });
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
    let (mut cur_bytes, mut cur_wtp, mut cur_dtt_peer) = match snap {
        DocSnapshot::SingleFile(b) => (b.as_ref().clone(), None, None),
        DocSnapshot::SplitWtaWtp { wta, wtp } => {
            (wta.as_ref().clone(), Some(wtp.as_ref().clone()), None)
        }
        DocSnapshot::PairedDatDtt { dat, dtt } => {
            (dat.as_ref().clone(), None, Some(dtt.as_ref().clone()))
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
                cur_dtt_peer = Some(dtt.as_ref().clone());
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
            let peer_dtt = find_pkz_peer_dtt_index(&pkz, &desc.name).and_then(|d_idx| pkz.extract_entry_with_lib(&cur_bytes, d_idx, oodle_lib.as_ref()).ok());
            cur_name = desc.name.clone();
            cur_bytes = pkz
                .extract_entry_with_lib(&cur_bytes, entry_idx, oodle_lib.as_ref())
                .map_err(|e| e.to_string())?;
            cur_wtp = None;
            cur_dtt_peer = peer_dtt;
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
        parts[container_end]
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
pub(crate) fn write_back_subblob(
    snap: &DocSnapshot,
    container_parts: &[&str],
    new_bytes: Vec<u8>,
    new_wtp: Option<Vec<u8>>,
    oodle_dll: Option<&Path>,
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
                let updated_child = write_back_subblob(
                    &DocSnapshot::single(child_orig),
                    &rest[1..],
                    new_bytes.clone(),
                    new_wtp.clone(),
                    oodle_dll,
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
                            dtt_modified = update_matching_wtp_in_dat(
                                &entry_name,
                                wta_ord,
                                &mut parsed_dtt,
                                wtp_bytes,
                            );
                        }
                    }
                }
                return Ok(DocSnapshot::PairedDatDtt {
                    dat: Arc::new(parsed_dat.to_bytes()),
                    dtt: if dtt_modified {
                        Arc::new(parsed_dtt.to_bytes())
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
            let updated_dtt = write_back_subblob(
                &DocSnapshot::SingleFile(dtt.clone()),
                rest,
                new_bytes.clone(),
                new_wtp.clone(),
                oodle_dll,
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
        let updated_child = write_back_subblob(
            &DocSnapshot::single(child_orig),
            rest,
            new_bytes.clone(),
            new_wtp.clone(),
            oodle_dll,
        )?;
        if let DocSnapshot::SingleFile(cb) = updated_child {
            dat.entries[entry_idx].data = cb.as_ref().clone();
        }
        let mut updated_dtt_bytes = None;
        if rest.is_empty() {
            if let Some(wtp_bytes) = new_wtp {
                if !update_matching_wtp_in_dat(&entry_name, wta_ord, &mut dat, &wtp_bytes) {
                    if let DocSnapshot::PairedDatDtt { dtt, .. } = snap {
                        if let Ok(mut parsed_dtt) = DatFile::from_bytes(dtt) {
                            if update_matching_wtp_in_dat(
                                &entry_name,
                                wta_ord,
                                &mut parsed_dtt,
                                &wtp_bytes,
                            ) {
                                updated_dtt_bytes = Some(Arc::new(parsed_dtt.to_bytes()));
                            }
                        }
                    }
                }
            }
        }
        return Ok(match snap {
            DocSnapshot::PairedDatDtt { dtt, .. } => DocSnapshot::PairedDatDtt {
                dat: Arc::new(dat.to_bytes()),
                dtt: updated_dtt_bytes.unwrap_or_else(|| dtt.clone()),
            },
            _ => DocSnapshot::single(dat.to_bytes()),
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
            if let Ok(dtt_orig) = pkz.extract_entry_with_lib(root_bytes, dtt_idx, oodle_lib.as_ref()) {
                let mut child_parts = Vec::with_capacity(rest.len() + 1);
                child_parts.push("root:dat");
                child_parts.extend_from_slice(rest);
                let paired_snap = DocSnapshot::paired(child_orig.clone(), dtt_orig.clone());
                let updated_paired = write_back_subblob(
                    &paired_snap,
                    &child_parts,
                    new_bytes.clone(),
                    new_wtp.clone(),
                    oodle_dll,
                )?;
                if let DocSnapshot::PairedDatDtt { dat: new_dat, dtt: new_dtt } = updated_paired {
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

        let updated_child = write_back_subblob(
            &DocSnapshot::single(child_orig),
            rest,
            new_bytes.clone(),
            new_wtp.clone(),
            oodle_dll,
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
                let updated_child = write_back_subblob(
                    &DocSnapshot::single(child_orig),
                    &rest[1..],
                    new_bytes.clone(),
                    new_wtp.clone(),
                    oodle_dll,
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
        let updated_child = write_back_subblob(
            &DocSnapshot::single(child_orig),
            rest,
            new_bytes.clone(),
            None,
            oodle_dll,
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

pub(crate) fn build_outline_from_session(sess: &StudioSession) -> Result<DocumentOutline, String> {
    let path = sess
        .doc_path
        .as_ref()
        .ok_or("No document currently loaded")?;
    let snap = sess
        .current
        .as_ref()
        .ok_or("No document snapshot in memory")?;

    let oodle_path_opt = sess.settings.oodle_path();

    let file_name = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("document")
        .to_string();

    if let DocSnapshot::PairedDatDtt { dat, dtt } = snap {
        let dtt_name = sess
            .dtt_path
            .as_ref()
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
            paired_path: sess
                .dtt_path
                .as_ref()
                .map(|p| p.to_string_lossy().into_owned()),
            file_name: format!("{file_name} + {dtt_name}"),
            root_format: "DAT+DTT".into(),
            endian: endian.into(),
            summary_right,
            dirty: sess.dirty,
            can_undo: !sess.undo_stack.is_empty(),
            can_redo: !sess.redo_stack.is_empty(),
            undo_label: sess.undo_stack.last().map(|u| u.description.clone()),
            redo_label: sess.redo_stack.last().map(|u| u.description.clone()),
            tree: vec![dat_node, dtt_node],
        });
    }

    let (main_bytes, wtp_bytes) = match snap {
        DocSnapshot::SingleFile(b) => (b.as_slice(), None),
        DocSnapshot::SplitWtaWtp { wta, wtp } => (wta.as_slice(), Some(wtp.as_slice())),
        DocSnapshot::PairedDatDtt { dat, .. } => (dat.as_slice(), None),
    };

    let root_format = detect_file_format_tag(&file_name, main_bytes).to_string();
    let root_node = build_nodes_for_blob(
        "",
        &file_name,
        main_bytes,
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
        "PKZ" => format!("PkzArchive · {} entries", root_node.children.len()),
        "EFF" => format!("EffectContainer · {} dirs", root_node.children.len()),
        "WMB" | "WMB3" => format!("{root_format} · {} sections", root_node.children.len()),
        _ => format!("{root_format} · {} B", main_bytes.len()),
    };

    Ok(DocumentOutline {
        path: path.to_string_lossy().into_owned(),
        paired_path: sess
            .wtp_path
            .as_ref()
            .map(|p| p.to_string_lossy().into_owned()),
        file_name,
        root_format,
        endian: endian.into(),
        summary_right,
        dirty: sess.dirty,
        can_undo: !sess.undo_stack.is_empty(),
        can_redo: !sess.redo_stack.is_empty(),
        undo_label: sess.undo_stack.last().map(|u| u.description.clone()),
        redo_label: sess.redo_stack.last().map(|u| u.description.clone()),
        tree: vec![root_node],
    })
}

/// Validate a user-supplied filesystem path used for export/import.
/// Requires a non-empty absolute path without `..` components; for writes the
/// parent directory must already exist.
fn validate_user_fs_path(path_str: &str, for_write: bool) -> Result<PathBuf, String> {
    let trimmed = path_str.trim();
    if trimmed.is_empty() {
        return Err("Empty filesystem path".into());
    }
    let p = PathBuf::from(trimmed);
    if !p.is_absolute() {
        return Err(format!(
            "Path must be absolute: '{}'",
            p.display()
        ));
    }
    for comp in p.components() {
        if matches!(comp, std::path::Component::ParentDir) {
            return Err(format!(
                "Path must not contain '..': '{}'",
                p.display()
            ));
        }
    }
    if for_write {
        let parent = p.parent().ok_or_else(|| {
            format!("Cannot write to filesystem root path '{}'", p.display())
        })?;
        if !parent.exists() {
            return Err(format!(
                "Parent directory does not exist: '{}'",
                parent.display()
            ));
        }
    } else if !p.is_file() {
        return Err(format!(
            "Path is not an existing file: '{}'",
            p.display()
        ));
    }
    Ok(p)
}

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
            let next_snap = write_back_subblob(&snap, &container_parts, new_bytes, None, oodle_opt)?;
            record_mutation(&mut sess, format!("Import XML into {node_id}"), next_snap);
            build_outline_from_session(&sess)
        }
        "set_pkz_compression" => {
            let (container_parts, _) = split_container_and_leaf(&node_id);
            let container_id = container_parts.join("/");
            let (_, sub_bytes, _, _) = resolve_subblob(&snap, &container_id, oodle_opt)?;
            let pkz = PkzArchive::from_bytes(&sub_bytes).map_err(|e| e.to_string())?;
            let requested_codec = payload_val
                .get("compression")
                .and_then(|v| v.as_str())
                .unwrap_or(&target_path)
                .trim();
            let allow_fallback = payload_val
                .get("allow_zstd_fallback")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            if requested_codec.eq_ignore_ascii_case("OodleKraken")
                && !allow_fallback
                && OodleLibrary::load(oodle_opt).is_err()
            {
                return Err("OODLE_DLL_MISSING: oo2core_*_win64.dll is required to compress with OodleKraken.".into());
            }
            let entry_indices: Option<Vec<usize>> = payload_val
                .get("entry_indices")
                .and_then(|v| serde_json::from_value(v.clone()).ok());
            let mut overrides = HashMap::new();
            if let Some(indices) = entry_indices {
                for idx in indices {
                    overrides.insert(idx, requested_codec.to_string());
                }
            } else {
                for idx in 0..pkz.descriptors.len() {
                    overrides.insert(idx, requested_codec.to_string());
                }
            }
            let repacked = pkz
                .repack_with_options(
                    &sub_bytes,
                    &HashMap::new(),
                    &[],
                    &[],
                    None,
                    None,
                    Some(&overrides),
                    None,
                    oodle_opt,
                )
                .map_err(|e| e.to_string())?;
            let next_snap = write_back_subblob(&snap, &container_parts, repacked, None, oodle_opt)?;
            record_mutation(
                &mut sess,
                format!("Set PKZ Compression to {requested_codec}"),
                next_snap,
            );
            build_outline_from_session(&sess)
        }
        "extract_pkz_folder" => {
            let (container_parts, leaf) = split_container_and_leaf(&node_id);
            let folder_prefix = payload_val
                .get("folder_prefix")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .or_else(|| leaf.strip_prefix("pkzfolder:").map(|s| s.to_string()))
                .ok_or("Missing PKZ folder prefix")?;
            let container_id = container_parts.join("/");
            let (_, sub_bytes, _, _) = resolve_subblob(&snap, &container_id, oodle_opt)?;
            let pkz = PkzArchive::from_bytes(&sub_bytes).map_err(|e| e.to_string())?;
            let out_dir = PathBuf::from(&target_path);
            fs::create_dir_all(&out_dir).map_err(|e| e.to_string())?;
            let prefix_slash = format!("{}/", folder_prefix.trim_matches('/'));
            let oodle_lib = OodleLibrary::load(oodle_opt).ok();
            for (idx, desc) in pkz.descriptors.iter().enumerate() {
                let norm = desc.name.replace('\\', "/");
                let clean = norm.trim_start_matches('/');
                if clean.starts_with(&prefix_slash) || clean == folder_prefix.trim_matches('/') {
                    let fallback = format!("entry_{idx}.bin");
                    let dest_file = sanitize_entry_path(&out_dir, clean, &fallback)
                        .map_err(|e| e.to_string())?;
                    let data = pkz
                        .extract_entry_with_lib(&sub_bytes, idx, oodle_lib.as_ref())
                        .map_err(|e| e.to_string())?;
                    if let Some(p) = dest_file.parent() {
                        fs::create_dir_all(p).map_err(|e| e.to_string())?;
                    }
                    fs::write(dest_file, data).map_err(|e| e.to_string())?;
                }
            }
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
            let prefix_slash = format!("{}/", folder_prefix.trim_matches('/'));

            let (repacked, desc) = if action == "remove_pkz_folder" {
                let mut removed = Vec::new();
                for (idx, d) in pkz.descriptors.iter().enumerate() {
                    let norm = d.name.replace('\\', "/");
                    let clean = norm.trim_start_matches('/');
                    if clean.starts_with(&prefix_slash) || clean == folder_prefix.trim_matches('/') {
                        removed.push(idx);
                    }
                }
                let r = pkz
                    .repack_with_options(&sub_bytes, &HashMap::new(), &[], &removed, None, None, None, None, oodle_opt)
                    .map_err(|e| e.to_string())?;
                (r, format!("Remove PKZ folder '{folder_prefix}' ({} files)", removed.len()))
            } else if action == "rename_pkz_folder" {
                let new_prefix = payload_val
                    .get("new_prefix")
                    .and_then(|v| v.as_str())
                    .unwrap_or(&target_path)
                    .trim()
                    .trim_matches('/')
                    .to_string();
                let mut renamed = HashMap::new();
                for (idx, d) in pkz.descriptors.iter().enumerate() {
                    let norm = d.name.replace('\\', "/");
                    let clean = norm.trim_start_matches('/');
                    if let Some(rest) = clean.strip_prefix(&prefix_slash) {
                        let updated = if new_prefix.is_empty() {
                            rest.to_string()
                        } else {
                            format!("{new_prefix}/{rest}")
                        };
                        renamed.insert(idx, updated);
                    }
                }
                let r = pkz
                    .repack_with_options(&sub_bytes, &HashMap::new(), &[], &[], None, None, None, Some(&renamed), oodle_opt)
                    .map_err(|e| e.to_string())?;
                (r, format!("Rename PKZ folder '{folder_prefix}' -> '{new_prefix}'"))
            } else {
                let idx = single_pkz_idx
                    .or_else(|| payload_val.get("entry_index").and_then(|v| v.as_u64()).map(|v| v as usize))
                    .ok_or("Missing PKZ entry index to move")?;
                let target_folder = payload_val
                    .get("target_folder")
                    .and_then(|v| v.as_str())
                    .unwrap_or(&target_path)
                    .trim()
                    .trim_matches('/');
                let old_name = pkz.descriptors.get(idx).ok_or("PKZ index out of bounds")?.name.clone();
                let base_name = old_name.replace('\\', "/").split('/').last().unwrap_or(&old_name).to_string();
                let new_name = if target_folder.is_empty() {
                    base_name
                } else {
                    format!("{target_folder}/{base_name}")
                };
                let mut renamed = HashMap::new();
                renamed.insert(idx, new_name.clone());
                let r = pkz
                    .repack_with_options(&sub_bytes, &HashMap::new(), &[], &[], None, None, None, Some(&renamed), oodle_opt)
                    .map_err(|e| e.to_string())?;
                (r, format!("Move PKZ entry #{idx} to '{new_name}'"))
            };
            let next_snap = write_back_subblob(&snap, &container_parts, repacked, None, oodle_opt)?;
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
            let (container_parts, _) = split_container_and_leaf(&node_id);
            let new_bytes = fs::read(&target_path).map_err(|e| e.to_string())?;
            let next_snap =
                write_back_subblob(&snap, &container_parts, new_bytes, None, oodle_opt)?;
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
                    pkz.extract_all(&sub_bytes, &out_dir, oodle_opt)
                        .map_err(|e| e.to_string())?;
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
                        dat.to_bytes(),
                        format!("Add {} file(s) to DAT", paths.len()),
                    )
                }
                "PKZ" => {
                    let pkz = PkzArchive::from_bytes(&sub_bytes).map_err(|e| e.to_string())?;
                    let folder_prefix = payload_val
                        .get("folder_prefix")
                        .and_then(|v| v.as_str())
                        .map(|s| s.trim_matches('/').to_string())
                        .or_else(|| leaf.strip_prefix("pkzfolder:").map(|s| s.trim_matches('/').to_string()))
                        .unwrap_or_default();
                    let mut added = Vec::new();
                    for p_str in &paths {
                        let p = Path::new(p_str);
                        let base_fname = p
                            .file_name()
                            .and_then(|s| s.to_str())
                            .unwrap_or("new_file.dat");
                        let fname = if folder_prefix.is_empty() {
                            base_fname.to_string()
                        } else {
                            format!("{folder_prefix}/{base_fname}")
                        };
                        let data = fs::read(p).map_err(|e| e.to_string())?;
                        added.push((fname, data));
                    }
                    let repacked = pkz
                        .repack_with_replacements(
                            &sub_bytes,
                            &HashMap::new(),
                            &added,
                            &[],
                            None,
                            None,
                            oodle_opt,
                        )
                        .map_err(|e| e.to_string())?;
                    (repacked, format!("Add {} file(s) to PKZ", paths.len()))
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
                        let stem = fname.split('.').next().unwrap_or("");
                        let id = if let Some(hex) = stem.strip_prefix("0x").or_else(|| stem.strip_prefix("0X")) {
                            u32::from_str_radix(hex, 16).ok()
                        } else {
                            stem.parse::<u32>().ok().or_else(|| u32::from_str_radix(stem, 16).ok())
                        }.unwrap_or(dir.files.len() as u32);
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

            let next_snap = write_back_subblob(
                &snap,
                &container_parts,
                new_container_bytes,
                None,
                oodle_opt,
            )?;
            sess.invalidate_texture_caches();
            record_mutation(&mut sess, desc, next_snap);
            build_outline_from_session(&sess)
        }
        "remove_entry" | "rename_entry" | "reorder_entry" => {
            let (mut container_parts, _) = split_container_and_leaf(&node_id);
            if container_parts.is_empty() {
                return Err("Cannot remove or reorder the root container itself".into());
            }
            let last_seg = container_parts.pop().unwrap();
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
                            (dat.to_bytes(), format!("Remove {} from DAT", removed.name))
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
                            (dat.to_bytes(), format!("Rename DAT entry to {new_name}"))
                        }
                        "reorder_entry" => {
                            let to_idx =
                                compute_reorder_target(idx, dat.entries.len(), &payload_val)?;
                            let item = dat.entries.remove(idx);
                            dat.entries.insert(to_idx, item);
                            (
                                dat.to_bytes(),
                                format!("Reorder DAT entry #{idx} -> #{to_idx}"),
                            )
                        }
                        _ => unreachable!(),
                    }
                } else if let Some(pkz_idx_str) = last_seg.strip_prefix("pkz:") {
                    let idx: usize = pkz_idx_str.parse().map_err(|_| "Invalid pkz index")?;
                    let mut pkz =
                        PkzArchive::from_bytes(&parent_bytes).map_err(|e| e.to_string())?;
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
                            let new_name = payload_val
                                .get("new_name")
                                .and_then(|v| v.as_str())
                                .unwrap_or(&target_path)
                                .trim()
                                .to_string();
                            pkz.descriptors[idx].name = new_name.clone();
                            let repacked = pkz
                                .repack_with_replacements(
                                    &parent_bytes,
                                    &HashMap::new(),
                                    &[],
                                    &[],
                                    None,
                                    None,
                                    oodle_opt,
                                )
                                .map_err(|e| e.to_string())?;
                            (repacked, format!("Rename PKZ entry to {new_name}"))
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

            let next_snap = write_back_subblob(
                &snap,
                &container_parts,
                new_parent_bytes,
                None,
                oodle_opt,
            )?;
            sess.invalidate_texture_caches();
            record_mutation(&mut sess, desc, next_snap);
            build_outline_from_session(&sess)
        }
        _ => Err(format!("Unsupported container node action: {action}")),
    }
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
