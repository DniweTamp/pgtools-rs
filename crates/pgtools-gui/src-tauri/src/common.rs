use pgtools_core::dat::DatFile;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

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


#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssetTreeNode {
    pub id: String,
    pub label: String,
    pub kind: String,
    pub format_badge: String,
    pub size: usize,
    #[serde(default)]
    pub modified: bool,
    #[serde(default)]
    pub compression: Option<String>,
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
    #[serde(default)]
    pub pending_uvd_encodes: Vec<PendingUvdSheetInfo>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status_message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MipLevelDesc {
    pub index: u32,
    pub label: String,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchOperationReport {
    pub files_processed: usize,
    pub converted: usize,
    pub skipped: usize,
    pub log_lines: Vec<String>,
    pub updated_outline: Option<DocumentOutline>,
}

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
