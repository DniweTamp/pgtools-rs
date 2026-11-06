// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use pgtools_core::{
    bt_formats::{
        AttFile, ColFileSummary, CosFile, EstFile, FlsFile, LitFile, MdbFile, MsqFile, ObiFile,
        XadFile,
    },
    bxm::BxmFile,
    cloth::{ClhFile, ClpFile, ClwFile},
    dat::DatFile,
    databases::{
        lookup_wmb_vertex_layout, BAYO2_COMBAT_HEAD_BONE_MAP, BAYO2_CUTSCENE_HEAD_BONE_MAP,
        BAYO2_PL0000_TO_BAYO_PL0010, BAYO2_PL0010_TO_BAYO_PL0010, BAYO2_TO_BAYO1_COMMON_BONES,
        NIER_TO_BAYO1_COMMON_BONES,
    },
    eff::{EffDirectory, EffFile, EffSubFile},
    exp::ExpFile,
    io::Endian,
    mot::MotFile,
    pkz::{OodleLibrary, PkzArchive},
    scr::ScrFile,
    shader_db::{
        convert_material_between_games, decode_wmb_material_rich, list_available_shaders,
        list_core_pretextures, switch_wmb3_material_shader, switch_wmb_material_shader,
        CorePretextureInfo, ShaderOptionDesc,
    },
    texture_conv::{
        build_dummy_dds, decode_bntx_subtexture_mip_filtered, decode_texture_mip_filtered,
        encode_rgba8_to_dds, encode_rgba_to_dds_with_nvtt, encode_rgba_to_xt1,
        encode_rgba_to_xt1_with_nvtt, inspect_texture_blob, parse_bntx_textures,
        replace_bntx_subtexture, swizzle_dds_to_xt1, TextureFormat, ASTC_FOOTPRINTS,
    },
    wmb::{DecodedWmbMaterial, WmbBatchHeader, WmbFile},
    wmb3::{Wmb3File, Wmb3Material, Wmb3NamedParameter, Wmb3TextureSlotView},
    wtb::{WtbFile, WtbGameProfile},
};
use pgtools_core::io::{sanitize_entry_path, validate_single_entry_name};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::fs;
use std::io::Cursor;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::State;

// [wmb-studio BEGIN phase1]
mod wmb_scene;
use pgtools_core::{
    scene::UidMap,
    scene_ops::{
        apply_wmb3_scene_op, apply_wmb_scene_op, enforce_shadow_last, sync_wmb3_invariants,
        WmbClipboardData,
    },
};
// [wmb-studio END]

// ============================================================================
// PERSISTENT STUDIO SETTINGS (Saved to OS AppData / Config Directory)
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
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
    // [wmb-studio BEGIN phase1]
    #[serde(default)]
    pub core_dat_b1: String,
    #[serde(default)]
    pub core_dat_b2: String,
    #[serde(default)]
    pub core_dat_b3: String,
    // [wmb-studio END]
    // [wmb-studio BEGIN phase2]
    #[serde(default = "default_wmb_hierarchy_ratio")]
    pub wmb_hierarchy_ratio: f32,
    #[serde(default)]
    pub wmb_wireframe: bool,
    #[serde(default)]
    pub wmb_normals_view: bool,
    #[serde(default)]
    pub wmb_skeleton_overlay: bool,
    #[serde(default = "default_wmb_grid_visible")]
    pub wmb_grid_visible: bool,
    // [wmb-studio END]
}

// [wmb-studio BEGIN phase2]
fn default_wmb_hierarchy_ratio() -> f32 {
    0.48
}
fn default_wmb_grid_visible() -> bool {
    true
}
// [wmb-studio END]

impl Default for StudioSettings {
    fn default() -> Self {
        Self {
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
            // [wmb-studio BEGIN phase1]
            core_dat_b1: String::new(),
            core_dat_b2: String::new(),
            core_dat_b3: String::new(),
            // [wmb-studio END]
            // [wmb-studio BEGIN phase2]
            wmb_hierarchy_ratio: 0.48,
            wmb_wireframe: false,
            wmb_normals_view: false,
            wmb_skeleton_overlay: false,
            wmb_grid_visible: true,
            // [wmb-studio END]
        }
    }
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

fn validate_optional_dll_setting(path_str: &str) -> Result<String, String> {
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

fn load_settings_from_disk() -> StudioSettings {
    let p = settings_file_path();
    if let Ok(txt) = fs::read_to_string(&p) {
        if let Ok(mut s) = serde_json::from_str::<StudioSettings>(&txt) {
            if validate_optional_dll_setting(&s.oodle_dll_path).is_err() {
                s.oodle_dll_path.clear();
            }
            if validate_optional_dll_setting(&s.nvtt_dll_path).is_err() {
                s.nvtt_dll_path.clear();
            }
            return s;
        }
    }
    StudioSettings::default()
}

fn save_settings_to_disk(settings: &StudioSettings) {
    let p = settings_file_path();
    if let Ok(json) = serde_json::to_string_pretty(settings) {
        let _ = fs::write(p, json);
    }
}

// ============================================================================
// NATIVE WINDOWS SHELL DRAG-OUT (SHDoDragDrop + IShellItem)
// ============================================================================

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

// ============================================================================
// SESSION & SNAPSHOT STATE (Including Paired DAT + DTT and WTA + WTP)
// ============================================================================

#[derive(Debug, Clone)]
pub(crate) enum DocSnapshot {
    SingleFile(Arc<Vec<u8>>),
    SplitWtaWtp {
        wta: Arc<Vec<u8>>,
        wtp: Arc<Vec<u8>>,
    },
    PairedDatDtt {
        dat: Arc<Vec<u8>>,
        dtt: Arc<Vec<u8>>,
    },
}

impl DocSnapshot {
    pub(crate) fn single(bytes: Vec<u8>) -> Self {
        Self::SingleFile(Arc::new(bytes))
    }

    pub(crate) fn split(wta: Vec<u8>, wtp: Vec<u8>) -> Self {
        Self::SplitWtaWtp {
            wta: Arc::new(wta),
            wtp: Arc::new(wtp),
        }
    }

    pub(crate) fn paired(dat: Vec<u8>, dtt: Vec<u8>) -> Self {
        Self::PairedDatDtt {
            dat: Arc::new(dat),
            dtt: Arc::new(dtt),
        }
    }

    pub(crate) fn byte_size(&self) -> usize {
        match self {
            Self::SingleFile(b) => b.len(),
            Self::SplitWtaWtp { wta, wtp } => wta.len() + wtp.len(),
            Self::PairedDatDtt { dat, dtt } => dat.len() + dtt.len(),
        }
    }
}

#[derive(Debug, Clone)]
struct UndoEntry {
    description: String,
    before: DocSnapshot,
    after: DocSnapshot,
    before_revision: u64,
    after_revision: u64,
    coalesce_key: Option<String>,
    timestamp_ms: u128,
    // [wmb-studio BEGIN phase1]
    before_uid_maps: BTreeMap<String, UidMap>,
    after_uid_maps: BTreeMap<String, UidMap>,
    before_structure_version: u64,
    after_structure_version: u64,
    // [wmb-studio END]
}

const MAX_WTX1_CACHE_BYTES: usize = 128 * 1024 * 1024;
const MAX_WTX1_CACHE_ENTRIES: usize = 128;
const MAX_THUMB_CACHE_ENTRIES: usize = 512;

pub(crate) struct StudioSession {
    doc_path: Option<PathBuf>,
    wtp_path: Option<PathBuf>,
    dtt_path: Option<PathBuf>,
    current: Option<DocSnapshot>,
    dirty: bool,
    current_revision: u64,
    saved_revision: u64,
    next_revision: u64,
    undo_stack: Vec<UndoEntry>,
    redo_stack: Vec<UndoEntry>,
    settings: StudioSettings,
    // [wmb-studio BEGIN phase1]
    pub(crate) doc_generation: u64,
    pub(crate) structure_version: u64,
    pub(crate) committed_structure_version: u64,
    pub(crate) uid_maps: BTreeMap<String, UidMap>,
    pub(crate) committed_uid_maps: BTreeMap<String, UidMap>,
    pub(crate) tex_thumb_cache: HashMap<String, (String, Option<String>)>,
    pub(crate) tex_thumb_order: VecDeque<String>,
    pub(crate) wtx1_cache: HashMap<(String, u32, u32), Vec<u8>>,
    pub(crate) wtx1_order: VecDeque<(String, u32, u32)>,
    pub(crate) wtx1_cache_bytes: usize,
    pub(crate) pre_wtb_cache: HashMap<String, (u64, Arc<WtbFile>)>,
    // [wmb-studio END]
}

impl StudioSession {
    pub(crate) fn insert_thumb_cached(&mut self, key: String, val: (String, Option<String>)) {
        if !self.tex_thumb_cache.contains_key(&key) {
            while self.tex_thumb_cache.len() >= MAX_THUMB_CACHE_ENTRIES {
                if let Some(old_k) = self.tex_thumb_order.pop_front() {
                    self.tex_thumb_cache.remove(&old_k);
                } else {
                    self.tex_thumb_cache.clear();
                    break;
                }
            }
            self.tex_thumb_order.push_back(key.clone());
        }
        self.tex_thumb_cache.insert(key, val);
    }

    pub(crate) fn insert_wtx1_cached(&mut self, key: (String, u32, u32), val: Vec<u8>) {
        let entry_bytes = val.len();
        if let Some(prev) = self.wtx1_cache.remove(&key) {
            self.wtx1_cache_bytes = self.wtx1_cache_bytes.saturating_sub(prev.len());
            self.wtx1_order.retain(|k| k != &key);
        }
        while (self.wtx1_cache_bytes + entry_bytes > MAX_WTX1_CACHE_BYTES
            || self.wtx1_cache.len() >= MAX_WTX1_CACHE_ENTRIES)
            && !self.wtx1_order.is_empty()
        {
            if let Some(old_k) = self.wtx1_order.pop_front() {
                if let Some(removed) = self.wtx1_cache.remove(&old_k) {
                    self.wtx1_cache_bytes = self.wtx1_cache_bytes.saturating_sub(removed.len());
                }
            }
        }
        self.wtx1_cache_bytes += entry_bytes;
        self.wtx1_order.push_back(key.clone());
        self.wtx1_cache.insert(key, val);
    }
}

impl Default for StudioSession {
    fn default() -> Self {
        Self {
            doc_path: None,
            wtp_path: None,
            dtt_path: None,
            current: None,
            dirty: false,
            current_revision: 0,
            saved_revision: 0,
            next_revision: 1,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            settings: load_settings_from_disk(),
            // [wmb-studio BEGIN phase1]
            doc_generation: 1,
            structure_version: 1,
            committed_structure_version: 1,
            uid_maps: BTreeMap::new(),
            committed_uid_maps: BTreeMap::new(),
            tex_thumb_cache: HashMap::new(),
            tex_thumb_order: VecDeque::new(),
            wtx1_cache: HashMap::new(),
            wtx1_order: VecDeque::new(),
            wtx1_cache_bytes: 0,
            pre_wtb_cache: HashMap::new(),
            // [wmb-studio END]
        }
    }
}

pub(crate) struct AppState {
    pub(crate) session: Mutex<StudioSession>,
    // [wmb-studio BEGIN phase1]
    pub(crate) clipboard: Mutex<Option<WmbClipboardData>>,
    // [wmb-studio END]
}

pub(crate) fn lock_session(state: &AppState) -> MutexGuard<'_, StudioSession> {
    state.session.lock().unwrap_or_else(|e| e.into_inner())
}

pub(crate) fn lock_clipboard(state: &AppState) -> MutexGuard<'_, Option<WmbClipboardData>> {
    state.clipboard.lock().unwrap_or_else(|e| e.into_inner())
}

pub(crate) fn atomic_write_file(dest: &Path, bytes: &[u8]) -> Result<(), String> {
    let parent = dest.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let fname = dest
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("output.bin");
    let pid = std::process::id();
    let now_ns = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let tmp_path = parent.join(format!(".{fname}.pgtmp.{pid}.{now_ns}"));
    fs::write(&tmp_path, bytes).map_err(|e| {
        let _ = fs::remove_file(&tmp_path);
        format!("Failed to write temporary file '{}': {e}", tmp_path.display())
    })?;
    fs::rename(&tmp_path, dest).map_err(|e| {
        let _ = fs::remove_file(&tmp_path);
        format!("Failed to commit saved file '{}': {e}", dest.display())
    })
}

