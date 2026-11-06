//! Comprehensive PlatinumGames Shader & Material Database (`Bayonetta 1`, `Bayonetta 2`, `Bayonetta 3` / `NieR:Automata`),
//! Core Engine Pretexture (`pre_texture.wtb` / `pre_texture.wta`) registry, layout-preserving shader switcher,
//! and Cross-Game Material Converter (`B1 <-> B2 <-> B3`), ported from `Bayonetta2Blender2Bayonetta-v24.9.3`.

use crate::databases::{lookup_material_type, MaterialParamType, MATERIAL_DATABASE};
use crate::wmb::{DecodedMaterialParameter, DecodedWmbMaterial, WmbMaterial};
use crate::wmb3::Wmb3Material;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::OnceLock;

const B2_SHADER_LAYOUTS_JSON: &str = include_str!("data/b2_shader_layouts.json");
const B3_SHADER_PARAMS_JSON: &str = include_str!("data/shader_params_bayo3.json");
const XCONV_DATA_JSON: &str = include_str!("data/xconv_data.json");

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct B2TextureSlotSpec {
    pub slot: usize,
    #[serde(default)]
    pub meaning: String,
    #[serde(default)]
    pub role: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct B2ParamVec4Spec {
    pub index: usize,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub components: Vec<Option<String>>,
    #[serde(default)]
    pub confidence: String,
    #[serde(default)]
    pub note: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct B2ShaderLayout {
    #[serde(default)]
    pub family: String,
    #[serde(default)]
    pub family_meaning: String,
    #[serde(default)]
    pub flags: String,
    #[serde(default)]
    pub letters: Vec<String>,
    #[serde(default)]
    pub tex_count: usize,
    #[serde(default)]
    pub mat_vec4: usize,
    #[serde(default)]
    pub matrix_blocks: usize,
    #[serde(default)]
    pub textures: Vec<B2TextureSlotSpec>,
    #[serde(default)]
    pub params: Vec<B2ParamVec4Spec>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CorePretextureInfo {
    #[serde(alias = "tex_id")]
    pub id: u32,
    #[serde(alias = "tex_id_hex")]
    pub id_hex: String,
    pub name: String,
    pub role: String,
    pub description: String,
    pub game: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShaderOptionDesc {
    #[serde(alias = "id")]
    pub key: String,
    pub label: String,
    pub family: String,
    pub summary: String,
    pub mat_type: Option<i16>,
}

static B2_LAYOUTS: OnceLock<BTreeMap<String, (String, B2ShaderLayout)>> = OnceLock::new();
static B3_PARAMS: OnceLock<BTreeMap<String, Vec<String>>> = OnceLock::new();
static B3_PARAM_GROUPS: OnceLock<BTreeMap<String, BTreeMap<usize, Vec<String>>>> = OnceLock::new();
static XCONV_DATA: OnceLock<serde_json::Value> = OnceLock::new();

pub fn b2_shader_map() -> &'static BTreeMap<String, (String, B2ShaderLayout)> {
    B2_LAYOUTS.get_or_init(|| {
        let mut map = BTreeMap::new();
        if let Ok(raw) = serde_json::from_str::<BTreeMap<String, B2ShaderLayout>>(B2_SHADER_LAYOUTS_JSON)
        {
            for (k, v) in raw {
                map.insert(k.to_ascii_lowercase(), (k, v));
            }
        }
        map
    })
}

pub fn lookup_b2_shader(shader_name: &str) -> Option<&'static (String, B2ShaderLayout)> {
    let key = shader_name.trim().trim_matches('\0').to_ascii_lowercase();
    b2_shader_map().get(&key)
}

pub fn b3_shader_params_map() -> &'static BTreeMap<String, Vec<String>> {
    B3_PARAMS.get_or_init(|| {
        let mut map = BTreeMap::new();
        if let Ok(raw) = serde_json::from_str::<BTreeMap<String, serde_json::Value>>(B3_SHADER_PARAMS_JSON)
        {
            for (k, v) in raw {
                let params: Vec<String> = v
                    .get("Parameters")
                    .and_then(|p| p.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|x| x.as_str().map(|s| s.to_string()))
                            .collect()
                    })
                    .unwrap_or_default();
                map.insert(k, params);
            }
        }
        map
    })
}

pub fn b3_shader_param_groups_map() -> &'static BTreeMap<String, BTreeMap<usize, Vec<String>>> {
    B3_PARAM_GROUPS.get_or_init(|| {
        let mut map = BTreeMap::new();
        if let Ok(raw) = serde_json::from_str::<BTreeMap<String, serde_json::Value>>(B3_SHADER_PARAMS_JSON)
        {
            for (k, v) in raw {
                let mut groups = BTreeMap::new();
                if let Some(p0) = v.get("Parameters").and_then(|p| p.as_array()) {
                    let arr: Vec<String> = p0
                        .iter()
                        .filter_map(|x| x.as_str().map(|s| s.to_string()))
                        .collect();
                    groups.insert(0, arr);
                }
                if let Some(pg_obj) = v.get("ParameterGroups").and_then(|p| p.as_object()) {
                    for (gk, gv) in pg_obj {
                        if let (Ok(gi), Some(g_arr)) = (gk.parse::<usize>(), gv.as_array()) {
                            let arr: Vec<String> = g_arr
                                .iter()
                                .filter_map(|x| x.as_str().map(|s| s.to_string()))
                                .collect();
                            groups.insert(gi, arr);
                        }
                    }
                }
                map.insert(k, groups);
            }
        }
        map
    })
}

pub fn lookup_b3_shader_group_params(shader_name: &str, group_index: usize) -> Option<&'static [String]> {
    let clean = shader_name.trim().trim_matches('\0');
    let map = b3_shader_param_groups_map();
    if let Some(gmap) = map.get(clean) {
        if let Some(v) = gmap.get(&group_index) {
            return Some(v.as_slice());
        }
    }
    if let Some((_, gmap)) = map.iter().find(|(k, _)| k.eq_ignore_ascii_case(clean)) {
        if let Some(v) = gmap.get(&group_index) {
            return Some(v.as_slice());
        }
    }
    let prefix = clean.split('_').next().unwrap_or(clean);
    let wildcard = format!("{prefix}_XXXXX");
    map.iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(&wildcard))
        .and_then(|(_, gmap)| gmap.get(&group_index))
        .map(|v| v.as_slice())
}

pub fn lookup_b3_shader_params(shader_name: &str) -> Option<&'static [String]> {
    lookup_b3_shader_group_params(shader_name, 0)
}

