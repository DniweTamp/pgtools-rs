//! Built-in procedural demo model generator (`build_demo_dat`) for Bayonetta 1 (`b1`),
//! Bayonetta 2 (`b2`), and Bayonetta 3 (`b3`) per Appendix E.6.

use crate::bone::{BoneIndexTranslateTable, Vec3};
use crate::dat::DatFile;
use crate::io::Endian;
use crate::shader_db::{switch_wmb3_material_shader, switch_wmb_material_shader};
use crate::texture_conv::{encode_rgba8_to_dds, encode_rgba_to_xt1, TextureFormat};
use crate::wmb::{
    BoneInfos, PackedNormal, PackedTangents, WmbBatch, WmbBatchHeader, WmbFile, WmbHeader,
    WmbMaterial, WmbMesh, WmbVertex,
};
use crate::wmb3::{
    Wmb3Batch, Wmb3Bone, Wmb3File, Wmb3Lod, Wmb3LodBatchInfo, Wmb3Material, Wmb3Mesh,
    Wmb3VertexGroup,
};
use crate::wtb::WtbFile;
use crate::{PgError, Result};
use image::{Rgba, RgbaImage};
use std::collections::BTreeMap;
use std::f32::consts::PI;

fn make_vertex(
    pos: [f32; 3],
    nrm: [f32; 3],
    tan: [f32; 4],
    uv: [f32; 2],
    bone_idx: u8,
    wide_nrm: bool,
) -> WmbVertex {
    let normal = PackedNormal {
        normal: nrm,
        wide: wide_nrm,
        raw_bytes: if wide_nrm {
            PackedNormal::pack_wide(nrm).to_le_bytes()
        } else {
            PackedNormal::pack_narrow(nrm)
        },
    };
    let mut bi = BoneInfos::default();
    bi.set_active_pairs(&[(bone_idx, 255)]);
    WmbVertex {
        position: Some(Vec3::new(pos[0], pos[1], pos[2])),
        position2: None,
        normal,
        float_normal: None,
        half_normal: Some([nrm[0], nrm[1], nrm[2], 0.0]),
        tangents: PackedTangents::from_xyzw(tan[0], tan[1], tan[2], tan[3]),
        mapping: uv,
        mapping2: Some(uv),
        mapping3: None,
        mapping4: None,
        mapping5: None,
        color: Some([255, 255, 255, 255]),
        bone_infos: Some(bi),
    }
}

/// Generate a closed box centered at `center` with half-extents `half` and bone `bone_idx`.
fn build_box_mesh(
    center: [f32; 3],
    half: [f32; 3],
    bone_idx: u8,
    wide_nrm: bool,
) -> (Vec<WmbVertex>, Vec<[usize; 3]>) {
    let faces: [([f32; 3], [f32; 4], [[f32; 3]; 4]); 6] = [
        // +Z front
        (
            [0.0, 0.0, 1.0],
            [1.0, 0.0, 0.0, 1.0],
            [
                [-1.0, 1.0, 1.0],
                [1.0, 1.0, 1.0],
                [1.0, -1.0, 1.0],
                [-1.0, -1.0, 1.0],
            ],
        ),
        // -Z back
        (
            [0.0, 0.0, -1.0],
            [-1.0, 0.0, 0.0, 1.0],
            [
                [1.0, 1.0, -1.0],
                [-1.0, 1.0, -1.0],
                [-1.0, -1.0, -1.0],
                [1.0, -1.0, -1.0],
            ],
        ),
        // +X right
        (
            [1.0, 0.0, 0.0],
            [0.0, 0.0, -1.0, 1.0],
            [
                [1.0, 1.0, 1.0],
                [1.0, 1.0, -1.0],
                [1.0, -1.0, -1.0],
                [1.0, -1.0, 1.0],
            ],
        ),
        // -X left
        (
            [-1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 1.0],
            [
                [-1.0, 1.0, -1.0],
                [-1.0, 1.0, 1.0],
                [-1.0, -1.0, 1.0],
                [-1.0, -1.0, -1.0],
            ],
        ),
        // +Y top
        (
            [0.0, 1.0, 0.0],
            [1.0, 0.0, 0.0, 1.0],
            [
                [-1.0, 1.0, -1.0],
                [1.0, 1.0, -1.0],
                [1.0, 1.0, 1.0],
                [-1.0, 1.0, 1.0],
            ],
        ),
        // -Y bottom
        (
            [0.0, -1.0, 0.0],
            [1.0, 0.0, 0.0, 1.0],
            [
                [-1.0, -1.0, 1.0],
                [1.0, -1.0, 1.0],
                [1.0, -1.0, -1.0],
                [-1.0, -1.0, -1.0],
            ],
        ),
    ];
    let uvs = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
    let mut verts = Vec::with_capacity(24);
    let mut tris = Vec::with_capacity(12);
    for (nrm, tan, corners) in faces {
        let base = verts.len();
        for (i, c) in corners.iter().enumerate() {
            let p = [
                center[0] + c[0] * half[0],
                center[1] + c[1] * half[1],
                center[2] + c[2] * half[2],
            ];
            verts.push(make_vertex(p, nrm, tan, uvs[i], bone_idx, wide_nrm));
        }
        tris.push([base, base + 2, base + 1]);
        tris.push([base, base + 3, base + 2]);
    }
    (verts, tris)
}

