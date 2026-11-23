//! Round-trip and rebuild tests for the 010-style struct formats that support
//! add/remove/reorder in the studio editors (COS, ATT, OBI, MSQ).
//!
//! These live in an integration target rather than `mod tests` inside `bt_formats.rs`
//! because the crate's unit-test target is currently broken by unrelated stale fixtures
//! in `texture_hash.rs` (it references `WmbHeader`/`Wmb3Material`/`WtbEntry` fields that no
//! longer exist), which would block running these.

use pgtools_core::bone::Vec3;
use pgtools_core::bt_formats::{AttFile, AttPoint, CosEntry, CosFile, MsqFile, ObiEntry, ObiFile};
use pgtools_core::io::Endian;

fn cos_entry(name: &str, seed: f32) -> CosEntry {
    CosEntry {
        name: name.to_string(),
        colors: [[seed, seed + 0.1, seed + 0.2]; 7],
        src_index: -1,
    }
}

/// 64-byte fixed name field padded with `filler`, the way the retail files store it.
fn cos_name_field(name: &str, filler: u8) -> [u8; 0x40] {
    let mut field = [filler; 0x40];
    let bytes = name.as_bytes();
    let copy_len = bytes.len().min(0x3f);
    field[..copy_len].copy_from_slice(&bytes[..copy_len]);
    field[copy_len] = 0;
    field
}

#[test]
fn cos_zero_entries_round_trips() {
    let cos = CosFile {
        version: 1,
        entries: Vec::new(),
    };
    let bytes = cos.to_bytes();
    assert_eq!(bytes.len(), 16);
    let back = CosFile::from_bytes(&bytes).expect("zero-entry COS must parse");
    assert!(back.entries.is_empty());
    assert_eq!(back.version, 1);
}

#[test]
fn cos_entries_round_trip_and_report_size() {
    let cos = CosFile {
        version: 3,
        entries: vec![cos_entry("Bayonetta", 0.0), cos_entry("Jeanne", 0.5)],
    };
    let bytes = cos.to_bytes();
    assert_eq!(bytes.len(), 16 + 2 * CosFile::ENTRY_LEN);
    let back = CosFile::from_bytes(&bytes).unwrap();
    assert_eq!(back.version, 3);
    assert_eq!(back.entries.len(), 2);
    assert_eq!(back.entries[0].name, "Bayonetta");
    assert_eq!(back.entries[1].colors, cos.entries[1].colors);
    assert_eq!(back.entries[1].src_index, 1);
}

#[test]
fn cos_name_longer_than_field_is_truncated() {
    let cos = CosFile {
        version: 1,
        entries: vec![cos_entry(&"x".repeat(80), 0.0)],
    };
    let back = CosFile::from_bytes(&cos.to_bytes()).unwrap();
    assert_eq!(back.entries[0].name.len(), 0x40);
}

/// Mirror of the `CosPres_*.cos` files shipped in Bayonetta 3's `core.dat`: an outer header with
/// `count = 0` whose offset points at a second header carrying the real count and record offset.
/// Name fields are padded with `0xFE`, as the retail files are.
fn wrapped_cos_fixture(names: &[&str]) -> Vec<u8> {
    let date = 0x2021_0910u32;
    let mut out = Vec::new();
    out.extend_from_slice(&0u32.to_le_bytes()); // outer size, patched below
    out.extend_from_slice(&date.to_le_bytes());
    out.extend_from_slice(&16u32.to_le_bytes()); // outer: nested header at 16
    out.extend_from_slice(&0u32.to_le_bytes()); // outer count stays 0
    out.extend_from_slice(&0u32.to_le_bytes()); // inner size, patched below
    out.extend_from_slice(&date.to_le_bytes());
    out.extend_from_slice(&(names.len() as u32).to_le_bytes());
    out.extend_from_slice(&16u32.to_le_bytes()); // inner: records at 16 + 16
    for (i, name) in names.iter().enumerate() {
        let seed = i as f32 * 0.25;
        for _ in 0..7 {
            out.extend_from_slice(&seed.to_le_bytes());
            out.extend_from_slice(&(seed + 0.1).to_le_bytes());
            out.extend_from_slice(&(seed + 0.2).to_le_bytes());
        }
        out.extend_from_slice(&cos_name_field(name, 0xFE));
    }
    let inner_size = (out.len() - 16) as u32;
    let total = out.len() as u32;
    out[0..4].copy_from_slice(&total.to_le_bytes());
    out[16..20].copy_from_slice(&inner_size.to_le_bytes());
    out
}

