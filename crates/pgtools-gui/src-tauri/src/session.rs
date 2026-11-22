use crate::settings::{load_settings_from_disk, save_settings_to_disk, StudioSettings};
use crate::vfs::{build_outline_from_session, DocumentOutline};
use pgtools_core::{
    dat::DatFile,
    scene::UidMap,
    scene_ops::WmbClipboardData,
    wtb::WtbFile,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::fs;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::State;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingUvdSheetInfo {
    pub wta_container_id: String,
    pub wta_name: String,
    pub texture_id: u32,
    pub texture_id_hex: String,
    pub group_name: String,
    pub width: u32,
    pub height: u32,
    pub original_format: String,
    pub game_profile: String,
    pub has_confirmed_xt1_flags: bool,
    #[serde(skip)]
    pub lossless_sheet: Option<Arc<image::RgbaImage>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SaveEncodeOptions {
    pub encode_format: String,
    pub quality: String,
    pub generate_mips: bool,
    pub solidify: bool,
    pub make_opaque: bool,
    pub sync_flags: bool,
}

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

    /// Apply forced custom DAT save alignment (`Some(align)`) to top-level `DAT` / `PairedDatDtt`
    /// snapshots at save time. When `custom_align` is `None`, returns `self.clone()` untouched so
    /// in-memory batch alignments are preserved and clean/already-aligned archives are not re-serialized.
    pub(crate) fn apply_dat_save_alignment(&self, custom_align: Option<usize>) -> Self {
        let Some(align) = custom_align else {
            return self.clone();
        };
        match self {
            Self::SingleFile(bytes) => {
                if bytes.len() >= 0x20 && (&bytes[0..4] == b"DAT\0" || &bytes[0..4] == b"\0TAD") {
                    if let Ok(dat) = DatFile::from_bytes(bytes) {
                        return Self::SingleFile(Arc::new(dat.to_bytes_with_alignment(Some(align))));
                    }
                }
                self.clone()
            }
            Self::PairedDatDtt { dat, dtt } => {
                let next_dat = if dat.len() >= 0x20 && (&dat[0..4] == b"DAT\0" || &dat[0..4] == b"\0TAD") {
                    DatFile::from_bytes(dat)
                        .map(|d| Arc::new(d.to_bytes_with_alignment(Some(align))))
                        .unwrap_or_else(|_| dat.clone())
                } else {
                    dat.clone()
                };
                let next_dtt = if dtt.len() >= 0x20 && (&dtt[0..4] == b"DAT\0" || &dtt[0..4] == b"\0TAD") {
                    DatFile::from_bytes(dtt)
                        .map(|d| Arc::new(d.to_bytes_with_alignment(Some(align))))
                        .unwrap_or_else(|_| dtt.clone())
                } else {
                    dtt.clone()
                };
                Self::PairedDatDtt {
                    dat: next_dat,
                    dtt: next_dtt,
                }
            }
            Self::SplitWtaWtp { .. } => self.clone(),
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct UndoEntry {
    pub(crate) description: String,
    before: DocSnapshot,
    after: DocSnapshot,
    before_revision: u64,
    after_revision: u64,
    coalesce_key: Option<String>,
    timestamp_ms: u128,
    before_uid_maps: BTreeMap<String, UidMap>,
    after_uid_maps: BTreeMap<String, UidMap>,
    before_structure_version: u64,
    after_structure_version: u64,
    before_pending_uvd_sheets: BTreeMap<(String, u32), PendingUvdSheetInfo>,
    after_pending_uvd_sheets: BTreeMap<(String, u32), PendingUvdSheetInfo>,
}

const MAX_WTX1_CACHE_BYTES: usize = 128 * 1024 * 1024;
const MAX_WTX1_CACHE_ENTRIES: usize = 128;
const MAX_THUMB_CACHE_ENTRIES: usize = 512;

pub(crate) struct StudioSession {
    pub(crate) doc_path: Option<PathBuf>,
    pub(crate) wtp_path: Option<PathBuf>,
    pub(crate) dtt_path: Option<PathBuf>,
    pub(crate) current: Option<DocSnapshot>,
    pub(crate) saved_snapshot: Option<DocSnapshot>,
    pub(crate) dirty: bool,
    pub(crate) current_revision: u64,
    pub(crate) saved_revision: u64,
    pub(crate) next_revision: u64,
    pub(crate) undo_stack: Vec<UndoEntry>,
    pub(crate) redo_stack: Vec<UndoEntry>,
    pub(crate) settings: StudioSettings,
    pub(crate) structure_version: u64,
    pub(crate) committed_structure_version: u64,
    pub(crate) uid_maps: BTreeMap<String, UidMap>,
    pub(crate) committed_uid_maps: BTreeMap<String, UidMap>,
    pub(crate) pending_uvd_sheets: BTreeMap<(String, u32), PendingUvdSheetInfo>,
    pub(crate) committed_pending_uvd_sheets: BTreeMap<(String, u32), PendingUvdSheetInfo>,
    pub(crate) tex_thumb_cache: HashMap<String, (String, Option<String>)>,
    pub(crate) tex_thumb_order: VecDeque<String>,
    pub(crate) wtx1_cache: HashMap<(String, u32, u32), Vec<u8>>,
    pub(crate) wtx1_order: VecDeque<(String, u32, u32)>,
    pub(crate) wtx1_cache_bytes: usize,
    pub(crate) pre_wtb_cache: HashMap<String, (u64, Arc<WtbFile>)>,
    pub(crate) paired_wtb_cache: HashMap<String, Arc<Vec<(String, Arc<WtbFile>)>>>,
    pub(crate) container_wtb_cache: HashMap<String, Arc<WtbFile>>,
}

impl StudioSession {
    pub(crate) fn invalidate_texture_caches(&mut self) {
        self.tex_thumb_cache.clear();
        self.tex_thumb_order.clear();
        self.wtx1_cache.clear();
        self.wtx1_order.clear();
        self.wtx1_cache_bytes = 0;
        self.paired_wtb_cache.clear();
        self.container_wtb_cache.clear();
    }

    pub(crate) fn invalidate_single_texture_cache(
        &mut self,
        node_ids: &[&str],
        container_id: &str,
    ) {
        for &nid in node_ids {
            if self.tex_thumb_cache.remove(nid).is_some() {
                self.tex_thumb_order.retain(|k| k != nid);
            }
        }
        self.wtx1_cache.clear();
        self.wtx1_order.clear();
        self.wtx1_cache_bytes = 0;
        self.paired_wtb_cache.clear();
        self.container_wtb_cache.remove(container_id);
    }

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
            saved_snapshot: None,
            dirty: false,
            current_revision: 0,
            saved_revision: 0,
            next_revision: 1,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            settings: load_settings_from_disk(),
            structure_version: 1,
            committed_structure_version: 1,
            uid_maps: BTreeMap::new(),
            committed_uid_maps: BTreeMap::new(),
            pending_uvd_sheets: BTreeMap::new(),
            committed_pending_uvd_sheets: BTreeMap::new(),
            tex_thumb_cache: HashMap::new(),
            tex_thumb_order: VecDeque::new(),
            wtx1_cache: HashMap::new(),
            wtx1_order: VecDeque::new(),
            wtx1_cache_bytes: 0,
            pre_wtb_cache: HashMap::new(),
            paired_wtb_cache: HashMap::new(),
            container_wtb_cache: HashMap::new(),
        }
    }
}

pub(crate) struct AppState {
    pub(crate) session: Mutex<StudioSession>,
    pub(crate) clipboard: Mutex<Option<WmbClipboardData>>,
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
    // Backup of dest1 so we can roll back if the second rename fails after dest1 is replaced.
    let bak1 = parent1.join(format!(".{fname1}.pgbak.{pid}.{now_ns}"));

    if let Err(e) = fs::write(&tmp1, bytes1) {
        let _ = fs::remove_file(&tmp1);
        return Err(format!("Failed to write temporary file '{}': {e}", tmp1.display()));
    }
    if let Err(e) = fs::write(&tmp2, bytes2) {
        let _ = fs::remove_file(&tmp1);
        let _ = fs::remove_file(&tmp2);
        return Err(format!("Failed to write temporary file '{}': {e}", tmp2.display()));
    }

    let had_dest1 = dest1.exists();
    if had_dest1 {
        if let Err(e) = fs::rename(dest1, &bak1) {
            let _ = fs::remove_file(&tmp1);
            let _ = fs::remove_file(&tmp2);
            return Err(format!(
                "Failed to stage backup for '{}': {e}",
                dest1.display()
            ));
        }
    }

    if let Err(e) = fs::rename(&tmp1, dest1) {
        let _ = fs::remove_file(&tmp1);
        let _ = fs::remove_file(&tmp2);
        if had_dest1 {
            let _ = fs::rename(&bak1, dest1);
        }
        return Err(format!("Failed to commit saved file '{}': {e}", dest1.display()));
    }

    if let Err(e) = fs::rename(&tmp2, dest2) {
        let _ = fs::remove_file(&tmp2);
        // Roll back dest1 to its previous contents when possible.
        let _ = fs::remove_file(dest1);
        if had_dest1 {
            let _ = fs::rename(&bak1, dest1);
        }
        return Err(format!("Failed to commit paired file '{}': {e}", dest2.display()));
    }

    let _ = fs::remove_file(&bak1);
    Ok(())
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            session: Mutex::new(StudioSession::default()),
            clipboard: Mutex::new(None),
        }
    }
}

