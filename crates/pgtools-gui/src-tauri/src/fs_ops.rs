use crate::common::{BatchOperationReport, DocSnapshot};
use crate::session::{atomic_write_file, build_outline_from_session, lock_session, record_mutation, AppState};
use crate::settings::save_settings_to_disk;
use crate::vfs::{is_supported_extension, resolve_subblob, split_container_and_leaf};
use pgtools_core::{
    bt_formats::MdbFile,
    bxm::BxmFile,
    cloth::{ClhFile, ClpFile, ClwFile},
    dat::DatFile,
    eff::EffFile,
    exp::ExpFile,
    io::{validate_single_entry_name, Endian},
    mot::MotFile,
    pkz::{clamp_compression_level, OodleLibrary, PkzArchive},
    scr::ScrFile,
    uvd::UvdFile,
    wmb::WmbFile,
    wmb3::Wmb3File,
    wtb::WtbFile,
};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::UNIX_EPOCH;
use tauri::State;

#[cfg(target_os = "windows")]
mod win_drag {
    use std::ffi::c_void;
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;

    #[repr(C)]
    struct Guid {
        data1: u32,
        data2: u16,
        data3: u16,
        data4: [u8; 8],
    }

    const IID_ISHELL_ITEM: Guid = Guid {
        data1: 0x43826D1E,
        data2: 0xE718,
        data3: 0x42EE,
        data4: [0xBC, 0x55, 0xA1, 0xE2, 0x61, 0xC3, 0x7B, 0xFE],
    };

    const BHID_DATA_OBJECT: Guid = Guid {
        data1: 0xB8C0BD9F,
        data2: 0xED24,
        data3: 0x455C,
        data4: [0x83, 0xE6, 0xD5, 0x39, 0x0C, 0x4F, 0xE8, 0xC4],
    };

    const IID_IDATA_OBJECT: Guid = Guid {
        data1: 0x0000010E,
        data2: 0x0000,
        data3: 0x0000,
        data4: [0xC0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x46],
    };

    #[repr(C)]
    struct IShellItemVtbl {
        query_interface: unsafe extern "system" fn(*mut c_void, *const Guid, *mut *mut c_void) -> i32,
        add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
        release: unsafe extern "system" fn(*mut c_void) -> u32,
        bind_to_handler: unsafe extern "system" fn(
            *mut c_void,
            *mut c_void,
            *const Guid,
            *const Guid,
            *mut *mut c_void,
        ) -> i32,
    }

    #[repr(C)]
    struct IShellItem {
        lp_vtbl: *const IShellItemVtbl,
    }

    #[repr(C)]
    struct IUnknownVtbl {
        query_interface: unsafe extern "system" fn(*mut c_void, *const Guid, *mut *mut c_void) -> i32,
        add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
        release: unsafe extern "system" fn(*mut c_void) -> u32,
    }

    #[repr(C)]
    struct IUnknown {
        lp_vtbl: *const IUnknownVtbl,
    }

    #[link(name = "ole32")]
    extern "system" {
        fn OleInitialize(pv_reserved: *mut c_void) -> i32;
    }

    #[link(name = "shell32")]
    extern "system" {
        fn SHCreateItemFromParsingName(
            psz_path: *const u16,
            pbc: *mut c_void,
            riid: *const Guid,
            ppv: *mut *mut c_void,
        ) -> i32;
        fn SHDoDragDrop(
            hwnd: *mut c_void,
            pdtobj: *mut c_void,
            pdsrc: *mut c_void,
            dw_effect: u32,
            pdw_effect: *mut u32,
        ) -> i32;
    }