/// Generate a UV sphere centered at `center` with `radius` (offset toward +X per E.6).
fn build_sphere_mesh(
    center: [f32; 3],
    radius: f32,
    rings: usize,
    sectors: usize,
    bone_idx: u8,
    wide_nrm: bool,
) -> (Vec<WmbVertex>, Vec<[usize; 3]>) {
    let mut verts = Vec::new();
    let mut tris = Vec::new();
    for r in 0..=rings {
        let v = (r as f32) / (rings as f32);
        let phi = v * PI;
        let sin_p = phi.sin();
        let cos_p = phi.cos();
        for s in 0..=sectors {
            let u = (s as f32) / (sectors as f32);
            let theta = u * 2.0 * PI;
            let nx = sin_p * theta.cos();
            let ny = cos_p;
            let nz = sin_p * theta.sin();
            let pos = [
                center[0] + radius * nx,
                center[1] + radius * ny,
                center[2] + radius * nz,
            ];
            let tx = -theta.sin();
            let tz = theta.cos();
            verts.push(make_vertex(
                pos,
                [nx, ny, nz],
                [tx, 0.0, tz, 1.0],
                [u, v],
                bone_idx,
                wide_nrm,
            ));
        }
    }
    let stride = sectors + 1;
    for r in 0..rings {
        for s in 0..sectors {
            let i0 = r * stride + s;
            let i1 = i0 + 1;
            let i2 = (r + 1) * stride + s;
            let i3 = i2 + 1;
            if r != 0 {
                tris.push([i0, i2, i1]);
            }
            if r + 1 != rings {
                tris.push([i1, i2, i3]);
            }
        }
    }
    (verts, tris)
}

/// Generate a cylinder along +Z (`z_start..z_end`).
fn build_cylinder_z(
    center_xy: [f32; 2],
    z_start: f32,
    z_end: f32,
    radius: f32,
    segments: usize,
    bone_idx: u8,
    wide_nrm: bool,
) -> (Vec<WmbVertex>, Vec<[usize; 3]>) {
    let mut verts = Vec::new();
    let mut tris = Vec::new();
    for ring in 0..=1 {
        let z = if ring == 0 { z_start } else { z_end };
        let v = ring as f32;
        for s in 0..=segments {
            let u = (s as f32) / (segments as f32);
            let ang = u * 2.0 * PI;
            let nx = ang.cos();
            let ny = ang.sin();
            let pos = [
                center_xy[0] + radius * nx,
                center_xy[1] + radius * ny,
                z,
            ];
            verts.push(make_vertex(
                pos,
                [nx, ny, 0.0],
                [-ny, nx, 0.0, 1.0],
                [u, v],
                bone_idx,
                wide_nrm,
            ));
        }
    }
    let stride = segments + 1;
    for s in 0..segments {
        let i0 = s;
        let i1 = s + 1;
        let i2 = stride + s;
        let i3 = stride + s + 1;
        tris.push([i0, i2, i1]);
        tris.push([i1, i2, i3]);
    }
    (verts, tris)
}

/// Generate a flat alpha-cutout quad "wing" offset on -X/+Y.
fn build_wing_quad(bone_idx: u8, wide_nrm: bool) -> (Vec<WmbVertex>, Vec<[usize; 3]>) {
    let nrm = [0.0, 0.0, 1.0];
    let tan = [1.0, 0.0, 0.0, 1.0];
    let verts = vec![
        make_vertex([-1.1, 1.6, -0.15], nrm, tan, [0.0, 0.0], bone_idx, wide_nrm),
        make_vertex([-0.3, 1.6, -0.15], nrm, tan, [1.0, 0.0], bone_idx, wide_nrm),
        make_vertex([-0.3, 0.6, -0.15], nrm, tan, [1.0, 1.0], bone_idx, wide_nrm),
        make_vertex([-1.1, 0.6, -0.15], nrm, tan, [0.0, 1.0], bone_idx, wide_nrm),
    ];
    let tris = vec![[0, 2, 1], [0, 3, 2]];
    (verts, tris)
}