pub(crate) fn atomic_write_pair(
    dest1: &Path,
    bytes1: &[u8],
    dest2: &Path,
    bytes2: &[u8],
) -> Result<(), String> {
    let parent1 = dest1.parent().unwrap_or_else(|| Path::new("."));
    let parent2 = dest2.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent1).map_err(|e| e.to_string())?;
    fs::create_dir_all(parent2).map_err(|e| e.to_string())?;
    let fname1 = dest1.file_name().and_then(|s| s.to_str()).unwrap_or("out1.bin");
    let fname2 = dest2.file_name().and_then(|s| s.to_str()).unwrap_or("out2.bin");
    let pid = std::process::id();
    let now_ns = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let tmp1 = parent1.join(format!(".{fname1}.pgtmp.{pid}.{now_ns}"));
    let tmp2 = parent2.join(format!(".{fname2}.pgtmp.{pid}.{now_ns}"));
    if let Err(e) = fs::write(&tmp1, bytes1) {
        let _ = fs::remove_file(&tmp1);
        return Err(format!("Failed to write temporary file '{}': {e}", tmp1.display()));
    }
    if let Err(e) = fs::write(&tmp2, bytes2) {
        let _ = fs::remove_file(&tmp1);
        let _ = fs::remove_file(&tmp2);
        return Err(format!("Failed to write temporary file '{}': {e}", tmp2.display()));
    }
    if let Err(e) = fs::rename(&tmp1, dest1) {
        let _ = fs::remove_file(&tmp1);
        let _ = fs::remove_file(&tmp2);
        return Err(format!("Failed to commit saved file '{}': {e}", dest1.display()));
    }
    if let Err(e) = fs::rename(&tmp2, dest2) {
        let _ = fs::remove_file(&tmp2);
        return Err(format!("Failed to commit paired file '{}': {e}", dest2.display()));
    }
    Ok(())
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            session: Mutex::new(StudioSession::default()),
            // [wmb-studio BEGIN phase1]
            clipboard: Mutex::new(None),
            // [wmb-studio END]
        }
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MaterialBankTextureOption {
    pub index: usize,
    pub texture_id: u32,
    pub texture_id_hex: String,
    pub label: String,
    pub format_name: String,
    pub width: u32,
    pub height: u32,
    pub node_id: String,
    pub thumbnail_data_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MaterialPickerOption {
    pub index: usize,
    pub label: String,
    pub shader_name: String,
    pub badge: String,
    pub node_id: String,
    pub first_tex_id_hex: Option<String>,
    pub thumbnail_data_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WmbSectionView {
    pub node_id: String,
    pub section: String,
    pub title: String,
    pub breadcrumb: String,
    pub spec_line: String,
    pub validation_msg: String,
    pub is_wmb3: bool,
    pub is_bayo2: bool,
    pub endian: String,
    pub data: serde_json::Value,
    pub material_decoded: Option<DecodedWmbMaterial>,
    pub wmb3_material: Option<Wmb3Material>,
    pub wmb3_named_params: Vec<Wmb3NamedParameter>,
    pub wmb3_texture_views: Vec<Wmb3TextureSlotView>,
    pub bank_textures: Vec<MaterialBankTextureOption>,
    pub core_pretextures: Vec<CorePretextureInfo>,
    pub available_shaders: Vec<ShaderOptionDesc>,
    pub material_options: Vec<MaterialPickerOption>,
}


fn batch_effective_material_id(hdr: &WmbBatchHeader, is_bayo2: bool) -> usize {
    if is_bayo2 {
        hdr.ex_mat_id.max(0) as usize
    } else {
        hdr.material_id as usize
    }
}

fn batch_is_shadow_proxy(hdr: &WmbBatchHeader, is_bayo2: bool) -> bool {
    if is_bayo2 {
        hdr.u_e1 == 48
    } else {
        hdr.u_e1 == 32 && hdr.u_e2 == 15
    }
}

fn batch_set_shadow_proxy(hdr: &mut WmbBatchHeader, is_bayo2: bool, enabled: bool) {
    if enabled {
        if is_bayo2 {
            hdr.u_e1 = 48;
            hdr.u_e2 = 0;
        } else {
            hdr.u_e1 = 32;
            hdr.u_e2 = 15;
        }
    } else {
        hdr.u_e1 = 0;
        hdr.u_e2 = 0;
    }
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

fn clean_win_verbatim_path(s: String) -> String {
    if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{rest}")
    } else if let Some(rest) = s.strip_prefix(r"\\?\") {
        rest.to_string()
    } else {
        s
    }
}

fn format_epoch_date(secs: u64) -> String {
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
fn build_nodes_for_blob(
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
                                    let eff_mat = batch_effective_material_id(&b.header, is_b2);
                                    let shadow_tag = if batch_is_shadow_proxy(&b.header, is_b2) {
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
                                        format_badge: if batch_is_shadow_proxy(&b.header, is_b2) {
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
            let peer_dtt = if desc.name.to_ascii_lowercase().ends_with(".dat") {
                let stem = &desc.name[..desc.name.len() - 4];
                let target_dtt = format!("{stem}.dtt");
                pkz.descriptors
                    .iter()
                    .position(|d| d.name.eq_ignore_ascii_case(&target_dtt))
                    .and_then(|d_idx| {
                        pkz.extract_entry_with_lib(&cur_bytes, d_idx, oodle_lib.as_ref()).ok()
                    })
            } else {
                None
            };
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
fn update_matching_wtp_in_dat(
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
fn write_back_subblob(
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
        let peer_dtt_idx = if entry_name.to_ascii_lowercase().ends_with(".dat") {
            let stem = &entry_name[..entry_name.len() - 4];
            let target_dtt = format!("{stem}.dtt");
            pkz.descriptors
                .iter()
                .position(|d| d.name.eq_ignore_ascii_case(&target_dtt))
        } else {
            None
        };

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

/// Discover all textures in sibling `.wtb` / `.wta` banks for a model inside a `.dat`/`.dtt`/`.scr`
/// and generate small thumbnail previews for any texture referenced by `active_tex_ids_or_indices`.
fn collect_model_bank_textures(
    snap: &DocSnapshot,
    model_node_id: &str,
    referenced_ids: &[u32],
    is_b1_indices: bool,
    oodle_dll: Option<&Path>,
) -> Vec<MaterialBankTextureOption> {
    let mut out = Vec::new();
    let (container_parts, _) = split_container_and_leaf(model_node_id);

    // Check if the model is inside a SCR container (`.../scrmdl:X`)
    if let Some(last) = container_parts.last() {
        if last.starts_with("scrmdl:") {
            let parent_parts = &container_parts[..container_parts.len() - 1];
            let scr_tex_id = if parent_parts.is_empty() {
                "scrtex:0".to_string()
            } else {
                format!("{}/scrtex:0", parent_parts.join("/"))
            };
            if let Ok((_, wtb_bytes, _, _)) = resolve_subblob(snap, &scr_tex_id, oodle_dll) {
                append_wtb_options(
                    &wtb_bytes,
                    None,
                    &scr_tex_id,
                    referenced_ids,
                    is_b1_indices,
                    &mut out,
                );
            }
            return out;
        }
    }

    // If inside a DAT/DTT, inspect all `.wtb` and `.wta` entries in the parent DAT (and paired DTT!)
    let mut candidate_roots: Vec<(String, Vec<u8>, Option<Vec<u8>>)> = Vec::new();
    if let DocSnapshot::PairedDatDtt { dat, dtt } = snap {
        candidate_roots.push((
            "root:dat".to_string(),
            dat.as_ref().clone(),
            Some(dtt.as_ref().clone()),
        ));
        candidate_roots.push(("root:dtt".to_string(), dtt.as_ref().clone(), None));
    } else if !container_parts.is_empty() {
        let parent_parts = &container_parts[..container_parts.len() - 1];
        let parent_id = parent_parts.join("/");
        if let Ok((_, parent_bytes, _, _)) = resolve_subblob(snap, &parent_id, oodle_dll) {
            candidate_roots.push((parent_id, parent_bytes, None));
        }
    }

    for (prefix, dat_bytes, peer_dtt) in candidate_roots {
        if let Ok(dat) = DatFile::from_bytes(&dat_bytes) {
            let parsed_dtt = peer_dtt.as_ref().and_then(|b| DatFile::from_bytes(b).ok());
            let dtt_slice = parsed_dtt.as_ref().map(|d| d.entries.as_slice());
            let total_wtas = dat
                .entries
                .iter()
                .filter(|e| e.extension.eq_ignore_ascii_case("wta"))
                .count();
            let mut wta_ord = 0usize;

            for (idx, e) in dat.entries.iter().enumerate() {
                let ext = e.extension.to_ascii_lowercase();
                if ext == "wtb" || ext == "wta" {
                    let wtp = if ext == "wta" {
                        let m = find_paired_wtp_bytes(
                            &e.name,
                            wta_ord,
                            total_wtas,
                            &dat.entries,
                            dtt_slice,
                        );
                        wta_ord += 1;
                        m
                    } else {
                        None
                    };
                    let bank_node_id = if prefix.is_empty() {
                        format!("dat:{idx}")
                    } else {
                        format!("{prefix}/dat:{idx}")
                    };
                    append_wtb_options(
                        &e.data,
                        wtp.as_deref(),
                        &bank_node_id,
                        referenced_ids,
                        is_b1_indices,
                        &mut out,
                    );
                }
            }
        }
    }

    out
}

fn append_wtb_options(
    wtb_bytes: &[u8],
    wtp_bytes: Option<&[u8]>,
    bank_node_id: &str,
    referenced_ids: &[u32],
    is_b1_indices: bool,
    out: &mut Vec<MaterialBankTextureOption>,
) {
    let Ok(wtb) = WtbFile::from_bytes(wtb_bytes, wtp_bytes) else {
        return;
    };
    for e in &wtb.entries {
        let tid = e.texture_id.unwrap_or(e.index as u32);
        let (fmt_name, w, h) = e
            .metadata
            .as_ref()
            .map(|m| (m.format.display_name(), m.width, m.height))
            .unwrap_or_else(|| ("TEX".to_string(), 0, 0));
        let is_ref = if is_b1_indices {
            referenced_ids.contains(&(e.index as u32))
        } else {
            referenced_ids.contains(&tid) || referenced_ids.contains(&(e.index as u32))
        };
        // Decode thumbnail for referenced textures (or first 16 textures in bank for fast browsing)
        let thumb = if is_ref || e.index < 64 {
            decode_texture_mip_filtered(&e.data, 0, "RGBA", false)
                .ok()
                .and_then(|(img, _, _)| {
                    let small = if img.width() > 96 || img.height() > 96 {
                        image::imageops::thumbnail(&img, 96, 96)
                    } else {
                        img
                    };
                    let mut buf = Cursor::new(Vec::new());
                    if small.write_to(&mut buf, image::ImageFormat::Png).is_ok() {
                        Some(format!(
                            "data:image/png;base64,{}",
                            BASE64.encode(buf.into_inner())
                        ))
                    } else {
                        None
                    }
                })
        } else {
            None
        };
        let tex_node_id = format!("{bank_node_id}/tex:{}", e.index);
        out.push(MaterialBankTextureOption {
            index: e.index,
            texture_id: tid,
            texture_id_hex: format!("0x{tid:08X}"),
            label: format!("tex{} (0x{tid:08X}) — {fmt_name} {w}×{h}", e.index),
            format_name: fmt_name,
            width: w,
            height: h,
            node_id: tex_node_id,
            thumbnail_data_url: thumb,
        });
    }
}

fn build_outline_from_session(sess: &StudioSession) -> Result<DocumentOutline, String> {
    let path = sess
        .doc_path
        .as_ref()
        .ok_or("No document currently loaded")?;
    let snap = sess
        .current
        .as_ref()
        .ok_or("No document snapshot in memory")?;

    let oodle_path_opt = if sess.settings.oodle_dll_path.is_empty() {
        None
    } else {
        Some(Path::new(sess.settings.oodle_dll_path.as_str()))
    };

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

fn record_mutation(sess: &mut StudioSession, description: impl Into<String>, next: DocSnapshot) {
    record_mutation_with_key(sess, description, next, None);
}

fn record_mutation_with_key(
    sess: &mut StudioSession,
    description: impl Into<String>,
    next: DocSnapshot,
    coalesce_key: Option<String>,
) {
    const MAX_UNDO_ENTRIES: usize = 20;
    const MAX_UNDO_BYTES: usize = 512 * 1024 * 1024;
    const COALESCE_WINDOW_MS: u128 = 1500;

    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let next_rev = sess.next_revision;
    sess.next_revision = sess.next_revision.wrapping_add(1);
    let desc_str = description.into();

    if let Some(prev) = sess.current.take() {
        let can_coalesce = match (&coalesce_key, sess.undo_stack.last()) {
            (Some(k), Some(last)) => {
                last.coalesce_key.as_ref() == Some(k)
                    && now_ms.saturating_sub(last.timestamp_ms) <= COALESCE_WINDOW_MS
            }
            _ => false,
        };

        if can_coalesce {
            if let Some(last) = sess.undo_stack.last_mut() {
                last.description = desc_str;
                last.after = next.clone();
                last.after_revision = next_rev;
                last.timestamp_ms = now_ms;
                last.after_uid_maps = sess.uid_maps.clone();
                last.after_structure_version = sess.structure_version;
            }
        } else {
            if sess.undo_stack.len() >= MAX_UNDO_ENTRIES {
                sess.undo_stack.remove(0);
            }
            sess.undo_stack.push(UndoEntry {
                description: desc_str,
                before: prev,
                after: next.clone(),
                before_revision: sess.current_revision,
                after_revision: next_rev,
                coalesce_key,
                timestamp_ms: now_ms,
                // [wmb-studio BEGIN phase1]
                before_uid_maps: sess.committed_uid_maps.clone(),
                after_uid_maps: sess.uid_maps.clone(),
                before_structure_version: sess.committed_structure_version,
                after_structure_version: sess.structure_version,
                // [wmb-studio END]
            });
            while sess.undo_stack.len() > 1 {
                let total_bytes: usize = sess
                    .undo_stack
                    .iter()
                    .map(|e| e.before.byte_size())
                    .sum();
                if total_bytes > MAX_UNDO_BYTES {
                    sess.undo_stack.remove(0);
                } else {
                    break;
                }
            }
        }
        sess.redo_stack.clear();
    }
    // [wmb-studio BEGIN phase1]
    sess.committed_uid_maps = sess.uid_maps.clone();
    sess.committed_structure_version = sess.structure_version;
    sess.doc_generation = sess.doc_generation.wrapping_add(1);
    // [wmb-studio END]
    sess.current_revision = next_rev;
    sess.current = Some(next);
    sess.dirty = sess.current_revision != sess.saved_revision;
}

// ============================================================================
// CSV / TEXT ENCODING HELPERS (UTF-8 / UTF-16LE / UTF-16BE / Shift-JIS)
// ============================================================================

fn decode_csv_bytes_auto(bytes: &[u8]) -> String {
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

fn parse_csv_table(text: &str) -> (Vec<String>, Vec<Vec<String>>) {
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

fn serialize_csv_table(headers: &[String], rows: &[Vec<String>]) -> Vec<u8> {
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

// ============================================================================
// TAURI IPC COMMANDS
// ============================================================================

#[tauri::command]
fn get_studio_settings(state: State<'_, AppState>) -> StudioSettings {
    lock_session(&state).settings.clone()
}

#[tauri::command]
fn save_studio_settings(
    mut settings: StudioSettings,
    state: State<'_, AppState>,
) -> Result<StudioSettings, String> {
    settings.oodle_dll_path = validate_optional_dll_setting(&settings.oodle_dll_path)?;
    settings.nvtt_dll_path = validate_optional_dll_setting(&settings.nvtt_dll_path)?;
    let mut sess = lock_session(&state);
    sess.settings = settings.clone();
    save_settings_to_disk(&sess.settings);
    Ok(sess.settings.clone())
}

#[tauri::command]
fn pick_file_dialog(
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
fn pick_multiple_files_dialog(
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
fn save_file_dialog(
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
fn pick_folder_dialog(title: Option<String>) -> Result<Option<String>, String> {
    let mut dlg = rfd::FileDialog::new();
    if let Some(t) = title {
        dlg = dlg.set_title(&t);
    }
    Ok(dlg.pick_folder().map(|p| p.to_string_lossy().into_owned()))
}

#[tauri::command]
fn reveal_in_explorer(path: String) -> Result<(), String> {
    let p = PathBuf::from(&path);
    #[cfg(target_os = "windows")]
    {
        Command::new("explorer")
            .args(["/select,", &p.to_string_lossy()])
            .spawn()
            .map_err(|e| format!("Failed to open Explorer: {e}"))?;
    }
    #[cfg(target_os = "macos")]
    {
        Command::new("open")
            .args(["-R", "--", &p.to_string_lossy()])
            .spawn()
            .map_err(|e| format!("Failed to reveal in Finder: {e}"))?;
    }
    #[cfg(target_os = "linux")]
    {
        let target = p.parent().unwrap_or(&p);
        Command::new("xdg-open")
            .args(["--", &target.to_string_lossy()])
            .spawn()
            .map_err(|e| format!("Failed to open file manager: {e}"))?;
    }
    Ok(())
}

/// Extract raw file bytes for `node_id` to a per-process temp staging folder and initiate a native OS
/// shell file drag (`SHDoDragDrop`) so the user can drop the file straight onto Windows Explorer or Desktop.
#[tauri::command]
fn drag_out_asset_node(node_id: String, state: State<'_, AppState>) -> Result<String, String> {
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
fn list_directory(
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
            save_settings_to_disk(&sess.settings);
        }
    }

    Ok((canon_str, entries))
}

#[tauri::command]
fn get_recent_files(state: State<'_, AppState>) -> Vec<String> {
    lock_session(&state).settings.recent_files.clone()
}

#[tauri::command]
fn open_document(path: String, state: State<'_, AppState>) -> Result<DocumentOutline, String> {
    let p = PathBuf::from(&path);
    let raw = fs::read(&p).map_err(|e| format!("Failed to read {}: {e}", p.display()))?;
    let ext = p
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();

    let mut wtp_path_opt = None;
    let mut dtt_path_opt = None;
    let mut primary_path = p.clone();

    let snapshot = if ext == "wta" {
        let wtp_p = p.with_extension("wtp");
        if let Ok(wtp_bytes) = fs::read(&wtp_p) {
            wtp_path_opt = Some(wtp_p);
            DocSnapshot::split(raw, wtp_bytes)
        } else {
            DocSnapshot::single(raw)
        }
    } else if ext == "wtp" {
        let wta_p = p.with_extension("wta");
        if let Ok(wta_bytes) = fs::read(&wta_p) {
            primary_path = wta_p;
            wtp_path_opt = Some(p.clone());
            DocSnapshot::split(wta_bytes, raw)
        } else {
            DocSnapshot::single(raw)
        }
    } else if ext == "dat" {
        let dtt_p = p.with_extension("dtt");
        if let Ok(dtt_bytes) = fs::read(&dtt_p) {
            dtt_path_opt = Some(dtt_p);
            DocSnapshot::paired(raw, dtt_bytes)
        } else {
            DocSnapshot::single(raw)
        }
    } else if ext == "dtt" {
        let dat_p = p.with_extension("dat");
        if let Ok(dat_bytes) = fs::read(&dat_p) {
            primary_path = dat_p;
            dtt_path_opt = Some(p.clone());
            DocSnapshot::paired(dat_bytes, raw)
        } else {
            DocSnapshot::single(raw)
        }
    } else {
        DocSnapshot::single(raw)
    };

    let mut sess = lock_session(&state);
    sess.doc_path = Some(primary_path);
    sess.wtp_path = wtp_path_opt;
    sess.dtt_path = dtt_path_opt;
    sess.current = Some(snapshot);
    sess.dirty = false;
    sess.current_revision = 0;
    sess.saved_revision = 0;
    sess.next_revision = 1;
    sess.undo_stack.clear();
    sess.redo_stack.clear();
    // [wmb-studio BEGIN phase1]
    sess.doc_generation = sess.doc_generation.wrapping_add(1);
    sess.structure_version = 1;
    sess.committed_structure_version = 1;
    sess.uid_maps.clear();
    sess.committed_uid_maps.clear();
    sess.tex_thumb_cache.clear();
    sess.tex_thumb_order.clear();
    sess.wtx1_cache.clear();
    sess.wtx1_order.clear();
    sess.wtx1_cache_bytes = 0;
    // [wmb-studio END]

    sess.settings.recent_files.retain(|r| r != &path);
    sess.settings.recent_files.insert(0, path);
    if sess.settings.recent_files.len() > 12 {
        sess.settings.recent_files.truncate(12);
    }
    save_settings_to_disk(&sess.settings);

    catch_unwind(AssertUnwindSafe(|| build_outline_from_session(&sess)))
        .map_err(|_| "Internal panic while parsing document outline".to_string())?
}

#[tauri::command]
fn save_document(
    output_path: Option<String>,
    state: State<'_, AppState>,
) -> Result<DocumentOutline, String> {
    let mut sess = lock_session(&state);
    let dest = output_path
        .map(PathBuf::from)
        .or_else(|| sess.doc_path.clone())
        .ok_or("No active document path")?;
    let snap = sess.current.as_ref().ok_or("No active document loaded")?.clone();

    match &snap {
        DocSnapshot::SingleFile(bytes) => {
            atomic_write_file(&dest, bytes)?;
        }
        DocSnapshot::SplitWtaWtp { wta, wtp } => {
            let wtp_dest = dest.with_extension("wtp");
            atomic_write_pair(&dest, wta, &wtp_dest, wtp)?;
            sess.wtp_path = Some(wtp_dest);
        }
        DocSnapshot::PairedDatDtt { dat, dtt } => {
            let dtt_dest = dest.with_extension("dtt");
            atomic_write_pair(&dest, dat, &dtt_dest, dtt)?;
            sess.dtt_path = Some(dtt_dest);
        }
    }

    sess.doc_path = Some(dest);
    sess.saved_revision = sess.current_revision;
    sess.dirty = false;
    build_outline_from_session(&sess)
}

#[tauri::command]
fn undo_document(state: State<'_, AppState>) -> Result<DocumentOutline, String> {
    let mut sess = lock_session(&state);
    let entry = sess.undo_stack.pop().ok_or("Nothing to undo")?;
    sess.current = Some(entry.before.clone());
    sess.current_revision = entry.before_revision;
    // [wmb-studio BEGIN phase1]
    sess.uid_maps = entry.before_uid_maps.clone();
    sess.committed_uid_maps = entry.before_uid_maps.clone();
    sess.structure_version = entry.before_structure_version;
    sess.committed_structure_version = entry.before_structure_version;
    sess.doc_generation = sess.doc_generation.wrapping_add(1);
    // [wmb-studio END]
    sess.redo_stack.push(entry);
    sess.dirty = sess.current_revision != sess.saved_revision;
    build_outline_from_session(&sess)
}

#[tauri::command]
fn redo_document(state: State<'_, AppState>) -> Result<DocumentOutline, String> {
    let mut sess = lock_session(&state);
    let entry = sess.redo_stack.pop().ok_or("Nothing to redo")?;
    sess.current = Some(entry.after.clone());
    sess.current_revision = entry.after_revision;
    // [wmb-studio BEGIN phase1]
    sess.uid_maps = entry.after_uid_maps.clone();
    sess.committed_uid_maps = entry.after_uid_maps.clone();
    sess.structure_version = entry.after_structure_version;
    sess.committed_structure_version = entry.after_structure_version;
    sess.doc_generation = sess.doc_generation.wrapping_add(1);
    // [wmb-studio END]
    sess.undo_stack.push(entry);
    sess.dirty = sess.current_revision != sess.saved_revision;
    build_outline_from_session(&sess)
}

#[tauri::command]
fn inspect_asset_node(
    node_id: String,
    mip_index: Option<u32>,
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
    let oodle_path_opt = if sess.settings.oodle_dll_path.is_empty() {
        None
    } else {
        Some(Path::new(sess.settings.oodle_dll_path.as_str()))
    };

    let (sub_name, sub_bytes, sub_wtp, leaf) = resolve_subblob(snap, &node_id, oodle_path_opt)?;
    let display_name = if sub_name == "root" {
        doc_name.to_string()
    } else {
        sub_name.clone()
    };

    let tag = detect_file_format_tag(&display_name, &sub_bytes);

    // Case 1: Texture inside a WTB/WTA/BNTX bank (`tex:<idx>`) or standalone `.dds`/`.gtx`/`.xt1`/`.bntx`
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
            &format!("{doc_name} / {display_name} / 2D"),
            &entry.data,
            entry.flags,
            entry.texture_id,
            &entry.extension,
            wtb.game_profile,
            mip_index.unwrap_or(0),
            channel.as_deref().unwrap_or("RGBA"),
            opaque.unwrap_or(false),
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
            &format!("{doc_name} / 2D"),
            &sub_bytes,
            0,
            None,
            &format!(".{}", tag.to_ascii_lowercase()),
            profile,
            mip_index.unwrap_or(0),
            channel.as_deref().unwrap_or("RGBA"),
            opaque.unwrap_or(false),
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
                &format!("{doc_name} / {display_name} / 2D"),
                &e.data,
                e.flags,
                e.texture_id,
                &e.extension,
                wtb.game_profile,
                mip_index.unwrap_or(0),
                channel.as_deref().unwrap_or("RGBA"),
                opaque.unwrap_or(false),
            );
            return Ok(serde_json::json!({
                "view_type": "TEXTURE",
                "texture": view,
            }));
        }
    }

    // Case 2: WMB / WMB3 3D Model hierarchy
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

    // Case 3: BXM / XML / GAD / GAArea
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

    // Case 4: CSV Table (with UTF-8 / UTF-16 / Shift-JIS auto-detection and max-column padding)
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

    // Case 5: EST Particle Effect Table (standalone or inside .eff inside .dat)
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

    // Case 6: Cloth Physics (.clp, .clh, .clw)
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

    // Case 7: Structured Formats (MOT / EXP / COS / ATT / OBI / MSQ / COL / FLS / XAD / LIT / MDB)
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
            let oodle_found = OodleLibrary::find_dll_path(oodle_path_opt);
            for (i, d) in pkz.descriptors.iter().enumerate() {
                container_entries.push(serde_json::json!({
                    "index": i,
                    "name": d.name,
                    "compression": d.compression,
                    "size": d.size,
                    "compressed_size": d.compressed_size,
                    "offset": d.offset,
                }));
            }
            pkz_info = serde_json::json!({
                "is_lowercase_magic": pkz.is_lowercase_magic,
                "is_32byte_descriptors": pkz.is_32byte_descriptors,
                "uses_oodle": pkz.uses_oodle(),
                "oodle_available": true,
                "oodle_fallback_active": oodle_found.is_none(),
                "oodle_path": oodle_found.map(|p| p.to_string_lossy().into_owned()).unwrap_or_else(|| "Built-in Pure-Rust Oodle Kraken (ooz) Fallback".into()),
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

    let hex_preview: String = sub_bytes
        .iter()
        .take(512)
        .collect::<Vec<_>>()
        .chunks(16)
        .enumerate()
        .map(|(row, ch)| {
            let hex: Vec<String> = ch.iter().map(|b| format!("{b:02X}")).collect();
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
        })
        .collect::<Vec<_>>()
        .join("\n");

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

#[allow(clippy::too_many_arguments)]
fn build_texture_node_view(
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
    let is_bntx = (blob.len() >= 4 && &blob[0..4] == b"BNTX")
        || (blob.len() >= 84 && &blob[80..84] == b"BNTX");

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

    let raw_header_hex: String = blob
        .iter()
        .take(128)
        .collect::<Vec<_>>()
        .chunks(16)
        .enumerate()
        .map(|(r, ch)| {
            let h: Vec<String> = ch.iter().map(|b| format!("{b:02X}")).collect();
            format!("{:04X}: {}", r * 16, h.join(" "))
        })
        .collect::<Vec<_>>()
        .join("\n");

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

fn build_wmb1_material_options(
    snap: &DocSnapshot,
    node_id: &str,
    wmb: &WmbFile,
    is_b2: bool,
    oodle_dll: Option<&Path>,
) -> (Vec<MaterialPickerOption>, Vec<MaterialBankTextureOption>) {
    let (container_parts, _) = split_container_and_leaf(node_id);
    let base_prefix = if container_parts.is_empty() {
        "wmb".to_string()
    } else {
        format!("{}/wmb", container_parts.join("/"))
    };
    let mut all_ref_ids = Vec::new();
    let mut decoded_mats = Vec::with_capacity(wmb.materials.len());
    for (mi, m) in wmb.materials.iter().enumerate() {
        let dec = decode_wmb_material_rich(m, mi, is_b2);
        for f in &dec.fields {
            if let Some(tid) = f.sampler_tex_id {
                if !all_ref_ids.contains(&tid) {
                    all_ref_ids.push(tid);
                }
            }
        }
        decoded_mats.push((mi, m.mat_type, dec));
    }
    let bank_textures = collect_model_bank_textures(snap, node_id, &all_ref_ids, !is_b2, oodle_dll);
    let mut mat_options = Vec::with_capacity(decoded_mats.len());
    for (mi, mat_type, dec) in decoded_mats {
        let first_tid = dec.fields.iter().find_map(|f| f.sampler_tex_id);
        let matched_bank = first_tid.and_then(|tid| {
            if !is_b2 {
                bank_textures.iter().find(|bt| bt.index as u32 == tid || bt.texture_id == tid)
            } else {
                bank_textures.iter().find(|bt| bt.texture_id == tid || bt.index as u32 == tid)
            }
        });
        let shader_s = dec.shader_name.clone().unwrap_or_else(|| "Unknown".to_string());
        mat_options.push(MaterialPickerOption {
            index: mi,
            label: format!("Material #{mi} ({shader_s})"),
            shader_name: shader_s,
            badge: format!("0x{mat_type:02X}"),
            node_id: format!("{base_prefix}:mat:{mi}"),
            first_tex_id_hex: first_tid.map(|t| format!("0x{t:08X}")),
            thumbnail_data_url: matched_bank.and_then(|b| b.thumbnail_data_url.clone()),
        });
    }
    (mat_options, bank_textures)
}

fn build_wmb1_section_view(
    snap: &DocSnapshot,
    node_id: &str,
    model_name: &str,
    wmb: &WmbFile,
    section_key: &str,
    b2_override: Option<bool>,
    oodle_dll: Option<&Path>,
) -> WmbSectionView {
    let is_b2 = b2_override.unwrap_or_else(|| wmb.is_bayo2());
    let endian_str = format!("{:?}", wmb.endian);
    let spec_line = format!(
        "{} | {endian_str} endian | {} verts | {} bones | {} materials | {} meshes",
        if is_b2 {
            "WMB2 (Bayonetta 2)"
        } else {
            "WMB1 (Bayonetta 1)"
        },
        wmb.vertexes.len(),
        wmb.bone_hierarchy.len(),
        wmb.materials.len(),
        wmb.meshes.len()
    );
    let (material_options, _shared_bank_textures) =
        build_wmb1_material_options(snap, node_id, wmb, is_b2, oodle_dll);

    if let Some(mat_idx_str) = section_key.strip_prefix("mat:") {
        if let Ok(mi) = mat_idx_str.parse::<usize>() {
            if let Some(mat) = wmb.materials.get(mi) {
                let dec = decode_wmb_material_rich(mat, mi, is_b2);
                let ref_ids: Vec<u32> = dec
                    .fields
                    .iter()
                    .filter_map(|f| f.sampler_tex_id)
                    .collect();
                let bank_textures =
                    collect_model_bank_textures(snap, node_id, &ref_ids, !dec.is_bayo2, oodle_dll);
                let game_key = if dec.is_bayo2 { "b2" } else { "b1" };
                return WmbSectionView {
                    node_id: node_id.into(),
                    section: format!("mat:{mi}"),
                    title: format!("Material #{mi}"),
                    breadcrumb: format!("{model_name} / Materials / #{mi}"),
                    spec_line,
                    validation_msg: format!(
                        "Shader {} (Type 0x{:02X}) · {} bytes",
                        dec.shader_name.as_deref().unwrap_or("Unknown"),
                        mat.mat_type,
                        mat.byte_size()
                    ),
                    is_wmb3: false,
                    is_bayo2: dec.is_bayo2,
                    endian: endian_str,
                    data: serde_json::to_value(&dec).unwrap_or_default(),
                    material_decoded: Some(dec),
                    wmb3_material: None,
                    wmb3_named_params: Vec::new(),
                    wmb3_texture_views: Vec::new(),
                    bank_textures,
                    core_pretextures: list_core_pretextures(game_key),
                    available_shaders: list_available_shaders(game_key),
                    material_options: material_options.clone(),
                };
            }
        }
    }

    if let Some(rest) = section_key.strip_prefix("mesh:") {
        let parts: Vec<&str> = rest.split(":batch:").collect();
        if let Ok(mesh_idx) = parts[0].parse::<usize>() {
            if let Some(mesh) = wmb.meshes.get(mesh_idx) {
                if parts.len() == 2 {
                    if let Ok(bi) = parts[1].parse::<usize>() {
                        if let Some(batch) = mesh.batches.get(bi) {
                            return WmbSectionView {
                                node_id: node_id.into(),
                                section: format!("mesh:{mesh_idx}:batch:{bi}"),
                                title: format!("Mesh #{mesh_idx} ({}) / Batch #{bi}", mesh.name),
                                breadcrumb: format!(
                                    "{model_name} / Meshes / #{mesh_idx} / Batch #{bi}"
                                ),
                                spec_line,
                                validation_msg: format!(
                                    "Material #{} · {} indices · Bone refs: {}{}",
                                    batch_effective_material_id(&batch.header, is_b2),
                                    batch.indices.len(),
                                    batch.bone_refs.len(),
                                    if batch_is_shadow_proxy(&batch.header, is_b2) {
                                        " · Shadow Proxy (u_e1/u_e2)"
                                    } else {
                                        ""
                                    }
                                ),
                                is_wmb3: false,
                                is_bayo2: is_b2,
                                endian: endian_str,
                                data: serde_json::json!({
                                    "mesh_index": mesh_idx,
                                    "mesh_name": mesh.name,
                                    "batch_index": bi,
                                    "batch": batch,
                                }),
                                material_decoded: None,
                                wmb3_material: None,
                                wmb3_named_params: Vec::new(),
                                wmb3_texture_views: Vec::new(),
                                bank_textures: Vec::new(),
                                core_pretextures: Vec::new(),
                                available_shaders: Vec::new(),
                                material_options: material_options.clone(),
                            };
                        }
                    }
                }
                return WmbSectionView {
                    node_id: node_id.into(),
                    section: format!("mesh:{mesh_idx}"),
                    title: format!("Mesh #{mesh_idx} ({})", mesh.name),
                    breadcrumb: format!("{model_name} / Meshes / #{mesh_idx}"),
                    spec_line,
                    validation_msg: format!(
                        "{} batches in mesh '{}'",
                        mesh.batches.len(),
                        mesh.name
                    ),
                    is_wmb3: false,
                    is_bayo2: is_b2,
                    endian: endian_str,
                    data: serde_json::to_value(mesh).unwrap_or_default(),
                    material_decoded: None,
                    wmb3_material: None,
                    wmb3_named_params: Vec::new(),
                    wmb3_texture_views: Vec::new(),
                    bank_textures: Vec::new(),
                    core_pretextures: Vec::new(),
                    available_shaders: Vec::new(),
                    material_options: material_options.clone(),
                };
            }
        }
    }

    // [wmb-studio BEGIN phase1]
    if let Some(bone_idx_str) = section_key.strip_prefix("bone:") {
        if let Ok(bi) = bone_idx_str.parse::<usize>() {
            let dumped = wmb.dump_bones();
            if let Some(b) = dumped.get(bi) {
                return WmbSectionView {
                    node_id: node_id.into(),
                    section: format!("bone:{bi}"),
                    title: format!("Bone #{} (global {})", bi, b.global_index),
                    breadcrumb: format!("{model_name} / Bones / #{bi}"),
                    spec_line,
                    validation_msg: format!(
                        "Local #{bi} · Global {} · Parent {}",
                        b.global_index, b.parent_index
                    ),
                    is_wmb3: false,
                    is_bayo2: is_b2,
                    endian: endian_str,
                    data: serde_json::json!({
                        "index": b.index,
                        "global_id": b.global_index,
                        "parent_index": b.parent_index,
                        "relative_position": [b.relative_position.x, b.relative_position.y, b.relative_position.z],
                        "position": [b.position.x, b.position.y, b.position.z],
                        "symmetric": b.symmetric,
                        "flag": b.flag,
                    }),
                    material_decoded: None,
                    wmb3_material: None,
                    wmb3_named_params: Vec::new(),
                    wmb3_texture_views: Vec::new(),
                    bank_textures: Vec::new(),
                    core_pretextures: Vec::new(),
                    available_shaders: Vec::new(),
                    material_options: material_options.clone(),
                };
            }
        }
    }
    // [wmb-studio END]

    match section_key {
        "bones" => {
            let bones: Vec<serde_json::Value> = (0..wmb.bone_hierarchy.len())
                .map(|i| {
                    let parent = wmb.bone_hierarchy[i];
                    let rel = wmb.bone_relative_positions.get(i).copied().unwrap_or_default();
                    let pos = wmb.bone_positions.get(i).copied().unwrap_or_default();
                    let global_id = wmb
                        .bone_index_translate_table
                        .invert()
                        .get(&(i as u16))
                        .copied()
                        .unwrap_or(i as u16);
                    serde_json::json!({
                        "index": i,
                        "global_id": global_id,
                        "parent_index": parent,
                        "relative_position": [rel.x, rel.y, rel.z],
                        "position": [pos.x, pos.y, pos.z],
                    })
                })
                .collect();
            WmbSectionView {
                node_id: node_id.into(),
                section: "bones".into(),
                title: format!("Skeleton Hierarchy ({} bones)", bones.len()),
                breadcrumb: format!("{model_name} / Bones"),
                spec_line,
                validation_msg: format!(
                    "{} bones · {} global translate entries",
                    bones.len(),
                    wmb.bone_index_translate_table.table.len()
                ),
                is_wmb3: false,
                is_bayo2: is_b2,
                endian: endian_str,
                data: serde_json::json!({
                    "bones": bones,
                    "translate_table": wmb.bone_index_translate_table.table,
                }),
                material_decoded: None,
                wmb3_material: None,
                wmb3_named_params: Vec::new(),
                wmb3_texture_views: Vec::new(),
                bank_textures: Vec::new(),
                core_pretextures: Vec::new(),
                available_shaders: Vec::new(),
                material_options: material_options.clone(),
            }
        }
        "vertexes" => {
            let preview_verts: Vec<_> = wmb.vertexes.iter().take(250).collect();
            WmbSectionView {
                node_id: node_id.into(),
                section: "vertexes".into(),
                title: format!("Vertex Buffer ({} total)", wmb.vertexes.len()),
                breadcrumb: format!("{model_name} / Vertexes"),
                spec_line,
                validation_msg: format!(
                    "Vertex flags 0x{:08X} · showing first {} of {} vertices",
                    wmb.header.vertex_flags,
                    preview_verts.len(),
                    wmb.vertexes.len()
                ),
                is_wmb3: false,
                is_bayo2: is_b2,
                endian: endian_str,
                data: serde_json::to_value(preview_verts).unwrap_or_default(),
                material_decoded: None,
                wmb3_material: None,
                wmb3_named_params: Vec::new(),
                wmb3_texture_views: Vec::new(),
                bank_textures: Vec::new(),
                core_pretextures: Vec::new(),
                available_shaders: Vec::new(),
                material_options: material_options.clone(),
            }
        }
        "materials" => {
            let mats: Vec<DecodedWmbMaterial> = wmb
                .materials
                .iter()
                .enumerate()
                .map(|(i, m)| decode_wmb_material_rich(m, i, is_b2))
                .collect();
            let first_dec = mats.first().cloned();
            let ref_ids: Vec<u32> = first_dec
                .as_ref()
                .map(|d| d.fields.iter().filter_map(|f| f.sampler_tex_id).collect())
                .unwrap_or_default();
            let game_key = if is_b2 { "b2" } else { "b1" };
            let bank_textures =
                collect_model_bank_textures(snap, node_id, &ref_ids, !is_b2, oodle_dll);
            WmbSectionView {
                node_id: node_id.into(),
                section: "materials".into(),
                title: format!("Materials Table ({})", mats.len()),
                breadcrumb: format!("{model_name} / Materials"),
                spec_line,
                validation_msg: format!(
                    "{} material definitions decoded against shader database",
                    mats.len()
                ),
                is_wmb3: false,
                is_bayo2: is_b2,
                endian: endian_str,
                data: serde_json::to_value(&mats).unwrap_or_default(),
                material_decoded: first_dec,
                wmb3_material: None,
                wmb3_named_params: Vec::new(),
                wmb3_texture_views: Vec::new(),
                bank_textures,
                core_pretextures: list_core_pretextures(game_key),
                available_shaders: list_available_shaders(game_key),
                material_options: material_options.clone(),
            }
        }
        "meshes" => WmbSectionView {
            node_id: node_id.into(),
            section: "meshes".into(),
            title: format!("Meshes Table ({})", wmb.meshes.len()),
            breadcrumb: format!("{model_name} / Meshes"),
            spec_line,
            validation_msg: format!("{} meshes parsed", wmb.meshes.len()),
            is_wmb3: false,
            is_bayo2: is_b2,
            endian: endian_str,
            data: serde_json::to_value(&wmb.meshes).unwrap_or_default(),
            material_decoded: None,
            wmb3_material: None,
            wmb3_named_params: Vec::new(),
            wmb3_texture_views: Vec::new(),
            bank_textures: Vec::new(),
            core_pretextures: Vec::new(),
            available_shaders: Vec::new(),
            material_options: material_options.clone(),
        },
        _ => WmbSectionView {
            node_id: node_id.into(),
            section: "header".into(),
            title: format!("WMB Header ({model_name})"),
            breadcrumb: format!("{model_name} / Header"),
            spec_line,
            validation_msg:
                "Offsets managed automatically on save — no manual offset calculation needed."
                    .into(),
            is_wmb3: false,
            is_bayo2: is_b2,
            endian: endian_str,
            data: serde_json::to_value(&wmb.header).unwrap_or_default(),
            material_decoded: None,
            wmb3_material: None,
            wmb3_named_params: Vec::new(),
            wmb3_texture_views: Vec::new(),
            bank_textures: Vec::new(),
            core_pretextures: Vec::new(),
            available_shaders: Vec::new(),
            material_options: material_options.clone(),
        },
    }
}

fn build_wmb3_material_options(
    snap: &DocSnapshot,
    node_id: &str,
    wmb3: &Wmb3File,
    oodle_dll: Option<&Path>,
) -> (Vec<MaterialPickerOption>, Vec<MaterialBankTextureOption>) {
    let (container_parts, _) = split_container_and_leaf(node_id);
    let base_prefix = if container_parts.is_empty() {
        "wmb3".to_string()
    } else {
        format!("{}/wmb3", container_parts.join("/"))
    };
    let mut all_ref_ids = Vec::new();
    for m in &wmb3.materials {
        for (_, tid) in &m.textures {
            if !all_ref_ids.contains(tid) {
                all_ref_ids.push(*tid);
            }
        }
    }
    let bank_textures = collect_model_bank_textures(snap, node_id, &all_ref_ids, false, oodle_dll);
    let mut mat_options = Vec::with_capacity(wmb3.materials.len());
    for (mi, m) in wmb3.materials.iter().enumerate() {
        let first_tid = m
            .textures
            .iter()
            .find(|(sname, _)| sname.contains("Albedo") || sname.contains("Color") || sname == "g_Texture0")
            .or_else(|| m.textures.first())
            .map(|(_, id)| *id);
        let matched_bank = first_tid.and_then(|tid| {
            bank_textures
                .iter()
                .find(|bt| bt.texture_id == tid || bt.index as u32 == tid)
        });
        mat_options.push(MaterialPickerOption {
            index: mi,
            label: format!("Material #{mi} {} ({})", m.name, m.shader_name),
            shader_name: m.shader_name.clone(),
            badge: m.technique_name.clone(),
            node_id: format!("{base_prefix}:mat:{mi}"),
            first_tex_id_hex: first_tid.map(|t| format!("0x{t:08X}")),
            thumbnail_data_url: matched_bank.and_then(|b| b.thumbnail_data_url.clone()),
        });
    }
    (mat_options, bank_textures)
}

fn build_wmb3_section_view(
    snap: &DocSnapshot,
    node_id: &str,
    model_name: &str,
    wmb3: &Wmb3File,
    section_key: &str,
    oodle_dll: Option<&Path>,
) -> WmbSectionView {
    let endian_str = "Little".to_string();
    let (material_options, _shared_bank_textures) =
        build_wmb3_material_options(snap, node_id, wmb3, oodle_dll);
    
    let total_verts: usize = wmb3.vertex_groups.iter().map(|vg| vg.vertexes.len()).sum();
    let game_label = if wmb3.version == 0x2020_0824 {
        "WMB3 (Bayonetta 3)"
    } else {
        "WMB3 (NieR:Automata / Astral Chain / MGRR)"
    };
    let spec_line = format!(
        "{game_label} | {endian_str} endian | {total_verts} verts | {} bones | {} materials | {} meshes",
        wmb3.bones.len(),
        wmb3.materials.len(),
        wmb3.meshes.len()
    );

    if let Some(mat_idx_str) = section_key.strip_prefix("mat:") {
        if let Ok(mi) = mat_idx_str.parse::<usize>() {
            if let Some(mat) = wmb3.materials.get(mi) {
                let ref_ids: Vec<u32> = mat.textures.iter().map(|&(_, id)| id).collect();
                let bank_textures =
                    collect_model_bank_textures(snap, node_id, &ref_ids, false, oodle_dll);
                let named_params = mat.named_parameters();
                let tex_views = mat.texture_views();
                return WmbSectionView {
                    node_id: node_id.into(),
                    section: format!("mat:{mi}"),
                    title: format!("WMB3 Material #{mi} ({})", mat.name),
                    breadcrumb: format!("{model_name} / Materials / #{mi}"),
                    spec_line,
                    validation_msg: format!(
                        "Shader {} · Technique {} · {} textures · {} parameters",
                        mat.shader_name,
                        mat.technique_name,
                        mat.textures.len(),
                        named_params.len() + mat.variables.len()
                    ),
                    is_wmb3: true,
                    is_bayo2: false,
                    endian: endian_str,
                    data: serde_json::to_value(mat).unwrap_or_default(),
                    material_decoded: None,
                    wmb3_material: Some(mat.clone()),
                    wmb3_named_params: named_params,
                    wmb3_texture_views: tex_views,
                    bank_textures,
                    core_pretextures: list_core_pretextures("b3"),
                    available_shaders: list_available_shaders("b3"),
                    material_options: material_options.clone(),
                };
            }
        }
    }

    // [wmb-studio BEGIN phase1]
    if let Some(mesh_idx_str) = section_key.strip_prefix("mesh:") {
        if let Ok(mi) = mesh_idx_str.parse::<usize>() {
            if let Some(mesh) = wmb3.meshes.get(mi) {
                return WmbSectionView {
                    node_id: node_id.into(),
                    section: format!("mesh:{mi}"),
                    title: format!("WMB3 Mesh #{mi} ({})", mesh.name),
                    breadcrumb: format!("{model_name} / Meshes / #{mi}"),
                    spec_line,
                    validation_msg: format!(
                        "Mesh {} · {} materials · {} bones",
                        mesh.name,
                        mesh.materials.len(),
                        mesh.bone_indices.len()
                    ),
                    is_wmb3: true,
                    is_bayo2: false,
                    endian: endian_str,
                    data: serde_json::json!({
                        "mesh_index": mi,
                        "mesh": mesh,
                        "meshes": wmb3.meshes,
                        "batches": wmb3.batches,
                        "lods": wmb3.lods,
                        "bone_sets": wmb3.bone_sets,
                    }),
                    material_decoded: None,
                    wmb3_material: None,
                    wmb3_named_params: Vec::new(),
                    wmb3_texture_views: Vec::new(),
                    bank_textures: Vec::new(),
                    core_pretextures: Vec::new(),
                    available_shaders: Vec::new(),
                    material_options: material_options.clone(),
                };
            }
        }
    }

    if let Some(lod_rest) = section_key.strip_prefix("lod:") {
        let parts: Vec<&str> = lod_rest.split(':').collect();
        let (mesh_opt, lvl_opt) = if parts.len() == 2 {
            (parts[0].parse::<usize>().ok(), parts[1].parse::<i32>().ok())
        } else {
            (None, parts[0].parse::<i32>().ok())
        };
        if let Some(lvl) = lvl_opt {
            if let Some((li, lod)) = wmb3
                .lods
                .iter()
                .enumerate()
                .find(|(_, l)| l.lod_level == lvl)
            {
                return WmbSectionView {
                    node_id: node_id.into(),
                    section: format!("lod:{lod_rest}"),
                    title: format!("WMB3 LOD {} ({})", lod.lod_level, lod.name),
                    breadcrumb: format!("{model_name} / LOD {}", lod.lod_level),
                    spec_line,
                    validation_msg: format!(
                        "LOD #{li} {} (level {}) · {} batches",
                        lod.name,
                        lod.lod_level,
                        lod.batch_infos.len()
                    ),
                    is_wmb3: true,
                    is_bayo2: false,
                    endian: endian_str,
                    data: serde_json::json!({
                        "lod_index": li,
                        "mesh_index": mesh_opt,
                        "lod": lod,
                        "meshes": wmb3.meshes,
                        "batches": wmb3.batches,
                        "lods": wmb3.lods,
                    }),
                    material_decoded: None,
                    wmb3_material: None,
                    wmb3_named_params: Vec::new(),
                    wmb3_texture_views: Vec::new(),
                    bank_textures: Vec::new(),
                    core_pretextures: Vec::new(),
                    available_shaders: Vec::new(),
                    material_options: material_options.clone(),
                };
            }
        }
    }

    if let Some(batch_idx_str) = section_key.strip_prefix("batch:") {
        if let Ok(bi) = batch_idx_str.parse::<usize>() {
            if let Some(batch) = wmb3.batches.get(bi) {
                let mut batch_info_val = serde_json::Value::Null;
                let mut lod_level = 0i32;
                for lod in &wmb3.lods {
                    let start = lod.batch_start as usize;
                    let end = start + lod.batch_infos.len();
                    if bi >= start && bi < end {
                        batch_info_val =
                            serde_json::to_value(&lod.batch_infos[bi - start]).unwrap_or_default();
                        lod_level = lod.lod_level;
                        break;
                    }
                }
                return WmbSectionView {
                    node_id: node_id.into(),
                    section: format!("batch:{bi}"),
                    title: format!("WMB3 Batch #{bi} (LOD {lod_level})"),
                    breadcrumb: format!("{model_name} / Batches / #{bi}"),
                    spec_line,
                    validation_msg: format!(
                        "VG #{} · {} tris ({} idx)",
                        batch.vertex_group_index, batch.num_primitives, batch.num_indices
                    ),
                    is_wmb3: true,
                    is_bayo2: false,
                    endian: endian_str,
                    data: serde_json::json!({
                        "batch_index": bi,
                        "batch": batch,
                        "batch_info": batch_info_val,
                        "meshes": wmb3.meshes,
                        "batches": wmb3.batches,
                        "lods": wmb3.lods,
                        "bone_sets": wmb3.bone_sets,
                    }),
                    material_decoded: None,
                    wmb3_material: None,
                    wmb3_named_params: Vec::new(),
                    wmb3_texture_views: Vec::new(),
                    bank_textures: Vec::new(),
                    core_pretextures: Vec::new(),
                    available_shaders: Vec::new(),
                    material_options: material_options.clone(),
                };
            }
        }
    }

    if let Some(bone_idx_str) = section_key.strip_prefix("bone:") {
        if let Ok(bi) = bone_idx_str.parse::<usize>() {
            if let Some(b) = wmb3.bones.get(bi) {
                return WmbSectionView {
                    node_id: node_id.into(),
                    section: format!("bone:{bi}"),
                    title: format!("WMB3 Bone #{bi} (global {})", b.id),
                    breadcrumb: format!("{model_name} / Bones / #{bi}"),
                    spec_line,
                    validation_msg: format!("Global ID {} · Parent {}", b.id, b.parent_index),
                    is_wmb3: true,
                    is_bayo2: false,
                    endian: endian_str,
                    data: serde_json::json!({
                        "index": bi,
                        "global_id": b.id,
                        "parent_index": b.parent_index,
                        "local_position": [b.local_position.x, b.local_position.y, b.local_position.z],
                        "local_rotation": [b.local_rotation.x, b.local_rotation.y, b.local_rotation.z],
                        "local_scale": [b.local_scale.x, b.local_scale.y, b.local_scale.z],
                        "position": [b.position.x, b.position.y, b.position.z],
                        "rotation": [b.rotation.x, b.rotation.y, b.rotation.z],
                        "scale": [b.scale.x, b.scale.y, b.scale.z],
                        "t_position": [b.t_position.x, b.t_position.y, b.t_position.z],
                        "bones": wmb3.bones,
                    }),
                    material_decoded: None,
                    wmb3_material: None,
                    wmb3_named_params: Vec::new(),
                    wmb3_texture_views: Vec::new(),
                    bank_textures: Vec::new(),
                    core_pretextures: Vec::new(),
                    available_shaders: Vec::new(),
                    material_options: material_options.clone(),
                };
            }
        }
    }
    // [wmb-studio END]

    match section_key {
        "bones" => WmbSectionView {
            node_id: node_id.into(),
            section: "bones".into(),
            title: format!("WMB3 Skeleton ({} bones)", wmb3.bones.len()),
            breadcrumb: format!("{model_name} / Bones"),
            spec_line,
            validation_msg: format!("{} bones with T-pose transforms", wmb3.bones.len()),
            is_wmb3: true,
            is_bayo2: false,
            endian: endian_str,
            data: serde_json::to_value(&wmb3.bones).unwrap_or_default(),
            material_decoded: None,
            wmb3_material: None,
            wmb3_named_params: Vec::new(),
            wmb3_texture_views: Vec::new(),
            bank_textures: Vec::new(),
            core_pretextures: Vec::new(),
            available_shaders: Vec::new(),
            material_options: material_options.clone(),
        },
        "materials" => {
            let first_mat = wmb3.materials.first().cloned();
            let ref_ids: Vec<u32> = first_mat
                .as_ref()
                .map(|m| m.textures.iter().map(|&(_, id)| id).collect())
                .unwrap_or_default();
            let bank_textures =
                collect_model_bank_textures(snap, node_id, &ref_ids, false, oodle_dll);
            let named_params = first_mat
                .as_ref()
                .map(|m| m.named_parameters())
                .unwrap_or_default();
            let tex_views = first_mat
                .as_ref()
                .map(|m| m.texture_views())
                .unwrap_or_default();
            WmbSectionView {
                node_id: node_id.into(),
                section: "materials".into(),
                title: format!("WMB3 Materials ({})", wmb3.materials.len()),
                breadcrumb: format!("{model_name} / Materials"),
                spec_line,
                validation_msg: format!("{} named shader materials", wmb3.materials.len()),
                is_wmb3: true,
                is_bayo2: false,
                endian: endian_str,
                data: serde_json::to_value(&wmb3.materials).unwrap_or_default(),
                material_decoded: None,
                wmb3_material: first_mat,
                wmb3_named_params: named_params,
                wmb3_texture_views: tex_views,
                bank_textures,
                core_pretextures: list_core_pretextures("b3"),
                available_shaders: list_available_shaders("b3"),
                material_options: material_options.clone(),
            }
        }
        "meshes" => WmbSectionView {
            node_id: node_id.into(),
            section: "meshes".into(),
            title: format!(
                "WMB3 Meshes ({}) & Batches ({})",
                wmb3.meshes.len(),
                wmb3.batches.len()
            ),
            breadcrumb: format!("{model_name} / Meshes"),
            spec_line,
            validation_msg: format!(
                "{} meshes · {} batches · {} LODs",
                wmb3.meshes.len(),
                wmb3.batches.len(),
                wmb3.lods.len()
            ),
            is_wmb3: true,
            is_bayo2: false,
            endian: endian_str,
            data: serde_json::json!({
                "meshes": wmb3.meshes,
                "batches": wmb3.batches,
                "lods": wmb3.lods,
                "bone_sets": wmb3.bone_sets,
            }),
            material_decoded: None,
            wmb3_material: None,
            wmb3_named_params: Vec::new(),
            wmb3_texture_views: Vec::new(),
            bank_textures: Vec::new(),
            core_pretextures: Vec::new(),
            available_shaders: Vec::new(),
            material_options: material_options.clone(),
        },
        _ => WmbSectionView {
            node_id: node_id.into(),
            section: "header".into(),
            title: format!("WMB3 Header ({model_name})"),
            breadcrumb: format!("{model_name} / Header"),
            spec_line,
            validation_msg: format!("Version 0x{:08X} · Flags 0x{:04X}", wmb3.version, wmb3.flags),
            is_wmb3: true,
            is_bayo2: false,
            endian: endian_str,
            data: serde_json::json!({
                "version": format!("0x{:08X}", wmb3.version),
                "u_a": wmb3.u_a,
                "flags": wmb3.flags,
                "u_c": wmb3.u_c,
                "bounding_box": wmb3.bounding_box,
                "vertex_groups_count": wmb3.vertex_groups.len(),
                "batches_count": wmb3.batches.len(),
                "lods_count": wmb3.lods.len(),
            }),
            material_decoded: None,
            wmb3_material: None,
            wmb3_named_params: Vec::new(),
            wmb3_texture_views: Vec::new(),
            bank_textures: Vec::new(),
            core_pretextures: Vec::new(),
            available_shaders: Vec::new(),
            material_options: material_options.clone(),
        },
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
fn mutate_texture_node(
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
            let new_blob = if low_path.ends_with(".png") || low_path.ends_with(".jpg") || low_path.ends_with(".jpeg") || low_path.ends_with(".tga") || low_path.ends_with(".bmp") {
                let img = image::load_from_memory(&raw).map_err(|e| e.to_string())?.to_rgba8();
                encode_image_for_bank_entry(&img, target_ext, target_fmt, generate_mips, apply_solidify, make_opaque, Some(&sub_bytes), nvtt_opt)
            } else if target_ext == ".xt1" && raw.starts_with(b"DDS ") {
                swizzle_dds_to_xt1(&raw, Some(&sub_bytes)).map_err(|e| e.to_string())?
            } else {
                raw
            };
            let next_snap = write_back_subblob(&snap, &container_parts, new_blob, None, oodle_opt)?;
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
            let new_blob = if low_path.ends_with(".png")
                || low_path.ends_with(".jpg")
                || low_path.ends_with(".jpeg")
                || low_path.ends_with(".tga")
                || low_path.ends_with(".bmp")
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
            let new_blob = if low_path.ends_with(".png")
                || low_path.ends_with(".jpg")
                || low_path.ends_with(".jpeg")
                || low_path.ends_with(".tga")
                || low_path.ends_with(".bmp")
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
            let is_bntx_container = (sub_bytes.len() >= 4 && &sub_bytes[0..4] == b"BNTX")
                || (sub_bytes.len() >= 84 && &sub_bytes[80..84] == b"BNTX");
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
                            let blob = if fname.ends_with(".png") {
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

    let is_standalone_bntx = (sub_bytes.len() >= 4 && &sub_bytes[0..4] == b"BNTX")
        || (sub_bytes.len() >= 84 && &sub_bytes[80..84] == b"BNTX");
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
    record_mutation(&mut sess, desc, next_snap);
    build_outline_from_session(&sess)
}

#[tauri::command]
fn export_texture_action(
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
            let is_bntx = (blob.len() >= 4 && &blob[0..4] == b"BNTX")
                || (blob.len() >= 84 && &blob[80..84] == b"BNTX");
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

#[tauri::command]
fn convert_material_action(
    src_game: String,
    dst_game: String,
    material_payload: serde_json::Value,
    target_shader: Option<String>,
) -> Result<serde_json::Value, String> {
    let wmb_mat: Option<DecodedWmbMaterial> = if src_game == "b1" || src_game == "b2" {
        serde_json::from_value(material_payload.clone()).ok()
    } else {
        None
    };
    let wmb3_mat: Option<Wmb3Material> = if src_game == "b3" {
        serde_json::from_value(material_payload).ok()
    } else {
        None
    };
    let (converted, report) = convert_material_between_games(
        &src_game,
        &dst_game,
        wmb_mat.as_ref(),
        wmb3_mat.as_ref(),
        target_shader.as_deref(),
    )?;
    Ok(serde_json::json!({
        "converted": converted,
        "report": report,
    }))
}

fn validate_bone_parent(
    num_bones: usize,
    bone_idx: usize,
    new_parent: i64,
    get_parent: impl Fn(usize) -> i16,
) -> Result<i16, String> {
    if new_parent < -1 || new_parent >= num_bones as i64 {
        return Err(format!(
            "Parent bone index {new_parent} is out of range (-1..{})",
            num_bones.saturating_sub(1)
        ));
    }
    if new_parent == bone_idx as i64 {
        return Err(format!("Bone #{bone_idx} cannot be its own parent"));
    }
    let mut cur = new_parent;
    let mut steps = 0usize;
    while cur >= 0 {
        if cur as usize == bone_idx {
            return Err(format!(
                "Cannot reparent bone #{bone_idx} to its own descendant #{new_parent} (cycle detected)"
            ));
        }
        if cur as usize >= num_bones || steps > num_bones {
            return Err("Cycle detected in bone hierarchy".into());
        }
        cur = get_parent(cur as usize) as i64;
        steps += 1;
    }
    Ok(new_parent as i16)
}

#[tauri::command]
fn mutate_wmb_node(
    node_id: String,
    operation: String,
    payload: serde_json::Value,
    state: State<'_, AppState>,
) -> Result<DocumentOutline, String> {
    // [wmb-studio BEGIN phase1]
    let clipboard_opt = if operation == "paste_items" {
        lock_clipboard(&state).clone()
    } else {
        None
    };
    // [wmb-studio END]
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

    // Work on a cloned UID map and structure flag; commit only after all fallible steps succeed (#5, #22)
    let orig_uid_map = sess
        .uid_maps
        .get(&container_id)
        .cloned()
        .unwrap_or_else(UidMap::new);
    let mut working_uid_map = orig_uid_map.clone();
    let mut bump_structure = false;
    let mut coalesce_key: Option<String> = None;

    // Full WMB3 mutation & binary re-serialization (`Wmb3File::to_bytes`)!
    if sub_bytes.starts_with(b"WMB3") || sub_bytes.starts_with(b"3BMW") {
        let mut wmb3 = Wmb3File::from_bytes(&sub_bytes).map_err(|e| e.to_string())?;
        let mut init_committed = orig_uid_map;
        init_committed.ensure_wmb3(&wmb3);
        working_uid_map.ensure_wmb3(&wmb3);

        let desc = match operation.as_str() {
            "export_obj" => {
                let out_path = payload
                    .get("path")
                    .and_then(|v| v.as_str())
                    .ok_or("Missing OBJ export path")?;
                fs::write(out_path, wmb3.export_obj()).map_err(|e| e.to_string())?;
                return build_outline_from_session(&sess);
            }
            "update_wmb3_material" => {
                let idx = payload
                    .get("index")
                    .and_then(|v| v.as_u64())
                    .ok_or("Missing WMB3 material index")? as usize;
                let updated: Wmb3Material = serde_json::from_value(
                    payload
                        .get("material")
                        .cloned()
                        .unwrap_or_else(|| payload.clone()),
                )
                .map_err(|e| e.to_string())?;
                let slot = wmb3
                    .materials
                    .get_mut(idx)
                    .ok_or_else(|| format!("WMB3 Material index #{idx} out of bounds"))?;
                *slot = updated;
                format!("Update WMB3 Material #{idx}")
            }
            "switch_shader" => {
                let idx = payload
                    .get("index")
                    .and_then(|v| v.as_u64())
                    .ok_or("Missing WMB3 material index")? as usize;
                let new_shader = payload
                    .get("shader")
                    .and_then(|v| v.as_str())
                    .ok_or("Missing shader name")?;
                let mat = wmb3
                    .materials
                    .get_mut(idx)
                    .ok_or("WMB3 Material index out of bounds")?;
                switch_wmb3_material_shader(mat, new_shader);
                format!("Switch WMB3 Material #{idx} Shader to {new_shader}")
            }
            "update_bone" => {
                let b_idx = payload
                    .get("index")
                    .and_then(|v| v.as_u64())
                    .ok_or("Missing bone index")? as usize;
                if b_idx >= wmb3.bones.len() {
                    return Err("WMB3 bone index out of bounds".into());
                }
                let validated_parent =
                    if let Some(parent) = payload.get("parent_index").and_then(|v| v.as_i64()) {
                        Some(validate_bone_parent(
                            wmb3.bones.len(),
                            b_idx,
                            parent,
                            |i| wmb3.bones[i].parent_index,
                        )?)
                    } else {
                        None
                    };
                let parent_world_pos = {
                    let eff_p = validated_parent.unwrap_or(wmb3.bones[b_idx].parent_index);
                    if eff_p >= 0 && (eff_p as usize) < wmb3.bones.len() {
                        Some(wmb3.bones[eff_p as usize].position)
                    } else {
                        None
                    }
                };
                let bone = &mut wmb3.bones[b_idx];
                if let Some(valid_p) = validated_parent {
                    bone.parent_index = valid_p;
                    if let Some(pp) = parent_world_pos {
                        bone.local_position = pgtools_core::bone::Vec3::new(
                            bone.position.x - pp.x,
                            bone.position.y - pp.y,
                            bone.position.z - pp.z,
                        );
                    } else {
                        bone.local_position = bone.position;
                    }
                    bump_structure = true;
                }
                if let Some(id_i64) = payload.get("global_id").and_then(|v| v.as_i64()) {
                    let new_gid = i16::try_from(id_i64).map_err(|_| "Invalid bone global_id")?;
                    let old_gid = bone.id;
                    bone.id = new_gid;
                    if old_gid >= 0 {
                        wmb3.bone_index_translate_table.table.remove(&(old_gid as u16));
                    }
                    if new_gid >= 0 {
                        wmb3.bone_index_translate_table
                            .table
                            .insert(new_gid as u16, b_idx as u16);
                    }
                    wmb3.raw_btt.clear();
                }
                if let Some(pos) = payload.get("position").and_then(|v| v.as_array()) {
                    if pos.len() == 3 {
                        bone.position.x = pos[0].as_f64().unwrap_or(bone.position.x as f64) as f32;
                        bone.position.y = pos[1].as_f64().unwrap_or(bone.position.y as f64) as f32;
                        bone.position.z = pos[2].as_f64().unwrap_or(bone.position.z as f64) as f32;
                        if payload.get("relative_position").is_none() {
                            if let Some(pp) = parent_world_pos {
                                bone.local_position = pgtools_core::bone::Vec3::new(
                                    bone.position.x - pp.x,
                                    bone.position.y - pp.y,
                                    bone.position.z - pp.z,
                                );
                            } else {
                                bone.local_position = bone.position;
                            }
                        }
                    }
                }
                if let Some(rel) = payload.get("relative_position").and_then(|v| v.as_array()) {
                    if rel.len() == 3 {
                        bone.local_position.x =
                            rel[0].as_f64().unwrap_or(bone.local_position.x as f64) as f32;
                        bone.local_position.y =
                            rel[1].as_f64().unwrap_or(bone.local_position.y as f64) as f32;
                        bone.local_position.z =
                            rel[2].as_f64().unwrap_or(bone.local_position.z as f64) as f32;
                        if payload.get("position").is_none() {
                            if let Some(pp) = parent_world_pos {
                                bone.position = pgtools_core::bone::Vec3::new(
                                    pp.x + bone.local_position.x,
                                    pp.y + bone.local_position.y,
                                    pp.z + bone.local_position.z,
                                );
                            } else {
                                bone.position = bone.local_position;
                            }
                        }
                    }
                }
                // [wmb-studio BEGIN phase2]
                if let Some(rot) = payload.get("rotation").and_then(|v| v.as_array()) {
                    if rot.len() == 3 {
                        bone.rotation.x = rot[0].as_f64().unwrap_or(bone.rotation.x as f64) as f32;
                        bone.rotation.y = rot[1].as_f64().unwrap_or(bone.rotation.y as f64) as f32;
                        bone.rotation.z = rot[2].as_f64().unwrap_or(bone.rotation.z as f64) as f32;
                        bone.local_rotation = bone.rotation;
                    }
                }
                if let Some(scl) = payload.get("scale").and_then(|v| v.as_array()) {
                    if scl.len() == 3 {
                        bone.scale.x = scl[0].as_f64().unwrap_or(bone.scale.x as f64) as f32;
                        bone.scale.y = scl[1].as_f64().unwrap_or(bone.scale.y as f64) as f32;
                        bone.scale.z = scl[2].as_f64().unwrap_or(bone.scale.z as f64) as f32;
                        bone.local_scale = bone.scale;
                    }
                }
                // [wmb-studio END]
                format!("Update WMB3 Bone #{b_idx}")
            }
            "update_wmb3_batch" => {
                let bi = if let Some(bid_str) = payload
                    .get("batch_id")
                    .or_else(|| payload.get("id"))
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.trim().is_empty())
                {
                    working_uid_map
                        .wmb3_batch_index_by_id(bid_str)
                        .ok_or_else(|| format!("WMB3 batch '{bid_str}' not found"))?
                } else {
                    payload
                        .get("batch_index")
                        .and_then(|v| v.as_u64())
                        .ok_or("Missing batch_id or batch_index")? as usize
                };
                if let Some(vgi) = payload.get("vertex_group_index").and_then(|v| v.as_u64()) {
                    if (vgi as usize) >= wmb3.vertex_groups.len() {
                        return Err(format!("WMB3 vertex_group_index {vgi} out of bounds"));
                    }
                }
                if let Some(bsi) = payload.get("bone_set_index").and_then(|v| v.as_i64()) {
                    if bsi < -1 || (bsi >= 0 && (bsi as usize) >= wmb3.bone_sets.len()) {
                        return Err(format!(
                            "WMB3 bone_set_index {bsi} out of bounds (-1..{})",
                            wmb3.bone_sets.len()
                        ));
                    }
                }
                if let Some(m_id) = payload.get("material_index").and_then(|v| v.as_u64()) {
                    if (m_id as usize) >= wmb3.materials.len() {
                        return Err(format!("WMB3 material_index {m_id} out of bounds"));
                    }
                }
                if let Some(mg_id) = payload
                    .get("mesh_index")
                    .or_else(|| payload.get("mesh_group_index"))
                    .and_then(|v| v.as_u64())
                {
                    if (mg_id as usize) >= wmb3.meshes.len() {
                        return Err(format!("WMB3 mesh_index {mg_id} out of bounds"));
                    }
                }
                if let Some(cn) = payload.get("call_tree_node_index").and_then(|v| v.as_i64()) {
                    if cn < -1 || (cn >= 0 && !wmb3.col_tree_nodes.is_empty() && (cn as usize) >= wmb3.col_tree_nodes.len()) {
                        return Err(format!(
                            "WMB3 call_tree_node_index {cn} out of bounds (-1..{})",
                            wmb3.col_tree_nodes.len()
                        ));
                    }
                }
                let batch = wmb3
                    .batches
                    .get_mut(bi)
                    .ok_or("WMB3 batch index out of bounds")?;
                if let Some(vgi) = payload.get("vertex_group_index").and_then(|v| v.as_u64()) {
                    batch.vertex_group_index = vgi as u32;
                }
                if let Some(bsi) = payload.get("bone_set_index").and_then(|v| v.as_i64()) {
                    batch.bone_set_index = bsi as i32;
                }
                if let Some(vs) = payload.get("vertex_start").and_then(|v| v.as_u64()) {
                    batch.vertex_start = vs as u32;
                }
                if let Some(ist) = payload.get("index_start").and_then(|v| v.as_u64()) {
                    batch.index_start = ist as u32;
                }
                if let Some(nv) = payload.get("num_vertexes").and_then(|v| v.as_u64()) {
                    batch.num_vertexes = nv as u32;
                }
                if let Some(ni) = payload.get("num_indices").and_then(|v| v.as_u64()) {
                    batch.num_indices = ni as u32;
                }
                if let Some(np) = payload.get("num_primitives").and_then(|v| v.as_u64()) {
                    batch.num_primitives = np as u32;
                }

                let req_lod_idx = payload.get("lod_index").and_then(|v| v.as_u64()).map(|v| v as usize);
                let req_mat = payload.get("material_index").and_then(|v| v.as_u64()).map(|v| v as u32);
                let req_mesh = payload
                    .get("mesh_index")
                    .or_else(|| payload.get("mesh_group_index"))
                    .and_then(|v| v.as_u64())
                    .map(|v| v as u32);
                let req_lod_vgi = payload.get("lod_vertex_group_index").and_then(|v| v.as_u64()).map(|v| v as u32);
                let req_col_node = payload.get("call_tree_node_index").and_then(|v| v.as_i64()).map(|v| v as i32);
                let req_mesh_mat = payload.get("mesh_material_index").and_then(|v| v.as_u64()).map(|v| v as u32);
                let req_ub = payload.get("u_b").and_then(|v| v.as_i64()).map(|v| v as i32);

                for (li, lod) in wmb3.lods.iter_mut().enumerate() {
                    if let Some(target_li) = req_lod_idx {
                        if li != target_li {
                            continue;
                        }
                    }
                    let start = lod.batch_start as usize;
                    let end = start + lod.batch_infos.len();
                    if bi >= start && bi < end {
                        let inf = &mut lod.batch_infos[bi - start];
                        if let Some(vgi) = req_lod_vgi {
                            inf.vertex_group_index = vgi;
                        } else if let Some(vgi) = payload.get("vertex_group_index").and_then(|v| v.as_u64()) {
                            inf.vertex_group_index = vgi as u32;
                        }
                        if let Some(m_id) = req_mat {
                            inf.material_index = m_id;
                        }
                        if let Some(mg_id) = req_mesh {
                            inf.mesh_index = mg_id;
                        }
                        if let Some(cn) = req_col_node {
                            inf.call_tree_node_index = cn;
                        }
                        if let Some(ub) = req_ub {
                            inf.u_b = ub;
                        }
                        if let Some(explicit_mm) = req_mesh_mat {
                            inf.mesh_material_index = explicit_mm;
                        } else if req_mat.is_some() || req_mesh.is_some() {
                            let target_mesh_idx = inf.mesh_index as usize;
                            let target_mat_u16 = inf.material_index as u16;
                            if let Some(mesh_grp) = wmb3.meshes.get_mut(target_mesh_idx) {
                                let local_mat_pos = if let Some(pos) =
                                    mesh_grp.materials.iter().position(|&m| m == target_mat_u16)
                                {
                                    pos
                                } else {
                                    mesh_grp.materials.push(target_mat_u16);
                                    mesh_grp.materials.len() - 1
                                };
                                inf.mesh_material_index = local_mat_pos as u32;
                            }
                        }
                    }
                }
                format!("Update WMB3 Batch #{bi}")
            }
            "set_tpose" => {
                wmb3.set_tpose();
                "Bake WMB3 Skeleton & Vertices to T-Pose".to_string()
            }
            "recompute_relative_positions" => {
                let positions: Vec<pgtools_core::bone::Vec3> =
                    wmb3.bones.iter().map(|b| b.position).collect();
                for (i, b) in wmb3.bones.iter_mut().enumerate() {
                    if b.parent_index >= 0 && (b.parent_index as usize) < positions.len() {
                        let pp = positions[b.parent_index as usize];
                        b.local_position = pgtools_core::bone::Vec3::new(
                            positions[i].x - pp.x,
                            positions[i].y - pp.y,
                            positions[i].z - pp.z,
                        );
                    } else {
                        b.local_position = positions[i];
                    }
                }
                "Recompute WMB3 Relative Bone Positions".to_string()
            }
            // [wmb-studio BEGIN phase2]
            "update_header" => {
                if let Some(fl) = payload.get("flags").and_then(|v| v.as_i64()) {
                    if fl < i16::MIN as i64 || fl > u16::MAX as i64 {
                        return Err(format!("WMB3 header flags {fl} out of 16-bit range"));
                    }
                    wmb3.flags = fl as i16;
                } else if let Some(flags_s) = payload
                    .get("vertex_flags_hex")
                    .or_else(|| payload.get("flags_hex"))
                    .and_then(|v| v.as_str())
                {
                    let clean = flags_s.trim().trim_start_matches("0x").trim_start_matches("0X");
                    let vf = u16::from_str_radix(clean, 16).map_err(|_| {
                        format!(
                            "Invalid WMB3 header flags '{flags_s}': expected 16-bit hex value (0x0000..0xFFFF)"
                        )
                    })?;
                    wmb3.flags = vf as i16;
                }
                if let Some(bb) = payload.get("bounding_box").and_then(|v| v.as_array()) {
                    if bb.len() == 6 {
                        for i in 0..6 {
                            if let Some(fv) = bb[i].as_f64() {
                                wmb3.bounding_box[i] = fv as f32;
                            }
                        }
                    }
                }
                "Update WMB3 Header fields".to_string()
            }
            "set_material_texture" => {
                let idx = payload
                    .get("index")
                    .and_then(|v| v.as_u64())
                    .ok_or("Missing WMB3 material index")? as usize;
                let slot = payload
                    .get("slot")
                    .and_then(|v| v.as_u64())
                    .ok_or("Missing WMB3 texture slot index")? as usize;
                let tid = payload.get("texture_id").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                let mat = wmb3
                    .materials
                    .get_mut(idx)
                    .ok_or_else(|| format!("WMB3 Material index #{idx} out of bounds"))?;
                let entry = mat
                    .textures
                    .get_mut(slot)
                    .ok_or_else(|| format!("WMB3 Material #{idx} texture slot #{slot} out of bounds"))?;
                entry.1 = tid;
                format!("Set WMB3 Material #{idx} Slot #{slot} to 0x{tid:08X}")
            }
            "set_material_param" => {
                let idx = payload
                    .get("index")
                    .and_then(|v| v.as_u64())
                    .ok_or("Missing WMB3 material index")? as usize;
                let key = payload.get("key").and_then(|v| v.as_str()).unwrap_or("");
                let vals: Vec<f32> = payload
                    .get("values")
                    .and_then(|v| v.as_array())
                    .map(|arr| arr.iter().filter_map(|x| x.as_f64().map(|f| f as f32)).collect())
                    .unwrap_or_default();
                let mat = wmb3.materials.get_mut(idx).ok_or("WMB3 Material index out of bounds")?;
                if let Some(rest) = key.strip_prefix("var_") {
                    if let Some((vi_str, v_name)) = rest.split_once('_') {
                        if let (Ok(vi), Some(&v0)) = (vi_str.parse::<usize>(), vals.first()) {
                            if let Some(var_slot) = mat.variables.get_mut(vi) {
                                var_slot.1 = v0;
                            }
                            let nps = mat.named_parameters();
                            for np in nps {
                                if np.name == v_name {
                                    if let Some(grp) = mat.parameter_groups.get_mut(np.group_index) {
                                        if let Some(p_slot) = grp.1.get_mut(np.param_index) {
                                            *p_slot = v0;
                                        }
                                    }
                                }
                            }
                        }
                    }
                } else {
                    let parts: Vec<&str> = key.splitn(3, '_').collect();
                    if parts.len() >= 2 {
                        if let (Ok(gi), Ok(pi), Some(&v0)) = (parts[0].parse::<usize>(), parts[1].parse::<usize>(), vals.first()) {
                            if let Some(grp) = mat.parameter_groups.get_mut(gi) {
                                if let Some(p_slot) = grp.1.get_mut(pi) {
                                    *p_slot = v0;
                                }
                            }
                            if let Some(&param_name) = parts.get(2) {
                                for (vname, vval) in &mut mat.variables {
                                    if vname == param_name {
                                        *vval = v0;
                                    }
                                }
                            }
                        }
                    }
                }
                coalesce_key = Some(format!("param:{container_id}:{idx}:{key}"));
                format!("Update WMB3 Material #{idx} Param {key}")
            }
            "add_bone" => {
                let parent_gid = payload.get("parent_global_id").and_then(|v| v.as_i64()).unwrap_or(-1);
                let parent_idx = if parent_gid >= 0 {
                    wmb3.bones
                        .iter()
                        .position(|b| b.id as i64 == parent_gid)
                        .map(|i| i as i16)
                        .unwrap_or(-1)
                } else {
                    -1
                };
                let next_gid = wmb3.bones.iter().map(|b| b.id).max().unwrap_or(0).saturating_add(1);
                let parent_pos = if parent_idx >= 0 {
                    wmb3.bones.get(parent_idx as usize).map(|b| b.position).unwrap_or_default()
                } else {
                    pgtools_core::bone::Vec3::default()
                };
                let new_local_idx = wmb3.bones.len() as u16;
                wmb3.bones.push(pgtools_core::wmb3::Wmb3Bone {
                    id: next_gid,
                    parent_index: parent_idx,
                    local_position: pgtools_core::bone::Vec3::new(0.0, 0.1, 0.0),
                    local_rotation: pgtools_core::bone::Vec3::default(),
                    local_scale: pgtools_core::bone::Vec3::new(1.0, 1.0, 1.0),
                    position: pgtools_core::bone::Vec3::new(parent_pos.x, parent_pos.y + 0.1, parent_pos.z),
                    rotation: pgtools_core::bone::Vec3::default(),
                    scale: pgtools_core::bone::Vec3::new(1.0, 1.0, 1.0),
                    t_position: pgtools_core::bone::Vec3::new(parent_pos.x, parent_pos.y + 0.1, parent_pos.z),
                });
                if !wmb3.bone_index_translate_table.table.is_empty() && next_gid >= 0 {
                    wmb3.bone_index_translate_table
                        .table
                        .insert(next_gid as u16, new_local_idx);
                    wmb3.raw_btt.clear();
                }
                bump_structure = true;
                format!("Add WMB3 Bone (gid {next_gid})")
            }
            "delete_bone" => {
                if wmb3.bones.len() <= 1 {
                    return Err("Cannot delete the only remaining bone".into());
                }
                let gid_i64 = payload
                    .get("global_id")
                    .and_then(|v| v.as_i64())
                    .ok_or("Missing bone global_id")?;
                let gid = i16::try_from(gid_i64).map_err(|_| format!("Invalid bone global_id {gid_i64}"))?;
                let del_idx = wmb3
                    .bones
                    .iter()
                    .position(|b| b.id == gid)
                    .or_else(|| {
                        if gid >= 0 && (gid as usize) < wmb3.bones.len() {
                            Some(gid as usize)
                        } else {
                            None
                        }
                    })
                    .ok_or("WMB3 bone not found")?;

                // Check if any vertex in any batch is actively skinned to `del_idx` (#17)
                for b in &wmb3.batches {
                    let Some(vg) = wmb3.vertex_groups.get(b.vertex_group_index as usize) else {
                        continue;
                    };
                    let bone_refs: Vec<usize> = if b.bone_set_index >= 0
                        && (b.bone_set_index as usize) < wmb3.bone_sets.len()
                    {
                        wmb3.bone_sets[b.bone_set_index as usize]
                            .iter()
                            .map(|&bi| {
                                if !wmb3.bone_map.is_empty() {
                                    *wmb3.bone_map.get(bi as usize).unwrap_or(&0) as usize
                                } else {
                                    bi.max(0) as usize
                                }
                            })
                            .collect()
                    } else if !wmb3.bone_map.is_empty() {
                        wmb3.bone_map.iter().map(|&bi| bi as usize).collect()
                    } else {
                        (0..wmb3.bones.len()).collect()
                    };
                    if !bone_refs.contains(&del_idx) {
                        continue;
                    }
                    let start = (b.index_start as usize).min(vg.indices.len());
                    let end = (start + b.num_indices as usize).min(vg.indices.len());
                    for &raw_vi in &vg.indices[start..end] {
                        let vi = ((raw_vi + b.vertex_start) as usize).min(vg.vertexes.len().saturating_sub(1));
                        if let Some(v) = vg.vertexes.get(vi) {
                            if let Some(bi_info) = v.bone_infos {
                                for (local_bi, bw) in bi_info.active_pairs() {
                                    if bw > 0 && bone_refs.get(local_bi as usize) == Some(&del_idx) {
                                        return Err(format!(
                                            "Cannot delete bone #{del_idx} (gid {gid}) because it is referenced by skinned mesh vertices"
                                        ));
                                    }
                                }
                            }
                        }
                    }
                }

                let del_parent = wmb3.bones[del_idx].parent_index;
                let fallback_bone = if del_parent >= 0 {
                    if (del_parent as usize) > del_idx {
                        (del_parent - 1) as usize
                    } else {
                        del_parent as usize
                    }
                } else {
                    0usize
                };

                wmb3.bones.remove(del_idx);
                for b in &mut wmb3.bones {
                    if b.parent_index == del_idx as i16 {
                        b.parent_index = if del_parent > del_idx as i16 {
                            del_parent - 1
                        } else {
                            del_parent
                        };
                    } else if b.parent_index > del_idx as i16 {
                        b.parent_index -= 1;
                    }
                }

                // Remap bone_index_translate_table & clear stale raw_btt (#17)
                if gid >= 0 {
                    wmb3.bone_index_translate_table.table.remove(&(gid as u16));
                }
                wmb3.bone_index_translate_table
                    .table
                    .retain(|_, &mut l| (l as usize) != del_idx);
                for l in wmb3.bone_index_translate_table.table.values_mut() {
                    if (*l as usize) > del_idx {
                        *l -= 1;
                    }
                }
                wmb3.raw_btt.clear();

                // Remap bone_map, bone_sets, and meshes[].bone_indices (#17)
                if !wmb3.bone_map.is_empty() {
                    for bm in &mut wmb3.bone_map {
                        if (*bm as usize) == del_idx {
                            *bm = fallback_bone as u32;
                        } else if (*bm as usize) > del_idx {
                            *bm -= 1;
                        }
                    }
                } else {
                    for set in &mut wmb3.bone_sets {
                        for bi in set.iter_mut() {
                            if (*bi as usize) == del_idx {
                                *bi = fallback_bone as i16;
                            } else if (*bi as usize) > del_idx {
                                *bi -= 1;
                            }
                        }
                    }
                }
                for mesh in &mut wmb3.meshes {
                    for bi in &mut mesh.bone_indices {
                        if (*bi as usize) == del_idx {
                            *bi = fallback_bone as u16;
                        } else if (*bi as usize) > del_idx {
                            *bi -= 1;
                        }
                    }
                }

                bump_structure = true;
                format!("Delete WMB3 Bone (gid {gid})")
            }
            "reorder_materials" => {
                let from_idx = payload.get("from_index").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
                let to_idx = payload.get("to_index").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
                if from_idx >= wmb3.materials.len() || wmb3.materials.is_empty() {
                    return Err("WMB3 Material index out of bounds".into());
                }
                let moved_mat = wmb3.materials.remove(from_idx);
                let moved_uid = working_uid_map.material_uids.remove(from_idx);
                let ins = if to_idx > from_idx {
                    to_idx.saturating_sub(1)
                } else {
                    to_idx
                }
                .min(wmb3.materials.len());
                wmb3.materials.insert(ins, moved_mat);
                working_uid_map.material_uids.insert(ins, moved_uid);
                let remap_idx = |old: u32| -> u32 {
                    let o = old as usize;
                    if o == from_idx {
                        ins as u32
                    } else if from_idx < ins && o > from_idx && o <= ins {
                        (o - 1) as u32
                    } else if ins < from_idx && o >= ins && o < from_idx {
                        (o + 1) as u32
                    } else {
                        old
                    }
                };
                for lod in &mut wmb3.lods {
                    for inf in &mut lod.batch_infos {
                        inf.material_index = remap_idx(inf.material_index);
                    }
                }
                for pair in &mut wmb3.mesh_material_pairs {
                    pair.1 = remap_idx(pair.1);
                }
                for m in &mut wmb3.meshes {
                    for mat_ref in &mut m.materials {
                        *mat_ref = remap_idx(*mat_ref as u32) as u16;
                    }
                }
                bump_structure = true;
                format!("Reorder WMB3 Material #{from_idx}")
            }
            // [wmb-studio END]
            // [wmb-studio BEGIN phase1]
            "set_batches_material"
            | "move_items"
            | "delete_items"
            | "duplicate_items"
            | "reorder_mesh_groups"
            | "mesh_group_add"
            | "mesh_group_duplicate"
            | "mesh_group_delete"
            | "mesh_group_rename"
            | "update_wmb3_lod"
            | "material_add"
            | "material_duplicate"
            | "material_delete"
            | "material_rename"
            | "paste_items" => {
                let bank_tex_ids_opt = if operation == "paste_items" {
                    Some(wmb_scene::collect_paired_wtb_texture_ids(
                        &snap, &node_id, oodle_opt,
                    ))
                } else {
                    None
                };
                let outcome = apply_wmb3_scene_op(
                    &mut wmb3,
                    &mut working_uid_map,
                    &operation,
                    &payload,
                    clipboard_opt.as_ref(),
                    bank_tex_ids_opt.as_deref(),
                )?;
                if outcome.structural {
                    bump_structure = true;
                }
                outcome.description
            }
            // [wmb-studio END]
            other => return Err(format!("Unsupported WMB3 operation: {other}")),
        };
        // [wmb-studio BEGIN phase1]
        sync_wmb3_invariants(&mut wmb3);
        working_uid_map.ensure_wmb3(&wmb3);
        let new_wmb3_bytes = wmb3.to_bytes().map_err(|e| e.to_string())?;
        let _ = Wmb3File::from_bytes(&new_wmb3_bytes)
            .map_err(|e| format!("WMB3 round-trip verification failed: {e}"))?;
        // [wmb-studio END]
        let next_snap =
            write_back_subblob(&snap, &container_parts, new_wmb3_bytes, None, oodle_opt)?;

        sess.committed_uid_maps
            .entry(container_id.clone())
            .or_insert(init_committed);
        sess.uid_maps.insert(container_id, working_uid_map);
        if bump_structure {
            sess.structure_version = sess.structure_version.max(1) + 1;
        }
        record_mutation_with_key(&mut sess, desc, next_snap, coalesce_key);
        return build_outline_from_session(&sess);
    }

    let mut wmb = WmbFile::from_bytes(&sub_bytes).map_err(|e| e.to_string())?;
    let mut target_endian = wmb.endian;
    let is_b2 = wmb.is_bayo2();
    let mut init_committed = orig_uid_map;
    init_committed.ensure_wmb(&wmb);
    working_uid_map.ensure_wmb(&wmb);

    let desc = match operation.as_str() {
        "update_material" => {
            let decoded: DecodedWmbMaterial =
                serde_json::from_value(payload).map_err(|e| e.to_string())?;
            let mat = wmb
                .materials
                .get_mut(decoded.index)
                .ok_or("Material index out of bounds")?;
            mat.apply_decoded(&decoded);
            format!("Update Material #{}", decoded.index)
        }
        "switch_shader" => {
            let idx = payload
                .get("index")
                .and_then(|v| v.as_u64())
                .ok_or("Missing material index")? as usize;
            let target = payload
                .get("shader")
                .and_then(|v| v.as_str())
                .ok_or("Missing shader/type")?;
            let use_b2 = payload
                .get("is_bayo2")
                .and_then(|v| v.as_bool())
                .unwrap_or(is_b2);
            let mat = wmb
                .materials
                .get_mut(idx)
                .ok_or("Material index out of bounds")?;
            let _ = switch_wmb_material_shader(mat, idx, use_b2, target);
            format!("Switch Material #{idx} to {target}")
        }
        "update_header" => {
            if payload.get("bounding_box").is_some() && payload.get("vertex_flags_hex").is_none() {
                return Err(
                    "WMB0-2 headers do not store a model-level bounding box (computed from mesh vertices)"
                        .into(),
                );
            }
            if let Some(flags_s) = payload.get("vertex_flags_hex").and_then(|v| v.as_str()) {
                let clean = flags_s.trim().trim_start_matches("0x").trim_start_matches("0X");
                let vf = u32::from_str_radix(clean, 16)
                    .map_err(|_| format!("Invalid hex vertex_flags '{flags_s}'"))?;
                let force = payload
                    .get("force")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                if !force
                    && lookup_wmb_vertex_layout(
                        vf,
                        wmb.header.vertex_ex_data_size,
                        wmb.header.vertex_ex_data,
                    )
                    .is_none()
                {
                    return Err(format!(
                        "Unsupported WMB vertex_flags 0x{vf:08X} for ex_data_size={} ex_data={}",
                        wmb.header.vertex_ex_data_size, wmb.header.vertex_ex_data
                    ));
                }
                wmb.header.vertex_flags = vf;
            }
            "Update WMB Header fields".to_string()
        }
        "update_bone" => {
            let b_idx = payload
                .get("index")
                .and_then(|v| v.as_u64())
                .ok_or("Missing bone index")? as usize;
            if b_idx >= wmb.bone_hierarchy.len() {
                return Err("Bone index out of bounds".into());
            }
            if let Some(parent) = payload.get("parent_index").and_then(|v| v.as_i64()) {
                let valid_p = validate_bone_parent(
                    wmb.bone_hierarchy.len(),
                    b_idx,
                    parent,
                    |i| wmb.bone_hierarchy[i],
                )?;
                wmb.bone_hierarchy[b_idx] = valid_p;
                bump_structure = true;
            }
            let parent_idx = wmb.bone_hierarchy[b_idx];
            let parent_world = if parent_idx >= 0 && (parent_idx as usize) < wmb.bone_positions.len() {
                Some(wmb.bone_positions[parent_idx as usize])
            } else {
                None
            };
            if payload.get("parent_index").is_some()
                && payload.get("position").is_none()
                && payload.get("relative_position").is_none()
            {
                if let (Some(&world_p), Some(rel_slot)) = (
                    wmb.bone_positions.get(b_idx),
                    wmb.bone_relative_positions.get_mut(b_idx),
                ) {
                    if let Some(pp) = parent_world {
                        rel_slot.x = world_p.x - pp.x;
                        rel_slot.y = world_p.y - pp.y;
                        rel_slot.z = world_p.z - pp.z;
                    } else {
                        *rel_slot = world_p;
                    }
                }
            }
            if let Some(pos) = payload.get("position").and_then(|v| v.as_array()) {
                if pos.len() == 3 {
                    if let Some(slot) = wmb.bone_positions.get_mut(b_idx) {
                        slot.x = pos[0].as_f64().unwrap_or(slot.x as f64) as f32;
                        slot.y = pos[1].as_f64().unwrap_or(slot.y as f64) as f32;
                        slot.z = pos[2].as_f64().unwrap_or(slot.z as f64) as f32;
                    }
                    if payload.get("relative_position").is_none() {
                        if let (Some(&world_p), Some(rel_slot)) = (
                            wmb.bone_positions.get(b_idx),
                            wmb.bone_relative_positions.get_mut(b_idx),
                        ) {
                            if let Some(pp) = parent_world {
                                rel_slot.x = world_p.x - pp.x;
                                rel_slot.y = world_p.y - pp.y;
                                rel_slot.z = world_p.z - pp.z;
                            } else {
                                *rel_slot = world_p;
                            }
                        }
                    }
                }
            }
            if let Some(rel) = payload.get("relative_position").and_then(|v| v.as_array()) {
                if rel.len() == 3 {
                    if let Some(slot) = wmb.bone_relative_positions.get_mut(b_idx) {
                        slot.x = rel[0].as_f64().unwrap_or(slot.x as f64) as f32;
                        slot.y = rel[1].as_f64().unwrap_or(slot.y as f64) as f32;
                        slot.z = rel[2].as_f64().unwrap_or(slot.z as f64) as f32;
                    }
                    if payload.get("position").is_none() {
                        if let (Some(&rel_p), Some(world_slot)) = (
                            wmb.bone_relative_positions.get(b_idx),
                            wmb.bone_positions.get_mut(b_idx),
                        ) {
                            if let Some(pp) = parent_world {
                                world_slot.x = pp.x + rel_p.x;
                                world_slot.y = pp.y + rel_p.y;
                                world_slot.z = pp.z + rel_p.z;
                            } else {
                                *world_slot = rel_p;
                            }
                        }
                    }
                }
            }
            format!("Update Bone #{b_idx}")
        }
        "update_batch" => {
            let (mesh_idx, batch_idx) = if let Some(bid_str) = payload
                .get("batch_item_id")
                .or_else(|| payload.get("id"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
            {
                working_uid_map
                    .wmb_batch_pos_by_id(bid_str)
                    .ok_or_else(|| format!("WMB batch '{bid_str}' not found"))?
            } else if let Some(bid_str) = payload
                .get("batch_id")
                .and_then(|v| v.as_str())
                .filter(|s| s.starts_with("batch:"))
            {
                working_uid_map
                    .wmb_batch_pos_by_id(bid_str)
                    .ok_or_else(|| format!("WMB batch '{bid_str}' not found"))?
            } else {
                let mi = payload
                    .get("mesh_index")
                    .and_then(|v| v.as_u64())
                    .ok_or("Missing mesh_index")? as usize;
                let bi = payload
                    .get("batch_index")
                    .and_then(|v| v.as_u64())
                    .ok_or("Missing batch_index")? as usize;
                (mi, bi)
            };
            if let Some(ue1) = payload.get("u_e1").and_then(|v| v.as_i64()) {
                if !(0..=255).contains(&ue1) {
                    return Err(format!("u_e1 value {ue1} out of u8 range (0..=255)"));
                }
            }
            if let Some(ue2) = payload.get("u_e2").and_then(|v| v.as_i64()) {
                if !(0..=255).contains(&ue2) {
                    return Err(format!("u_e2 value {ue2} out of u8 range (0..=255)"));
                }
            }
            if let Some(prim) = payload.get("primitive_type").and_then(|v| v.as_i64()) {
                if prim != 4 && prim != 5 {
                    return Err(format!(
                        "Invalid WMB primitive_type {prim}: expected 4 (Triangle List) or 5 (Triangle Strip)"
                    ));
                }
            }
            if let Some(fl_hex) = payload.get("flags_hex").and_then(|v| v.as_str()) {
                let clean = fl_hex.trim().trim_start_matches("0x").trim_start_matches("0X");
                if u16::from_str_radix(clean, 16).is_err() {
                    return Err(format!(
                        "Invalid batch flags_hex '{fl_hex}': expected 16-bit hex (0x0000..0xFFFF)"
                    ));
                }
            }
            let max_bones = wmb.bone_hierarchy.len();
            if let Some(br_arr) = payload.get("bone_refs").and_then(|v| v.as_array()) {
                for item in br_arr {
                    let Some(val) = item.as_i64() else {
                        return Err("Invalid bone_refs entry: expected integer".into());
                    };
                    if val < 0 || val > 255 {
                        return Err(format!("bone_refs entry {val} out of u8 range (0..=255)"));
                    }
                    if max_bones > 0 && (val as usize) >= max_bones {
                        return Err(format!(
                            "bone_refs entry {val} exceeds skeleton bone count ({max_bones})"
                        ));
                    }
                }
            }
            let mesh = wmb.meshes.get_mut(mesh_idx).ok_or("Mesh index out of bounds")?;
            let batch = mesh
                .batches
                .get_mut(batch_idx)
                .ok_or("Batch index out of bounds")?;

            // Effective material selection (from Material Picker popover)
            if let Some(mat_id) = payload.get("material_id").and_then(|v| v.as_u64()) {
                if is_b2 {
                    batch.header.ex_mat_id = mat_id as i16;
                } else {
                    batch.header.material_id = mat_id as u8;
                }
            }
            // Explicit raw header fields (from Batch Header editor)
            if let Some(raw_mat) = payload.get("raw_material_id").and_then(|v| v.as_u64()) {
                batch.header.material_id = raw_mat as u8;
            }
            if let Some(ex_mat) = payload.get("ex_mat_id").and_then(|v| v.as_i64()) {
                batch.header.ex_mat_id = ex_mat as i16;
            }
            if let Some(bid) = payload.get("batch_id").and_then(|v| v.as_i64()) {
                batch.header.batch_id = bid as i16;
            }
            if let Some(mid) = payload.get("mesh_id").and_then(|v| v.as_i64()) {
                batch.header.mesh_id = mid as i16;
            }
            if let Some(fl) = payload.get("flags").and_then(|v| v.as_u64()) {
                batch.header.flags = fl as u16;
            }
            if let Some(fl_hex) = payload.get("flags_hex").and_then(|v| v.as_str()) {
                let clean = fl_hex.trim().trim_start_matches("0x").trim_start_matches("0X");
                if let Ok(parsed) = u16::from_str_radix(clean, 16) {
                    batch.header.flags = parsed;
                }
            }
            if let Some(hbr) = payload.get("has_bone_refs").and_then(|v| v.as_u64()) {
                batch.header.has_bone_refs = hbr as u8;
            }
            if let Some(ue1) = payload.get("u_e1").and_then(|v| v.as_u64()) {
                batch.header.u_e1 = ue1 as u8;
            }
            if let Some(ue2) = payload.get("u_e2").and_then(|v| v.as_u64()) {
                batch.header.u_e2 = ue2 as u8;
            }
            if let Some(sh_proxy) = payload.get("shadow_proxy").and_then(|v| v.as_bool()) {
                batch_set_shadow_proxy(&mut batch.header, is_b2, sh_proxy);
            }
            if let Some(vs) = payload.get("vertex_start").and_then(|v| v.as_u64()) {
                batch.header.vertex_start = vs as u32;
            }
            if let Some(ve) = payload.get("vertex_end").and_then(|v| v.as_u64()) {
                batch.header.vertex_end = ve as u32;
            }
            if let Some(prim) = payload.get("primitive_type").and_then(|v| v.as_i64()) {
                batch.header.primitive_type = prim as i32;
            }
            if let Some(oi) = payload.get("offset_indices").and_then(|v| v.as_u64()) {
                batch.header.offset_indices = oi as u32;
            }
            if let Some(ni) = payload.get("num_indices").and_then(|v| v.as_i64()) {
                batch.header.num_indices = ni as i32;
            }
            if let Some(vo) = payload.get("vertex_offset").and_then(|v| v.as_i64()) {
                batch.header.vertex_offset = vo as i32;
            }
            if let Some(uf_arr) = payload.get("u_f").and_then(|v| v.as_array()) {
                for (i, item) in uf_arr.iter().take(7).enumerate() {
                    if let Some(val) = item.as_i64() {
                        batch.header.u_f[i] = val as i32;
                    }
                }
            }
            if let Some(br_arr) = payload.get("bone_refs").and_then(|v| v.as_array()) {
                batch.bone_refs = br_arr
                    .iter()
                    .filter_map(|v| v.as_u64().map(|n| n as u8))
                    .collect();
                batch.header.has_bone_refs = if batch.bone_refs.is_empty() { 0 } else { 1 };
            }
            if let Some(unb_arr) = payload.get("unknown_no_bones").and_then(|v| v.as_array()) {
                for (i, item) in unb_arr.iter().take(4).enumerate() {
                    if let Some(val) = item.as_f64() {
                        batch.unknown_no_bones[i] = val as f32;
                    }
                }
            }
            batch.header.offset_indices = batch.effective_indices_offset();
            format!("Update Mesh #{mesh_idx} Batch #{batch_idx}")
        }
        "toggle_wmb_mesh_shadow_proxy" => {
            let mesh_idx = payload
                .get("mesh_index")
                .and_then(|v| v.as_u64())
                .ok_or("Missing mesh_index")? as usize;
            let mesh = wmb.meshes.get_mut(mesh_idx).ok_or("Mesh index out of bounds")?;
            let target_enabled = payload
                .get("enabled")
                .and_then(|v| v.as_bool())
                .unwrap_or_else(|| !mesh.batches.iter().all(|b| batch_is_shadow_proxy(&b.header, is_b2)));
            for b in &mut mesh.batches {
                batch_set_shadow_proxy(&mut b.header, is_b2, target_enabled);
            }
            bump_structure = true;
            format!(
                "{} Shadow Proxy on Mesh #{mesh_idx} ({})",
                if target_enabled { "Enable" } else { "Disable" },
                mesh.name
            )
        }
        "move_wmb_batch" => {
            let from_mesh = payload
                .get("from_mesh_index")
                .and_then(|v| v.as_u64())
                .ok_or("Missing from_mesh_index")? as usize;
            let from_batch = payload
                .get("from_batch_index")
                .and_then(|v| v.as_u64())
                .ok_or("Missing from_batch_index")? as usize;
            let to_mesh = payload
                .get("to_mesh_index")
                .and_then(|v| v.as_u64())
                .ok_or("Missing to_mesh_index")? as usize;
            let to_batch_opt = payload
                .get("to_batch_index")
                .and_then(|v| v.as_u64())
                .map(|v| v as usize);

            if from_mesh >= wmb.meshes.len() || to_mesh >= wmb.meshes.len() {
                return Err("Mesh index out of bounds".into());
            }
            if from_batch >= wmb.meshes[from_mesh].batches.len() {
                return Err("Batch index out of bounds".into());
            }
            let mut moved = wmb.meshes[from_mesh].batches.remove(from_batch);
            let moved_uid = working_uid_map.wmb_batch_uids[from_mesh].remove(from_batch);
            moved.header.mesh_id = to_mesh as i16;
            let dst_batches = &mut wmb.meshes[to_mesh].batches;
            let ins_pos = to_batch_opt.unwrap_or(dst_batches.len()).min(dst_batches.len());
            dst_batches.insert(ins_pos, moved);
            working_uid_map.wmb_batch_uids[to_mesh].insert(ins_pos, moved_uid);

            for (mi, m) in wmb.meshes.iter_mut().enumerate() {
                for (bi, b) in m.batches.iter_mut().enumerate() {
                    b.header.mesh_id = mi as i16;
                    b.header.batch_id = bi as i16;
                }
            }
            bump_structure = true;
            format!("Move Batch #{from_batch} from Mesh #{from_mesh} to Mesh #{to_mesh}")
        }
        "duplicate_wmb_batch" => {
            let mesh_idx = payload
                .get("mesh_index")
                .and_then(|v| v.as_u64())
                .ok_or("Missing mesh_index")? as usize;
            let batch_idx = payload
                .get("batch_index")
                .and_then(|v| v.as_u64())
                .ok_or("Missing batch_index")? as usize;
            let mesh = wmb.meshes.get_mut(mesh_idx).ok_or("Mesh index out of bounds")?;
            let cloned = mesh
                .batches
                .get(batch_idx)
                .ok_or("Batch index out of bounds")?
                .clone();
            mesh.batches.insert(batch_idx + 1, cloned);
            let new_uid = working_uid_map.alloc();
            working_uid_map.wmb_batch_uids[mesh_idx].insert(batch_idx + 1, new_uid);
            for (bi, b) in mesh.batches.iter_mut().enumerate() {
                b.header.batch_id = bi as i16;
            }
            bump_structure = true;
            format!("Duplicate Mesh #{mesh_idx} Batch #{batch_idx}")
        }
        "delete_wmb_batch" => {
            let mesh_idx = payload
                .get("mesh_index")
                .and_then(|v| v.as_u64())
                .ok_or("Missing mesh_index")? as usize;
            let batch_idx = payload
                .get("batch_index")
                .and_then(|v| v.as_u64())
                .ok_or("Missing batch_index")? as usize;
            let mesh = wmb.meshes.get_mut(mesh_idx).ok_or("Mesh index out of bounds")?;
            if mesh.batches.len() <= 1 {
                return Err("Cannot delete the only remaining batch in a mesh".into());
            }
            if batch_idx >= mesh.batches.len() {
                return Err("Batch index out of bounds".into());
            }
            mesh.batches.remove(batch_idx);
            working_uid_map.wmb_batch_uids[mesh_idx].remove(batch_idx);
            for (bi, b) in mesh.batches.iter_mut().enumerate() {
                b.header.batch_id = bi as i16;
            }
            bump_structure = true;
            format!("Delete Mesh #{mesh_idx} Batch #{batch_idx}")
        }
        "cleanup_vertexes" => {
            wmb.cleanup_vertexes();
            bump_structure = true;
            "WMB Cleanup Unreferenced Vertexes".to_string()
        }
        "cleanup_materials" => {
            wmb.cleanup_materials();
            bump_structure = true;
            "WMB Cleanup Unused Materials".to_string()
        }
        "cleanup_material_sizes" => {
            wmb.cleanup_material_sizes().map_err(|e| e.to_string())?;
            "WMB Clamp Material Sizes to Shader Spec".to_string()
        }
        "maximize_material_sizes" => {
            wmb.maximize_material_sizes().map_err(|e| e.to_string())?;
            "WMB Maximize Material Sizes".to_string()
        }
        "remove_triangle_strips" => {
            wmb.remove_triangle_strips();
            "WMB Convert Triangle Strips to Triangles".to_string()
        }
        "normals_to_wide" => {
            wmb.normals_to_wide();
            "WMB Convert Normals to 10-10-10-2 Wide".to_string()
        }
        "check_normals" => {
            wmb.check_normals();
            "WMB Normalize & Check Vertex Normals".to_string()
        }
        "recompute_relative_positions" => {
            wmb.recompute_relative_positions();
            "WMB Recompute Relative Bone Positions".to_string()
        }
        "scale" => {
            let s = payload.get("scale").and_then(|v| v.as_f64()).unwrap_or(1.0) as f32;
            wmb.scale(s);
            format!("WMB Scale Model by {s}")
        }
        "swap_endian" => {
            target_endian = wmb.endian.swap();
            format!("WMB Convert Endianness to {target_endian:?}")
        }
        "export_obj" => {
            let out_path = payload
                .get("path")
                .and_then(|v| v.as_str())
                .ok_or("Missing OBJ export path")?;
            fs::write(out_path, wmb.export_obj()).map_err(|e| e.to_string())?;
            return build_outline_from_session(&sess);
        }
        // [wmb-studio BEGIN phase2]
        "set_material_texture" => {
            let idx = payload
                .get("index")
                .and_then(|v| v.as_u64())
                .ok_or("Missing material index")? as usize;
            let slot = payload
                .get("slot")
                .and_then(|v| v.as_u64())
                .ok_or("Missing texture slot index")? as usize;
            let tid = payload.get("texture_id").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
            let mat = wmb
                .materials
                .get_mut(idx)
                .ok_or_else(|| format!("Material index #{idx} out of bounds"))?;
            let mut dec = decode_wmb_material_rich(mat, idx, is_b2);
            let mut cur_slot = 0usize;
            let mut found = false;
            for f in &mut dec.fields {
                if f.sampler_tex_id.is_some() {
                    if cur_slot == slot {
                        f.sampler_tex_id = Some(tid);
                        found = true;
                        break;
                    }
                    cur_slot += 1;
                }
            }
            if !found {
                return Err(format!("Material #{idx} texture slot #{slot} out of bounds"));
            }
            mat.apply_decoded(&dec);
            format!("Set Material #{idx} Slot #{slot} to 0x{tid:08X}")
        }
        "set_material_param" => {
            let idx = payload
                .get("index")
                .and_then(|v| v.as_u64())
                .ok_or("Missing material index")? as usize;
            let key = payload.get("key").and_then(|v| v.as_str()).unwrap_or("");
            let vals: Vec<f32> = payload
                .get("values")
                .and_then(|v| v.as_array())
                .map(|arr| arr.iter().filter_map(|x| x.as_f64().map(|f| f as f32)).collect())
                .unwrap_or_default();
            let mat = wmb.materials.get_mut(idx).ok_or("Material index out of bounds")?;
            let mut dec = decode_wmb_material_rich(mat, idx, is_b2);
            for f in &mut dec.fields {
                if f.sampler_tex_id.is_none() && f.name.eq_ignore_ascii_case(key) {
                    let mut flts = f.float_values.unwrap_or([0.0; 4]);
                    for (k, &v) in vals.iter().take(4).enumerate() {
                        flts[k] = v;
                    }
                    f.float_values = Some(flts);
                    break;
                }
            }
            mat.apply_decoded(&dec);
            coalesce_key = Some(format!("param:{container_id}:{idx}:{key}"));
            format!("Update Material #{idx} Param {key}")
        }
        "add_bone" => {
            let parent_gid = payload.get("parent_global_id").and_then(|v| v.as_i64()).unwrap_or(-1) as i32;
            let inv = wmb.bone_index_translate_table.invert();
            let parent_idx = if parent_gid >= 0 {
                wmb.bone_index_translate_table
                    .table
                    .get(&(parent_gid as u16))
                    .copied()
                    .map(|l| l as i16)
                    .or_else(|| {
                        if (parent_gid as usize) < wmb.bone_hierarchy.len() {
                            Some(parent_gid as i16)
                        } else {
                            None
                        }
                    })
                    .unwrap_or(-1)
            } else {
                -1
            };
            let new_local = wmb.bone_hierarchy.len() as u16;
            let next_gid = inv.values().copied().max().unwrap_or(new_local).max(new_local) + 1;
            let parent_pos = if parent_idx >= 0 {
                wmb.bone_positions.get(parent_idx as usize).copied().unwrap_or_default()
            } else {
                pgtools_core::bone::Vec3::default()
            };
            wmb.bone_hierarchy.push(parent_idx);
            wmb.bone_relative_positions.push(pgtools_core::bone::Vec3::new(0.0, 0.1, 0.0));
            wmb.bone_positions.push(pgtools_core::bone::Vec3::new(parent_pos.x, parent_pos.y + 0.1, parent_pos.z));
            if !wmb.bone_symmetries.is_empty() {
                wmb.bone_symmetries.push(-1);
            }
            if !wmb.bone_flags.is_empty() {
                wmb.bone_flags.push(0);
            }
            wmb.bone_index_translate_table.table.insert(next_gid, new_local);
            wmb.header.num_bones = wmb.bone_hierarchy.len() as i32;
            bump_structure = true;
            format!("Add Bone #{new_local} (gid {next_gid})")
        }
        "delete_bone" => {
            if wmb.bone_hierarchy.len() <= 1 {
                return Err("Cannot delete the only remaining bone".into());
            }
            let gid = payload.get("global_id").and_then(|v| v.as_i64()).unwrap_or(-1) as i32;
            let del_idx = wmb
                .bone_index_translate_table
                .table
                .get(&(gid as u16))
                .copied()
                .map(|l| l as usize)
                .or_else(|| {
                    if gid >= 0 && (gid as usize) < wmb.bone_hierarchy.len() {
                        Some(gid as usize)
                    } else {
                        None
                    }
                })
                .ok_or("Bone not found")?;
            let del_parent = wmb.bone_hierarchy[del_idx];
            wmb.bone_hierarchy.remove(del_idx);
            if del_idx < wmb.bone_relative_positions.len() {
                wmb.bone_relative_positions.remove(del_idx);
            }
            if del_idx < wmb.bone_positions.len() {
                wmb.bone_positions.remove(del_idx);
            }
            if del_idx < wmb.bone_symmetries.len() {
                wmb.bone_symmetries.remove(del_idx);
            }
            if del_idx < wmb.bone_flags.len() {
                wmb.bone_flags.remove(del_idx);
            }
            for p in &mut wmb.bone_hierarchy {
                if *p == del_idx as i16 {
                    *p = if del_parent > del_idx as i16 {
                        del_parent - 1
                    } else {
                        del_parent
                    };
                } else if *p > del_idx as i16 {
                    *p -= 1;
                }
            }
            wmb.bone_index_translate_table.table.retain(|_, &mut l| (l as usize) != del_idx);
            for l in wmb.bone_index_translate_table.table.values_mut() {
                if (*l as usize) > del_idx {
                    *l -= 1;
                }
            }
            let fallback_u8 = del_parent.max(0) as u8;
            for m in &mut wmb.meshes {
                for b in &mut m.batches {
                    for br in &mut b.bone_refs {
                        if (*br as usize) == del_idx {
                            *br = if (fallback_u8 as usize) > del_idx {
                                fallback_u8.saturating_sub(1)
                            } else {
                                fallback_u8
                            };
                        } else if (*br as usize) > del_idx {
                            *br -= 1;
                        }
                    }
                }
            }
            wmb.header.num_bones = wmb.bone_hierarchy.len() as i32;
            bump_structure = true;
            format!("Delete Bone #{del_idx}")
        }
        "reorder_materials" => {
            let from_idx = payload.get("from_index").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
            let to_idx = payload.get("to_index").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
            if from_idx >= wmb.materials.len() || wmb.materials.is_empty() {
                return Err("Material index out of bounds".into());
            }
            let moved_mat = wmb.materials.remove(from_idx);
            let moved_uid = working_uid_map.material_uids.remove(from_idx);
            let ins = if to_idx > from_idx {
                to_idx.saturating_sub(1)
            } else {
                to_idx
            }
            .min(wmb.materials.len());
            wmb.materials.insert(ins, moved_mat);
            working_uid_map.material_uids.insert(ins, moved_uid);
            let remap_idx = |old: usize| -> usize {
                if old == from_idx {
                    ins
                } else if from_idx < ins && old > from_idx && old <= ins {
                    old - 1
                } else if ins < from_idx && old >= ins && old < from_idx {
                    old + 1
                } else {
                    old
                }
            };
            for m in &mut wmb.meshes {
                for b in &mut m.batches {
                    let cur = b.header.effective_material_id(is_b2);
                    let next_m = remap_idx(cur);
                    if is_b2 {
                        b.header.ex_mat_id = next_m as i16;
                    } else {
                        b.header.material_id = next_m as u8;
                    }
                }
            }
            bump_structure = true;
            format!("Reorder Material #{from_idx}")
        }
        // [wmb-studio END]
        // [wmb-studio BEGIN phase1]
        "set_batches_material"
        | "move_items"
        | "delete_items"
        | "duplicate_items"
        | "reorder_mesh_groups"
        | "mesh_group_add"
        | "mesh_group_duplicate"
        | "mesh_group_delete"
        | "mesh_group_rename"
        | "material_add"
        | "material_duplicate"
        | "material_delete"
        | "material_rename"
        | "paste_items" => {
            let bank_tex_ids_opt = if operation == "paste_items" {
                Some(wmb_scene::collect_paired_wtb_texture_ids(
                    &snap, &node_id, oodle_opt,
                ))
            } else {
                None
            };
            let outcome = apply_wmb_scene_op(
                &mut wmb,
                &mut working_uid_map,
                &operation,
                &payload,
                clipboard_opt.as_ref(),
                bank_tex_ids_opt.as_deref(),
            )?;
            if outcome.structural {
                bump_structure = true;
            }
            outcome.description
        }
        // [wmb-studio END]
        other => return Err(format!("Unknown WMB operation: {other}")),
    };

    // [wmb-studio BEGIN phase1]
    enforce_shadow_last(&mut wmb, &mut working_uid_map);
    let new_wmb_bytes = wmb.to_bytes(target_endian).map_err(|e| e.to_string())?;
    let _ = WmbFile::from_bytes(&new_wmb_bytes)
        .map_err(|e| format!("WMB round-trip verification failed: {e}"))?;
    // [wmb-studio END]
    let next_snap = write_back_subblob(&snap, &container_parts, new_wmb_bytes, None, oodle_opt)?;

    sess.committed_uid_maps
        .entry(container_id.clone())
        .or_insert(init_committed);
    sess.uid_maps.insert(container_id, working_uid_map);
    if bump_structure {
        sess.structure_version = sess.structure_version.max(1) + 1;
    }
    record_mutation_with_key(&mut sess, desc, next_snap, coalesce_key);
    build_outline_from_session(&sess)
}

#[tauri::command]
fn mutate_bxm_node(
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
    let new_bytes = bxm.to_bytes(Endian::Big);
    let next_snap = write_back_subblob(&snap, &container_parts, new_bytes, None, oodle_opt)?;
    record_mutation(&mut sess, "Edit BXM XML Tree", next_snap);
    build_outline_from_session(&sess)
}

#[tauri::command]
fn mutate_struct_node(
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
            let headers: Vec<String> = updated_data
                .get("headers")
                .and_then(|v| serde_json::from_value(v.clone()).ok())
                .unwrap_or_default();
            let rows: Vec<Vec<String>> = updated_data
                .get("rows")
                .and_then(|v| serde_json::from_value(v.clone()).ok())
                .unwrap_or_default();
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
            (cos.to_bytes(), "Update COS Costume Colors".to_string())
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
                obi.apply_to_bytes(&sub_bytes),
                "Update OBI Object Placement".to_string(),
            )
        }
        "MSQ" => {
            let msq: MsqFile = serde_json::from_value(updated_data).map_err(|e| e.to_string())?;
            (
                msq.apply_to_bytes(&sub_bytes),
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

    let next_snap = write_back_subblob(&snap, &container_parts, new_bytes, None, oodle_opt)?;
    record_mutation(&mut sess, desc, next_snap);
    build_outline_from_session(&sess)
}

#[tauri::command]
fn mutate_cloth_node(
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

    let next_snap = write_back_subblob(&snap, &container_parts, new_bytes, None, oodle_opt)?;
    record_mutation(&mut sess, desc, next_snap);
    build_outline_from_session(&sess)
}

#[tauri::command]
fn mutate_container_node(
    node_id: String,
    action: String,
    target_path: String,
    payload: Option<serde_json::Value>,
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
    let payload_val = payload.unwrap_or(serde_json::Value::Null);

    match action.as_str() {
        "export_bxm_xml" => {
            let (_, sub_bytes, _, _) = resolve_subblob(&snap, &node_id, oodle_opt)?;
            let bxm = BxmFile::from_bytes(&sub_bytes).map_err(|e| e.to_string())?;
            fs::write(&target_path, bxm.to_xml_string()).map_err(|e| e.to_string())?;
            build_outline_from_session(&sess)
        }
        "import_bxm_xml" => {
            let (container_parts, _) = split_container_and_leaf(&node_id);
            let container_id = container_parts.join("/");
            let (_, sub_bytes, _, _) = resolve_subblob(&snap, &container_id, oodle_opt)?;
            let (orig_tag, orig_flags) = BxmFile::from_bytes(&sub_bytes)
                .map(|b| (b.tag, b.flags))
                .unwrap_or_else(|_| ("BXM".to_string(), 0));
            let xml_str = fs::read_to_string(&target_path).map_err(|e| e.to_string())?;
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
            record_mutation(&mut sess, desc, next_snap);
            build_outline_from_session(&sess)
        }
        _ => Err(format!("Unsupported container node action: {action}")),
    }
}

fn compute_reorder_target(
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

#[tauri::command]
fn run_file_context_operation(
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
            let (dat, orig_ext) = DatFile::create_from_dir(&p).map_err(|e| e.to_string())?;
            let out_file = dest_path
                .map(PathBuf::from)
                .unwrap_or_else(|| parent.join(format!("{stem}{orig_ext}")));
            fs::write(&out_file, dat.to_bytes()).map_err(|e| e.to_string())?;
            Ok(format!("Repacked folder into {}", out_file.display()))
        }
        "repack_folder_pkz" => {
            let mut files = Vec::new();
            for item in fs::read_dir(&p).map_err(|e| e.to_string())?.flatten() {
                if item.path().is_file() {
                    let name = item.file_name().to_string_lossy().into_owned();
                    let bytes = fs::read(item.path()).map_err(|e| e.to_string())?;
                    files.push((name, bytes));
                }
            }
            files.sort_by(|a, b| a.0.cmp(&b.0));
            let pkz_bytes = PkzArchive::build(&files, true).map_err(|e| e.to_string())?;
            let out_file = dest_path
                .map(PathBuf::from)
                .unwrap_or_else(|| parent.join(format!("{stem}.pkz")));
            fs::write(&out_file, pkz_bytes).map_err(|e| e.to_string())?;
            Ok(format!(
                "Packed {} files into {}",
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
                _ => return Err(format!("Endian swap not supported for .{ext}")),
            };
            let out_file = parent.join(format!("{stem}_swapped.{ext}"));
            fs::write(&out_file, out_bytes).map_err(|e| e.to_string())?;
            Ok(format!("Swapped endianness to {}", out_file.display()))
        }
        other => Err(format!("Unknown file operation: {other}")),
    }
}


fn ensure_not_subpath(src: &Path, dst: &Path) -> Result<(), String> {
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

fn unique_dest_path(dir: &Path, file_name: &str) -> PathBuf {
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
fn fs_batch_operation(
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
                let src_p = Path::new(sp);
                if src_p.is_dir() {
                    fs::remove_dir_all(src_p).map_err(|e| e.to_string())?;
                } else if src_p.exists() {
                    fs::remove_file(src_p).map_err(|e| e.to_string())?;
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

// [webview-data BEGIN]
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
// [webview-data END]

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
            get_studio_settings,
            save_studio_settings,
            pick_file_dialog,
            pick_multiple_files_dialog,
            save_file_dialog,
            pick_folder_dialog,
            reveal_in_explorer,
            drag_out_asset_node,
            list_directory,
            get_recent_files,
            open_document,
            save_document,
            undo_document,
            redo_document,
            inspect_asset_node,
            mutate_texture_node,
            export_texture_action,
            convert_material_action,
            mutate_wmb_node,
            mutate_bxm_node,
            mutate_struct_node,
            mutate_cloth_node,
            mutate_container_node,
            run_file_context_operation,
            fs_batch_operation,
            // [wmb-studio BEGIN phase1]
            wmb_scene::get_wmb_scene,
            wmb_scene::get_wmb_texture,
            wmb_scene::get_pre_texture,
            wmb_scene::resolve_legacy_node,
            wmb_scene::create_demo_model,
            wmb_scene::wmb_clipboard_copy,
            wmb_scene::wmb_clipboard_status,
            // [wmb-studio END]
        ])
        .run(tauri::generate_context!())
        .expect("error while running PGTools Studio");
}