#[test]
fn cos_wrapped_layout_is_detected() {
    let data = wrapped_cos_fixture(&["Bayonetta", "Jeanne", "Viola"]);
    assert_eq!(data.len(), 32 + 3 * CosFile::ENTRY_LEN);

    let cos = CosFile::from_bytes(&data).expect("wrapped COS must parse");
    assert_eq!(cos.entries.len(), 3);
    assert_eq!(cos.entries[0].name, "Bayonetta");
    assert_eq!(cos.entries[2].name, "Viola");
    assert_eq!(cos.version, 0x2021_0910);
    // untouched round-trip is byte-identical, filler included
    assert_eq!(cos.rebuild(&data), data);
}

#[test]
fn cos_rebuild_keeps_wrapped_layout_and_filler() {
    let data = wrapped_cos_fixture(&["Bayonetta", "Jeanne"]);
    let records_offset = 32usize;
    let mut cos = CosFile::from_bytes(&data).unwrap();
    cos.entries[0].colors[0] = [0.5, 0.25, 0.125];

    let rebuilt = cos.rebuild(&data);
    assert_eq!(rebuilt.len(), data.len(), "wrapped file keeps its preamble");
    // outer count stays 0, inner count follows the entry list
    assert_eq!(&rebuilt[12..16], &0u32.to_le_bytes());
    assert_eq!(&rebuilt[24..28], &2u32.to_le_bytes());
    assert_eq!(
        u32::from_le_bytes(rebuilt[0..4].try_into().unwrap()) as usize,
        rebuilt.len()
    );

    // Only the edited colour may change: name filler must survive untouched rows.
    let diffs: Vec<usize> = data
        .iter()
        .zip(rebuilt.iter())
        .enumerate()
        .filter(|(_, (a, b))| a != b)
        .map(|(i, _)| i)
        .collect();
    assert!(!diffs.is_empty(), "edited colour should change bytes");
    assert!(
        diffs
            .iter()
            .all(|&i| i >= records_offset && i < records_offset + 7 * 12),
        "unexpected byte changes outside the edited colour: {diffs:?}"
    );

    let back = CosFile::from_bytes(&rebuilt).unwrap();
    assert_eq!(back.entries[0].colors, cos.entries[0].colors);
    assert_eq!(back.entries[1].name, "Jeanne");
}

#[test]
fn cos_rename_writes_fresh_name_field() {
    let data = wrapped_cos_fixture(&["Bayonetta", "Jeanne"]);
    let mut cos = CosFile::from_bytes(&data).unwrap();
    cos.entries[1].name = "Viola".to_string();

    let rebuilt = cos.rebuild(&data);
    let back = CosFile::from_bytes(&rebuilt).unwrap();
    assert_eq!(back.entries[1].name, "Viola");

    // A rewritten field is NUL-padded, not filled with the original 0xFE.
    let name_off = 32 + CosFile::ENTRY_LEN + 7 * 12;
    assert_eq!(rebuilt[name_off + 5], 0x00);
    assert_eq!(rebuilt[name_off + 6], 0x00);
}

#[test]
fn cos_rebuild_handles_growth_in_wrapped_layout() {
    let data = wrapped_cos_fixture(&["Bayonetta"]);
    let mut cos = CosFile::from_bytes(&data).unwrap();
    cos.entries.push(cos_entry("Jeanne", 0.5));

    let rebuilt = cos.rebuild(&data);
    assert_eq!(rebuilt.len(), 32 + 2 * CosFile::ENTRY_LEN);
    assert_eq!(&rebuilt[24..28], &2u32.to_le_bytes());
    // The original row keeps its filler bytes.
    assert_eq!(
        &rebuilt[32 + 7 * 12..32 + CosFile::ENTRY_LEN],
        &data[32 + 7 * 12..32 + CosFile::ENTRY_LEN]
    );

    let back = CosFile::from_bytes(&rebuilt).unwrap();
    assert_eq!(back.entries.len(), 2);
    assert_eq!(back.entries[1].name, "Jeanne");
}