pub fn xconv_data() -> &'static serde_json::Value {
    XCONV_DATA.get_or_init(|| {
        serde_json::from_str(XCONV_DATA_JSON).unwrap_or(serde_json::Value::Null)
    })
}

/// Built-in core engine textures (`pre_texture.wtb` / `pre_texture.wta`) for Bayonetta 1, 2, and 3.
pub fn list_core_pretextures(game: &str) -> Vec<CorePretextureInfo> {
    match game.to_ascii_lowercase().as_str() {
        "b1" | "wmb1" => vec![
            CorePretextureInfo { id: 0x80000000, id_hex: "0x80000000".into(), name: "pre_blank (4x4 white)".into(), role: "albedo2".into(), description: "Unused Color_2 / relief fallback".into(), game: "b1".into() },
            CorePretextureInfo { id: 0x80000001, id_hex: "0x80000001".into(), name: "pre_cubemap".into(), role: "cube".into(), description: "Default environment cubemap".into(), game: "b1".into() },
            CorePretextureInfo { id: 0x80000002, id_hex: "0x80000002".into(), name: "pre_relief_grey".into(), role: "normal".into(), description: "Grey grain relief map (matches B2 pre_weather)".into(), game: "b1".into() },
            CorePretextureInfo { id: 0x80000003, id_hex: "0x80000003".into(), name: "pre_cubemap_small".into(), role: "cube".into(), description: "16x16 environment cubemap".into(), game: "b1".into() },
            CorePretextureInfo { id: 0x80000004, id_hex: "0x80000004".into(), name: "pre_white / pre_lightmap".into(), role: "albedo".into(), description: "Opaque white / neutral lightmap".into(), game: "b1".into() },
            CorePretextureInfo { id: 0x80000005, id_hex: "0x80000005".into(), name: "pre_normalmap".into(), role: "normal".into(), description: "Neutral tangent-space normal map".into(), game: "b1".into() },
            CorePretextureInfo { id: 0x80000006, id_hex: "0x80000006".into(), name: "pre_pow (spec LUT)".into(), role: "spec_lut".into(), description: "Specular power look-up table".into(), game: "b1".into() },
            CorePretextureInfo { id: 0x80000007, id_hex: "0x80000007".into(), name: "pre_spechard_cube".into(), role: "cube".into(), description: "Hard specular environment cubemap".into(), game: "b1".into() },
            CorePretextureInfo { id: 0x80000008, id_hex: "0x80000008".into(), name: "pre_specsoft_cube".into(), role: "cube".into(), description: "Soft specular environment cubemap".into(), game: "b1".into() },
            CorePretextureInfo { id: 0x80000009, id_hex: "0x80000009".into(), name: "pre_refhard_cube".into(), role: "cube".into(), description: "Hard reflection environment cubemap".into(), game: "b1".into() },
            CorePretextureInfo { id: 0x8000000A, id_hex: "0x8000000A".into(), name: "pre_refsoft_cube".into(), role: "cube".into(), description: "Soft reflection environment cubemap".into(), game: "b1".into() },
            CorePretextureInfo { id: 0x8000000B, id_hex: "0x8000000B".into(), name: "pre_relief_default".into(), role: "normal".into(), description: "Default B1 relief map".into(), game: "b1".into() },
        ],
        "b3" | "wmb3" => vec![
            CorePretextureInfo { id: 0x5864A24F, id_hex: "0x5864A24F".into(), name: "pre_white".into(), role: "albedo".into(), description: "Opaque white swatch".into(), game: "b3".into() },
            CorePretextureInfo { id: 0x132ED5C3, id_hex: "0x132ED5C3".into(), name: "pre_black".into(), role: "albedo2".into(), description: "Opaque black swatch".into(), game: "b3".into() },
            CorePretextureInfo { id: 0x62FC2788, id_hex: "0x62FC2788".into(), name: "pre_grey50".into(), role: "albedo".into(), description: "50% grey swatch".into(), game: "b3".into() },
            CorePretextureInfo { id: 0x4E9C16F4, id_hex: "0x4E9C16F4".into(), name: "pre_lightmap".into(), role: "lightmap".into(), description: "Neutral white lightmap".into(), game: "b3".into() },
            CorePretextureInfo { id: 0x7FD4929A, id_hex: "0x7FD4929A".into(), name: "pre_normalmap".into(), role: "normal".into(), description: "Neutral tangent-space normal map".into(), game: "b3".into() },
            CorePretextureInfo { id: 0x3176288C, id_hex: "0x3176288C".into(), name: "pre_normalmapflat".into(), role: "normal".into(), description: "Exact flat normal map (B3)".into(), game: "b3".into() },
            CorePretextureInfo { id: 0x6A9BFB55, id_hex: "0x6A9BFB55".into(), name: "pre_maskmap".into(), role: "mask".into(), description: "Default PBR mask map (0, 0, 1)".into(), game: "b3".into() },
            CorePretextureInfo { id: 0x1FBC0984, id_hex: "0x1FBC0984".into(), name: "pre_cubemap".into(), role: "cube".into(), description: "Default environment cubemap".into(), game: "b3".into() },
            CorePretextureInfo { id: 0x3F44A76D, id_hex: "0x3F44A76D".into(), name: "pre_cubemap_128".into(), role: "cube".into(), description: "128x128 environment cubemap".into(), game: "b3".into() },
            CorePretextureInfo { id: 0x32A26F52, id_hex: "0x32A26F52".into(), name: "pre_subsurfacelut".into(), role: "sss_lut".into(), description: "Subsurface scattering LUT".into(), game: "b3".into() },
            CorePretextureInfo { id: 0x40EC4FF8, id_hex: "0x40EC4FF8".into(), name: "pre_dithernoise".into(), role: "extra".into(), description: "Dither noise pattern".into(), game: "b3".into() },
        ],
        _ => vec![
            CorePretextureInfo { id: 0x5864A24F, id_hex: "0x5864A24F".into(), name: "pre_white".into(), role: "albedo".into(), description: "Opaque white (core pre_texture.wta)".into(), game: "b2".into() },
            CorePretextureInfo { id: 0x132ED5C3, id_hex: "0x132ED5C3".into(), name: "pre_black".into(), role: "albedo2".into(), description: "Opaque black (core pre_texture.wta)".into(), game: "b2".into() },
            CorePretextureInfo { id: 0x4E9C16F4, id_hex: "0x4E9C16F4".into(), name: "pre_lightmap".into(), role: "lightmap".into(), description: "White lightmap / neutral mask".into(), game: "b2".into() },
            CorePretextureInfo { id: 0x7FD4929A, id_hex: "0x7FD4929A".into(), name: "pre_normalmap".into(), role: "normal".into(), description: "Neutral tangent-space normal map".into(), game: "b2".into() },
            CorePretextureInfo { id: 0x31184D4B, id_hex: "0x31184D4B".into(), name: "pre_2ndmap".into(), role: "albedo2".into(), description: "Neutral second colour map".into(), game: "b2".into() },
            CorePretextureInfo { id: 0x2C579F2B, id_hex: "0x2C579F2B".into(), name: "pre_weather".into(), role: "normal".into(), description: "Stage detail/weather normal map".into(), game: "b2".into() },
            CorePretextureInfo { id: 0x5CDFEA4E, id_hex: "0x5CDFEA4E".into(), name: "pre_noImage".into(), role: "extra".into(), description: "Fallback placeholder texture".into(), game: "b2".into() },
            CorePretextureInfo { id: 0x0DA7BFF1, id_hex: "0x0DA7BFF1".into(), name: "pre_pow".into(), role: "spec_lut".into(), description: "Character specular power LUT".into(), game: "b2".into() },
            CorePretextureInfo { id: 0x50E2DB38, id_hex: "0x50E2DB38".into(), name: "pre_pow2".into(), role: "spec_lut".into(), description: "Stage specular power LUT".into(), game: "b2".into() },
            CorePretextureInfo { id: 0x1FBC0984, id_hex: "0x1FBC0984".into(), name: "pre_cubemap".into(), role: "cube".into(), description: "Default environment cubemap".into(), game: "b2".into() },
            CorePretextureInfo { id: 0x1FC3A7B0, id_hex: "0x1FC3A7B0".into(), name: "pre_cubemap_hard".into(), role: "cube".into(), description: "Hard environment cubemap".into(), game: "b2".into() },
            CorePretextureInfo { id: 0x215C9D15, id_hex: "0x215C9D15".into(), name: "pre_cubemap_soft".into(), role: "cube".into(), description: "Soft environment cubemap".into(), game: "b2".into() },
            CorePretextureInfo { id: 0x33F404A9, id_hex: "0x33F404A9".into(), name: "pre_cubemap_normal".into(), role: "cube".into(), description: "Normal environment cubemap".into(), game: "b2".into() },
            CorePretextureInfo { id: 0x3619F83E, id_hex: "0x3619F83E".into(), name: "pre_refhard_cube".into(), role: "cube".into(), description: "Hard reflection cubemap".into(), game: "b2".into() },
            CorePretextureInfo { id: 0x1D418CE6, id_hex: "0x1D418CE6".into(), name: "pre_refsoft_cube".into(), role: "cube".into(), description: "Soft reflection cubemap".into(), game: "b2".into() },
            CorePretextureInfo { id: 0x750D0069, id_hex: "0x750D0069".into(), name: "pre_spechard_cube".into(), role: "cube".into(), description: "Hard specular cubemap".into(), game: "b2".into() },
            CorePretextureInfo { id: 0x5E5574B1, id_hex: "0x5E5574B1".into(), name: "pre_specsoft_cube".into(), role: "cube".into(), description: "Soft specular cubemap".into(), game: "b2".into() },
            CorePretextureInfo { id: 0x6CF4D190, id_hex: "0x6CF4D190".into(), name: "pre_noiseMix".into(), role: "extra".into(), description: "Noise mix texture".into(), game: "b2".into() },
            CorePretextureInfo { id: 0x0D8502B7, id_hex: "0x0D8502B7".into(), name: "pre_noiseN".into(), role: "normal".into(), description: "Noise normal map".into(), game: "b2".into() },
        ],
    }
}