/// Generate the 4 procedural 64x64 RGBA8 textures required by Appendix E.6:
/// 0: Albedo with asymmetric 'F' + arrow marker and colored corners (TL=Red, TR=Green, BL=Blue, BR=Yellow)
/// 1: Tangent-space normal map with obvious concentric/diagonal bump pattern
/// 2: Spec/Mask texture with 4 distinct channel patterns (R=horizontal stripes, G=vertical stripes, B=checkerboard, A=radial gradient)
/// 3: Alpha-cutout wing texture with asymmetric L-shaped wing silhouette
fn build_demo_images() -> [RgbaImage; 4] {
    let dim = 64u32;

    // 0: Albedo with asymmetric "F" + arrow and colored corners
    let mut albedo = RgbaImage::new(dim, dim);
    for y in 0..dim {
        for x in 0..dim {
            let mut px = [210u8, 215, 225, 255];
            // Colored corners (top-row-first D3D UV convention: v=0 at y=0)
            if x < 12 && y < 12 {
                px = [235, 45, 45, 255]; // Top-left: Red
            } else if x >= dim - 12 && y < 12 {
                px = [45, 220, 65, 255]; // Top-right: Green
            } else if x < 12 && y >= dim - 12 {
                px = [50, 95, 240, 255]; // Bottom-left: Blue
            } else if x >= dim - 12 && y >= dim - 12 {
                px = [245, 215, 40, 255]; // Bottom-right: Yellow
            }
            // Large asymmetric 'F' in the left/center
            let in_f_stem = (16..22).contains(&x) && (12..52).contains(&y);
            let in_f_top = (16..38).contains(&x) && (12..18).contains(&y);
            let in_f_mid = (16..32).contains(&x) && (28..34).contains(&y);
            // Up-right arrow on the right side
            let in_arrow_shaft = (44..48).contains(&x) && (20..48).contains(&y);
            let in_arrow_head = y >= 16 && y < 26 && (x as i32 - 46).abs() <= (y as i32 - 16);
            if in_f_stem || in_f_top || in_f_mid {
                px = [25, 30, 45, 255];
            } else if in_arrow_shaft || in_arrow_head {
                px = [220, 85, 20, 255];
            }
            albedo.put_pixel(x, y, Rgba(px));
        }
    }

    // 1: Tangent-space normal map with obvious bevel/bump pattern
    let mut normal_map = RgbaImage::new(dim, dim);
    for y in 0..dim {
        for x in 0..dim {
            let fx = (x as f32) / (dim as f32) * 2.0 - 1.0;
            let fy = (y as f32) / (dim as f32) * 2.0 - 1.0;
            let r_sq = fx * fx + fy * fy;
            let (nx, ny, nz) = if r_sq < 0.55 {
                let gx = -fx * 0.65;
                let gy = -fy * 0.65;
                let gz = (1.0 - gx * gx - gy * gy).max(0.1).sqrt();
                (gx, gy, gz)
            } else if ((x / 8) + (y / 8)) % 2 == 0 {
                (0.25, 0.25, 0.935)
            } else {
                (-0.25, -0.25, 0.935)
            };
            let r = ((nx * 0.5 + 0.5) * 255.0).clamp(0.0, 255.0) as u8;
            let g = ((ny * 0.5 + 0.5) * 255.0).clamp(0.0, 255.0) as u8;
            let b = ((nz * 0.5 + 0.5) * 255.0).clamp(0.0, 255.0) as u8;
            normal_map.put_pixel(x, y, Rgba([r, g, b, 255]));
        }
    }

    // 2: Spec / Mask with 4 distinct channel patterns (R, G, B, A)
    let mut mask_map = RgbaImage::new(dim, dim);
    for y in 0..dim {
        for x in 0..dim {
            // R: horizontal stripes
            let r = if (y / 8) % 2 == 0 { 230u8 } else { 35u8 };
            // G: vertical stripes
            let g = if (x / 8) % 2 == 0 { 220u8 } else { 45u8 };
            // B: diagonal / checkerboard
            let b = if ((x / 16) ^ (y / 16)) & 1 == 0 {
                240u8
            } else {
                60u8
            };
            // A: radial ring
            let dx = (x as f32) - 32.0;
            let dy = (y as f32) - 32.0;
            let dist = (dx * dx + dy * dy).sqrt();
            let a = if (10.0..26.0).contains(&dist) {
                255u8
            } else {
                90u8
            };
            mask_map.put_pixel(x, y, Rgba([r, g, b, a]));
        }
    }

    // 3: Alpha-cutout asymmetric wing texture
    let mut cutout_map = RgbaImage::new(dim, dim);
    for y in 0..dim {
        for x in 0..dim {
            // Asymmetric wing blade shape
            let inside = x >= 8
                && x <= 56
                && y >= 6
                && y <= 58
                && (x + y / 2 <= 64)
                && !(x > 34 && y > 32);
            if inside {
                let edge = x < 14 || y < 12;
                let col = if edge {
                    [240u8, 175, 55, 255]
                } else {
                    [155u8, 70, 230, 255]
                };
                cutout_map.put_pixel(x, y, Rgba(col));
            } else {
                cutout_map.put_pixel(x, y, Rgba([0, 0, 0, 0]));
            }
        }
    }

    [albedo, normal_map, mask_map, cutout_map]
}

