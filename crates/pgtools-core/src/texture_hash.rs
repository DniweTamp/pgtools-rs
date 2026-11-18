use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::shader_db::{list_core_pretextures, lookup_b2_shader};
use crate::wmb::WmbFile;
use crate::wmb3::Wmb3File;
use crate::wtb::WtbFile;

/// Mask for PlatinumGames 31-bit texture hashes (`CRC32 & 0x7FFF_FFFF`, top byte `& 0x7F`).
pub const CRC32_7F_MASK: u32 = 0x7FFF_FFFF;

/// Returns `true` when `id` has bit 31 cleared (`(id & 0x8000_0000) == 0`).
#[inline]
pub fn is_valid_crc32_7f(id: u32) -> bool {
    (id & 0x8000_0000) == 0
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextureHashRemapEntry {
    pub old_id: u32,
    pub new_id: u32,
    pub wmb_occurrences: usize,
    pub wtb_occurrences: usize,
    pub collision_probed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextureHashReport {
    pub format: String,
    pub skipped_b1: bool,
    pub remaps: Vec<TextureHashRemapEntry>,
    pub wmb_modified: bool,
    pub wtb_modified_indices: Vec<usize>,
}

fn collect_wmb_b2_texture_ids(wmb: &WmbFile) -> Vec<u32> {
    let mut ids = Vec::new();
    for &(id, _) in &wmb.tex_infos {
        ids.push(id);
    }
    for (idx, mat) in wmb.materials.iter().enumerate() {
        if let Some((_, lay)) = mat.shader_name.as_deref().and_then(lookup_b2_shader) {
            for t in &lay.textures {
                if let Some(&tid) = mat.material_data.get(t.slot) {
                    ids.push(tid);
                }
            }
        } else {
            let dec = mat.decode_with_hint(idx, true);
            for f in &dec.fields {
                if let Some(tid) = f.sampler_tex_id {
                    ids.push(tid);
                }
            }
        }
    }
    ids
}

fn collect_wmb3_texture_ids(wmb3: &Wmb3File) -> Vec<u32> {
    let mut ids = Vec::new();
    for mat in &wmb3.materials {
        for &(_, tid) in &mat.textures {
            ids.push(tid);
        }
    }
    ids
}

fn plan_texture_hash_remap(
    wmb_ids: &[u32],
    wtb_banks: &[&WtbFile],
    pretexture_game: &str,
) -> (BTreeMap<u32, (u32, bool)>, BTreeMap<u32, usize>, BTreeMap<u32, usize>) {
    let mut wmb_counts: BTreeMap<u32, usize> = BTreeMap::new();
    let mut wtb_counts: BTreeMap<u32, usize> = BTreeMap::new();
    let mut used_valid_ids: BTreeSet<u32> = BTreeSet::new();
    let mut wtb_all_ids: BTreeSet<u32> = BTreeSet::new();
    let mut invalid_ids: BTreeSet<u32> = BTreeSet::new();

    for &id in wmb_ids {
        *wmb_counts.entry(id).or_insert(0) += 1;
        if is_valid_crc32_7f(id) {
            used_valid_ids.insert(id);
        } else {
            invalid_ids.insert(id);
        }
    }

    for bank in wtb_banks {
        for entry in &bank.entries {
            if let Some(tid) = entry.texture_id {
                *wtb_counts.entry(tid).or_insert(0) += 1;
                wtb_all_ids.insert(tid);
                if is_valid_crc32_7f(tid) {
                    used_valid_ids.insert(tid);
                }
            }
        }
    }

    for pre in list_core_pretextures(pretexture_game) {
        if is_valid_crc32_7f(pre.id) {
            used_valid_ids.insert(pre.id);
        }
    }

    let mut plan: BTreeMap<u32, (u32, bool)> = BTreeMap::new();
    for old_id in invalid_ids {
        let base = old_id & CRC32_7F_MASK;
        let wtb_has_old = wtb_all_ids.contains(&old_id);

        // If the paired WTA/WTB (or pretextures) already has `base` and does not have `old_id`,
        // point the WMB directly to `base` without probing or modifying the WTA/WTB.
        if !wtb_has_old && used_valid_ids.contains(&base) && base != 0 {
            plan.insert(old_id, (base, false));
            continue;
        }

        let mut cand = if base == 0 { 1 } else { base };
        let mut probed = false;
        while used_valid_ids.contains(&cand) || cand == 0 {
            cand = cand.wrapping_add(1) & CRC32_7F_MASK;
            probed = true;
        }
        used_valid_ids.insert(cand);
        plan.insert(old_id, (cand, probed));
    }

    (plan, wmb_counts, wtb_counts)
}

/// Check and optionally remap invalid (`bit 31 == 1`) texture hashes in a Bayonetta 2 `WmbFile`
/// and any paired `WtbFile` banks. Bayonetta 1 WMBs are skipped untouched because B1 uses
/// index-based textures and `0x8000000x` pretexture IDs.
pub fn remap_wmb_texture_hashes(
    wmb: &mut WmbFile,
    wtb_banks: &mut [&mut WtbFile],
    dry_run: bool,
) -> TextureHashReport {
    if !wmb.is_bayo2() {
        return TextureHashReport {
            format: "WMB (Bayonetta 1 - skipped)".to_string(),
            skipped_b1: true,
            remaps: Vec::new(),
            wmb_modified: false,
            wtb_modified_indices: Vec::new(),
        };
    }

    let wmb_ids = collect_wmb_b2_texture_ids(wmb);
    let bank_refs: Vec<&WtbFile> = wtb_banks.iter().map(|b| &**b).collect();
    let (plan, wmb_counts, wtb_counts) = plan_texture_hash_remap(&wmb_ids, &bank_refs, "b2");

    let remaps: Vec<TextureHashRemapEntry> = plan
        .iter()
        .map(|(&old_id, &(new_id, collision_probed))| TextureHashRemapEntry {
            old_id,
            new_id,
            wmb_occurrences: wmb_counts.get(&old_id).copied().unwrap_or(0),
            wtb_occurrences: wtb_counts.get(&old_id).copied().unwrap_or(0),
            collision_probed,
        })
        .collect();

    if dry_run || plan.is_empty() {
        return TextureHashReport {
            format: "WMB (Bayonetta 2)".to_string(),
            skipped_b1: false,
            remaps,
            wmb_modified: false,
            wtb_modified_indices: Vec::new(),
        };
    }

    let mut wmb_modified = false;
    for (id, _) in &mut wmb.tex_infos {
        if let Some(&(new_id, _)) = plan.get(id) {
            *id = new_id;
            wmb_modified = true;
        }
    }

    for (idx, mat) in wmb.materials.iter_mut().enumerate() {
        if let Some((_, lay)) = mat.shader_name.as_deref().and_then(lookup_b2_shader) {
            for t in &lay.textures {
                if let Some(slot_val) = mat.material_data.get_mut(t.slot) {
                    if let Some(&(new_id, _)) = plan.get(slot_val) {
                        *slot_val = new_id;
                        wmb_modified = true;
                    }
                }
            }
        } else {
            let mut dec = mat.decode_with_hint(idx, true);
            let mut mat_changed = false;
            for f in &mut dec.fields {
                if let Some(tid) = f.sampler_tex_id {
                    if let Some(&(new_id, _)) = plan.get(&tid) {
                        f.sampler_tex_id = Some(new_id);
                        mat_changed = true;
                    }
                }
            }
            if mat_changed {
                mat.apply_decoded(&dec);
                wmb_modified = true;
            }
        }
    }

    let mut wtb_modified_indices = Vec::new();
    for (bank_idx, bank) in wtb_banks.iter_mut().enumerate() {
        let mut bank_changed = false;
        for entry in &mut bank.entries {
            if let Some(tid) = entry.texture_id {
                if let Some(&(new_id, _)) = plan.get(&tid) {
                    entry.texture_id = Some(new_id);
                    bank_changed = true;
                }
            }
        }
        if bank_changed {
            wtb_modified_indices.push(bank_idx);
        }
    }

    TextureHashReport {
        format: "WMB (Bayonetta 2)".to_string(),
        skipped_b1: false,
        remaps,
        wmb_modified,
        wtb_modified_indices,
    }
}

/// Check and optionally remap invalid (`bit 31 == 1`) texture hashes in a `Wmb3File`
/// (Bayonetta 3 / NieR:Automata) and any paired `WtbFile` banks.
pub fn remap_wmb3_texture_hashes(
    wmb3: &mut Wmb3File,
    wtb_banks: &mut [&mut WtbFile],
    dry_run: bool,
) -> TextureHashReport {
    let wmb_ids = collect_wmb3_texture_ids(wmb3);
    let bank_refs: Vec<&WtbFile> = wtb_banks.iter().map(|b| &**b).collect();
    let (plan, wmb_counts, wtb_counts) = plan_texture_hash_remap(&wmb_ids, &bank_refs, "b3");

    let remaps: Vec<TextureHashRemapEntry> = plan
        .iter()
        .map(|(&old_id, &(new_id, collision_probed))| TextureHashRemapEntry {
            old_id,
            new_id,
            wmb_occurrences: wmb_counts.get(&old_id).copied().unwrap_or(0),
            wtb_occurrences: wtb_counts.get(&old_id).copied().unwrap_or(0),
            collision_probed,
        })
        .collect();

    if dry_run || plan.is_empty() {
        return TextureHashReport {
            format: "WMB3 (Bayonetta 3 / NieR)".to_string(),
            skipped_b1: false,
            remaps,
            wmb_modified: false,
            wtb_modified_indices: Vec::new(),
        };
    }

    let mut wmb_modified = false;
    for mat in &mut wmb3.materials {
        for (_, tid) in &mut mat.textures {
            if let Some(&(new_id, _)) = plan.get(tid) {
                *tid = new_id;
                wmb_modified = true;
            }
        }
    }

    let mut wtb_modified_indices = Vec::new();
    for (bank_idx, bank) in wtb_banks.iter_mut().enumerate() {
        let mut bank_changed = false;
        for entry in &mut bank.entries {
            if let Some(tid) = entry.texture_id {
                if let Some(&(new_id, _)) = plan.get(&tid) {
                    entry.texture_id = Some(new_id);
                    bank_changed = true;
                }
            }
        }
        if bank_changed {
            wtb_modified_indices.push(bank_idx);
        }
    }

    TextureHashReport {
        format: "WMB3 (Bayonetta 3 / NieR)".to_string(),
        skipped_b1: false,
        remaps,
        wmb_modified,
        wtb_modified_indices,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::io::Endian;
    use crate::wmb::{WmbHeader, WmbMaterial};
    use crate::wmb3::Wmb3Material;
    use crate::wtb::{WtbEntry, WtbGameProfile};

    fn make_wtb(ids: &[u32]) -> WtbFile {
        WtbFile {
            endian: Endian::Little,
            is_split_wta_wtp: true,
            game_profile: WtbGameProfile::Bayo3SwitchXt1,
            entries: ids
                .iter()
                .map(|&id| WtbEntry {
                    texture_id: Some(id),
                    flags: 0x20,
                    extension: ".xt1".to_string(),
                    name: format!("0x{id:08X}.xt1"),
                    data: vec![0; 16],
                })
                .collect(),
        }
    }

    #[test]
    fn remaps_b2_tex_infos_materials_and_wtb_with_collision_probe() {
        let mut wmb = WmbFile {
            endian: Endian::Little,
            header: WmbHeader {
                magic: *b"WMB\0",
                version: 0x0001_0000,
                u_a: 0,
                vertex_format: 0,
                u_b: 0,
                u_c: 0,
                bounding_box: [0.0; 6],
                offset_positions: 0,
                offset_vertexes: 0,
                offset_bone_hierarchy: 0,
                offset_bone_relative_positions: 0,
                offset_bone_positions: 0,
                offset_bone_index_translate_table: 0,
                offset_u_j: 0,
                offset_bone_symmetries: 0,
                offset_bone_flags: 0,
                offset_shader_names: 0x100,
                offset_tex_infos: 0x200,
                offset_materials: 0,
                offset_meshes: 0,
                u_k: 0,
                u_l: 0,
            },
            positions: vec![],
            vertexes: vec![],
            bone_hierarchy: vec![],
            bone_relative_positions: vec![],
            bone_positions: vec![],
            bone_index_translate_table: Default::default(),
            u_j_raw: None,
            bone_symmetries: vec![],
            bone_flags: vec![],
            tex_infos: vec![(0x8123_4567, 1), (0x0123_4567, 2), (0x9000_0001, 3)],
            materials: vec![WmbMaterial {
                mat_type: 0,
                flag: 0,
                shader_name: Some("CNS00_00X".to_string()),
                material_data: vec![0x8123_4567, 0x0123_4567, 0x9000_0001],
            }],
            meshes: vec![],
        };
        let mut wtb = make_wtb(&[0x8123_4567, 0x0123_4567, 0x9000_0001]);
        let mut banks = [&mut wtb];

        let report = remap_wmb_texture_hashes(&mut wmb, &mut banks, false);
        assert!(!report.skipped_b1);
        assert!(report.wmb_modified);
        assert_eq!(report.wtb_modified_indices, vec![0]);
        assert_eq!(report.remaps.len(), 2);

        // 0x8123_4567 collides with 0x0123_4567 -> probed to 0x0123_4568
        assert_eq!(wmb.tex_infos[0].0, 0x0123_4568);
        assert_eq!(wmb.tex_infos[1].0, 0x0123_4567);
        assert_eq!(wmb.tex_infos[2].0, 0x1000_0001);
        assert_eq!(wtb.entries[0].texture_id, Some(0x0123_4568));
        assert_eq!(wtb.entries[1].texture_id, Some(0x0123_4567));
        assert_eq!(wtb.entries[2].texture_id, Some(0x1000_0001));
    }

    #[test]
    fn remaps_wmb3_and_keeps_wtb_untouched_when_masked_id_already_in_wtb() {
        let mut wmb3 = Wmb3File {
            version: 0x2016_0116,
            u_a: 0,
            flags: 0,
            u_c: 0,
            bounding_box: [0.0; 6],
            bones: vec![],
            bone_index_translate_table: Default::default(),
            raw_btt: vec![],
            vertex_groups: vec![],
            batches: vec![],
            lods: vec![],
            col_tree_nodes: vec![],
            bone_map: vec![],
            bone_sets: vec![],
            materials: vec![Wmb3Material {
                version: [0; 4],
                name: "mat0".to_string(),
                effect_name: "PBS00".to_string(),
                technique_name: "Default".to_string(),
                u_a: 0,
                textures: vec![
                    ("g_AlbedoMap".to_string(), 0xABCD_EF01),
                    ("g_NormalMap".to_string(), 0x7FD4_929A),
                ],
                parameter_groups: vec![],
                variables: vec![],
            }],
            meshes: vec![],
            mesh_material_pairs: vec![],
            world_data: vec![],
            tail: [0, 0],
        };
        let mut wtb = make_wtb(&[0x2BCD_EF01]);
        let mut banks = [&mut wtb];

        let report = remap_wmb3_texture_hashes(&mut wmb3, &mut banks, false);
        assert!(report.wmb_modified);
        assert!(report.wtb_modified_indices.is_empty());
        assert_eq!(wmb3.materials[0].textures[0].1, 0x2BCD_EF01);
        assert_eq!(wmb3.materials[0].textures[1].1, 0x7FD4_929A);
        assert_eq!(wtb.entries[0].texture_id, Some(0x2BCD_EF01));
    }
}