    pub fn do_native_file_drag(file_path: &Path) -> Result<(), String> {
        let wide: Vec<u16> = file_path
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        unsafe {
            let _ = OleInitialize(std::ptr::null_mut());
            let mut shell_item: *mut c_void = std::ptr::null_mut();
            let hr = SHCreateItemFromParsingName(
                wide.as_ptr(),
                std::ptr::null_mut(),
                &IID_ISHELL_ITEM,
                &mut shell_item,
            );
            if hr < 0 || shell_item.is_null() {
                return Err(format!(
                    "SHCreateItemFromParsingName failed (HRESULT 0x{hr:08X})"
                ));
            }
            let item_ref = &*(shell_item as *const IShellItem);
            let mut data_obj: *mut c_void = std::ptr::null_mut();
            let hr_bind = ((*item_ref.lp_vtbl).bind_to_handler)(
                shell_item,
                std::ptr::null_mut(),
                &BHID_DATA_OBJECT,
                &IID_IDATA_OBJECT,
                &mut data_obj,
            );
            if hr_bind >= 0 && !data_obj.is_null() {
                let mut effect: u32 = 0;
                // DROPEFFECT_COPY (1) | DROPEFFECT_MOVE (2)
                let _ = SHDoDragDrop(
                    std::ptr::null_mut(),
                    data_obj,
                    std::ptr::null_mut(),
                    1 | 2,
                    &mut effect,
                );
                let dobj_ref = &*(data_obj as *const IUnknown);
                ((*dobj_ref.lp_vtbl).release)(data_obj);
            }
            ((*item_ref.lp_vtbl).release)(shell_item);
            if hr_bind < 0 {
                return Err(format!("BindToHandler failed (HRESULT 0x{hr_bind:08X})"));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FsEntry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub size: u64,
    pub modified_epoch_ms: u64,
    pub modified_str: String,
    pub extension: String,
    pub is_supported: bool,
}

pub(crate) fn clean_win_verbatim_path(s: String) -> String {
    if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{rest}")
    } else if let Some(rest) = s.strip_prefix(r"\\?\") {
        rest.to_string()
    } else {
        s
    }
}

pub(crate) fn format_epoch_date(secs: u64) -> String {
    let days = (secs / 86400) as i64;
    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = (yoe as i64) + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if m <= 2 { y + 1 } else { y };
    format!("{year:04}-{m:02}-{d:02}")
}

#[tauri::command]
pub fn pick_file_dialog(
    title: Option<String>,
    extensions: Option<Vec<String>>,
) -> Result<Option<String>, String> {
    let mut dlg = rfd::FileDialog::new();
    if let Some(t) = title {
        dlg = dlg.set_title(&t);
    }
    if let Some(exts) = extensions {
        let refs: Vec<&str> = exts.iter().map(|s| s.as_str()).collect();
        if !refs.is_empty() {
            dlg = dlg.add_filter("Supported Files", &refs);
        }
    }
    Ok(dlg.pick_file().map(|p| p.to_string_lossy().into_owned()))
}

#[tauri::command]
pub fn pick_multiple_files_dialog(
    title: Option<String>,
    extensions: Option<Vec<String>>,
) -> Result<Vec<String>, String> {
    let mut dlg = rfd::FileDialog::new();
    if let Some(t) = title {
        dlg = dlg.set_title(&t);
    }
    if let Some(exts) = extensions {
        let refs: Vec<&str> = exts.iter().map(|s| s.as_str()).collect();
        if !refs.is_empty() {
            dlg = dlg.add_filter("Supported Files", &refs);
        }
    }
    Ok(dlg
        .pick_files()
        .unwrap_or_default()
        .into_iter()
        .map(|p| p.to_string_lossy().into_owned())
        .collect())
}

#[tauri::command]
pub fn save_file_dialog(
    title: Option<String>,
    default_name: Option<String>,
) -> Result<Option<String>, String> {
    let mut dlg = rfd::FileDialog::new();
    if let Some(t) = title {
        dlg = dlg.set_title(&t);
    }
    if let Some(name) = default_name {
        dlg = dlg.set_file_name(&name);
    }
    Ok(dlg.save_file().map(|p| p.to_string_lossy().into_owned()))
}

#[tauri::command]
pub fn pick_folder_dialog(title: Option<String>) -> Result<Option<String>, String> {
    let mut dlg = rfd::FileDialog::new();
    if let Some(t) = title {
        dlg = dlg.set_title(&t);
    }
    Ok(dlg.pick_folder().map(|p| p.to_string_lossy().into_owned()))
}

#[tauri::command]
pub fn reveal_in_explorer(path: String) -> Result<(), String> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Err("Empty path".into());
    }
    let raw = PathBuf::from(trimmed);
    // Require a real, existing path and use the canonical form for the OS helper.
    let p = fs::canonicalize(&raw).map_err(|e| {
        format!(
            "Cannot reveal '{}': path does not exist or is inaccessible ({e})",
            raw.display()
        )
    })?;
    #[cfg(any(target_os = "windows", target_os = "macos"))]
    let display = clean_win_verbatim_path(p.to_string_lossy().into_owned());
    #[cfg(target_os = "windows")]
    {
        // Single argv element: "/select,<canonical-path>"
        Command::new("explorer")
            .arg(format!("/select,{display}"))
            .spawn()
            .map_err(|e| format!("Failed to open Explorer: {e}"))?;
    }
    #[cfg(target_os = "macos")]
    {
        Command::new("open")
            .args(["-R", "--", &display])
            .spawn()
            .map_err(|e| format!("Failed to reveal in Finder: {e}"))?;
    }
    #[cfg(target_os = "linux")]
    {
        let target = p.parent().unwrap_or(&p);
        let target_s = clean_win_verbatim_path(target.to_string_lossy().into_owned());
        Command::new("xdg-open")
            .args(["--", &target_s])
            .spawn()
            .map_err(|e| format!("Failed to open file manager: {e}"))?;
    }
    Ok(())
}

/// Extract raw file bytes for `node_id` to a per-process temp staging folder and initiate a native OS
/// shell file drag (`SHDoDragDrop`) so the user can drop the file straight onto Windows Explorer or Desktop.
#[tauri::command]
pub fn drag_out_asset_node(node_id: String, state: State<'_, AppState>) -> Result<String, String> {
    let (file_name, raw_bytes) = {
        let sess = lock_session(&state);
        let snap = sess.current.as_ref().ok_or("No active document loaded")?;
        let oodle_opt = if sess.settings.oodle_dll_path.is_empty() {
            None
        } else {
            Some(Path::new(sess.settings.oodle_dll_path.as_str()))
        };
        let (container_parts, leaf) = split_container_and_leaf(&node_id);
        let container_id = container_parts.join("/");
        let (sub_name, sub_bytes, sub_wtp, _) = resolve_subblob(snap, &container_id, oodle_opt)?;

        if let Some(tex_idx_str) = leaf.strip_prefix("tex:") {
            let tex_idx: usize = tex_idx_str
                .parse()
                .map_err(|_| format!("Invalid texture index '{tex_idx_str}'"))?;
            let wtb = WtbFile::from_bytes(&sub_bytes, sub_wtp.as_deref()).map_err(|e| e.to_string())?;
            let entry = wtb.entries.get(tex_idx).ok_or("Texture index out of range")?;
            let hash_s = entry
                .texture_id
                .map(|h| format!("_0x{h:08X}"))
                .unwrap_or_default();
            (
                format!("tex{}{}{}", entry.index, hash_s, entry.extension),
                entry.data.clone(),
            )
        } else {
            let doc_fallback = sess
                .doc_path
                .as_ref()
                .and_then(|p| p.file_name())
                .and_then(|s| s.to_str())
                .unwrap_or("extracted.bin")
                .to_string();
            let raw_name = if sub_name == "root" || sub_name.is_empty() {
                doc_fallback
            } else {
                Path::new(&sub_name)
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or(&sub_name)
                    .to_string()
            };
            let clean_name = validate_single_entry_name(&raw_name)
                .unwrap_or_else(|_| "extracted.bin".to_string());
            (clean_name, sub_bytes)
        }
    };

    let staging_dir = std::env::temp_dir().join(format!("pgtools-studio-drag-{}", std::process::id()));
    if let Ok(rd) = fs::read_dir(&staging_dir) {
        for item in rd.flatten() {
            let _ = fs::remove_file(item.path());
        }
    }
    fs::create_dir_all(&staging_dir).map_err(|e| e.to_string())?;
    let staged_file = staging_dir.join(&file_name);
    atomic_write_file(&staged_file, &raw_bytes)?;

    #[cfg(target_os = "windows")]
    {
        win_drag::do_native_file_drag(&staged_file)?;
    }

    Ok(staged_file.to_string_lossy().into_owned())
}