fn append_geometry_to_wmb(
    all_verts: &mut Vec<WmbVertex>,
    all_positions: &mut Vec<Vec3>,
    verts: Vec<WmbVertex>,
    tris: Vec<[usize; 3]>,
    mesh_id: i16,
    batch_id: i16,
    mat_id: usize,
    is_b2: bool,
    is_shadow: bool,
) -> WmbBatch {
    let base = all_verts.len();
    for v in verts {
        all_positions.push(v.position.unwrap_or_default());
        all_verts.push(v);
    }
    let abs_tris: Vec<[usize; 3]> = tris
        .into_iter()
        .map(|[a, b, c]| [a + base, b + base, c + base])
        .collect();
    let mut header = WmbBatchHeader {
        batch_id,
        mesh_id,
        flags: 0x8001,
        ex_mat_id: if is_b2 { mat_id as i16 } else { 0 },
        material_id: mat_id as u8,
        has_bone_refs: 1,
        u_e1: 0,
        u_e2: 0,
        vertex_start: 0,
        vertex_end: 0,
        primitive_type: 4,
        offset_indices: 0x60,
        num_indices: 0,
        vertex_offset: 0,
        u_f: [0; 7],
    };
    header.set_shadow_proxy(is_b2, is_shadow);
    let mut batch = WmbBatch {
        header,
        bone_refs: vec![0, 1, 2, 3],
        unknown_no_bones: [0.0; 4],
        indices: Vec::new(),
    };
    batch.set_triangles(&abs_tris);
    batch.header.offset_indices = batch.effective_indices_offset();
    batch
}

