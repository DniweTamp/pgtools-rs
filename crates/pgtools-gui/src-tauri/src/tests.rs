use crate::formats_cmd::{decode_csv_bytes_auto, format_hex_preview, parse_csv_table, serialize_csv_table};
use crate::fs_ops::{clean_win_verbatim_path, ensure_not_subpath, format_epoch_date, unique_dest_path};
use crate::session::{record_mutation, record_mutation_with_key, DocSnapshot, StudioSession};
use crate::vfs::{
    compute_reorder_target, detect_file_format_tag, find_paired_wtp_bytes, is_bntx_blob,
    parse_eff_entry_id, resolve_subblob, split_container_and_leaf, update_matching_wtp_in_dat,
    write_back_subblob,
};
use crate::wmb_scene::validate_bone_parent;
use pgtools_core::dat::{DatEntry, DatFile};

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

    let mut dat_file = DatFile { entries: dtt_entries };
    assert!(update_matching_wtp_in_dat("pl0000.wta", 0, &mut dat_file, &[0xCC, 0xDD]));
    assert_eq!(dat_file.entries[1].data, vec![0xCC, 0xDD]);
}

#[test]
fn vfs_subblob_resolve_and_write_back_round_trip() {
    let inner_dat = DatFile {
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
    assert_eq!(headers, vec!["\x832C".to_string(), "AB".to_string()]);
    assert!(rows.is_empty());

    let serialized = serialize_csv_table(
        &["Name".to_string(), "Note".to_string()],
        &[vec!["Bayo".to_string(), "Hello, "World"".to_string()]],
    );
    let (h2, r2) = parse_csv_table(&decode_csv_bytes_auto(&serialized));
    assert_eq!(h2, vec!["Name", "Note"]);
    assert_eq!(r2, vec![vec!["Bayo", "Hello, "World""]]);
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
    assert_eq!(compute_reorder_target(1, 5, Some(-1), None), Some(0));
    assert_eq!(compute_reorder_target(0, 5, None, Some(3)), Some(3));
    assert_eq!(parse_eff_entry_id("0x1A"), 26);
    assert_eq!(parse_eff_entry_id("42"), 42);
    assert_eq!(clean_win_verbatim_path(r"\?\C:ooar".to_string()), r"C:ooar");
    assert_eq!(format_epoch_date(0), "1970-01-01");
    assert!(format_hex_preview(b"ABCD", 16, true).contains("41 42 43 44"));
    let tmp = std::env::temp_dir();
    assert!(ensure_not_subpath(&tmp, &tmp.join("child")).is_err());
    let u = unique_dest_path(&tmp, "nonexistent_pgtools_test.bin");
    assert!(u.ends_with("nonexistent_pgtools_test.bin"));
}