#[test]
fn cos_wrapped_empty_then_regrown_stays_wrapped() {
    let data = wrapped_cos_fixture(&["Bayonetta"]);
    let mut cos = CosFile::from_bytes(&data).unwrap();
    cos.entries.clear();

    let emptied = cos.rebuild(&data);
    assert_eq!(emptied.len(), 32);
    assert_eq!(&emptied[12..16], &0u32.to_le_bytes());
    assert_eq!(&emptied[24..28], &0u32.to_le_bytes());

    let mut back = CosFile::from_bytes(&emptied).expect("empty wrapped COS must parse");
    assert!(back.entries.is_empty());

    back.entries.push(cos_entry("Viola", 0.25));
    let regrown = back.rebuild(&emptied);
    assert_eq!(regrown.len(), 32 + CosFile::ENTRY_LEN);
    assert_eq!(&regrown[12..16], &0u32.to_le_bytes());
    assert_eq!(&regrown[24..28], &1u32.to_le_bytes());

    let final_cos = CosFile::from_bytes(&regrown).unwrap();
    assert_eq!(final_cos.entries.len(), 1);
    assert_eq!(final_cos.entries[0].name, "Viola");
}

#[test]
fn cos_flat_file_is_not_wrapped_on_rebuild() {
    let cos = CosFile {
        version: 7,
        entries: vec![cos_entry("Bayonetta", 0.0)],
    };
    let flat = cos.to_bytes();
    assert_eq!(cos.rebuild(&flat), flat);
}

#[test]
fn att_add_and_remove_round_trip() {
    let mut att = AttFile {
        endian: Endian::Little,
        points: vec![AttPoint {
            source_bone: 1,
            target_bone: 2,
            option: 0,
        }],
    };
    att.points.push(AttPoint {
        source_bone: 5,
        target_bone: 6,
        option: 7,
    });
    let bytes = att.to_bytes();
    assert_eq!(bytes.len(), 4 + 2 * 12);
    assert_eq!(AttFile::from_bytes(&bytes).unwrap().points.len(), 2);

    att.points.remove(0);
    let back = AttFile::from_bytes(&att.to_bytes()).unwrap();
    assert_eq!(
        back.points,
        vec![AttPoint {
            source_bone: 5,
            target_bone: 6,
            option: 7
        }]
    );
}

// --- OBI: reserved bytes must travel with the record that owns them ---

fn obi_fixture() -> Vec<u8> {
    let mut data = vec![0u8; ObiFile::HEADER_LEN];
    data[0..4].copy_from_slice(&0xDEAD_BEEFu32.to_le_bytes());
    data[4..8].copy_from_slice(&2i32.to_le_bytes());
    for i in 0..2usize {
        let mut rec = vec![0u8; ObiFile::RECORD_LEN];
        rec[0..4].copy_from_slice(&((i as u32) + 1).to_le_bytes());
        // reserved bytes the editor does not model
        rec[100] = 0xA0 + i as u8;
        rec[101] = 0xB0 + i as u8;
        data.extend_from_slice(&rec);
    }
    data.extend_from_slice(&[0xAA, 0xBB, 0xCC, 0xDD]); // trailing bytes
    data
}

#[test]
fn obi_reorder_moves_reserved_bytes_with_the_record() {
    let original = obi_fixture();
    let mut obi = ObiFile::from_bytes(&original).unwrap();
    obi.entries.swap(0, 1);

    let rebuilt = obi.rebuild(&original);
    let back = ObiFile::from_bytes(&rebuilt).unwrap();
    assert_eq!(back.entries[0].object_id, 2);
    assert_eq!(back.entries[1].object_id, 1);
    assert_eq!(rebuilt[ObiFile::HEADER_LEN + 100], 0xA1);
    assert_eq!(rebuilt[ObiFile::HEADER_LEN + 101], 0xB1);
    assert_eq!(&rebuilt[rebuilt.len() - 4..], &[0xAA, 0xBB, 0xCC, 0xDD]);
    assert_eq!(&rebuilt[4..8], &2i32.to_le_bytes());
}