fn build_demo_wmb0_or_2(is_b2: bool) -> Result<(Vec<u8>, Vec<u8>)> {
    let images = build_demo_images();
    let mut wtb = WtbFile::new(Endian::Little, false);
    let tex_ids: [u32; 4] = if is_b2 {
        [0x1000_0001, 0x1000_0002, 0x1000_0003, 0x1000_0004]
    } else {
        [0, 1, 2, 3]
    };
    for (i, img) in images.iter().enumerate() {
        let dds = encode_rgba8_to_dds(img);
        let tid = if is_b2 { Some(tex_ids[i]) } else { None };
        wtb.push(dds, 0x2000_0004, tid)?;
    }
    let (wtb_bytes, _) = wtb.to_bytes();

    // Build 3 materials:
    // B1: 0 = type 110 (pls03), 1 = type 68 (pls01), 2 = type 66 (obs04 alpha-tested wing, with missing tex id 99 on envmap slot)
    // B2: 0 = sis00_bxxxn (body/head), 1 = chr00_bxxxx (arm), 2 = chs00_bxxxx (wing, with missing tex id 0xDEAD_BEEF)
    let mut materials = Vec::new();
    if !is_b2 {
        let mut m0 = WmbMaterial {
            mat_type: 110,
            flag: 0,
            shader_name: None,
            material_data: Vec::new(),
        };
        let mut d0 = switch_wmb_material_shader(&mut m0, 0, false, "110");
        for f in &mut d0.fields {
            if f.name == "Color_1" {
                f.sampler_tex_id = Some(0);
            } else if f.name == "reliefmap" {
                f.sampler_tex_id = Some(1);
            }
        }
        m0.apply_decoded(&d0);
        materials.push(m0);

        let mut m1 = WmbMaterial {
            mat_type: 68,
            flag: 0,
            shader_name: None,
            material_data: Vec::new(),
        };
        let mut d1 = switch_wmb_material_shader(&mut m1, 1, false, "68");
        for f in &mut d1.fields {
            if f.name == "Color_1" {
                f.sampler_tex_id = Some(0);
            } else if f.name == "Spec_Mask" {
                f.sampler_tex_id = Some(2);
            }
        }
        m1.apply_decoded(&d1);
        materials.push(m1);

        let mut m2 = WmbMaterial {
            mat_type: 66,
            flag: 0,
            shader_name: None,
            material_data: Vec::new(),
        };
        let mut d2 = switch_wmb_material_shader(&mut m2, 2, false, "66");
        for f in &mut d2.fields {
            if f.name == "Color_1" {
                f.sampler_tex_id = Some(3);
            } else if f.sampler_tex_id.is_some() {
                // Intentionally reference a texture id absent from the bank (E.6)
                f.sampler_tex_id = Some(99);
            }
        }
        m2.apply_decoded(&d2);
        materials.push(m2);
    } else {
        let shaders = ["Ois00_SBCEX", "Har00_SBXXX", "Alp00_SXXXX"];
        for (mi, sh) in shaders.iter().enumerate() {
            let mut m = WmbMaterial {
                mat_type: 0,
                flag: 0,
                shader_name: Some((*sh).to_string()),
                material_data: Vec::new(),
            };
            let mut dec = switch_wmb_material_shader(&mut m, mi, true, sh);
            for (si, f) in dec.fields.iter_mut().enumerate() {
                if f.sampler_tex_id.is_some() {
                    if mi == 2 && si == 0 {
                        f.sampler_tex_id = Some(tex_ids[3]);
                    } else if mi == 2 && si == 1 {
                        // Absent texture ID to exercise fallback (E.6)
                        f.sampler_tex_id = Some(0xDEAD_BEEF);
                    } else if si == 0 {
                        f.sampler_tex_id = Some(tex_ids[0]);
                    } else if si == 1 {
                        f.sampler_tex_id = Some(tex_ids[1]);
                    } else if si == 2 {
                        f.sampler_tex_id = Some(tex_ids[2]);
                    }
                }
            }
            m.apply_decoded(&dec);
            materials.push(m);
        }
    }

    // Build 2 mesh groups, 5 batches total (including 1 shadow proxy batch at the end of mesh 0)
    let mut all_verts = Vec::new();
    let mut all_positions = Vec::new();

    // Mesh 0 ("body"): body box (mat 0), head sphere offset +X (mat 0), low-poly shadow proxy of body (mat 0, shadow=true)
    let (body_v, body_t) = build_box_mesh([0.0, 1.0, 0.0], [0.45, 0.75, 0.25], 0, is_b2);
    let b_body = append_geometry_to_wmb(
        &mut all_verts,
        &mut all_positions,
        body_v,
        body_t,
        0,
        0,
        0,
        is_b2,
        false,
    );

    let (head_v, head_t) = build_sphere_mesh([0.35, 2.05, 0.0], 0.32, 6, 8, 1, is_b2);
    let b_head = append_geometry_to_wmb(
        &mut all_verts,
        &mut all_positions,
        head_v,
        head_t,
        0,
        1,
        0,
        is_b2,
        false,
    );

    let (sh_v, sh_t) = build_box_mesh([0.0, 1.0, 0.0], [0.42, 0.72, 0.22], 0, is_b2);
    let b_shadow = append_geometry_to_wmb(
        &mut all_verts,
        &mut all_positions,
        sh_v,
        sh_t,
        0,
        2,
        0,
        is_b2,
        true,
    );

    // Mesh 1 ("accessories"): cylinder arm along +Z (mat 1), alpha-cutout wing quad (mat 2)
    let (arm_v, arm_t) = build_cylinder_z([0.55, 1.25], 0.0, 0.95, 0.14, 8, 2, is_b2);
    let b_arm = append_geometry_to_wmb(
        &mut all_verts,
        &mut all_positions,
        arm_v,
        arm_t,
        1,
        0,
        1,
        is_b2,
        false,
    );

    let (wing_v, wing_t) = build_wing_quad(3, is_b2);
    let b_wing = append_geometry_to_wmb(
        &mut all_verts,
        &mut all_positions,
        wing_v,
        wing_t,
        1,
        1,
        2,
        is_b2,
        false,
    );

    let meshes = vec![
        WmbMesh {
            id: 0,
            u_a1: 0,
            u_a2: 0,
            offset_batch_offsets: 0x80,
            u_b: 0,
            u_c: [0; 4],
            name: "body".to_string(),
            bounding_matrix: [0.0, 1.0, 0.0, 1.5, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0],
            batches: vec![b_body, b_head, b_shadow],
        },
        WmbMesh {
            id: 1,
            u_a1: 0,
            u_a2: 0,
            offset_batch_offsets: 0x80,
            u_b: 0,
            u_c: [0; 4],
            name: "accessories".to_string(),
            bounding_matrix: [0.0, 1.2, 0.3, 1.5, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0],
            batches: vec![b_arm, b_wing],
        },
    ];

    // 4 bones in a chain with distinct local (0..3) and global (10, 25, 42, 77) IDs
    let bone_hierarchy = vec![-1i16, 0, 1, 2];
    let bone_positions = vec![
        Vec3::new(0.0, 0.0, 0.0),
        Vec3::new(0.0, 1.0, 0.0),
        Vec3::new(0.35, 2.05, 0.0),
        Vec3::new(0.55, 1.25, 0.45),
    ];
    let mut btt = BoneIndexTranslateTable::new();
    btt.table.insert(10, 0);
    btt.table.insert(25, 1);
    btt.table.insert(42, 2);
    btt.table.insert(77, 3);

    let mut wmb = WmbFile {
        endian: Endian::Little,
        header: WmbHeader {
            u_a: 0,
            vertex_flags: 0x4800_001F,
            num_vertexes: all_verts.len() as i32,
            vertex_ex_data_size: 1,
            vertex_ex_data: 1,
            u_e: 0,
            offset_positions: 0,
            offset_vertexes: 0,
            offset_vertexes_ex_data: 0,
            u_g: [0; 4],
            num_bones: 4,
            offset_bone_hierarchy: 0,
            offset_bone_relative_position: 0,
            offset_bone_position: 0,
            offset_bone_index_translate_table: 0,
            num_materials: materials.len() as i32,
            offset_materials_offsets: 0,
            offset_materials: 0,
            num_meshes: meshes.len() as i32,
            offset_meshes_offsets: 0,
            offset_meshes: 0,
            u_k: 0,
            u_l: 0,
            offset_u_j: 0,
            offset_bone_symmetries: 0,
            offset_bone_flags: 0,
            offset_shader_names: if is_b2 { 1 } else { 0 },
            offset_tex_infos: if is_b2 { 1 } else { 0 },
            u_m: 0,
            u_n: 0,
        },
        positions: Vec::new(),
        vertexes: all_verts,
        bone_hierarchy,
        bone_relative_positions: vec![Vec3::ZERO; 4],
        bone_positions,
        bone_index_translate_table: btt,
        u_j_raw: None,
        bone_symmetries: vec![-1; 4],
        bone_flags: vec![5; 4],
        tex_infos: if is_b2 {
            tex_ids.iter().map(|&id| (id, 0)).collect()
        } else {
            Vec::new()
        },
        materials,
        meshes,
    };
    wmb.recompute_relative_positions();
    let wmb_bytes = wmb.to_bytes(Endian::Little)?;
    Ok((wmb_bytes, wtb_bytes))
}

