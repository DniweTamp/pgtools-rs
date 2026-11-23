use pgtools_core::{
    dat::{DEFAULT_CUSTOM_DAT_ALIGNMENT, MAX_CUSTOM_DAT_ALIGNMENT},
    pkz::{clamp_compression_level, OodleLibrary, DEFAULT_ZSTD_LEVEL},
};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct StudioSettings {
    pub theme: String,
    pub recent_files: Vec<String>,
    pub last_dir: String,
    pub hide_unsupported: bool,
    pub file_sort_col: String,
    pub file_sort_asc: bool,
    pub left_width: u32,
    pub right_width: u32,
    pub asset_ratio: f32,
    pub oodle_dll_path: String,
    pub nvtt_dll_path: String,
    pub oodle_configured: bool,
    pub nvtt_configured: bool,
    pub pkz_default_compression: String,
    pub pkz_default_level: i32,
    pub force_custom_dat_alignment: bool,
    pub custom_dat_alignment: usize,
    pub core_dat_b1: String,
    pub core_dat_b2: String,
    pub core_dat_b3: String,
    pub wmb_hierarchy_ratio: f32,
    pub wmb_wireframe: bool,
    pub wmb_normals_view: bool,
    pub wmb_skeleton_overlay: bool,
    pub wmb_grid_visible: bool,
    pub default_import_format: String,
    pub default_encode_quality: String,
    pub sync_xt1_flags_on_import: bool,
    pub asset_browser_zoom: u8,
    pub uvd_padding: u32,
    pub uvd_max_size: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OodleValidationReport {
    pub found: bool,
    pub resolved_path: String,
    pub can_decompress: bool,
    pub can_compress: bool,
    pub message: String,
}

pub(crate) fn sanitize_custom_dat_alignment(val: usize) -> usize {
    if val == 0 {
        DEFAULT_CUSTOM_DAT_ALIGNMENT
    } else {
        val.min(MAX_CUSTOM_DAT_ALIGNMENT)
    }
}

fn sanitize_uvd_padding(val: u32) -> u32 {
    val.min(64)
}

fn sanitize_uvd_max_size(val: u32) -> u32 {
    if val == 0 {
        4096
    } else {
        val.clamp(64, 16384).next_power_of_two()
    }
}

impl StudioSettings {
    pub(crate) fn oodle_path(&self) -> Option<&Path> {
        if self.oodle_dll_path.trim().is_empty() {
            None
        } else {
            Some(Path::new(self.oodle_dll_path.as_str()))
        }
    }

    pub(crate) fn nvtt_path(&self) -> Option<&Path> {
        if self.nvtt_dll_path.trim().is_empty() {
            None
        } else {
            Some(Path::new(self.nvtt_dll_path.as_str()))
        }
    }

    /// Returns `Some(custom_dat_alignment)` when `force_custom_dat_alignment` is enabled,
    /// or `None` when the default per-file-type alignment behavior is active.
    pub(crate) fn effective_dat_alignment(&self) -> Option<usize> {
        if self.force_custom_dat_alignment {
            Some(sanitize_custom_dat_alignment(self.custom_dat_alignment))
        } else {
            None
        }
    }

    pub(crate) fn refresh_dll_status(&mut self) {
        self.oodle_configured = check_oodle_dll_configured(&self.oodle_dll_path);
        self.nvtt_configured = check_nvtt_dll_configured(&self.nvtt_dll_path);
        self.custom_dat_alignment = sanitize_custom_dat_alignment(self.custom_dat_alignment);
        self.uvd_padding = sanitize_uvd_padding(self.uvd_padding);
        self.uvd_max_size = sanitize_uvd_max_size(self.uvd_max_size);
    }
}

impl Default for StudioSettings {
    fn default() -> Self {
        let mut s = Self {
            theme: "Dark".to_string(),
            recent_files: Vec::new(),
            last_dir: String::new(),
            hide_unsupported: true,
            file_sort_col: "name".to_string(),
            file_sort_asc: true,
            left_width: 310,
            right_width: 380,
            asset_ratio: 0.50,
            oodle_dll_path: String::new(),
            nvtt_dll_path: String::new(),
            oodle_configured: false,
            nvtt_configured: false,
            pkz_default_compression: "ZStandard".to_string(),
            pkz_default_level: DEFAULT_ZSTD_LEVEL,
            force_custom_dat_alignment: false,
            custom_dat_alignment: DEFAULT_CUSTOM_DAT_ALIGNMENT,
            core_dat_b1: String::new(),
            core_dat_b2: String::new(),
            core_dat_b3: String::new(),
            wmb_hierarchy_ratio: 0.48,
            wmb_wireframe: false,
            wmb_normals_view: false,
            wmb_skeleton_overlay: false,
            wmb_grid_visible: true,
            default_import_format: "Auto".to_string(),
            default_encode_quality: "Normal".to_string(),
            sync_xt1_flags_on_import: true,
            asset_browser_zoom: 0,
            uvd_padding: 4,
            uvd_max_size: 4096,
        };
        s.refresh_dll_status();
        s
    }
}

fn check_oodle_dll_configured(path_str: &str) -> bool {
    let trimmed = path_str.trim();
    if trimmed.is_empty() {
        return OodleLibrary::find_dll_path(None).is_some();
    }
    validate_optional_dll_setting(trimmed).is_ok()
}

fn check_nvtt_dll_configured(path_str: &str) -> bool {
    let trimmed = path_str.trim();
    if !trimmed.is_empty() {
        return validate_optional_dll_setting(trimmed).is_ok();
    }
    let mut search_dirs: Vec<PathBuf> = vec![PathBuf::new()];
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            search_dirs.push(parent.to_path_buf());
        }
    }
    for dir in &search_dirs {
        for default_name in ["nvtt30205.dll", "nvtt.dll", "libnvtt.so", "libnvtt.dylib"] {
            let pb = if dir.as_os_str().is_empty() {
                Path::new(default_name).to_path_buf()
            } else {
                dir.join(default_name)
            };
            if pb.is_file() {
                return true;
            }
        }
    }
    false
}

