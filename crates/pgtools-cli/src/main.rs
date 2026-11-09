//! `pgtools` CLI — Unified command-line interface for PlatinumGames modding and asset conversion.
//!
//! Provides clean subcommands for every format operation (`dat`, `pkz`, `eff`, `scr`, `wtx`, `wtb`,
//! `wmb`, `mot`, `exp`, `bxm`, `cloth`, `inspect`) AND supports busybox-style `argv[0]` multicall
//! dispatch so symlinking `dat_extractor`, `dat_creator`, `bxm_decoder`, `wmb_cleanup`, etc. to
//! `pgtools` works as a drop-in replacement for existing batch scripts.

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use pgtools_core::{
    bxm::BxmFile,
    cloth::{ClhFile, ClpFile, ClwFile},
    dat::DatFile,
    eff::EffFile,
    exp::ExpFile,
    io::Endian,
    mot::MotFile,
    pkz::PkzArchive,
    scr::ScrFile,
    demo::build_demo_dat,
    scene::{build_wmb3_scene, build_wmb_scene, fnv1a_64_hex, TextureRefDesc, UidMap},
    texture_conv::decode_dds_to_rgba8,
    wmb::WmbFile,
    wmb3::Wmb3File,
    wtb::WtbFile,
};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Parser)]
#[command(
    name = "pgtools",
    version,
    about = "High-performance Rust CLI for PlatinumGames file formats (Bayonetta 1/2/3, NieR:Automata, Vanquish, MGRR, Astral Chain)"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Extract one or more .dat / .dtt / .evn / .eff archives (replaces `dat_extractor`)
    DatExtract {
        #[arg(required = true)]
        files: Vec<PathBuf>,
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// Repack an extracted directory into a .dat / .dtt archive (replaces `dat_creator`)
    DatCreate {
        input_dir: PathBuf,
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// List files inside a .dat / .dtt archive or directory (replaces `dat_ls`)
    DatLs {
        target: PathBuf,
        #[arg(short, long)]
        yaml: bool,
    },
    /// Extract .pkz archives with Zstd or Oodle Kraken decompression (replaces `pkz_extractor`)
    PkzExtract {
        #[arg(required = true)]
        files: Vec<PathBuf>,
        #[arg(short, long)]
        output: Option<PathBuf>,
        #[arg(long)]
        oodle_dll: Option<PathBuf>,
    },
    /// Pack a directory of files recursively into a .pkz archive
    PkzCreate {
        input_dir: PathBuf,
        #[arg(short, long)]
        output: PathBuf,
        #[arg(long, default_value_t = true)]
        zstd: bool,
        #[arg(long)]
        compression: Option<String>,
        #[arg(long)]
        level: Option<i32>,
        #[arg(long, default_value_t = false)]
        force_compression: bool,
        #[arg(long)]
        oodle_dll: Option<PathBuf>,
    },
    /// Extract an .eff or .idd multi-directory effect container (replaces `eff_idd_extractor`)
    EffExtract {
        file: PathBuf,
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// Repack an extracted .eff or .idd directory (replaces `eff_idd_creator`)
    EffCreate {
        input_dir: PathBuf,
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// Extract models (.wmb) and textures (.wtb) from a .scr stage file (replaces `scr_extractor`)
    ScrExtract {
        file: PathBuf,
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// Repack a directory of .wmb models and .wtb textures into a .scr stage file (replaces `scr_creator`)
    ScrCreate {
        input_dir: PathBuf,
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// Extract textures from .wtb or .wta/.wtp (replaces `wtx_extractor`)
    WtxExtract {
        file: PathBuf,
        wtp_file: Option<PathBuf>,
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Also decode extracted DDS/GTX textures to .png previews
        #[arg(long)]
        png: bool,
    },
    /// Pack a directory of .dds / .gtx / .bntx / .xt1 textures into .wtb or .wta+.wtp (replaces `wtx_creator`)
    WtxCreate {
        input_dir: PathBuf,
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// Convert Wii U .wtb or .wta+.wtp (.gtx) to PC .wtb (.dds) natively in Rust (replaces `wtb_convert_wiiu_pc`)
    WtbConvertWiiuPc {
        file: PathBuf,
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// Import textures from source .wtb/.wta into target .wtb (replaces `wtb_import_textures`)
    WtbImportTextures {
        target_file: PathBuf,
        source_file: PathBuf,
        #[arg(short, long)]
        overwrite: bool,
    },
    /// Decode binary .bxm / .xml into UTF-8 XML text (replaces `bxm_decoder`)
    BxmDecode {
        input: PathBuf,
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// Encode UTF-8 XML text into binary .bxm (replaces `bxm_encoder`)
    BxmEncode {
        input: PathBuf,
        #[arg(short, long)]
        output: Option<PathBuf>,
        #[arg(short, long, default_value = "BXM")]
        tag: String,
    },
    /// Convert or remap .clp / _clp.bxm cloth physics files (replaces `clp_convert` & `clp_convert_wiiu_pc`)
    ClpConvert {
        input: PathBuf,
        #[arg(short, long)]
        output: Option<PathBuf>,
        #[arg(long)]
        swap_endian: bool,
    },
    /// Convert or remap .clh / _clh.bxm cloth collision files (replaces `clh_convert` & `clh_convert_wiiu_pc`)
    ClhConvert {
        input: PathBuf,
        #[arg(short, long)]
        output: Option<PathBuf>,
        #[arg(long)]
        swap_endian: bool,
    },
    /// Convert or remap .clw / _clw.bxm cloth wind files (replaces `clw_convert` & `clw_convert_wiiu_pc`)
    ClwConvert {
        input: PathBuf,
        #[arg(short, long)]
        output: Option<PathBuf>,
        #[arg(long)]
        swap_endian: bool,
    },
    /// Inspect, decode frames, remap bones, or swap endianness of .mot files (replaces `mot_tool` & `mot_convert_wiiu_pc`)
    MotTool {
        input: PathBuf,
        #[arg(short, long)]
        output: Option<PathBuf>,
        #[arg(long)]
        decode: bool,
        #[arg(long)]
        decode_frame: Option<usize>,
        #[arg(short = 'e', long)]
        swap_endian: bool,
        #[arg(long)]
        overwrite: bool,
    },
    /// Inspect, add entries, recompute layout, or swap endianness of .exp files (replaces `exp_tool` & `exp_convert_wiiu_pc`)
    ExpTool {
        input: PathBuf,
        #[arg(short, long)]
        output: Option<PathBuf>,
        #[arg(short = 'e', long)]
        swap_endian: bool,
        #[arg(long)]
        overwrite: bool,
    },
    /// Inspect, clean up, scale, swap endianness (Wii U <-> PC), or export Wavefront .obj for .wmb and .wmb3 models (replaces `wmb_cleanup` & `wmb_convert_wiiu_pc`)
    WmbTool {
        input: PathBuf,
        #[arg(short, long)]
        output: Option<PathBuf>,
        #[arg(short = 'v', long)]
        cleanup_vertexes: bool,
        #[arg(long)]
        cleanup_materials: bool,
        #[arg(long)]
        cleanup_material_sizes: bool,
        #[arg(long)]
        maximize_material_sizes: bool,
        #[arg(long)]
        remove_triangle_strips: bool,
        #[arg(long)]
        normals_to_wide: bool,
        #[arg(long)]
        check_normals: bool,
        #[arg(short = 'e', long)]
        swap_endian: bool,
        #[arg(long)]
        scale: Option<f32>,
        #[arg(long)]
        export_obj: Option<PathBuf>,
        #[arg(long)]
        dump_bones: bool,
        #[arg(long)]
        dump_materials: bool,
        #[arg(long)]
        overwrite: bool,
    },
    /// Find common global bone IDs across multiple .wmb or .dat files (replaces `wmb_common_bones`)
    WmbCommonBones {
        #[arg(required = true)]
        files: Vec<PathBuf>,
    },
    /// Dump the global-to-local BoneIndexTranslateTable from a .wmb or .dat file (replaces `wmb_get_bone_map`)
    WmbGetBoneMap {
        file: PathBuf,
    },
    /// Dump the WmbSceneManifest JSON for a .wmb, .wmb3, or .dat file
    WmbSceneDump {
        file: PathBuf,
        #[arg(long)]
        no_binary: bool,
    },
    /// Generate a built-in procedural demo model (.dat containing demo.wmb and demo.wtb) for b1, b2, or b3
    DemoModel {
        game: String,
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
}

fn load_bone_map_from_path_or_dat(path: &Path) -> Result<std::collections::BTreeMap<u16, u16>> {
    let raw = fs::read(path).with_context(|| format!("Failed to read {}", path.display()))?;
    let model_bytes = if path.extension().and_then(|e| e.to_str()) == Some("dat") {
        let dat = DatFile::from_bytes(&raw)?;
        let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
        let target_wmb = format!("{stem}.wmb");
        let entry = dat
            .entries
            .iter()
            .find(|e| e.name == target_wmb || e.extension == "wmb")
            .ok_or_else(|| anyhow::anyhow!("No .wmb found inside {}", path.display()))?;
        entry.data.clone()
    } else {
        raw
    };
    if model_bytes.starts_with(b"WMB3") || model_bytes.starts_with(b"3BMW") {
        let wmb3 = Wmb3File::from_bytes(&model_bytes)?;
        Ok(wmb3.bone_index_translate_table.table)
    } else {
        let wmb = WmbFile::from_bytes(&model_bytes)?;
        Ok(wmb.bone_index_translate_table.table)
    }
}

/// Resolve busybox-style `argv[0]` symlink/binary names (`dat_extractor`, `dat_creator`,
/// `bxm_decoder`, `wmb_cleanup`, etc.) into canonical `pgtools <subcommand>` arguments.
fn resolve_multicall_args() -> Vec<std::ffi::OsString> {
    let args: Vec<std::ffi::OsString> = std::env::args_os().collect();
    if args.is_empty() {
        return args;
    }
    let bin_stem = Path::new(&args[0])
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("pgtools")
        .to_ascii_lowercase()
        .replace('-', "_");

    let mapped: Option<&[&str]> = match bin_stem.as_str() {
        "dat_extractor" => Some(&["dat-extract"]),
        "dat_creator" => Some(&["dat-create"]),
        "dat_ls" => Some(&["dat-ls"]),
        "pkz_extractor" => Some(&["pkz-extract"]),
        "pkz_creator" => Some(&["pkz-create"]),
        "eff_idd_extractor" | "eff_extractor" => Some(&["eff-extract"]),
        "eff_idd_creator" | "eff_creator" => Some(&["eff-create"]),
        "scr_extractor" => Some(&["scr-extract"]),
        "scr_creator" => Some(&["scr-create"]),
        "wtx_extractor" | "wtb_extractor" => Some(&["wtx-extract"]),
        "wtx_creator" | "wtb_creator" => Some(&["wtx-create"]),
        "wtb_convert_wiiu_pc" => Some(&["wtb-convert-wiiu-pc"]),
        "wtb_import_textures" => Some(&["wtb-import-textures"]),
        "bxm_decoder" => Some(&["bxm-decode"]),
        "bxm_encoder" => Some(&["bxm-encode"]),
        "clp_convert" => Some(&["clp-convert"]),
        "clp_convert_wiiu_pc" => Some(&["clp-convert", "--swap-endian"]),
        "clh_convert" => Some(&["clh-convert"]),
        "clh_convert_wiiu_pc" => Some(&["clh-convert", "--swap-endian"]),
        "clw_convert" => Some(&["clw-convert"]),
        "clw_convert_wiiu_pc" => Some(&["clw-convert", "--swap-endian"]),
        "mot_tool" => Some(&["mot-tool"]),
        "mot_convert_wiiu_pc" => Some(&["mot-tool", "--swap-endian"]),
        "exp_tool" => Some(&["exp-tool"]),
        "exp_convert_wiiu_pc" => Some(&["exp-tool", "--swap-endian"]),
        "wmb_cleanup" | "wmb_tool" => Some(&["wmb-tool"]),
        "wmb_convert_wiiu_pc" => Some(&["wmb-tool", "--swap-endian"]),
        "wmb_common_bones" => Some(&["wmb-common-bones"]),
        "wmb_get_bone_map" => Some(&["wmb-get-bone-map"]),
        _ => None,
    };

    if let Some(injected) = mapped {
        let mut rewritten = Vec::with_capacity(args.len() + injected.len());
        rewritten.push(std::ffi::OsString::from("pgtools"));
        for &tok in injected {
            rewritten.push(std::ffi::OsString::from(tok));
        }
        rewritten.extend(args.into_iter().skip(1));
        rewritten
    } else {
        args
    }
}

fn main() -> Result<()> {
    let cli = Cli::parse_from(resolve_multicall_args());

    match cli.command {
        Commands::DatExtract { files, output } => {
            for f in files {
                let raw = fs::read(&f).with_context(|| format!("Failed to read {}", f.display()))?;
                let dat = DatFile::from_bytes(&raw)?;
                let ext = f
                    .extension()
                    .and_then(|e| e.to_str())
                    .map(|e| format!(".{e}"))
                    .unwrap_or_else(|| ".dat".to_string());
                let stem = f.file_stem().and_then(|s| s.to_str()).unwrap_or("archive");
                let dir_name = format!("{}{}", stem, ext.replace('.', "_"));
                let base_dir = output
                    .clone()
                    .unwrap_or_else(|| f.parent().unwrap_or(Path::new(".")).to_path_buf());
                let dest = base_dir.join(dir_name);
                let warnings = dat.extract_to_dir(&dest, &ext)?;
                for w in warnings {
                    eprintln!("Warning: {w}");
                }
                println!("Extracted {} files to {}", dat.entries.len(), dest.display());
            }
        }
        Commands::DatCreate { input_dir, output } => {
            let (dat, ext) = DatFile::create_from_dir(&input_dir)?;
            let out_path = if let Some(o) = output {
                o
            } else {
                let out_dir = input_dir.join("dat_output");
                fs::create_dir_all(&out_dir)?;
                let suffix = ext.replace('.', "_");
                let dir_name = input_dir
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or("output");
                let base = dir_name.trim_end_matches(&suffix);
                out_dir.join(format!("{base}{ext}"))
            };
            fs::write(&out_path, dat.to_bytes())?;
            println!("Created DAT archive at {}", out_path.display());
        }
        Commands::DatLs { target, yaml } => {
            let mut list = Vec::new();
            let raw = fs::read(&target)?;
            DatFile::list_recursive(&target.to_string_lossy(), &raw, &mut list)?;
            if yaml {
                println!("{}", serde_yaml::to_string(&list)?);
            } else {
                for item in list {
                    println!("{item}");
                }
            }
        }
        Commands::PkzExtract {
            files,
            output,
            oodle_dll,
        } => {
            for f in files {
                let raw = fs::read(&f)?;
                let pkz = PkzArchive::from_bytes(&raw)?;
                let dest = output
                    .clone()
                    .unwrap_or_else(|| f.parent().unwrap_or(Path::new(".")).to_path_buf());
                let extracted = pkz.extract_all(&raw, &dest, oodle_dll.as_deref())?;
                println!("Extracted {} files from {}", extracted.len(), f.display());
            }
        }
        Commands::PkzCreate {
            input_dir,
            output,
            zstd,
            compression,
            level,
            force_compression,
            oodle_dll,
        } => {
            let codec = compression.unwrap_or_else(|| {
                if zstd {
                    "ZStandard".to_string()
                } else {
                    "None".to_string()
                }
            });
            let lvl = level.unwrap_or_else(|| pgtools_core::pkz::default_compression_level(&codec));
            let bytes = PkzArchive::build_from_dir(
                &input_dir,
                &codec,
                lvl,
                force_compression,
                oodle_dll.as_deref(),
            )?;
            fs::write(&output, bytes)?;
            println!("Created PKZ archive at {}", output.display());
        }
        Commands::EffExtract { file, output } => {
            let raw = fs::read(&file)?;
            let eff = EffFile::from_bytes(&raw)?;
            let ext = file
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| format!(".{e}"))
                .unwrap_or_else(|| ".eff".to_string());
            let stem = file.file_stem().and_then(|s| s.to_str()).unwrap_or("eff");
            let base = output.unwrap_or_else(|| file.parent().unwrap_or(Path::new(".")).to_path_buf());
            let dest = base.join(format!("{}{}", stem, ext.replace('.', "_")));
            eff.extract_to_dir(&dest, &ext)?;
            println!("Extracted EFF/IDD to {}", dest.display());
        }
        Commands::EffCreate { input_dir, output } => {
            let (eff, ext) = EffFile::create_from_dir(&input_dir)?;
            let out_path = output.unwrap_or_else(|| {
                let out_dir = input_dir.join("eff_output");
                let _ = fs::create_dir_all(&out_dir);
                let suffix = ext.replace('.', "_");
                let stem = input_dir
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or("output")
                    .trim_end_matches(&suffix);
                out_dir.join(format!("{stem}{ext}"))
            });
            fs::write(&out_path, eff.to_bytes()?)?;
            println!("Created EFF/IDD at {}", out_path.display());
        }
        Commands::ScrExtract { file, output } => {
            let raw = fs::read(&file)?;
            let scr = ScrFile::from_bytes(&raw)?;
            let stem = file.file_stem().and_then(|s| s.to_str()).unwrap_or("scr");
            let base = output.unwrap_or_else(|| file.parent().unwrap_or(Path::new(".")).to_path_buf());
            let dest = base.join(format!("{stem}_scr"));
            scr.extract_to_dir(&dest)?;
            println!("Extracted SCR ({} models) to {}", scr.models.len(), dest.display());
        }
        Commands::ScrCreate { input_dir, output } => {
            let meta_dir = input_dir.join(".metadata");
            let big: bool = serde_yaml::from_str(&fs::read_to_string(meta_dir.join("big.yaml"))?)?;
            let is_bayo2: bool =
                serde_yaml::from_str(&fs::read_to_string(meta_dir.join("bayo2.yaml"))?)?;
            let unknown_header: u32 =
                serde_yaml::from_str(&fs::read_to_string(meta_dir.join("unknown.yaml"))?)
                    .unwrap_or(1);
            let models_metadata =
                serde_yaml::from_str(&fs::read_to_string(meta_dir.join("models_metadata.yaml"))?)?;

            let mut wmb_files: Vec<PathBuf> = fs::read_dir(&input_dir)?
                .filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("wmb"))
                .collect();
            wmb_files.sort();
            let models: Vec<Vec<u8>> = wmb_files
                .iter()
                .map(|p| fs::read(p))
                .collect::<std::io::Result<_>>()?;
            let textures = if !is_bayo2 && input_dir.join("000.wtb").exists() {
                Some(fs::read(input_dir.join("000.wtb"))?)
            } else {
                None
            };

            let scr = ScrFile {
                is_bayo2,
                endian: if big { Endian::Big } else { Endian::Little },
                unknown_header,
                models_metadata,
                models,
                textures,
            };
            let out_path = output.unwrap_or_else(|| {
                let out_dir = input_dir.join("scr_output");
                let _ = fs::create_dir_all(&out_dir);
                let name = input_dir
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or("out")
                    .replace("_scr", ".scr");
                out_dir.join(name)
            });
            fs::write(&out_path, scr.to_bytes())?;
            println!("Created SCR at {}", out_path.display());
        }
        Commands::WtxExtract {
            file,
            wtp_file,
            output,
            png,
        } => {
            let raw = fs::read(&file)?;
            let ext = file
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| format!(".{e}"))
                .unwrap_or_else(|| ".wtb".to_string());
            let wtp_bytes = if ext == ".wta" {
                let wtp_p = wtp_file.unwrap_or_else(|| file.with_extension("wtp"));
                Some(fs::read(wtp_p)?)
            } else {
                None
            };
            let wtb = WtbFile::from_bytes(&raw, wtp_bytes.as_deref())?;
            let stem = file.file_stem().and_then(|s| s.to_str()).unwrap_or("tex");
            let base = output.unwrap_or_else(|| file.parent().unwrap_or(Path::new(".")).to_path_buf());
            let dest = base.join(format!("{}{}", stem, ext.replace('.', "_")));
            wtb.extract_to_dir(&dest, stem, &ext)?;
            if png {
                for (i, e) in wtb.entries.iter().enumerate() {
                    if let Ok(img) = decode_dds_to_rgba8(&e.data) {
                        let _ = img.save(dest.join(format!("{}_{:03}.png", stem, i)));
                    }
                }
            }
            println!("Extracted {} textures to {}", wtb.entries.len(), dest.display());
        }
        Commands::WtxCreate { input_dir, output } => {
            let meta_dir = input_dir.join(".metadata");
            let big: bool = if meta_dir.join("big.yaml").exists() {
                serde_yaml::from_str(&fs::read_to_string(meta_dir.join("big.yaml"))?)?
            } else {
                false
            };
            let ext: String = if meta_dir.join("extension.yaml").exists() {
                serde_yaml::from_str(&fs::read_to_string(meta_dir.join("extension.yaml"))?)?
            } else {
                ".wtb".to_string()
            };
            let flags: Vec<u32> = if meta_dir.join("texture_flags.yaml").exists() {
                serde_yaml::from_str(&fs::read_to_string(meta_dir.join("texture_flags.yaml"))?)?
            } else {
                Vec::new()
            };
            let idx: Vec<u32> = if meta_dir.join("texture_idx.yaml").exists() {
                serde_yaml::from_str(&fs::read_to_string(meta_dir.join("texture_idx.yaml"))?)?
            } else {
                Vec::new()
            };
            let tex_infos: Vec<u32> = if meta_dir.join("texture_infos.yaml").exists() {
                serde_yaml::from_str(&fs::read_to_string(meta_dir.join("texture_infos.yaml"))?)?
            } else {
                Vec::new()
            };
            let unknown_hdr: u32 = if meta_dir.join("unknown.yaml").exists() {
                serde_yaml::from_str(&fs::read_to_string(meta_dir.join("unknown.yaml"))?)?
            } else {
                1
            };

            let mut tex_paths: Vec<PathBuf> = fs::read_dir(&input_dir)?
                .filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| {
                    matches!(
                        p.extension().and_then(|e| e.to_str()),
                        Some("dds" | "gtx" | "bntx" | "xt1")
                    )
                })
                .collect();
            tex_paths.sort_by(|a, b| {
                let parse_idx = |p: &Path| -> Option<usize> {
                    let stem = p.file_stem()?.to_str()?;
                    let (_, num_str) = stem.rsplit_once('_')?;
                    num_str.parse::<usize>().ok()
                };
                match (parse_idx(a), parse_idx(b)) {
                    (Some(ia), Some(ib)) => ia.cmp(&ib).then_with(|| a.cmp(b)),
                    _ => a.cmp(b),
                }
            });

            let mut wtb = WtbFile::new(if big { Endian::Big } else { Endian::Little }, ext == ".wta");
            wtb.unknown = unknown_hdr;
            for (i, p) in tex_paths.iter().enumerate() {
                let data = fs::read(p)?;
                let fl = flags.get(i).copied().unwrap_or(0);
                let tid = idx.get(i).copied();
                wtb.push(data, fl, tid)?;
            }
            if !tex_infos.is_empty() {
                wtb.texture_infos = tex_infos;
            }
            let (main_bytes, wtp_opt) = wtb.to_bytes();
            let out_path = output.unwrap_or_else(|| {
                let out_dir = input_dir.join("wtx_output");
                let _ = fs::create_dir_all(&out_dir);
                let name = input_dir
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or("out")
                    .replace("_wta", "")
                    .replace("_wtb", "");
                out_dir.join(format!("{name}{ext}"))
            });
            fs::write(&out_path, main_bytes)?;
            if let Some(wtp_bytes) = wtp_opt {
                fs::write(out_path.with_extension("wtp"), wtp_bytes)?;
            }
            println!("Created texture archive at {}", out_path.display());
        }
        Commands::WtbConvertWiiuPc { file, output } => {
            let raw = fs::read(&file)?;
            let wtp_bytes = if file.extension().and_then(|e| e.to_str()) == Some("wta") {
                Some(fs::read(file.with_extension("wtp"))?)
            } else {
                None
            };
            let wtb = WtbFile::from_bytes(&raw, wtp_bytes.as_deref())?;
            let pc_wtb = wtb.convert_wiiu_to_pc()?;
            let (bytes, _) = pc_wtb.to_bytes();
            let out_path = output.unwrap_or_else(|| {
                let _ = fs::create_dir_all("wtb_output");
                PathBuf::from("wtb_output")
                    .join(file.with_extension("wtb").file_name().unwrap())
            });
            fs::write(&out_path, bytes)?;
            println!("Converted Wii U textures to PC WTB at {}", out_path.display());
        }
        Commands::WtbImportTextures {
            target_file,
            source_file,
            overwrite,
        } => {
            let mut target_wtb = WtbFile::from_bytes(&fs::read(&target_file)?, None)?;
            let src_raw = fs::read(&source_file)?;
            let src_wtp = if source_file.extension().and_then(|e| e.to_str()) == Some("wta") {
                Some(fs::read(source_file.with_extension("wtp"))?)
            } else {
                None
            };
            let src_wtb = WtbFile::from_bytes(&src_raw, src_wtp.as_deref())?;
            let converted_src = if src_wtb.endian.is_big() {
                src_wtb.convert_wiiu_to_pc()?
            } else {
                src_wtb
            };
            for e in converted_src.entries {
                target_wtb.push(e.data, e.flags, None)?;
            }
            let (bytes, _) = target_wtb.to_bytes();
            let out_path = if overwrite {
                target_file
            } else {
                let _ = fs::create_dir_all("wtb_output");
                PathBuf::from("wtb_output").join(target_file.file_name().unwrap())
            };
            fs::write(&out_path, bytes)?;
            println!("Imported textures into {}", out_path.display());
        }
        Commands::BxmDecode { input, output } => {
            let raw = fs::read(&input)?;
            let bxm = BxmFile::from_bytes(&raw)?;
            let xml = bxm.to_xml_string();
            let out_path = output.unwrap_or_else(|| PathBuf::from(format!("{}.xml", input.display())));
            fs::write(&out_path, xml)?;
            println!("Decoded BXM to {}", out_path.display());
        }
        Commands::BxmEncode { input, output, tag } => {
            let xml = fs::read_to_string(&input)?;
            let bxm = BxmFile::from_xml_str(&xml, &tag)?;
            let out_path = output.unwrap_or_else(|| input.with_extension("bxm"));
            fs::write(&out_path, bxm.to_bytes(Endian::Big))?;
            println!("Encoded XML to {}", out_path.display());
        }
        Commands::ClpConvert {
            input,
            output,
            swap_endian,
        } => {
            let raw = fs::read(&input)?;
            let clp = if input.extension().and_then(|e| e.to_str()) == Some("bxm") {
                ClpFile::from_bxm(&raw)?
            } else {
                ClpFile::from_bytes(&raw)?
            };
            let target_endian = if swap_endian {
                clp.endian.swap()
            } else {
                Endian::Little
            };
            let out_path = output.unwrap_or_else(|| input.with_extension("clp"));
            fs::write(&out_path, clp.to_bytes(target_endian))?;
            println!("Wrote CLP to {}", out_path.display());
        }
        Commands::ClhConvert {
            input,
            output,
            swap_endian,
        } => {
            let raw = fs::read(&input)?;
            let clh = if input.extension().and_then(|e| e.to_str()) == Some("bxm") {
                ClhFile::from_bxm(&raw)?
            } else {
                ClhFile::from_bytes(&raw)?
            };
            let target_endian = if swap_endian {
                clh.endian.swap()
            } else {
                Endian::Little
            };
            let out_path = output.unwrap_or_else(|| input.with_extension("clh"));
            fs::write(&out_path, clh.to_bytes(target_endian))?;
            println!("Wrote CLH to {}", out_path.display());
        }
        Commands::ClwConvert {
            input,
            output,
            swap_endian,
        } => {
            let raw = fs::read(&input)?;
            let clw = if input.extension().and_then(|e| e.to_str()) == Some("bxm") {
                ClwFile::from_bxm(&raw)?
            } else {
                ClwFile::from_bytes(&raw)?
            };
            let target_endian = if swap_endian {
                clw.endian.swap()
            } else {
                Endian::Little
            };
            let out_path = output.unwrap_or_else(|| input.with_extension("clw"));
            fs::write(&out_path, clw.to_bytes(target_endian))?;
            println!("Wrote CLW to {}", out_path.display());
        }
        Commands::MotTool {
            input,
            output,
            decode,
            decode_frame,
            swap_endian,
            overwrite,
        } => {
            let raw = fs::read(&input)?;
            let mot = MotFile::from_bytes(&raw)?;
            if decode {
                println!("{}", serde_yaml::to_string(&mot.decode())?);
                return Ok(());
            }
            if let Some(fi) = decode_frame {
                println!("{}", serde_yaml::to_string(&mot.decode_frame(fi)?)?);
                return Ok(());
            }
            let target_endian = if swap_endian {
                mot.endian.swap()
            } else {
                mot.endian
            };
            let out_path = if overwrite {
                input
            } else if let Some(o) = output {
                o
            } else {
                let _ = fs::create_dir_all("mot_output");
                PathBuf::from("mot_output").join(input.file_name().unwrap())
            };
            fs::write(&out_path, mot.to_bytes(target_endian))?;
            println!("Wrote MOT to {}", out_path.display());
        }
        Commands::ExpTool {
            input,
            output,
            swap_endian,
            overwrite,
        } => {
            let raw = fs::read(&input)?;
            let exp = ExpFile::from_bytes(&raw)?;
            let target_endian = if swap_endian {
                exp.endian().swap()
            } else {
                exp.endian()
            };
            let out_path = if overwrite {
                input
            } else if let Some(o) = output {
                o
            } else {
                let _ = fs::create_dir_all("exp_output");
                PathBuf::from("exp_output").join(input.file_name().unwrap())
            };
            fs::write(&out_path, exp.to_bytes(target_endian))?;
            println!("Wrote EXP to {}", out_path.display());
        }
        Commands::WmbTool {
            input,
            output,
            cleanup_vertexes,
            cleanup_materials,
            cleanup_material_sizes,
            maximize_material_sizes,
            remove_triangle_strips,
            normals_to_wide,
            check_normals,
            swap_endian,
            scale,
            export_obj,
            dump_bones,
            dump_materials,
            overwrite,
        } => {
            let raw = fs::read(&input)?;
            if raw.starts_with(b"WMB3") || raw.starts_with(b"3BMW") {
                let mut wmb3 = Wmb3File::from_bytes(&raw)?;
                if dump_bones {
                    println!("{}", serde_yaml::to_string(&wmb3.bones)?);
                    return Ok(());
                }
                if dump_materials {
                    println!("{}", serde_yaml::to_string(&wmb3.materials)?);
                    return Ok(());
                }
                if let Some(obj_path) = export_obj {
                    fs::write(&obj_path, wmb3.export_obj())?;
                    println!("Exported Wavefront OBJ to {}", obj_path.display());
                    return Ok(());
                }
                if let Some(s) = scale {
                    for vg in &mut wmb3.vertex_groups {
                        for v in &mut vg.vertexes {
                            if let Some(ref mut p) = v.position {
                                p.x *= s;
                                p.y *= s;
                                p.z *= s;
                            }
                        }
                    }
                    for b in &mut wmb3.bones {
                        b.local_position.x *= s;
                        b.local_position.y *= s;
                        b.local_position.z *= s;
                        b.position.x *= s;
                        b.position.y *= s;
                        b.position.z *= s;
                        b.t_position.x *= s;
                        b.t_position.y *= s;
                        b.t_position.z *= s;
                    }
                    for v in &mut wmb3.bounding_box {
                        *v *= s;
                    }
                }
                let out_path = if overwrite {
                    input
                } else if let Some(o) = output {
                    o
                } else {
                    let _ = fs::create_dir_all("wmb_output");
                    PathBuf::from("wmb_output").join(input.file_name().unwrap())
                };
                fs::write(&out_path, wmb3.to_bytes()?)?;
                println!("Wrote WMB3 to {}", out_path.display());
                return Ok(());
            }

            let mut wmb = WmbFile::from_bytes(&raw)?;
            if dump_bones {
                println!("{}", serde_yaml::to_string(&wmb.dump_bones())?);
                return Ok(());
            }
            if dump_materials {
                let mats: Vec<_> = wmb
                    .materials
                    .iter()
                    .enumerate()
                    .map(|(i, m)| m.decode(i))
                    .collect();
                println!("{}", serde_yaml::to_string(&mats)?);
                return Ok(());
            }
            if let Some(obj_path) = export_obj {
                fs::write(&obj_path, wmb.export_obj())?;
                println!("Exported Wavefront OBJ to {}", obj_path.display());
                return Ok(());
            }
            if check_normals {
                wmb.check_normals();
            }
            if normals_to_wide {
                wmb.normals_to_wide();
            }
            if remove_triangle_strips {
                wmb.remove_triangle_strips();
            }
            if cleanup_materials {
                wmb.cleanup_materials();
            }
            if cleanup_material_sizes {
                wmb.cleanup_material_sizes()?;
            }
            if maximize_material_sizes {
                wmb.maximize_material_sizes()?;
            }
            if cleanup_vertexes {
                wmb.cleanup_vertexes();
            }
            if let Some(s) = scale {
                wmb.scale(s);
            }
            let target_endian = if swap_endian {
                wmb.endian.swap()
            } else {
                wmb.endian
            };
            let out_path = if overwrite {
                input
            } else if let Some(o) = output {
                o
            } else {
                let _ = fs::create_dir_all("wmb_output");
                PathBuf::from("wmb_output").join(input.file_name().unwrap())
            };
            fs::write(&out_path, wmb.to_bytes(target_endian)?)?;
            println!("Wrote WMB to {}", out_path.display());
        }
        Commands::WmbCommonBones { files } => {
            let mut common: Option<BTreeSet<u16>> = None;
            for f in files {
                let table = load_bone_map_from_path_or_dat(&f)?;
                let keys: BTreeSet<u16> = table.keys().copied().collect();
                common = Some(match common {
                    Some(prev) => prev.intersection(&keys).copied().collect(),
                    None => keys,
                });
            }
            let list: Vec<u16> = common.unwrap_or_default().into_iter().collect();
            println!("{}", serde_yaml::to_string(&list)?);
        }
        Commands::WmbGetBoneMap { file } => {
            let table = load_bone_map_from_path_or_dat(&file)?;
            println!("{}", serde_yaml::to_string(&table)?);
        }
        Commands::WmbSceneDump { file, no_binary } => {
            let _ = no_binary;
            let raw = fs::read(&file).with_context(|| format!("Failed to read {}", file.display()))?;
            let fname = file
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("model.wmb")
                .to_string();
            let (model_name, model_bytes, bank_textures) = if raw.starts_with(b"DAT\0") || raw.starts_with(b"\0TAD") {
                let dat = DatFile::from_bytes(&raw)?;
                let mut tex_refs = Vec::new();
                for e in &dat.entries {
                    if e.extension.eq_ignore_ascii_case("wtb") {
                        if let Ok(wtb) = WtbFile::from_bytes(&e.data, None) {
                            for te in &wtb.entries {
                                let tid = te.texture_id.unwrap_or(te.index as u32);
                                let (fmt, w, h) = te
                                    .metadata
                                    .as_ref()
                                    .map(|m| (m.format.display_name(), m.width, m.height))
                                    .unwrap_or_else(|| ("TEX".to_string(), 0, 0));
                                tex_refs.push(TextureRefDesc {
                                    texture_id: tid,
                                    id_hex: format!("0x{tid:08X}"),
                                    name: format!("tex{}", te.index),
                                    width: w,
                                    height: h,
                                    format_name: fmt,
                                    content_hash: fnv1a_64_hex(&te.data),
                                    used_by_material_ids: Vec::new(),
                                    thumbnail_data_url: None,
                                });
                            }
                        }
                    }
                }
                let entry = dat
                    .entries
                    .iter()
                    .find(|e| e.extension.eq_ignore_ascii_case("wmb"))
                    .ok_or_else(|| anyhow::anyhow!("No .wmb entry inside {}", file.display()))?;
                (entry.name.clone(), entry.data.clone(), tex_refs)
            } else {
                (fname, raw, Vec::new())
            };

            let mut uid_map = UidMap::new();
            let manifest = if model_bytes.starts_with(b"WMB3") || model_bytes.starts_with(b"3BMW") {
                let wmb3 = Wmb3File::from_bytes(&model_bytes)?;
                let (m, _) = build_wmb3_scene(&wmb3, &mut uid_map, "root", &model_name, 1, bank_textures, &[]);
                m
            } else {
                let wmb = WmbFile::from_bytes(&model_bytes)?;
                let (m, _) = build_wmb_scene(&wmb, &mut uid_map, "root", &model_name, 1, bank_textures, &[]);
                m
            };
            println!("{}", serde_json::to_string_pretty(&manifest)?);
        }
        Commands::DemoModel { game, output } => {
            let dat_bytes = build_demo_dat(&game)?;
            let out_path = output.unwrap_or_else(|| PathBuf::from(format!("pgtools-demo-{}.dat", game.to_ascii_lowercase())));
            fs::write(&out_path, dat_bytes)?;
            println!("Created demo model DAT at {}", out_path.display());
        }
    }

    Ok(())
}