pub fn lookup_pretexture_label(game: &str, id: u32) -> Option<String> {
    for p in list_core_pretextures(game) {
        if p.id == id {
            return Some(p.name);
        }
    }
    if game == "b1" && id >= 0x8000_0000 {
        return Some(format!("pre_texture[{}]", id & 0xFFFF));
    }
    None
}

pub fn list_available_shaders(game: &str) -> Vec<ShaderOptionDesc> {
    match game.to_ascii_lowercase().as_str() {
        "b1" | "wmb1" => MATERIAL_DATABASE
            .iter()
            .map(|m| {
                let sh = m.shader.unwrap_or("unmapped_type");
                let samplers = m.layout.iter().filter(|f| f.param_type.is_sampler()).count();
                let vec4s = m.layout.len().saturating_sub(samplers);
                ShaderOptionDesc {
                    key: m.mat_type.to_string(),
                    label: format!("Type {} (0x{:02X}) — {}", m.mat_type, m.mat_type, sh),
                    family: sh.get(11..14).unwrap_or("b1").to_uppercase(),
                    summary: format!("{} B · {} samplers · {} vec4s", m.byte_size, samplers, vec4s),
                    mat_type: Some(m.mat_type),
                }
            })
            .collect(),
        "b3" | "wmb3" => b3_shader_params_map()
            .iter()
            .map(|(name, params)| {
                let fam = name.split('_').next().unwrap_or(name).to_string();
                ShaderOptionDesc {
                    key: name.clone(),
                    label: format!("{} ({} params)", name, params.len()),
                    family: fam,
                    summary: format!("{} Group-0 shader uniforms", params.len()),
                    mat_type: None,
                }
            })
            .collect(),
        _ => b2_shader_map()
            .values()
            .map(|(orig_name, lay)| ShaderOptionDesc {
                key: orig_name.clone(),
                label: format!("{} — {}", orig_name, lay.family_meaning),
                family: lay.family.to_uppercase(),
                summary: format!("{} textures · {} vec4 params", lay.tex_count, lay.mat_vec4),
                mat_type: None,
            })
            .collect(),
    }
}

