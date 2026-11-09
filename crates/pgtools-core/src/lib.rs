//! `pgtools-core` — Pure Rust implementation of the PlatinumGames (`bayonetta_tools`) library.
//!
//! Supports archive extraction/repacking (`DAT`/`DTT`/`EVN`, `PKZ`, `EFF`/`IDD`, `SCR`),
//! 3D model manipulation and cross-platform conversion (`WMB`, `WMB2`, `WMB3`, `MDB`),
//! texture extraction, swizzling, and format conversion (`WTB`, `WTA`/`WTP`, `DDS`, `GTX`, `BNTX`, `XT1`),
//! skeletal animations and expression constraints (`MOT`, `MOT2`/`B3MOT`, `EXP`, `EXP2`),
//! binary XML (`BXM`, `GAD`, `GAArea`), cloth physics (`CLP`, `CLH`, `CLW`), stage lighting
//! (`FLS`, `FLA`, `CTA`, `LIT`), and bonus 010 Editor formats (`EST`, `COL`, `MSQ`, `COS`, `OBI`, `ATT`).

pub mod astc;
pub mod bone;
pub mod bt_formats;
pub mod bxm;
pub mod cloth;
pub mod dat;
pub mod databases;
pub mod demo;
pub mod eff;
pub mod exp;
pub mod io;
pub mod linalg;
pub mod mot;
pub mod oodle_fallback;
pub mod pkz;
pub mod scene;
pub mod scene_ops;
pub mod scr;
pub mod shader_db;
pub mod texture_conv;
pub mod wmb;
pub mod wmb3;
pub mod wtb;

pub use dat::{
    collect_dat_targets, dat_alignment_for_ext, parse_dat_alignment, validate_custom_dat_alignment,
    DatEntry, DatFile, DatHashMap, DEFAULT_CUSTOM_DAT_ALIGNMENT, MAX_CUSTOM_DAT_ALIGNMENT,
};

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum PgError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Invalid file magic: expected {expected}, got {actual:?}")]
    InvalidMagic { expected: &'static str, actual: Vec<u8> },
    #[error("Invalid or corrupted binary data: {0}")]
    InvalidData(String),
    #[error("Unsupported format or version: {0}")]
    Unsupported(String),
    #[error("XML error: {0}")]
    Xml(String),
    #[error("Serialization error: {0}")]
    Serialization(String),
}

pub type Result<T> = std::result::Result<T, PgError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Platform {
    Pc = 1,
    WiiU = 2,
    Xbox360 = 3,
    Ps3 = 4,
    Switch = 5,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Game {
    Bayonetta = 1,
    Bayonetta2 = 2,
    NierAutomata = 3,
    Vanquish = 4,
    AnarchyReigns = 5,
    MetalGearRising = 6,
    AstralChain = 7,
    Bayonetta3 = 8,
}
