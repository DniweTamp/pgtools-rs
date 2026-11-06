use crate::formats_cmd::format_hex_preview;
use crate::session::{lock_session, record_mutation, AppState};
use crate::vfs::{
    build_outline_from_session, is_bntx_blob, resolve_subblob, split_container_and_leaf,
    write_back_subblob, DocumentOutline,
};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use pgtools_core::{
    texture_conv::{
        build_dummy_dds, decode_bntx_subtexture_mip_filtered, decode_texture_mip_filtered,
        encode_rgba8_to_dds, encode_rgba_to_dds_with_nvtt, encode_rgba_to_xt1,
        encode_rgba_to_xt1_with_nvtt, inspect_texture_blob, parse_bntx_textures,
        replace_bntx_subtexture, swizzle_dds_to_xt1, TextureFormat, ASTC_FOOTPRINTS,
    },
    wtb::{WtbFile, WtbGameProfile},
};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use tauri::State;

pub(crate) fn is_raster_image_path(low_path: &str) -> bool {
    low_path.ends_with(".png")
        || low_path.ends_with(".jpg")
        || low_path.ends_with(".jpeg")
        || low_path.ends_with(".tga")
        || low_path.ends_with(".bmp")
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MipLevelDesc {
    pub index: u32,
    pub label: String,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TextureNodeView {
    pub node_id: String,
    pub index: usize,
    pub title: String,
    pub breadcrumb: String,
    pub spec_line: String,
    pub validation_ok: bool,
    pub validation_msg: String,
    pub format_name: String,
    pub container_ext: String,
    pub game_profile: String,
    pub has_confirmed_xt1_flags: bool,
    pub width: u32,
    pub height: u32,
    pub mip_count: u32,
    pub mips: Vec<MipLevelDesc>,
    pub size_bytes: usize,
    pub flags_hex: String,
    pub table2_bytes_str: String,
    pub has_crc32_flag: bool,
    pub srgb_flag: bool,
    pub cubemap_flag: bool,
    pub texture_id_hex: String,
    pub preview_data_url: Option<String>,
    pub raw_header_hex: String,
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn build_texture_node_view(
    node_id: &str,
    index: usize,
    breadcrumb: &str,
    blob: &[u8],
    flags: u32,
    texture_id: Option<u32>,
    ext: &str,
    game_profile: WtbGameProfile,
    mip_index: u32,
    channel: &str,
    opaque: bool,
) -> TextureNodeView {
    let is_bntx = is_bntx_blob(blob);

    let (format_name, width, height, mip_count, sub_name) = if is_bntx {
        if let Ok(recs) = parse_bntx_textures(blob) {
            let rec = recs.get(index).unwrap_or(&recs[0]);
            (
                rec.format.display_name(),
                rec.width,
                rec.height,
                rec.mip_count.max(1) as u32,
                Some(rec.name.clone()),
            )
        } else {
            ("Unknown".to_string(), 0, 0, 1, None)
        }
    } else {
        match inspect_texture_blob(blob) {
            Ok(m) => (
                m.format.display_name(),
                m.width,
                m.height,
                m.mip_count.max(1),
                m.name,
            ),
            Err(_) => ("Unknown".to_string(), 0, 0, 1, None),
        }
    };

    let mut mips = Vec::new();
    let mut mw = width.max(1);
    let mut mh = height.max(1);
    for m in 0..mip_count {
        mips.push(MipLevelDesc {
            index: m,
            label: format!("mip {m} ({mw}x{mh})"),
            width: mw,
            height: mh,
        });
        mw = (mw / 2).max(1);
        mh = (mh / 2).max(1);
    }

    let decode_res = if is_bntx {
        decode_bntx_subtexture_mip_filtered(blob, index, mip_index, channel, opaque)
    } else {
        decode_texture_mip_filtered(blob, mip_index, channel, opaque)
    };

    let (preview_data_url, validation_ok, validation_msg) = match decode_res {
        Ok((img, _, _)) => {
            let mut png_buf = Cursor::new(Vec::new());
            let url = if img.write_to(&mut png_buf, image::ImageFormat::Png).is_ok() {
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
                format!("Decoded cleanly ({}) — layout verified.", game_profile.label()),
            )
        }
        Err(e) => (None, false, format!("Preview decode warning: {e}")),
    };

    let fb_le = flags.to_le_bytes();
    let table2_bytes_str = format!(
        "{:02X} {:02X} {:02X} {:02X}",
        fb_le[0], fb_le[1], fb_le[2], fb_le[3]
    );
    let has_confirmed_xt1_flags = game_profile.has_confirmed_xt1_flags();
    let has_crc32_flag = (flags & 0x8000_0000) != 0 || texture_id.is_some();
    let srgb_flag = (flags & 0x0200_0000) != 0;
    let cubemap_flag = (flags & 0x0000_0200) != 0;

    let size_mib = (blob.len() as f64) / (1024.0 * 1024.0);
    let size_str = if size_mib >= 0.1 {
        format!("{size_mib:.2} MiB")
    } else {
        format!("{:.1} KiB", (blob.len() as f64) / 1024.0)
    };

    let spec_line = format!(
        "{format_name} ({ext}) | {width} × {height} × 1 | 2D | {mip_count} mip levels | {size_str}"
    );

    let raw_header_hex = format_hex_preview(blob, 128, false);

    let title = if let Some(n) = sub_name {
        format!("Texture #{index} ({n})")
    } else {
        format!("Texture #{index}")
    };

    TextureNodeView {
        node_id: node_id.to_string(),
        index,
        title,
        breadcrumb: breadcrumb.to_string(),
        spec_line,
        validation_ok,
        validation_msg,
        format_name,
        container_ext: ext.to_string(),
        game_profile: game_profile.label().to_string(),
        has_confirmed_xt1_flags,
        width,
        height,
        mip_count,
        mips,
        size_bytes: blob.len(),
        flags_hex: format!("0x{flags:08X}"),
        table2_bytes_str,
        has_crc32_flag,
        srgb_flag,
        cubemap_flag,
        texture_id_hex: format!("0x{:08X}", texture_id.unwrap_or(0)),
        preview_data_url,
        raw_header_hex,
    }
}

fn parse_requested_texture_format(fmt_str: &str, fallback: TextureFormat) -> TextureFormat {
    let up = fmt_str.trim().to_ascii_uppercase();
    if up.is_empty() || up == "AUTO" || up.starts_with("AUTO ") {
        return fallback;
    }
    if up.starts_with("ASTC_") {
        let srgb = up.ends_with("_SRGB");
        for &(bw, bh, tag) in &ASTC_FOOTPRINTS {
            if up.contains(&tag.to_ascii_uppercase()) {
                return TextureFormat::Astc(bw, bh, srgb);
            }
        }
        return TextureFormat::Astc(4, 4, srgb);
    }
    match up.as_str() {
        "BC1" | "BC1_UNORM" | "DXT1" => TextureFormat::Bc1,
        "BC1_SRGB" => TextureFormat::Bc1Srgb,
        "BC2" | "BC2_UNORM" | "DXT3" => TextureFormat::Bc2,
        "BC2_SRGB" => TextureFormat::Bc2Srgb,
        "BC3" | "BC3_UNORM" | "DXT5" => TextureFormat::Bc3,
        "BC3_SRGB" => TextureFormat::Bc3Srgb,
        "BC4" | "BC4_UNORM" | "ATI1" => TextureFormat::Bc4,
        "BC5" | "BC5_UNORM" | "ATI2" => TextureFormat::Bc5,
        "BC6H" | "BC6H_UF16" => TextureFormat::Bc6hUf16,
        "BC6H_SF16" => TextureFormat::Bc6hSf16,
        "BC7" | "BC7_UNORM" => TextureFormat::Bc7,
        "BC7_SRGB" => TextureFormat::Bc7Srgb,
        "RGBA8" | "RGBA8_UNORM" => TextureFormat::Rgba8,
        "RGBA8_SRGB" => TextureFormat::Rgba8Srgb,
        "BGRA8" | "BGRA8_UNORM" => TextureFormat::Bgra8,
        "BGRA8_SRGB" => TextureFormat::Bgra8Srgb,
        "R8" | "R8_UNORM" | "AF_L8_UNORM" => TextureFormat::AfL8Unorm,
        "RG8" | "RG8_UNORM" | "R8G8_UNORM" => TextureFormat::Rg8Unorm,
        "RGBA16F" | "RGBA16_FLOAT" => TextureFormat::Rgba16Float,
        _ => fallback,
    }
}

fn encode_image_for_bank_entry(
    base_img: &image::RgbaImage,
    target_ext: &str,
    target_format: TextureFormat,
    generate_mips: bool,
    apply_solidify: bool,
    make_opaque: bool,
    template_xt1_header: Option<&[u8]>,
    nvtt_dll: Option<&Path>,
) -> Vec<u8> {
    if target_ext == ".xt1" {
        return encode_rgba_to_xt1_with_nvtt(
            base_img,
            target_format,
            generate_mips,
            apply_solidify,
            make_opaque,
            template_xt1_header,
            nvtt_dll,
        );
    }
    let enc_fmt = match target_format {
        TextureFormat::Unknown(_) => TextureFormat::Bc3,
        other => other,
    };
    encode_rgba_to_dds_with_nvtt(
        base_img,
        enc_fmt,
        generate_mips,
        apply_solidify,
        make_opaque,
        nvtt_dll,
    )
}

#[tauri::command]
pub fn mutate_texture_node(
    node_id: String,
    action: String,
    payload: serde_json::Value,
    state: State<'_, AppState>,
) -> Result<DocumentOutline, String> {
    let mut sess = lock_session(&state);
    let snap = sess.current.clone().ok_or("No active document loaded")?;
    let oodle_path = sess.settings.oodle_dll_path.clone();
    let nvtt_path = sess.settings.nvtt_dll_path.clone();
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
    let (_, sub_bytes, sub_wtp, _) = resolve_subblob(&snap, &container_id, oodle_opt)?;

    let tex_idx: usize = leaf
        .strip_prefix("tex:")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);

    if leaf.is_empty() && (sub_bytes.starts_with(b"DDS ") || sub_bytes.starts_with(b"XT1\0")) {
        if action == "replace_file" {
            let file_path = payload.get("path").and_then(|v| v.as_str()).ok_or("Missing replacement file path")?;
            let generate_mips = payload.get("generate_mips").and_then(|v| v.as_bool()).unwrap_or(true);
            let apply_solidify = payload.get("solidify").and_then(|v| v.as_bool()).unwrap_or(true);
            let make_opaque = payload.get("make_opaque").and_then(|v| v.as_bool()).unwrap_or(false);
            let fmt_str = payload.get("encode_format").and_then(|v| v.as_str()).unwrap_or("Auto");
            let existing_meta = inspect_texture_blob(&sub_bytes).ok();
            let existing_fmt = existing_meta.as_ref().map(|m| m.format).unwrap_or(TextureFormat::Bc3);
            let target_fmt = parse_requested_texture_format(fmt_str, existing_fmt);
            let target_ext = if sub_bytes.starts_with(b"XT1\0") { ".xt1" } else { ".dds" };
            let raw = fs::read(file_path).map_err(|e| e.to_string())?;
            let low_path = file_path.to_ascii_lowercase();
            let new_blob = if is_raster_image_path(&low_path) {
                let img = image::load_from_memory(&raw).map_err(|e| e.to_string())?.to_rgba8();
                encode_image_for_bank_entry(&img, target_ext, target_fmt, generate_mips, apply_solidify, make_opaque, Some(&sub_bytes), nvtt_opt)
            } else if target_ext == ".xt1" && raw.starts_with(b"DDS ") {
                swizzle_dds_to_xt1(&raw, Some(&sub_bytes)).map_err(|e| e.to_string())?
            } else {
                raw
            };
            let next_snap = write_back_subblob(&snap, &container_parts, new_blob, None, oodle_opt)?;
            sess.invalidate_texture_caches();
            record_mutation(&mut sess, format!("Replace standalone {target_ext} texture ({target_fmt:?})"), next_snap);
            return build_outline_from_session(&sess);
        }
    }
    let mut wtb = WtbFile::from_bytes(&sub_bytes, sub_wtp.as_deref()).map_err(|e| e.to_string())?;

    let desc = match action.as_str() {
        "set_hash" => {
            let hex_str = payload
                .get("hash")
                .and_then(|v| v.as_str())
                .unwrap_or("0")
                .trim()
                .trim_start_matches("0x")
                .trim_start_matches("0X");
            let val =
                u32::from_str_radix(hex_str, 16).map_err(|e| format!("Invalid hex hash: {e}"))?;
            let entry = wtb
                .entries
                .get_mut(tex_idx)
                .ok_or("Texture index out of range")?;
            entry.texture_id = Some(val);
            format!("Set Texture #{tex_idx} CRC32 hash to 0x{val:08X}")
        }
        "calc_crc32" => {
            let name = payload
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim()
                .to_ascii_lowercase();
            let crc = crc32fast::hash(name.as_bytes()) & 0x7FFF_FFFF;
            let entry = wtb
                .entries
                .get_mut(tex_idx)
                .ok_or("Texture index out of range")?;
            entry.texture_id = Some(crc);
            format!("Calculate CRC32 for '{name}' (0x{crc:08X})")
        }
        "set_flags" => {
            let flags_val = if let Some(hex_s) = payload.get("flags_hex").and_then(|v| v.as_str()) {
                let clean = hex_s.trim().trim_start_matches("0x").trim_start_matches("0X");
                u32::from_str_radix(clean, 16).map_err(|e| format!("Invalid flags hex: {e}"))?
            } else {
                wtb.entries.get(tex_idx).map(|e| e.flags).unwrap_or(0)
            };
            let entry = wtb
                .entries
                .get_mut(tex_idx)
                .ok_or("Texture index out of range")?;
            entry.flags = flags_val;
            format!("Update Texture #{tex_idx} flags to 0x{flags_val:08X}")
        }
        "dummy_out" => {
            let kind = payload
                .get("kind")
                .and_then(|v| v.as_str())
                .unwrap_or("white");
            let entry = wtb
                .entries
                .get_mut(tex_idx)
                .ok_or("Texture index out of range")?;
            if entry.extension == ".xt1" {
                let rgba = match kind {
                    "black" => [0u8, 0, 0, 255],
                    "normal" => [128u8, 128, 255, 255],
                    _ => [255u8, 255, 255, 255],
                };
                let mut img = image::RgbaImage::new(8, 8);
                for p in img.pixels_mut() {
                    p.0 = rgba;
                }
                let xt1 = encode_rgba_to_xt1(
                    &img,
                    TextureFormat::Bc1,
                    false,
                    false,
                    false,
                    Some(&entry.data),
                );
                entry.metadata = inspect_texture_blob(&xt1).ok();
                entry.data = xt1;
            } else {
                let dds = build_dummy_dds(kind);
                entry.metadata = inspect_texture_blob(&dds).ok();
                entry.extension = ".dds".to_string();
                entry.data = dds;
            }
            format!("Dummy out Texture #{tex_idx} ({kind})")
        }
        "replace_file" => {
            let file_path = payload
                .get("path")
                .and_then(|v| v.as_str())
                .ok_or("Missing replacement file path")?;
            let generate_mips = payload
                .get("generate_mips")
                .and_then(|v| v.as_bool())
                .unwrap_or(true);
            let apply_solidify = payload
                .get("solidify")
                .and_then(|v| v.as_bool())
                .unwrap_or(true);
            let make_opaque = payload
                .get("make_opaque")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let fmt_str = payload
                .get("encode_format")
                .and_then(|v| v.as_str())
                .unwrap_or("Auto");

            let raw = fs::read(file_path).map_err(|e| e.to_string())?;
            let entry = wtb
                .entries
                .get_mut(tex_idx)
                .ok_or("Texture index out of range")?;
            let existing_fmt = entry
                .metadata
                .as_ref()
                .map(|m| m.format)
                .unwrap_or(TextureFormat::Bc3);
            let target_fmt = parse_requested_texture_format(fmt_str, existing_fmt);

            let low_path = file_path.to_ascii_lowercase();
            let new_blob = if is_raster_image_path(&low_path)
            {
                let img = image::load_from_memory(&raw)
                    .map_err(|e| format!("Failed to decode image: {e}"))?
                    .to_rgba8();
                if entry.extension == ".bntx" {
                    replace_bntx_subtexture(
                        &entry.data,
                        tex_idx,
                        Err(&img),
                        Some(target_fmt),
                        generate_mips,
                        apply_solidify,
                        make_opaque,
                    )
                    .map_err(|e| e.to_string())?
                } else {
                    encode_image_for_bank_entry(
                        &img,
                        &entry.extension,
                        target_fmt,
                        generate_mips,
                        apply_solidify,
                        make_opaque,
                        Some(&entry.data),
                        nvtt_opt,
                    )
                }
            } else if entry.extension == ".xt1" && raw.starts_with(b"DDS ") {
                swizzle_dds_to_xt1(&raw, Some(&entry.data)).map_err(|e| e.to_string())?
            } else if entry.extension == ".bntx" && raw.starts_with(b"DDS ") {
                replace_bntx_subtexture(&entry.data, tex_idx, Ok(&raw), None, generate_mips, apply_solidify, make_opaque)
                    .map_err(|e| e.to_string())?
            } else {
                raw
            };
            entry.metadata = inspect_texture_blob(&new_blob).ok();
            if let Some(ref m) = entry.metadata {
                entry.extension = m.container_ext.clone();
            }
            entry.data = new_blob;
            format!("Replace Texture #{tex_idx} ({target_fmt:?})")
        }
        "add_texture" => {
            let file_path = payload
                .get("path")
                .and_then(|v| v.as_str())
                .ok_or("Missing file path")?;
            let generate_mips = payload
                .get("generate_mips")
                .and_then(|v| v.as_bool())
                .unwrap_or(true);
            let apply_solidify = payload
                .get("solidify")
                .and_then(|v| v.as_bool())
                .unwrap_or(true);
            let make_opaque = payload
                .get("make_opaque")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let fmt_str = payload
                .get("encode_format")
                .and_then(|v| v.as_str())
                .unwrap_or("BC3_UNORM");
            let target_fmt = parse_requested_texture_format(fmt_str, TextureFormat::Bc3);
            let bank_ext = wtb
                .entries
                .first()
                .map(|e| e.extension.clone())
                .unwrap_or_else(|| ".dds".to_string());

            let raw = fs::read(file_path).map_err(|e| e.to_string())?;
            let low_path = file_path.to_ascii_lowercase();
            let new_blob = if is_raster_image_path(&low_path)
            {
                let img = image::load_from_memory(&raw)
                    .map_err(|e| format!("Failed to decode image: {e}"))?
                    .to_rgba8();
                encode_image_for_bank_entry(
                    &img,
                    &bank_ext,
                    target_fmt,
                    generate_mips,
                    apply_solidify,
                    make_opaque,
                    wtb.entries.first().map(|e| e.data.as_slice()),
                    nvtt_opt,
                )
            } else {
                raw
            };
            let default_flags = if bank_ext == ".xt1" {
                0x8000_0000
            } else {
                0x2000_0004
            };
            wtb.push(new_blob, default_flags, Some(0))
                .map_err(|e| e.to_string())?;
            format!("Add texture #{}", wtb.entries.len() - 1)
        }
        "delete_texture" => {
            if wtb.entries.len() <= 1 {
                return Err("Cannot delete the last remaining texture in a bank".into());
            }
            wtb.entries.remove(tex_idx);
            for (i, e) in wtb.entries.iter_mut().enumerate() {
                e.index = i;
            }
            format!("Delete Texture #{tex_idx}")
        }
        "convert_wiiu_to_pc" => {
            wtb = wtb.convert_wiiu_to_pc().map_err(|e| e.to_string())?;
            "Convert WTB bank Wii U (.gtx) -> PC (.dds)".to_string()
        }
        "batch_replace_folder" => {
            let folder = payload
                .get("folder")
                .and_then(|v| v.as_str())
                .ok_or("Missing folder path")?;
            let is_bntx_container = is_bntx_blob(&sub_bytes);
            let mut shared_bntx = if is_bntx_container {
                Some(sub_bytes.clone())
            } else {
                None
            };
            let mut replaced = 0usize;
            for item in fs::read_dir(folder).map_err(|e| e.to_string())?.flatten() {
                let fname = item.file_name().to_string_lossy().to_ascii_lowercase();
                for entry in &mut wtb.entries {
                    let prefix = format!("tex{}", entry.index);
                    let hash_match = entry
                        .texture_id
                        .map(|h| fname.contains(&format!("{h:08x}")))
                        .unwrap_or(false);
                    if fname.starts_with(&prefix) || hash_match {
                        if let Ok(raw) = fs::read(item.path()) {
                            let fmt = entry
                                .metadata
                                .as_ref()
                                .map(|m| m.format)
                                .unwrap_or(TextureFormat::Bc3);
                            if let Some(ref mut bntx_buf) = shared_bntx {
                                let updated_res = if fname.ends_with(".dds") {
                                    replace_bntx_subtexture(
                                        bntx_buf,
                                        entry.index,
                                        Ok(&raw),
                                        Some(fmt),
                                        true,
                                        false,
                                        false,
                                    )
                                } else if let Ok(img) = image::load_from_memory(&raw) {
                                    let rgba = img.to_rgba8();
                                    replace_bntx_subtexture(
                                        bntx_buf,
                                        entry.index,
                                        Err(&rgba),
                                        Some(fmt),
                                        true,
                                        true,
                                        false,
                                    )
                                } else {
                                    continue;
                                };
                                if let Ok(next_bntx) = updated_res {
                                    *bntx_buf = next_bntx;
                                    replaced += 1;
                                }
                                break;
                            }
                            let blob = if is_raster_image_path(&fname) {
                                if let Ok(img) = image::load_from_memory(&raw) {
                                    encode_image_for_bank_entry(
                                        &img.to_rgba8(),
                                        &entry.extension,
                                        fmt,
                                        true,
                                        true,
                                        false,
                                        Some(&entry.data),
                                        nvtt_opt,
                                    )
                                } else {
                                    continue;
                                }
                            } else {
                                raw
                            };
                            entry.metadata = inspect_texture_blob(&blob).ok();
                            entry.data = blob;
                            replaced += 1;
                            break;
                        }
                    }
                }
            }
            if let Some(final_bntx) = shared_bntx {
                for entry in &mut wtb.entries {
                    entry.data = final_bntx.clone();
                }
            }
            format!("Batch replaced {replaced} textures from folder")
        }
        other => return Err(format!("Unknown texture mutation: {other}")),
    };

    let is_standalone_bntx = is_bntx_blob(&sub_bytes);
    let (new_wtb_bytes, new_wtp_bytes) = if is_standalone_bntx {
        (
            wtb.entries
                .get(tex_idx)
                .map(|e| e.data.clone())
                .unwrap_or(sub_bytes),
            None,
        )
    } else {
        wtb.to_bytes()
    };
    let next_snap = write_back_subblob(
        &snap,
        &container_parts,
        new_wtb_bytes,
        new_wtp_bytes,
        oodle_opt,
    )?;
    sess.invalidate_texture_caches();
    record_mutation(&mut sess, desc, next_snap);
    build_outline_from_session(&sess)
}

#[tauri::command]
pub fn export_texture_action(
    node_id: String,
    scope: String,
    format: String,
    dest_path: String,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let sess = lock_session(&state);
    let snap = sess.current.as_ref().ok_or("No active document loaded")?;
    let oodle_opt = if sess.settings.oodle_dll_path.is_empty() {
        None
    } else {
        Some(Path::new(sess.settings.oodle_dll_path.as_str()))
    };
    let (container_parts, leaf) = split_container_and_leaf(&node_id);
    let container_id = container_parts.join("/");
    let (_, sub_bytes, sub_wtp, _) = resolve_subblob(snap, &container_id, oodle_opt)?;

    let export_one =
        |blob: &[u8], sub_idx: usize, out_file: &Path, fmt: &str| -> Result<(), String> {
            let is_bntx = is_bntx_blob(blob);
            match fmt {
                "png" => {
                    let (img, _, _) = if is_bntx {
                        decode_bntx_subtexture_mip_filtered(blob, sub_idx, 0, "RGBA", false)
                            .map_err(|e| e.to_string())?
                    } else {
                        decode_texture_mip_filtered(blob, 0, "RGBA", false)
                            .map_err(|e| e.to_string())?
                    };
                    img.save(out_file).map_err(|e| e.to_string())
                }
                "dds" if !blob.starts_with(b"DDS ") => {
                    let (img, _, _) = if is_bntx {
                        decode_bntx_subtexture_mip_filtered(blob, sub_idx, 0, "RGBA", false)
                            .map_err(|e| e.to_string())?
                    } else {
                        decode_texture_mip_filtered(blob, 0, "RGBA", false)
                            .map_err(|e| e.to_string())?
                    };
                    let dds = encode_rgba8_to_dds(&img);
                    fs::write(out_file, dds).map_err(|e| e.to_string())
                }
                _ => fs::write(out_file, blob).map_err(|e| e.to_string()),
            }
        };

    if scope == "single" {
        let tex_idx: usize = leaf
            .strip_prefix("tex:")
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        let wtb = WtbFile::from_bytes(&sub_bytes, sub_wtp.as_deref()).map_err(|e| e.to_string())?;
        let entry = wtb
            .entries
            .get(tex_idx)
            .ok_or("Texture index out of range")?;
        export_one(&entry.data, tex_idx, Path::new(&dest_path), &format)?;
        return Ok(format!("Exported Texture #{tex_idx} to {dest_path}"));
    }

    let out_dir = PathBuf::from(&dest_path);
    fs::create_dir_all(&out_dir).map_err(|e| e.to_string())?;
    let wtb = WtbFile::from_bytes(&sub_bytes, sub_wtp.as_deref()).map_err(|e| e.to_string())?;
    let mut count = 0usize;
    for e in &wtb.entries {
        let hash_s = e
            .texture_id
            .map(|h| format!("_0x{h:08X}"))
            .unwrap_or_default();
        let ext = if format == "png" {
            ".png"
        } else if format == "dds" {
            ".dds"
        } else {
            e.extension.as_str()
        };
        let file_path = out_dir.join(format!("tex{}{}{}", e.index, hash_s, ext));
        if export_one(&e.data, e.index, &file_path, &format).is_ok() {
            count += 1;
        }
    }
    Ok(format!("Exported {count} textures to {}", out_dir.display()))
}