/// Decode a `WmbMaterial` (`WMB1` or `WMB2`) using either the `b2_shader_layouts.json` database
/// (when `is_bayo2` is true or `shader_name` matches a Bayonetta 2 shader) or `MATERIAL_DATABASE` (`materials.json`).
pub fn decode_wmb_material_rich(
    mat: &WmbMaterial,
    index: usize,
    is_bayo2_hint: bool,
) -> DecodedWmbMaterial {
    let has_b2_shader = mat
        .shader_name
        .as_deref()
        .and_then(lookup_b2_shader)
        .is_some();
    let is_b2 = is_bayo2_hint || has_b2_shader;

    if is_b2 {
        if let Some(sh_str) = mat.shader_name.as_deref() {
            if let Some((canon_name, lay)) = lookup_b2_shader(sh_str) {
                let mut fields = Vec::new();
                for t in &lay.textures {
                    let tid = mat.material_data.get(t.slot).copied().unwrap_or(0);
                    let eff_role = if t.role.trim().is_empty() {
                        t.meaning.as_str()
                    } else {
                        t.role.as_str()
                    };
                    let param_type = if eff_role.contains("cube") || t.meaning.contains("cube") {
                        MaterialParamType::SamplerCube
                    } else {
                        MaterialParamType::Sampler2D
                    };
                    let pre_lbl = lookup_pretexture_label("b2", tid);
                    let slug: String = eff_role
                        .chars()
                        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
                        .collect();
                    fields.push(DecodedMaterialParameter {
                        name: format!("slot_{}_{}", t.slot, slug.trim_matches('_')),
                        param_type,
                        sampler_tex_id: Some(tid),
                        float_values: None,
                        role: Some(eff_role.to_string()),
                        description: Some(t.meaning.clone()),
                        component_labels: None,
                        pretexture_name: pre_lbl,
                        is_color: false,
                    });
                }

                let base = lay.tex_count + 16 * lay.matrix_blocks;
                for p in &lay.params {
                    let word_start = base + 4 * p.index;
                    let mut flts = [0.0f32; 4];
                    for k in 0..4 {
                        if let Some(&bits) = mat.material_data.get(word_start + k) {
                            flts[k] = f32::from_bits(bits);
                        }
                    }
                    let comps: [String; 4] = std::array::from_fn(|k| {
                        p.components
                            .get(k)
                            .and_then(|c| c.clone())
                            .unwrap_or_else(|| ["x", "y", "z", "w"][k].to_string())
                    });
                    let name_low = p.name.to_ascii_lowercase();
                    let is_color = name_low.contains("color")
                        || name_low.contains("tint")
                        || (comps[0] == "r" && comps[1] == "g" && comps[2] == "b");
                    fields.push(DecodedMaterialParameter {
                        name: p.name.clone(),
                        param_type: MaterialParamType::F4Float4,
                        sampler_tex_id: None,
                        float_values: Some(flts),
                        role: None,
                        description: if p.note.is_empty() {
                            None
                        } else {
                            Some(p.note.clone())
                        },
                        component_labels: Some(comps),
                        pretexture_name: None,
                        is_color,
                    });
                }

                // Append any trailing raw vec4 words beyond the documented layout so 100% of data is editable
                let documented_words = base + 4 * lay.params.len();
                if mat.material_data.len() > documented_words {
                    let mut extra_cursor = documented_words;
                    let mut extra_idx = 0usize;
                    while extra_cursor < mat.material_data.len() {
                        let mut flts = [0.0f32; 4];
                        for k in 0..4 {
                            if let Some(&bits) = mat.material_data.get(extra_cursor + k) {
                                flts[k] = f32::from_bits(bits);
                            }
                        }
                        fields.push(DecodedMaterialParameter {
                            name: format!("extra_vec4_{extra_idx}"),
                            param_type: MaterialParamType::F4Float4,
                            sampler_tex_id: None,
                            float_values: Some(flts),
                            role: None,
                            description: Some(format!(
                                "Additional uniform words [{}..{}]",
                                extra_cursor,
                                (extra_cursor + 3).min(mat.material_data.len().saturating_sub(1))
                            )),
                            component_labels: Some(["x".into(), "y".into(), "z".into(), "w".into()]),
                            pretexture_name: None,
                            is_color: false,
                        });
                        extra_cursor += 4;
                        extra_idx += 1;
                    }
                }

                let expected_bytes = 20 + (lay.tex_count + 16 * lay.matrix_blocks + 4 * lay.mat_vec4) * 4;
                return DecodedWmbMaterial {
                    index,
                    mat_type: mat.mat_type,
                    flag: mat.flag,
                    is_bayo2: true,
                    shader_name: Some(canon_name.clone()),
                    shader_family: Some(format!(
                        "{}: {}",
                        lay.family.to_uppercase(),
                        lay.family_meaning
                    )),
                    shader_flags_notes: lay
                        .flags
                        .chars()
                        .zip(lay.letters.iter())
                        .filter(|(_, m)| !m.is_empty() && *m != "none" && *m != "?")
                        .map(|(c, m)| format!("{c}: {m}"))
                        .collect(),
                    expected_byte_size: Some(expected_bytes),
                    fields,
                    raw_dwords: mat.material_data.clone(),
                };
            }
        }
    }

    // Bayonetta 1 layout (`MATERIAL_DATABASE`) or fallback raw dwords
    let spec = lookup_material_type(mat.mat_type);
    let mut fields = Vec::new();
    let mut cursor = 0usize;
    if let Some(s) = spec {
        for f in s.layout {
            if f.param_type.is_sampler() {
                let tex_id = mat.material_data.get(cursor).copied().unwrap_or(0);
                let pre_lbl = lookup_pretexture_label("b1", tex_id);
                fields.push(DecodedMaterialParameter {
                    name: f.name.to_string(),
                    param_type: f.param_type,
                    sampler_tex_id: Some(tex_id),
                    float_values: None,
                    role: Some(b1_sampler_role(f.name).to_string()),
                    description: Some(format!("{:?}", f.param_type)),
                    component_labels: None,
                    pretexture_name: pre_lbl,
                    is_color: false,
                });
                cursor += 1;
            } else {
                let mut flts = [0.0f32; 4];
                for k in 0..4 {
                    if let Some(&bits) = mat.material_data.get(cursor + k) {
                        flts[k] = f32::from_bits(bits);
                    }
                }
                let n_low = f.name.to_ascii_lowercase();
                let is_color = n_low.contains("diffuse")
                    || n_low.contains("color")
                    || n_low == "fog"
                    || n_low.contains("ambient");
                let comps = if is_color {
                    Some(["r".into(), "g".into(), "b".into(), "a".into()])
                } else if n_low.contains("tile") {
                    Some(["tile_u".into(), "tile_v".into(), "z".into(), "w".into()])
                } else {
                    Some(["x".into(), "y".into(), "z".into(), "w".into()])
                };
                fields.push(DecodedMaterialParameter {
                    name: f.name.to_string(),
                    param_type: f.param_type,
                    sampler_tex_id: None,
                    float_values: Some(flts),
                    role: None,
                    description: Some(format!("{:?}", f.param_type)),
                    component_labels: comps,
                    pretexture_name: None,
                    is_color,
                });
                cursor += 4;
            }
        }
    }

    // If there are remaining dwords (or unknown B1/B2 shader), expose them as editable fields!
    if cursor < mat.material_data.len() {
        let mut extra_idx = 0usize;
        while cursor < mat.material_data.len() {
            if is_b2 && cursor < 5 {
                let tid = mat.material_data[cursor];
                fields.push(DecodedMaterialParameter {
                    name: format!("tex_slot_{cursor}"),
                    param_type: MaterialParamType::Sampler2D,
                    sampler_tex_id: Some(tid),
                    float_values: None,
                    role: Some("texture".into()),
                    description: Some(format!("Word [{cursor}]")),
                    component_labels: None,
                    pretexture_name: lookup_pretexture_label("b2", tid),
                    is_color: false,
                });
                cursor += 1;
            } else {
                let mut flts = [0.0f32; 4];
                for k in 0..4 {
                    if let Some(&bits) = mat.material_data.get(cursor + k) {
                        flts[k] = f32::from_bits(bits);
                    }
                }
                fields.push(DecodedMaterialParameter {
                    name: format!("param_vec4_{extra_idx}"),
                    param_type: MaterialParamType::F4Float4,
                    sampler_tex_id: None,
                    float_values: Some(flts),
                    role: None,
                    description: Some(format!("Words [{cursor}..{}]", cursor + 3)),
                    component_labels: Some(["x".into(), "y".into(), "z".into(), "w".into()]),
                    pretexture_name: None,
                    is_color: false,
                });
                cursor += 4;
                extra_idx += 1;
            }
        }
    }

    DecodedWmbMaterial {
        index,
        mat_type: mat.mat_type,
        flag: mat.flag,
        is_bayo2: is_b2,
        shader_name: mat
            .shader_name
            .clone()
            .or_else(|| spec.and_then(|s| s.shader.map(|sh| sh.to_string()))),
        shader_family: spec.and_then(|s| s.shader.map(|sh| format!("Bayonetta 1 ({sh})"))),
        shader_flags_notes: Vec::new(),
        expected_byte_size: spec.map(|s| s.byte_size),
        fields,
        raw_dwords: mat.material_data.clone(),
    }
}