#[tauri::command]
pub fn list_directory(
    dir_path: Option<String>,
    state: State<'_, AppState>,
) -> Result<(String, Vec<FsEntry>), String> {
    let saved_last_dir = {
        let sess = lock_session(&state);
        sess.settings.last_dir.clone()
    };
    let target = dir_path
        .filter(|s| !s.trim().is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            if !saved_last_dir.is_empty() && Path::new(&saved_last_dir).is_dir() {
                Some(PathBuf::from(saved_last_dir))
            } else {
                None
            }
        })
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

    let canonical = fs::canonicalize(&target).unwrap_or_else(|_| target.clone());
    let rd =
        fs::read_dir(&target).map_err(|e| format!("Failed to read {}: {e}", target.display()))?;
    let mut entries = Vec::new();
    for item in rd.flatten() {
        let path = item.path();
        let meta = item.metadata().ok();
        let is_dir = meta.as_ref().map(|m| m.is_dir()).unwrap_or(false);
        let size = meta.as_ref().map(|m| m.len()).unwrap_or(0);
        let modified_epoch_ms = meta
            .as_ref()
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        let modified_str = if modified_epoch_ms > 0 {
            format_epoch_date(modified_epoch_ms / 1000)
        } else {
            "—".to_string()
        };
        let name = item.file_name().to_string_lossy().into_owned();
        let extension = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let is_supported = is_dir || is_supported_extension(&extension);
        entries.push(FsEntry {
            name,
            path: clean_win_verbatim_path(path.to_string_lossy().into_owned()),
            is_dir,
            size,
            modified_epoch_ms,
            modified_str,
            extension,
            is_supported,
        });
    }
    entries.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| a.name.cmp(&b.name)));

    let canon_str = clean_win_verbatim_path(canonical.to_string_lossy().into_owned());
    {
        let mut sess = lock_session(&state);
        if sess.settings.last_dir != canon_str {
            sess.settings.last_dir = canon_str.clone();
            let _ = save_settings_to_disk(&sess.settings);
        }
    }

    Ok((canon_str, entries))
}