#[test]
fn obi_remove_and_append_renumbers() {
    let original = obi_fixture();
    let mut obi = ObiFile::from_bytes(&original).unwrap();
    obi.entries.remove(0);
    obi.entries.push(ObiEntry {
        object_id: 3,
        object_name: ObiFile::format_object_id(3),
        flags: 0,
        position: Vec3 {
            x: 1.0,
            y: 2.0,
            z: 3.0,
        },
        orientation: Vec3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        },
        scale: Vec3 {
            x: 1.0,
            y: 1.0,
            z: 1.0,
        },
        difficulty_selector: 2,
        src_index: -1,
    });

    let rebuilt = obi.rebuild(&original);
    let back = ObiFile::from_bytes(&rebuilt).unwrap();
    assert_eq!(back.entries.len(), 2);
    assert_eq!(back.entries[0].object_id, 2);
    assert_eq!(back.entries[1].object_id, 3);
    assert_eq!(back.entries[1].difficulty_selector, 2);
    assert_eq!(rebuilt[ObiFile::HEADER_LEN + 100], 0xA1);
    assert_eq!(
        rebuilt[ObiFile::HEADER_LEN + ObiFile::RECORD_LEN + 100],
        0x00
    );
    assert_eq!(&rebuilt[4..8], &2i32.to_le_bytes());
}

// --- MSQ ---

fn msq_fixture() -> Vec<u8> {
    let mut data = vec![0u8; MsqFile::HEADER_LEN];
    data[0..4].copy_from_slice(&2u32.to_le_bytes());
    data[4..8].copy_from_slice(&8u32.to_le_bytes());
    for i in 0..2u32 {
        let mut rec = vec![0u8; MsqFile::RECORD_LEN];
        let name = format!("ev{i}");
        rec[..name.len()].copy_from_slice(name.as_bytes());
        rec[0x20..0x24].copy_from_slice(&(i + 10).to_le_bytes());
        rec[0x24..0x28].copy_from_slice(&(i + 20).to_le_bytes());
        rec[0x28..0x2C].copy_from_slice(&(i + 30).to_le_bytes());
        rec[0x30..0x34].copy_from_slice(&(i + 40).to_le_bytes());
        rec[0x4A] = 0xE0 + i as u8;
        data.extend_from_slice(&rec);
    }
    data.extend_from_slice(&[0x11, 0x22]);
    data
}

#[test]
fn msq_reorder_carries_reserved_tail() {
    let original = msq_fixture();
    let mut msq = MsqFile::from_bytes(&original).unwrap();
    msq.events.swap(0, 1);

    let rebuilt = msq.rebuild(&original);
    let back = MsqFile::from_bytes(&rebuilt).unwrap();
    assert_eq!(back.events[0].name, "ev1");
    assert_eq!(back.events[0].id, 11);
    assert_eq!(rebuilt[MsqFile::HEADER_LEN + 0x4A], 0xE1);
    assert_eq!(&rebuilt[rebuilt.len() - 2..], &[0x11, 0x22]);
    assert_eq!(&rebuilt[0..4], &2u32.to_le_bytes());
}

#[test]
fn msq_name_rewrite_clears_stale_bytes() {
    let original = msq_fixture();
    let mut msq = MsqFile::from_bytes(&original).unwrap();
    msq.events[0].name = "a".to_string();

    let rebuilt = msq.rebuild(&original);
    let back = MsqFile::from_bytes(&rebuilt).unwrap();
    assert_eq!(back.events[0].name, "a");
    assert_eq!(rebuilt[MsqFile::HEADER_LEN + 1], 0);
    assert_eq!(rebuilt[MsqFile::HEADER_LEN + 2], 0);
}