fn b1_sampler_role(name: &str) -> &'static str {
    match name {
        "Color_1" => "albedo",
        "Color_2" => "albedo2",
        "Color_3" => "albedo3",
        "lightmap" => "lightmap",
        "reliefmap" => "normal",
        "envmap" => "cube",
        "env_amb" => "env_amb",
        "Spec_Mask" => "spec_mask",
        "Spec_Pow" => "spec_lut",
        "refractmap" => "refract",
        _ => "extra",
    }
}

/// Switch a `WmbMaterial` to a new shader/type while preserving matching texture roles and uniform parameters!
pub fn switch_wmb_material_shader(
    mat: &mut WmbMaterial,
    index: usize,
    is_bayo2: bool,
    target_shader_or_type: &str,
) -> DecodedWmbMaterial {
    let old_dec = decode_wmb_material_rich(mat, index, is_bayo2);
    let mut role_to_tex: BTreeMap<String, u32> = BTreeMap::new();
    let mut name_to_vec4: BTreeMap<String, [f32; 4]> = BTreeMap::new();
    for f in &old_dec.fields {
        if let Some(tid) = f.sampler_tex_id {
            if let Some(ref r) = f.role {
                role_to_tex.entry(r.to_ascii_lowercase()).or_insert(tid);
            }
            role_to_tex.insert(f.name.to_ascii_lowercase(), tid);
        } else if let Some(flts) = f.float_values {
            name_to_vec4.insert(f.name.to_ascii_lowercase(), flts);
        }
    }

    if is_bayo2 || lookup_b2_shader(target_shader_or_type).is_some() {
        if let Some((canon_name, lay)) = lookup_b2_shader(target_shader_or_type) {
            mat.shader_name = Some(canon_name.clone());
            let total_words = lay.tex_count + 16 * lay.matrix_blocks + 4 * lay.mat_vec4;
            let mut new_words = vec![0u32; total_words];
            for t in &lay.textures {
                let r_low = if t.role.trim().is_empty() {
                    t.meaning.to_ascii_lowercase()
                } else {
                    t.role.to_ascii_lowercase()
                };
                let tid = role_to_tex
                    .get(&r_low)
                    .copied()
                    .or_else(|| {
                        if r_low.contains("albedo") {
                            role_to_tex.get("albedo").copied()
                        } else if r_low.contains("normal") {
                            role_to_tex.get("normal").copied().or(Some(0x7FD4929A))
                        } else if r_low.contains("lut") {
                            Some(0x0DA7BFF1)
                        } else if r_low.contains("lightmap") {
                            Some(0x4E9C16F4)
                        } else if r_low.contains("cube") {
                            Some(0x1FBC0984)
                        } else {
                            Some(0x5864A24F)
                        }
                    })
                    .unwrap_or(0x5864A24F);
                if t.slot < new_words.len() {
                    new_words[t.slot] = tid;
                }
            }
            let base = lay.tex_count + 16 * lay.matrix_blocks;
            for p in &lay.params {
                let w_off = base + 4 * p.index;
                let flts = name_to_vec4
                    .get(&p.name.to_ascii_lowercase())
                    .copied()
                    .unwrap_or_else(|| match p.name.as_str() {
                        "base_color" | "color" => [0.5, 0.5, 0.5, 1.0],
                        "specular" => [2.0, 0.96875, 20.0, 1.0],
                        "uv_tile" => [1.0, 1.0, -1.0, -1.0],
                        _ => [0.0, -1.0, -1.0, -1.0],
                    });
                for k in 0..4 {
                    if w_off + k < new_words.len() {
                        new_words[w_off + k] = flts[k].to_bits();
                    }
                }
            }
            mat.material_data = new_words;
            return decode_wmb_material_rich(mat, index, true);
        }
    }

    // Bayonetta 1 type switch (supports both decimal "48" and hex "0x30")
    let trimmed_target = target_shader_or_type.trim();
    let parsed_b1_type = if let Some(hex) = trimmed_target
        .strip_prefix("0x")
        .or_else(|| trimmed_target.strip_prefix("0X"))
    {
        i16::from_str_radix(hex, 16)
    } else {
        trimmed_target.parse::<i16>()
    };
    if let Ok(new_type) = parsed_b1_type {
        if let Some(spec) = lookup_material_type(new_type) {
            mat.mat_type = new_type;
            mat.shader_name = spec.shader.map(|s| s.to_string());
            let mut new_words = Vec::new();
            for f in spec.layout {
                if f.param_type.is_sampler() {
                    let role = b1_sampler_role(f.name);
                    let tid = role_to_tex
                        .get(&f.name.to_ascii_lowercase())
                        .or_else(|| role_to_tex.get(role))
                        .copied()
                        .unwrap_or(if role == "normal" { 0x80000005 } else { 0x80000004 });
                    new_words.push(tid);
                } else {
                    let flts = name_to_vec4
                        .get(&f.name.to_ascii_lowercase())
                        .copied()
                        .unwrap_or_else(|| match f.name {
                            "diffuse" => [1.0, 1.0, 1.0, 1.0],
                            "Color_1_Tile" | "tile" => [1.0, 1.0, -1.0, -1.0],
                            "bias" | "Env_Amb_Param" => [1.0, -1.0, -1.0, -1.0],
                            _ => [0.0, -1.0, -1.0, -1.0],
                        });
                    for k in 0..4 {
                        new_words.push(flts[k].to_bits());
                    }
                }
            }
            let target_dwords = (spec.byte_size.saturating_sub(4)) / 4;
            new_words.resize(target_dwords, 0);
            mat.material_data = new_words;
        }
    }
    decode_wmb_material_rich(mat, index, false)
}

