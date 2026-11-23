use crate::session::{lock_session, record_mutation, AppState};
use crate::texture_cmd::build_texture_node_view;
use crate::vfs::{
    build_outline_from_session, compute_pkz_modified_map, detect_file_format_tag, resolve_subblob,
    split_container_and_leaf, write_back_subblob_with_align, DocumentOutline,
};
use crate::wmb_scene::{build_wmb1_section_view, build_wmb3_section_view};
use pgtools_core::{
    bt_formats::{
        AttFile, ColFileSummary, CosFile, EstFile, FlsFile, LitFile, MdbFile, MsqFile, ObiFile,
        XadFile,
    },
    bxm::BxmFile,
    cloth::{ClhFile, ClpFile, ClwFile},
    dat::DatFile,
    databases::{
        BAYO2_COMBAT_HEAD_BONE_MAP, BAYO2_CUTSCENE_HEAD_BONE_MAP, BAYO2_PL0000_TO_BAYO_PL0010,
        BAYO2_PL0010_TO_BAYO_PL0010, BAYO2_TO_BAYO1_COMMON_BONES, NIER_TO_BAYO1_COMMON_BONES,
    },
    eff::EffFile,
    exp::ExpFile,
    io::Endian,
    mot::MotFile,
    pkz::{OodleLibrary, PkzArchive},
    scr::ScrFile,
    wmb::WmbFile,
    wmb3::Wmb3File,
    wtb::{WtbFile, WtbGameProfile},
};
use std::collections::BTreeMap;
use std::path::Path;
use tauri::State;