#[tauri::command]
pub fn run_file_context_operation(
    file_path: String,
    operation: String,
    dest_path: Option<String>,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let oodle_path = lock_session(&state)
        .settings
        .oodle_dll_path
        .clone();
    let oodle_opt = if oodle_path.is_empty() {
        None
    } else {
        Some(Path::new(oodle_path.as_str()))
    };
    let p = PathBuf::from(&file_path);
    let ext = p
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("output");
    let parent = p.parent().unwrap_or(Path::new("."));

    match operation.as_str() {
        "extract_archive" => {
            let raw = fs::read(&p).map_err(|e| e.to_string())?;
            let out_dir = dest_path
                .map(PathBuf::from)
                .unwrap_or_else(|| parent.join(format!("{stem}_{ext}")));
            match ext.as_str() {
                "pkz" => {
                    let pkz = PkzArchive::from_bytes(&raw).map_err(|e| e.to_string())?;
                    let files = pkz
                        .extract_all(&raw, &out_dir, oodle_opt)
                        .map_err(|e| e.to_string())?;
                    Ok(format!(
                        "Extracted {} PKZ entries to {}",
                        files.len(),
                        out_dir.display()
                    ))
                }
                "eff" | "idd" if raw.starts_with(b"EF2\0") || raw.starts_with(b"IDD\0") => {
                    let eff = EffFile::from_bytes(&raw).map_err(|e| e.to_string())?;
                    eff.extract_to_dir(&out_dir, &format!(".{ext}"))
                        .map_err(|e| e.to_string())?;
                    Ok(format!("Extracted EFF/IDD to {}", out_dir.display()))
                }
                "scr" => {
                    let scr = ScrFile::from_bytes(&raw).map_err(|e| e.to_string())?;
                    scr.extract_to_dir(&out_dir).map_err(|e| e.to_string())?;
                    Ok(format!(
                        "Extracted SCR ({} models) to {}",
                        scr.models.len(),
                        out_dir.display()
                    ))
                }
                "wtb" | "wta" | "bntx" => {
                    let wtp = fs::read(p.with_extension("wtp")).ok();
                    let wtb =
                        WtbFile::from_bytes(&raw, wtp.as_deref()).map_err(|e| e.to_string())?;
                    wtb.extract_to_dir(&out_dir, stem, &format!(".{ext}"))
                        .map_err(|e| e.to_string())?;
                    Ok(format!(
                        "Extracted {} textures to {}",
                        wtb.entries.len(),
                        out_dir.display()
                    ))
                }
                _ => {
                    let dat = DatFile::from_bytes(&raw).map_err(|e| e.to_string())?;
                    dat.extract_to_dir(&out_dir, &format!(".{ext}"))
                        .map_err(|e| e.to_string())?;
                    Ok(format!(
                        "Extracted {} DAT entries to {}",
                        dat.entries.len(),
                        out_dir.display()
                    ))
                }
            }
        }
        "repack_folder_dat" => {
            let dat_align = lock_session(&state).settings.effective_dat_alignment();
            let (dat, orig_ext) = DatFile::create_from_dir(&p).map_err(|e| e.to_string())?;
            let out_file = dest_path
                .map(PathBuf::from)
                .unwrap_or_else(|| parent.join(format!("{stem}{orig_ext}")));
            fs::write(&out_file, dat.to_bytes_with_alignment(dat_align))
                .map_err(|e| e.to_string())?;
            if let Some(al) = dat_align {
                Ok(format!(
                    "Repacked folder into {} (custom alignment: {} B / 0x{:X})",
                    out_file.display(),
                    al,
                    al
                ))
            } else {
                Ok(format!("Repacked folder into {}", out_file.display()))
            }
        }
        "repack_folder_pkz" => {
            let (default_codec, default_level) = {
                let s = &lock_session(&state).settings;
                (s.pkz_default_compression.clone(), s.pkz_default_level)
            };
            let files = PkzArchive::collect_dir_files_recursive(&p).map_err(|e| e.to_string())?;
            if files.is_empty() {
                return Err("The folder has no files to pack into a PKZ archive".into());
            }
            let effective_codec = if (default_codec.eq_ignore_ascii_case("OodleKraken")
                || default_codec.eq_ignore_ascii_case("Oodle"))
                && !OodleLibrary::load(oodle_opt)
                    .map(|l| l.can_compress())
                    .unwrap_or(false)
            {
                "ZStandard"
            } else {
                default_codec.as_str()
            };
            let pkz_bytes = PkzArchive::build_with_options(
                &files,
                effective_codec,
                Some(clamp_compression_level(effective_codec, default_level)),
                false,
                oodle_opt,
            )
            .map_err(|e| e.to_string())?;
            let out_file = dest_path
                .map(PathBuf::from)
                .unwrap_or_else(|| parent.join(format!("{stem}.pkz")));
            atomic_write_file(&out_file, &pkz_bytes)?;
            Ok(format!(
                "Packed {} file(s) recursively into {}",
                files.len(),
                out_file.display()
            ))
        }
        "convert_wtb_wiiu_pc" => {
            let raw = fs::read(&p).map_err(|e| e.to_string())?;
            let wtp = fs::read(p.with_extension("wtp")).ok();
            let wtb = WtbFile::from_bytes(&raw, wtp.as_deref()).map_err(|e| e.to_string())?;
            let pc_wtb = wtb.convert_wiiu_to_pc().map_err(|e| e.to_string())?;
            let (bytes, _) = pc_wtb.to_bytes();
            let out_file = dest_path
                .map(PathBuf::from)
                .unwrap_or_else(|| parent.join(format!("{stem}_pc.wtb")));
            fs::write(&out_file, bytes).map_err(|e| e.to_string())?;
            Ok(format!(
                "Converted Wii U textures to PC WTB: {}",
                out_file.display()
            ))
        }
        "wmb_export_obj" => {
            let raw = fs::read(&p).map_err(|e| e.to_string())?;
            let out_file = dest_path
                .map(PathBuf::from)
                .unwrap_or_else(|| p.with_extension("obj"));
            if raw.starts_with(b"WMB3") || raw.starts_with(b"3BMW") {
                let wmb3 = Wmb3File::from_bytes(&raw).map_err(|e| e.to_string())?;
                fs::write(&out_file, wmb3.export_obj()).map_err(|e| e.to_string())?;
            } else if raw.starts_with(b"JK") && ext == "mdb" {
                let obj = MdbFile::export_obj(&raw).map_err(|e| e.to_string())?;
                fs::write(&out_file, obj).map_err(|e| e.to_string())?;
            } else {
                let wmb = WmbFile::from_bytes(&raw).map_err(|e| e.to_string())?;
                fs::write(&out_file, wmb.export_obj()).map_err(|e| e.to_string())?;
            }
            Ok(format!("Exported Wavefront OBJ to {}", out_file.display()))
        }
        "bxm_decode_xml" => {
            let raw = fs::read(&p).map_err(|e| e.to_string())?;
            let bxm = BxmFile::from_bytes(&raw).map_err(|e| e.to_string())?;
            let out_file = PathBuf::from(format!("{}.xml", p.display()));
            fs::write(&out_file, bxm.to_xml_string()).map_err(|e| e.to_string())?;
            Ok(format!("Decoded BXM to {}", out_file.display()))
        }
        "bxm_encode_xml" => {
            let xml = fs::read_to_string(&p).map_err(|e| e.to_string())?;
            let bxm = BxmFile::from_xml_str(&xml, "BXM").map_err(|e| e.to_string())?;
            let out_file = p.with_extension("bxm");
            fs::write(&out_file, bxm.to_bytes(Endian::Big)).map_err(|e| e.to_string())?;
            Ok(format!("Encoded XML to {}", out_file.display()))
        }
        "swap_endian_file" => {
            let raw = fs::read(&p).map_err(|e| e.to_string())?;
            let out_bytes = match ext.as_str() {
                "wmb" => {
                    let mut wmb = WmbFile::from_bytes(&raw).map_err(|e| e.to_string())?;
                    let target = wmb.endian.swap();
                    wmb.to_bytes(target).map_err(|e| e.to_string())?
                }
                "clp" => {
                    let clp = ClpFile::from_bytes(&raw).map_err(|e| e.to_string())?;
                    clp.to_bytes(clp.endian.swap())
                }
                "clh" => {
                    let clh = ClhFile::from_bytes(&raw).map_err(|e| e.to_string())?;
                    clh.to_bytes(clh.endian.swap())
                }
                "clw" => {
                    let clw = ClwFile::from_bytes(&raw).map_err(|e| e.to_string())?;
                    clw.to_bytes(clw.endian.swap())
                }
                "mot" => {
                    let mot = MotFile::from_bytes(&raw).map_err(|e| e.to_string())?;
                    mot.to_bytes(mot.endian.swap())
                }
                "exp" => {
                    let exp = ExpFile::from_bytes(&raw).map_err(|e| e.to_string())?;
                    exp.to_bytes(exp.endian().swap())
                }
                "scr" => {
                    let mut scr = ScrFile::from_bytes(&raw).map_err(|e| e.to_string())?;
                    scr.endian = scr.endian.swap();
                    scr.to_bytes()
                }
                "uvd" => {
                    let mut uvd = UvdFile::from_bytes(&raw).map_err(|e| e.to_string())?;
                    uvd.endian = uvd.endian.swap();
                    uvd.to_bytes()
                }
                _ => return Err(format!("Endian swap not supported for .{ext}")),
            };
            let out_file = parent.join(format!("{stem}_swapped.{ext}"));
            fs::write(&out_file, out_bytes).map_err(|e| e.to_string())?;
            Ok(format!("Swapped endianness to {}", out_file.display()))
        }
        other => Err(format!("Unknown file operation: {other}")),
    }
}