/// Switch a `Wmb3Material` shader while preserving matching texture slots and named Group 0 parameters!
pub fn switch_wmb3_material_shader(mat: &mut Wmb3Material, new_shader: &str) {
    let old_names = lookup_b3_shader_params(&mat.shader_name)
        .map(|s| s.to_vec())
        .unwrap_or_default();
    let mut old_param_by_name: BTreeMap<String, f32> = BTreeMap::new();
    if let Some((_, g0)) = mat.parameter_groups.first() {
        for (i, &val) in g0.iter().enumerate() {
            if let Some(nm) = old_names.get(i) {
                if nm != "Unknown" {
                    old_param_by_name.insert(nm.clone(), val);
                }
            }
        }
    }
    mat.shader_name = new_shader.trim().to_string();
    if let Some(new_names) = lookup_b3_shader_params(&mat.shader_name) {
        let mut new_g0 = Vec::with_capacity(new_names.len());
        for (i, nm) in new_names.iter().enumerate() {
            if nm != "Unknown" {
                if let Some(&v) = old_param_by_name.get(nm) {
                    new_g0.push(v);
                    continue;
                }
            }
            let fallback = mat
                .parameter_groups
                .first()
                .and_then(|(_, g0)| g0.get(i).copied())
                .unwrap_or_else(|| match nm.as_str() {
                    "g_Tile_X" | "g_Tile_Y" | "g_AlbedoColor_X" | "g_AlbedoColor_Y"
                    | "g_AlbedoColor_Z" | "g_PriSpecularColor_X" | "g_PriSpecularColor_Y"
                    | "g_PriSpecularColor_Z" | "g_SecSpecularColor_X" | "g_SecSpecularColor_Y"
                    | "g_SecSpecularColor_Z" | "g_GlassColor_X" | "g_GlassColor_Y"
                    | "g_GlassColor_Z" | "g_FresnelColor_X" | "g_FresnelColor_Y"
                    | "g_FresnelColor_Z" | "g_UseNormalMap" | "g_UseOcclusionMap"
                    | "g_SpecRefRate" | "g_DiffuseIntensity" | "g_HairNormalIntensity"
                    | "g_AmbientOcclusionMax" | "g_SpecularMaskMax" | "g_OcclusionMaskMax"
                    | "g_RoughnessMax" | "g_MetalMax" | "g_FresnelMax" | "g_inLightIntensity" => 1.0,
                    "g_LayerColor1_X" | "g_LayerColor1_Y" | "g_LayerColor1_Z"
                    | "g_LayerColor2_X" | "g_LayerColor2_Y" | "g_LayerColor2_Z"
                    | "g_BottomAlbedoColor_X" | "g_BottomAlbedoColor_Y" | "g_BottomAlbedoColor_Z"
                    | "g_PriAlpha" | "g_SecAlpha" | "g_SpecularShadowClampMax"
                    | "g_BlendThresholdMax" => 0.8,
                    "g_HairColor_X" | "g_HairColor_Y" | "g_HairColor_Z" => 0.25,
                    "g_Roughness" | "g_PriRoughness" | "g_SecRoughness" | "g_NormalIntensity"
                    | "g_WrapDiffuseStrength" | "g_TransStrength" | "g_SSSBias"
                    | "g_CurvatureScale" | "g_InteriorBlendRate" | "g_RefractionBias"
                    | "g_ClearCoatRate" | "g_RockSpecIntensity" | "g_SandSpecIntensity"
                    | "g_SandRoughness" => 0.5,
                    "g_AmbientOcclusionMin" | "g_SpecularMaskMin" | "g_OcclusionMaskMin"
                    | "g_SpecularShadowClampMin" | "g_BlendThresholdMin" => 0.15,
                    "g_SpaceWidth" | "g_SpaceHeight" | "g_SpaceDepth" => 1.5,
                    "g_SpecularPower" => 1.25,
                    _ => 0.0,
                });
            new_g0.push(fallback);
        }
        if mat.parameter_groups.is_empty() {
            mat.parameter_groups.push((0, new_g0));
        } else {
            mat.parameter_groups[0].1 = new_g0;
        }
    }
}

