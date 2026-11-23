use crate::common::{
    format_hex_preview, AssetTreeNode, BatchOperationReport, DocSnapshot, DocumentOutline,
    MipLevelDesc,
};
use crate::session::{atomic_write_file, build_outline_from_session, lock_session, record_mutation, AppState};
use crate::uvd_resolve::resolve_effective_texture_node;
use crate::vfs::{
    detect_file_format_tag, is_bntx_blob, resolve_subblob, split_container_and_leaf,
    write_back_subblob_with_align,
};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use pgtools_core::{
    dat::DatFile,
    scene::encode_wtx2_blob,
    texture_conv::{
        build_dummy_dds, decode_bntx_subtexture_mip_filtered, decode_cubemap_six_faces,
        decode_texture_layer_filtered, decode_texture_mip_filtered,
        encode_rgba_to_dds_with_quality, encode_rgba_to_xt1_with_options,
        export_texture_to_dds_lossless, extract_gpu_texture_surface, get_nvtt_status_description,
        inspect_texture_blob, is_gpu_direct_format, parse_bntx_textures, pick_mip_for_max_dim,
        recompress_astc_texture_blob_to_bc, replace_bntx_subtexture_with_options,
        swizzle_dds_to_xt1, texture_blob_has_pixel_payload_at, AstcRecompressConfig,
        EncodeQuality, TextureDimension, TextureFormat, ASTC_FOOTPRINTS,
    },
    wtb::{
        WtbFile, WtbGameProfile, XT1_FLAG_BASE, XT1_FLAG_CUBEMAP, XT1_FLAG_HAS_CRC32,
        XT1_FLAG_SRGB,
    },
};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Cursor;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tauri::State;

fn is_raster_image_path(low_path: &str) -> bool {
    low_path.ends_with(".png")
        || low_path.ends_with(".jpg")
        || low_path.ends_with(".jpeg")
        || low_path.ends_with(".tga")
        || low_path.ends_with(".bmp")
}