pub(crate) fn ensure_not_subpath(src: &Path, dst: &Path) -> Result<(), String> {
    if !src.is_dir() {
        return Ok(());
    }
    let src_canon = fs::canonicalize(src).map_err(|e| e.to_string())?;
    let dst_check = if dst.exists() {
        fs::canonicalize(dst).map_err(|e| e.to_string())?
    } else if let Some(parent) = dst.parent() {
        let p_canon = fs::canonicalize(parent).map_err(|e| e.to_string())?;
        if let Some(fname) = dst.file_name() {
            p_canon.join(fname)
        } else {
            p_canon
        }
    } else {
        dst.to_path_buf()
    };
    if dst_check.starts_with(&src_canon) {
        return Err(format!(
            "Cannot copy or move folder '{}' into itself ('{}')",
            src.display(),
            dst.display()
        ));
    }
    Ok(())
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> std::io::Result<()> {
    let src_canon = fs::canonicalize(src)?;
    fs::create_dir_all(dst)?;
    let dst_canon = fs::canonicalize(dst)?;
    if dst_canon.starts_with(&src_canon) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "Destination directory is inside source directory",
        ));
    }
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        if ty.is_symlink() {
            continue;
        }
        let dst_path = dst.join(entry.file_name());
        if ty.is_dir() {
            copy_dir_recursive(&entry.path(), &dst_path)?;
        } else {
            fs::copy(entry.path(), dst_path)?;
        }
    }
    Ok(())
}

pub(crate) fn unique_dest_path(dir: &Path, file_name: &str) -> PathBuf {
    let candidate = dir.join(file_name);
    if !candidate.exists() {
        return candidate;
    }
    let p = Path::new(file_name);
    let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or(file_name);
    let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("");
    for i in 1..1000 {
        let name = if ext.is_empty() {
            if i == 1 { format!("{stem}_copy") } else { format!("{stem}_copy{i}") }
        } else if i == 1 {
            format!("{stem}_copy.{ext}")
        } else {
            format!("{stem}_copy{i}.{ext}")
        };
        let cand = dir.join(name);
        if !cand.exists() {
            return cand;
        }
    }
    candidate
}

