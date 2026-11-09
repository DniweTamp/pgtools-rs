// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod formats_cmd;
mod fs_ops;
mod session;
mod settings;
mod texture_cmd;
mod vfs;
mod wmb_scene;
#[cfg(test)]
mod tests;

use session::AppState;
use std::fs;
use std::path::PathBuf;

// WebView2 appends `EBWebView` to whatever data directory it is given. Tauri's
// default lives in `%LOCALAPPDATA%\<bundle identifier>`, which can end up with a
// restrictive integrity label (for example, after an older build was launched
// elevated). A normal launch then cannot write to it, WebView2 reports
// HRESULT 0x800700AA ("The requested resource is in use"), and Tauri aborts before
// a window is shown -- which looks like the app flashing once and closing.
// Use a dedicated, per-app data directory instead, with a temp fallback.
#[cfg(target_os = "windows")]
fn webview_data_dir_candidates() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(custom) = std::env::var_os("PGTOOLS_WEBVIEW_DATA_DIR") {
        dirs.push(PathBuf::from(custom));
    }
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        dirs.push(
            PathBuf::from(local)
                .join("pgtools-studio")
                .join("webview2"),
        );
    }
    dirs.push(std::env::temp_dir().join("pgtools-studio").join("webview2"));
    dirs
}

fn make_window_builder<'a>(
    app: &'a tauri::App,
    window_config: Option<&tauri::utils::config::WindowConfig>,
) -> Result<tauri::WebviewWindowBuilder<'a, tauri::Wry, tauri::AppHandle>, String> {
    match window_config {
        Some(config) => tauri::WebviewWindowBuilder::from_config(app.handle(), config)
            .map_err(|e| e.to_string()),
        None => Ok(tauri::WebviewWindowBuilder::new(
            app.handle(),
            "main",
            tauri::WebviewUrl::App("index.html".into()),
        )),
    }
}

fn create_main_window(app: &mut tauri::App) -> Result<(), String> {
    let window_config = app.config().app.windows.first().cloned();

    #[cfg(target_os = "windows")]
    let candidates = webview_data_dir_candidates();
    #[cfg(not(target_os = "windows"))]
    let candidates: Vec<PathBuf> = Vec::new();

    if candidates.is_empty() {
        return make_window_builder(app, window_config.as_ref())?
            .build()
            .map(|_| ())
            .map_err(|e| e.to_string());
    }

    let mut last_error = String::new();
    for dir in candidates {
        if let Err(e) = fs::create_dir_all(&dir) {
            last_error = format!("{}\n  {e}", dir.display());
            continue;
        }
        match make_window_builder(app, window_config.as_ref())?
            .data_directory(dir.clone())
            .build()
        {
            Ok(_) => return Ok(()),
            Err(e) => last_error = format!("{}\n  {e}", dir.display()),
        }
    }
    Err(last_error)
}

fn show_startup_error(message: &str) {
    rfd::MessageDialog::new()
        .set_title("PGTools Studio")
        .set_level(rfd::MessageLevel::Error)
        .set_description(message)
        .show();
}

fn main() {
    tauri::Builder::default()
        .manage(AppState::default())
        .setup(|app| {
            if let Err(error) = create_main_window(app) {
                show_startup_error(&format!(
                    "PGTools Studio could not create its WebView2 window.\n\n{error}\n\n\
                     Try closing other copies of PGTools Studio, then delete this folder and \
                     start the app again:\n%LOCALAPPDATA%\\com.platinumgames.pgtools"
                ));
                std::process::exit(1);
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            settings::get_studio_settings,
            settings::save_studio_settings,
            settings::validate_oodle_dll,
            fs_ops::pick_file_dialog,
            fs_ops::pick_multiple_files_dialog,
            fs_ops::save_file_dialog,
            fs_ops::pick_folder_dialog,
            fs_ops::reveal_in_explorer,
            fs_ops::drag_out_asset_node,
            fs_ops::list_directory,
            settings::get_recent_files,
            session::open_document,
            session::save_document,
            session::undo_document,
            session::redo_document,
            formats_cmd::inspect_asset_node,
            texture_cmd::mutate_texture_node,
            texture_cmd::export_texture_action,
            texture_cmd::get_nvtt_status,
            texture_cmd::get_cubemap_faces_preview,
            texture_cmd::get_texture_thumbnails_batch,
            texture_cmd::create_empty_bank_document,
            texture_cmd::scan_batch_replace_folder,
            texture_cmd::run_batch_recompress_action,
            texture_cmd::run_folder_texture_export,
            wmb_scene::convert_material_action,
            wmb_scene::mutate_wmb_node,
            formats_cmd::mutate_bxm_node,
            formats_cmd::mutate_struct_node,
            formats_cmd::mutate_cloth_node,
            vfs::mutate_container_node,
            fs_ops::run_file_context_operation,
            fs_ops::run_batch_dat_align_action,
            fs_ops::pack_folder_to_pkz,
            fs_ops::fs_batch_operation,
            wmb_scene::get_wmb_scene,
            wmb_scene::get_wmb_texture,
            wmb_scene::get_pre_texture,
            wmb_scene::resolve_legacy_node,
            wmb_scene::create_demo_model,
            wmb_scene::wmb_clipboard_copy,
            wmb_scene::wmb_clipboard_status,
        ])
        .run(tauri::generate_context!())
        .expect("error while running PGTools Studio");
}
