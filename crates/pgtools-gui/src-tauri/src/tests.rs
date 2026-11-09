use crate::formats_cmd::{decode_csv_bytes_auto, format_hex_preview, parse_csv_table, serialize_csv_table};
use crate::fs_ops::{clean_win_verbatim_path, ensure_not_subpath, format_epoch_date, unique_dest_path};
use crate::session::{record_mutation, record_mutation_with_key, DocSnapshot, StudioSession};
use crate::vfs::{
    collect_paths_for_pkz_add, compute_pkz_modified_map, compute_reorder_target,
    detect_file_format_tag, extract_pkz_entries_with_overwrite_check, find_paired_wtp_bytes,
    is_bntx_blob, parse_eff_entry_id, resolve_subblob, revert_pkz_subset,
    split_container_and_leaf, update_matching_wtp_in_dat, write_back_subblob,
};
use crate::wmb_scene::validate_bone_parent;
use pgtools_core::{
    dat::{DatEntry, DatFile},
    pkz::{is_compression_worthwhile, normalize_pkz_virtual_path, PkzArchive},
};
use std::collections::HashMap;
use std::fs;

#[test]
fn bntx_magic_detected_at_both_offsets() {
    assert!(is_bntx_blob(b"BNTX_rest_of_header"));
    let mut wrapped = vec![0u8; 96];
    wrapped[80..84].copy_from_slice(b"BNTX");
    assert!(is_bntx_blob(&wrapped));
    assert!(!is_bntx_blob(b"WTB\0"));
    assert_eq!(detect_file_format_tag("tex.bin", &wrapped), "BNTX");
}

#[test]
fn container_and_leaf_splitting_handles_nested_and_eff_pairs() {
    let (parts, leaf) = split_container_and_leaf("root:dat/dat:2/effdir:0/efffile:1");
    assert_eq!(parts, vec!["root:dat", "dat:2", "effdir:0", "efffile:1"]);
    assert_eq!(leaf, "");

    let (parts2, leaf2) = split_container_and_leaf("pkz:0/dat:1/wmb3:mat:4");
    assert_eq!(parts2, vec!["pkz:0", "dat:1"]);
    assert_eq!(leaf2, "wmb3:mat:4");

    let (parts3, leaf3) = split_container_and_leaf("dat:0/effdir:2");
    assert_eq!(parts3, vec!["dat:0"]);
    assert_eq!(leaf3, "effdir:2");

    let (parts4, leaf4) = split_container_and_leaf("pkzfolder:pl/sub/inner");
    assert!(parts4.is_empty());
    assert_eq!(leaf4, "pkzfolder:pl/sub/inner");

    let (parts5, leaf5) = split_container_and_leaf("dat:0/pkzfolder:pl/sub/inner");
    assert_eq!(parts5, vec!["dat:0"]);
    assert_eq!(leaf5, "pkzfolder:pl/sub/inner");
}

#[test]
fn wta_wtp_pairing_matches_by_stem_single_or_ordinal() {
    let dat_entries = vec![
        DatEntry {
            name: "pl0000.wta".to_string(),
            extension: "wta".to_string(),
            data: vec![1, 2],
        },
        DatEntry {
            name: "sub.wta".to_string(),
            extension: "wta".to_string(),
            data: vec![3, 4],
        },
    ];
    let dtt_entries = vec![
        DatEntry {
            name: "other.wtp".to_string(),
            extension: "wtp".to_string(),
            data: vec![0xAA],
        },
        DatEntry {
            name: "pl0000.wtp".to_string(),
            extension: "wtp".to_string(),
            data: vec![0xBB],
        },
    ];
    // Stem match wins over ordinal
    let matched = find_paired_wtp_bytes("pl0000.wta", 0, 2, &dat_entries, Some(&dtt_entries));
    assert_eq!(matched, Some(vec![0xBB]));
    // Ordinal fallback when stem doesn't match
    let matched_ord = find_paired_wtp_bytes("sub.wta", 1, 2, &dat_entries, Some(&dtt_entries));
    assert_eq!(matched_ord, Some(vec![0xBB]));

    let mut dat_file = DatFile::new(pgtools_core::io::Endian::Big);
    dat_file.entries = dtt_entries;
    assert!(update_matching_wtp_in_dat("pl0000.wta", 0, &mut dat_file, &[0xCC, 0xDD]));
    assert_eq!(dat_file.entries[1].data, vec![0xCC, 0xDD]);
}