#[tauri::command]
pub fn fs_batch_operation(
    operation: String,
    source_paths: Vec<String>,
    dest_dir: Option<String>,
    new_name: Option<String>,
) -> Result<String, String> {
    match operation.as_str() {
        "copy" | "paste_copy" => {
            let target_dir = PathBuf::from(dest_dir.ok_or("Missing destination directory")?);
            let mut count = 0usize;
            for sp in &source_paths {
                let src_p = Path::new(sp);
                let Some(fname) = src_p.file_name().and_then(|s| s.to_str()) else {
                    continue;
                };
                let dst_p = unique_dest_path(&target_dir, fname);
                if src_p.is_dir() {
                    ensure_not_subpath(src_p, &dst_p)?;
                    copy_dir_recursive(src_p, &dst_p).map_err(|e| e.to_string())?;
                } else {
                    fs::copy(src_p, &dst_p).map_err(|e| e.to_string())?;
                }
                count += 1;
            }
            Ok(format!("Copied {count} item(s)"))
        }
        "move" | "paste_cut" => {
            let target_dir = PathBuf::from(dest_dir.ok_or("Missing destination directory")?);
            let mut count = 0usize;
            for sp in &source_paths {
                let src_p = Path::new(sp);
                let Some(fname) = src_p.file_name() else {
                    continue;
                };
                let dst_p = target_dir.join(fname);
                if src_p == dst_p {
                    continue;
                }
                if dst_p.exists() || dst_p.symlink_metadata().is_ok() {
                    return Err(format!(
                        "Cannot move '{}': destination '{}' already exists",
                        src_p.display(),
                        dst_p.display()
                    ));
                }
                ensure_not_subpath(src_p, &dst_p)?;
                if fs::rename(src_p, &dst_p).is_err() {
                    if src_p.is_dir() {
                        copy_dir_recursive(src_p, &dst_p).map_err(|e| e.to_string())?;
                        fs::remove_dir_all(src_p).map_err(|e| e.to_string())?;
                    } else {
                        fs::copy(src_p, &dst_p).map_err(|e| e.to_string())?;
                        fs::remove_file(src_p).map_err(|e| e.to_string())?;
                    }
                }
                count += 1;
            }
            Ok(format!("Moved {count} item(s)"))
        }
        "duplicate" => {
            let mut count = 0usize;
            for sp in &source_paths {
                let src_p = Path::new(sp);
                let Some(parent) = src_p.parent() else {
                    continue;
                };
                let Some(fname) = src_p.file_name().and_then(|s| s.to_str()) else {
                    continue;
                };
                let dst_p = unique_dest_path(parent, fname);
                if src_p.is_dir() {
                    ensure_not_subpath(src_p, &dst_p)?;
                    copy_dir_recursive(src_p, &dst_p).map_err(|e| e.to_string())?;
                } else {
                    fs::copy(src_p, &dst_p).map_err(|e| e.to_string())?;
                }
                count += 1;
            }
            Ok(format!("Duplicated {count} item(s)"))
        }
        "rename" => {
            let sp = source_paths.first().ok_or("No source path specified for rename")?;
            let raw_target_name = new_name.ok_or("Missing new name")?;
            let target_name =
                validate_single_entry_name(&raw_target_name).map_err(|e| e.to_string())?;
            let src_p = Path::new(sp);
            let parent = src_p.parent().ok_or("Cannot determine parent folder")?;
            let dst_p = parent.join(&target_name);
            if dst_p.parent() != Some(parent) {
                return Err("Invalid rename target path".into());
            }
            if src_p == dst_p {
                return Ok(format!("Renamed to {target_name}"));
            }
            if dst_p.exists() || dst_p.symlink_metadata().is_ok() {
                return Err(format!(
                    "Cannot rename to '{target_name}': file or folder already exists"
                ));
            }
            fs::rename(src_p, &dst_p).map_err(|e| e.to_string())?;
            Ok(format!("Renamed to {target_name}"))
        }
        "delete" => {
            let mut count = 0usize;
            for sp in &source_paths {
                let trimmed = sp.trim();
                if trimmed.is_empty() {
                    return Err("Empty path in delete request".into());
                }
                let src_p = Path::new(trimmed);
                // Reject relative / traversal-shaped paths; require an existing absolute path.
                if !src_p.is_absolute() {
                    return Err(format!(
                        "Refusing to delete relative path '{}'; provide an absolute path",
                        src_p.display()
                    ));
                }
                for comp in src_p.components() {
                    if matches!(comp, std::path::Component::ParentDir) {
                        return Err(format!(
                            "Refusing to delete path containing '..': '{}'",
                            src_p.display()
                        ));
                    }
                }
                let canon = fs::canonicalize(src_p).map_err(|e| {
                    format!(
                        "Cannot delete '{}': path does not exist or is inaccessible ({e})",
                        src_p.display()
                    )
                })?;
                // Refuse deleting drive roots / filesystem roots.
                if canon.parent().is_none() {
                    return Err(format!(
                        "Refusing to delete filesystem root '{}'",
                        canon.display()
                    ));
                }
                if canon.is_dir() {
                    fs::remove_dir_all(&canon).map_err(|e| e.to_string())?;
                } else {
                    fs::remove_file(&canon).map_err(|e| e.to_string())?;
                }
                count += 1;
            }
            Ok(format!("Deleted {count} item(s)"))
        }
        "new_folder" => {
            let base = PathBuf::from(dest_dir.ok_or("Missing directory path")?);
            let raw_folder_name = new_name.unwrap_or_else(|| "New Folder".to_string());
            let folder_name =
                validate_single_entry_name(&raw_folder_name).map_err(|e| e.to_string())?;
            let dst_p = unique_dest_path(&base, &folder_name);
            if dst_p.parent() != Some(base.as_path()) {
                return Err("Invalid new folder target path".into());
            }
            fs::create_dir_all(&dst_p).map_err(|e| e.to_string())?;
            Ok(format!("Created folder {}", dst_p.display()))
        }
        other => Err(format!("Unsupported filesystem batch operation: {other}")),
    }
}

#[tauri::command]
pub fn pack_folder_to_pkz(
    source_dir: String,
    dest_path: String,
    compression: String,
    level: i32,
    force_compression: Option<bool>,
    allow_zstd_fallback: Option<bool>,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let oodle_path = lock_session(&state).settings.oodle_dll_path.clone();
    let oodle_opt = if oodle_path.trim().is_empty() {
        None
    } else {
        Some(Path::new(oodle_path.as_str()))
    };
    let req_codec = compression.trim();
    let clamped_lvl = clamp_compression_level(req_codec, level);
    let allow_fallback = allow_zstd_fallback.unwrap_or(false);
    if (req_codec.eq_ignore_ascii_case("OodleKraken") || req_codec.eq_ignore_ascii_case("Oodle"))
        && !allow_fallback
        && !OodleLibrary::load(oodle_opt)
            .map(|l| l.can_compress())
            .unwrap_or(false)
    {
        return Err("OODLE_DLL_MISSING: oo2core_*_win64.dll with OodleLZ_Compress is required to compress with OodleKraken.".into());
    }

    let src_p = PathBuf::from(source_dir.trim());
    let dst_p = PathBuf::from(dest_path.trim());
    let files = PkzArchive::collect_dir_files_recursive(&src_p).map_err(|e| e.to_string())?;
    if files.is_empty() {
        return Err("The selected folder has no files to pack".into());
    }
    let total_raw: usize = files.iter().map(|(_, b)| b.len()).sum();
    let effective_codec = if (req_codec.eq_ignore_ascii_case("OodleKraken")
        || req_codec.eq_ignore_ascii_case("Oodle"))
        && allow_fallback
        && !OodleLibrary::load(oodle_opt)
            .map(|l| l.can_compress())
            .unwrap_or(false)
    {
        "ZStandard"
    } else {
        req_codec
    };
    let effective_lvl = clamp_compression_level(effective_codec, clamped_lvl);
    let pkz_bytes = PkzArchive::build_with_options(
        &files,
        effective_codec,
        Some(effective_lvl),
        force_compression.unwrap_or(false),
        oodle_opt,
    )
    .map_err(|e| e.to_string())?;
    atomic_write_file(&dst_p, &pkz_bytes)?;

    {
        let mut sess = lock_session(&state);
        sess.settings.pkz_default_compression = req_codec.to_string();
        sess.settings.pkz_default_level = clamped_lvl;
        let _ = save_settings_to_disk(&sess.settings);
    }

    Ok(format!(
        "Packed {} file(s) into {} ({} B -> {} B)",
        files.len(),
        dst_p.file_name().and_then(|s| s.to_str()).unwrap_or(&dest_path),
        total_raw,
        pkz_bytes.len()
    ))
}




