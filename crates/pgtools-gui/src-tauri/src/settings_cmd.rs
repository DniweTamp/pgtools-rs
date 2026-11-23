use crate::session::{lock_session, AppState};
use crate::settings::{
    sanitize_custom_dat_alignment, save_settings_to_disk, validate_optional_dll_setting,
    OodleValidationReport, StudioSettings,
};
use pgtools_core::pkz::{clamp_compression_level, OodleLibrary};
use std::path::Path;
use tauri::State;

#[tauri::command]
pub fn get_studio_settings(state: State<'_, AppState>) -> StudioSettings {
    let mut sess = lock_session(&state);
    sess.settings.refresh_dll_status();
    sess.settings.clone()
}

#[tauri::command]
pub fn save_studio_settings(
    mut settings: StudioSettings,
    state: State<'_, AppState>,
) -> Result<StudioSettings, String> {
    settings.oodle_dll_path = validate_optional_dll_setting(&settings.oodle_dll_path)?;
    settings.nvtt_dll_path = validate_optional_dll_setting(&settings.nvtt_dll_path)?;
    settings.custom_dat_alignment = sanitize_custom_dat_alignment(settings.custom_dat_alignment);
    settings.refresh_dll_status();
    settings.pkz_default_level = clamp_compression_level(
        &settings.pkz_default_compression,
        settings.pkz_default_level,
    );
    let mut sess = lock_session(&state);
    sess.settings = settings;
    save_settings_to_disk(&sess.settings);
    Ok(sess.settings.clone())
}

#[tauri::command]
pub fn validate_oodle_dll(
    path: Option<String>,
    state: State<'_, AppState>,
) -> OodleValidationReport {
    let configured = path.unwrap_or_else(|| lock_session(&state).settings.oodle_dll_path.clone());
    let trimmed = configured.trim();
    let explicit_opt = if trimmed.is_empty() {
        None
    } else {
        Some(Path::new(trimmed))
    };

    if let Some(exp) = explicit_opt {
        if !exp.is_file() {
            let auto = OodleLibrary::find_dll_path(None);
            return OodleValidationReport {
                found: false,
                resolved_path: auto
                    .as_ref()
                    .map(|p| p.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                can_decompress: false,
                can_compress: false,
                message: if let Some(a) = auto {
                    format!(
                        "Not found at '{trimmed}'. Auto-detect found {}.",
                        a.display()
                    )
                } else {
                    format!("Not found at '{trimmed}', and no oo2core*.dll in application folder.")
                },
            };
        }
    }

    match OodleLibrary::load(explicit_opt) {
        Ok(lib) => {
            let can_dec = lib.can_decompress();
            let can_comp = lib.can_compress();
            let resolved = lib.resolved_path.to_string_lossy().into_owned();
            let message = if can_dec && can_comp {
                format!("{resolved} loads and supports both OodleKraken compression and decompression.")
            } else if can_dec {
                format!("{resolved} loads for decompression only (missing OodleLZ_Compress export).")
            } else {
                format!("{resolved} does not export OodleLZ_Decompress.")
            };
            OodleValidationReport {
                found: true,
                resolved_path: resolved,
                can_decompress: can_dec,
                can_compress: can_comp,
                message,
            }
        }
        Err(_) => OodleValidationReport {
            found: false,
            resolved_path: String::new(),
            can_decompress: false,
            can_compress: false,
            message: "No oo2core*.dll configured (built-in pure-Rust ooz decompressor active for reading; OodleKraken compression requires oo2core_*_win64.dll).".to_string(),
        },
    }
}

#[tauri::command]
pub fn get_recent_files(state: State<'_, AppState>) -> Vec<String> {
    lock_session(&state).settings.recent_files.clone()
}