pub(crate) fn record_mutation(sess: &mut StudioSession, description: impl Into<String>, next: DocSnapshot) {
    record_mutation_with_key(sess, description, next, None);
}

pub(crate) fn record_mutation_with_key(
    sess: &mut StudioSession,
    description: impl Into<String>,
    next: DocSnapshot,
    coalesce_key: Option<String>,
) {
    const MAX_UNDO_ENTRIES: usize = 20;
    // Cap retained undo history so large archive edits do not pin multi-GB RSS.
    // Counts both before and after snapshot sizes per entry.
    const MAX_UNDO_BYTES: usize = 256 * 1024 * 1024;
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
                last.after_pending_uvd_sheets = sess.pending_uvd_sheets.clone();
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
                before_uid_maps: sess.committed_uid_maps.clone(),
                after_uid_maps: sess.uid_maps.clone(),
                before_structure_version: sess.committed_structure_version,
                after_structure_version: sess.structure_version,
                before_pending_uvd_sheets: sess.committed_pending_uvd_sheets.clone(),
                after_pending_uvd_sheets: sess.pending_uvd_sheets.clone(),
            });
            while sess.undo_stack.len() > 1 {
                let total_bytes: usize = sess
                    .undo_stack
                    .iter()
                    .map(|e| e.before.byte_size().saturating_add(e.after.byte_size()))
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
    sess.committed_uid_maps = sess.uid_maps.clone();
    sess.committed_structure_version = sess.structure_version;
    sess.committed_pending_uvd_sheets = sess.pending_uvd_sheets.clone();
    sess.current_revision = next_rev;
    sess.current = Some(next);
    sess.dirty = sess.current_revision != sess.saved_revision;
}