#[test]
fn vfs_subblob_resolve_and_write_back_round_trip() {
    let inner_dat = DatFile {
        endian: pgtools_core::io::Endian::Big,
        entries: vec![
            DatEntry {
                name: "a.bxm".to_string(),
                extension: "bxm".to_string(),
                data: b"OLD_PAYLOAD".to_vec(),
            },
            DatEntry {
                name: "b.txt".to_string(),
                extension: "txt".to_string(),
                data: b"KEEP_ME".to_vec(),
            },
        ],
        hash_map: None,
    };
    let snap = DocSnapshot::single(inner_dat.to_bytes());
    let (name, bytes, _, _) = resolve_subblob(&snap, "dat:0", None).expect("resolve dat:0");
    assert_eq!(name, "a.bxm");
    assert_eq!(bytes, b"OLD_PAYLOAD");

    let next_snap = write_back_subblob(&snap, &["dat:0"], b"NEW_PAYLOAD".to_vec(), None, None)
        .expect("write back dat:0");
    let (_, updated_bytes, _, _) = resolve_subblob(&next_snap, "dat:0", None).expect("resolve updated");
    let (_, sibling_bytes, _, _) = resolve_subblob(&next_snap, "dat:1", None).expect("resolve sibling");
    assert_eq!(updated_bytes, b"NEW_PAYLOAD");
    assert_eq!(sibling_bytes, b"KEEP_ME");
}

#[test]
fn csv_codec_preserves_shift_jis_trail_bytes_and_quotes() {
    // Shift-JIS lead byte 0x83 followed by 0x2C (ASCII comma) must not split CSV columns
    let sjis_bytes = [0x83u8, 0x2C, b',', b'A', b'B'];
    let decoded = decode_csv_bytes_auto(&sjis_bytes);
    let (headers, rows) = parse_csv_table(&decoded);
    assert_eq!(headers, vec!["\\x832C".to_string(), "AB".to_string()]);
    assert!(rows.is_empty());

    let serialized = serialize_csv_table(
        &["Name".to_string(), "Note".to_string()],
        &[vec!["Bayo".to_string(), "Hello, \"World\"".to_string()]],
    );
    let (h2, r2) = parse_csv_table(&decode_csv_bytes_auto(&serialized));
    assert_eq!(h2, vec!["Name", "Note"]);
    assert_eq!(r2, vec![vec!["Bayo", "Hello, \"World\""]]);
}

#[test]
fn validate_bone_parent_rejects_self_out_of_range_and_cycles() {
    // 0 -> -1, 1 -> 0, 2 -> 1
    let parents = [-1i16, 0, 1];
    assert!(validate_bone_parent(3, 0, 0, |i| parents[i]).is_err());
    assert!(validate_bone_parent(3, 0, 2, |i| parents[i]).is_err());
    assert!(validate_bone_parent(3, 0, 3, |i| parents[i]).is_err());
    assert_eq!(validate_bone_parent(3, 2, -1, |i| parents[i]).unwrap(), -1);
    assert_eq!(validate_bone_parent(3, 2, 0, |i| parents[i]).unwrap(), 0);
}

#[test]
fn session_undo_coalescing_and_cache_invalidation() {
    let mut sess = StudioSession::default();
    sess.current = Some(DocSnapshot::single(vec![1]));
    sess.insert_thumb_cached("k1".to_string(), ("h1".to_string(), None));
    assert_eq!(sess.tex_thumb_cache.len(), 1);

    record_mutation_with_key(
        &mut sess,
        "Param edit 1",
        DocSnapshot::single(vec![2]),
        Some("mat:0:param".to_string()),
    );
    record_mutation_with_key(
        &mut sess,
        "Param edit 2",
        DocSnapshot::single(vec![3]),
        Some("mat:0:param".to_string()),
    );
    assert_eq!(sess.undo_stack.len(), 1);
    assert_eq!(sess.undo_stack[0].description, "Param edit 2");

    record_mutation(&mut sess, "Separate edit", DocSnapshot::single(vec![4]));
    assert_eq!(sess.undo_stack.len(), 2);

    sess.invalidate_texture_caches();
    assert!(sess.tex_thumb_cache.is_empty());
}

#[test]
fn util_helpers_reorder_hex_and_paths() {
    assert_eq!(
        compute_reorder_target(1, 5, &serde_json::json!({ "direction": "up" })),
        Ok(0)
    );
    assert_eq!(
        compute_reorder_target(0, 5, &serde_json::json!({ "to_index": 3 })),
        Ok(3)
    );
    assert_eq!(parse_eff_entry_id("0x1A"), 26);
    assert_eq!(parse_eff_entry_id("42"), 42);
    assert_eq!(clean_win_verbatim_path(r"\\?\C:\foo\bar".to_string()), r"C:\foo\bar");
    assert_eq!(format_epoch_date(0), "1970-01-01");
    assert!(format_hex_preview(b"ABCD", 16, true).contains("41 42 43 44"));
    let tmp = std::env::temp_dir();
    assert!(ensure_not_subpath(&tmp, &tmp.join("child")).is_err());
    let u = unique_dest_path(&tmp, "nonexistent_pgtools_test.bin");
    assert!(u.ends_with("nonexistent_pgtools_test.bin"));
}