/// Parse a batch-replace filename stem (`tex<index>_0x<hash>`, `tex<index>`, or `0x<hash>`)
/// with exact digit/hex boundaries so `tex1` never falsely matches `tex10`.
pub(crate) fn parse_replacement_filename_stem(stem: &str) -> (Option<usize>, Option<u32>) {
    let low = stem.trim().to_ascii_lowercase();
    if let Some(rest) = low.strip_prefix("tex") {
        if let Some((idx_part, hash_part)) = rest.split_once("_0x") {
            if !idx_part.is_empty()
                && idx_part.chars().all(|c| c.is_ascii_digit())
                && !hash_part.is_empty()
                && hash_part.len() <= 8
                && hash_part.chars().all(|c| c.is_ascii_hexdigit())
            {
                if let (Ok(idx), Ok(hash)) =
                    (idx_part.parse::<usize>(), u32::from_str_radix(hash_part, 16))
                {
                    return (Some(idx), Some(hash));
                }
            }
        } else if !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit()) {
            if let Ok(idx) = rest.parse::<usize>() {
                return (Some(idx), None);
            }
        }
    } else if let Some(hex_part) = low.strip_prefix("0x") {
        if !hex_part.is_empty()
            && hex_part.len() <= 8
            && hex_part.chars().all(|c| c.is_ascii_hexdigit())
        {
            if let Ok(hash) = u32::from_str_radix(hex_part, 16) {
                return (None, Some(hash));
            }
        }
    }
    (None, None)
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
    pub dimension: String,
    pub width: u32,
    pub height: u32,
    pub depth: u32,
    pub layer_count: u32,
    pub current_layer: u32,
    pub mip_count: u32,
    pub mips: Vec<MipLevelDesc>,
    pub size_bytes: usize,
    pub flags_hex: String,
    pub table2_bytes_str: String,
    pub has_crc32_flag: bool,
    pub srgb_flag: bool,
    pub cubemap_flag: bool,
    pub texture_id_hex: String,
    pub embedded_name: Option<String>,
    pub nvtt_status: String,
    pub preview_data_url: Option<String>,
    #[serde(default)]
    pub gpu_direct: bool,
    pub raw_header_hex: String,
    #[serde(default)]
    pub uvd_mode: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CubemapPreviewPayload {
    pub face_data_urls: Vec<String>,
    pub auto_exposure_scale: f32,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchReplaceScanResult {
    pub bank_label: String,
    pub has_confirmed_xt1_flags: bool,
    pub matched: usize,
    pub missing: usize,
    pub ambiguous: usize,
    pub unsupported: usize,
    pub unused: usize,
    pub log_lines: Vec<String>,
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
    layer_index: u32,
    channel: &str,
    opaque: bool,
    nvtt_opt: Option<&Path>,
) -> TextureNodeView {
    let is_bntx = is_bntx_blob(blob);

    let (detected_format, format_name, dimension, width, height, depth, base_layers, mip_count, embedded_name) =
        if is_bntx {
            if let Ok(recs) = parse_bntx_textures(blob) {
                let rec = recs.get(index).unwrap_or(&recs[0]);
                let dim = if rec.surface_dim == 3 || rec.array_length == 6 {
                    TextureDimension::TexCube
                } else if rec.depth > 1 || rec.array_length > 1 {
                    TextureDimension::TexAnimatedLayers
                } else {
                    TextureDimension::Tex2D
                };
                let layers = match dim {
                    TextureDimension::TexCube => 6,
                    TextureDimension::TexAnimatedLayers => rec.depth.max(rec.array_length).max(1),
                    TextureDimension::Tex2D => 1,
                };
                (
                    rec.format,
                    rec.format.display_name(),
                    dim,
                    rec.width,
                    rec.height,
                    rec.depth.max(1),
                    layers,
                    rec.mip_count.max(1) as u32,
                    Some(rec.name.clone()),
                )
            } else {
                (
                    TextureFormat::Unknown(0),
                    "Unknown".to_string(),
                    TextureDimension::Tex2D,
                    0,
                    0,
                    1,
                    1,
                    1,
                    None,
                )
            }
        } else {
            match inspect_texture_blob(blob) {
                Ok(m) => (
                    m.format,
                    m.format.display_name(),
                    m.dimension,
                    m.width,
                    m.height,
                    m.depth.max(1),
                    m.layer_count.max(1),
                    m.mip_count.max(1),
                    m.name,
                ),
                Err(_) => (
                    TextureFormat::Unknown(0),
                    "Unknown".to_string(),
                    TextureDimension::Tex2D,
                    0,
                    0,
                    1,
                    1,
                    1,
                    None,
                ),
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

    let has_payload = texture_blob_has_pixel_payload_at(blob, Some(index));
    let allow_gpu_direct = has_payload
        && is_gpu_direct_format(detected_format)
        && !node_id.contains("uvdgrp:")
        && !node_id.contains("uvditem:")
        && width > 0
        && height > 0;
    let (preview_data_url, gpu_direct, layer_count, validation_ok, validation_msg) = if !has_payload {
        (
            None,
            false,
            base_layers,
            false,
            "Texture payload missing (header-only WTA entry without WTP/DTT pixel data).".to_string(),
        )
    } else if allow_gpu_direct {
        (
            None,
            true,
            base_layers,
            true,
            format!("GPU-direct ({}) — zero CPU decode.", game_profile.label()),
        )
    } else {
        let decode_res =
            decode_texture_layer_filtered(blob, index, mip_index, layer_index, channel, opaque);
        match decode_res {
        Ok((img, _, _, layers_at_mip)) => {
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
                false,
                layers_at_mip.max(base_layers),
                true,
                format!("Decoded cleanly ({}) — layout verified.", game_profile.label()),
            )
        }
        Err(e) => (
            None,
            false,
            base_layers,
            false,
            format!("Preview decode warning: {e}"),
        ),
        }
    };

    let fb_le = flags.to_le_bytes();
    let table2_bytes_str = format!(
        "{:02X} {:02X} {:02X} {:02X}",
        fb_le[0], fb_le[1], fb_le[2], fb_le[3]
    );
    let has_confirmed_xt1_flags = game_profile.has_confirmed_xt1_flags();
    let has_crc32_flag = if has_confirmed_xt1_flags {
        (flags & XT1_FLAG_HAS_CRC32) != 0
    } else {
        texture_id.is_some()
    };
    let srgb_flag = has_confirmed_xt1_flags && (flags & XT1_FLAG_SRGB) != 0;
    let cubemap_flag = has_confirmed_xt1_flags && (flags & XT1_FLAG_CUBEMAP) != 0;

    let size_mib = (blob.len() as f64) / (1024.0 * 1024.0);
    let size_str = if size_mib >= 0.1 {
        format!("{size_mib:.2} MiB")
    } else {
        format!("{:.1} KiB", (blob.len() as f64) / 1024.0)
    };

    let dim_label = dimension.label();
    let spec_line = format!(
        "{format_name} ({ext}) | {width} × {height} × {depth} | {dim_label} | {mip_count} mip levels | {size_str}"
    );

    let raw_header_hex = format_hex_preview(blob, 128, false);

    let title = if let Some(ref n) = embedded_name {
        format!("Texture #{index} ({n})")
    } else {
        format!("Texture #{index}")
    };

    let (_, nvtt_status) = get_nvtt_status_description(nvtt_opt);
    let current_layer = layer_index.min(layer_count.saturating_sub(1));

    TextureNodeView {
        node_id: node_id.to_string(),
        index,
        title,
        breadcrumb: format!("{breadcrumb} / {dim_label}"),
        spec_line,
        validation_ok,
        validation_msg,
        format_name,
        container_ext: ext.to_string(),
        game_profile: game_profile.label().to_string(),
        has_confirmed_xt1_flags,
        dimension: dim_label.to_string(),
        width,
        height,
        depth,
        layer_count,
        current_layer,
        mip_count,
        mips,
        size_bytes: blob.len(),
        flags_hex: format!("0x{flags:08X}"),
        table2_bytes_str,
        has_crc32_flag,
        srgb_flag,
        cubemap_flag,
        texture_id_hex: format!("0x{:08X}", texture_id.unwrap_or(0)),
        embedded_name,
        nvtt_status,
        preview_data_url,
        gpu_direct,
        raw_header_hex,
        uvd_mode: None,
    }
}

pub(crate) fn parse_requested_texture_format(fmt_str: &str, fallback: TextureFormat) -> TextureFormat {
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

#[allow(clippy::too_many_arguments)]
pub(crate) fn encode_image_for_bank_entry(
    base_img: &image::RgbaImage,
    target_ext: &str,
    target_format: TextureFormat,
    mip_count_override: Option<u32>,
    generate_mips: bool,
    apply_solidify: bool,
    make_opaque: bool,
    quality: EncodeQuality,
    template_xt1_header: Option<&[u8]>,
    nvtt_dll: Option<&Path>,
) -> Vec<u8> {
    if target_ext == ".xt1" {
        return encode_rgba_to_xt1_with_options(
            base_img,
            target_format,
            mip_count_override,
            generate_mips,
            apply_solidify,
            make_opaque,
            quality,
            template_xt1_header,
            nvtt_dll,
        );
    }
    let enc_fmt = match target_format {
        TextureFormat::Unknown(_) => TextureFormat::Bc3,
        other => other,
    };
    encode_rgba_to_dds_with_quality(
        base_img,
        enc_fmt,
        mip_count_override,
        generate_mips,
        apply_solidify,
        make_opaque,
        quality,
        nvtt_dll,
    )
}

#[tauri::command]
pub fn get_cubemap_faces_preview(
    node_id: String,
    mip_index: u32,
    opaque: bool,
    state: State<'_, AppState>,
) -> Result<CubemapPreviewPayload, String> {
    let sess = lock_session(&state);
    let snap = sess.current.as_ref().ok_or("No active document loaded")?;
    let oodle_opt = sess.settings.oodle_path();
    let effective_node_id = resolve_effective_texture_node(snap, &node_id, oodle_opt);
    let (container_parts, leaf) = split_container_and_leaf(&effective_node_id);
    let container_id = container_parts.join("/");
    let (_, sub_bytes, sub_wtp, _) = resolve_subblob(snap, &container_id, oodle_opt)?;

    let (blob, sub_idx) = if leaf.is_empty()
        && (sub_bytes.starts_with(b"DDS ")
            || sub_bytes.starts_with(b"XT1\0")
            || sub_bytes.starts_with(b"Gfx2")
            || is_bntx_blob(&sub_bytes))
    {
        (sub_bytes, 0usize)
    } else {
        let tex_idx: usize = leaf
            .strip_prefix("tex:")
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        let wtb = WtbFile::from_bytes(&sub_bytes, sub_wtp.as_deref()).map_err(|e| e.to_string())?;
        let entry = wtb
            .entries
            .get(tex_idx)
            .ok_or("Texture index out of range")?;
        (entry.data.clone(), tex_idx)
    };

    let (faces, auto_exposure_scale) =
        decode_cubemap_six_faces(&blob, sub_idx, mip_index, opaque).map_err(|e| e.to_string())?;
    let w = faces.first().map(|f| f.width()).unwrap_or(1);
    let h = faces.first().map(|f| f.height()).unwrap_or(1);
    let mut face_data_urls = Vec::with_capacity(faces.len());
    for face in faces {
        let mut png_buf = Cursor::new(Vec::new());
        face.write_to(&mut png_buf, image::ImageFormat::Png)
            .map_err(|e| e.to_string())?;
        face_data_urls.push(format!(
            "data:image/png;base64,{}",
            BASE64.encode(png_buf.into_inner())
        ));
    }

    Ok(CubemapPreviewPayload {
        face_data_urls,
        auto_exposure_scale,
        width: w,
        height: h,
    })
}

/// Resolve a texture node's raw container blob and sub-index for direct GPU surface extraction.
fn resolve_texture_blob_for_node(
    snap: &DocSnapshot,
    node_id: &str,
    oodle_opt: Option<&Path>,
) -> Result<(Vec<u8>, usize, u32), String> {
    let (container_parts, leaf) = split_container_and_leaf(node_id);
    let container_id = container_parts.join("/");
    let (_, sub_bytes, sub_wtp, _) = resolve_subblob(snap, &container_id, oodle_opt)?;
    let tex_idx: usize = leaf
        .strip_prefix("tex:")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    if leaf.is_empty()
        && (sub_bytes.starts_with(b"DDS ")
            || sub_bytes.starts_with(b"XT1\0")
            || sub_bytes.starts_with(b"Gfx2")
            || is_bntx_blob(&sub_bytes))
    {
        return Ok((sub_bytes, 0, 0));
    }
    if is_bntx_blob(&sub_bytes) {
        return Ok((sub_bytes, tex_idx, 0));
    }
    let wtb = WtbFile::from_bytes(&sub_bytes, sub_wtp.as_deref()).map_err(|e| e.to_string())?;
    let entry = wtb
        .entries
        .get(tex_idx)
        .ok_or_else(|| format!("Texture index {tex_idx} out of range"))?;
    Ok((entry.data.clone(), entry.index, entry.flags))
}

#[tauri::command]
pub async fn get_texture_gpu_surface(
    node_id: String,
    mip_index: u32,
    layer_index: u32,
    state: State<'_, AppState>,
) -> Result<tauri::ipc::Response, String> {
    let (container_id, leaf) = {
        let (container_parts, leaf_ref) = split_container_and_leaf(&node_id);
        (container_parts.join("/"), leaf_ref.to_string())
    };
    let (snap, oodle_path, cached_wtb) = {
        let sess = lock_session(&state);
        let snap = sess.current.clone().ok_or("No active document loaded")?;
        let cached_wtb = if leaf.starts_with("tex:") {
            sess.container_wtb_cache.get(&container_id).cloned()
        } else {
            None
        };
        (snap, sess.settings.oodle_dll_path.clone(), cached_wtb)
    };

    let container_id_bg = container_id.clone();
    let (wtx2, new_wtb) = tauri::async_runtime::spawn_blocking(move || -> Result<(Vec<u8>, Option<Arc<WtbFile>>), String> {
        let oodle_opt = if oodle_path.is_empty() {
            None
        } else {
            Some(Path::new(oodle_path.as_str()))
        };
        if let Some(wtb) = cached_wtb {
            let tex_idx: usize = leaf.strip_prefix("tex:").and_then(|s| s.parse().ok()).unwrap_or(0);
            let entry = wtb.entries.get(tex_idx).ok_or_else(|| format!("Texture index {tex_idx} out of range"))?;
            let surf = extract_gpu_texture_surface(&entry.data, entry.index, mip_index, layer_index)
                .map_err(|e| format!("Failed to extract GPU texture surface: {e}"))?;
            let bytes = encode_wtx2_blob(surf.width, surf.height, false, surf.is_srgb, surf.gpu_format, surf.data.len() as u32, &surf.data);
            return Ok((bytes, None));
        }

        if leaf.starts_with("tex:") {
            let (_, sub_bytes, sub_wtp, _) = resolve_subblob(&snap, &container_id_bg, oodle_opt)?;
            if !is_bntx_blob(&sub_bytes) {
                let wtb = Arc::new(WtbFile::from_bytes(&sub_bytes, sub_wtp.as_deref()).map_err(|e| e.to_string())?);
                let tex_idx: usize = leaf.strip_prefix("tex:").and_then(|s| s.parse().ok()).unwrap_or(0);
                let entry = wtb.entries.get(tex_idx).ok_or_else(|| format!("Texture index {tex_idx} out of range"))?;
                let surf = extract_gpu_texture_surface(&entry.data, entry.index, mip_index, layer_index)
                    .map_err(|e| format!("Failed to extract GPU texture surface: {e}"))?;
                let bytes = encode_wtx2_blob(surf.width, surf.height, false, surf.is_srgb, surf.gpu_format, surf.data.len() as u32, &surf.data);
                return Ok((bytes, Some(wtb)));
            }
        }

        let (blob, sub_idx, _flags) = resolve_texture_blob_for_node(&snap, &node_id, oodle_opt)?;
        let surf = extract_gpu_texture_surface(&blob, sub_idx, mip_index, layer_index)
            .map_err(|e| format!("Failed to extract GPU texture surface: {e}"))?;
        let bytes = encode_wtx2_blob(
            surf.width,
            surf.height,
            false,
            surf.is_srgb,
            surf.gpu_format,
            surf.data.len() as u32,
            &surf.data,
        );
        Ok((bytes, None))
    })
    .await
    .map_err(|e| format!("GPU surface worker failed: {e}"))??;

    if let Some(wtb) = new_wtb {
        let mut sess = lock_session(&state);
        sess.container_wtb_cache.insert(container_id, wtb);
    }

    Ok(tauri::ipc::Response::new(wtx2))
}

#[tauri::command]
pub async fn get_texture_gpu_cubemap(
    node_id: String,
    mip_index: u32,
    state: State<'_, AppState>,
) -> Result<tauri::ipc::Response, String> {
    let (snap, oodle_path, cached_wtbs) = {
        let sess = lock_session(&state);
        let snap = sess.current.clone().ok_or("No active document loaded")?;
        (
            snap,
            sess.settings.oodle_dll_path.clone(),
            sess.container_wtb_cache.clone(),
        )
    };

    let (wtx2, newly_cached) = tauri::async_runtime::spawn_blocking(
        move || -> Result<(Vec<u8>, Option<(String, Arc<WtbFile>)>), String> {
            let oodle_opt = if oodle_path.is_empty() {
                None
            } else {
                Some(Path::new(oodle_path.as_str()))
            };
            let effective_node_id =
                resolve_effective_texture_node(&snap, &node_id, oodle_opt);
            let (container_parts, leaf) = split_container_and_leaf(&effective_node_id);
            let container_id = container_parts.join("/");

            let mut newly_cached: Option<(String, Arc<WtbFile>)> = None;
            let (blob, sub_idx) = if leaf.starts_with("tex:") {
                let tex_idx: usize = leaf
                    .strip_prefix("tex:")
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0);
                if let Some(wtb) = cached_wtbs.get(&container_id) {
                    let entry = wtb
                        .entries
                        .get(tex_idx)
                        .ok_or_else(|| format!("Texture index {tex_idx} out of range"))?;
                    (entry.data.clone(), entry.index)
                } else {
                    let (_, sub_bytes, sub_wtp, _) =
                        resolve_subblob(&snap, &container_id, oodle_opt)?;
                    if is_bntx_blob(&sub_bytes) {
                        (sub_bytes, tex_idx)
                    } else {
                        let wtb = Arc::new(
                            WtbFile::from_bytes(&sub_bytes, sub_wtp.as_deref())
                                .map_err(|e| e.to_string())?,
                        );
                        let entry = wtb
                            .entries
                            .get(tex_idx)
                            .ok_or_else(|| format!("Texture index {tex_idx} out of range"))?;
                        let res = (entry.data.clone(), entry.index);
                        newly_cached = Some((container_id, wtb));
                        res
                    }
                }
            } else {
                let (b, idx, _) =
                    resolve_texture_blob_for_node(&snap, &effective_node_id, oodle_opt)?;
                (b, idx)
            };

            let mut faces = Vec::with_capacity(6);
            for face in 0..6u32 {
                let surf = extract_gpu_texture_surface(&blob, sub_idx, mip_index, face)
                    .map_err(|e| format!("Failed to extract GPU cubemap face {face}: {e}"))?;
                faces.push(surf);
            }
            let w = faces[0].width;
            let h = faces[0].height;
            let is_srgb = faces[0].is_srgb;
            let gpu_format = faces[0].gpu_format;
            let face_byte_len = faces[0].data.len();
            let mut combined = Vec::with_capacity(face_byte_len * 6);
            for f in &faces {
                if f.data.len() == face_byte_len {
                    combined.extend_from_slice(&f.data);
                } else {
                    let mut padded = vec![0u8; face_byte_len];
                    let copy_len = f.data.len().min(face_byte_len);
                    padded[..copy_len].copy_from_slice(&f.data[..copy_len]);
                    combined.extend_from_slice(&padded);
                }
            }
            let bytes = encode_wtx2_blob(
                w,
                h,
                true,
                is_srgb,
                gpu_format,
                face_byte_len as u32,
                &combined,
            );
            Ok((bytes, newly_cached))
        },
    )
    .await
    .map_err(|e| format!("GPU cubemap worker failed: {e}"))??;

    if let Some((cid, wtb)) = newly_cached {
        let mut sess = lock_session(&state);
        sess.container_wtb_cache.insert(cid, wtb);
    }

    Ok(tauri::ipc::Response::new(wtx2))
}

/// Binary batch thumbnail response (`WTB2`):
/// `[0..4] "WTB2"`, `[4..8] count: u32`, repeated: `[id_len: u32][id_utf8][blob_len: u32][wtx2_bytes]`
#[tauri::command]
pub async fn get_texture_gpu_thumbnails_batch(
    node_ids: Vec<String>,
    state: State<'_, AppState>,
) -> Result<tauri::ipc::Response, String> {
    let (snap, oodle_path, mut cached_wtbs) = {
        let sess = lock_session(&state);
        let snap = sess.current.clone().ok_or("No active document loaded")?;
        let cached_wtbs = sess.container_wtb_cache.clone();
        (snap, sess.settings.oodle_dll_path.clone(), cached_wtbs)
    };

    let (out, newly_cached) = tauri::async_runtime::spawn_blocking(move || {
        let oodle_opt = if oodle_path.is_empty() {
            None
        } else {
            Some(Path::new(oodle_path.as_str()))
        };
        let mut newly_cached: HashMap<String, Arc<WtbFile>> = HashMap::new();
        let mut out = Vec::with_capacity(8 + node_ids.len() * 4096);
        out.extend_from_slice(b"WTB2");
        out.extend_from_slice(&(node_ids.len() as u32).to_le_bytes());

        let extract_thumb_wtx2 = |blob: &[u8], sub_idx: usize| -> Option<Vec<u8>> {
            let target_mip = if is_bntx_blob(blob) {
                parse_bntx_textures(blob)
                    .ok()
                    .and_then(|recs| recs.get(sub_idx).or_else(|| recs.first()).cloned())
                    .map(|t| pick_mip_for_max_dim(t.width, t.height, t.mip_offsets.len().max(1) as u32, 128))
                    .unwrap_or(0)
            } else {
                inspect_texture_blob(blob)
                    .ok()
                    .map(|m| pick_mip_for_max_dim(m.width, m.height, m.mip_count.max(1), 128))
                    .unwrap_or(0)
            };
            let surf = extract_gpu_texture_surface(blob, sub_idx, target_mip, 0).ok()?;
            Some(encode_wtx2_blob(
                surf.width,
                surf.height,
                false,
                surf.is_srgb,
                surf.gpu_format,
                surf.data.len() as u32,
                &surf.data,
            ))
        };

        for nid in node_ids {
            let (container_parts, leaf) = split_container_and_leaf(&nid);
            let container_id = container_parts.join("/");
            let wtx2 = if leaf.starts_with("tex:") {
                let tex_idx: usize = leaf.strip_prefix("tex:").and_then(|s| s.parse().ok()).unwrap_or(0);
                if let Some(wtb) = cached_wtbs.get(&container_id) {
                    wtb.entries
                        .get(tex_idx)
                        .and_then(|e| extract_thumb_wtx2(&e.data, e.index))
                        .unwrap_or_default()
                } else if let Ok((_, sub_bytes, sub_wtp, _)) = resolve_subblob(&snap, &container_id, oodle_opt) {
                    if is_bntx_blob(&sub_bytes) {
                        extract_thumb_wtx2(&sub_bytes, tex_idx).unwrap_or_default()
                    } else if let Ok(wtb) = WtbFile::from_bytes(&sub_bytes, sub_wtp.as_deref()) {
                        let wtb_arc = Arc::new(wtb);
                        let res = wtb_arc
                            .entries
                            .get(tex_idx)
                            .and_then(|e| extract_thumb_wtx2(&e.data, e.index))
                            .unwrap_or_default();
                        cached_wtbs.insert(container_id.clone(), wtb_arc.clone());
                        newly_cached.insert(container_id, wtb_arc);
                        res
                    } else {
                        Vec::new()
                    }
                } else {
                    Vec::new()
                }
            } else {
                resolve_texture_blob_for_node(&snap, &nid, oodle_opt)
                    .ok()
                    .and_then(|(blob, sub_idx, _flags)| extract_thumb_wtx2(&blob, sub_idx))
                    .unwrap_or_default()
            };

            let id_bytes = nid.as_bytes();
            out.extend_from_slice(&(id_bytes.len() as u32).to_le_bytes());
            out.extend_from_slice(id_bytes);
            out.extend_from_slice(&(wtx2.len() as u32).to_le_bytes());
            out.extend_from_slice(&wtx2);
        }

        (out, newly_cached)
    })
    .await
    .map_err(|e| format!("Thumbnail batch worker failed: {e}"))?;

    if !newly_cached.is_empty() {
        let mut sess = lock_session(&state);
        for (k, v) in newly_cached {
            sess.container_wtb_cache.insert(k, v);
        }
    }

    Ok(tauri::ipc::Response::new(out))
}

#[tauri::command]
pub fn get_texture_thumbnails_batch(
    node_ids: Vec<String>,
    state: State<'_, AppState>,
) -> Result<Vec<(String, Option<String>)>, String> {
    let mut sess = lock_session(&state);
    let snap = sess.current.clone().ok_or("No active document loaded")?;
    let oodle_path = sess.settings.oodle_dll_path.clone();
    let oodle_opt = if oodle_path.is_empty() {
        None
    } else {
        Some(Path::new(oodle_path.as_str()))
    };

    let mut results = Vec::with_capacity(node_ids.len());
    for nid in node_ids {
        if let Some((_, cached_url)) = sess.tex_thumb_cache.get(&nid) {
            results.push((nid, cached_url.clone()));
            continue;
        }
        let (container_parts, leaf) = split_container_and_leaf(&nid);
        let container_id = container_parts.join("/");
        let Ok((_, sub_bytes, sub_wtp, _)) = resolve_subblob(&snap, &container_id, oodle_opt) else {
            results.push((nid, None));
            continue;
        };
        let tex_idx: usize = leaf
            .strip_prefix("tex:")
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        let decoded_opt = if leaf.is_empty()
            && (sub_bytes.starts_with(b"DDS ")
                || sub_bytes.starts_with(b"XT1\0")
                || sub_bytes.starts_with(b"Gfx2")
                || is_bntx_blob(&sub_bytes))
        {
            decode_texture_layer_filtered(&sub_bytes, 0, 0, 0, "RGBA", false).ok()
        } else if let Ok(wtb) = WtbFile::from_bytes(&sub_bytes, sub_wtp.as_deref()) {
            wtb.entries.get(tex_idx).and_then(|e| {
                decode_texture_layer_filtered(&e.data, tex_idx, 0, 0, "RGBA", false).ok()
            })
        } else {
            None
        };

        let data_url = decoded_opt.and_then(|(img, _, _, _)| {
            let thumb = if img.width() > 160 || img.height() > 160 {
                image::imageops::thumbnail(&img, 128, 128)
            } else {
                img
            };
            let mut png_buf = Cursor::new(Vec::new());
            if thumb.write_to(&mut png_buf, image::ImageFormat::Png).is_ok() {
                Some(format!(
                    "data:image/png;base64,{}",
                    BASE64.encode(png_buf.into_inner())
                ))
            } else {
                None
            }
        });
        sess.insert_thumb_cached(nid.clone(), (nid.clone(), data_url.clone()));
        results.push((nid, data_url));
    }
    Ok(results)
}

#[tauri::command]
pub fn create_empty_bank_document(
    output_path: String,
    is_split_wta_wtp: bool,
    game_profile: String,
    state: State<'_, AppState>,
) -> Result<DocumentOutline, String> {
    let profile = match game_profile.trim().to_ascii_lowercase().as_str() {
        "bayo1" | "pc" | "dds" => WtbGameProfile::Bayo1PcDds,
        "nier" => WtbGameProfile::NierAutomataDds,
        "bayo2_wiiu" | "gtx" => WtbGameProfile::Bayo2WiiUGtx,
        _ => WtbGameProfile::SwitchXt1,
    };
    let wtb = WtbFile::new_empty(is_split_wta_wtp, profile);
    let (wta_or_wtb_bytes, wtp_bytes_opt) = wtb.to_bytes();

    let mut primary_path = PathBuf::from(output_path.trim());
    if is_split_wta_wtp {
        primary_path.set_extension("wta");
        let wtp_path = primary_path.with_extension("wtp");
        let wtp_bytes = wtp_bytes_opt.unwrap_or_default();
        atomic_write_file(&primary_path, &wta_or_wtb_bytes)?;
        atomic_write_file(&wtp_path, &wtp_bytes)?;

        let mut sess = lock_session(&state);
        sess.doc_path = Some(primary_path);
        sess.wtp_path = Some(wtp_path);
        sess.dtt_path = None;
        sess.current = Some(DocSnapshot::split(wta_or_wtb_bytes, wtp_bytes));
        sess.dirty = false;
        sess.undo_stack.clear();
        sess.redo_stack.clear();
        sess.invalidate_texture_caches();
        build_outline_from_session(&sess)
    } else {
        primary_path.set_extension("wtb");
        atomic_write_file(&primary_path, &wta_or_wtb_bytes)?;

        let mut sess = lock_session(&state);
        sess.doc_path = Some(primary_path);
        sess.wtp_path = None;
        sess.dtt_path = None;
        sess.current = Some(DocSnapshot::single(wta_or_wtb_bytes));
        sess.dirty = false;
        sess.undo_stack.clear();
        sess.redo_stack.clear();
        sess.invalidate_texture_caches();
        build_outline_from_session(&sess)
    }
}

#[derive(Debug, Clone)]
enum ReplacementKind {
    Xt1,
    Dds,
    Image,
}

impl ReplacementKind {
    fn priority(&self, is_xt1_bank: bool) -> u8 {
        match self {
            Self::Xt1 if is_xt1_bank => 3,
            Self::Xt1 => 1,
            Self::Dds => 2,
            Self::Image => 1,
        }
    }

    fn label(&self) -> &'static str {
        match self {
            Self::Xt1 => "raw .xt1",
            Self::Dds => "lossless .dds",
            Self::Image => "re-encode image",
        }
    }
}

fn classify_replacement_file(path: &Path) -> Option<ReplacementKind> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "xt1" => Some(ReplacementKind::Xt1),
        "dds" => Some(ReplacementKind::Dds),
        "png" | "tga" | "bmp" | "jpg" | "jpeg" => Some(ReplacementKind::Image),
        _ => None,
    }
}