#[tauri::command]
pub fn open_document(path: String, state: State<'_, AppState>) -> Result<DocumentOutline, String> {
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
    sess.saved_snapshot = Some(snapshot.clone());
    sess.current = Some(snapshot);
    sess.dirty = false;
    sess.current_revision = 0;
    sess.saved_revision = 0;
    sess.next_revision = 1;
    sess.undo_stack.clear();
    sess.redo_stack.clear();
    sess.structure_version = 1;
    sess.committed_structure_version = 1;
    sess.uid_maps.clear();
    sess.committed_uid_maps.clear();
    sess.pending_uvd_sheets.clear();
    sess.committed_pending_uvd_sheets.clear();
    sess.invalidate_texture_caches();

    sess.settings.recent_files.retain(|r| r != &path);
    sess.settings.recent_files.insert(0, path);
    if sess.settings.recent_files.len() > 12 {
        sess.settings.recent_files.truncate(12);
    }
    let _ = save_settings_to_disk(&sess.settings);

    catch_unwind(AssertUnwindSafe(|| build_outline_from_session(&sess)))
        .map_err(|_| "Internal panic while parsing document outline".to_string())?
}

#[tauri::command]
pub fn save_document(
    output_path: Option<String>,
    encode_options: Option<SaveEncodeOptions>,
    state: State<'_, AppState>,
) -> Result<DocumentOutline, String> {
    // Snapshot under the lock, then drop it for the duration of recompression + disk I/O so other
    // commands (preview, list, undo) are not blocked on large document saves.
    let (dest, mut raw_snap, expected_revision, dat_align, pending_list, oodle_path, nvtt_path) = {
        let sess = lock_session(&state);
        let dest = output_path
            .map(PathBuf::from)
            .or_else(|| sess.doc_path.clone())
            .ok_or("No active document path")?;
        let snap = sess
            .current
            .as_ref()
            .ok_or("No active document loaded")?
            .clone();
        let pending: Vec<PendingUvdSheetInfo> =
            sess.pending_uvd_sheets.values().cloned().collect();
        (
            dest,
            snap,
            sess.current_revision,
            sess.settings.effective_dat_alignment(),
            pending,
            sess.settings.oodle_dll_path.clone(),
            sess.settings.nvtt_dll_path.clone(),
        )
    };

    if !pending_list.is_empty() {
        if let Some(ref opts) = encode_options {
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
            raw_snap = crate::uvd_cmd::reencode_pending_uvd_sheets_for_save(
                &raw_snap,
                &pending_list,
                opts,
                oodle_opt,
                nvtt_opt,
                dat_align,
            )?;
        }
    }

    let snap = raw_snap.apply_dat_save_alignment(dat_align);

    let mut paired_wtp: Option<PathBuf> = None;
    let mut paired_dtt: Option<PathBuf> = None;
    match &snap {
        DocSnapshot::SingleFile(bytes) => {
            atomic_write_file(&dest, bytes)?;
        }
        DocSnapshot::SplitWtaWtp { wta, wtp } => {
            let wtp_dest = dest.with_extension("wtp");
            atomic_write_pair(&dest, wta, &wtp_dest, wtp)?;
            paired_wtp = Some(wtp_dest);
        }
        DocSnapshot::PairedDatDtt { dat, dtt } => {
            let dtt_dest = dest.with_extension("dtt");
            atomic_write_pair(&dest, dat, &dtt_dest, dtt)?;
            paired_dtt = Some(dtt_dest);
        }
    }

    let mut sess = lock_session(&state);
    // Only mark clean if the in-memory revision is still the one we saved.
    if sess.current_revision == expected_revision {
        sess.doc_path = Some(dest);
        if let Some(p) = paired_wtp {
            sess.wtp_path = Some(p);
        }
        if let Some(p) = paired_dtt {
            sess.dtt_path = Some(p);
        }
        sess.current = Some(snap.clone());
        sess.saved_snapshot = Some(snap);
        sess.saved_revision = expected_revision;
        sess.pending_uvd_sheets.clear();
        sess.committed_pending_uvd_sheets.clear();
        sess.invalidate_texture_caches();
        sess.dirty = false;
    } else {
        // Concurrent mutation while we wrote — leave dirty; paths still updated to dest.
        sess.doc_path = Some(dest);
        if let Some(p) = paired_wtp {
            sess.wtp_path = Some(p);
        }
        if let Some(p) = paired_dtt {
            sess.dtt_path = Some(p);
        }
        sess.saved_snapshot = Some(snap);
        sess.dirty = true;
    }
    build_outline_from_session(&sess)
}