#[test]
fn pkz_worthwhile_threshold_and_force_compression() {
    assert!(is_compression_worthwhile(89, 100, false));
    assert!(!is_compression_worthwhile(90, 100, false));
    assert!(is_compression_worthwhile(95, 100, true));

    // Tiny incompressible payload should fall back to "None" unless forced
    let tiny = b"tiny".to_vec();
    let (label_auto, payload_auto) =
        PkzArchive::compress_payload(&tiny, "ZStandard", Some(3), false, "ZStandard", None)
            .expect("compress auto");
    assert_eq!(label_auto, "None");
    assert_eq!(payload_auto, tiny);

    let (label_forced, payload_forced) =
        PkzArchive::compress_payload(&tiny, "ZStandard", Some(3), true, "ZStandard", None)
            .expect("compress forced");
    assert_eq!(label_forced, "ZStandard");
    assert!(!payload_forced.is_empty());

    // Highly compressible payload should use ZStandard under the 90% rule
    let compressible = vec![b'A'; 4096];
    let (label_comp, payload_comp) = PkzArchive::compress_payload(
        &compressible,
        "ZStandard",
        Some(6),
        false,
        "ZStandard",
        None,
    )
    .expect("compress repetitive");
    assert_eq!(label_comp, "ZStandard");
    assert!(payload_comp.len() < compressible.len() / 2);
}

#[test]
fn pkz_build_from_dir_recursive_and_selective_revert() {
    let dir = std::env::temp_dir().join(format!("pgtools_pkz_test_{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("pl")).unwrap();
    fs::create_dir_all(dir.join("wp")).unwrap();
    fs::write(dir.join("pl").join("pl0000.dat"), vec![b'X'; 2048]).unwrap();
    fs::write(dir.join("wp").join("wp0010.dat"), b"short").unwrap();

    let base_bytes = PkzArchive::build_from_dir(&dir, "ZStandard", 3, false, None)
        .expect("build_from_dir");
    let base_pkz = PkzArchive::from_bytes(&base_bytes).expect("parse base pkz");
    assert_eq!(base_pkz.descriptors.len(), 2);
    assert_eq!(base_pkz.descriptors[0].name, "pl/pl0000.dat");
    assert_eq!(base_pkz.descriptors[0].compression, "ZStandard");
    assert_eq!(base_pkz.descriptors[1].name, "wp/wp0010.dat");
    // "short" is 5 bytes, so 90% rule stores it as "None"
    assert_eq!(base_pkz.descriptors[1].compression, "None");

    // Stage a replacement on index 0 and add a new entry
    let mut replacements = HashMap::new();
    replacements.insert(0, vec![b'Y'; 2048]);
    let added = vec![("ui/hud.dat".to_string(), b"NEW_UI".to_vec())];
    let cur_bytes = base_pkz
        .repack_with_replacements(&base_bytes, &replacements, &added, &[], None, Some("None"), None)
        .expect("repack modified");
    let cur_pkz = PkzArchive::from_bytes(&cur_bytes).expect("parse cur pkz");

    let (mod_map, staged_count) =
        compute_pkz_modified_map(&cur_bytes, &cur_pkz, Some(&base_bytes));
    assert_eq!(staged_count, 2);
    assert_eq!(mod_map.get(&0), Some(&true));
    assert_eq!(mod_map.get(&1), Some(&false));
    assert_eq!(mod_map.get(&2), Some(&true));

    // Revert only index 0 ("pl/pl0000.dat"), leaving added "ui/hud.dat" intact
    let (reverted_0_bytes, rev_count) =
        revert_pkz_subset(&cur_bytes, &base_bytes, Some(&[0]), None).expect("revert index 0");
    assert_eq!(rev_count, 1);
    let rev_pkz = PkzArchive::from_bytes(&reverted_0_bytes).expect("parse reverted pkz");
    assert_eq!(rev_pkz.descriptors.len(), 3);
    assert_eq!(
        rev_pkz.extract_entry(&reverted_0_bytes, 0, None).unwrap(),
        vec![b'X'; 2048]
    );

    // Extract overwrite check
    let out_dir = dir.join("extracted");
    let written1 = extract_pkz_entries_with_overwrite_check(
        &rev_pkz,
        &reverted_0_bytes,
        &[0, 1],
        &out_dir,
        None,
        None,
    )
    .expect("first extract");
    assert_eq!(written1, 2);

    // Second extract without policy must report EXTRACT_OVERWRITE:2
    let err = extract_pkz_entries_with_overwrite_check(
        &rev_pkz,
        &reverted_0_bytes,
        &[0, 1],
        &out_dir,
        None,
        None,
    )
    .unwrap_err();
    assert_eq!(err, "EXTRACT_OVERWRITE:2");

    // Skip policy writes 0 files
    let written_skip = extract_pkz_entries_with_overwrite_check(
        &rev_pkz,
        &reverted_0_bytes,
        &[0, 1],
        &out_dir,
        Some("skip"),
        None,
    )
    .expect("skip extract");
    assert_eq!(written_skip, 0);

    // Recursive folder add collection
    let add_items =
        collect_paths_for_pkz_add(&[dir.join("pl").to_string_lossy().into_owned()], "mod_root")
            .expect("collect folder for add");
    assert_eq!(add_items.len(), 1);
    assert_eq!(add_items[0].0, "mod_root/pl0000.dat");
    assert_eq!(
        normalize_pkz_virtual_path(r"\pl\sub\\pl0000.dat").unwrap(),
        "pl/sub/pl0000.dat"
    );

    let _ = fs::remove_dir_all(&dir);
}