fn build_demo_wmb3() -> Result<(Vec<u8>, Vec<u8>)> {
    let images = build_demo_images();
    let mut wtb = WtbFile::new(Endian::Little, false);
    let tex_ids: [u32; 4] = [0x3000_0001, 0x3000_0002, 0x3000_0003, 0x3000_0004];
    for (i, img) in images.iter().enumerate() {
        let xt1 = encode_rgba_to_xt1(img, TextureFormat::Bc3, false, false, false, None);
        wtb.push(xt1, 0x8000_0000, Some(tex_ids[i]))?;
    }
    let (wtb_bytes, _) = wtb.to_bytes();

    // Vertex Group 0 (flags=7, primary=28B, ex=12B): LOD 0 batches + LOD 1 body batch
    // Vertex Group 1 (flags=0, primary=28B, ex=0B — different layout!): LOD 1 arm batch
    let mut vg0_verts = Vec::new();
    let mut vg0_indices = Vec::new();
    let mut vg1_verts = Vec::new();
    let mut vg1_indices = Vec::new();

    let mut push_to_vg = |vg_idx: u32,
                          verts: Vec<WmbVertex>,
                          tris: Vec<[usize; 3]>|
     -> Wmb3Batch {
        let (target_v, target_i) = if vg_idx == 0 {
            (&mut vg0_verts, &mut vg0_indices)
        } else {
            (&mut vg1_verts, &mut vg1_indices)
        };
        let v_start = target_v.len() as u32;
        let i_start = target_i.len() as u32;
        let num_v = verts.len() as u32;
        target_v.extend(verts);
        for [a, b, c] in &tris {
            target_i.push(*a as u32);
            target_i.push(*b as u32);
            target_i.push(*c as u32);
        }
        let num_i = (tris.len() * 3) as u32;
        Wmb3Batch {
            vertex_group_index: vg_idx,
            bone_set_index: 0,
            vertex_start: v_start,
            index_start: i_start,
            num_vertexes: num_v,
            num_indices: num_i,
            num_primitives: tris.len() as u32,
        }
    };

    // LOD 0 batches (4 batches across mesh 0 and mesh 1)
    let (body_v, body_t) = build_box_mesh([0.0, 1.0, 0.0], [0.45, 0.75, 0.25], 0, true);
    let b0_body = push_to_vg(0, body_v, body_t);

    let (head_v, head_t) = build_sphere_mesh([0.35, 2.05, 0.0], 0.32, 6, 8, 1, true);
    let b1_head = push_to_vg(0, head_v, head_t);

    let (arm_v, arm_t) = build_cylinder_z([0.55, 1.25], 0.0, 0.95, 0.14, 8, 2, true);
    let b2_arm = push_to_vg(0, arm_v, arm_t);

    let (wing_v, wing_t) = build_wing_quad(3, true);
    let b3_wing = push_to_vg(0, wing_v, wing_t);

    // LOD 1 batches (2 simplified batches, second one in VG 1 with a different layout!)
    let (lod1_body_v, lod1_body_t) = build_box_mesh([0.0, 1.0, 0.0], [0.44, 0.74, 0.24], 0, true);
    let b4_lod1_body = push_to_vg(0, lod1_body_v, lod1_body_t);

    let (lod1_arm_v, lod1_arm_t) = build_cylinder_z([0.55, 1.25], 0.0, 0.95, 0.14, 4, 2, true);
    let b5_lod1_arm = push_to_vg(1, lod1_arm_v, lod1_arm_t);

    let vertex_groups = vec![
        Wmb3VertexGroup {
            offset_vertexes: 0,
            offset_vertexes_ex_data: 0,
            u_a: 0,
            u_b: 0,
            vertex_size: 28,
            vertex_ex_data_size: 12,
            u_c: 0,
            u_d: 0,
            num_vertexes: vg0_verts.len() as u32,
            vertex_flags: 7,
            offset_indices: 0,
            num_indices: vg0_indices.len() as u32,
            vertexes: vg0_verts,
            indices: vg0_indices,
            raw_vertex_bytes: Vec::new(),
            raw_ex_bytes: Vec::new(),
        },
        Wmb3VertexGroup {
            offset_vertexes: 0,
            offset_vertexes_ex_data: 0,
            u_a: 0,
            u_b: 0,
            vertex_size: 28,
            vertex_ex_data_size: 0,
            u_c: 0,
            u_d: 0,
            num_vertexes: vg1_verts.len() as u32,
            vertex_flags: 0,
            offset_indices: 0,
            num_indices: vg1_indices.len() as u32,
            vertexes: vg1_verts,
            indices: vg1_indices,
            raw_vertex_bytes: Vec::new(),
            raw_ex_bytes: Vec::new(),
        },
    ];

    // 4 Materials for B3:
    // 0: PBS00_XXXXX (body/head: albedo + normal + mask)
    // 1: PBS00_XXXXX (arm: albedo + mask + absent normal map 0xDEAD_0003)
    // 2: PBS00_XXXXX (wing: cutout albedo)
    // 3: UNIMPLEMENTED_DEMO00 (deliberately unimplemented shader name to exercise fallback badge)
    let make_b3_mat = |name: &str, shader: &str, texs: Vec<(&str, u32)>| -> Wmb3Material {
        let mut m = Wmb3Material {
            date: [2026, 10, 29, 0],
            name: name.to_string(),
            shader_name: shader.to_string(),
            technique_name: "Default".to_string(),
            u_a: 1,
            textures: texs.into_iter().map(|(k, v)| (k.to_string(), v)).collect(),
            parameter_groups: Vec::new(),
            variables: Vec::new(),
        };
        switch_wmb3_material_shader(&mut m, shader);
        m
    };

    // [wmb-studio BEGIN phase3]
    let materials = vec![
        make_b3_mat(
            "mat_body_pbs",
            "PBS00_XXXXX",
            vec![
                ("g_AlbedoMap", tex_ids[0]),
                ("g_MaskMap", tex_ids[2]),
                ("g_NormalMap", tex_ids[1]),
            ],
        ),
        make_b3_mat(
            "mat_arm_bayo00",
            "BAYO00_XXXXX",
            vec![
                ("g_AlbedoMap1", tex_ids[0]),
                ("g_NormalMap", tex_ids[1]),
                ("g_MaskMap", tex_ids[2]),
                ("g_NormalMap3", 0xDEAD_0003), // intentionally absent from bank
            ],
        ),
        make_b3_mat(
            "mat_wing_hair20",
            "Hair20_XXXXX",
            vec![
                ("g_AlbedoMap1", tex_ids[3]),
                ("g_NormalMap", tex_ids[1]),
                ("g_NormalMap2", tex_ids[2]),
            ],
        ),
        make_b3_mat(
            "mat_unimplemented_demo",
            "CUSTOM_UNIMPLEMENTED_99",
            vec![("g_AlbedoMap", tex_ids[0])],
        ),
    ];
    // [wmb-studio END]

    let lods = vec![
        Wmb3Lod {
            name: "lod0".to_string(),
            lod_level: 0,
            batch_start: 0,
            batch_infos: vec![
                Wmb3LodBatchInfo {
                    vertex_group_index: 0,
                    mesh_index: 0,
                    material_index: 0,
                    call_tree_node_index: -1,
                    mesh_material_index: 0,
                    u_b: 0,
                },
                Wmb3LodBatchInfo {
                    vertex_group_index: 0,
                    mesh_index: 0,
                    material_index: 0,
                    call_tree_node_index: -1,
                    mesh_material_index: 0,
                    u_b: 0,
                },
                Wmb3LodBatchInfo {
                    vertex_group_index: 0,
                    mesh_index: 1,
                    material_index: 1,
                    call_tree_node_index: -1,
                    mesh_material_index: 1,
                    u_b: 0,
                },
                Wmb3LodBatchInfo {
                    vertex_group_index: 0,
                    mesh_index: 1,
                    material_index: 2,
                    call_tree_node_index: -1,
                    mesh_material_index: 2,
                    u_b: 0,
                },
            ],
        },
        Wmb3Lod {
            name: "lod1".to_string(),
            lod_level: 1,
            batch_start: 4,
            batch_infos: vec![
                Wmb3LodBatchInfo {
                    vertex_group_index: 0,
                    mesh_index: 0,
                    material_index: 0,
                    call_tree_node_index: -1,
                    mesh_material_index: 0,
                    u_b: 0,
                },
                Wmb3LodBatchInfo {
                    vertex_group_index: 1,
                    mesh_index: 1,
                    material_index: 3,
                    call_tree_node_index: -1,
                    mesh_material_index: 3,
                    u_b: 0,
                },
            ],
        },
    ];

    let meshes = vec![
        Wmb3Mesh {
            name: "body".to_string(),
            bounding_box: [-0.5, 0.2, -0.35, 0.7, 2.4, 0.35],
            materials: vec![0],
            bone_indices: vec![0, 1],
        },
        Wmb3Mesh {
            name: "accessories".to_string(),
            bounding_box: [-1.1, 0.6, -0.2, 0.7, 1.6, 1.0],
            materials: vec![1, 2, 3],
            bone_indices: vec![2, 3],
        },
    ];

    let bones = vec![
        Wmb3Bone {
            id: 10,
            parent_index: -1,
            local_position: Vec3::new(0.0, 0.0, 0.0),
            local_rotation: Vec3::ZERO,
            local_scale: Vec3::ONE,
            position: Vec3::new(0.0, 0.0, 0.0),
            rotation: Vec3::ZERO,
            scale: Vec3::ONE,
            t_position: Vec3::new(0.0, 0.0, 0.0),
        },
        Wmb3Bone {
            id: 25,
            parent_index: 0,
            local_position: Vec3::new(0.0, 1.0, 0.0),
            local_rotation: Vec3::ZERO,
            local_scale: Vec3::ONE,
            position: Vec3::new(0.0, 1.0, 0.0),
            rotation: Vec3::ZERO,
            scale: Vec3::ONE,
            t_position: Vec3::new(0.0, 1.0, 0.0),
        },
        Wmb3Bone {
            id: 42,
            parent_index: 1,
            local_position: Vec3::new(0.35, 1.05, 0.0),
            local_rotation: Vec3::ZERO,
            local_scale: Vec3::ONE,
            position: Vec3::new(0.35, 2.05, 0.0),
            rotation: Vec3::ZERO,
            scale: Vec3::ONE,
            t_position: Vec3::new(0.35, 2.05, 0.0),
        },
        Wmb3Bone {
            id: 77,
            parent_index: 2,
            local_position: Vec3::new(0.20, -0.80, 0.45),
            local_rotation: Vec3::ZERO,
            local_scale: Vec3::ONE,
            position: Vec3::new(0.55, 1.25, 0.45),
            rotation: Vec3::ZERO,
            scale: Vec3::ONE,
            t_position: Vec3::new(0.55, 1.25, 0.45),
        },
    ];

    let mut btt = BoneIndexTranslateTable::new();
    btt.table.insert(10, 0);
    btt.table.insert(25, 1);
    btt.table.insert(42, 2);
    btt.table.insert(77, 3);

    let wmb3 = Wmb3File {
        version: 0x2020_0824,
        u_a: 0,
        flags: 0x0,
        u_c: 0,
        bounding_box: [-1.1, 0.0, -0.35, 0.7, 2.4, 1.0],
        bones,
        bone_index_translate_table: btt,
        raw_btt: Vec::new(),
        vertex_groups,
        batches: vec![
            b0_body,
            b1_head,
            b2_arm,
            b3_wing,
            b4_lod1_body,
            b5_lod1_arm,
        ],
        lods,
        col_tree_nodes: Vec::new(),
        bone_map: vec![0, 1, 2, 3],
        bone_sets: vec![vec![0, 1, 2, 3]],
        materials,
        meshes,
        mesh_material_pairs: vec![(0, 0), (1, 1), (1, 2), (1, 3)],
        world_data: Vec::new(),
        tail: [0, 0],
    };

    let wmb3_bytes = wmb3.to_bytes()?;
    Ok((wmb3_bytes, wtb_bytes))
}

/// Build a self-contained `.dat` container holding `demo.wmb` and `demo.wtb` for `b1`, `b2`, or `b3`.
pub fn build_demo_dat(game: &str) -> Result<Vec<u8>> {
    let g = game.trim().to_ascii_lowercase();
    let (wmb_bytes, wtb_bytes) = match g.as_str() {
        "b1" | "bayo1" | "wmb1" => build_demo_wmb0_or_2(false)?,
        "b2" | "bayo2" | "wmb2" => build_demo_wmb0_or_2(true)?,
        "b3" | "bayo3" | "wmb3" => build_demo_wmb3()?,
        other => {
            return Err(PgError::Unsupported(format!(
                "Unsupported demo model game '{other}' (expected b1, b2, or b3)"
            )))
        }
    };

    let mut dat = DatFile::new(Endian::Little);
    dat.hash_map = Some(crate::dat::DatHashMap {
        pre_hash_shift: 30,
        hashes: Vec::new(),
    });
    dat.push("demo.wmb", wmb_bytes)?;
    dat.push("demo.wtb", wtb_bytes)?;
    dat.rebuild_hash_map();
    Ok(dat.to_bytes())
}

#[allow(dead_code)]
fn _assert_btree_used(_: BTreeMap<u32, u32>) {}