struct PlannedEntryMatch {
    tex_index: usize,
    chosen_file: Option<(PathBuf, ReplacementKind)>,
}

fn refresh_standalone_bntx_wtb(wtb: &mut WtbFile, next_bntx: &[u8]) {
    let prev_ids: Vec<Option<u32>> = wtb.entries.iter().map(|e| e.texture_id).collect();
    let prev_flags: Vec<u32> = wtb.entries.iter().map(|e| e.flags).collect();
    if let Ok(mut rebuilt) = WtbFile::from_bntx_bytes(next_bntx) {
        for (i, e) in rebuilt.entries.iter_mut().enumerate() {
            if let Some(&tid) = prev_ids.get(i) {
                e.texture_id = tid;
            }
            if let Some(&fl) = prev_flags.get(i) {
                e.flags = fl;
            }
        }
        *wtb = rebuilt;
    } else {
        for e in &mut wtb.entries {
            e.data = next_bntx.to_vec();
        }
    }
}

fn build_batch_replace_plan(
    wtb: &WtbFile,
    folder: &Path,
) -> Result<(Vec<PlannedEntryMatch>, BatchReplaceScanResult), String> {
    let is_xt1_bank = wtb.game_profile == WtbGameProfile::SwitchXt1;
    let mut candidate_files: Vec<(PathBuf, String, ReplacementKind)> = Vec::new();
    for item in fs::read_dir(folder).map_err(|e| e.to_string())?.flatten() {
        let p = item.path();
        if !p.is_file() {
            continue;
        }
        let Some(kind) = classify_replacement_file(&p) else {
            continue;
        };
        let Some(stem) = p.file_stem().and_then(|s| s.to_str()).map(|s| s.to_string()) else {
            continue;
        };
        candidate_files.push((p, stem, kind));
    }
    candidate_files.sort_by(|a, b| a.0.cmp(&b.0));

    let mut used_file_indices = std::collections::HashSet::new();
    let mut plans = Vec::with_capacity(wtb.entries.len());
    let mut matched = 0usize;
    let mut missing = 0usize;
    let mut ambiguous = 0usize;
    let mut unsupported = 0usize;
    let mut log_lines = Vec::new();

    for entry in &wtb.entries {
        let is_non_2d = entry
            .metadata
            .as_ref()
            .map(|m| m.dimension != TextureDimension::Tex2D)
            .unwrap_or(false);
        let embedded_bntx_name = entry
            .metadata
            .as_ref()
            .and_then(|m| m.name.as_ref())
            .map(|s| s.trim().to_ascii_lowercase());

        let mut matching_cands: Vec<(usize, &PathBuf, &ReplacementKind)> = Vec::new();
        for (f_idx, (path, stem, kind)) in candidate_files.iter().enumerate() {
            let (parsed_idx, parsed_hash) = parse_replacement_filename_stem(stem);
            let exact_idx_and_hash = match (parsed_idx, parsed_hash, entry.texture_id) {
                (Some(i), Some(h), Some(eh)) => i == entry.index && h == eh,
                _ => false,
            };
            let exact_idx_only = parsed_idx == Some(entry.index) && parsed_hash.is_none();
            let exact_hash_only = parsed_idx.is_none()
                && parsed_hash.is_some()
                && parsed_hash == entry.texture_id;
            let exact_bntx_name = embedded_bntx_name
                .as_ref()
                .map(|bn| stem.trim().eq_ignore_ascii_case(bn))
                .unwrap_or(false);

            if exact_idx_and_hash || exact_idx_only || exact_hash_only || exact_bntx_name {
                matching_cands.push((f_idx, path, kind));
            }
        }

        if matching_cands.is_empty() {
            missing += 1;
            plans.push(PlannedEntryMatch {
                tex_index: entry.index,
                chosen_file: None,
            });
            continue;
        }

        for &(f_idx, _, _) in &matching_cands {
            used_file_indices.insert(f_idx);
        }

        let max_prio = matching_cands
            .iter()
            .map(|(_, _, k)| k.priority(is_xt1_bank))
            .max()
            .unwrap_or(0);
        let top_cands: Vec<_> = matching_cands
            .into_iter()
            .filter(|(_, _, k)| k.priority(is_xt1_bank) == max_prio)
            .collect();

        let hash_label = entry
            .texture_id
            .map(|h| format!("0x{h:08X}"))
            .unwrap_or_else(|| "no-hash".to_string());

        if top_cands.len() > 1 {
            ambiguous += 1;
            let names: Vec<String> = top_cands
                .iter()
                .filter_map(|(_, p, _)| p.file_name().and_then(|s| s.to_str()).map(String::from))
                .collect();
            log_lines.push(format!(
                "[AMBIGUOUS] tex{} ({hash_label}): multiple equal-priority matches ({})",
                entry.index,
                names.join(", ")
            ));
            plans.push(PlannedEntryMatch {
                tex_index: entry.index,
                chosen_file: None,
            });
            continue;
        }

        let (_, chosen_path, chosen_kind) = top_cands[0];
        let fname = chosen_path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("file");

        if is_non_2d && !matches!(chosen_kind, ReplacementKind::Xt1) {
            unsupported += 1;
            log_lines.push(format!(
                "[UNSUPPORTED] tex{} ({hash_label}): non-2D texture cannot be re-encoded from {fname}",
                entry.index
            ));
            plans.push(PlannedEntryMatch {
                tex_index: entry.index,
                chosen_file: None,
            });
            continue;
        }

        matched += 1;
        log_lines.push(format!(
            "[MATCH] tex{} ({hash_label}) <- {fname} ({})",
            entry.index,
            chosen_kind.label()
        ));
        plans.push(PlannedEntryMatch {
            tex_index: entry.index,
            chosen_file: Some((chosen_path.clone(), chosen_kind.clone())),
        });
    }

    let mut unused_count = 0usize;
    for (f_idx, (path, _, _)) in candidate_files.iter().enumerate() {
        if !used_file_indices.contains(&f_idx) {
            unused_count += 1;
            if let Some(fname) = path.file_name().and_then(|s| s.to_str()) {
                log_lines.push(format!("[UNUSED] {fname} (no matching texture in bank)"));
            }
        }
    }

    let summary = BatchReplaceScanResult {
        bank_label: format!(
            "{} ({} textures)",
            wtb.game_profile.label(),
            wtb.entries.len()
        ),
        has_confirmed_xt1_flags: wtb.game_profile.has_confirmed_xt1_flags(),
        matched,
        missing,
        ambiguous,
        unsupported,
        unused: unused_count,
        log_lines,
    };

    Ok((plans, summary))
}