fn settings_file_path() -> PathBuf {
    let base = std::env::var("APPDATA")
        .or_else(|_| std::env::var("XDG_CONFIG_HOME"))
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            std::env::var("HOME")
                .map(|h| PathBuf::from(h).join(".config"))
                .unwrap_or_else(|_| std::env::temp_dir())
        });
    let dir = base.join("pgtools-studio");
    let _ = fs::create_dir_all(&dir);
    dir.join("pgtools_settings.json")
}

pub(crate) fn validate_optional_dll_setting(path_str: &str) -> Result<String, String> {
    let trimmed = path_str.trim();
    if trimmed.is_empty() {
        return Ok(String::new());
    }
    let p = Path::new(trimmed);
    if !p.is_file() {
        return Err(format!("Configured library path does not exist or is not a file: '{trimmed}'"));
    }
    let ext = p
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if !matches!(ext.as_str(), "dll" | "so" | "dylib") {
        return Err(format!("Configured library path must be a .dll, .so, or .dylib file: '{trimmed}'"));
    }
    Ok(fs::canonicalize(p)
        .unwrap_or_else(|_| p.to_path_buf())
        .to_string_lossy()
        .into_owned())
}

pub(crate) fn load_settings_from_disk() -> StudioSettings {
    let p = settings_file_path();
    if let Ok(txt) = fs::read_to_string(&p) {
        if let Ok(mut s) = serde_json::from_str::<StudioSettings>(&txt) {
            if validate_optional_dll_setting(&s.oodle_dll_path).is_err() {
                s.oodle_dll_path.clear();
            }
            if validate_optional_dll_setting(&s.nvtt_dll_path).is_err() {
                s.nvtt_dll_path.clear();
            }
            s.pkz_default_level =
                clamp_compression_level(&s.pkz_default_compression, s.pkz_default_level);
            s.custom_dat_alignment = sanitize_custom_dat_alignment(s.custom_dat_alignment);
            s.refresh_dll_status();
            return s;
        }
    }
    StudioSettings::default()
}

pub(crate) fn save_settings_to_disk(settings: &StudioSettings) {
    let p = settings_file_path();
    if let Ok(json) = serde_json::to_string_pretty(settings) {
        let _ = fs::write(p, json);
    }
}