#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub fn run_batch_dat_align_action(
    source_mode: String,
    folder_path: Option<String>,
    file_paths: Option<Vec<String>>,
    recursive: bool,
    pair_dtt: bool,
    custom_align: Option<usize>,
    per_type_default: bool,
    align_nested: bool,
    output_folder: Option<String>,
    state: State<'_, AppState>,
) -> Result<BatchOperationReport, String> {
    let target_align: Option<usize> = if per_type_default {
        None
    } else {
        let val = custom_align.unwrap_or(pgtools_core::dat::DEFAULT_CUSTOM_DAT_ALIGNMENT);
        let validated =
            pgtools_core::dat::validate_custom_dat_alignment(val).map_err(|e| e.to_string())?;
        Some(validated)
    };

    let align_label = match target_align {
        Some(al) => format!("{al} B (0x{al:X})"),
        None => "per-file-type default".to_string(),
    };

    if source_mode == "current" {
        let mut sess = lock_session(&state);
        let cur_snap = sess.current.clone().ok_or("No active document loaded")?;
        let oodle_path = sess.settings.oodle_dll_path.clone();
        let oodle_opt = if oodle_path.is_empty() {
            None
        } else {
            Some(Path::new(oodle_path.as_str()))
        };

        let mut log_lines = Vec::new();
        let mut converted = 0usize;
        let mut skipped = 0usize;

        let next_snap = match &cur_snap {
            DocSnapshot::SingleFile(bytes) => {
                if bytes.len() >= 0x20 && (&bytes[0..4] == b"DAT\0" || &bytes[0..4] == b"\0TAD") {
                    let old_len = bytes.len();
                    let (realigned, entries, nested_cnt) =
                        DatFile::realign_bytes(bytes, target_align, align_nested)
                            .map_err(|e| e.to_string())?;
                    converted += 1;
                    let nested_note = if nested_cnt > 0 {
                        format!(", {nested_cnt} nested .dat")
                    } else {
                        String::new()
                    };
                    log_lines.push(format!(
                        "[Document] ({entries} entries{nested_note}): {old_len} B -> {} B [align: {align_label}]",
                        realigned.len()
                    ));
                    DocSnapshot::single(realigned)
                } else if bytes.len() >= 4 && (&bytes[0..4] == b"pkzl" || &bytes[0..4] == b"PKZL" || &bytes[0..4] == b"PKZB") {
                    let pkz = PkzArchive::from_bytes(bytes).map_err(|e| e.to_string())?;
                    let oodle_lib = OodleLibrary::load(oodle_opt).ok();
                    let mut replacements = std::collections::HashMap::new();
                    for (idx, desc) in pkz.descriptors.iter().enumerate() {
                        let ext = Path::new(&desc.name)
                            .extension()
                            .and_then(|e| e.to_str())
                            .unwrap_or("")
                            .to_ascii_lowercase();
                        if !pair_dtt && ext == "dtt" {
                            skipped += 1;
                            continue;
                        }
                        if matches!(ext.as_str(), "dat" | "dtt" | "evn") {
                            if let Ok(entry_bytes) = pkz.extract_entry_with_lib(bytes, idx, oodle_lib.as_ref()) {
                                if let Ok((realigned, entries, nested_cnt)) =
                                    DatFile::realign_bytes(&entry_bytes, target_align, align_nested)
                                {
                                    log_lines.push(format!(
                                        "[PKZ #{}] {} ({entries} entries{}): {} B -> {} B [align: {align_label}]",
                                        idx,
                                        desc.name,
                                        if nested_cnt > 0 { format!(", +{nested_cnt} nested") } else { String::new() },
                                        entry_bytes.len(),
                                        realigned.len()
                                    ));
                                    replacements.insert(idx, realigned);
                                    converted += 1;
                                } else {
                                    skipped += 1;
                                }
                            }
                        }
                    }
                    if replacements.is_empty() {
                        return Err("No .dat / .dtt / .evn entries found inside active PKZ archive".into());
                    }
                    let repacked = pkz
                        .repack_with_replacements(bytes, &replacements, &[], &[], None, None, oodle_opt)
                        .map_err(|e| e.to_string())?;
                    DocSnapshot::single(repacked)
                } else {
                    return Err("Active document is not a .dat / .dtt / .evn archive (or .pkz containing .dat files)".into());
                }
            }
            DocSnapshot::PairedDatDtt { dat, dtt } => {
                let (new_dat, dat_entries, dat_nested) =
                    DatFile::realign_bytes(dat, target_align, align_nested)
                        .map_err(|e| e.to_string())?;
                converted += 1;
                log_lines.push(format!(
                    "[Document .dat] ({dat_entries} entries{}): {} B -> {} B [align: {align_label}]",
                    if dat_nested > 0 { format!(", +{dat_nested} nested") } else { String::new() },
                    dat.len(),
                    new_dat.len()
                ));
                let new_dtt = if pair_dtt {
                    let (nd, dtt_entries, dtt_nested) =
                        DatFile::realign_bytes(dtt, target_align, align_nested)
                            .map_err(|e| e.to_string())?;
                    converted += 1;
                    log_lines.push(format!(
                        "[Document .dtt] ({dtt_entries} entries{}): {} B -> {} B [align: {align_label}]",
                        if dtt_nested > 0 { format!(", +{dtt_nested} nested") } else { String::new() },
                        dtt.len(),
                        nd.len()
                    ));
                    nd
                } else {
                    skipped += 1;
                    dtt.as_ref().clone()
                };
                DocSnapshot::paired(new_dat, new_dtt)
            }
            DocSnapshot::SplitWtaWtp { .. } => {
                return Err("Active document is a WTA/WTP texture bank, not a DAT archive".into());
            }
        };

        sess.invalidate_texture_caches();
        record_mutation(
            &mut sess,
            format!("Batch DAT align ({align_label})"),
            next_snap,
        );
        let updated_outline = Some( build_outline_from_session(&sess)? );
        log_lines.push(format!(
            "Done: re-aligned {converted} archive(s) in current document to {align_label}."
        ));
        return Ok(BatchOperationReport {
            files_processed: converted + skipped,
            converted,
            skipped,
            log_lines,
            updated_outline,
        });
    }

    let out_root_opt = output_folder
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .map(PathBuf::from);

    let targets: Vec<(PathBuf, Option<PathBuf>)> = if source_mode == "files" {
        let raw_list = file_paths.unwrap_or_default();
        if raw_list.is_empty() {
            return Err("No target .dat / .dtt / .evn files selected".into());
        }
        let path_vec: Vec<PathBuf> = raw_list
            .into_iter()
            .map(|s| PathBuf::from(s.trim()))
            .filter(|p| p.is_file())
            .collect();
        pgtools_core::dat::collect_dat_targets(&path_vec, false, pair_dtt, false, out_root_opt.as_deref())
    } else {
        let folder_str = folder_path.ok_or("Missing source folder path")?;
        let root_dir = PathBuf::from(folder_str.trim());
        if !root_dir.is_dir() {
            return Err(format!("Folder does not exist: {}", root_dir.display()));
        }
        if let Some(ref out_root) = out_root_opt {
            let root_canon = fs::canonicalize(&root_dir).unwrap_or_else(|_| root_dir.clone());
            let out_canon = if out_root.exists() {
                fs::canonicalize(out_root).unwrap_or_else(|_| out_root.clone())
            } else {
                out_root.clone()
            };
            if root_canon == out_canon {
                return Err(
                    "Custom output folder cannot be the same as the source folder (select 'Overwrite existing files' or choose a different output folder)"
                        .into(),
                );
            }
        }
        pgtools_core::dat::collect_dat_targets(
            std::slice::from_ref(&root_dir),
            recursive,
            pair_dtt,
            false,
            out_root_opt.as_deref(),
        )
    };

    if let Some(ref out_root) = out_root_opt {
        fs::create_dir_all(out_root).map_err(|e| e.to_string())?;
    }

    let mut converted = 0usize;
    let mut skipped = 0usize;
    let mut log_lines = Vec::new();

    for (idx, (file_path, base_root)) in targets.iter().enumerate() {
        let fname = file_path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("archive.dat");
        let Ok(raw) = fs::read(file_path) else {
            skipped += 1;
            log_lines.push(format!("[{}/{}] {fname}: failed to read file", idx + 1, targets.len()));
            continue;
        };
        let old_len = raw.len();
        match DatFile::realign_bytes(&raw, target_align, align_nested) {
            Ok((realigned, entries, nested_cnt)) => {
                let dest_file = if let Some(ref out_root) = out_root_opt {
                    let rel = if let Some(root) = base_root {
                        file_path
                            .strip_prefix(root)
                            .map(|p| p.to_path_buf())
                            .unwrap_or_else(|_| PathBuf::from(fname))
                    } else {
                        PathBuf::from(fname)
                    };
                    out_root.join(rel)
                } else {
                    file_path.clone()
                };
                match atomic_write_file(&dest_file, &realigned) {
                    Ok(()) => {
                        converted += 1;
                        let delta = (realigned.len() as isize) - (old_len as isize);
                        let delta_str = if delta >= 0 {
                            format!("+{delta} B")
                        } else {
                            format!("{delta} B")
                        };
                        let nested_str = if nested_cnt > 0 {
                            format!(", +{nested_cnt} nested .dat")
                        } else {
                            String::new()
                        };
                        log_lines.push(format!(
                            "[{}/{}] {fname} ({entries} entries{nested_str}): {old_len} B -> {} B ({delta_str}) [align: {align_label}]",
                            idx + 1,
                            targets.len(),
                            realigned.len()
                        ));
                    }
                    Err(err) => {
                        skipped += 1;
                        log_lines.push(format!("[{}/{}] {fname}: write error ({err})", idx + 1, targets.len()));
                    }
                }
            }
            Err(err) => {
                skipped += 1;
                log_lines.push(format!("[{}/{}] {fname}: skipped ({err})", idx + 1, targets.len()));
            }
        }
    }

    log_lines.push(format!(
        "Done: scanned {} archive(s), re-aligned {converted}, skipped {skipped} (alignment: {align_label}).",
        targets.len()
    ));

    Ok(BatchOperationReport {
        files_processed: targets.len(),
        converted,
        skipped,
        log_lines,
        updated_outline: None,
    })
}