pub(crate) fn format_hex_preview(bytes: &[u8], max_bytes: usize, include_ascii: bool) -> String {
    bytes
        .iter()
        .take(max_bytes)
        .collect::<Vec<_>>()
        .chunks(16)
        .enumerate()
        .map(|(row, ch)| {
            let hex: Vec<String> = ch.iter().map(|b| format!("{b:02X}")).collect();
            if include_ascii {
                let ascii: String = ch
                    .iter()
                    .map(|&&b| {
                        if (0x20..=0x7E).contains(&b) {
                            b as char
                        } else {
                            '.'
                        }
                    })
                    .collect();
                format!("{:04X}: {:<47}  {}", row * 16, hex.join(" "), ascii)
            } else {
                format!("{:04X}: {}", row * 16, hex.join(" "))
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub(crate) fn decode_csv_bytes_auto(bytes: &[u8]) -> String {
    if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        return String::from_utf8_lossy(&bytes[3..]).into_owned();
    }
    if bytes.starts_with(&[0xFF, 0xFE]) {
        let u16s: Vec<u16> = bytes[2..]
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        return String::from_utf16_lossy(&u16s);
    }
    if bytes.starts_with(&[0xFE, 0xFF]) {
        let u16s: Vec<u16> = bytes[2..]
            .chunks_exact(2)
            .map(|c| u16::from_be_bytes([c[0], c[1]]))
            .collect();
        return String::from_utf16_lossy(&u16s);
    }
    if let Ok(s) = std::str::from_utf8(bytes) {
        return s.to_string();
    }
    // Shift-JIS / CP932 safe fallback so multi-byte trailing bytes (including 0x5C / 0x2C) don't corrupt CSV columns
    let mut out = String::with_capacity(bytes.len());
    let mut i = 0usize;
    while i < bytes.len() {
        let b = bytes[i];
        if b < 0x80 {
            out.push(b as char);
            i += 1;
        } else if (0xA1..=0xDF).contains(&b) {
            // Half-width katakana U+FF61..U+FF9F
            if let Some(ch) = char::from_u32(0xFF61 + (b - 0xA1) as u32) {
                out.push(ch);
            }
            i += 1;
        } else if ((0x81..=0x9F).contains(&b) || (0xE0..=0xFC).contains(&b))
            && i + 1 < bytes.len()
        {
            let b2 = bytes[i + 1];
            out.push_str(&format!("\\x{b:02X}{b2:02X}"));
            i += 2;
        } else {
            out.push_str(&format!("\\x{b:02X}"));
            i += 1;
        }
    }
    out
}

pub(crate) fn parse_csv_table(text: &str) -> (Vec<String>, Vec<Vec<String>>) {
    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
    let mut rows: Vec<Vec<String>> = Vec::new();
    for line in normalized.lines() {
        if line.trim().is_empty() && rows.is_empty() {
            continue;
        }
        let mut row = Vec::new();
        let mut cur = String::new();
        let mut in_quotes = false;
        let mut chars = line.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '"' {
                if in_quotes && chars.peek() == Some(&'"') {
                    cur.push('"');
                    let _ = chars.next();
                } else {
                    in_quotes = !in_quotes;
                }
            } else if c == ',' && !in_quotes {
                row.push(cur.clone());
                cur.clear();
            } else {
                cur.push(c);
            }
        }
        row.push(cur);
        rows.push(row);
    }
    if rows.is_empty() {
        return (vec!["Col 1".to_string()], Vec::new());
    }
    let max_cols = rows.iter().map(|r| r.len()).max().unwrap_or(1).max(1);
    let mut headers = rows.remove(0);
    while headers.len() < max_cols {
        headers.push(format!("Col {}", headers.len() + 1));
    }
    for (i, h) in headers.iter_mut().enumerate() {
        if h.trim().is_empty() {
            *h = format!("Col {}", i + 1);
        }
    }
    for r in &mut rows {
        while r.len() < max_cols {
            r.push(String::new());
        }
    }
    (headers, rows)
}

pub(crate) fn serialize_csv_table(headers: &[String], rows: &[Vec<String>]) -> Vec<u8> {
    let escape = |cell: &str| -> String {
        if cell.contains(',') || cell.contains('"') || cell.contains('\n') {
            format!("\"{}\"", cell.replace('"', "\"\""))
        } else {
            cell.to_string()
        }
    };
    let mut out = String::new();
    if !headers.is_empty() {
        let hdr_line: Vec<String> = headers.iter().map(|h| escape(h)).collect();
        out.push_str(&hdr_line.join(","));
        out.push('\n');
    }
    for r in rows {
        let line: Vec<String> = r.iter().map(|c| escape(c)).collect();
        out.push_str(&line.join(","));
        out.push('\n');
    }
    out.into_bytes()
}

#[tauri::command]
pub fn inspect_asset_node(
    node_id: String,
    mip_index: Option<u32>,
    layer_index: Option<u32>,
    channel: Option<String>,
    opaque: Option<bool>,
    b2_override: Option<bool>,
    state: State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    let sess = lock_session(&state);
    let snap = sess.current.as_ref().ok_or("No active document loaded")?;
    let doc_name = sess
        .doc_path
        .as_ref()
        .and_then(|p| p.file_name())
        .and_then(|s| s.to_str())
        .unwrap_or("document");
    let oodle_path_opt = sess.settings.oodle_path();
    let nvtt_path_opt = sess.settings.nvtt_path();
    let req_layer = layer_index.unwrap_or(0);

    let (sub_name, sub_bytes, sub_wtp, leaf) = resolve_subblob(snap, &node_id, oodle_path_opt)?;
    let display_name = if sub_name == "root" {
        doc_name.to_string()
    } else {
        sub_name.clone()
    };

    let tag = detect_file_format_tag(&display_name, &sub_bytes);
    if tag == "UVD" || leaf.starts_with("uvdgrp:") {
        return crate::uvd_cmd::build_uvd_inspect_payload(
            snap,
            &node_id,
            doc_name,
            &display_name,
            &sub_bytes,
            &leaf,
            mip_index.unwrap_or(0),
            req_layer,
            channel.as_deref().unwrap_or("RGBA"),
            opaque.unwrap_or(false),
            sess.settings.uvd_padding,
            sess.settings.uvd_max_size,
            oodle_path_opt,
            nvtt_path_opt,
        );
    }
    if let Some(tex_idx_str) = leaf.strip_prefix("tex:") {
        let tex_idx: usize = tex_idx_str.parse().map_err(|_| "Invalid texture index")?;
        let wtb = WtbFile::from_bytes(&sub_bytes, sub_wtp.as_deref()).map_err(|e| e.to_string())?;
        let entry = wtb
            .entries
            .get(tex_idx)
            .ok_or_else(|| format!("Texture #{tex_idx} out of bounds"))?;
        let view = build_texture_node_view(
            &node_id,
            tex_idx,
            &format!("{doc_name} / {display_name}"),
            &entry.data,
            entry.flags,
            entry.texture_id,
            &entry.extension,
            wtb.game_profile,
            mip_index.unwrap_or(0),
            req_layer,
            channel.as_deref().unwrap_or("RGBA"),
            opaque.unwrap_or(false),
            nvtt_path_opt,
        );
        return Ok(serde_json::json!({
            "view_type": "TEXTURE",
            "texture": view,
        }));
    }

    if matches!(tag, "DDS" | "GTX" | "XT1" | "BNTX") {
        let profile = match tag {
            "XT1" => WtbGameProfile::SwitchXt1,
            "GTX" => WtbGameProfile::Bayo2WiiUGtx,
            "BNTX" => WtbGameProfile::Bayo2SwitchBntx,
            _ => WtbGameProfile::Bayo1PcDds,
        };
        let view = build_texture_node_view(
            &node_id,
            0,
            &format!("{doc_name}"),
            &sub_bytes,
            0,
            None,
            &format!(".{}", tag.to_ascii_lowercase()),
            profile,
            mip_index.unwrap_or(0),
            req_layer,
            channel.as_deref().unwrap_or("RGBA"),
            opaque.unwrap_or(false),
            nvtt_path_opt,
        );
        return Ok(serde_json::json!({
            "view_type": "TEXTURE",
            "texture": view,
        }));
    }

    if tag == "WTB" && leaf.is_empty() {
        let wtb = WtbFile::from_bytes(&sub_bytes, sub_wtp.as_deref()).map_err(|e| e.to_string())?;
        if !wtb.entries.is_empty() {
            let e = &wtb.entries[0];
            let first_id = if node_id == "root" || node_id.is_empty() {
                "tex:0".to_string()
            } else {
                format!("{node_id}/tex:0")
            };
            let view = build_texture_node_view(
                &first_id,
                0,
                &format!("{doc_name} / {display_name}"),
                &e.data,
                e.flags,
                e.texture_id,
                &e.extension,
                wtb.game_profile,
                mip_index.unwrap_or(0),
                req_layer,
                channel.as_deref().unwrap_or("RGBA"),
                opaque.unwrap_or(false),
                nvtt_path_opt,
            );
            return Ok(serde_json::json!({
                "view_type": "TEXTURE",
                "texture": view,
            }));
        }
    }
    if tag == "WMB" {
        let wmb = WmbFile::from_bytes(&sub_bytes).map_err(|e| e.to_string())?;
        let section_key = leaf.strip_prefix("wmb:").unwrap_or("header");
        let wmb_view =
            build_wmb1_section_view(snap, &node_id, &display_name, &wmb, section_key, b2_override, oodle_path_opt);
        return Ok(serde_json::json!({
            "view_type": "WMB",
            "wmb": wmb_view,
        }));
    }

    if tag == "WMB3" {
        let wmb3 = Wmb3File::from_bytes(&sub_bytes).map_err(|e| e.to_string())?;
        let section_key = leaf.strip_prefix("wmb3:").unwrap_or("header");
        let wmb3_view = build_wmb3_section_view(
            snap,
            &node_id,
            &display_name,
            &wmb3,
            section_key,
            oodle_path_opt,
        );
        return Ok(serde_json::json!({
            "view_type": "WMB",
            "wmb": wmb3_view,
        }));
    }
    if tag == "BXM"
        || display_name.to_ascii_lowercase().ends_with(".bxm")
        || display_name.to_ascii_lowercase().ends_with(".xml")
        || display_name.to_ascii_lowercase().ends_with(".gad")
    {
        let bxm = BxmFile::from_bytes(&sub_bytes).map_err(|e| e.to_string())?;
        let xml_text = bxm.to_xml_string();
        return Ok(serde_json::json!({
            "view_type": "BXM",
            "node_id": node_id,
            "title": display_name,
            "tag": bxm.tag,
            "flags": bxm.flags,
            "root": bxm.root,
            "xml": xml_text,
            "size_bytes": sub_bytes.len(),
        }));
    }
    if tag == "CSV" || display_name.to_ascii_lowercase().ends_with(".csv") {
        let text = decode_csv_bytes_auto(&sub_bytes);
        let (headers, rows) = parse_csv_table(&text);
        return Ok(serde_json::json!({
            "view_type": "CSV",
            "node_id": node_id,
            "title": display_name,
            "headers": headers,
            "rows": rows,
            "size_bytes": sub_bytes.len(),
        }));
    }
    if tag == "EST" || display_name.to_ascii_lowercase().ends_with(".est") {
        if let Ok(est) = EstFile::from_bytes(&sub_bytes) {
            return Ok(serde_json::json!({
                "view_type": "EST",
                "node_id": node_id,
                "title": display_name,
                "spec_line": format!("{} · {:?} endian · {} records · {} B", est.version_label, est.endian, est.records.len(), sub_bytes.len()),
                "est": est,
                "size_bytes": sub_bytes.len(),
            }));
        }
    }
    if tag == "CLP" || display_name.to_ascii_lowercase().ends_with(".clp") {
        let clp = ClpFile::from_bytes(&sub_bytes).map_err(|e| e.to_string())?;
        return Ok(serde_json::json!({
            "view_type": "CLOTH",
            "cloth_kind": "CLP",
            "node_id": node_id,
            "title": display_name,
            "endian": format!("{:?}", clp.endian),
            "data": clp,
        }));
    }
    if tag == "CLH" || display_name.to_ascii_lowercase().ends_with(".clh") {
        let clh = ClhFile::from_bytes(&sub_bytes).map_err(|e| e.to_string())?;
        return Ok(serde_json::json!({
            "view_type": "CLOTH",
            "cloth_kind": "CLH",
            "node_id": node_id,
            "title": display_name,
            "endian": format!("{:?}", clh.endian),
            "data": clh,
        }));
    }
    if tag == "CLW" || display_name.to_ascii_lowercase().ends_with(".clw") {
        let clw = ClwFile::from_bytes(&sub_bytes).map_err(|e| e.to_string())?;
        return Ok(serde_json::json!({
            "view_type": "CLOTH",
            "cloth_kind": "CLW",
            "node_id": node_id,
            "title": display_name,
            "endian": format!("{:?}", clw.endian),
            "data": clw,
        }));
    }
    if tag == "MOT" {
        let mot = MotFile::from_bytes(&sub_bytes).map_err(|e| e.to_string())?;
        return Ok(serde_json::json!({
            "view_type": "RAW_STRUCT",
            "struct_kind": "MOT",
            "node_id": node_id,
            "title": display_name,
            "spec_line": format!("{} · {:?} endian · {} frames · {} bone tracks", if mot.is_bayo2 { "MOT2 / B3MOT" } else { "MOT1" }, mot.endian, mot.frame_count, mot.records.len()),
            "data": mot,
        }));
    }
    if tag == "EXP" {
        let exp = ExpFile::from_bytes(&sub_bytes).map_err(|e| e.to_string())?;
        return Ok(serde_json::json!({
            "view_type": "RAW_STRUCT",
            "struct_kind": "EXP",
            "node_id": node_id,
            "title": display_name,
            "spec_line": format!("EXP · {:?} endian · {} bytes", exp.endian(), sub_bytes.len()),
            "data": exp,
        }));
    }
    if tag == "COS" {
        let cos = CosFile::from_bytes(&sub_bytes).map_err(|e| e.to_string())?;
        return Ok(serde_json::json!({
            "view_type": "RAW_STRUCT",
            "struct_kind": "COS",
            "node_id": node_id,
            "title": display_name,
            "spec_line": format!("COS · {} costume color palettes", cos.entries.len()),
            "data": cos,
        }));
    }
    if tag == "ATT" {
        let att = AttFile::from_bytes(&sub_bytes).map_err(|e| e.to_string())?;
        return Ok(serde_json::json!({
            "view_type": "RAW_STRUCT",
            "struct_kind": "ATT",
            "node_id": node_id,
            "title": display_name,
            "spec_line": format!("ATT · {} bone attachment points", att.points.len()),
            "data": att,
        }));
    }
    if tag == "OBI" {
        let obi = ObiFile::from_bytes(&sub_bytes).map_err(|e| e.to_string())?;
        return Ok(serde_json::json!({
            "view_type": "RAW_STRUCT",
            "struct_kind": "OBI",
            "node_id": node_id,
            "title": display_name,
            "spec_line": format!("OBI · {} object placement instances", obi.entries.len()),
            "data": obi,
        }));
    }
    if tag == "MSQ" {
        let msq = MsqFile::from_bytes(&sub_bytes).map_err(|e| e.to_string())?;
        return Ok(serde_json::json!({
            "view_type": "RAW_STRUCT",
            "struct_kind": "MSQ",
            "node_id": node_id,
            "title": display_name,
            "spec_line": format!("MSQ · {} sequence events", msq.events.len()),
            "data": msq,
        }));
    }
    if tag == "COL" {
        let col = ColFileSummary::from_bytes(&sub_bytes).map_err(|e| e.to_string())?;
        return Ok(serde_json::json!({
            "view_type": "RAW_STRUCT",
            "struct_kind": "COL",
            "node_id": node_id,
            "title": display_name,
            "spec_line": format!("COL · {} collision groups · {} vertices · {} indices", col.groups.len(), col.num_vertices, col.num_indices),
            "data": col,
        }));
    }
    if tag == "FLS" {
        let fls = FlsFile::from_bytes(&sub_bytes).map_err(|e| e.to_string())?;
        return Ok(serde_json::json!({
            "view_type": "RAW_STRUCT",
            "struct_kind": "FLS",
            "node_id": node_id,
            "title": display_name,
            "spec_line": format!("FLS · {} stage lighting filters & fog banks", fls.filters.len()),
            "data": fls,
        }));
    }
    if tag == "XAD" {
        let xad = XadFile::from_bytes(&sub_bytes).map_err(|e| e.to_string())?;
        return Ok(serde_json::json!({
            "view_type": "RAW_STRUCT",
            "struct_kind": "XAD",
            "node_id": node_id,
            "title": display_name,
            "spec_line": format!("XAD (.fla / .cta) · {} lighting / cubemap trigger areas", xad.entries.len()),
            "data": xad,
        }));
    }
    if tag == "LIT" {
        let lit = LitFile::from_bytes(&sub_bytes).map_err(|e| e.to_string())?;
        return Ok(serde_json::json!({
            "view_type": "RAW_STRUCT",
            "struct_kind": "LIT",
            "node_id": node_id,
            "title": display_name,
            "spec_line": format!("LIT · {} stage lights & shadow/sun lights", lit.lights.len()),
            "data": lit,
        }));
    }
    if tag == "MDB" {
        let mdb = MdbFile::from_bytes(&sub_bytes).map_err(|e| e.to_string())?;
        return Ok(serde_json::json!({
            "view_type": "RAW_STRUCT",
            "struct_kind": "MDB",
            "node_id": node_id,
            "title": display_name,
            "spec_line": format!("MDB · {} positions · {} bones · {} meshes", mdb.num_positions, mdb.num_bones, mdb.meshes.len()),
            "data": mdb,
        }));
    }

    // Container overview (DAT / DTT / PKZ / EFF / SCR / raw binary)
    let mut container_entries = Vec::new();
    let mut pkz_info = serde_json::Value::Null;
    if tag == "DAT" {
        if let Ok(dat) = DatFile::from_bytes(&sub_bytes) {
            for (i, e) in dat.entries.iter().enumerate() {
                container_entries.push(serde_json::json!({
                    "index": i,
                    "name": e.name,
                    "extension": e.extension,
                    "size": e.data.len(),
                }));
            }
        }
    } else if tag == "PKZ" {
        if let Ok(pkz) = PkzArchive::from_bytes(&sub_bytes) {
            let (container_parts, _) = split_container_and_leaf(&node_id);
            let container_id = container_parts.join("/");
            let baseline_bytes = sess
                .saved_snapshot
                .as_ref()
                .and_then(|bs| resolve_subblob(bs, &container_id, oodle_path_opt).ok())
                .map(|(_, b, _, _)| b);
            let (mod_map, staged_count) =
                compute_pkz_modified_map(&sub_bytes, &pkz, baseline_bytes.as_deref());
            let oodle_lib = OodleLibrary::load(oodle_path_opt).ok();
            let oodle_found = oodle_lib
                .as_ref()
                .map(|l| l.resolved_path.clone())
                .or_else(|| OodleLibrary::find_dll_path(oodle_path_opt));
            let oodle_can_compress = oodle_lib.as_ref().map(|l| l.can_compress()).unwrap_or(false);

            for (i, d) in pkz.descriptors.iter().enumerate() {
                let fmt_label = Path::new(&d.name)
                    .extension()
                    .and_then(|e| e.to_str())
                    .filter(|s| !s.is_empty())
                    .map(|s| s.to_ascii_uppercase())
                    .unwrap_or_else(|| "BIN".to_string());
                let is_mod = mod_map.get(&i).copied().unwrap_or(false);
                container_entries.push(serde_json::json!({
                    "index": i,
                    "name": d.name,
                    "format": fmt_label,
                    "compression": d.compression,
                    "size": d.size,
                    "compressed_size": d.compressed_size,
                    "offset": d.offset,
                    "modified": is_mod,
                }));
            }
            pkz_info = serde_json::json!({
                "is_lowercase_magic": pkz.is_lowercase_magic,
                "is_32byte_descriptors": pkz.is_32byte_descriptors,
                "uses_oodle": pkz.uses_oodle(),
                "oodle_available": true,
                "oodle_fallback_active": oodle_found.is_none(),
                "oodle_can_compress": oodle_can_compress,
                "oodle_path": oodle_found.map(|p| p.to_string_lossy().into_owned()).unwrap_or_else(|| "Built-in Pure-Rust Oodle Kraken (ooz) Fallback".into()),
                "staged_count": staged_count,
                "default_compression": sess.settings.pkz_default_compression,
                "default_level": sess.settings.pkz_default_level,
            });
        }
    } else if tag == "EFF" {
        if let Ok(eff) = EffFile::from_bytes(&sub_bytes) {
            for (di, d) in eff.directories.iter().enumerate() {
                for (fi, f) in d.files.iter().enumerate() {
                    container_entries.push(serde_json::json!({
                        "index": fi,
                        "dir_index": di,
                        "dir_name": format!("{} (#{})", d.name, d.dir_id),
                        "name": f.file_name,
                        "size": f.data.len(),
                    }));
                }
            }
        }
    } else if tag == "SCR" {
        if let Ok(scr) = ScrFile::from_bytes(&sub_bytes) {
            for (mi, meta) in scr.models_metadata.iter().enumerate() {
                container_entries.push(serde_json::json!({
                    "index": mi,
                    "name": format!("{:03}_{}.wmb", mi, meta.name),
                    "extension": "wmb",
                    "size": scr.models.get(mi).map(|b| b.len()).unwrap_or(0),
                    "translation": [meta.transform[0], meta.transform[1], meta.transform[2]],
                    "rotation": [meta.transform[3], meta.transform[4], meta.transform[5]],
                    "scale": [meta.transform[6], meta.transform[7], meta.transform[8]],
                }));
            }
        }
    }

    let hex_preview = format_hex_preview(&sub_bytes, 512, true);

    let folder_prefix = leaf.strip_prefix("pkzfolder:").map(|s| s.to_string());
    Ok(serde_json::json!({
        "view_type": "CONTAINER",
        "node_id": node_id,
        "title": if let Some(ref fp) = folder_prefix { format!("{display_name} / {fp}") } else { display_name },
        "format": tag,
        "folder_prefix": folder_prefix,
        "size_bytes": sub_bytes.len(),
        "entries": container_entries,
        "pkz_info": pkz_info,
        "hex_preview": hex_preview,
    }))
}

#[tauri::command]
pub fn mutate_bxm_node(
    node_id: String,
    xml_content: String,
    tag: String,
    state: State<'_, AppState>,
) -> Result<DocumentOutline, String> {
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
    let orig_flags = resolve_subblob(&snap, &container_id, oodle_opt)
        .ok()
        .and_then(|(_, b, _, _)| BxmFile::from_bytes(&b).ok())
        .map(|b| b.flags)
        .unwrap_or(0);
    let mut bxm = BxmFile::from_xml_str(&xml_content, &tag).map_err(|e| e.to_string())?;
    bxm.flags = orig_flags;
    let dat_align = sess.settings.effective_dat_alignment();
    let new_bytes = bxm.to_bytes(Endian::Big);
    let next_snap = write_back_subblob_with_align(&snap, &container_parts, new_bytes, None, oodle_opt, dat_align)?;
    record_mutation(&mut sess, "Edit BXM XML Tree", next_snap);
    build_outline_from_session(&sess)
}

#[tauri::command]
pub fn mutate_struct_node(
    node_id: String,
    struct_kind: String,
    updated_data: serde_json::Value,
    state: State<'_, AppState>,
) -> Result<DocumentOutline, String> {
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

    let (new_bytes, desc) = match struct_kind.as_str() {
        "CSV" => {
            let headers_val = updated_data
                .get("headers")
                .ok_or("Missing CSV headers field")?;
            let rows_val = updated_data
                .get("rows")
                .ok_or("Missing CSV rows field")?;
            let headers: Vec<String> =
                serde_json::from_value(headers_val.clone()).map_err(|e| format!("Invalid CSV headers: {e}"))?;
            let rows: Vec<Vec<String>> =
                serde_json::from_value(rows_val.clone()).map_err(|e| format!("Invalid CSV rows: {e}"))?;
            (
                serialize_csv_table(&headers, &rows),
                "Update CSV Table".to_string(),
            )
        }
        "EST" => {
            let est: EstFile = serde_json::from_value(updated_data).map_err(|e| e.to_string())?;
            (
                est.apply_to_bytes(&sub_bytes),
                "Update EST Particle Effect Records".to_string(),
            )
        }
        "COS" => {
            let cos: CosFile = serde_json::from_value(updated_data).map_err(|e| e.to_string())?;
            (
                cos.rebuild(&sub_bytes),
                "Update COS Costume Colors".to_string(),
            )
        }
        "ATT" => {
            let att: AttFile = serde_json::from_value(updated_data).map_err(|e| e.to_string())?;
            (
                att.to_bytes(),
                "Update ATT Bone Attachment Points".to_string(),
            )
        }
        "OBI" => {
            let obi: ObiFile = serde_json::from_value(updated_data).map_err(|e| e.to_string())?;
            (
                obi.rebuild(&sub_bytes),
                "Update OBI Object Placement".to_string(),
            )
        }
        "MSQ" => {
            let msq: MsqFile = serde_json::from_value(updated_data).map_err(|e| e.to_string())?;
            (
                msq.rebuild(&sub_bytes),
                "Update MSQ Sequence Events".to_string(),
            )
        }
        "COL" => {
            let col: ColFileSummary =
                serde_json::from_value(updated_data).map_err(|e| e.to_string())?;
            (
                col.apply_to_bytes(&sub_bytes),
                "Update COL Collision Groups".to_string(),
            )
        }
        "FLS" => {
            let fls: FlsFile = serde_json::from_value(updated_data).map_err(|e| e.to_string())?;
            (
                fls.apply_to_bytes(&sub_bytes),
                "Update FLS Stage Filters & Fog".to_string(),
            )
        }
        "XAD" => {
            let xad: XadFile = serde_json::from_value(updated_data).map_err(|e| e.to_string())?;
            (
                xad.apply_to_bytes(&sub_bytes),
                "Update XAD (.fla/.cta) Trigger Areas".to_string(),
            )
        }
        "LIT" => {
            let lit: LitFile = serde_json::from_value(updated_data).map_err(|e| e.to_string())?;
            (
                lit.apply_to_bytes(&sub_bytes),
                "Update LIT Stage Lights".to_string(),
            )
        }
        "MDB" => {
            let mdb: MdbFile = serde_json::from_value(updated_data).map_err(|e| e.to_string())?;
            (
                mdb.apply_to_bytes(&sub_bytes),
                "Update MDB Model Meshes".to_string(),
            )
        }
        "MOT" => {
            let mot: MotFile = serde_json::from_value(updated_data).map_err(|e| e.to_string())?;
            (
                mot.to_bytes(mot.endian),
                "Update MOT Motion Tracks".to_string(),
            )
        }
        "EXP" => {
            let exp: ExpFile = serde_json::from_value(updated_data).map_err(|e| e.to_string())?;
            (
                exp.to_bytes(exp.endian()),
                "Update EXP Expression Constraints".to_string(),
            )
        }
        other => return Err(format!("Unsupported struct_kind for mutation: {other}")),
    };

    let dat_align = sess.settings.effective_dat_alignment();
    let next_snap = write_back_subblob_with_align(&snap, &container_parts, new_bytes, None, oodle_opt, dat_align)?;
    record_mutation(&mut sess, desc, next_snap);
    build_outline_from_session(&sess)
}

#[tauri::command]
pub fn mutate_cloth_node(
    node_id: String,
    operation: String,
    preset: Option<String>,
    updated_json: Option<serde_json::Value>,
    state: State<'_, AppState>,
) -> Result<DocumentOutline, String> {
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
    let (sub_name, sub_bytes, _, _) = resolve_subblob(&snap, &container_id, oodle_opt)?;
    let fname = sub_name.to_ascii_lowercase();

    let build_remap = |p: &str| -> Result<BTreeMap<i32, i32>, String> {
        let slice = match p {
            "bayo2_pl0000_to_bayo1" => BAYO2_PL0000_TO_BAYO_PL0010,
            "bayo2_pl0010_to_bayo1" => BAYO2_PL0010_TO_BAYO_PL0010,
            "bayo2_combat_head" => BAYO2_COMBAT_HEAD_BONE_MAP,
            "bayo2_cutscene_head" => BAYO2_CUTSCENE_HEAD_BONE_MAP,
            "bayo2_common" => BAYO2_TO_BAYO1_COMMON_BONES,
            "nier_to_bayo1" => NIER_TO_BAYO1_COMMON_BONES,
            other => return Err(format!("Unknown bone remap preset: {other}")),
        };
        Ok(slice.iter().map(|&(k, v)| (k as i32, v as i32)).collect())
    };

    let (new_bytes, desc) = if fname.ends_with(".clp")
        || detect_file_format_tag(&fname, &sub_bytes) == "CLP"
    {
        let mut clp = if let Some(val) = updated_json {
            serde_json::from_value::<ClpFile>(val).map_err(|e| e.to_string())?
        } else {
            ClpFile::from_bytes(&sub_bytes).map_err(|e| e.to_string())?
        };
        let mut endian = clp.endian;
        let d = match operation.as_str() {
            "swap_endian" => {
                endian = clp.endian.swap();
                format!("Swap CLP Endianness to {endian:?}")
            }
            "remap_bones" => {
                let pr = preset.as_deref().unwrap_or("bayo2_pl0010_to_bayo1");
                let map = build_remap(pr)?;
                clp.remap(&map, true, true).map_err(|e| e.to_string())?;
                format!("Remap CLP Bones ({pr})")
            }
            _ => "Update CLP Cloth Parameters".to_string(),
        };
        (clp.to_bytes(endian), d)
    } else if fname.ends_with(".clh") || detect_file_format_tag(&fname, &sub_bytes) == "CLH" {
        let mut clh = if let Some(val) = updated_json {
            serde_json::from_value::<ClhFile>(val).map_err(|e| e.to_string())?
        } else {
            ClhFile::from_bytes(&sub_bytes).map_err(|e| e.to_string())?
        };
        let mut endian = clh.endian;
        let d = match operation.as_str() {
            "swap_endian" => {
                endian = clh.endian.swap();
                format!("Swap CLH Endianness to {endian:?}")
            }
            "remap_bones" => {
                let pr = preset.as_deref().unwrap_or("bayo2_pl0010_to_bayo1");
                let map = build_remap(pr)?;
                clh.remap(&map).map_err(|e| e.to_string())?;
                format!("Remap CLH Capsule Bones ({pr})")
            }
            _ => "Update CLH Collision Capsules".to_string(),
        };
        (clh.to_bytes(endian), d)
    } else {
        let mut clw = if let Some(val) = updated_json {
            serde_json::from_value::<ClwFile>(val).map_err(|e| e.to_string())?
        } else {
            ClwFile::from_bytes(&sub_bytes).map_err(|e| e.to_string())?
        };
        let mut endian = clw.endian;
        let d = match operation.as_str() {
            "swap_endian" => {
                endian = clw.endian.swap();
                format!("Swap CLW Endianness to {endian:?}")
            }
            "remap_bones" => {
                let pr = preset.as_deref().unwrap_or("bayo2_pl0010_to_bayo1");
                let map = build_remap(pr)?;
                clw.remap(&map).map_err(|e| e.to_string())?;
                format!("Remap CLW Wind Bones ({pr})")
            }
            _ => "Update CLW Wind Entries".to_string(),
        };
        (clw.to_bytes(endian), d)
    };

    let dat_align = sess.settings.effective_dat_alignment();
    let next_snap = write_back_subblob_with_align(&snap, &container_parts, new_bytes, None, oodle_opt, dat_align)?;
    record_mutation(&mut sess, desc, next_snap);
    build_outline_from_session(&sess)
}