/// Convert a material between Bayonetta 1 (`b1`), Bayonetta 2 (`b2`), and Bayonetta 3 (`b3`),
/// returning a human-readable conversion report alongside the converted material JSON.
pub fn convert_material_between_games(
    src_game: &str,
    dst_game: &str,
    src_wmb_mat: Option<&DecodedWmbMaterial>,
    src_wmb3_mat: Option<&Wmb3Material>,
    target_shader_override: Option<&str>,
) -> Result<(serde_json::Value, Vec<String>), String> {
    let mut report = Vec::new();
    let mut albedo_tex: Option<u32> = None;
    let mut normal_tex: Option<u32> = None;
    let mut lightmap_tex: Option<u32> = None;
    let mut color_rgb = [1.0f32, 1.0, 1.0];
    let mut tile_uv = [1.0f32, 1.0];
    let mut spec_intensity = 1.0f32;
    let mut roughness = 0.45f32;

    if let Some(m) = src_wmb_mat {
        for f in &m.fields {
            if let Some(tid) = f.sampler_tex_id {
                let r = f.role.as_deref().unwrap_or(&f.name).to_ascii_lowercase();
                if r.contains("albedo") || f.name == "Color_1" {
                    albedo_tex.get_or_insert(tid);
                } else if r.contains("normal") || f.name == "reliefmap" {
                    normal_tex.get_or_insert(tid);
                } else if r.contains("lightmap") {
                    lightmap_tex.get_or_insert(tid);
                }
            } else if let Some(flts) = f.float_values {
                let n = f.name.to_ascii_lowercase();
                if n == "diffuse" || n == "base_color" || n == "color" {
                    let scale = if src_game == "b2" { 2.0 } else { 1.0 };
                    color_rgb = [flts[0] * scale, flts[1] * scale, flts[2] * scale];
                } else if n == "color_1_tile" || n == "uv_tile" {
                    tile_uv = [flts[0], flts[1]];
                } else if n == "specular" {
                    spec_intensity = flts[0];
                    if src_game == "b2" && flts[2] > 0.0 {
                        roughness = (2.0 / (flts[2] + 2.0)).sqrt().clamp(0.05, 1.0);
                    }
                }
            }
        }
    } else if let Some(m3) = src_wmb3_mat {
        for (slot, tid) in &m3.textures {
            if slot.contains("Albedo") {
                albedo_tex.get_or_insert(*tid);
            } else if slot.contains("Normal") {
                normal_tex.get_or_insert(*tid);
            } else if slot.contains("LightMap") {
                lightmap_tex.get_or_insert(*tid);
            }
        }
        if let Some(names) = lookup_b3_shader_params(&m3.shader_name) {
            if let Some((_, g0)) = m3.parameter_groups.first() {
                for (i, nm) in names.iter().enumerate() {
                    if let Some(&v) = g0.get(i) {
                        match nm.as_str() {
                            "g_AlbedoColor_X" => color_rgb[0] = v,
                            "g_AlbedoColor_Y" => color_rgb[1] = v,
                            "g_AlbedoColor_Z" => color_rgb[2] = v,
                            "g_Tile_X" => tile_uv[0] = v,
                            "g_Tile_Y" => tile_uv[1] = v,
                            "g_Roughness" | "g_PriRoughness" => roughness = v.clamp(0.05, 1.0),
                            _ => {}
                        }
                    }
                }
            }
        }
    }

    match dst_game.to_ascii_lowercase().as_str() {
        "b1" => {
            let target_type: i16 = target_shader_override
                .and_then(|s| s.parse().ok())
                .unwrap_or(if normal_tex.is_some() { 127 } else { 68 });
            let xc = xconv_data();
            let b1_type_tpl = xc
                .get("b1")
                .and_then(|b| b.get("types"))
                .and_then(|t| t.get(target_type.to_string()));
            let b1_names_tpl = xc.get("b1").and_then(|b| b.get("names"));
            let tpl_flags = b1_type_tpl
                .and_then(|t| t.get("flags"))
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as u16;
            let mut dummy = WmbMaterial {
                mat_type: target_type,
                flag: tpl_flags,
                shader_name: None,
                material_data: Vec::new(),
            };
            let mut dec = switch_wmb_material_shader(
                &mut dummy,
                src_wmb_mat.map(|m| m.index).unwrap_or(0),
                false,
                &target_type.to_string(),
            );
            for f in &mut dec.fields {
                let tpl_val = b1_type_tpl
                    .and_then(|t| t.get("values"))
                    .and_then(|v| v.get(&f.name))
                    .or_else(|| b1_names_tpl.and_then(|n| n.get(&f.name)));
                if f.sampler_tex_id.is_some() {
                    if let Some(tid) = tpl_val.and_then(|v| v.as_u64()) {
                        f.sampler_tex_id = Some(tid as u32);
                    }
                    if f.name == "Color_1" {
                        if let Some(t) = albedo_tex {
                            f.sampler_tex_id = Some(if t >= 0x8000_0000 || t < 256 { t } else { 0 });
                        }
                    } else if f.name == "reliefmap" {
                        if let Some(t) = normal_tex {
                            f.sampler_tex_id = Some(if t < 256 { t } else { 0x80000005 });
                        }
                    }
                } else {
                    if let Some(arr) = tpl_val.and_then(|v| v.as_array()) {
                        if arr.len() >= 4 {
                            f.float_values = Some([
                                arr[0].as_f64().unwrap_or(0.0) as f32,
                                arr[1].as_f64().unwrap_or(0.0) as f32,
                                arr[2].as_f64().unwrap_or(0.0) as f32,
                                arr[3].as_f64().unwrap_or(0.0) as f32,
                            ]);
                        }
                    }
                    if f.name == "diffuse" {
                        f.float_values = Some([color_rgb[0], color_rgb[1], color_rgb[2], 1.0]);
                    } else if f.name == "Color_1_Tile" {
                        f.float_values = Some([tile_uv[0], tile_uv[1], -1.0, -1.0]);
                    } else if f.name == "specular" {
                        let s = (spec_intensity * 0.5).max(0.2);
                        f.float_values = Some([s, s, s, 1.0]);
                    }
                }
            }
            dummy.apply_decoded(&dec);
            dec.raw_dwords = dummy.material_data.clone();
            report.push(format!(
                "Converted {} material -> Bayonetta 1 Type {} ({})",
                src_game.to_uppercase(),
                target_type,
                dec.shader_name.as_deref().unwrap_or("B1")
            ));
            Ok((serde_json::to_value(dec).map_err(|e| e.to_string())?, report))
        }
        "b2" => {
            let target_sh = target_shader_override.unwrap_or("ois00_xbceX");
            let xc = xconv_data();
            let donor = xc
                .get("b2")
                .and_then(|b| b.get("donors"))
                .and_then(|d| d.get(target_sh.to_ascii_lowercase()));
            let donor_words: Vec<u32> = donor
                .and_then(|d| d.get("words"))
                .and_then(|w| w.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|x| x.as_u64().map(|v| v as u32))
                        .collect()
                })
                .unwrap_or_default();
            let donor_flag = donor
                .and_then(|d| d.get("flag"))
                .and_then(|f| f.as_u64())
                .unwrap_or(0) as u16;
            let mut dummy = WmbMaterial {
                mat_type: 0,
                flag: donor_flag,
                shader_name: Some(target_sh.to_string()),
                material_data: donor_words,
            };
            let mut dec = switch_wmb_material_shader(
                &mut dummy,
                src_wmb_mat.map(|m| m.index).unwrap_or(0),
                true,
                target_sh,
            );
            let is_b1_pretex = |t: u32| (0x8000_0000..=0x8000_00FF).contains(&t);
            for f in &mut dec.fields {
                if f.sampler_tex_id.is_some() {
                    let r = f.role.as_deref().unwrap_or("").to_ascii_lowercase();
                    if r.contains("albedo") {
                        if let Some(t) = albedo_tex {
                            f.sampler_tex_id = Some(if t > 256 && !is_b1_pretex(t) {
                                t
                            } else {
                                0x5864A24F
                            });
                        }
                    } else if r.contains("normal") {
                        if let Some(t) = normal_tex {
                            f.sampler_tex_id = Some(if t > 256 && !is_b1_pretex(t) {
                                t
                            } else {
                                0x7FD4929A
                            });
                        }
                    } else if r.contains("lightmap") {
                        if let Some(t) = lightmap_tex {
                            f.sampler_tex_id = Some(t);
                        }
                    }
                } else if f.name == "base_color" || f.name == "color" {
                    f.float_values = Some([
                        color_rgb[0] * 0.5,
                        color_rgb[1] * 0.5,
                        color_rgb[2] * 0.5,
                        1.0,
                    ]);
                } else if f.name == "uv_tile" {
                    f.float_values = Some([tile_uv[0], tile_uv[1], -1.0, -1.0]);
                } else if f.name == "specular" {
                    let power = (2.0 / (roughness * roughness) - 2.0).clamp(1.0, 200.0);
                    f.float_values = Some([spec_intensity.max(1.0), 0.96875, power, 1.0]);
                }
            }
            dummy.apply_decoded(&dec);
            dec.raw_dwords = dummy.material_data.clone();
            report.push(format!(
                "Converted {} material -> Bayonetta 2 Shader '{}'",
                src_game.to_uppercase(),
                dec.shader_name.as_deref().unwrap_or(target_sh)
            ));
            Ok((serde_json::to_value(dec).map_err(|e| e.to_string())?, report))
        }
        "b3" => {
            let target_sh = target_shader_override.unwrap_or("PBS00_XXXXX");
            let xc = xconv_data();
            let b3_donor = xc
                .get("b3")
                .and_then(|b| b.get("donors"))
                .and_then(|d| {
                    d.get(target_sh).or_else(|| {
                        d.as_object().and_then(|obj| {
                            obj.iter()
                                .find(|(k, _)| k.eq_ignore_ascii_case(target_sh))
                                .map(|(_, v)| v)
                        })
                    })
                });
            let donor_tech = b3_donor
                .and_then(|d| d.get("technique"))
                .and_then(|t| t.as_str())
                .unwrap_or("Default")
                .to_string();
            let mut donor_textures: Vec<(String, u32)> = b3_donor
                .and_then(|d| d.get("textures"))
                .and_then(|t| t.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|pair| {
                            let p = pair.as_array()?;
                            Some((
                                p.first()?.as_str()?.to_string(),
                                p.get(1)?.as_u64()? as u32,
                            ))
                        })
                        .collect()
                })
                .unwrap_or_else(|| {
                    vec![
                        ("g_AlbedoMap".into(), 0x5864A24F),
                        ("g_NormalMap".into(), 0x7FD4929A),
                        ("g_MaskMap".into(), 0x6A9BFB55),
                        ("g_LightMap".into(), 0x4E9C16F4),
                        ("g_EnvMap".into(), 0x1FBC0984),
                    ]
                });
            for (slot_name, tid) in &mut donor_textures {
                if slot_name.contains("Albedo") {
                    if let Some(t) = albedo_tex {
                        *tid = t;
                    }
                } else if slot_name.contains("Normal") && !slot_name.ends_with('2') {
                    if let Some(t) = normal_tex {
                        *tid = t;
                    }
                } else if slot_name.contains("LightMap") {
                    if let Some(t) = lightmap_tex {
                        *tid = t;
                    }
                }
            }
            let donor_groups: Vec<(i32, Vec<f32>)> = b3_donor
                .and_then(|d| d.get("groups"))
                .and_then(|g| g.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|grp| {
                            let ga = grp.as_array()?;
                            let flag = ga.first()?.as_i64()? as i32;
                            let flts: Vec<f32> = ga
                                .get(1)?
                                .as_array()?
                                .iter()
                                .filter_map(|x| x.as_f64().map(|v| v as f32))
                                .collect();
                            Some((flag, flts))
                        })
                        .collect()
                })
                .unwrap_or_default();
            let donor_vars: Vec<(String, f32)> = b3_donor
                .and_then(|d| d.get("variables"))
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|pair| {
                            let pa = pair.as_array()?;
                            Some((
                                pa.first()?.as_str()?.to_string(),
                                pa.get(1)?.as_f64()? as f32,
                            ))
                        })
                        .collect()
                })
                .unwrap_or_else(|| vec![("Binormal0".into(), 0.0)]);
            let mut m3 = Wmb3Material {
                date: [2020, 8, 24, 1],
                name: src_wmb3_mat
                    .map(|m| m.name.clone())
                    .unwrap_or_else(|| "ConvertedMaterial".to_string()),
                shader_name: target_sh.to_string(),
                technique_name: donor_tech,
                u_a: 1,
                textures: donor_textures,
                parameter_groups: donor_groups,
                variables: donor_vars,
            };
            switch_wmb3_material_shader(&mut m3, target_sh);
            if let Some(names) = lookup_b3_shader_params(&m3.shader_name) {
                if let Some((_, g0)) = m3.parameter_groups.first_mut() {
                    for (i, nm) in names.iter().enumerate() {
                        if let Some(slot) = g0.get_mut(i) {
                            match nm.as_str() {
                                "g_AlbedoColor_X" => *slot = color_rgb[0],
                                "g_AlbedoColor_Y" => *slot = color_rgb[1],
                                "g_AlbedoColor_Z" => *slot = color_rgb[2],
                                "g_Tile_X" => *slot = tile_uv[0],
                                "g_Tile_Y" => *slot = tile_uv[1],
                                "g_Roughness" | "g_PriRoughness" => *slot = roughness,
                                "g_bRoughnessOverWrite" | "g_bMetalicOverWrite" => *slot = 1.0,
                                _ => {}
                            }
                        }
                    }
                }
            }
            report.push(format!(
                "Converted {} material -> Bayonetta 3 Shader '{}'",
                src_game.to_uppercase(),
                target_sh
            ));
            Ok((serde_json::to_value(m3).map_err(|e| e.to_string())?, report))
        }
        other => Err(format!("Unsupported target game: {other}")),
    }
}
