//! `WMB3` (*Bayonetta 3* `0x20200824` / *NieR:Automata* `0x20160116` / *Astral Chain* / *MGRR*)
//! 3D model parser, byte-accurate serializer (`to_bytes()`), T-pose skinning baker,
//! material parameter decoder (`shader_params_bayo3.json`), and OBJ exporter.
//!
//! Matches both `Bayonetta2Blender2Bayonetta-v24.9.3` (`wmb/wmb3/wmb3_format.py` and
//! `wmb/wmb3/wmb3_materials.py`) and `bayonetta_tools` (`lib/bayonetta/wmb3.rb`).

use crate::bone::{BoneIndexTranslateTable, Vec3};
use crate::databases::lookup_wmb3_vertex_layout_with_strides;
use crate::io::{align, checked_count, BinReader, BinWriter, Endian};
use crate::linalg::{Matrix, Vector};
use crate::shader_db::{lookup_b3_shader_group_params, lookup_pretexture_label};
use crate::wmb::WmbVertex;
use crate::{PgError, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Wmb3InfoPair {
    pub offset: u32,
    pub number: u32,
}

impl Wmb3InfoPair {
    pub fn read(r: &mut BinReader) -> Result<Self> {
        Ok(Self {
            offset: r.read_u32()?,
            number: r.read_u32()?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Wmb3Bone {
    pub id: i16,
    pub parent_index: i16,
    pub local_position: Vec3,
    pub local_rotation: Vec3,
    pub local_scale: Vec3,
    pub position: Vec3,
    pub rotation: Vec3,
    pub scale: Vec3,
    pub t_position: Vec3,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Wmb3VertexGroup {
    pub offset_vertexes: u32,
    pub offset_vertexes_ex_data: u32,
    pub u_a: u32,
    pub u_b: u32,
    pub vertex_size: u32,
    pub vertex_ex_data_size: u32,
    pub u_c: u32,
    pub u_d: u32,
    pub num_vertexes: u32,
    pub vertex_flags: u32,
    pub offset_indices: u32,
    pub num_indices: u32,
    pub vertexes: Vec<WmbVertex>,
    pub indices: Vec<u32>,
    #[serde(skip)]
    pub raw_vertex_bytes: Vec<u8>,
    #[serde(skip)]
    pub raw_ex_bytes: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Wmb3Batch {
    pub vertex_group_index: u32,
    pub bone_set_index: i32,
    pub vertex_start: u32,
    pub index_start: u32,
    pub num_vertexes: u32,
    pub num_indices: u32,
    pub num_primitives: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Wmb3LodBatchInfo {
    pub vertex_group_index: u32,
    pub mesh_index: u32,
    pub material_index: u32,
    pub call_tree_node_index: i32,
    pub mesh_material_index: u32,
    pub u_b: i32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Wmb3Lod {
    pub name: String,
    pub lod_level: i32,
    pub batch_start: u32,
    pub batch_infos: Vec<Wmb3LodBatchInfo>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Wmb3NamedParameter {
    pub group_index: usize,
    pub param_index: usize,
    pub key: String,
    pub name: String,
    pub value: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Wmb3TextureSlotView {
    pub slot_index: usize,
    pub name: String,
    pub texture_id: u32,
    pub texture_id_hex: String,
    pub pretexture_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Wmb3Material {
    pub date: [u16; 4],
    pub name: String,
    pub shader_name: String,
    pub technique_name: String,
    pub u_a: u32,
    pub textures: Vec<(String, u32)>,
    pub parameter_groups: Vec<(i32, Vec<f32>)>,
    pub variables: Vec<(String, f32)>,
}

impl Wmb3Material {
    /// Return a list of named parameters (combining `parameter_groups` 0..3 with `shader_params_bayo3.json`).
    pub fn named_parameters(&self) -> Vec<Wmb3NamedParameter> {
        let mut out = Vec::new();
        for (g_idx, (_, vals)) in self.parameter_groups.iter().enumerate() {
            let grp_names = lookup_b3_shader_group_params(&self.shader_name, g_idx).unwrap_or(&[]);
            for (p_idx, &v) in vals.iter().enumerate() {
                let db_name = grp_names
                    .get(p_idx)
                    .map(|s| s.as_str())
                    .filter(|&s| !s.is_empty() && s != "Unknown");
                let default_group_name = match (g_idx, p_idx) {
                    (1, 0) => Some("g_OffShadowCast"),
                    (1, 1) => Some("g_FrustumClipID"),
                    (2, 0) => Some("g_ChoiceCubeMapt"),
                    (2, 1) => Some("g_ChoiceCubeMapNo"),
                    (2, 2) => Some("g_OffReceiveLight"),
                    (2, 3) => Some("g_LocalSortNo"),
                    (3, 0) => Some("g_TexSwapId_AlbedoMap"),
                    (3, 1) => Some("g_TexSwapId_NormalMap"),
                    (3, 2) => Some("g_TexSwapId_NormalMap2"),
                    (3, 5) => Some("g_TexSwapId_MaskMap"),
                    (3, 8) => Some("g_TexSwapId_DetailNormalMap"),
                    _ => None,
                };
                let resolved_opt = db_name.or(default_group_name);
                let (raw_name, has_known_name) = if let Some(known) = resolved_opt {
                    (known.to_string(), true)
                } else if g_idx == 0 {
                    (
                        grp_names
                            .get(p_idx)
                            .cloned()
                            .unwrap_or_else(|| format!("Param_{p_idx:02}")),
                        p_idx < grp_names.len(),
                    )
                } else {
                    (format!("Group{g_idx}_{p_idx:02}"), false)
                };
                let key = if has_known_name {
                    format!("{g_idx}_{p_idx:02}_{raw_name}")
                } else {
                    format!("{g_idx}_{p_idx:02}")
                };
                out.push(Wmb3NamedParameter {
                    group_index: g_idx,
                    param_index: p_idx,
                    key,
                    name: raw_name,
                    value: v,
                });
            }
        }
        out
    }

    pub fn texture_views(&self) -> Vec<Wmb3TextureSlotView> {
        self.textures
            .iter()
            .enumerate()
            .map(|(i, (nm, tid))| Wmb3TextureSlotView {
                slot_index: i,
                name: nm.clone(),
                texture_id: *tid,
                texture_id_hex: format!("0x{tid:08X}"),
                pretexture_name: lookup_pretexture_label("b3", *tid),
            })
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Wmb3Mesh {
    pub name: String,
    pub bounding_box: [f32; 6],
    pub materials: Vec<u16>,
    pub bone_indices: Vec<u16>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Wmb3File {
    pub version: u32,
    pub u_a: i32,
    pub flags: i16,
    pub u_c: i16,
    pub bounding_box: [f32; 6],
    pub bones: Vec<Wmb3Bone>,
    pub bone_index_translate_table: BoneIndexTranslateTable,
    #[serde(default)]
    pub raw_btt: Vec<u8>,
    pub vertex_groups: Vec<Wmb3VertexGroup>,
    pub batches: Vec<Wmb3Batch>,
    pub lods: Vec<Wmb3Lod>,
    #[serde(default)]
    pub col_tree_nodes: Vec<[u8; 32]>,
    pub bone_map: Vec<u32>,
    pub bone_sets: Vec<Vec<i16>>,
    pub materials: Vec<Wmb3Material>,
    pub meshes: Vec<Wmb3Mesh>,
    pub mesh_material_pairs: Vec<(u32, u32)>,
    #[serde(default)]
    pub world_data: Vec<[u8; 24]>,
    #[serde(default)]
    pub tail: [u32; 2],
}

impl Wmb3File {
    pub fn from_bytes(data: &[u8]) -> Result<Self> {
        if data.len() < 0x88 {
            return Err(PgError::InvalidData("WMB3 file shorter than 0x88 header".into()));
        }
        if &data[0..4] != b"WMB3" && &data[0..4] != b"3BMW" {
            return Err(PgError::InvalidMagic {
                expected: "WMB3",
                actual: data[0..4].to_vec(),
            });
        }

        let endian = if &data[0..4] == b"3BMW" {
            Endian::Big
        } else {
            Endian::Little
        };
        let mut r = BinReader::new(data, endian);
        let _id = r.read_u32()?;
        let version = r.read_u32()?;
        let u_a = r.read_i32()?;
        let flags = r.read_i16()?;
        let u_c = r.read_i16()?;
        let mut bounding_box = [0.0f32; 6];
        for slot in &mut bounding_box {
            *slot = r.read_f32()?;
        }

        let info_bones = Wmb3InfoPair::read(&mut r)?;
        let info_translate = Wmb3InfoPair::read(&mut r)?;
        let info_vgs = Wmb3InfoPair::read(&mut r)?;
        let info_batches = Wmb3InfoPair::read(&mut r)?;
        let info_lods = Wmb3InfoPair::read(&mut r)?;
        let info_col_tree = Wmb3InfoPair::read(&mut r)?;
        let info_bone_map = Wmb3InfoPair::read(&mut r)?;
        let info_bone_sets = Wmb3InfoPair::read(&mut r)?;
        let info_materials = Wmb3InfoPair::read(&mut r)?;
        let info_meshes = Wmb3InfoPair::read(&mut r)?;
        let info_pairs = Wmb3InfoPair::read(&mut r)?;
        let info_world = Wmb3InfoPair::read(&mut r)?;
        let tail = if data.len() >= 0x90 {
            [r.read_u32().unwrap_or(0), r.read_u32().unwrap_or(0)]
        } else {
            [0, 0]
        };

        let num_bones = if info_bones.offset != 0 && info_bones.number > 0 {
            checked_count(
                info_bones.number as usize,
                88,
                data.len().saturating_sub(info_bones.offset as usize),
                "WMB3 bones",
            )?
        } else {
            0
        };
        let mut bones = Vec::with_capacity(num_bones);
        if num_bones > 0 {
            r.seek(info_bones.offset as usize)?;
            for _ in 0..info_bones.number {
                bones.push(Wmb3Bone {
                    id: r.read_i16()?,
                    parent_index: r.read_i16()?,
                    local_position: Vec3::read(&mut r)?,
                    local_rotation: Vec3::read(&mut r)?,
                    local_scale: Vec3::read(&mut r)?,
                    position: Vec3::read(&mut r)?,
                    rotation: Vec3::read(&mut r)?,
                    scale: Vec3::read(&mut r)?,
                    t_position: Vec3::read(&mut r)?,
                });
            }
        }

        let mut raw_btt = Vec::new();
        let bone_index_translate_table = if info_translate.offset != 0 && info_translate.number > 0 {
            let btt_start = info_translate.offset as usize;
            let btt_end = (btt_start + info_translate.number as usize).min(data.len());
            if btt_start < btt_end {
                raw_btt = data[btt_start..btt_end].to_vec();
            }
            r.seek(btt_start)?;
            BoneIndexTranslateTable::read(&mut r).unwrap_or_else(|_| BoneIndexTranslateTable::new())
        } else {
            BoneIndexTranslateTable::new()
        };

        let num_vgs = if info_vgs.offset != 0 && info_vgs.number > 0 {
            checked_count(
                info_vgs.number as usize,
                48,
                data.len().saturating_sub(info_vgs.offset as usize),
                "WMB3 vertex_groups",
            )?
        } else {
            0
        };
        let mut vertex_groups = Vec::with_capacity(num_vgs);
        for i in 0..num_vgs {
            r.seek((info_vgs.offset as usize) + i * 48)?;
            let offset_vertexes = r.read_u32()?;
            let offset_vertexes_ex_data = r.read_u32()?;
            let vg_ua = r.read_u32()?;
            let vg_ub = r.read_u32()?;
            let vertex_size = r.read_u32()?;
            let vertex_ex_data_size = r.read_u32()?;
            let vg_uc = r.read_u32()?;
            let vg_ud = r.read_u32()?;
            let num_vertexes = r.read_u32()?;
            let vertex_flags = r.read_u32()?;
            let offset_indices = r.read_u32()?;
            let num_indices = r.read_u32()?;

            let v_bytes_len = (num_vertexes as usize).saturating_mul(vertex_size as usize);
            let raw_vertex_bytes = if offset_vertexes != 0
                && (offset_vertexes as usize) + v_bytes_len <= data.len()
            {
                data[(offset_vertexes as usize)..(offset_vertexes as usize) + v_bytes_len].to_vec()
            } else {
                Vec::new()
            };

            let ex_bytes_len = (num_vertexes as usize).saturating_mul(vertex_ex_data_size as usize);
            let raw_ex_bytes = if offset_vertexes_ex_data != 0
                && (offset_vertexes_ex_data as usize) + ex_bytes_len <= data.len()
            {
                data[(offset_vertexes_ex_data as usize)
                    ..(offset_vertexes_ex_data as usize) + ex_bytes_len]
                    .to_vec()
            } else {
                Vec::new()
            };

            if num_vertexes > 0 && offset_vertexes != 0 {
                checked_count(
                    num_vertexes as usize,
                    (vertex_size as usize).max(1),
                    data.len().saturating_sub(offset_vertexes as usize),
                    "WMB3 vertexes",
                )?;
            } else if num_vertexes > 0 {
                return Err(PgError::InvalidData(
                    "WMB3 vertex group has non-zero num_vertexes with zero offset".into(),
                ));
            }
            if num_indices > 0 && offset_indices != 0 {
                let idx_elem = if (flags & 0x8) != 0 { 4 } else { 2 };
                checked_count(
                    num_indices as usize,
                    idx_elem,
                    data.len().saturating_sub(offset_indices as usize),
                    "WMB3 indices",
                )?;
            } else if num_indices > 0 {
                return Err(PgError::InvalidData(
                    "WMB3 vertex group has non-zero num_indices with zero offset".into(),
                ));
            }

            let layout_opt =
                lookup_wmb3_vertex_layout_with_strides(vertex_flags, vertex_size, vertex_ex_data_size);

            let mut vertexes = vec![WmbVertex::default(); num_vertexes as usize];
            if let Some(layout) = layout_opt {
                if offset_vertexes != 0 && !layout.primary_fields.is_empty() && vertex_size > 0 {
                    for (vi, v) in vertexes.iter_mut().enumerate() {
                        let v_start = (offset_vertexes as usize) + vi * (vertex_size as usize);
                        if v_start + (vertex_size as usize) > data.len() {
                            break;
                        }
                        r.seek(v_start)?;
                        for f in layout.primary_fields {
                            if r.pos() + f.field_type.byte_size() <= v_start + (vertex_size as usize)
                            {
                                let _ = v.read_field(&mut r, f.name, f.field_type, true);
                            }
                        }
                    }
                }
                if offset_vertexes_ex_data != 0
                    && !layout.ex_fields.is_empty()
                    && vertex_ex_data_size > 0
                {
                    for (vi, v) in vertexes.iter_mut().enumerate() {
                        let ex_start =
                            (offset_vertexes_ex_data as usize) + vi * (vertex_ex_data_size as usize);
                        if ex_start + (vertex_ex_data_size as usize) > data.len() {
                            break;
                        }
                        r.seek(ex_start)?;
                        for f in layout.ex_fields {
                            if r.pos() + f.field_type.byte_size()
                                <= ex_start + (vertex_ex_data_size as usize)
                            {
                                let _ = v.read_field(&mut r, f.name, f.field_type, true);
                            }
                        }
                    }
                }
            } else if offset_vertexes != 0 && vertex_size >= 20 {
                // Fallback reader for custom/modded vertex strides: read position (12B) + tangents (4B) + uv1 (4B)
                for (vi, v) in vertexes.iter_mut().enumerate() {
                    let v_start = (offset_vertexes as usize) + vi * (vertex_size as usize);
                    if v_start + 20 > data.len() {
                        break;
                    }
                    r.seek(v_start)?;
                    v.position = Vec3::read(&mut r).ok();
                    let _ = r.read_u32();
                    if let (Ok(u), Ok(vt)) = (r.read_f16(), r.read_f16()) {
                        v.mapping = [u, vt];
                    }
                }
            }

            let mut indices = Vec::with_capacity(num_indices as usize);
            if offset_indices != 0 && num_indices > 0 {
                r.seek(offset_indices as usize)?;
                let use_u32 = (flags & 0x8) != 0;
                for _ in 0..num_indices {
                    let idx = if use_u32 {
                        r.read_u32().unwrap_or(0)
                    } else {
                        r.read_u16().unwrap_or(0) as u32
                    };
                    indices.push(idx);
                }
            }

            vertex_groups.push(Wmb3VertexGroup {
                offset_vertexes,
                offset_vertexes_ex_data,
                u_a: vg_ua,
                u_b: vg_ub,
                vertex_size,
                vertex_ex_data_size,
                u_c: vg_uc,
                u_d: vg_ud,
                num_vertexes,
                vertex_flags,
                offset_indices,
                num_indices,
                vertexes,
                indices,
                raw_vertex_bytes,
                raw_ex_bytes,
            });
        }

        let num_batches = if info_batches.offset != 0 && info_batches.number > 0 {
            checked_count(
                info_batches.number as usize,
                28,
                data.len().saturating_sub(info_batches.offset as usize),
                "WMB3 batches",
            )?
        } else {
            0
        };
        let mut batches = Vec::with_capacity(num_batches);
        if num_batches > 0 {
            r.seek(info_batches.offset as usize)?;
            for _ in 0..num_batches {
                batches.push(Wmb3Batch {
                    vertex_group_index: r.read_u32()?,
                    bone_set_index: r.read_i32()?,
                    vertex_start: r.read_u32()?,
                    index_start: r.read_u32()?,
                    num_vertexes: r.read_u32()?,
                    num_indices: r.read_u32()?,
                    num_primitives: r.read_u32()?,
                });
            }
        }

        let num_lods = if info_lods.offset != 0 && info_lods.number > 0 {
            checked_count(
                info_lods.number as usize,
                20,
                data.len().saturating_sub(info_lods.offset as usize),
                "WMB3 lods",
            )?
        } else {
            0
        };
        let mut lods = Vec::with_capacity(num_lods);
        for i in 0..num_lods {
            r.seek((info_lods.offset as usize) + i * 20)?;
            let offset_name = r.read_u32()? as usize;
            let lod_level = r.read_i32()?;
            let batch_start = r.read_u32()?;
            let offset_batch_infos = r.read_u32()? as usize;
            let num_batch_infos = r.read_u32()? as usize;

            let name = if offset_name > 0 && offset_name < data.len() {
                r.read_cstring_at(offset_name).unwrap_or_else(|_| format!("LOD{i}"))
            } else {
                format!("LOD{i}")
            };
            let valid_bi = if offset_batch_infos > 0 && num_batch_infos > 0 {
                checked_count(
                    num_batch_infos,
                    24,
                    data.len().saturating_sub(offset_batch_infos),
                    "WMB3 lod batch_infos",
                )?
            } else {
                0
            };
            let mut batch_infos = Vec::with_capacity(valid_bi);
            if valid_bi > 0 {
                r.seek(offset_batch_infos)?;
                for _ in 0..valid_bi {
                    batch_infos.push(Wmb3LodBatchInfo {
                        vertex_group_index: r.read_u32()?,
                        mesh_index: r.read_u32()?,
                        material_index: r.read_u32()?,
                        call_tree_node_index: r.read_i32()?,
                        mesh_material_index: r.read_u32()?,
                        u_b: r.read_i32()?,
                    });
                }
            }
            lods.push(Wmb3Lod {
                name,
                lod_level,
                batch_start,
                batch_infos,
            });
        }

        let num_col_tree = if info_col_tree.offset != 0 && info_col_tree.number > 0 {
            checked_count(
                info_col_tree.number as usize,
                32,
                data.len().saturating_sub(info_col_tree.offset as usize),
                "WMB3 col_tree_nodes",
            )?
        } else {
            0
        };
        let mut col_tree_nodes = Vec::with_capacity(num_col_tree);
        if num_col_tree > 0 {
            r.seek(info_col_tree.offset as usize)?;
            for _ in 0..num_col_tree {
                if let Ok(b) = r.read_bytes(32) {
                    let mut arr = [0u8; 32];
                    arr.copy_from_slice(b);
                    col_tree_nodes.push(arr);
                }
            }
        }

        let num_bone_map = if info_bone_map.offset != 0 && info_bone_map.number > 0 {
            checked_count(
                info_bone_map.number as usize,
                4,
                data.len().saturating_sub(info_bone_map.offset as usize),
                "WMB3 bone_map",
            )?
        } else {
            0
        };
        let mut bone_map = Vec::with_capacity(num_bone_map);
        if num_bone_map > 0 {
            r.seek(info_bone_map.offset as usize)?;
            for _ in 0..num_bone_map {
                bone_map.push(r.read_u32()?);
            }
        }

        let num_bone_sets = if info_bone_sets.offset != 0 && info_bone_sets.number > 0 {
            checked_count(
                info_bone_sets.number as usize,
                8,
                data.len().saturating_sub(info_bone_sets.offset as usize),
                "WMB3 bone_sets",
            )?
        } else {
            0
        };
        let mut bone_sets = Vec::with_capacity(num_bone_sets);
        for i in 0..num_bone_sets {
            r.seek((info_bone_sets.offset as usize) + i * 8)?;
            let off_bi = r.read_u32()? as usize;
            let num_bi = r.read_u32()? as usize;
            let valid_bi = if off_bi > 0 && num_bi > 0 {
                checked_count(
                    num_bi,
                    2,
                    data.len().saturating_sub(off_bi),
                    "WMB3 bone_set entries",
                )?
            } else {
                0
            };
            let mut set = Vec::with_capacity(valid_bi);
            if valid_bi > 0 {
                r.seek(off_bi)?;
                for _ in 0..valid_bi {
                    set.push(r.read_i16()?);
                }
            }
            bone_sets.push(set);
        }

        let num_materials = if info_materials.offset != 0 && info_materials.number > 0 {
            checked_count(
                info_materials.number as usize,
                48,
                data.len().saturating_sub(info_materials.offset as usize),
                "WMB3 materials",
            )?
        } else {
            0
        };
        let mut materials = Vec::with_capacity(num_materials);
        for i in 0..num_materials {
            r.seek((info_materials.offset as usize) + i * 48)?;
            let date = [r.read_u16()?, r.read_u16()?, r.read_u16()?, r.read_u16()?];
            let off_name = r.read_u32()? as usize;
            let off_shader = r.read_u32()? as usize;
            let off_tech = r.read_u32()? as usize;
            let mat_ua = r.read_u32()?;
            let off_tex = r.read_u32()? as usize;
            let num_tex = r.read_u32()? as usize;
            let off_pg = r.read_u32()? as usize;
            let num_pg = r.read_u32()? as usize;
            let off_var = r.read_u32()? as usize;
            let num_var = r.read_u32()? as usize;

            let name = if off_name > 0 && off_name < data.len() {
                r.read_cstring_at(off_name).unwrap_or_default()
            } else {
                format!("Material_{i}")
            };
            let shader_name = if off_shader > 0 && off_shader < data.len() {
                r.read_cstring_at(off_shader).unwrap_or_default()
            } else {
                String::new()
            };
            let technique_name = if off_tech > 0 && off_tech < data.len() {
                r.read_cstring_at(off_tech).unwrap_or_else(|_| "Default".into())
            } else {
                "Default".into()
            };

            let valid_tex = if off_tex > 0 && num_tex > 0 {
                checked_count(
                    num_tex,
                    8,
                    data.len().saturating_sub(off_tex),
                    "WMB3 material textures",
                )?
            } else {
                0
            };
            let mut textures = Vec::with_capacity(valid_tex);
            for j in 0..valid_tex {
                r.seek(off_tex + j * 8)?;
                let tn_off = r.read_u32()? as usize;
                let tid = r.read_u32()?;
                let tname = if tn_off > 0 && tn_off < data.len() {
                    r.read_cstring_at(tn_off).unwrap_or_else(|_| format!("g_Tex{j}"))
                } else {
                    format!("g_Tex{j}")
                };
                textures.push((tname, tid));
            }

            let valid_pg = if off_pg > 0 && num_pg > 0 {
                checked_count(
                    num_pg,
                    12,
                    data.len().saturating_sub(off_pg),
                    "WMB3 material parameter_groups",
                )?
            } else {
                0
            };
            let mut parameter_groups = Vec::with_capacity(valid_pg);
            for j in 0..valid_pg {
                r.seek(off_pg + j * 12)?;
                let pg_idx = r.read_i32()?;
                let p_off = r.read_u32()? as usize;
                let p_num = r.read_u32()? as usize;
                let valid_p = if p_off > 0 && p_num > 0 {
                    checked_count(
                        p_num,
                        4,
                        data.len().saturating_sub(p_off),
                        "WMB3 material params",
                    )?
                } else {
                    0
                };
                let mut params = Vec::with_capacity(valid_p);
                if valid_p > 0 {
                    r.seek(p_off)?;
                    for _ in 0..valid_p {
                        params.push(r.read_f32()?);
                    }
                }
                parameter_groups.push((pg_idx, params));
            }

            let valid_var = if off_var > 0 && num_var > 0 {
                checked_count(
                    num_var,
                    8,
                    data.len().saturating_sub(off_var),
                    "WMB3 material variables",
                )?
            } else {
                0
            };
            let mut variables = Vec::with_capacity(valid_var);
            for j in 0..valid_var {
                r.seek(off_var + j * 8)?;
                let vn_off = r.read_u32()? as usize;
                let v_val = r.read_f32()?;
                let vname = if vn_off > 0 && vn_off < data.len() {
                    r.read_cstring_at(vn_off).unwrap_or_else(|_| format!("Var_{j}"))
                } else {
                    format!("Var_{j}")
                };
                variables.push((vname, v_val));
            }

            materials.push(Wmb3Material {
                date,
                name,
                shader_name,
                technique_name,
                u_a: mat_ua,
                textures,
                parameter_groups,
                variables,
            });
        }

        let num_meshes = if info_meshes.offset != 0 && info_meshes.number > 0 {
            checked_count(
                info_meshes.number as usize,
                44,
                data.len().saturating_sub(info_meshes.offset as usize),
                "WMB3 meshes",
            )?
        } else {
            0
        };
        let mut meshes = Vec::with_capacity(num_meshes);
        for i in 0..num_meshes {
            r.seek((info_meshes.offset as usize) + i * 44)?;
            let off_name = r.read_u32()? as usize;
            let mut bb = [0.0f32; 6];
            for slot in &mut bb {
                *slot = r.read_f32()?;
            }
            let off_mats = r.read_u32()? as usize;
            let num_mats = r.read_u32()? as usize;
            let off_bi = r.read_u32()? as usize;
            let num_bi = r.read_u32()? as usize;

            let name = if off_name > 0 && off_name < data.len() {
                r.read_cstring_at(off_name).unwrap_or_else(|_| format!("Mesh_{i}"))
            } else {
                format!("Mesh_{i}")
            };
            let valid_mats = if off_mats > 0 && num_mats > 0 {
                checked_count(
                    num_mats,
                    2,
                    data.len().saturating_sub(off_mats),
                    "WMB3 mesh materials",
                )?
            } else {
                0
            };
            let mut m_mats = Vec::with_capacity(valid_mats);
            if valid_mats > 0 {
                r.seek(off_mats)?;
                for _ in 0..valid_mats {
                    m_mats.push(r.read_u16()?);
                }
            }
            let valid_bi = if off_bi > 0 && num_bi > 0 {
                checked_count(
                    num_bi,
                    2,
                    data.len().saturating_sub(off_bi),
                    "WMB3 mesh bone_indices",
                )?
            } else {
                0
            };
            let mut m_bi = Vec::with_capacity(valid_bi);
            if valid_bi > 0 {
                r.seek(off_bi)?;
                for _ in 0..valid_bi {
                    m_bi.push(r.read_u16()?);
                }
            }
            meshes.push(Wmb3Mesh {
                name,
                bounding_box: bb,
                materials: m_mats,
                bone_indices: m_bi,
            });
        }

        let num_pairs = if info_pairs.offset != 0 && info_pairs.number > 0 {
            checked_count(
                info_pairs.number as usize,
                8,
                data.len().saturating_sub(info_pairs.offset as usize),
                "WMB3 mesh_material_pairs",
            )?
        } else {
            0
        };
        let mut mesh_material_pairs = Vec::with_capacity(num_pairs);
        if num_pairs > 0 {
            r.seek(info_pairs.offset as usize)?;
            for _ in 0..num_pairs {
                mesh_material_pairs.push((r.read_u32()?, r.read_u32()?));
            }
        }

        let num_world = if info_world.offset != 0 && info_world.number > 0 {
            checked_count(
                info_world.number as usize,
                24,
                data.len().saturating_sub(info_world.offset as usize),
                "WMB3 world_data",
            )?
        } else {
            0
        };
        let mut world_data = Vec::with_capacity(num_world);
        if num_world > 0 {
            r.seek(info_world.offset as usize)?;
            for _ in 0..num_world {
                if let Ok(b) = r.read_bytes(24) {
                    let mut arr = [0u8; 24];
                    arr.copy_from_slice(b);
                    world_data.push(arr);
                }
            }
        }

        Ok(Self {
            version,
            u_a,
            flags,
            u_c,
            bounding_box,
            bones,
            bone_index_translate_table,
            raw_btt,
            vertex_groups,
            batches,
            lods,
            col_tree_nodes,
            bone_map,
            bone_sets,
            materials,
            meshes,
            mesh_material_pairs,
            world_data,
            tail,
        })
    }

    /// Serialize this `WMB3` model back to binary bytes, matching `wmb3_format.py`'s `WMB3.to_bytes()`
    /// section ordering and 16-byte buffer/table alignment rules.
    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        let mut out = vec![0u8; 0x90];
        let use_u32_indices = (self.flags & 0x8) != 0;

        let put_bytes = |buf: &mut Vec<u8>, bytes: &[u8], alignment: usize| -> u32 {
            if bytes.is_empty() {
                return 0;
            }
            if alignment > 1 {
                let aligned = align(buf.len(), alignment);
                buf.resize(aligned, 0);
            }
            let pos = buf.len() as u32;
            buf.extend_from_slice(bytes);
            pos
        };

        let put_str = |buf: &mut Vec<u8>, s: &str| -> u32 {
            let pos = buf.len() as u32;
            buf.extend_from_slice(s.as_bytes());
            buf.push(0);
            pos
        };

        let reserve = |buf: &mut Vec<u8>, size: usize, alignment: usize| -> u32 {
            if size == 0 {
                return 0;
            }
            if alignment > 1 {
                let aligned = align(buf.len(), alignment);
                buf.resize(aligned, 0);
            }
            let pos = buf.len() as u32;
            buf.resize(buf.len() + size, 0);
            pos
        };

        // 1. Bones (0x58 bytes each)
        let off_bones = if !self.bones.is_empty() {
            let mut bw = BinWriter::new(Endian::Little);
            for b in &self.bones {
                bw.write_i16(b.id);
                bw.write_i16(b.parent_index);
                b.local_position.write(&mut bw);
                b.local_rotation.write(&mut bw);
                b.local_scale.write(&mut bw);
                b.position.write(&mut bw);
                b.rotation.write(&mut bw);
                b.scale.write(&mut bw);
                b.t_position.write(&mut bw);
            }
            put_bytes(&mut out, &bw.into_inner(), 1)
        } else {
            0
        };

        // 2. Bone Index Translate Table
        let btt_bytes = if !self.raw_btt.is_empty() {
            self.raw_btt.clone()
        } else if !self.bone_index_translate_table.table.is_empty() {
            let mut bw = BinWriter::new(Endian::Little);
            self.bone_index_translate_table.write(&mut bw);
            bw.into_inner()
        } else {
            Vec::new()
        };
        let off_btt = put_bytes(&mut out, &btt_bytes, 1);

        // 3. Vertex Groups
        let nvg = self.vertex_groups.len();
        let off_vgs = reserve(&mut out, nvg * 48, 16);
        for (i, vg) in self.vertex_groups.iter().enumerate() {
            let (v_buf, ex_buf, v_stride, ex_stride) = if let Some(layout) =
                lookup_wmb3_vertex_layout_with_strides(
                    vg.vertex_flags,
                    vg.vertex_size,
                    vg.vertex_ex_data_size,
                ) {
                let mut vw = BinWriter::new(Endian::Little);
                for v in &vg.vertexes {
                    for f in layout.primary_fields {
                        v.write_field(&mut vw, f.name, f.field_type, true);
                    }
                }
                let mut exw = BinWriter::new(Endian::Little);
                if !layout.ex_fields.is_empty() {
                    for v in &vg.vertexes {
                        for f in layout.ex_fields {
                            v.write_field(&mut exw, f.name, f.field_type, true);
                        }
                    }
                }
                (
                    vw.into_inner(),
                    exw.into_inner(),
                    layout.primary_stride() as u32,
                    layout.ex_stride() as u32,
                )
            } else {
                (
                    vg.raw_vertex_bytes.clone(),
                    vg.raw_ex_bytes.clone(),
                    vg.vertex_size,
                    vg.vertex_ex_data_size,
                )
            };

            let v_off = put_bytes(&mut out, &v_buf, 16);
            let ex_off = if !ex_buf.is_empty() {
                put_bytes(&mut out, &ex_buf, 16)
            } else {
                0
            };

            let mut idx_w = BinWriter::new(Endian::Little);
            for &idx in &vg.indices {
                if use_u32_indices {
                    idx_w.write_u32(idx);
                } else {
                    idx_w.write_u16(idx as u16);
                }
            }
            let idx_off = put_bytes(&mut out, &idx_w.into_inner(), 16);

            let mut rec_w = BinWriter::new(Endian::Little);
            rec_w.write_u32(v_off);
            rec_w.write_u32(ex_off);
            rec_w.write_u32(vg.u_a);
            rec_w.write_u32(vg.u_b);
            rec_w.write_u32(v_stride);
            rec_w.write_u32(ex_stride);
            rec_w.write_u32(vg.u_c);
            rec_w.write_u32(vg.u_d);
            rec_w.write_u32(vg.vertexes.len() as u32);
            rec_w.write_u32(vg.vertex_flags);
            rec_w.write_u32(idx_off);
            rec_w.write_u32(vg.indices.len() as u32);
            let rec_bytes = rec_w.into_inner();
            let pos = (off_vgs as usize) + i * 48;
            out[pos..pos + 48].copy_from_slice(&rec_bytes);
        }

        // 4. Batches
        let off_batches = if !self.batches.is_empty() {
            let mut bw = BinWriter::new(Endian::Little);
            for b in &self.batches {
                bw.write_u32(b.vertex_group_index);
                bw.write_i32(b.bone_set_index);
                bw.write_u32(b.vertex_start);
                bw.write_u32(b.index_start);
                bw.write_u32(b.num_vertexes);
                bw.write_u32(b.num_indices);
                bw.write_u32(b.num_primitives);
            }
            put_bytes(&mut out, &bw.into_inner(), 1)
        } else {
            0
        };

        // 5. LODs
        let off_lods = reserve(&mut out, self.lods.len() * 20, 1);
        for (i, lod) in self.lods.iter().enumerate() {
            let mut inf_w = BinWriter::new(Endian::Little);
            for inf in &lod.batch_infos {
                inf_w.write_u32(inf.vertex_group_index);
                inf_w.write_u32(inf.mesh_index);
                inf_w.write_u32(inf.material_index);
                inf_w.write_i32(inf.call_tree_node_index);
                inf_w.write_u32(inf.mesh_material_index);
                inf_w.write_i32(inf.u_b);
            }
            let inf_off = put_bytes(&mut out, &inf_w.into_inner(), 1);
            let name_off = put_str(&mut out, &lod.name);

            let mut lw = BinWriter::new(Endian::Little);
            lw.write_u32(name_off);
            lw.write_i32(lod.lod_level);
            lw.write_u32(lod.batch_start);
            lw.write_u32(inf_off);
            lw.write_u32(lod.batch_infos.len() as u32);
            let lb = lw.into_inner();
            let pos = (off_lods as usize) + i * 20;
            out[pos..pos + 20].copy_from_slice(&lb);
        }

        // 6. Mesh/Material pairs
        let off_pairs = if !self.mesh_material_pairs.is_empty() {
            let mut pw = BinWriter::new(Endian::Little);
            for &(m, mat) in &self.mesh_material_pairs {
                pw.write_u32(m);
                pw.write_u32(mat);
            }
            put_bytes(&mut out, &pw.into_inner(), 16)
        } else {
            0
        };

        // 7. ColTree nodes
        let off_coltree = if !self.col_tree_nodes.is_empty() {
            let mut cw = Vec::with_capacity(self.col_tree_nodes.len() * 32);
            for n in &self.col_tree_nodes {
                cw.extend_from_slice(n);
            }
            put_bytes(&mut out, &cw, 16)
        } else {
            0
        };

        // 8. Bone sets
        let off_bonesets = if !self.bone_sets.is_empty() {
            let table_off = reserve(&mut out, self.bone_sets.len() * 8, 16);
            for (i, set) in self.bone_sets.iter().enumerate() {
                let mut sw = BinWriter::new(Endian::Little);
                for &b in set {
                    sw.write_i16(b);
                }
                let s_off = put_bytes(&mut out, &sw.into_inner(), 16);
                let pos = (table_off as usize) + i * 8;
                out[pos..pos + 4].copy_from_slice(&s_off.to_le_bytes());
                out[pos + 4..pos + 8].copy_from_slice(&(set.len() as u32).to_le_bytes());
            }
            table_off
        } else {
            0
        };

        // 9. Bone map
        let off_bonemap = if !self.bone_map.is_empty() {
            let mut bmw = BinWriter::new(Endian::Little);
            for &bm in &self.bone_map {
                bmw.write_u32(bm);
            }
            put_bytes(&mut out, &bmw.into_inner(), 16)
        } else {
            0
        };

        // 10. Meshes
        let off_meshes = reserve(&mut out, self.meshes.len() * 44, 1);
        for (i, me) in self.meshes.iter().enumerate() {
            let name_off = put_str(&mut out, &me.name);
            let mut mw = BinWriter::new(Endian::Little);
            for &m in &me.materials {
                mw.write_u16(m);
            }
            let mat_off = put_bytes(&mut out, &mw.into_inner(), 1);
            let mut bw = BinWriter::new(Endian::Little);
            for &b in &me.bone_indices {
                bw.write_u16(b);
            }
            let bone_off = put_bytes(&mut out, &bw.into_inner(), 1);

            let mut rec_w = BinWriter::new(Endian::Little);
            rec_w.write_u32(name_off);
            for &f in &me.bounding_box {
                rec_w.write_f32(f);
            }
            rec_w.write_u32(mat_off);
            rec_w.write_u32(me.materials.len() as u32);
            rec_w.write_u32(bone_off);
            rec_w.write_u32(me.bone_indices.len() as u32);
            let rb = rec_w.into_inner();
            let pos = (off_meshes as usize) + i * 44;
            out[pos..pos + 44].copy_from_slice(&rb);
        }

        // 11. Materials
        let off_materials = reserve(&mut out, self.materials.len() * 48, 16);
        for (i, mat) in self.materials.iter().enumerate() {
            let name_off = put_str(&mut out, &mat.name);
            let shader_off = put_str(&mut out, &mat.shader_name);
            let tech_off = put_str(&mut out, &mat.technique_name);

            let tex_off = if !mat.textures.is_empty() {
                let t_table = reserve(&mut out, mat.textures.len() * 8, 1);
                for (k, (tn, tid)) in mat.textures.iter().enumerate() {
                    let tn_off = put_str(&mut out, tn);
                    let pos = (t_table as usize) + k * 8;
                    out[pos..pos + 4].copy_from_slice(&tn_off.to_le_bytes());
                    out[pos + 4..pos + 8].copy_from_slice(&tid.to_le_bytes());
                }
                t_table
            } else {
                0
            };

            let pg_off = if !mat.parameter_groups.is_empty() {
                let g_table = reserve(&mut out, mat.parameter_groups.len() * 12, 16);
                for (k, (g_idx, vals)) in mat.parameter_groups.iter().enumerate() {
                    let mut vw = BinWriter::new(Endian::Little);
                    for &v in vals {
                        vw.write_f32(v);
                    }
                    let v_off = put_bytes(&mut out, &vw.into_inner(), 16);
                    let pos = (g_table as usize) + k * 12;
                    out[pos..pos + 4].copy_from_slice(&g_idx.to_le_bytes());
                    out[pos + 4..pos + 8].copy_from_slice(&v_off.to_le_bytes());
                    out[pos + 8..pos + 12].copy_from_slice(&(vals.len() as u32).to_le_bytes());
                }
                g_table
            } else {
                0
            };

            let var_off = if !mat.variables.is_empty() {
                let v_table = reserve(&mut out, mat.variables.len() * 8, 16);
                for (k, (vn, val)) in mat.variables.iter().enumerate() {
                    let vn_off = put_str(&mut out, vn);
                    let pos = (v_table as usize) + k * 8;
                    out[pos..pos + 4].copy_from_slice(&vn_off.to_le_bytes());
                    out[pos + 4..pos + 8].copy_from_slice(&val.to_le_bytes());
                }
                v_table
            } else {
                0
            };

            let mut mw = BinWriter::new(Endian::Little);
            for &d in &mat.date {
                mw.write_u16(d);
            }
            mw.write_u32(name_off);
            mw.write_u32(shader_off);
            mw.write_u32(tech_off);
            let ua = if mat.shader_name.to_ascii_lowercase().starts_with("null00") {
                0
            } else {
                mat.u_a
            };
            mw.write_u32(ua);
            mw.write_u32(tex_off);
            mw.write_u32(mat.textures.len() as u32);
            mw.write_u32(pg_off);
            mw.write_u32(mat.parameter_groups.len() as u32);
            mw.write_u32(var_off);
            mw.write_u32(mat.variables.len() as u32);
            let mb = mw.into_inner();
            let pos = (off_materials as usize) + i * 48;
            out[pos..pos + 48].copy_from_slice(&mb);
        }

        // 12. World data
        let off_world = if !self.world_data.is_empty() {
            let mut ww = Vec::with_capacity(self.world_data.len() * 24);
            for w in &self.world_data {
                ww.extend_from_slice(w);
            }
            put_bytes(&mut out, &ww, 16)
        } else {
            0
        };

        // Write 0x90 Header
        let mut hw = BinWriter::new(Endian::Little);
        hw.write_bytes(b"WMB3");
        hw.write_u32(self.version);
        hw.write_i32(self.u_a);
        hw.write_i16(self.flags);
        hw.write_i16(self.u_c);
        for &b in &self.bounding_box {
            hw.write_f32(b);
        }
        let pairs = [
            (off_bones, self.bones.len() as u32),
            (off_btt, btt_bytes.len() as u32),
            (off_vgs, nvg as u32),
            (off_batches, self.batches.len() as u32),
            (off_lods, self.lods.len() as u32),
            (off_coltree, self.col_tree_nodes.len() as u32),
            (off_bonemap, self.bone_map.len() as u32),
            (off_bonesets, self.bone_sets.len() as u32),
            (off_materials, self.materials.len() as u32),
            (off_meshes, self.meshes.len() as u32),
            (off_pairs, self.mesh_material_pairs.len() as u32),
            (off_world, self.world_data.len() as u32),
        ];
        for (o, c) in pairs {
            hw.write_u32(o);
            hw.write_u32(c);
        }
        hw.write_u32(self.tail[0]);
        hw.write_u32(self.tail[1]);
        let hb = hw.into_inner();
        out[0..0x90].copy_from_slice(&hb[0..0x90]);

        Ok(out)
    }

    pub fn texture_ids(&self) -> Vec<u32> {
        let mut ids: BTreeSet<u32> = BTreeSet::new();
        for m in &self.materials {
            for &(_, tid) in &m.textures {
                ids.insert(tid);
            }
        }
        ids.into_iter().collect()
    }

    pub fn set_tpose(&mut self) {
        let neutral_scale = [1.0f32, 1.0, 1.0];
        let inverse_bind_pose: Vec<Matrix> = self
            .bones
            .iter()
            .map(|b| {
                let parent_scale =
                    if b.parent_index >= 0 && (b.parent_index as usize) < self.bones.len() {
                        self.bones[b.parent_index as usize].scale.to_array()
                    } else {
                        neutral_scale
                    };
                Matrix::inverse_transformation(
                    b.position.to_array(),
                    b.rotation.to_array(),
                    b.scale.to_array(),
                    parent_scale,
                    None,
                )
            })
            .collect();

        let target_pose: Vec<Matrix> = self
            .bones
            .iter()
            .map(|b| Matrix::translation(b.t_position.x, b.t_position.y, b.t_position.z))
            .collect();

        let orig_t_positions: Vec<Vec3> = self.bones.iter().map(|b| b.t_position).collect();
        for b in &mut self.bones {
            b.position = b.t_position;
            if b.parent_index >= 0 && (b.parent_index as usize) < orig_t_positions.len() {
                let p = orig_t_positions[b.parent_index as usize];
                b.local_position = Vec3::new(
                    b.t_position.x - p.x,
                    b.t_position.y - p.y,
                    b.t_position.z - p.z,
                );
            } else {
                b.local_position = b.t_position;
            }
            b.rotation = Vec3::ZERO;
            b.scale = Vec3::ONE;
            b.local_rotation = Vec3::ZERO;
            b.local_scale = Vec3::ONE;
        }

        let multiplied: Vec<Matrix> = target_pose
            .into_iter()
            .zip(inverse_bind_pose)
            .map(|(t, inv)| t * inv)
            .collect();

        let mut visited: BTreeSet<(usize, usize)> = BTreeSet::new();
        for b in &self.batches {
            let vgi = b.vertex_group_index as usize;
            let Some(vg) = self.vertex_groups.get_mut(vgi) else {
                continue;
            };
            let bone_refs: Vec<usize> =
                if b.bone_set_index >= 0 && (b.bone_set_index as usize) < self.bone_sets.len() {
                    self.bone_sets[b.bone_set_index as usize]
                        .iter()
                        .map(|&bi| *self.bone_map.get(bi as usize).unwrap_or(&0) as usize)
                        .collect()
                } else {
                    self.bone_map.iter().map(|&bi| bi as usize).collect()
                };

            let end_idx = ((b.index_start + b.num_indices) as usize).min(vg.indices.len());
            for idx_pos in (b.index_start as usize)..end_idx {
                let vi = (vg.indices[idx_pos] + b.vertex_start) as usize;
                if vi >= vg.vertexes.len() || !visited.insert((vgi, vi)) {
                    continue;
                }
                let v = &mut vg.vertexes[vi];
                if let Some(bi_info) = v.bone_infos {
                    let mut v_mat = Matrix::zero();
                    for (local_bi, bw) in bi_info.active_pairs() {
                        if let Some(&global_bi) = bone_refs.get(local_bi as usize) {
                            if let Some(&m) = multiplied.get(global_bi) {
                                v_mat = v_mat + m * ((bw as f32) / 255.0);
                            }
                        }
                    }
                    if let Some(p) = v.position {
                        let np = v_mat * Vector::vec3(p.x, p.y, p.z);
                        v.position = Some(Vec3::new(np.x(), np.y(), np.z()));
                    }
                }
            }
        }
    }

    /// Export this `WMB3` model to Wavefront `.obj` format.
    pub fn export_obj(&self) -> String {
        let mut out = String::from("# Exported by pgtools-rs (WMB3)\n");
        let mut v_offset = 1usize;
        for (vgi, vg) in self.vertex_groups.iter().enumerate() {
            out.push_str(&format!("g vertex_group_{vgi}\n"));
            for v in &vg.vertexes {
                let p = v.position.unwrap_or_default();
                out.push_str(&format!("v {:.6} {:.6} {:.6}\n", p.x, p.y, p.z));
            }
            for v in &vg.vertexes {
                out.push_str(&format!("vt {:.6} {:.6}\n", v.mapping[0], 1.0 - v.mapping[1]));
            }
            for v in &vg.vertexes {
                let n = v.normal.normal;
                out.push_str(&format!("vn {:.6} {:.6} {:.6}\n", n[0], n[1], n[2]));
            }
            for b in self.batches.iter().filter(|b| b.vertex_group_index as usize == vgi) {
                let start = b.index_start as usize;
                let end = (start + b.num_indices as usize).min(vg.indices.len());
                for tri in vg.indices[start..end].chunks_exact(3) {
                    let i0 = (tri[0] + b.vertex_start) as usize + v_offset;
                    let i1 = (tri[1] + b.vertex_start) as usize + v_offset;
                    let i2 = (tri[2] + b.vertex_start) as usize + v_offset;
                    out.push_str(&format!("f {i0}/{i0}/{i0} {i1}/{i1}/{i1} {i2}/{i2}/{i2}\n"));
                }
            }
            v_offset += vg.vertexes.len();
        }
        out
    }
}