#[tauri::command]
pub fn undo_document(state: State<'_, AppState>) -> Result<DocumentOutline, String> {
    let mut sess = lock_session(&state);
    let entry = sess.undo_stack.pop().ok_or("Nothing to undo")?;
    sess.current = Some(entry.before.clone());
    sess.current_revision = entry.before_revision;
    sess.uid_maps = entry.before_uid_maps.clone();
    sess.committed_uid_maps = entry.before_uid_maps.clone();
    sess.structure_version = entry.before_structure_version;
    sess.committed_structure_version = entry.before_structure_version;
    sess.pending_uvd_sheets = entry.before_pending_uvd_sheets.clone();
    sess.committed_pending_uvd_sheets = entry.before_pending_uvd_sheets.clone();
    sess.invalidate_texture_caches();
    sess.redo_stack.push(entry);
    sess.dirty = sess.current_revision != sess.saved_revision;
    build_outline_from_session(&sess)
}

#[tauri::command]
pub fn redo_document(state: State<'_, AppState>) -> Result<DocumentOutline, String> {
    let mut sess = lock_session(&state);
    let entry = sess.redo_stack.pop().ok_or("Nothing to redo")?;
    sess.current = Some(entry.after.clone());
    sess.current_revision = entry.after_revision;
    sess.uid_maps = entry.after_uid_maps.clone();
    sess.committed_uid_maps = entry.after_uid_maps.clone();
    sess.structure_version = entry.after_structure_version;
    sess.committed_structure_version = entry.after_structure_version;
    sess.pending_uvd_sheets = entry.after_pending_uvd_sheets.clone();
    sess.committed_pending_uvd_sheets = entry.after_pending_uvd_sheets.clone();
    sess.invalidate_texture_caches();
    sess.undo_stack.push(entry);
    sess.dirty = sess.current_revision != sess.saved_revision;
    build_outline_from_session(&sess)
}