#[tauri::command]
pub fn scan_batch_replace_folder(
    node_id: String,
    folder: String,
    state: State<'_, AppState>,
) -> Result<BatchReplaceScanResult, String> {
    let sess = lock_session(&state);
    let snap = sess.current.as_ref().ok_or("No active document loaded")?;
    let oodle_opt = sess.settings.oodle_path();
    let (container_parts, _) = split_container_and_leaf(&node_id);
    let container_id = container_parts.join("/");
    let (_, sub_bytes, sub_wtp, _) = resolve_subblob(snap, &container_id, oodle_opt)?;
    let wtb = WtbFile::from_bytes(&sub_bytes, sub_wtp.as_deref()).map_err(|e| e.to_string())?;
    let (_, summary) = build_batch_replace_plan(&wtb, Path::new(&folder))?;
    Ok(summary)
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

    let effective_node_id = resolve_effective_texture_node(&snap, &node_id, oodle_opt);
    let (container_parts, leaf) = split_container_and_leaf(&effective_node_id);
    let container_id = container_parts.join("/");
    let (_, sub_bytes, sub_wtp, _) = resolve_subblob(&snap, &container_id, oodle_opt)?;

    let tex_idx: usize = leaf
        .strip_prefix("tex:")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);

    if leaf.is_empty() && (sub_bytes.starts_with(b"DDS ") || sub_bytes.starts_with(b"XT1\0")) {
        if action == "replace_file" {
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
            let quality = payload
                .get("quality")
                .and_then(|v| v.as_str())
                .map(EncodeQuality::from_str_name)
                .unwrap_or(EncodeQuality::Normal);
            let fmt_str = payload
                .get("encode_format")
                .and_then(|v| v.as_str())
                .unwrap_or("Auto");
            let existing_meta = inspect_texture_blob(&sub_bytes).ok();
            let existing_fmt = existing_meta
                .as_ref()
                .map(|m| m.format)
                .unwrap_or(TextureFormat::Bc3);
            let target_fmt = parse_requested_texture_format(fmt_str, existing_fmt);
            let target_ext = if sub_bytes.starts_with(b"XT1\0") {
                ".xt1"
            } else {
                ".dds"
            };
            let raw = fs::read(file_path).map_err(|e| e.to_string())?;
            let low_path = file_path.to_ascii_lowercase();
            let new_blob = if is_raster_image_path(&low_path) {
                let img = image::load_from_memory(&raw)
                    .map_err(|e| e.to_string())?
                    .to_rgba8();
                encode_image_for_bank_entry(
                    &img,
                    target_ext,
                    target_fmt,
                    None,
                    generate_mips,
                    apply_solidify,
                    make_opaque,
                    quality,
                    Some(&sub_bytes),
                    nvtt_opt,
                )
            } else if target_ext == ".xt1" && raw.starts_with(b"DDS ") {
                swizzle_dds_to_xt1(&raw, Some(&sub_bytes)).map_err(|e| e.to_string())?
            } else {
                raw
            };
            let dat_align = sess.settings.effective_dat_alignment();
            let next_snap =
                write_back_subblob_with_align(&snap, &container_parts, new_blob, None, oodle_opt, dat_align)?;
            sess.invalidate_single_texture_cache(&[&node_id, &effective_node_id], &container_id);
            record_mutation(
                &mut sess,
                format!("Replace standalone {target_ext} texture ({target_fmt:?})"),
                next_snap,
            );
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
            if wtb.game_profile == WtbGameProfile::SwitchXt1 {
                entry.flags |= XT1_FLAG_BASE | XT1_FLAG_HAS_CRC32;
            }
            format!("Set Texture #{tex_idx} CRC32 hash to 0x{val:08X}")
        }
        "calc_crc32" => {
            let entry = wtb
                .entries
                .get_mut(tex_idx)
                .ok_or("Texture index out of range")?;
            let provided_name = payload
                .get("name")
                .and_then(|v| v.as_str())
                .map(|s| s.trim())
                .filter(|s| !s.is_empty());
            let embedded_bntx_name = entry
                .metadata
                .as_ref()
                .and_then(|m| m.name.as_deref())
                .map(|s| s.trim())
                .filter(|s| !s.is_empty());
            let source_name = provided_name
                .or(embedded_bntx_name)
                .ok_or("No texture name provided for CRC32 calculation")?
                .to_string();
            let crc = crc32fast::hash(source_name.to_ascii_lowercase().as_bytes()) & 0x7FFF_FFFF;
            entry.texture_id = Some(crc);
            if wtb.game_profile == WtbGameProfile::SwitchXt1 {
                entry.flags |= XT1_FLAG_BASE | XT1_FLAG_HAS_CRC32;
            }
            format!("Calculate CRC32 for '{source_name}' (0x{crc:08X})")
        }
        "toggle_xt1_flag" => {
            if wtb.game_profile != WtbGameProfile::SwitchXt1 {
                return Err(
                    "XT1 table2 flag checkboxes only apply to Bayonetta 3 / Astral Chain (SwitchXt1) banks"
                        .into(),
                );
            }
            let flag_name = payload
                .get("flag")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let entry = wtb
                .entries
                .get_mut(tex_idx)
                .ok_or("Texture index out of range")?;
            entry.flags |= XT1_FLAG_BASE;
            match flag_name {
                "crc32" => {
                    let next_on = (entry.flags & XT1_FLAG_HAS_CRC32) == 0;
                    if next_on {
                        entry.flags |= XT1_FLAG_HAS_CRC32;
                        if entry.texture_id.is_none() {
                            entry.texture_id = Some(0);
                        }
                    } else {
                        entry.flags &= !XT1_FLAG_HAS_CRC32;
                        let any_active_crc = wtb
                            .entries
                            .iter()
                            .any(|e| (e.flags & XT1_FLAG_HAS_CRC32) != 0);
                        if !any_active_crc {
                            for e in &mut wtb.entries {
                                e.texture_id = None;
                            }
                        } else if let Some(cur) = wtb.entries.get_mut(tex_idx) {
                            if cur.texture_id.is_none() {
                                cur.texture_id = Some(0);
                            }
                        }
                    }
                    format!(
                        "Toggle Texture #{tex_idx} Has CRC32 hash flag ({})",
                        if next_on { "ON" } else { "OFF" }
                    )
                }
                "srgb" => {
                    let next_on = (entry.flags & XT1_FLAG_SRGB) == 0;
                    if next_on {
                        entry.flags |= XT1_FLAG_SRGB;
                    } else {
                        entry.flags &= !XT1_FLAG_SRGB;
                    }
                    format!(
                        "Toggle Texture #{tex_idx} sRGB flag ({})",
                        if next_on { "ON" } else { "OFF" }
                    )
                }
                "cubemap" => {
                    let next_on = (entry.flags & XT1_FLAG_CUBEMAP) == 0;
                    if next_on {
                        entry.flags |= XT1_FLAG_CUBEMAP;
                    } else {
                        entry.flags &= !XT1_FLAG_CUBEMAP;
                    }
                    format!(
                        "Toggle Texture #{tex_idx} Cube map usage flag ({})",
                        if next_on { "ON" } else { "OFF" }
                    )
                }
                other => return Err(format!("Unknown XT1 flag '{other}'")),
            }
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
            let is_bntx_container = is_bntx_blob(&sub_bytes);
            let entry = wtb
                .entries
                .get_mut(tex_idx)
                .ok_or("Texture index out of range")?;
            let rgba = match kind {
                "black" => [0u8, 0, 0, 255],
                "normal" => [128u8, 128, 255, 255],
                _ => [255u8, 255, 255, 255],
            };
            let mut img = image::RgbaImage::new(8, 8);
            for p in img.pixels_mut() {
                p.0 = rgba;
            }
            let is_srgb = entry
                .metadata
                .as_ref()
                .map(|m| m.format.is_srgb())
                .unwrap_or(false);

            if entry.extension == ".xt1" {
                let dummy_fmt = TextureFormat::Astc(8, 8, is_srgb);
                let xt1 = encode_rgba_to_xt1_with_options(
                    &img,
                    dummy_fmt,
                    Some(1),
                    false,
                    false,
                    false,
                    EncodeQuality::Normal,
                    Some(&entry.data),
                    nvtt_opt,
                );
                entry.metadata = inspect_texture_blob(&xt1).ok();
                entry.data = xt1;
            } else if entry.extension == ".bntx" {
                let dummy_fmt = TextureFormat::Astc(8, 8, is_srgb);
                let next_bntx = replace_bntx_subtexture_with_options(
                    &entry.data,
                    tex_idx,
                    Err(&img),
                    Some(dummy_fmt),
                    false,
                    false,
                    false,
                    EncodeQuality::Normal,
                    nvtt_opt,
                )
                .map_err(|e| e.to_string())?;
                if is_bntx_container {
                    refresh_standalone_bntx_wtb(&mut wtb, &next_bntx);
                } else {
                    entry.metadata = inspect_texture_blob(&next_bntx).ok();
                    entry.data = next_bntx;
                }
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
            let sync_flags = payload
                .get("sync_flags")
                .and_then(|v| v.as_bool())
                .unwrap_or(true);
            let quality = payload
                .get("quality")
                .and_then(|v| v.as_str())
                .map(EncodeQuality::from_str_name)
                .unwrap_or(EncodeQuality::Normal);
            let fmt_str = payload
                .get("encode_format")
                .and_then(|v| v.as_str())
                .unwrap_or("Auto");

            let raw = fs::read(file_path).map_err(|e| e.to_string())?;
            let is_bntx_container = is_bntx_blob(&sub_bytes);
            let game_profile = wtb.game_profile;
            let entry = wtb
                .entries
                .get_mut(tex_idx)
                .ok_or("Texture index out of range")?;
            let existing_fmt = entry
                .metadata
                .as_ref()
                .map(|m| m.format)
                .unwrap_or(TextureFormat::Bc3);
            let existing_bntx_name = entry.metadata.as_ref().and_then(|m| m.name.clone());
            let target_fmt = parse_requested_texture_format(fmt_str, existing_fmt);

            let low_path = file_path.to_ascii_lowercase();
            let new_blob = if is_raster_image_path(&low_path) {
                let img = image::load_from_memory(&raw)
                    .map_err(|e| format!("Failed to decode image: {e}"))?
                    .to_rgba8();
                if entry.extension == ".bntx" {
                    replace_bntx_subtexture_with_options(
                        &entry.data,
                        tex_idx,
                        Err(&img),
                        Some(target_fmt),
                        generate_mips,
                        apply_solidify,
                        make_opaque,
                        quality,
                        nvtt_opt,
                    )
                    .map_err(|e| e.to_string())?
                } else {
                    encode_image_for_bank_entry(
                        &img,
                        &entry.extension,
                        target_fmt,
                        None,
                        generate_mips,
                        apply_solidify,
                        make_opaque,
                        quality,
                        Some(&entry.data),
                        nvtt_opt,
                    )
                }
            } else if entry.extension == ".xt1" && raw.starts_with(b"DDS ") {
                swizzle_dds_to_xt1(&raw, Some(&entry.data)).map_err(|e| e.to_string())?
            } else if entry.extension == ".bntx" && raw.starts_with(b"DDS ") {
                replace_bntx_subtexture_with_options(
                    &entry.data,
                    tex_idx,
                    Ok(&raw),
                    None,
                    generate_mips,
                    apply_solidify,
                    make_opaque,
                    quality,
                    nvtt_opt,
                )
                .map_err(|e| e.to_string())?
            } else {
                raw
            };
            entry.metadata = inspect_texture_blob(&new_blob).ok();
            if let Some(ref mut m) = entry.metadata {
                entry.extension = m.container_ext.clone();
                if m.name.is_none() {
                    m.name = existing_bntx_name;
                }
            }
            entry.data = new_blob.clone();
            // Strictly isolate flag auto-sync to Bayonetta 3 / Astral Chain (SwitchXt1);
            // never recalculate or overwrite entry.texture_id on replace!
            if game_profile == WtbGameProfile::SwitchXt1 && sync_flags {
                entry.sync_xt1_flags_from_metadata(game_profile);
            }
            if is_bntx_container {
                refresh_standalone_bntx_wtb(&mut wtb, &new_blob);
            }
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
            let sync_flags = payload
                .get("sync_flags")
                .and_then(|v| v.as_bool())
                .unwrap_or(true);
            let quality = payload
                .get("quality")
                .and_then(|v| v.as_str())
                .map(EncodeQuality::from_str_name)
                .unwrap_or(EncodeQuality::Normal);
            let fmt_str = payload
                .get("encode_format")
                .and_then(|v| v.as_str())
                .unwrap_or("BC3_UNORM");
            let target_fmt = parse_requested_texture_format(fmt_str, TextureFormat::Bc3);
            let bank_ext = wtb
                .entries
                .first()
                .map(|e| e.extension.clone())
                .unwrap_or_else(|| match wtb.game_profile {
                    WtbGameProfile::SwitchXt1 => ".xt1".to_string(),
                    WtbGameProfile::Bayo2WiiUGtx => ".gtx".to_string(),
                    WtbGameProfile::Bayo2SwitchBntx => ".bntx".to_string(),
                    _ => ".dds".to_string(),
                });

            let raw = fs::read(file_path).map_err(|e| e.to_string())?;
            let low_path = file_path.to_ascii_lowercase();
            let new_blob = if is_raster_image_path(&low_path) {
                let img = image::load_from_memory(&raw)
                    .map_err(|e| format!("Failed to decode image: {e}"))?
                    .to_rgba8();
                encode_image_for_bank_entry(
                    &img,
                    &bank_ext,
                    target_fmt,
                    None,
                    generate_mips,
                    apply_solidify,
                    make_opaque,
                    quality,
                    wtb.entries.first().map(|e| e.data.as_slice()),
                    nvtt_opt,
                )
            } else if bank_ext == ".xt1" && raw.starts_with(b"DDS ") {
                swizzle_dds_to_xt1(&raw, wtb.entries.first().map(|e| e.data.as_slice()))
                    .map_err(|e| e.to_string())?
            } else {
                raw
            };
            let initial_tid = payload
                .get("name")
                .and_then(|v| v.as_str())
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .map(|n| crc32fast::hash(n.to_ascii_lowercase().as_bytes()) & 0x7FFF_FFFF)
                .or(Some(0));
            let default_flags = wtb
                .game_profile
                .default_entry_flags(wtb.is_split_wta_wtp, initial_tid.is_some());
            wtb.push(new_blob, default_flags, initial_tid)
                .map_err(|e| e.to_string())?;
            if wtb.game_profile == WtbGameProfile::SwitchXt1 && sync_flags {
                if let Some(last) = wtb.entries.last_mut() {
                    last.sync_xt1_flags_from_metadata(wtb.game_profile);
                }
            }
            format!("Add texture #{}", wtb.entries.len() - 1)
        }
        "delete_texture" => {
            if tex_idx >= wtb.entries.len() {
                return Err("Texture index out of range".into());
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
            let apply_solidify = payload
                .get("solidify")
                .and_then(|v| v.as_bool())
                .unwrap_or(true);
            let sync_flags = payload
                .get("sync_flags")
                .and_then(|v| v.as_bool())
                .unwrap_or(true);
            let quality = payload
                .get("quality")
                .and_then(|v| v.as_str())
                .map(EncodeQuality::from_str_name)
                .unwrap_or(EncodeQuality::Normal);

            let (plans, _) = build_batch_replace_plan(&wtb, Path::new(folder))?;
            let is_bntx_container = is_bntx_blob(&sub_bytes);
            let mut shared_bntx = if is_bntx_container {
                Some(sub_bytes.clone())
            } else {
                None
            };
            let game_profile = wtb.game_profile;
            let mut replaced = 0usize;

            for plan in plans {
                let Some((ref path, ref kind)) = plan.chosen_file else {
                    continue;
                };
                let Ok(raw) = fs::read(path) else {
                    continue;
                };
                let Some(entry) = wtb.entries.get_mut(plan.tex_index) else {
                    continue;
                };
                let existing_fmt = entry
                    .metadata
                    .as_ref()
                    .map(|m| m.format)
                    .unwrap_or(TextureFormat::Bc3);
                let existing_mips = entry
                    .metadata
                    .as_ref()
                    .map(|m| m.mip_count.max(1))
                    .unwrap_or(1);

                if let Some(ref mut bntx_buf) = shared_bntx {
                    let updated_res = match kind {
                        ReplacementKind::Dds => replace_bntx_subtexture_with_options(
                            bntx_buf,
                            entry.index,
                            Ok(&raw),
                            Some(existing_fmt),
                            existing_mips > 1,
                            false,
                            false,
                            quality,
                            nvtt_opt,
                        ),
                        ReplacementKind::Image => {
                            if let Ok(img) = image::load_from_memory(&raw) {
                                let rgba = img.to_rgba8();
                                replace_bntx_subtexture_with_options(
                                    bntx_buf,
                                    entry.index,
                                    Err(&rgba),
                                    Some(existing_fmt),
                                    existing_mips > 1,
                                    apply_solidify,
                                    false,
                                    quality,
                                    nvtt_opt,
                                )
                            } else {
                                continue;
                            }
                        }
                        ReplacementKind::Xt1 => continue,
                    };
                    if let Ok(next_bntx) = updated_res {
                        *bntx_buf = next_bntx;
                        replaced += 1;
                    }
                    continue;
                }

                let next_blob = match kind {
                    ReplacementKind::Xt1 => raw,
                    ReplacementKind::Dds => {
                        if entry.extension == ".xt1" {
                            let Ok(swz) = swizzle_dds_to_xt1(&raw, Some(&entry.data)) else {
                                continue;
                            };
                            swz
                        } else if entry.extension == ".bntx" {
                            let Ok(nb) = replace_bntx_subtexture_with_options(
                                &entry.data,
                                0,
                                Ok(&raw),
                                Some(existing_fmt),
                                existing_mips > 1,
                                false,
                                false,
                                quality,
                                nvtt_opt,
                            ) else {
                                continue;
                            };
                            nb
                        } else {
                            raw
                        }
                    }
                    ReplacementKind::Image => {
                        let Ok(img) = image::load_from_memory(&raw) else {
                            continue;
                        };
                        let rgba = img.to_rgba8();
                        if entry.extension == ".bntx" {
                            let Ok(nb) = replace_bntx_subtexture_with_options(
                                &entry.data,
                                0,
                                Err(&rgba),
                                Some(existing_fmt),
                                existing_mips > 1,
                                apply_solidify,
                                false,
                                quality,
                                nvtt_opt,
                            ) else {
                                continue;
                            };
                            nb
                        } else {
                            encode_image_for_bank_entry(
                                &rgba,
                                &entry.extension,
                                existing_fmt,
                                Some(existing_mips),
                                existing_mips > 1,
                                apply_solidify,
                                false,
                                quality,
                                Some(&entry.data),
                                nvtt_opt,
                            )
                        }
                    }
                };

                let prev_bntx_name = entry.metadata.as_ref().and_then(|m| m.name.clone());
                entry.metadata = inspect_texture_blob(&next_blob).ok();
                if let Some(ref mut m) = entry.metadata {
                    if m.name.is_none() {
                        m.name = prev_bntx_name;
                    }
                }
                entry.data = next_blob;
                if game_profile == WtbGameProfile::SwitchXt1 && sync_flags {
                    entry.sync_xt1_flags_from_metadata(game_profile);
                }
                replaced += 1;
            }
            if let Some(final_bntx) = shared_bntx {
                refresh_standalone_bntx_wtb(&mut wtb, &final_bntx);
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
    let dat_align = sess.settings.effective_dat_alignment();
    let next_snap = write_back_subblob_with_align(
        &snap,
        &container_parts,
        new_wtb_bytes,
        new_wtp_bytes,
        oodle_opt,
        dat_align,
    )?;
    match action.as_str() {
        "replace_file" | "dummy_out" => {
            sess.invalidate_single_texture_cache(&[&node_id, &effective_node_id], &container_id);
        }
        "set_hash" | "calc_crc32" | "toggle_xt1_flag" | "set_flags" | "add_texture" => {
            sess.invalidate_single_texture_cache(&[], &container_id);
        }
        _ => {
            sess.invalidate_texture_caches();
        }
    }
    record_mutation(&mut sess, desc, next_snap);
    build_outline_from_session(&sess)
}

fn export_single_blob_to_path(
    blob: &[u8],
    sub_idx: usize,
    out_file: &Path,
    fmt: &str,
) -> Result<(), String> {
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
        "dds" => {
            let dds = export_texture_to_dds_lossless(blob, sub_idx).map_err(|e| e.to_string())?;
            fs::write(out_file, dds).map_err(|e| e.to_string())
        }
        _ => fs::write(out_file, blob).map_err(|e| e.to_string()),
    }
}

fn collect_bank_node_ids_recursive(nodes: &[AssetTreeNode], out: &mut Vec<String>) {
    for n in nodes {
        if matches!(n.kind.as_str(), "WTB" | "BNTX") {
            out.push(n.id.clone());
        } else if matches!(n.kind.as_str(), "DDS" | "GTX" | "XT1") && !n.id.contains("/tex:") {
            out.push(n.id.clone());
        }
        if !n.children.is_empty() {
            collect_bank_node_ids_recursive(&n.children, out);
        }
    }
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
    let oodle_opt = sess.settings.oodle_path();

    if scope == "container" {
        let outline = build_outline_from_session(&sess)?;
        let mut bank_ids = Vec::new();
        collect_bank_node_ids_recursive(&outline.tree, &mut bank_ids);
        if bank_ids.is_empty() {
            return Err("No texture banks found in the active container".into());
        }
        let out_root = PathBuf::from(&dest_path);
        fs::create_dir_all(&out_root).map_err(|e| e.to_string())?;
        let multi_banks = bank_ids.len() > 1;
        let mut total_exported = 0usize;

        for bid in bank_ids {
            let (c_parts, _) = split_container_and_leaf(&bid);
            let cid = c_parts.join("/");
            let Ok((sub_name, sub_bytes, sub_wtp, _)) = resolve_subblob(snap, &cid, oodle_opt)
            else {
                continue;
            };
            let bank_dir = if multi_banks {
                let stem = Path::new(&sub_name)
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("bank");
                out_root.join(stem)
            } else {
                out_root.clone()
            };
            let _ = fs::create_dir_all(&bank_dir);
            if let Ok(wtb) = WtbFile::from_bytes(&sub_bytes, sub_wtp.as_deref()) {
                for e in &wtb.entries {
                    let hash_s = e
                        .texture_id
                        .map(|h| format!("_0x{h:08X}"))
                        .unwrap_or_default();
                    let ext = match format.as_str() {
                        "png" => ".png",
                        "dds" => ".dds",
                        _ => e.extension.as_str(),
                    };
                    let fpath = bank_dir.join(format!("tex{}{}{}", e.index, hash_s, ext));
                    if export_single_blob_to_path(&e.data, e.index, &fpath, &format).is_ok() {
                        total_exported += 1;
                    }
                }
            }
        }
        return Ok(format!(
            "Exported {total_exported} textures across container to {}",
            out_root.display()
        ));
    }

    let effective_node_id = resolve_effective_texture_node(snap, &node_id, oodle_opt);
    let (container_parts, leaf) = split_container_and_leaf(&effective_node_id);
    let container_id = container_parts.join("/");
    let (_, sub_bytes, sub_wtp, _) = resolve_subblob(snap, &container_id, oodle_opt)?;

    if scope == "single" {
        if leaf.is_empty()
            && (sub_bytes.starts_with(b"DDS ")
                || sub_bytes.starts_with(b"XT1\0")
                || sub_bytes.starts_with(b"Gfx2")
                || is_bntx_blob(&sub_bytes))
        {
            export_single_blob_to_path(&sub_bytes, 0, Path::new(&dest_path), &format)?;
            return Ok(format!("Exported texture to {dest_path}"));
        }
        let tex_idx: usize = leaf
            .strip_prefix("tex:")
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        let wtb = WtbFile::from_bytes(&sub_bytes, sub_wtp.as_deref()).map_err(|e| e.to_string())?;
        let entry = wtb
            .entries
            .get(tex_idx)
            .ok_or("Texture index out of range")?;
        export_single_blob_to_path(&entry.data, tex_idx, Path::new(&dest_path), &format)?;
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
        if export_single_blob_to_path(&e.data, e.index, &file_path, &format).is_ok() {
            count += 1;
        }
    }
    Ok(format!("Exported {count} textures to {}", out_dir.display()))
}

fn collect_texture_files_in_folder(root: &Path, recursive: bool) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(rd) = fs::read_dir(&dir) else {
            continue;
        };
        let mut entries: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
        entries.sort();
        for p in entries {
            if p.is_dir() {
                if recursive {
                    stack.push(p);
                }
                continue;
            }
            let ext = p
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            // Skip .dtt if sibling .dat exists (since .dat pairs with .dtt automatically)
            if ext == "dtt" && p.with_extension("dat").is_file() {
                continue;
            }
            if matches!(
                ext.as_str(),
                "dat" | "dtt" | "wta" | "wtb" | "xt1" | "bntx" | "gtx" | "dds"
            ) {
                files.push(p);
            }
        }
    }
    files
}

fn recompress_wtb_in_place(
    wtb: &mut WtbFile,
    is_standalone_bntx: bool,
    cfg: &AstcRecompressConfig,
    sync_flags: bool,
    nvtt_opt: Option<&Path>,
    log_lines: &mut Vec<String>,
) -> (usize, usize) {
    let mut converted = 0usize;
    let mut skipped = 0usize;
    let game_profile = wtb.game_profile;

    if is_standalone_bntx {
        if let Some(first) = wtb.entries.first() {
            let mut cur_bntx = first.data.clone();
            let count = wtb.entries.len();
            for idx in 0..count {
                match recompress_astc_texture_blob_to_bc(&cur_bntx, idx, cfg, nvtt_opt) {
                    Ok(Some((next_bntx, msg))) => {
                        cur_bntx = next_bntx;
                        converted += 1;
                        log_lines.push(format!("    #{idx}: {msg}"));
                    }
                    Ok(None) => {
                        skipped += 1;
                    }
                    Err(e) => {
                        skipped += 1;
                        log_lines.push(format!("    #{idx}: skipped ({e})"));
                    }
                }
            }
            if converted > 0 {
                if let Ok(rebuilt) = WtbFile::from_bntx_bytes(&cur_bntx) {
                    *wtb = rebuilt;
                }
            }
        }
        return (converted, skipped);
    }

    for entry in &mut wtb.entries {
        match recompress_astc_texture_blob_to_bc(&entry.data, 0, cfg, nvtt_opt) {
            Ok(Some((new_blob, msg))) => {
                let prev_name = entry.metadata.as_ref().and_then(|m| m.name.clone());
                entry.metadata = inspect_texture_blob(&new_blob).ok();
                if let Some(ref mut m) = entry.metadata {
                    if m.name.is_none() {
                        m.name = prev_name;
                    }
                }
                entry.data = new_blob;
                if game_profile == WtbGameProfile::SwitchXt1 && sync_flags {
                    entry.sync_xt1_flags_from_metadata(game_profile);
                }
                converted += 1;
                log_lines.push(format!("    #{}: {msg}", entry.index));
            }
            Ok(None) => {
                skipped += 1;
            }
            Err(e) => {
                skipped += 1;
                log_lines.push(format!("    #{}: skipped ({e})", entry.index));
            }
        }
    }

    (converted, skipped)
}

#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub fn run_batch_recompress_action(
    source_mode: String,
    folder_path: Option<String>,
    recursive: bool,
    opaque_format: String,
    alpha_format: String,
    opaque_quality: String,
    alpha_quality: String,
    downscale: bool,
    skip_no_mips: bool,
    sync_flags: bool,
    state: State<'_, AppState>,
) -> Result<BatchOperationReport, String> {
    let cfg = AstcRecompressConfig {
        opaque_bc7: opaque_format.to_ascii_uppercase().contains("BC7"),
        alpha_bc7: alpha_format.to_ascii_uppercase().contains("BC7"),
        opaque_quality: EncodeQuality::from_str_name(&opaque_quality),
        alpha_quality: EncodeQuality::from_str_name(&alpha_quality),
        downscale,
        skip_downscale_without_mips: skip_no_mips,
    };

    if source_mode == "current" {
        let mut sess = lock_session(&state);
        let mut cur_snap = sess.current.clone().ok_or("No active document loaded")?;
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

        let outline = build_outline_from_session(&sess)?;
        let mut bank_ids = Vec::new();
        collect_bank_node_ids_recursive(&outline.tree, &mut bank_ids);
        if bank_ids.is_empty() {
            bank_ids.push("root".to_string());
        }

        let mut total_conv = 0usize;
        let mut total_skip = 0usize;
        let mut log_lines = Vec::new();

        for bid in bank_ids {
            let (c_parts, _) = split_container_and_leaf(&bid);
            let cid = c_parts.join("/");
            let Ok((sub_name, sub_bytes, sub_wtp, _)) = resolve_subblob(&cur_snap, &cid, oodle_opt)
            else {
                continue;
            };
            log_lines.push(format!("[Document] {sub_name}"));
            if sub_bytes.starts_with(b"XT1\0") {
                match recompress_astc_texture_blob_to_bc(&sub_bytes, 0, &cfg, nvtt_opt) {
                    Ok(Some((new_xt1, msg))) => {
                        let dat_align = sess.settings.effective_dat_alignment();
                        cur_snap =
                            write_back_subblob_with_align(&cur_snap, &c_parts, new_xt1, None, oodle_opt, dat_align)?;
                        total_conv += 1;
                        log_lines.push(format!("    #0: {msg}"));
                    }
                    _ => {
                        total_skip += 1;
                    }
                }
                continue;
            }
            let is_bntx = is_bntx_blob(&sub_bytes);
            if let Ok(mut wtb) = WtbFile::from_bytes(&sub_bytes, sub_wtp.as_deref()) {
                let (c, s) = recompress_wtb_in_place(
                    &mut wtb,
                    is_bntx,
                    &cfg,
                    sync_flags,
                    nvtt_opt,
                    &mut log_lines,
                );
                total_conv += c;
                total_skip += s;
                if c > 0 {
                    let (new_wta, new_wtp) = if is_bntx {
                        (
                            wtb.entries
                                .first()
                                .map(|e| e.data.clone())
                                .unwrap_or(sub_bytes),
                            None,
                        )
                    } else {
                        wtb.to_bytes()
                    };
                    let dat_align = sess.settings.effective_dat_alignment();
                    cur_snap =
                        write_back_subblob_with_align(&cur_snap, &c_parts, new_wta, new_wtp, oodle_opt, dat_align)?;
                }
            }
        }

        let updated_outline = if total_conv > 0 {
            sess.invalidate_texture_caches();
            record_mutation(
                &mut sess,
                format!("Batch recompress {total_conv} ASTC texture(s) to BC"),
                cur_snap,
            );
            Some(build_outline_from_session(&sess)?)
        } else {
            None
        };

        log_lines.push(format!(
            "Done: converted {total_conv} texture(s), skipped {total_skip}."
        ));
        return Ok(BatchOperationReport {
            files_processed: 1,
            converted: total_conv,
            skipped: total_skip,
            log_lines,
            updated_outline,
        });
    }

    // Folder mode
    let folder_str = folder_path.ok_or("Missing source folder path")?;
    let root_dir = PathBuf::from(&folder_str);
    if !root_dir.is_dir() {
        return Err(format!("Folder does not exist: {}", root_dir.display()));
    }
    let nvtt_path = lock_session(&state).settings.nvtt_dll_path.clone();
    let nvtt_opt = if nvtt_path.is_empty() {
        None
    } else {
        Some(Path::new(nvtt_path.as_str()))
    };

    let files = collect_texture_files_in_folder(&root_dir, recursive);
    let mut total_conv = 0usize;
    let mut total_skip = 0usize;
    let mut log_lines = Vec::new();

    for (idx, file) in files.iter().enumerate() {
        let fname = file
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("file");
        log_lines.push(format!("[{}/{}] {fname}", idx + 1, files.len()));
        let Ok(raw) = fs::read(file) else {
            log_lines.push("    failed to read file".to_string());
            continue;
        };
        let ext = file
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();

        if ext == "xt1" || raw.starts_with(b"XT1\0") {
            match recompress_astc_texture_blob_to_bc(&raw, 0, &cfg, nvtt_opt) {
                Ok(Some((new_xt1, msg))) => {
                    if atomic_write_file(file, &new_xt1).is_ok() {
                        total_conv += 1;
                        log_lines.push(format!("    {msg}"));
                    }
                }
                _ => {
                    total_skip += 1;
                }
            }
            continue;
        }

        if ext == "bntx" || is_bntx_blob(&raw) {
            if let Ok(mut wtb) = WtbFile::from_bntx_bytes(&raw) {
                let (c, s) = recompress_wtb_in_place(
                    &mut wtb,
                    true,
                    &cfg,
                    sync_flags,
                    nvtt_opt,
                    &mut log_lines,
                );
                total_conv += c;
                total_skip += s;
                if c > 0 {
                    if let Some(first) = wtb.entries.first() {
                        let _ = atomic_write_file(file, &first.data);
                    }
                }
            }
            continue;
        }

        if ext == "wta" || ext == "wtb" {
            let wtp_path = file.with_extension("wtp");
            let wtp_opt = if ext == "wta" && wtp_path.is_file() {
                fs::read(&wtp_path).ok()
            } else {
                None
            };
            if let Ok(mut wtb) = WtbFile::from_bytes(&raw, wtp_opt.as_deref()) {
                let (c, s) = recompress_wtb_in_place(
                    &mut wtb,
                    false,
                    &cfg,
                    sync_flags,
                    nvtt_opt,
                    &mut log_lines,
                );
                total_conv += c;
                total_skip += s;
                if c > 0 {
                    let (new_wta, new_wtp) = wtb.to_bytes();
                    let _ = atomic_write_file(file, &new_wta);
                    if let Some(wtp_bytes) = new_wtp {
                        let _ = atomic_write_file(&wtp_path, &wtp_bytes);
                    }
                }
            }
            continue;
        }

        if ext == "dat" || ext == "dtt" {
            let Ok(mut dat) = DatFile::from_bytes(&raw) else {
                continue;
            };
            let dtt_path = file.with_extension("dtt");
            let mut dtt_opt = if ext == "dat" && dtt_path.is_file() {
                fs::read(&dtt_path)
                    .ok()
                    .and_then(|b| DatFile::from_bytes(&b).ok())
            } else {
                None
            };

            let mut file_conv = 0usize;
            for i in 0..dat.entries.len() {
                let tag = detect_file_format_tag(&dat.entries[i].name, &dat.entries[i].data);
                if tag == "WTB" {
                    let wta_name = dat.entries[i].name.clone();
                    let wta_bytes = dat.entries[i].data.clone();
                    let paired_wtp = crate::vfs::find_paired_wtp_bytes(
                        &wta_name,
                        0,
                        1,
                        &dat.entries,
                        dtt_opt.as_ref().map(|d| d.entries.as_slice()),
                    );
                    if let Ok(mut wtb) = WtbFile::from_bytes(&wta_bytes, paired_wtp.as_deref()) {
                        let (c, s) = recompress_wtb_in_place(
                            &mut wtb,
                            false,
                            &cfg,
                            sync_flags,
                            nvtt_opt,
                            &mut log_lines,
                        );
                        total_conv += c;
                        total_skip += s;
                        file_conv += c;
                        if c > 0 {
                            let (new_wta, new_wtp) = wtb.to_bytes();
                            dat.entries[i].data = new_wta;
                            if let Some(wtp_bytes) = new_wtp {
                                if let Some(ref mut dtt_file) = dtt_opt {
                                    crate::vfs::update_matching_wtp_in_dat(
                                        &wta_name, 0, dtt_file, &wtp_bytes,
                                    );
                                } else {
                                    crate::vfs::update_matching_wtp_in_dat(
                                        &wta_name, 0, &mut dat, &wtp_bytes,
                                    );
                                }
                            }
                        }
                    }
                } else if tag == "BNTX" {
                    let bntx_bytes = dat.entries[i].data.clone();
                    if let Ok(mut wtb) = WtbFile::from_bntx_bytes(&bntx_bytes) {
                        let (c, s) = recompress_wtb_in_place(
                            &mut wtb,
                            true,
                            &cfg,
                            sync_flags,
                            nvtt_opt,
                            &mut log_lines,
                        );
                        total_conv += c;
                        total_skip += s;
                        file_conv += c;
                        if c > 0 {
                            if let Some(first) = wtb.entries.first() {
                                dat.entries[i].data = first.data.clone();
                            }
                        }
                    }
                }
            }
            if file_conv > 0 {
                let dat_align = lock_session(&state).settings.effective_dat_alignment();
                let _ = atomic_write_file(file, &dat.to_bytes_with_alignment(dat_align));
                if let Some(dtt_file) = dtt_opt {
                    let _ = atomic_write_file(&dtt_path, &dtt_file.to_bytes_with_alignment(dat_align));
                }
            }
        }
    }

    log_lines.push(format!(
        "Done: scanned {} file(s), converted {total_conv} texture(s), skipped {total_skip}.",
        files.len()
    ));
    Ok(BatchOperationReport {
        files_processed: files.len(),
        converted: total_conv,
        skipped: total_skip,
        log_lines,
        updated_outline: None,
    })
}

#[tauri::command]
pub fn run_folder_texture_export(
    source_folder: String,
    recursive: bool,
    output_folder: String,
    subfolder_per_bank: bool,
    format: String,
    include_bntx_name: bool,
    _state: State<'_, AppState>,
) -> Result<BatchOperationReport, String> {
    let src_root = PathBuf::from(&source_folder);
    let out_root = PathBuf::from(&output_folder);
    if !src_root.is_dir() {
        return Err(format!("Source folder does not exist: {}", src_root.display()));
    }
    fs::create_dir_all(&out_root).map_err(|e| e.to_string())?;

    let files = collect_texture_files_in_folder(&src_root, recursive);
    let mut written = 0usize;
    let mut skipped = 0usize;
    let mut log_lines = Vec::new();

    for (idx, file) in files.iter().enumerate() {
        let fname = file
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("file");
        let stem = file
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("bank");
        log_lines.push(format!("[{}/{}] {fname}", idx + 1, files.len()));

        let Ok(raw) = fs::read(file) else {
            skipped += 1;
            log_lines.push("    failed to read file".to_string());
            continue;
        };
        let ext = file
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();

        let target_dir = if subfolder_per_bank {
            let rel_parent = file
                .parent()
                .and_then(|p| p.strip_prefix(&src_root).ok())
                .unwrap_or_else(|| Path::new(""));
            let d = out_root.join(rel_parent).join(stem);
            let _ = fs::create_dir_all(&d);
            d
        } else {
            out_root.clone()
        };

        let mut export_wtb_entries = |wtb: &WtbFile, bank_prefix: &str| {
            for e in &wtb.entries {
                let hash_s = e
                    .texture_id
                    .map(|h| format!("_0x{h:08X}"))
                    .unwrap_or_default();
                let bntx_part = if include_bntx_name {
                    e.metadata
                        .as_ref()
                        .and_then(|m| m.name.as_deref())
                        .map(|n| format!("_{n}"))
                        .unwrap_or_default()
                } else {
                    String::new()
                };
                let out_ext = match format.as_str() {
                    "png" => ".png",
                    "dds" => ".dds",
                    _ => e.extension.as_str(),
                };
                let out_name = if subfolder_per_bank {
                    format!("tex{}{bntx_part}{hash_s}{out_ext}", e.index)
                } else {
                    format!("{bank_prefix}_tex{}{bntx_part}{hash_s}{out_ext}", e.index)
                };
                let out_path = target_dir.join(&out_name);
                match export_single_blob_to_path(&e.data, e.index, &out_path, &format) {
                    Ok(()) => {
                        written += 1;
                    }
                    Err(err) => {
                        skipped += 1;
                        log_lines.push(format!("    tex{}: skipped ({err})", e.index));
                    }
                }
            }
        };

        if matches!(ext.as_str(), "xt1" | "dds" | "gtx") {
            let out_ext = match format.as_str() {
                "png" => ".png".to_string(),
                "dds" => ".dds".to_string(),
                _ => format!(".{ext}"),
            };
            let out_path = target_dir.join(format!("{stem}{out_ext}"));
            match export_single_blob_to_path(&raw, 0, &out_path, &format) {
                Ok(()) => written += 1,
                Err(err) => {
                    skipped += 1;
                    log_lines.push(format!("    skipped ({err})"));
                }
            }
            continue;
        }

        if ext == "bntx" || is_bntx_blob(&raw) {
            if let Ok(wtb) = WtbFile::from_bntx_bytes(&raw) {
                export_wtb_entries(&wtb, stem);
            }
            continue;
        }

        if ext == "wta" || ext == "wtb" {
            let wtp_path = file.with_extension("wtp");
            let wtp_opt = if ext == "wta" && wtp_path.is_file() {
                fs::read(&wtp_path).ok()
            } else {
                None
            };
            if let Ok(wtb) = WtbFile::from_bytes(&raw, wtp_opt.as_deref()) {
                export_wtb_entries(&wtb, stem);
            }
            continue;
        }

        if ext == "dat" || ext == "dtt" {
            let Ok(dat) = DatFile::from_bytes(&raw) else {
                continue;
            };
            let dtt_path = file.with_extension("dtt");
            let dtt_opt = if ext == "dat" && dtt_path.is_file() {
                fs::read(&dtt_path)
                    .ok()
                    .and_then(|b| DatFile::from_bytes(&b).ok())
            } else {
                None
            };
            for e in &dat.entries {
                let tag = detect_file_format_tag(&e.name, &e.data);
                if tag == "WTB" {
                    let paired_wtp = crate::vfs::find_paired_wtp_bytes(
                        &e.name,
                        0,
                        1,
                        &dat.entries,
                        dtt_opt.as_ref().map(|d| d.entries.as_slice()),
                    );
                    if let Ok(wtb) = WtbFile::from_bytes(&e.data, paired_wtp.as_deref()) {
                        export_wtb_entries(&wtb, stem);
                    }
                } else if tag == "BNTX" {
                    if let Ok(wtb) = WtbFile::from_bntx_bytes(&e.data) {
                        export_wtb_entries(&wtb, stem);
                    }
                }
            }
        }
    }

    log_lines.push(format!(
        "Done: scanned {} file(s), exported {written} texture(s), skipped {skipped}.",
        files.len()
    ));
    Ok(BatchOperationReport {
        files_processed: files.len(),
        converted: written,
        skipped,
        log_lines,
        updated_outline: None,
    })
}
