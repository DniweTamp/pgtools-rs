import { MaterialDesc, TextureSlotDesc } from "../../contract";
import { RenderQueueBucket, STUDIO_VERTEX_SHADER } from "../common";
import { B3_PBR_COMMON_GLSL } from "./pbrCommon";

interface B3SamplerSlotBinding {
  albedo?: TextureSlotDesc;
  normal?: TextureSlotDesc;
  mask?: TextureSlotDesc;
  normal2?: TextureSlotDesc;
  normal3?: TextureSlotDesc;
  indexMap?: TextureSlotDesc;
  envCube?: TextureSlotDesc;
  irradCube?: TextureSlotDesc;
}

const KNOWN_B3_SLOT_NAMES = new Set([
  "g_albedomap",
  "g_albedomap1",
  "g_albedomap2",
  "g_albedomap3",
  "g_normalmap",
  "g_normalmap1",
  "g_normalmap2",
  "g_normalmap3",
  "g_detailnormalmap",
  "g_maskmap",
  "g_maskmap2",
  "g_lightmap",
  "g_irradiancemap",
  "g_envmap",
  "g_envmapsp",
  "g_blendmap",
  "g_patternmap",
  "g_pulsemap",
  "g_flowmap",
  "g_parallaxmap",
  "g_curvaturemap",
  "g_subsurfacelut",
  "g_lut",
  "g_dither",
  "g_ceiltexture",
  "g_floortexture",
  "g_wallxytexture",
  "g_wallzytexture",
]);

function isGenericSlotName(name: string): boolean {
  const low = name.trim().toLowerCase();
  if (!low) return true;
  if (/^(g_tex\d+|slot_\d+|tex_?\d+|texture_?\d+)$/.test(low)) return true;
  return !KNOWN_B3_SLOT_NAMES.has(low);
}

/**
 * Looks up a slot by hardware slot number (`hwSlot`), falling back to dense array position
 * (`denseIdx`) when WMB3 materials store unnamed textures densely in `0..num_tex` (`Wmb3Material::texture_views`).
 */
function getUnnamedSlot(
  bySlot: Map<number, TextureSlotDesc>,
  slots: TextureSlotDesc[],
  hwSlot: number,
  denseIdx?: number
): TextureSlotDesc | undefined {
  const direct = bySlot.get(hwSlot);
  if (direct && isGenericSlotName(direct.name)) {
    return direct;
  }
  if (denseIdx !== undefined && denseIdx < slots.length) {
    const dense = slots[denseIdx];
    if (dense && isGenericSlotName(dense.name)) {
      return dense;
    }
  }
  return undefined;
}

/**
 * Resolves the texture slots for a Bayonetta 3 material across all documented shader families
 * (`PBS00`..`PBS99`, `BAYO00`, `Hair20/21/22`, `SKN20/21/25/30/31`, `HMSKN*`, `Gls*`, `HZR00`, `JWL*`, `CNS*`).
 */
export function resolveB3SamplerSlots(mat: MaterialDesc): B3SamplerSlotBinding {
  const slots = mat.textureSlots;
  const bySlot = new Map<number, TextureSlotDesc>();
  const byNameLow = new Map<string, TextureSlotDesc>();
  for (const s of slots) {
    bySlot.set(s.slot, s);
    byNameLow.set(s.name.toLowerCase(), s);
  }

  const shLow = (mat.shaderName ?? "").trim().toLowerCase();
  const isBayo00 = shLow.startsWith("bayo00");
  const isHair22 = shLow.startsWith("hair22");
  const isHair21 = shLow.startsWith("hair21");
  const isHair = shLow.startsWith("hair20") || isHair21 || isHair22;
  const isFbr = shLow.startsWith("fbr");
  const isSkn30Or31 = shLow.startsWith("skn30") || shLow.startsWith("skn31");
  const isSkin = shLow.startsWith("skn") || shLow.startsWith("hmskn");
  const isPbs02 = shLow.startsWith("pbs02");
  const isPbsNoMask =
    shLow.startsWith("pbs01") ||
    shLow.startsWith("pbs03") ||
    shLow.startsWith("pbs04") ||
    shLow.startsWith("pbs21") ||
    shLow.startsWith("pbs91") ||
    shLow.startsWith("pbs99");
  const isGls14 = shLow.startsWith("gls14");
  const isJwl = shLow.startsWith("jwl");

  let albedo: TextureSlotDesc | undefined;
  let normal: TextureSlotDesc | undefined;
  let mask: TextureSlotDesc | undefined;
  let normal2: TextureSlotDesc | undefined;
  let normal3: TextureSlotDesc | undefined;
  let indexMap: TextureSlotDesc | undefined;

  if (isHair22) {
    // Hair22: slot 1 = g_NormalMap, slot 2 = g_AlbedoMap1 (iridescence ramp), slot 3 = g_NormalMap2
    normal =
      byNameLow.get("g_normalmap") ??
      getUnnamedSlot(bySlot, slots, 1, 0) ??
      slots.find((s) => s.role === "normal");
    albedo =
      byNameLow.get("g_albedomap1") ??
      byNameLow.get("g_albedomap") ??
      getUnnamedSlot(bySlot, slots, 2, 1) ??
      getUnnamedSlot(bySlot, slots, 0, 0);
    normal2 =
      byNameLow.get("g_normalmap2") ?? getUnnamedSlot(bySlot, slots, 3, 2);
  } else if (isHair21) {
    // Hair21: no albedo texture; slot 1 = g_NormalMap, slot 3 = g_NormalMap2
    normal =
      byNameLow.get("g_normalmap") ??
      getUnnamedSlot(bySlot, slots, 1, 0) ??
      slots.find((s) => s.role === "normal");
    normal2 =
      byNameLow.get("g_normalmap2") ?? getUnnamedSlot(bySlot, slots, 3, 1);
  } else if (isBayo00) {
    // BAYO00: 0 = g_AlbedoMap1, 1 = g_NormalMap, 3 = g_NormalMap2, 4 = g_NormalMap3, 7 = g_MaskMap
    albedo =
      byNameLow.get("g_albedomap1") ??
      byNameLow.get("g_albedomap") ??
      getUnnamedSlot(bySlot, slots, 0, 0);
    normal =
      byNameLow.get("g_normalmap") ??
      getUnnamedSlot(bySlot, slots, 1, 1) ??
      slots.find((s) => s.role === "normal");
    normal2 =
      byNameLow.get("g_normalmap2") ?? getUnnamedSlot(bySlot, slots, 3, 2);
    normal3 =
      byNameLow.get("g_normalmap3") ?? getUnnamedSlot(bySlot, slots, 4, 3);
    indexMap =
      byNameLow.get("g_maskmap") ??
      getUnnamedSlot(bySlot, slots, 7, 4) ??
      slots.find((s) => s.role === "mask" || s.role === "spec");
    mask = indexMap;
  } else if (isHair) {
    // Hair20 WMB3 slot order: 0 = g_AlbedoMap1, 1 = g_NormalMap2 (pink strand map), 2 = g_NormalMap (base normal + B=AO/A=mask)
    albedo =
      byNameLow.get("g_albedomap1") ??
      byNameLow.get("g_albedomap") ??
      getUnnamedSlot(bySlot, slots, 0, 0);
    normal =
      byNameLow.get("g_normalmap") ??
      getUnnamedSlot(bySlot, slots, 2, 2) ??
      slots.find((s) => s.name.toLowerCase() === "g_normalmap");
    normal2 =
      byNameLow.get("g_normalmap2") ?? getUnnamedSlot(bySlot, slots, 1, 1);
    mask = byNameLow.get("g_maskmap");
  } else if (isFbr) {
    // FBR00 (§07b §1.1): slot 0 = g_AlbedoMap1, slot 6 = g_NormalMap3 (un-tiled base normal + .a layer2 blend),
    // slot 1 = g_NormalMap1 (tiled micro-weave detail normal + .a specMask), slot 2 = g_AlbedoMap2, slot 3 = g_NormalMap2
    albedo =
      byNameLow.get("g_albedomap1") ??
      byNameLow.get("g_albedomap") ??
      getUnnamedSlot(bySlot, slots, 0, 0);
    normal =
      byNameLow.get("g_normalmap3") ??
      byNameLow.get("g_normalmap") ??
      getUnnamedSlot(bySlot, slots, 6, 4) ??
      byNameLow.get("g_normalmap1") ??
      slots.find((s) => s.role === "normal");
    normal2 =
      byNameLow.get("g_normalmap1") ??
      byNameLow.get("g_detailnormalmap") ??
      getUnnamedSlot(bySlot, slots, 1, 1);
    mask =
      byNameLow.get("g_albedomap2") ??
      byNameLow.get("g_maskmap") ??
      getUnnamedSlot(bySlot, slots, 2, 2);
    normal3 =
      byNameLow.get("g_normalmap2") ?? getUnnamedSlot(bySlot, slots, 3, 3);
  } else if (isSkn30Or31) {
    // SKN30 / SKN31: 0 = g_AlbedoMap, 1 = g_AlbedoMap2 (rgba: .a = parallax), 2 = tex_meltNormal (SKN31), 3 = g_NormalMap, 4 = g_NormalMap2
    albedo =
      byNameLow.get("g_albedomap") ??
      byNameLow.get("g_albedomap1") ??
      getUnnamedSlot(bySlot, slots, 0, 0);
    mask =
      byNameLow.get("g_albedomap2") ??
      byNameLow.get("g_maskmap") ??
      getUnnamedSlot(bySlot, slots, 1, 1);
    normal =
      byNameLow.get("g_normalmap") ??
      getUnnamedSlot(bySlot, slots, 3, 2) ??
      slots.find((s) => s.role === "normal");
    normal2 =
      byNameLow.get("g_normalmap2") ??
      byNameLow.get("g_detailnormalmap") ??
      getUnnamedSlot(bySlot, slots, 4, 3);
  } else if (isSkin) {
    // SKN20 / SKN21 / SKN25 / HMSKN*: 0 = g_AlbedoMap, 1 = g_NormalMap, 2 = g_AlbedoMap2, 3 = g_DetailNormalMap, 15 = AO map (SKN25)
    albedo =
      byNameLow.get("g_albedomap") ??
      byNameLow.get("g_albedomap1") ??
      getUnnamedSlot(bySlot, slots, 0, 0);
    normal =
      byNameLow.get("g_normalmap") ??
      getUnnamedSlot(bySlot, slots, 1, 1) ??
      getUnnamedSlot(bySlot, slots, 3, 2) ??
      slots.find((s) => s.role === "normal");
    mask =
      byNameLow.get("g_albedomap2") ??
      byNameLow.get("g_maskmap") ??
      getUnnamedSlot(bySlot, slots, 2, 2);
    normal2 =
      byNameLow.get("g_detailnormalmap") ??
      byNameLow.get("g_normalmap2") ??
      getUnnamedSlot(bySlot, slots, 3, 3);
    indexMap =
      byNameLow.get("g_occlusionmap") ??
      byNameLow.get("g_aomap") ??
      getUnnamedSlot(bySlot, slots, 15);
  } else if (isPbs02) {
    // PBS02: 0 = g_AlbedoMap, 2 = g_NormalMap2 (.a = primary roughness), 3 = g_NormalMap (.a = second-layer roughness)
    albedo =
      byNameLow.get("g_albedomap") ?? getUnnamedSlot(bySlot, slots, 0, 0);
    normal2 =
      byNameLow.get("g_normalmap2") ?? getUnnamedSlot(bySlot, slots, 2, 1);
    normal =
      byNameLow.get("g_normalmap") ??
      getUnnamedSlot(bySlot, slots, 3, 2) ??
      slots.find((s) => s !== normal2 && s.role === "normal");
    mask = normal2;
  } else if (isPbsNoMask) {
    // PBS01 / PBS03 / PBS04 / PBS21 / PBS91 / PBS99: no mask map (slot 0 = albedo, slot 3 = normal, slot 4 = detail normal in PBS04)
    albedo =
      byNameLow.get("g_albedomap") ??
      byNameLow.get("g_albedomap1") ??
      getUnnamedSlot(bySlot, slots, 0, 0) ??
      slots.find((s) => s.role === "albedo");
    normal =
      byNameLow.get("g_normalmap") ??
      byNameLow.get("g_normalmap1") ??
      getUnnamedSlot(bySlot, slots, 3, 1) ??
      slots.find((s) => s !== albedo && s.role === "normal");
    normal2 =
      byNameLow.get("g_detailnormalmap") ??
      byNameLow.get("g_normalmap2") ??
      getUnnamedSlot(bySlot, slots, 4, 2);
  } else if (isGls14) {
    // Gls14: slot 0 = g_BlendMap (8x8 crack jitter, NOT room albedo!), slot 1 = g_AlbedoMap2, slot 2 = g_MaskMap, slot 3 = g_AlbedoMap3, slot 4 = g_DetailNormalMap
    albedo =
      byNameLow.get("g_floortexture") ??
      byNameLow.get("g_wallxytexture") ??
      byNameLow.get("g_albedomap2") ??
      byNameLow.get("g_albedomap3") ??
      byNameLow.get("g_albedomap") ??
      getUnnamedSlot(bySlot, slots, 1, 1);
    mask =
      byNameLow.get("g_maskmap") ??
      byNameLow.get("g_blendmap") ??
      getUnnamedSlot(bySlot, slots, 2, 2);
    normal =
      byNameLow.get("g_detailnormalmap") ??
      byNameLow.get("g_normalmap") ??
      getUnnamedSlot(bySlot, slots, 4);
  } else if (isJwl) {
    // JWL00 / JWL01: slot 0 = g_AlbedoMap1, slot 1 = g_AlbedoMap2, slot 2 = g_NormalMap
    albedo =
      byNameLow.get("g_albedomap1") ??
      byNameLow.get("g_albedomap") ??
      getUnnamedSlot(bySlot, slots, 0, 0);
    mask = byNameLow.get("g_albedomap2") ?? getUnnamedSlot(bySlot, slots, 1, 1);
    normal =
      byNameLow.get("g_normalmap") ??
      getUnnamedSlot(bySlot, slots, 2, 1) ??
      getUnnamedSlot(bySlot, slots, 3, 2) ??
      slots.find((s) => s.role === "normal");
  } else {
    // Standard PBS00 / PBS30 / PBS35 / PBS40 / Gls / HZR00 / CNS
    albedo =
      byNameLow.get("g_albedomap") ??
      byNameLow.get("g_albedomap1") ??
      byNameLow.get("g_floortexture") ??
      byNameLow.get("g_wallxytexture") ??
      getUnnamedSlot(bySlot, slots, 0, 0) ??
      slots.find((s) => s.role === "albedo") ??
      slots[0];
    mask =
      byNameLow.get("g_maskmap") ??
      byNameLow.get("g_albedomap2") ??
      byNameLow.get("g_blendmap") ??
      byNameLow.get("g_flowmap") ??
      byNameLow.get("g_pulsemap") ??
      byNameLow.get("g_patternmap") ??
      getUnnamedSlot(bySlot, slots, 2, 1) ??
      slots.find(
        (s) => s !== albedo && (s.role === "mask" || s.role === "spec")
      );
    normal =
      byNameLow.get("g_normalmap") ??
      byNameLow.get("g_normalmap1") ??
      getUnnamedSlot(bySlot, slots, 3, 2) ??
      getUnnamedSlot(bySlot, slots, 1) ??
      slots.find((s) => s !== albedo && s !== mask && s.role === "normal");
    normal2 =
      byNameLow.get("g_normalmap2") ??
      byNameLow.get("g_detailnormalmap") ??
      getUnnamedSlot(bySlot, slots, 4, 3);
  }

  const envCube =
    byNameLow.get("g_envmap") ??
    getUnnamedSlot(bySlot, slots, 5) ??
    slots.find((s) => s.role === "env");
  const irradCube =
    byNameLow.get("g_irradiancemap") ?? getUnnamedSlot(bySlot, slots, 12);
  return {
    albedo,
    normal,
    mask,
    normal2,
    normal3,
    indexMap,
    envCube,
    irradCube,
  };
}

// ============================================================================
// 1. PBS00 — Reference Metallic-Roughness PBR (`PS_FP_Pbs00_*`)
// Also handles PBS01, PBS02, PBS03, PBS04, PBS21, PBS30, PBS35, PBS40, PBS91, PBS99
// via `uB3PbsVariant`:
//   0  = PBS00 / PBS30 / PBS35 / PBS40 (MaskMap R=metallic, G=roughness, B=emissive/AO)
//   1  = PBS01 (metallic from albedo.a remapped by g_MetalMinMax, roughness from normal.a remapped by g_RoughnessMinMax)
//   2  = PBS02 (dual normal + clear coat: primary roughness in normal2.a, layer-2 roughness in normal.a, coat blends both)
//   3  = PBS03 (no mask map: roughness from normal.a unless g_bRoughnessOverWrite, AO from normal.b, scalar g_Metallic)
//   4  = PBS04 (no mask map: roughness from normal.a, AO from normal.b, RNM detail normal on slot 4, metallic from albedo.a in D variant else g_Metallic)
//   21 = PBS21 (baked glass PBR: no material block overrides; metallic = albedo.a, roughness = normal.a, output alpha = 1.0)
//   91 = PBS91 (leanest PBR: no mask map; normal.rg = normal, normal.b = metallic, normal.a^2 = roughness)
//   99 = PBS99 (Maziora thin-film interference using g_mazioraParam1/2/3 + g_MetalMinMax & g_RoughnessMinMax)
// ============================================================================
const B3_PBS_FRAGMENT_SHADER = /* glsl */ `
${B3_PBR_COMMON_GLSL}

uniform sampler2D uTexAlbedo;
uniform sampler2D uTexMask;
uniform sampler2D uTexNormal;
uniform sampler2D uTexNormal2;
uniform samplerCube uTexEnv;
uniform samplerCube uTexIrradiance;

uniform int uB3PbsVariant;
uniform int uB3PbsHasDFlag;
uniform int uHasTexAlbedo;
uniform int uHasTexMask;
uniform int uHasTexNormal;
uniform int uHasTexNormal2;

uniform float g_UseNormalMap;
uniform float g_bAlbedoOverWrite;
uniform float g_bRoughnessOverWrite;
uniform float g_bMetalicOverWrite;
uniform float g_1BitMask;
uniform vec2 g_Tile;
uniform vec2 g_Tile2;
uniform vec2 g_DetailNormalTile;
uniform float g_UseDetailNormalMap;
uniform float g_DetailNormalIntensity;
uniform float g_AlbedoAMetallic;
uniform float g_SpecRefRate;
uniform float g_UseOcclusionMap;
uniform float g_UV2Use;
uniform vec3 g_AlbedoColor;
uniform float g_EmissiveIntensity;
uniform float g_EmissiveMaskIntensity;
uniform vec3 g_EmissiveColor;
uniform float g_bUseEmissiveColor;
uniform float g_Roughness;
uniform float g_Metallic;

// PBS01 & PBS99 remap bounds
uniform vec2 g_RoughnessMinMax;
uniform vec2 g_MetalMinMax;

// PBS02 clear-coat & PBS99 Maziora params
uniform float g_bRoughnessOverWrite2;
uniform float g_ClearCoatRate;
uniform float g_ClearCoatFresnel;
uniform vec2 g_ClearCoatFresnelMinMax;
uniform float g_Roughness2;
uniform vec3 g_BottomAlbedoColor;
uniform vec3 g_MazioraParams; // x = Param1, y = Param2, z = Param3

void main() {
  vec2 baseUv = (g_UV2Use > 1.5) ? vUv2 : ((g_UV2Use > 0.5) ? vUv.zw : vUv.xy);
  vec2 uv = baseUv * g_Tile;

  vec4 sAlbedo = (uHasTexAlbedo == 1) ? texture2D(uTexAlbedo, uv) : vec4(0.82, 0.82, 0.85, 1.0);
  vec4 sMask = (uHasTexMask == 1) ? texture2D(uTexMask, uv) : vec4(0.0, 0.45, 1.0, 1.0);
  vec4 sNormal = (uHasTexNormal == 1) ? texture2D(uTexNormal, uv) : vec4(0.5, 0.5, 1.0, 0.45);

  float alpha = sAlbedo.a;
  if (uDebugMode == 0 && g_1BitMask > 0.5 && alpha <= 0.3333) {
    discard;
  }

  vec3 V = normalize(-vEyeW);
  vec3 NwGeo = normalize(vNrmW);
  vec3 NvGeo = normalize(vNrmV);
  vec3 Nw = NwGeo;
  vec3 Nv = NvGeo;

  // PBS02 clear-coat factor (SHADER_ALGORITHMS_DEEP.md §3 lines 493-500)
  float coat = 0.0;
  if (uB3PbsVariant == 2) {
    float ndv = saturate(dot(NwGeo, V));
    float f = pow(max(ndv, 1e-4), max(g_ClearCoatFresnel, 0.1));
    float fDenom = max(g_ClearCoatFresnelMinMax.y - g_ClearCoatFresnelMinMax.x, 1e-4);
    float tCoat = saturate((f - g_ClearCoatFresnelMinMax.x) / fDenom);
    float sCoat = tCoat * tCoat * (3.0 - 2.0 * tCoat);
    coat = clamp(g_ClearCoatRate * (1.0 - sCoat), 0.0, 1.0);
  }

  vec2 uv1Valid = ResolveLightMapUv(vUv, vUv2);

  // Normal decode: always reconstructs Z from RG
  if (g_UseNormalMap > 0.5 && uHasTexNormal == 1) {
    vec3 nTS = DecodeB3NormalTS(sNormal);
    if (uUseSecondaryNormals == 1 && uB3PbsVariant == 2 && uHasTexNormal2 == 1) {
      // PBS02 blends primary normal (slot 3) and secondary normal (slot 2, tiled by g_Tile2) by coat weight (§3 line 535)
      vec4 sNorm2 = texture2D(uTexNormal2, baseUv * g_Tile2);
      vec3 nTS2 = DecodeB3NormalTS(sNorm2);
      nTS = normalize(mix(nTS, nTS2, coat));
    } else if (uUseSecondaryNormals == 1 && uB3PbsVariant == 4 && uHasTexNormal2 == 1) {
      // PBS04 RNM detail normal blend on attr.uv1 * g_DetailNormalTile (SHADER_ALGORITHMS_DEEP.md §01b line 1044)
      vec2 uvDetail = uv1Valid * g_DetailNormalTile;
      vec4 sDetail = texture2D(uTexNormal2, uvDetail);
      vec2 nBase = sNormal.rg * 2.0 - 1.0;
      vec2 nHat = nBase * inversesqrt(1.0 + dot(nBase, nBase));
      vec2 nDet = (sDetail.rg * 2.0 - 1.0) * max(g_DetailNormalIntensity, 0.25);
      vec2 nBlend = nDet + nHat;
      float zBlend = sqrt(max(0.0, 1.0 - dot(nBlend, nBlend)));
      nTS = normalize(vec3(nBlend, zBlend));
    } else if (uUseSecondaryNormals == 1 && (uB3PbsVariant == 0 || uB3PbsVariant == 3) && uHasTexNormal2 == 1 && (g_UseDetailNormalMap > 0.5 || uB3PbsVariant == 3)) {
      // PBS00 / PBS03 / PBS40 detail normal map tiled by g_DetailNormalTile
      vec2 uvDetail = baseUv * g_DetailNormalTile;
      vec4 sDetail = texture2D(uTexNormal2, uvDetail);
      vec3 nDetTS = DecodeB3NormalTS(sDetail);
      float detStr = clamp(g_DetailNormalIntensity, 0.15, 1.5);
      nTS = normalize(vec3(nTS.xy + nDetTS.xy * detStr, nTS.z));
    }
    Nw = PerturbB3Normal(nTS, 1.0, vTanW, NwGeo);
    Nv = PerturbB3Normal(nTS, 1.0, vTanV, NvGeo);
  }

  // Base Albedo (overwritten by g_AlbedoColor when g_bAlbedoOverWrite != 0)
  vec3 base = (g_bAlbedoOverWrite > 0.5)
    ? DecodeB3Albedo(g_AlbedoColor)
    : DecodeB3Albedo(sAlbedo.rgb);

  // Roughness, Metallic, and AO per PBS family variant
  float rough = 0.45;
  float metal = 0.0;
  float ao = 1.0;

  if (uB3PbsVariant == 1) {
    // PBS01 (§2 lines 293-303): metallic from albedo.a remapped by [g_MetalMin, g_MetalMax]; roughness from normal.a
    float rIn = (g_UseNormalMap > 0.5 && uHasTexNormal == 1) ? sNormal.a : g_Roughness;
    float rawR = mix(g_RoughnessMinMax.x, g_RoughnessMinMax.y, rIn);
    float rawM = mix(g_MetalMinMax.x, g_MetalMinMax.y, sAlbedo.a);
    rough = (g_bRoughnessOverWrite > 0.5) ? g_Roughness : rawR;
    metal = (g_bMetalicOverWrite > 0.5) ? g_Metallic : rawM;
    alpha = 1.0;
  } else if (uB3PbsVariant == 2) {
    // PBS02 (§3 lines 471-483): primary roughness in slot 2 .a (uTexNormal2), second-layer roughness in slot 3 .a (uTexNormal)
    vec4 sNorm2 = (uHasTexNormal2 == 1) ? texture2D(uTexNormal2, baseUv * g_Tile2) : sNormal;
    float r1 = (g_bRoughnessOverWrite > 0.5 || uHasTexNormal2 == 0) ? g_Roughness : sNorm2.a;
    float r2 = (g_bRoughnessOverWrite2 > 0.5 || uHasTexNormal == 0) ? g_Roughness2 : sNormal.a;
    rough = mix(r2, r1, coat);
    metal = g_Metallic;
    if (dot(g_BottomAlbedoColor, g_BottomAlbedoColor) > 1e-4) {
      base = mix(DecodeB3Albedo(g_BottomAlbedoColor), base, coat);
    }
    alpha = 1.0;
  } else if (uB3PbsVariant == 3) {
    // PBS03 (§01b lines 722-748): roughness in normal.a unless g_bRoughnessOverWrite; AO in normal.b; scalar g_Metallic
    rough = (g_bRoughnessOverWrite > 0.5 || uHasTexNormal == 0) ? g_Roughness : sNormal.a;
    metal = g_Metallic;
    if (uHasTexNormal == 1) {
      ao = clamp(sNormal.b, 0.15, 1.0);
    }
    alpha = 1.0;
  } else if (uB3PbsVariant == 4) {
    // PBS04 (§01b lines 972-1012): roughness in normal.a, AO in normal.b; metallic = albedo.a when g_AlbedoAMetallic (PG1[5]) != 0 or D flag
    rough = (g_bRoughnessOverWrite > 0.5 || uHasTexNormal == 0) ? g_Roughness : sNormal.a;
    metal = ((uB3PbsHasDFlag == 1 || g_AlbedoAMetallic > 0.5) && uHasTexAlbedo == 1) ? sAlbedo.a : g_Metallic;
    if (uHasTexNormal == 1) {
      ao = clamp(sNormal.b, 0.15, 1.0);
    }
    alpha = 1.0;
  } else if (uB3PbsVariant == 21) {
    // PBS21 (§01c lines 1261-1278): baked glass PBR; metallic = albedo.a, roughness = normal.a, out.a = 1.0
    rough = (uHasTexNormal == 1) ? sNormal.a : g_Roughness;
    metal = (uHasTexAlbedo == 1) ? sAlbedo.a : g_Metallic;
    alpha = 1.0;
  } else if (uB3PbsVariant == 91) {
    // PBS91 (§01d lines 2030-2069): normal.b = metallic, normal.a^2 = roughness
    rough = (uHasTexNormal == 1) ? (sNormal.a * sNormal.a) : g_Roughness;
    metal = (uHasTexNormal == 1) ? sNormal.b : g_Metallic;
    alpha = 1.0;
  } else if (uB3PbsVariant == 99) {
    // PBS99 (§01d lines 2205-2234): no mask map; mode 0 uses g_RoughnessMin / g_MetalMin overridden by g_Roughness / g_Metallic
    rough = (g_bRoughnessOverWrite > 0.5) ? g_Roughness : g_RoughnessMinMax.x;
    metal = (g_bMetalicOverWrite > 0.5) ? g_Metallic : g_MetalMinMax.x;
    alpha = 1.0;
  } else {
    // PBS00 / PBS30 / PBS35 / PBS40: MaskMap R = metallic, G = roughness
    rough = (g_bRoughnessOverWrite > 0.5 || uHasTexMask == 0) ? g_Roughness : sMask.g;
    metal = (g_bMetalicOverWrite > 0.5 || uHasTexMask == 0) ? g_Metallic : sMask.r;
    if (g_UseOcclusionMap > 0.5 && uHasTexMask == 1) {
      ao = clamp(sMask.b, 0.15, 1.0);
    }
  }

  rough = clamp(rough, 0.04, 1.0);
  metal = clamp(metal, 0.0, 1.0);

  // PBS99 Maziora thin-film interference shift (§01d lines 2241-2255)
  if (uB3PbsVariant == 99) {
    float ndotv = saturate(dot(Nw, V));
    float thickness = max(g_MazioraParams.x, 0.5) * (1.0 - ndotv);
    float amp = max(0.0, 1.0 + thickness * 0.25);
    vec3 w = (sin(thickness * vec3(15.7, 14.13, 17.27)) * amp * 0.5 + 0.5) * g_MazioraParams.y;
    vec3 factor = vec3(1.0) + g_MazioraParams.z * (w - vec3(1.0));
    base *= max(factor, vec3(0.0));
  }

  // PBS00 F0 and diffuse weight (SHADER_ALGORITHMS_DEEP.md §1 lines 117-118)
  vec3 F0 = mix(vec3(0.03), base, metal);
  vec3 diffK = base * (1.0 - metal);
  float specScale = max(g_SpecRefRate, 0.0);

  vec3 direct = EvaluateB3DirectPbr(Nw, V, diffK, rough, F0);
  vec3 ibl = EvaluateB3Ibl(Nw, V, diffK, rough, F0, ao, specScale, uTexEnv, uTexIrradiance);
  vec3 colour = direct + ibl;

  // Emissive contribution (PBS00 / PBS30 / PBS40 §01d lines 1886-1890: masked by sMask.b * g_EmissiveMaskIntensity)
  if (g_EmissiveIntensity > 0.001 || g_bUseEmissiveColor > 0.5) {
    vec3 emCol = (g_bUseEmissiveColor > 0.5) ? g_EmissiveColor : base;
    float emMask = (uHasTexMask == 1) ? (sMask.b * max(g_EmissiveMaskIntensity, 1.0)) : 1.0;
    float emScale = max(g_EmissiveIntensity, g_bUseEmissiveColor > 0.5 ? 1.0 : 0.0);
    colour += emCol * emMask * emScale * 0.35;
  }

  gl_FragColor = FinishB3Fragment(colour, alpha, Nw, Nv, vPosV);
}
`;

// ============================================================================
// 2. BAYO00 — Bayonetta Hybrid Body PBR + Hair-Cloth (`PS_FP_Bayo00_*`)
// ============================================================================
const B3_BAYO00_FRAGMENT_SHADER = /* glsl */ `
${B3_PBR_COMMON_GLSL}

uniform sampler2D uTexAlbedo;
uniform sampler2D uTexNormal;
uniform sampler2D uTexNormal2;
uniform sampler2D uTexNormal3;
uniform sampler2D uTexIndex;
uniform samplerCube uTexEnv;
uniform samplerCube uTexIrradiance;

uniform int uHasTexAlbedo;
uniform int uHasTexNormal;
uniform int uHasTexNormal2;
uniform int uHasTexNormal3;
uniform int uHasTexIndex;

uniform float g_1BitMask;
uniform float g_HairNormalIntensity;
uniform float g_HairMetallic;
uniform float g_PriAlpha;
uniform float g_SecAlpha;
uniform vec3 g_PriSpecularColor;
uniform float g_PriRoughness;
uniform vec3 g_SecSpecularColor;
uniform float g_SecRoughness;
uniform float g_SpecularPower;
uniform float g_HairOffset;
uniform vec3 g_HairColor;
uniform float g_DiffuseIntensity;
uniform vec2 g_AmbientOcclusionMinMax;
uniform vec2 g_SpecularMaskMinMax;
uniform vec2 g_OcclusionMaskMinMax;

void main() {
  vec2 uv = vUv.xy;

  vec4 sAlbedo = (uHasTexAlbedo == 1) ? texture2D(uTexAlbedo, uv) : vec4(0.22, 0.20, 0.26, 1.0);
  // Slot 1 g_NormalMap: RG = normal, B = AO, A = roughness (SHADER_ALGORITHMS_DEEP.md §1.1)
  vec4 sNorm = (uHasTexNormal == 1) ? texture2D(uTexNormal, uv) : vec4(0.5, 0.5, 0.85, 0.45);
  // Slot 4 g_NormalMap3: RG = hair normal, B = spec/occlusion mask, A = normal-blend / hair mask
  vec4 sNorm3 = (uHasTexNormal3 == 1) ? texture2D(uTexNormal3, uv) : vec4(0.5, 0.5, 0.75, 0.0);
  // Slot 7 g_MaskMap: R = metallic, GB = recolour digits, A = hair-region mask
  vec4 sIndex = (uHasTexIndex == 1) ? texture2D(uTexIndex, uv) : vec4(0.0, 0.0, 0.0, 0.25);

  if (uDebugMode == 0 && g_1BitMask > 0.5 && sAlbedo.a <= 0.3333) {
    discard;
  }

  // Blend body normal (slot 1 .rg) and hair normal (slot 4 .rg) by hair region mask (slot 7 .a * slot 4 .a)
  float hairBlend = clamp(sIndex.a * saturate((sNorm3.a - 0.02) * max(g_HairNormalIntensity, 1.0)), 0.0, 1.0);
  vec3 nBodyTS = DecodeB3NormalTS(sNorm);
  vec3 nMixTS = nBodyTS;
  if (uUseSecondaryNormals == 1) {
    vec3 nHairTS = (uHasTexNormal3 == 1) ? DecodeB3NormalTS(sNorm3) : nBodyTS;
    nMixTS = normalize(mix(nBodyTS, nHairTS, hairBlend));
    if (uHasTexNormal2 == 1) {
      vec4 sNorm2 = texture2D(uTexNormal2, uv);
      vec3 nDetTS = DecodeB3NormalTS(sNorm2);
      nMixTS = normalize(vec3(nMixTS.xy + nDetTS.xy * 0.35, nMixTS.z));
    }
  }

  vec3 Nw = normalize(vNrmW);
  vec3 Nv = normalize(vNrmV);
  if (uHasTexNormal == 1) {
    Nw = PerturbB3Normal(nMixTS, 1.0, vTanW, Nw);
    Nv = PerturbB3Normal(nMixTS, 1.0, vTanV, Nv);
  }

  // Body channels (SHADER_ALGORITHMS_DEEP.md §1.3 & §1.5)
  vec3 base = DecodeB3Albedo(sAlbedo.rgb);
  // If recolour mask (sIndex.g or sIndex.b) is active, tint toward g_HairColor using sAlbedo.g
  float recolourWeight = clamp(max(sIndex.g, sIndex.b), 0.0, 1.0);
  if (recolourWeight > 0.05) {
    vec3 palCol = mix(base, DecodeB3Albedo(g_HairColor), 0.65);
    base = mix(base, sAlbedo.g * palCol, recolourWeight * 0.5);
  }

  float metal = (uHasTexIndex == 1) ? clamp(sIndex.r, 0.0, 1.0) : clamp(g_HairMetallic, 0.0, 1.0);
  float rough = (uHasTexNormal == 1) ? clamp(sNorm.a, 0.05, 1.0) : clamp(g_PriRoughness, 0.05, 1.0);
  float aoBody = (uHasTexNormal == 1)
    ? clamp(mix(g_AmbientOcclusionMinMax.x, g_AmbientOcclusionMinMax.y, sNorm.b), 0.1, 1.0)
    : 1.0;
  float occMask = (uHasTexNormal3 == 1)
    ? clamp(mix(g_OcclusionMaskMinMax.x, g_OcclusionMaskMinMax.y, sNorm3.b), 0.1, 1.0)
    : 1.0;
  float ao = mix(aoBody, aoBody * occMask, hairBlend);
  float specMask = (uHasTexNormal3 == 1)
    ? clamp(mix(g_SpecularMaskMinMax.x, g_SpecularMaskMinMax.y, sNorm3.b), 0.1, 1.0)
    : 1.0;

  vec3 F0_body = mix(vec3(0.03), base, metal);
  vec3 diff_body = base * (1.0 - metal) * max(g_DiffuseIntensity, 0.25);

  vec3 V = normalize(-vEyeW);

  // Body PBR lighting + IBL
  vec3 directBody = EvaluateB3DirectPbr(Nw, V, diff_body, rough, F0_body);
  vec3 iblBody = EvaluateB3Ibl(Nw, V, diff_body, rough, F0_body, ao, specMask, uTexEnv, uTexIrradiance);

  // Two-lobe anisotropic hair strand highlight rotated by g_HairOffset (§1.5 lines 2621-2637)
  vec3 Tstrand = RotateStrandTangentB3(Nw, vTanW, g_HairOffset);
  vec3 hairTint = DecodeB3Albedo(g_HairColor);
  vec3 priF0 = mix(vec3(0.03), hairTint * g_PriSpecularColor, clamp(g_HairMetallic, 0.0, 1.0));
  vec3 secF0 = hairTint * g_SecSpecularColor * max(g_SecRoughness, 0.15);
  float roughPri = clamp(g_SecRoughness * g_PriRoughness, 0.05, 1.0);
  float roughSec = clamp(g_SecRoughness * g_SecRoughness, 0.05, 1.0);

  vec3 hairSpec =
    EvaluateB3HairDualLobes(
      Nw,
      Tstrand,
      V,
      STUDIO_KEY_TO_LIGHT,
      priF0,
      roughPri,
      g_PriAlpha,
      secF0,
      roughSec,
      g_SecAlpha,
      g_SpecularPower
    ) +
    EvaluateB3HairDualLobes(
      Nw,
      Tstrand,
      V,
      STUDIO_FILL_TO_LIGHT,
      priF0,
      roughPri,
      g_PriAlpha * 0.45,
      secF0,
      roughSec,
      g_SecAlpha * 0.45,
      g_SpecularPower
    );

  float hairGate = max(sIndex.a, 0.35) * specMask;
  vec3 colour = directBody + iblBody + hairSpec * hairGate;

  gl_FragColor = FinishB3Fragment(colour, 1.0, Nw, Nv, vPosV);
}
`;

// ============================================================================
// 3. Hair20 / Hair21 / Hair22 — Layered Anisotropic Strand Hair (`PS_FP_Hair20/21/22_*`)
// ============================================================================
const B3_HAIR_FRAGMENT_SHADER = /* glsl */ `
${B3_PBR_COMMON_GLSL}

uniform sampler2D uTexAlbedo;
uniform sampler2D uTexMask;
uniform sampler2D uTexNormal;
uniform sampler2D uTexNormal2;
uniform samplerCube uTexEnv;
uniform samplerCube uTexIrradiance;

// 20 = Hair20 / FUR02 / CES00, 21 = Hair21, 22 = Hair22, 30 = FUR01, 40 = FBR00
uniform int uB3HairVariant;
uniform int uHasTexAlbedo;
uniform int uHasTexMask;
uniform int uHasTexNormal;
uniform int uHasTexNormal2;

uniform float g_bAlbedoOverWrite;
uniform float g_1BitMask;
uniform float g_OnlyLayer1;       // PG1[2] (1_02): 1.0 = single-layer, 0.0 = dual-layer D variant
uniform float g_EyeBlowMode;      // PG1[3] (1_03): 1.0 = eyebrow / eyelash mode
uniform float g_UseAlphaEffect;   // PG1[4] (1_04): 1.0 = standard strand alpha mode, 0.0 = lightmap-UV hair albedo mode when g_OnlyLayer1 == 0
uniform vec2 g_Tile;              // FUR01 g_Tile_X/Y (0_16/0_17)
uniform vec2 g_Tile2;             // FBR00 g_Layer2_Tiling_X/Y (0_40/0_41)
uniform vec2 g_DetailNormalTile;  // FUR01 g_NormalTile_X/Y (0_18/0_19) or FBR00 g_Layer1_Tiling_X/Y (0_28/0_29)
uniform float g_Layer1_UseNormalMap;    // FBR00 (0_30)
uniform float g_Layer1_UseSpecularMask; // FBR00 (0_31)
uniform float g_Metallic;
uniform float g_PriAlpha;
uniform float g_SecAlpha;
uniform vec3 g_PriSpecularColor;
uniform float g_PriRoughness;
uniform vec3 g_SecSpecularColor;
uniform float g_SecRoughness;
uniform float g_TransStrength;
uniform float g_SpecularPower;
uniform float g_HairOffset;
uniform vec3 g_LayerColor1;
uniform float g_DiffuseIntensity;
uniform vec3 g_LayerColor2;
uniform float g_WrapDiffuseStrength;
uniform vec2 g_AmbientOcclusionMinMax;
uniform vec2 g_SpecularMaskMinMax;
uniform float g_NormalIntensity;
uniform float g_BinormalIntensity; // Hair20: ~1.0 on most materials (0.5-3.0); scales the pink strand map
uniform float g_IridescenceStrength;
uniform float g_IridescenceCoefficient;
uniform float g_IridescenceCoefficient3;
uniform vec3 g_IridescencePhase; // Hair20: vec3(g_IridescenceR, g_IridescenceG, g_IridescenceB) in radians

// Hair21 & Hair22 dedicated tiling & iridescence uniforms (SHADER_ALGORITHMS_DEEP.md §3.1 & §4.4)
uniform vec2 g_TilingMap1UV;
uniform vec2 g_TilingMap2UV;
uniform vec3 g_IridescenceStr;     // x = Diff, y = PriSpec, z = SecSpec
uniform vec3 g_IridescenceFresnel; // x = Pow, y = Min, z = Max
uniform vec4 g_IridescenceCoord;   // x = ViewRate, y = MapVRate, z = VTile, w = VOffset

// ============================================================================
// Hair20 strand model, ported from the game's PS_FP_Hair20 (Maxwell -> GLSL transpile).
//   pink g_NormalMap2: R,G = strand TANGENT in tangent space (x*BinormalIntensity, y ~ -1, z = 1-|xy|), B = spec mask, A = coverage
//   sampled twice (2nd = parallax-shifted), blended by smoothstep(g_HairMin, g_HairMax, N.V)
//   normal map is applied in the (T, W) frame (its Y axis follows the strand), lobes are Marschner-style Gaussians
//   in sin(theta_r) + sin(theta_i - shift), g_HairOffset is the cuticle tilt (secondary shift = 4 * g_HairOffset)
// Values that live in the game's cb1/cb9 and can't be recovered from the dump are the B3H_* constants below.
// ============================================================================
uniform vec2  g_HairMinMax;               // (g_HairMin, g_HairMax): view-angle blend range
uniform float g_ParallaxIntensity;        // 0 on 40/104 materials, up to 0.2
uniform float g_BumpHeight;               // 0 or -0.03
uniform float g_ShadowSpecularIntensity;  // 0.5 on 103/104 materials; 0 if not uploaded (= plain N.L like before)

// Layer "B" set (game: lerp(A, B, g_NormalMap.a) when g_UseLayerColorB is on) and the other params decoded from the game shader.
// Anything not uploaded reads as 0 and falls back to the previous behaviour.
uniform float g_UseLayerColorB;           // 1 on 3/104 materials (inferred to be the game's "C" variant)
uniform vec3  g_LayerColorB1;
uniform vec3  g_LayerColorB2;
uniform float g_DiffuseIntensityB;
uniform float g_WrapDiffuseStrengthB;
uniform vec2  g_OcclusionMaskMinMax;      // (g_OcclusionMaskMin, g_OcclusionMaskMax), blended with the pink map's B
uniform float g_FresnelExp;               // transmission curve: smoothstep(remap(lerp(Fraction, 1, (1-N.V)^Exp), Min, Max))
uniform float g_FresnelFraction;
uniform float g_FresnelMin;
uniform float g_FresnelMax;

const float B3H_AO_ON_DIRECT      = 0.0;  // game applies baked AO only to the ambient/IBL term. 1 = also darken direct light + spec (v4 behaviour).
const float B3H_AO_FORCE_VISIBLE  = 0.0;  // 1 = old hack: when AmbientOcclusionMin >= Max, still show the baked AO.

const float B3H_LIGHT_RADIANCE   = 50.0;  // overall spec scale vs. the game's light units. Tune to taste.
const float B3H_ROUGH_PULL       = 0.0;  // c9[46]: pulls roughness toward 0.01 (x0.4). Unknown, 0 = off.
const float B3H_TAN_CLAMP        = 1.0;  // c1[11]: max angle in the tan() intensity shaping. Unknown, keep < 1.55.
const float B3H_LIGHT_TO_NORMAL  = 0.0;  // c9[59]: lerp light dir toward the shading normal. Unknown, 0 = pure light dir.
const float B3H_LIGHT_WRAP       = 1.0;  // c5[79]: N.L wrap on the spec visibility. 1 = plain N.L.
const float B3H_BUMP_SCALE       = 1.0;  // c1[8]: multiplies g_BumpHeight in the parallax denominator.
const float B3H_AZIMUTH          = 1.0;  // 1 = literal azimuth width term from the game, 0 = off.
const float B3H_BETA_MIN         = 0.01; // floor on the primary lobe width (the azimuth term can drive it to 0).
const float B3H_BITANGENT_SIGN   = 1.0;  // flip if streaks / normal-map Y look mirrored.
const float B3H_FILL_WEIGHT      = 0.5;  // fill light weight (same 0.5 as the old evaluator).

vec3 B3HairPinkVec(vec4 s, float k) {
  vec2 xy = s.rg * 2.0 - 1.0;
  float len = sqrt(saturate(dot(xy, xy)));
  return normalize(vec3(xy.x * k, xy.y, (1.0 - len) * k));
}

vec3 B3HairStrandDir(vec3 pv, vec3 N, vec3 T) {
  vec3 B = cross(N, T) * B3H_BITANGENT_SIGN;
  return normalize(pv.z * N + pv.x * T - pv.y * B);
}

vec3 B3HairShadingNormal(vec2 nxy, float intensity, vec3 N, vec3 T, vec3 W) {
  float nl = min(length(nxy), 1.0);
  return normalize((1.0 - nl) * N + intensity * (nxy.x * T - nxy.y * W));
}

float B3HairGauss(float x, float beta) {
  float b = max(beta, B3H_BETA_MIN);
  return 0.398942 / b * exp(-0.5 * x * x / (b * b));
}

// lerp(1, tan(clamp(1.5 * cos^2(angle))), s): the game's intensity shaping on each lobe
float B3HairShape(float cosAngle, float s) {
  float a = clamp(1.5 * cosAngle * cosAngle, 0.0, min(B3H_TAN_CLAMP, 1.55));
  return mix(1.0, tan(a), s);
}

vec3 EvaluateB3HairStrandLobes(
  vec3 W, vec3 Ns, vec3 NgW, vec3 V, vec3 L,
  vec3 priF0, float priEff, float priAlpha,
  vec3 secF0, float secEff, float secAlpha,
  float hairOffset, float shapeMix, float shadowSpecInt,
  float transMask, bool enableIrid
) {
  vec3 Lb = normalize(mix(L, Ns, B3H_LIGHT_TO_NORMAL));
  float sR = dot(V, W);
  float sI = dot(Lb, W);
  float cI = sqrt(max(1.0 - sI * sI, 0.0));

  float az = 1.0;
  if (B3H_AZIMUTH > 0.5) {
    vec3 A  = V  - sR * Ns;
    vec3 Bq = Lb - sI * Ns;
    float d = dot(A, Bq) * inversesqrt(max(dot(A, A) * dot(Bq, Bq), 1e-8));
    az = sqrt(saturate(0.5 * d + 0.5));
  }

  // primary: unshifted, width = az * PriRough * SecRough * 0.95
  float gP = B3HairGauss(sR + sI, az * priEff * secEff * 0.95) * B3HairShape(cI, shapeMix);

  // secondary: incident angle shifted by 4 * g_HairOffset, width = SecRough^2
  float a4 = 4.0 * hairOffset;
  float sinI4 = sI * cos(a4) - cI * sin(a4);
  float cosI4 = cI * cos(a4) + sI * sin(a4);
  float gS = B3HairGauss(sR + sinI4, secEff * secEff) * B3HairShape(cosI4, shapeMix);

  // Hair20 vR iridescence: dedicated 2 * g_HairOffset lobe (width = 1.4 * secEff)
  // multiplied by (1 - 2 * burleyF) -> cosine phase in radians, modulating specular only.
  vec3 irid = vec3(1.0);
  if (enableIrid && g_IridescenceStrength > 1e-4) {
    float a2 = 2.0 * hairOffset;
    float sinI2 = sI * cos(a2) - cI * sin(a2);
    float cosI2 = cI * cos(a2) + sI * sin(a2);
    float iridLobe = B3HairGauss(sR + sinI2, secEff * 1.4) * B3HairShape(cosI2, shapeMix);

    vec3 H = normalize(V + L);
    float ldh = saturate(dot(L, H));
    float fv = 1.0 - saturate(dot(Ns, V));
    float fl = 1.0 - saturate(dot(Ns, L));
    float fv2 = fv * fv;
    float fl2 = fl * fl;
    float r2 = secEff * secEff;
    float fd90 = 0.5 * r2 + 2.0 * r2 * ldh * ldh;
    float burleyF = (1.0 + (fd90 - 1.0) * (fv2 * fv2 * fv))
                  * (1.0 + (fd90 - 1.0) * (fl2 * fl2 * fl));
    float fresnelSign = 1.0 - 2.0 * burleyF;

    vec3 phase = (g_IridescencePhase + iridLobe * fresnelSign) * g_IridescenceCoefficient
               + g_IridescenceCoefficient3;
    vec3 wave = 0.5 * cos(phase) + 0.5;
    float iridWeight = g_IridescenceStrength * g_IridescenceStrength * (1.0 - saturate(transMask));
    irid = mix(vec3(1.0), wave, iridWeight);
  }

  float wrapNL = saturate(1.0 + B3H_LIGHT_WRAP * (dot(NgW, L) - 1.0));
  float vis = mix(wrapNL, 1.0, saturate(shadowSpecInt));
  return (priF0 * priAlpha * gP + secF0 * secAlpha * gS) * irid * vis;
}

void main() {
  vec2 uv0 = vUv.xy;
  vec2 attrUv1 = ResolveLightMapUv(vUv, vUv2);
  // In Hair20, when g_OnlyLayer1 == 0, g_EyeBlowMode == 0, and g_UseAlphaEffect == 0 (e.g. pl3101 NEWhair_822),
  // g_AlbedoMap1 is baked in lightmap UV space (attrUv1), while strand cutout alpha remains in pink g_NormalMap2.a at UV0.
  bool useLmUvForAlbedo = (uB3HairVariant == 20 && g_OnlyLayer1 < 0.5 && g_EyeBlowMode < 0.5 && g_UseAlphaEffect < 0.5);

  vec2 uvAlb = uv0;
  vec2 uv1 = uv0;
  vec2 uv2 = uv0;

  if (uB3HairVariant == 21) {
    // Hair21: no AlbedoMap1; slot 1 uses UV0 * g_TilingMap1UV, slot 3 uses UV0 * g_TilingMap2UV
    uv1 = uv0 * g_TilingMap1UV;
    uv2 = uv0 * g_TilingMap2UV;
  } else if (uB3HairVariant == 22) {
    // Hair22 (§4.5 lines 2990-2991): slot 1 uses UV0 * g_TilingMap1UV, slot 3 uses UV1 * g_TilingMap2UV
    uv1 = uv0 * g_TilingMap1UV;
    uv2 = attrUv1 * g_TilingMap2UV;
  } else if (uB3HairVariant == 30) {
    // FUR01 (§07b §2.2 lines 10550-10551): albedo uses UV0 * g_Tile, normal uses UV0 * g_NormalTile
    uvAlb = uv0 * g_Tile;
    uv1 = uv0 * g_DetailNormalTile;
    uv2 = uv1;
  } else if (uB3HairVariant == 40) {
    // FBR00 (§07b §1.1 & §1.5): slot 0 (Albedo1) and slot 6 (Normal3 base normal) use un-tiled UV0;
    // slot 1 (Normal1 detail normal + .a specMask) uses UV0 * g_Layer1_Tiling_X/Y
    uvAlb = uv0;
    uv1 = uv0;
    uv2 = uv0 * g_DetailNormalTile;
  } else {
    // Hair20 (§2.4):
    // - g_AlbedoMap1 (uTexAlbedo) uses UV0 by default, or lightmap UV (attrUv1) when useLmUvForAlbedo is true (pl3101 NEWhair_822).
    // - g_NormalMap.rg (uTexNormal) and pink g_NormalMap2.rgba (uTexNormal2) use UV0 (vUv.xy).
    // - g_NormalMap.ba (uTexNormal B = AO, A = lightmap-space mask) uses lightmap UV (attrUv1).
    uvAlb = useLmUvForAlbedo ? attrUv1 : uv0;
    uv1 = uv0;
    uv2 = uv0;
  }

  vec4 sAlbedo = (uHasTexAlbedo == 1) ? texture2D(uTexAlbedo, uvAlb) : vec4(0.18, 0.16, 0.22, 1.0);
  // g_NormalMap: RG = normal1 (sampled at uv1 = UV0 on Hair20), B = AO & A = macro alpha (sampled at lightmap UV attrUv1 on Hair20)
  vec4 sNorm1 = (uHasTexNormal == 1) ? texture2D(uTexNormal, uv1) : vec4(0.5, 0.5, 0.85, 1.0);
  vec4 sNorm1Lm = (uHasTexNormal == 1)
    ? ((uB3HairVariant == 20) ? texture2D(uTexNormal, attrUv1) : sNorm1)
    : vec4(0.5, 0.5, 0.85, 1.0);
  // g_NormalMap2 (pink strand map in Hair20, or Slot 1 g_NormalMap1 in FBR00): RG = normal2, B = specular mask, A = strand alpha
  vec4 sNorm2 = (uHasTexNormal2 == 1) ? texture2D(uTexNormal2, uv2) : vec4(0.5, 0.5, 0.90, 1.0);

  float hairAlpha = 1.0;
  if (uB3HairVariant == 40) {
    // FBR00 is an opaque fabric shader (out.a = 1.0); only discard if g_1BitMask is explicitly enabled
    hairAlpha = (g_1BitMask > 0.5 && uHasTexAlbedo == 1) ? sAlbedo.a : 1.0;
    if (uDebugMode == 0 && g_1BitMask > 0.5 && hairAlpha <= 0.3333) {
      discard;
    }
  } else if (uB3HairVariant == 30) {
    // FUR01: coverage alpha is in sAlbedo.a
    hairAlpha = (uHasTexAlbedo == 1) ? sAlbedo.a : 1.0;
    if (uDebugMode == 0 && g_1BitMask > 0.5 && hairAlpha <= 0.3333) {
      discard;
    }
  } else {
    // Hair20 / Hair21 / Hair22 cutout alpha:
    // Cutout alpha comes directly from pink g_NormalMap2.a (or g_NormalMap.a when g_NormalMap2 is absent).
    // Only display pixels with alpha > 0.3333 (do not test sNorm2.a > 0.005 per-pixel, which would turn alpha == 0 into 1.0).
    if (uHasTexNormal2 == 1) {
      hairAlpha = sNorm2.a;
    } else if (uHasTexNormal == 1) {
      hairAlpha = sNorm1Lm.a;
    } else {
      hairAlpha = 1.0;
    }

    if (uDebugMode == 0 && hairAlpha <= 0.3333) {
      discard;
    }
  }

  // Decode & blend the two normal maps when NormalMap 2/3 toggle is enabled
  vec3 Nw = normalize(vNrmW);
  vec3 Nv = normalize(vNrmV);
  if (uHasTexNormal == 1) {
    vec3 n1 = DecodeB3NormalTS(sNorm1);
    if (uUseSecondaryNormals == 1 && uHasTexNormal2 == 1) {
      if (uB3HairVariant == 40) {
        if (g_Layer1_UseNormalMap > 0.5) {
          vec3 n2 = DecodeB3NormalTS(sNorm2);
          n1 = normalize(mix(n1, n2, clamp(sNorm2.a, 0.25, 0.85)));
        }
      } else if (sNorm2.g > 0.2) {
        // real two-channel normal in slot 2 (the pink strand map has G ~ 0 and is handled as a shift below)
        vec3 n2 = DecodeB3NormalTS(sNorm2);
        n1 = normalize(mix(n1, n2, clamp(g_NormalIntensity, 0.0, 1.0)));
      }
    }
    Nw = PerturbB3Normal(n1, 1.0, vTanW, Nw);
    Nv = PerturbB3Normal(n1, 1.0, vTanV, Nv);
  }

  // Rotate strand tangent frame by g_HairOffset (§2.4 lines 2794-2795)
  vec3 Tstrand = RotateStrandTangentB3(Nw, vTanW, g_HairOffset);

  // ---- Hair20 strand model: strand direction from the pink map, normal map in the (T, W) frame ----
  bool useStrandModel = (uB3HairVariant < 30 && uHasTexNormal2 == 1 && sNorm2.g < 0.2);
  vec3 hairW = Tstrand;
  float pinkB = sNorm2.b;
  // smoothstep(g_HairMin, g_HairMax, N.V): blends LayerColor1 -> LayerColor2 on dual-layer materials and the two pink-map samples
  float hairBlend = 0.0;
  {
    vec2 hmx = (g_HairMinMax.y > g_HairMinMax.x + 1e-4) ? g_HairMinMax : vec2(0.2, 1.0);
    float hbx = saturate((saturate(dot(normalize(vNrmW), normalize(-vEyeW))) - hmx.x) / (hmx.y - hmx.x));
    hairBlend = hbx * hbx * (3.0 - 2.0 * hbx);
  }
  if (useStrandModel) {
    float binI = (g_BinormalIntensity > 0.0) ? g_BinormalIntensity : 1.0;  // never 0 in the dumps: 0 = not uploaded
    vec3 N0w = normalize(vNrmW);
    vec3 N0v = normalize(vNrmV);
    vec3 T0w = SafeOrthoTangentB3(N0w, vTanW);
    vec3 T0v = SafeOrthoTangentB3(N0v, vTanV);
    vec3 Vw = normalize(-vEyeW);

    // second strand-map sample, parallax-shifted (offset is ~PI^2 as decoded, so it is tiny on real materials)
    vec4 sNorm2B = sNorm2;
    if (g_ParallaxIntensity > 0.0) {
      float den = g_BumpHeight * B3H_BUMP_SCALE + dot(Vw, N0w);
      den = (abs(den) < 1e-3) ? 1e-3 : den;
      float pk = -g_ParallaxIntensity / den;
      vec3 B0w = cross(N0w, T0w) * B3H_BITANGENT_SIGN;
      vec2 poff = vec2(dot(Vw, T0w), -dot(Vw, B0w)) * pk * g_ParallaxIntensity;
      sNorm2B = texture2D(uTexNormal2, uv2 + poff);
    }
    vec3 pv = normalize(mix(B3HairPinkVec(sNorm2, binI), B3HairPinkVec(sNorm2B, binI), hairBlend));
    pinkB = mix(sNorm2.b, sNorm2B.b, hairBlend);

    vec2 nxy = (uHasTexNormal == 1) ? (sNorm1.rg * 2.0 - 1.0) : vec2(0.0);
    hairW = B3HairStrandDir(pv, N0w, T0w);
    Nw = B3HairShadingNormal(nxy, g_NormalIntensity, N0w, T0w, hairW);
    Nv = B3HairShadingNormal(nxy, g_NormalIntensity, N0v, T0v, B3HairStrandDir(pv, N0v, T0v));
  }

  // Layer colour (§2.4 lines 2797-2799: layer = albedo.rgb * mat.g_LayerColor1; FBR00 Layer 2 blend)
  vec3 layerCol = DecodeB3Albedo(g_LayerColor1);
  // Game (Hair20/21/22): D variant (g_OnlyLayer1 == 0) blends LayerColor1 -> LayerColor2 by smoothstep(N.V);
  // C variant (g_UseLayerColorB) first lerps each layer A -> B by g_NormalMap.a (the same alpha also lerps DiffuseIntensity / WrapDiffuseStrength).
  bool useLayerB = (uB3HairVariant < 30) && (g_UseLayerColorB > 0.5) && (uHasTexNormal == 1);
  float layerBWeight = useLayerB ? sNorm1Lm.a : 0.0;
  if (uB3HairVariant < 30) {
    vec3 lc1 = DecodeB3Albedo(g_LayerColor1);
    vec3 lc2 = DecodeB3Albedo(g_LayerColor2);
    if (useLayerB) {
      lc1 = mix(lc1, DecodeB3Albedo(g_LayerColorB1), layerBWeight);
      lc2 = mix(lc2, DecodeB3Albedo(g_LayerColorB2), layerBWeight);
    }
    layerCol = (g_OnlyLayer1 < 0.5) ? mix(lc1, lc2, hairBlend) : lc1;
  }
  vec3 base = (uB3HairVariant == 21 || uB3HairVariant == 22 || uHasTexAlbedo == 0)
    ? layerCol
    : DecodeB3Albedo(sAlbedo.rgb) * layerCol;
  if (uB3HairVariant == 40 && g_bAlbedoOverWrite > 0.5) {
    base = layerCol;
  }
  if (uB3HairVariant == 40 && uHasTexMask == 1) {
    vec4 sAlb2 = texture2D(uTexMask, uv0 * g_Tile2);
    vec3 layer2Col = DecodeB3Albedo(sAlb2.rgb) * DecodeB3Albedo(g_LayerColor2);
    float layer2Weight = (uHasTexNormal == 1) ? clamp(1.0 - sNorm1Lm.a, 0.0, 1.0) : 0.0;
    base = mix(base, layer2Col, layer2Weight);
  }

  vec3 V = normalize(-vEyeW);
  vec3 priSpecTint = g_PriSpecularColor;
  vec3 secSpecTint = g_SecSpecularColor;

  // Hair22 / FvRV Iridescence ramp along view-normal film coordinate (§4.4 lines 2963-2980)
  if (uB3HairVariant == 22) {
    float ndotv = saturate(dot(Nw, V));
    float viewTerm = 1.0 - ndotv;
    float filmV = (viewTerm * g_IridescenceCoord.x + vUv.y * g_IridescenceCoord.y)
                  * g_IridescenceCoord.z + g_IridescenceCoord.w;
    vec3 ramp = (uHasTexAlbedo == 1)
      ? DecodeB3Albedo(texture2D(uTexAlbedo, vec2(0.0, filmV)).rgb)
      : (0.5 + 0.5 * cos(6.28318 * (filmV + vec3(0.0, 0.33, 0.67))));
    float f = pow(ndotv, max(g_IridescenceFresnel.x, 0.5));
    f = saturate(mix(g_IridescenceFresnel.y, g_IridescenceFresnel.z, f));
    priSpecTint = mix(priSpecTint, ramp, saturate(f * g_IridescenceStr.y));
    secSpecTint = mix(secSpecTint, ramp, saturate(f * g_IridescenceStr.z));
    base += ramp * (f * g_IridescenceStr.x);
  }

  // NormalMap1.b is AO (sampled at lightmap UV attrUv1 on Hair20); NormalMap2.b is the strand specular mask (at UV0 on Hair20).
  // Many Hair20 materials store g_AmbientOcclusionMin == g_AmbientOcclusionMax == 1.0 (or 0.5) while still baking head/scalp AO in g_NormalMap.b;
  // when min >= max, apply sNorm1Lm.b directly so the lightmap AO is always visible.
  float ao = 1.0;
  if (uHasTexNormal == 1 && uB3HairVariant != 40) {
    float rawAo = sNorm1Lm.b;
    bool aoFlat = !(g_AmbientOcclusionMinMax.y > g_AmbientOcclusionMinMax.x + 1e-3);
    if (!aoFlat || uB3HairVariant >= 30 || B3H_AO_FORCE_VISIBLE > 0.5) {
      if (!aoFlat) {
        ao = clamp(mix(g_AmbientOcclusionMinMax.x, g_AmbientOcclusionMinMax.y, rawAo), 0.02, 1.0);
      } else {
        ao = clamp(rawAo * max(g_AmbientOcclusionMinMax.y, 0.5), 0.02, 1.0);
      }
    }
  }
  // game: baked AO (g_NormalMap.b remapped by g_AmbientOcclusionMin/Max) * lerp(g_OcclusionMaskMin, g_OcclusionMaskMax, pink.b)
  if (uB3HairVariant < 30 && uHasTexNormal2 == 1 && g_OcclusionMaskMinMax.y > g_OcclusionMaskMinMax.x + 1e-4) {
    ao *= mix(g_OcclusionMaskMinMax.x, g_OcclusionMaskMinMax.y, pinkB);
  }
  float specMask = 1.0;
  if (uB3HairVariant == 40) {
    specMask = (uHasTexNormal2 == 1 && g_Layer1_UseSpecularMask > 0.5) ? clamp(sNorm2.a, 0.1, 1.0) : 1.0;
  } else if (uHasTexNormal2 == 1) {
    specMask = clamp(mix(g_SpecularMaskMinMax.x, g_SpecularMaskMinMax.y, pinkB), 0.1, 1.0);
  } else if (uHasTexNormal == 1) {
    specMask = clamp(mix(g_SpecularMaskMinMax.x, g_SpecularMaskMinMax.y, sNorm1.b), 0.1, 1.0);
  }

  // Strand diffuse. Game (Hair20/21/22): layer * DiffuseIntensity * lerp(1, sat(N.L), WrapDiffuseStrength), both lerped A -> B by normal alpha on C variants.
  float diffI = g_DiffuseIntensity;
  float wrapAmt = g_WrapDiffuseStrength;
  if (useLayerB) {
    diffI = mix(diffI, g_DiffuseIntensityB, layerBWeight);
    wrapAmt = mix(wrapAmt, g_WrapDiffuseStrengthB, layerBWeight);
  }
  float wrap = clamp(wrapAmt, 0.0, 1.0);
  float keyWrap;
  float fillWrap;
  float diffGain;
  if (uB3HairVariant < 30) {
    keyWrap = mix(1.0, saturate(dot(Nw, STUDIO_KEY_TO_LIGHT)), wrap);
    fillWrap = mix(1.0, saturate(dot(Nw, STUDIO_FILL_TO_LIGHT)), wrap);
    diffGain = diffI;
  } else {
    keyWrap = saturate((dot(Nw, STUDIO_KEY_TO_LIGHT) + wrap) / (1.0 + wrap));
    fillWrap = saturate((dot(Nw, STUDIO_FILL_TO_LIGHT) + wrap) / (1.0 + wrap));
    diffGain = max(g_DiffuseIntensity, 0.35);
  }
  vec3 diff = base * diffGain * (keyWrap * STUDIO_KEY_INTENSITY + fillWrap * STUDIO_FILL_INTENSITY);

  // Dual specular lobes (§2.3 lines 2775-2777)
  vec3 priF0 = layerCol * priSpecTint;
  vec3 secF0 = DecodeB3Albedo(g_LayerColor2) * secSpecTint;
  float rPri = clamp(g_PriRoughness, 0.05, 1.0);
  float rSec = clamp(g_SecRoughness, 0.05, 1.0);

  // Back-lit strand transmission rim (also attenuates Hair20 vR iridescence)
  float backLit = pow(saturate(1.0 - saturate(dot(Nw, V))), 3.0) * clamp(g_TransStrength, 0.0, 2.0);
  if (uB3HairVariant < 30 && g_FresnelMax > g_FresnelMin + 1e-4) {
    // game: smoothstep(sat((lerp(Fraction, 1, (1 - N.V)^Exp) - Min) / (Max - Min))) * g_TransStrength
    float fr = pow(max(1.0 - saturate(dot(Nw, V)), 1e-4), g_FresnelExp);
    fr = mix(g_FresnelFraction, 1.0, fr);
    float tr = saturate((fr - g_FresnelMin) / (g_FresnelMax - g_FresnelMin));
    backLit = tr * tr * (3.0 - 2.0 * tr) * clamp(g_TransStrength, 0.0, 2.0);
  }

  vec3 dualSpec;
  if (useStrandModel) {
    float rawMask = mix(g_SpecularMaskMinMax.x, g_SpecularMaskMinMax.y, pinkB);
    float shapeMix = clamp(rawMask * g_SpecularPower, 0.0, 3.0);
    float pull = B3H_ROUGH_PULL * 0.4;
    float priEff = max(mix(g_PriRoughness, 0.01, pull), 0.01);
    float secEff = max(mix(g_SecRoughness, 0.01, pull), 0.01);
    vec3 NgW = normalize(vNrmW);

    // spec colour: the same layer colour as the diffuse, F0 = mix(0.03, layer * spec, metallic)
    vec3 layerTint = (uB3HairVariant == 21 || uB3HairVariant == 22 || uHasTexAlbedo == 0)
      ? vec3(1.0) : DecodeB3Albedo(sAlbedo.rgb);
    vec3 layerBlend = layerTint * layerCol;
    float met = clamp(g_Metallic, 0.0, 1.0);
    vec3 priF0s = mix(vec3(0.03), layerBlend * priSpecTint, met);
    vec3 secF0s = mix(vec3(0.03), layerBlend * secSpecTint, met);

    // Gate Hair20 vR iridescence: untuned brows/lashes keep stock Coeff3=0.145 & Phase=(0.5, 1.0, 1.5)
    bool enableHair20Irid = (uB3HairVariant == 20)
      && (g_IridescenceStrength > 0.01)
      && (abs(g_IridescenceCoefficient3 - 0.145) > 0.01 || abs(g_IridescencePhase.z - 1.5) > 0.01);

    float specGain = g_SpecularPower * 0.0795775 * B3H_LIGHT_RADIANCE;
    dualSpec = specGain * (
      EvaluateB3HairStrandLobes(hairW, Nw, NgW, V, STUDIO_KEY_TO_LIGHT,
        priF0s, priEff, g_PriAlpha, secF0s, secEff, g_SecAlpha, g_HairOffset, shapeMix, g_ShadowSpecularIntensity,
        backLit, enableHair20Irid) +
      B3H_FILL_WEIGHT * EvaluateB3HairStrandLobes(hairW, Nw, NgW, V, STUDIO_FILL_TO_LIGHT,
        priF0s, priEff, g_PriAlpha, secF0s, secEff, g_SecAlpha, g_HairOffset, shapeMix, g_ShadowSpecularIntensity,
        backLit, enableHair20Irid));
  } else {
    dualSpec =
      EvaluateB3HairDualLobes(Nw, Tstrand, V, STUDIO_KEY_TO_LIGHT, priF0, rPri, g_PriAlpha, secF0, rSec, g_SecAlpha, g_SpecularPower) +
      EvaluateB3HairDualLobes(Nw, Tstrand, V, STUDIO_FILL_TO_LIGHT, priF0, rPri, g_PriAlpha * 0.5, secF0, rSec, g_SecAlpha * 0.5, g_SpecularPower);
  }

  vec3 F0 = mix(vec3(0.03), base, clamp(g_Metallic, 0.0, 1.0));
  vec3 ibl = EvaluateB3Ibl(Nw, V, base * (1.0 - g_Metallic * 0.5), rPri * rSec, F0, 1.0, specMask, uTexEnv, uTexIrradiance);
  vec3 transTerm = base * backLit * 0.35;

  // baked AO darkens the ambient/IBL term (the game doesn't apply it to direct light); B3H_AO_ON_DIRECT blends it back in
  float aoDirect = (uB3HairVariant < 30) ? mix(1.0, ao, B3H_AO_ON_DIRECT) : ao;
  vec3 colour = (diff + dualSpec * specMask + transTerm) * aoDirect + ibl * ao;
  gl_FragColor = FinishB3Fragment(colour, hairAlpha, Nw, Nv, vPosV);
}
`;

// ============================================================================
// 3b. FBR00 — Two-layer Fabric (`PS_FP_Fbr00_*`), ported from the game's transpiled pixel shaders.
//   samplers (tN = material slot N): t0 g_AlbedoMap1 (uTexAlbedo), t1 g_NormalMap1 (uTexNormal2, layer-1 tiled normal, .a spec mask),
//   t2 g_AlbedoMap2 (uTexMask, layer-2 colour, never tiled), t3 g_NormalMap2 (uTexNormal3, layer-2 normal, .a spec mask),
//   t6 g_NormalMap3 (uTexNormal, base normal, .a = layer mask). No baked AO is read by the game.
//   UV sets: UV0 = vUv.xy, UV1 = lightmap/second UV. g_Layer1_TilingMode (2 or 3 on every real material):
//     normal1: mode 0 -> UV0, modes 1-3 -> UV1 * Layer1_Tiling;  albedo1: UV1 * Layer1_Tiling only in mode 1, else UV0;
//     normal2: UV1 * Layer1_Tiling only in mode 3, else UV0;  albedo2 / base normal: UV0.   Layer2_Tiling* are never read.
//   layer mask w = sat((baseNormal.a - g_Layer1_THold) / g_THoldRange); roughness/metallic/diffuse/sheen/spec mask/normal all lerp layer1 -> layer2 by w.
//   normals: base normal (xy * lerp(DefaultNormalRate)) first, then lerp(n1, n2, w) applied in a frame built around the base-perturbed normal.
//   lighting: GGX (alpha = r^2) * Schlick * Smith-joint, Frostbite-style roughness diffuse, per light sat(diffuse + 0.5 * spec * mask).
// Anything the host doesn't feed reads as 0 and falls back to sensible defaults (see `fed` below).
// ============================================================================
const B3_FABRIC_FRAGMENT_SHADER = /* glsl */ `
${B3_PBR_COMMON_GLSL}

uniform sampler2D uTexAlbedo;
uniform sampler2D uTexMask;
uniform sampler2D uTexNormal;
uniform sampler2D uTexNormal2;
uniform sampler2D uTexNormal3;
uniform samplerCube uTexEnv;
uniform samplerCube uTexIrradiance;

uniform int uHasTexAlbedo;
uniform int uHasTexMask;
uniform int uHasTexNormal;
uniform int uHasTexNormal2;
uniform int uHasTexNormal3;

uniform float g_1BitMask;                 // selects the cutout variant: alpha comes from g_NormalMap3.a
uniform float g_Layer2_Use;               // selects the dual-layer variant
uniform float g_Layer1_TilingMode;
uniform vec2  g_Layer1_Tiling;
uniform vec2  g_DetailNormalTile;         // fallback alias for g_Layer1_Tiling
uniform vec3  g_Layer1_BaseColor;
uniform vec3  g_LayerColor1;              // fallback alias for g_Layer1_BaseColor
uniform float g_Layer1_UseAlbedo;
uniform float g_Layer1_UseNormalMap;
uniform float g_Layer1_UseSpecularMask;
uniform float g_Layer2_UseNormalMap;
uniform float g_Layer2_UseSpecularMask;
uniform float g_Layer1_THold;
uniform float g_THoldRange;
uniform float g_Layer1_Roughness;
uniform float g_Layer2_Roughness;
uniform float g_Layer1_Metallic;
uniform float g_Layer2_Metallic;
uniform float g_Layer1_DiffuseWeight;
uniform float g_Layer2_DiffuseWeight;
uniform float g_Layer1_DefaultNormalRate;
uniform float g_Layer2_DefaultNormalRate;
uniform vec3  g_Layer1_SheenColorMul;
uniform vec3  g_Layer2_SheenColorMul;
uniform vec3  g_Layer1_SheenColorAdd;
uniform vec3  g_Layer2_SheenColorAdd;
uniform float g_Layer1_SheenIntencity;    // (sic)
uniform float g_Layer2_SheenIntencity;
uniform float g_Layer1_SheenOpacity;
uniform float g_Layer2_SheenOpacity;
uniform float g_FresneRate;               // (sic)
uniform float g_FresnelIntensity;
uniform float g_FresnelExp;
uniform float g_FresnelFraction;
uniform float g_FresnelMin;
uniform float g_FresnelMax;

const float B3F_MIN_ALPHA      = 0.002; // c1[6] in the game, unknown; floor on alpha = r^2 (real roughness is 0.55-1.0 so it never matters)
const float B3F_SPEC_GAIN      = 1.0;   // overall multiplier on the direct spec
const float B3F_IBL_SPEC_GAIN  = 0.85;  // overall multiplier on the IBL spec
const float B3F_ENV_BLUR       = 1.0;   // weight scale for blurring the env cube toward the irradiance cube by roughness
const float B3F_SHEEN_STRENGTH = 1.0;   // scales the sheen tint (the exact in-game blend is only approximated)
const float B3F_FORCE_DUAL     = 0.0;   // 1 = always dual-layer when AlbedoMap2 / NormalMap3 exist, ignoring g_Layer2_Use

// base normal first (xy scaled by DefaultNormalRate), then the tiled layer normal in a frame built around it
vec3 B3FabricNormal(vec3 N0, vec3 T0, vec3 nBaseTS, float rate, vec3 nLayerTS) {
  vec3 nb = normalize(vec3(nBaseTS.xy * rate, nBaseTS.z));
  vec3 B0 = normalize(cross(N0, T0));
  vec3 Nb = normalize(nb.x * T0 + nb.y * B0 + nb.z * N0);
  vec3 Bb = normalize(cross(Nb, T0));
  vec3 Tb = normalize(cross(Bb, Nb));
  return normalize(nLayerTS.x * Tb + nLayerTS.y * Bb + nLayerTS.z * Nb);
}

vec3 B3FabricEnvBrdf(vec3 F0, float a, float NV) {
  vec4 r = a * vec4(-1.0, -0.0275, -0.572, 0.022) + vec4(1.0, 0.0425, 1.04, -0.04);
  float a004 = min(r.x * r.x, exp2(-9.28 * NV)) * r.x + r.y;
  vec2 ab = vec2(-1.04, 1.04) * a004 + r.zw;
  return F0 * ab.x + ab.y;
}

// one light in the rig's units: gain * min(diffuse / PI + 0.5 * spec * mask, 1), as the game saturates per light
vec3 B3FabricLight(vec3 N, vec3 V, vec3 L, vec3 diffColour, vec3 F0, float r, float specMask, float gain) {
  float NL = saturate(dot(N, L));
  if (NL <= 0.0) return vec3(0.0);
  float NV = max(saturate(dot(N, V)), 1e-3);
  float NLm = max(NL, 1e-3);
  vec3 H = normalize(L + V);
  float NH = saturate(dot(N, H));
  float VH = saturate(dot(V, H));

  float al = max(r * r, B3F_MIN_ALPHA);
  float al2 = al * al;
  float dd = NH * NH * (al2 - 1.0) + 1.0;
  float D = al2 / (dd * dd);
  float vis = 1.0 / (NLm * (al + NV * (1.0 - al)) + NV * (al + NLm * (1.0 - al)));
  float fc = 1.0 - VH;
  float fc2 = fc * fc;
  vec3 F = F0 + (vec3(1.0) - F0) * (fc2 * fc2 * fc);
  vec3 spec = F * (D * vis * 0.5 * NL) * specMask * B3F_SPEC_GAIN;

  // Frostbite-style roughness diffuse: fd90 = 0.5 r + 2 r (L.H)^2
  float fd90 = 0.5 * r + 2.0 * r * VH * VH;
  float cl = 1.0 - NL;
  float cv = 1.0 - NV;
  float fdL = 1.0 + (fd90 - 1.0) * (cl * cl * cl * cl * cl);
  float fdV = 1.0 + (fd90 - 1.0) * (cv * cv * cv * cv * cv);
  vec3 diffuse = diffColour * (fdL * fdV * NL);

  return gain * min(diffuse / B3_PI + spec, vec3(1.0));
}

void main() {
  vec2 uv0 = vUv.xy;
  vec2 uv1 = ResolveLightMapUv(vUv, vUv2);
  vec3 V = normalize(-vEyeW);
  vec3 N0w = normalize(vNrmW);
  vec3 N0v = normalize(vNrmV);
  vec3 T0w = SafeOrthoTangentB3(N0w, vTanW);
  vec3 T0v = SafeOrthoTangentB3(N0v, vTanV);

  // ---- parameters, with defaults when the host doesn't feed them ----
  bool fed = (g_Layer1_Roughness + g_Layer2_Roughness) > 0.001;
  float r1 = fed ? g_Layer1_Roughness : 0.8;
  float r2 = fed ? g_Layer2_Roughness : 0.9;
  float m1 = fed ? g_Layer1_Metallic : 0.0;
  float m2 = fed ? g_Layer2_Metallic : 0.0;
  float dw1 = fed ? g_Layer1_DiffuseWeight : 1.0;
  float dw2 = fed ? g_Layer2_DiffuseWeight : 0.8;
  float nr1 = fed ? g_Layer1_DefaultNormalRate : 1.0;
  float nr2 = fed ? g_Layer2_DefaultNormalRate : 1.0;
  float tHold = fed ? g_Layer1_THold : 0.5;
  float tRange = fed ? max(g_THoldRange, 1e-3) : 0.05;
  int tilingMode = fed ? int(floor(g_Layer1_TilingMode + 0.5)) : 2;
  bool useAlbedo1 = fed ? (g_Layer1_UseAlbedo > 0.5) : true;
  bool useNormal1 = fed ? (g_Layer1_UseNormalMap > 0.5) : true;
  bool useNormal2 = fed ? (g_Layer2_UseNormalMap > 0.5) : false;
  bool useMask1 = fed ? (g_Layer1_UseSpecularMask > 0.5) : true;
  bool useMask2 = fed ? (g_Layer2_UseSpecularMask > 0.5) : false;
  bool cutout = g_1BitMask > 0.5;
  bool dual = !cutout && (g_Layer2_Use > 0.5 || B3F_FORCE_DUAL > 0.5 || !fed) && uHasTexMask == 1 && uHasTexNormal == 1;

  vec2 tile = (g_Layer1_Tiling.x > 0.0 && g_Layer1_Tiling.y > 0.0) ? g_Layer1_Tiling
            : ((g_DetailNormalTile.x > 0.0 && g_DetailNormalTile.y > 0.0) ? g_DetailNormalTile : vec2(1.0));
  vec2 uvTiled = uv1 * tile;
  vec2 uvNormal1 = (tilingMode >= 1) ? uvTiled : uv0;
  vec2 uvAlbedo1 = (tilingMode == 1) ? uvTiled : uv0;
  vec2 uvNormal2 = (tilingMode == 3) ? uvTiled : uv0;

  // ---- base normal (+ layer mask in .a) ----
  vec4 sBase = (uHasTexNormal == 1) ? texture2D(uTexNormal, uv0) : vec4(0.5, 0.5, 1.0, 0.0);
  float w = dual ? saturate((sBase.a - tHold) / tRange) : 0.0;
  float alpha = 1.0;
  if (cutout) {
    alpha = (uHasTexNormal == 1) ? sBase.a : 1.0;
    if (uDebugMode == 0 && alpha <= 0.3333) { discard; }
  }

  // ---- layer normals + spec masks ----
  vec4 sN1 = (uHasTexNormal2 == 1) ? texture2D(uTexNormal2, uvNormal1) : vec4(0.5, 0.5, 1.0, 1.0);
  vec4 sN2 = (dual && uHasTexNormal3 == 1) ? texture2D(uTexNormal3, uvNormal2) : vec4(0.5, 0.5, 1.0, 1.0);
  vec3 n1 = (useNormal1 && uHasTexNormal2 == 1) ? DecodeB3NormalTS(sN1) : vec3(0.0, 0.0, 1.0);
  vec3 n2 = (useNormal2 && dual && uHasTexNormal3 == 1) ? DecodeB3NormalTS(sN2) : vec3(0.0, 0.0, 1.0);
  vec3 nLayer = normalize(mix(n1, n2, w));
  float mask1 = (useMask1 && uHasTexNormal2 == 1) ? sN1.a : 1.0;
  float mask2 = (useMask2 && dual && uHasTexNormal3 == 1) ? sN2.a : 1.0;
  float specMask = mix(mask1, mask2, w);

  // z is reconstructed from xy (not read from the blue channel); the base normal's blue is NOT used as AO
  vec3 nBase = (uHasTexNormal == 1) ? DecodeB3NormalTS(sBase) : vec3(0.0, 0.0, 1.0);
  float normalRate = mix(nr1, nr2, w);
  vec3 Nw = B3FabricNormal(N0w, T0w, nBase, normalRate, nLayer);
  vec3 Nv = B3FabricNormal(N0v, T0v, nBase, normalRate, nLayer);

  // ---- layer colours ----
  vec3 baseColor1 = (dot(g_Layer1_BaseColor, g_Layer1_BaseColor) > 1e-6) ? g_Layer1_BaseColor : g_LayerColor1;
  if (dot(baseColor1, baseColor1) < 1e-6) { baseColor1 = vec3(1.0); }
  vec3 colour1 = DecodeB3Albedo(baseColor1);
  if (useAlbedo1 && uHasTexAlbedo == 1) {
    colour1 *= DecodeB3Albedo(texture2D(uTexAlbedo, uvAlbedo1).rgb);
  }
  vec3 colour2 = colour1;
  if (dual) {
    colour2 = DecodeB3Albedo(texture2D(uTexMask, uv0).rgb);   // raw AlbedoMap2: no layer-2 base colour, never tiled
  }
  vec3 colour = mix(colour1, colour2, w);

  float rough = clamp(mix(r1, r2, w), 0.0, 1.0);
  float metal = clamp(mix(m1, m2, w), 0.0, 1.0);
  float diffW = mix(dw1, dw2, w);
  float NV = saturate(dot(Nw, V));

  // ---- Fresnel-weighted sheen (structure decoded from the game; the final blend is approximated) ----
  float tF = 1.0 - NV;
  float fresA = tF * tF * (3.0 - 2.0 * tF);
  float fresMask = fresA;
  if (g_FresnelMax > g_FresnelMin + 1e-4) {
    float fr = pow(max(tF, 1e-4), g_FresnelExp);
    fr = mix(g_FresnelFraction, 1.0, fr);
    float tr = saturate((fr - g_FresnelMin) / (g_FresnelMax - g_FresnelMin));
    float fresB = tr * tr * (3.0 - 2.0 * tr);
    fresMask = clamp(mix(fresA, fresB * g_FresnelIntensity, g_FresneRate), 0.0, 1.0);
  }
  vec3 sheenMul = fed ? mix(g_Layer1_SheenColorMul, g_Layer2_SheenColorMul, w) : vec3(1.0);
  vec3 sheenAdd = fed ? mix(g_Layer1_SheenColorAdd, g_Layer2_SheenColorAdd, w) : vec3(0.0);
  float sheenInt = fed ? mix(g_Layer1_SheenIntencity, g_Layer2_SheenIntencity, w) : 1.0;
  float sheenOp = fed ? mix(g_Layer1_SheenOpacity, g_Layer2_SheenOpacity, w) : 0.5;
  vec3 sheenCol = clamp(specMask * (sheenMul * colour + sheenAdd) * sheenInt, 0.0, 1.0);
  float sheenAmt = clamp(fresMask * sheenOp * B3F_SHEEN_STRENGTH, 0.0, 1.0);
  vec3 colourDiff = mix(colour, colour * sheenCol, sheenAmt) * ((1.0 - metal) * diffW);

  // ---- lights ----
  vec3 F0 = mix(vec3(0.03), colour, metal);
  float gKey = B3_PI * STUDIO_KEY_INTENSITY * 1.45;
  float gFill = B3_PI * STUDIO_FILL_INTENSITY * 0.95;
  vec3 direct = B3FabricLight(Nw, V, STUDIO_KEY_TO_LIGHT, colourDiff, F0, rough, specMask, gKey)
              + B3FabricLight(Nw, V, STUDIO_FILL_TO_LIGHT, colourDiff, F0, rough, specMask, gFill);

  // ---- IBL: diffuse as before (specScale 0), spec = env * BRDF-LUT fit * spec mask ----
  vec3 iblDiffuse = EvaluateB3Ibl(Nw, V, colourDiff, rough, F0, 1.0, 0.0, uTexEnv, uTexIrradiance);
  vec3 R = reflect(-V, Nw);
  vec3 envSharp = texCUBE_game(uTexEnv, R).rgb;
  vec3 envSoft = texCUBE_game(uTexIrradiance, R).rgb;
  vec3 envSpec = mix(envSharp, envSoft, saturate(rough * B3F_ENV_BLUR));
  vec3 iblSpec = envSpec * B3FabricEnvBrdf(F0, rough, NV) * specMask * B3F_IBL_SPEC_GAIN;

  vec3 colourOut = direct + iblDiffuse + iblSpec;
  gl_FragColor = FinishB3Fragment(colourOut, alpha, Nw, Nv, vPosV);
}
`;

// ============================================================================
// 4. SKN20 / SKN21 / SKN25 / SKN30 / SKN31 / HMSKN* — Character Skin with SSS LUT
// (`SHADER_ALGORITHMS_DEEP.md` §06b lines 8107-8704)
//   uB3SkinVariant:
//     20 = SKN20 / HMSKN* (AO in normal.b, roughness = max(normal.a^2, 0.001), toon specular shadow lerp(spec, 1.0, mask))
//     21 = SKN21 (simplified skin: g_SSSColor tint, no spec-shadow / AO)
//     25 = SKN25 (SKN20 with explicit AO map on slot 15 / uTexIndex)
//     30 = SKN30 / SKN31 (two-layer Fresnel shell with constant-depth parallax g_ParallaxDepth, out.a = albedo.a)
// ============================================================================
const B3_SKIN_FRAGMENT_SHADER = /* glsl */ `
${B3_PBR_COMMON_GLSL}

uniform sampler2D uTexAlbedo;
uniform sampler2D uTexNormal;
uniform sampler2D uTexNormal2;
uniform sampler2D uTexMask;
uniform sampler2D uTexIndex;
uniform samplerCube uTexEnv;
uniform samplerCube uTexIrradiance;

uniform int uB3SkinVariant;
uniform int uHasTexAlbedo;
uniform int uHasTexNormal;
uniform int uHasTexNormal2;
uniform int uHasTexMask;
uniform int uHasTexIndex;

uniform float g_UseDetailNormalMap;
uniform vec2 g_DetailNormalTile;
uniform float g_DetailNormalIntensity;
uniform float g_CurvatureScale;
uniform float g_Metallic;
uniform float g_SSSBias;
uniform float g_BendNormalRate;
uniform vec3 g_SSSColor;
uniform float g_SpecularShadowRate;
uniform vec2 g_SpecularShadowClampMinMax;
uniform float g_RamePower;
uniform float g_RameLightStrength;
uniform vec2 g_AmbientOcclusionMinMax;

// SKN30 / SKN31 two-layer Fresnel shell & constant-depth parallax uniforms (§4.2-4.3 lines 8464-8525)
uniform float g_InnerRoughnessAdd;
uniform float g_OuterRoughnessAdd;
uniform float g_InnerAlbedoIntensity;
uniform float g_ParallaxDepth;
uniform vec2 g_ParallaxUVIntensity;
uniform vec3 g_AlbedoFresnelParams; // x = Intensity, y = Exp, z = BaseFrac
uniform vec2 g_OuterFresnelMinMax;

// ============================================================================
// Skin specular, ported from the game's PS_FP_Skin20 / Skin21 / Skin25 (Maxwell -> GLSL transpile).
//   roughness a = g_NormalMap.a (Skin20/25) or g_AlbedoMap2.g (Skin21); GGX alpha = a^2 (NOT a^4)
//   direct: F_Schlick(F0 = 0.03) * D_GGX(alpha) * Vis_SmithJointApprox(alpha) * N.L * 0.5, times the albedo alpha (specular mask)
//   per light the game saturates diffuse + spec before the light colour, so highlights can't blow out
//   IBL: env cube at a mip chosen by a, times the BRDF LUT (F0 * A + B), times the same albedo-alpha mask
// ============================================================================
const float B3S_GAME_NORMAL_DECODE = 1.0;  // 1 = game's z = 1 - |xy| (then normalise), 0 = sqrt(1 - |xy|^2)
const float B3S_MIN_ALPHA          = 0.002; // c1[5] in the game, unknown; floor on alpha = a^2
const float B3S_SPEC_GAIN          = 1.0;   // overall multiplier on the direct spec
const float B3S_IBL_SPEC_GAIN      = 0.85;  // overall multiplier on the IBL spec (was specScale 0.85)
const float B3S_ENV_BLUR           = 1.0;   // weight scale for blurring the env cube toward the irradiance cube by roughness

vec3 B3SkinDecodeNormal(vec4 s) {
  vec2 xy = s.rg * 2.0 - 1.0;
  float len = sqrt(saturate(dot(xy, xy)));
  return normalize(vec3(xy, 1.0 - len));
}

// F * D * Vis * N.L * 0.5, all in the game's units (no 1/PI on D, no 0.5 on Vis, the 0.5 is applied here)
vec3 B3SkinSpecLobe(vec3 N, vec3 V, vec3 L, float a, vec3 F0) {
  float NL = saturate(dot(N, L));
  if (NL <= 0.0) return vec3(0.0);
  float NV = max(saturate(dot(N, V)), 1e-3);
  float NLm = max(NL, 1e-3);
  vec3 H = normalize(L + V);
  float NH = saturate(dot(N, H));
  float VH = saturate(dot(V, H));
  float al = max(a * a, B3S_MIN_ALPHA);
  float al2 = al * al;
  float dd = NH * NH * (al2 - 1.0) + 1.0;
  float D = al2 / (dd * dd);
  float vis = 1.0 / (NLm * (al + NV * (1.0 - al)) + NV * (al + NLm * (1.0 - al)));
  float fc = 1.0 - VH;
  float fc2 = fc * fc;
  vec3 F = F0 + (vec3(1.0) - F0) * (fc2 * fc2 * fc);
  return F * (D * vis * 0.5 * NL);
}

// Karis' mobile analytic fit of the split-sum BRDF LUT (the game samples a 2D LUT at (N.V, 1 - a))
vec3 B3SkinEnvBrdf(vec3 F0, float a, float NV) {
  vec4 r = a * vec4(-1.0, -0.0275, -0.572, 0.022) + vec4(1.0, 0.0425, 1.04, -0.04);
  float a004 = min(r.x * r.x, exp2(-9.28 * NV)) * r.x + r.y;
  vec2 ab = vec2(-1.04, 1.04) * a004 + r.zw;
  return F0 * ab.x + ab.y;
}

// the game saturates (diffuse + spec) per light before multiplying the light colour; gain converts to this rig's units
vec3 B3SkinLightOut(vec3 diffuseU, vec3 specGame, float gain) {
  float g = max(gain, 1e-3);
  return g * min(diffuseU / g + specGame, vec3(1.0));
}

void main() {
  vec2 uv = vUv.xy;
  vec3 V = normalize(-vEyeW);
  vec3 NwGeo = normalize(vNrmW);
  vec3 NvGeo = normalize(vNrmV);

  // Slot 2 (or Slot 1 in SKN30/31) g_AlbedoMap2: RGB = layer2 colour + spec mask, A = parallax mask in SKN30/31
  vec4 sLayer2 = (uHasTexMask == 1) ? texture2D(uTexMask, uv) : vec4(0.95, 0.68, 0.62, 1.0);

  // SKN30 / SKN31 constant-depth parallax offset (§7 lines 8680-8683)
  vec2 uvParallax = uv;
  if (uB3SkinVariant == 30) {
    vec3 T = SafeOrthoTangentB3(NwGeo, vTanW);
    vec3 B = cross(NwGeo, T);
    vec2 vT = vec2(dot(V, T), dot(V, B));
    uvParallax = uv + vT * g_ParallaxDepth * g_ParallaxUVIntensity * sLayer2.a;
  }

  // Slot 0 g_AlbedoMap: RGB = skin base, A = specular-shadow driver (SKN20/25) or output alpha (SKN30)
  vec4 sAlbedo = (uHasTexAlbedo == 1) ? texture2D(uTexAlbedo, uvParallax) : vec4(0.92, 0.78, 0.72, 1.0);
  // Normal map: RG = normal (Z reconstructed), B = AO (SKN20), A = roughness (SKN20/25/30)
  vec4 sNorm = (uHasTexNormal == 1) ? texture2D(uTexNormal, uv) : vec4(0.5, 0.5, 0.90, 0.48);

  vec3 Nw = NwGeo;
  vec3 Nv = NvGeo;
  if (uHasTexNormal == 1) {
    vec3 nTS = (B3S_GAME_NORMAL_DECODE > 0.5 && uB3SkinVariant != 30) ? B3SkinDecodeNormal(sNorm) : DecodeB3NormalTS(sNorm);
    if (uUseSecondaryNormals == 1 && uHasTexNormal2 == 1) {
      if (uB3SkinVariant == 30) {
        vec4 sNormIn = texture2D(uTexNormal2, uvParallax);
        vec3 nInTS = DecodeB3NormalTS(sNormIn);
        nTS = normalize(mix(nTS, nInTS, 0.45));
      } else if (g_UseDetailNormalMap > 0.5) {
        // SKN20 / SKN25 D-variant pore detail normal tiled by g_DetailNormalTile (from Variables)
        vec4 sDet = texture2D(uTexNormal2, uv * g_DetailNormalTile);
        vec3 nDetTS = DecodeB3NormalTS(sDet);
        float detStr = clamp(g_DetailNormalIntensity, 0.15, 1.2);
        nTS = normalize(vec3(nTS.xy + nDetTS.xy * detStr, nTS.z));
      }
    }
    Nw = PerturbB3Normal(nTS, 1.0, vTanW, NwGeo);
    Nv = PerturbB3Normal(nTS, 1.0, vTanV, NvGeo);
  }

  // Bent normal for subsurface wrap (§1.3 lines 8180-8183)
  vec3 Nb = normalize(mix(Nw, NwGeo, clamp(g_BendNormalRate, 0.0, 1.0)));

  vec3 base = DecodeB3Albedo(sAlbedo.rgb);
  float metal = (uB3SkinVariant == 21) ? 0.0 : clamp(g_Metallic, 0.0, 1.0);

  // Roughness & AO per skin family variant (§1.2, §2.2, §3.2, §4.2)
  float rough = 0.45;
  float ao = 1.0;
  if (uB3SkinVariant == 21) {
    rough = 0.48;
    ao = 1.0;
  } else if (uB3SkinVariant == 30) {
    float rBase = (uHasTexNormal == 1) ? sNorm.a : 0.42;
    rough = clamp(rBase + g_InnerRoughnessAdd + g_OuterRoughnessAdd, 0.06, 1.0);
    float ndv = saturate(dot(Nw, V));
    float fBase = g_AlbedoFresnelParams.z;
    float fb = saturate(fBase + (1.0 - fBase) * pow(1.0 - ndv, max(g_AlbedoFresnelParams.y, 1.0))) * g_AlbedoFresnelParams.x;
    float outerFres = smoothstep(g_OuterFresnelMinMax.x, g_OuterFresnelMinMax.y, 1.0 - ndv);
    vec3 innerCol = base * max(g_InnerAlbedoIntensity, 0.5);
    base = mix(innerCol, base, clamp(1.0 - max(fb, outerFres) * 0.35, 0.25, 1.0));
  } else {
    // SKN20 / SKN25 / HMSKN*: rough = max(tex_normal.a^2, 0.001) (§1.2 line 8149)
    float rawRough = (uHasTexNormal == 1) ? max(sNorm.a * sNorm.a, 0.001) : 0.45;
    rough = clamp(rawRough, 0.08, 1.0);
    if (uB3SkinVariant == 25) {
      // SKN25: AO from slot 15 (uTexIndex.r), NOT normal.b (§3.2 line 8406)
      float aoSample = (uHasTexIndex == 1) ? texture2D(uTexIndex, uv).r : 1.0;
      ao = clamp(mix(g_AmbientOcclusionMinMax.x, g_AmbientOcclusionMinMax.y, aoSample), 0.15, 1.0);
    } else if (uHasTexNormal == 1) {
      ao = clamp(mix(g_AmbientOcclusionMinMax.x, g_AmbientOcclusionMinMax.y, sNorm.b), 0.15, 1.0);
    }
  }

  vec3 F0 = mix(vec3(0.03), base, metal);
  vec3 diffK = base * (1.0 - metal);

  // Game: the skin spec uses a = g_NormalMap.a (Skin20/25) or g_AlbedoMap2.g (Skin21) as perceptual roughness, and albedo alpha as the spec mask.
  // (Passing a^2 into EvaluateB3DirectPbr / EvaluateB3Ibl squared it twice, which is what made everything glossy.)
  bool gameSpec = (uB3SkinVariant != 30);
  float aRough = 0.5;
  if (uB3SkinVariant == 21) {
    aRough = (uHasTexMask == 1) ? sLayer2.g : 0.5;
  } else if (uHasTexNormal == 1) {
    aRough = sNorm.a;
  }
  aRough = clamp(aRough, 0.0, 1.0);
  float specMaskA = (uB3SkinVariant == 30 || uHasTexAlbedo == 0) ? 1.0 : sAlbedo.a;

  // Wrapped subsurface diffuse + SSS terminator bleed (§1.3 lines 8183-8195 & §2.2 g_SSSColor)
  float sssBias = clamp(g_SSSBias, 0.05, 0.85);
  float ndotlKey = dot(Nw, STUDIO_KEY_TO_LIGHT);
  float ndotlBent = dot(Nb, STUDIO_KEY_TO_LIGHT);
  float halfLambert = 0.5 + 0.5 * mix(ndotlKey, ndotlBent, sssBias);
  float directLam = saturate(ndotlKey);
  float terminator = smoothstep(0.0, 0.55, halfLambert) * (1.0 - directLam);
  vec3 sssTint = (uB3SkinVariant == 21)
    ? DecodeB3Albedo(g_SSSColor)
    : vec3(1.0, 0.34, 0.24) * clamp(max(g_CurvatureScale, 0.35), 0.15, 1.2);
  vec3 sssKey = diffK * (directLam + terminator * sssTint * sssBias * 1.35) * STUDIO_KEY_INTENSITY * 1.45;

  float fillLam = saturate(dot(Nb, STUDIO_FILL_TO_LIGHT) * 0.7 + 0.3);
  vec3 sssFill = diffK * fillLam * STUDIO_FILL_INTENSITY * 0.95;
  vec3 sssDiffuse = sssKey + sssFill;

  vec3 specOnly = vec3(0.0);
  vec3 specKeyG = vec3(0.0);
  vec3 specFillG = vec3(0.0);
  vec3 iblSpecG = vec3(0.0);
  if (gameSpec) {
    // F0 on the direct path is lerped twice by metallic in the game (0.03 + (albedo - 0.03) * M^2); the IBL path lerps once
    vec3 F0d = mix(vec3(0.03), base, metal * metal);
    specKeyG  = B3SkinSpecLobe(Nw, V, STUDIO_KEY_TO_LIGHT,  aRough, F0d) * specMaskA * B3S_SPEC_GAIN;
    specFillG = B3SkinSpecLobe(Nw, V, STUDIO_FILL_TO_LIGHT, aRough, F0d) * specMaskA * B3S_SPEC_GAIN;

    vec3 envSharp = texCUBE_game(uTexEnv, reflect(-V, Nw)).rgb;
    vec3 envSoft  = texCUBE_game(uTexIrradiance, reflect(-V, Nw)).rgb;
    vec3 envSpec  = mix(envSharp, envSoft, saturate(aRough * B3S_ENV_BLUR));
    iblSpecG = envSpec * B3SkinEnvBrdf(F0, aRough, saturate(dot(Nw, V))) * specMaskA * mix(0.35, 1.0, ao) * B3S_IBL_SPEC_GAIN;
  } else {
    specOnly = EvaluateB3DirectPbr(Nw, V, vec3(0.0), rough, F0);
  }

  // Optional T-permutation ramé micro-sheen (§6 line 8646)
  vec3 rameTerm = vec3(0.0);
  if (g_RameLightStrength > 0.001) {
    float ndotv = saturate(dot(Nw, V));
    float rameRim = pow(1.0 - ndotv, max(g_RamePower, 2.0));
    float rameScale = clamp(g_RameLightStrength, 0.0, 1.5);
    rameTerm = vec3(1.0, 0.94, 0.90) * rameRim * rameScale * 0.25;
  }

  vec3 colour;
  if (gameSpec) {
    // diffuse IBL exactly as before (specScale 0 returns only the diffuse part); spec is the game-style term above
    vec3 iblDiffuse = EvaluateB3Ibl(Nw, V, diffK, rough, F0, ao, 0.0, uTexEnv, uTexIrradiance);
    float gKey  = B3_PI * STUDIO_KEY_INTENSITY * 1.45;
    float gFill = B3_PI * STUDIO_FILL_INTENSITY * 0.95;
    colour = B3SkinLightOut(sssKey * ao, specKeyG, gKey) + B3SkinLightOut(sssFill * ao, specFillG, gFill)
           + iblDiffuse + iblSpecG + rameTerm;
  } else {
    vec3 ibl = EvaluateB3Ibl(Nw, V, diffK, rough, F0, ao, 0.85, uTexEnv, uTexIrradiance);
    colour = sssDiffuse * ao + specOnly + ibl + rameTerm;
  }
  float outAlpha = (uB3SkinVariant == 30) ? sAlbedo.a : 1.0;

  gl_FragColor = FinishB3Fragment(colour, outAlpha, Nw, Nv, vPosV);
}
`;

// ============================================================================
// 5. Gls14 / Gls15 / Gls01 / Gls02 / Gls03 / Gls20 / Gls22 / Gls31 — Glass & Interior Windows
// (`PBS_BAYO_HAIR_material_mapping.md` §6.10 & §6.18; `SHADER_ALGORITHMS_DEEP.md` §03a-03b)
//   uB3GlassVariant:
//     14 = Gls14 / Gls15 (window glass tinted by g_GlassColor + environment reflection + DivX x DivY pane grid;
//          optional interior projection when room texture or interior light is present)
//     3  = Gls03 (interior room projection using wallFrequencies & TileXZ)
//     2  = Gls01 / Gls02 / Gls20 / Gls22 / Gls31 (transparent Fresnel glass)
// ============================================================================
const B3_GLASS_FRAGMENT_SHADER = /* glsl */ `
${B3_PBR_COMMON_GLSL}

uniform sampler2D uTexAlbedo;
uniform sampler2D uTexMask;
uniform sampler2D uTexNormal;
uniform samplerCube uTexEnv;
uniform samplerCube uTexIrradiance;

uniform int uB3GlassVariant; // 14 = Gls14/15, 3 = Gls03, 2 = Gls01/02/20/22/31
uniform int uHasTexAlbedo;
uniform int uHasTexMask;
uniform int uHasTexNormal;

uniform vec3 g_GlassColor;
uniform vec3 g_SpaceDimensions;
uniform vec2 g_DivisionNum;
uniform float g_InteriorBlendRate;
uniform float g_inLightIntensity;
uniform float g_Roughness;
uniform float g_Metallic;

void main() {
  vec2 uv = vUv.xy;

  vec4 sAlbedo = (uHasTexAlbedo == 1) ? texture2D(uTexAlbedo, uv) : vec4(0.75, 0.88, 0.96, 0.35);
  vec4 sMask = (uHasTexMask == 1) ? texture2D(uTexMask, uv) : vec4(1.0, 0.1, 0.15, 1.0);

  vec3 Nw = normalize(vNrmW);
  vec3 Nv = normalize(vNrmV);
  if (uHasTexNormal == 1) {
    vec4 sNorm = texture2D(uTexNormal, uv);
    vec3 nTS = DecodeB3NormalTS(sNorm);
    Nw = PerturbB3Normal(nTS, 1.0, vTanW, Nw);
    Nv = PerturbB3Normal(nTS, 1.0, vTanV, Nv);
  }

  vec3 V = normalize(-vEyeW);
  float ndotv = saturate(dot(Nw, V));
  float fresnel = pow(1.0 - ndotv, 4.0);
  vec3 envRefl = texCUBE_game(uTexEnv, reflect(-V, Nw)).rgb;
  vec3 envIrrad = texCUBE_game(uTexIrradiance, Nw).rgb;

  if (uB3GlassVariant == 14 || uB3GlassVariant == 3) {
    // Gls14 / Gls15 / Gls03: Window glass tinted by g_GlassColor + env cube + DivX x DivY cell grid (§6.10)
    vec3 glassTint = DecodeB3Albedo(g_GlassColor);
    vec2 div = max(g_DivisionNum, vec2(1.0));
    vec2 cellUv = fract(uv * div);
    float paneEdge = smoothstep(0.015, 0.045, min(min(cellUv.x, 1.0 - cellUv.x), min(cellUv.y, 1.0 - cellUv.y)));

    vec3 roomScale = max(g_SpaceDimensions, vec3(0.5));
    vec3 viewRay = -V;
    vec3 localPos = fract(uv.xyy * vec3(2.0, 2.0, 1.0)) - 0.5;
    vec3 raySign = step(vec3(0.0), viewRay) * 2.0 - 1.0;
    vec3 invRay = 1.0 / (raySign * max(abs(viewRay), vec3(1e-3)));
    vec3 tBox = (0.5 * raySign - localPos) * invRay * roomScale;
    float tHit = min(tBox.x, min(tBox.y, tBox.z));
    vec3 hitP = localPos + viewRay * tHit * 0.25;

    vec2 roomUv = uv + hitP.xy * 0.35;
    vec3 interiorCol = (uHasTexAlbedo == 1)
      ? DecodeB3Albedo(texture2D(uTexAlbedo, roomUv).rgb)
      : envIrrad * 0.65;
    float inLight = clamp(g_inLightIntensity > 10.0 ? (g_inLightIntensity * 0.0005) : g_inLightIntensity, 0.35, 1.5);
    interiorCol *= (0.7 + 0.3 * paneEdge) * inLight;

    vec3 exteriorRefl = envRefl * mix(vec3(0.35), glassTint, 0.65) * (0.3 + 0.7 * fresnel);
    vec3 combined = mix(exteriorRefl + glassTint * envIrrad * 0.35, interiorCol * glassTint, clamp(g_InteriorBlendRate, 0.15, 0.85));
    gl_FragColor = FinishB3Fragment(combined, 1.0, Nw, Nv, vPosV);
    return;
  }

  // Gls01 / Gls02 / Gls20 / Gls22 / Gls31: Transparent Fresnel glass (SHADER_ALGORITHMS_DEEP.md §03a-03b)
  vec3 tint = DecodeB3Albedo(sAlbedo.rgb) * DecodeB3Albedo(g_GlassColor);
  float metal = (uHasTexMask == 1) ? clamp(sMask.r, 0.0, 1.0) : clamp(g_Metallic, 0.0, 1.0);
  float rough = (uHasTexMask == 1) ? clamp(sMask.b * 0.35, 0.03, 0.6) : clamp(g_Roughness, 0.03, 0.6);
  vec3 F0 = mix(vec3(0.04), tint, metal);

  vec3 direct = EvaluateB3DirectPbr(Nw, V, tint * (1.0 - metal), rough, F0);
  vec3 ibl = EvaluateB3Ibl(Nw, V, tint * 0.35, rough, F0, 1.0, 1.35, uTexEnv, uTexIrradiance);
  vec3 colour = direct + ibl + envRefl * fresnel * 0.55;
  float outAlpha = clamp(max(sAlbedo.a, 0.18) + fresnel * 0.55, 0.22, 0.95);

  gl_FragColor = FinishB3Fragment(colour, outAlpha, Nw, Nv, vPosV);
}
`;

// ============================================================================
// 6. HZR00 / Sand00 / Wtr* / Lava00 — Layered Ground / Terrain Shader
// (`SHADER_ALGORITHMS_DEEP.md` §04 lines 4657-4700)
// ============================================================================
const B3_GROUND_FRAGMENT_SHADER = /* glsl */ `
${B3_PBR_COMMON_GLSL}

uniform sampler2D uTexAlbedo;
uniform sampler2D uTexMask;
uniform sampler2D uTexNormal;
uniform sampler2D uTexNormal2;
uniform samplerCube uTexEnv;
uniform samplerCube uTexIrradiance;

uniform int uHasTexAlbedo;
uniform int uHasTexMask;
uniform int uHasTexNormal;
uniform int uHasTexNormal2;

uniform float g_RockMetallic;
uniform float g_RockSpecIntensity;
uniform float g_SandRoughness;
uniform float g_SandMetallic;
uniform float g_SandSpecIntensity;
uniform vec2 g_BlendThresholdMinMax;
uniform vec3 g_EmissiveColor;
uniform float g_EmissiveIntensity;

void main() {
  vec2 uv = vUv.xy;

  vec4 sRock = (uHasTexAlbedo == 1) ? texture2D(uTexAlbedo, uv) : vec4(0.42, 0.40, 0.38, 1.0);
  vec4 sSand = (uHasTexMask == 1) ? texture2D(uTexMask, uv * 2.0) : vec4(0.66, 0.60, 0.50, 1.0);

  // Blend factor from vertex color R and height threshold (HZR00 g_BlendThresholdMin/Max)
  float denom = max(g_BlendThresholdMinMax.y - g_BlendThresholdMinMax.x, 1e-3);
  float blendW = smoothstep(0.0, 1.0, (vColor.r * sRock.a - g_BlendThresholdMinMax.x) / denom);
  if (uHasTexMask == 0) blendW = 0.0;

  vec3 Nw = normalize(vNrmW);
  vec3 Nv = normalize(vNrmV);
  if (uHasTexNormal == 1) {
    vec4 n1 = texture2D(uTexNormal, uv);
    vec3 nTS = DecodeB3NormalTS(n1);
    if (uUseSecondaryNormals == 1 && uHasTexNormal2 == 1) {
      vec4 n2 = texture2D(uTexNormal2, uv * 2.0);
      nTS = normalize(mix(nTS, DecodeB3NormalTS(n2), blendW));
    }
    Nw = PerturbB3Normal(nTS, 1.0, vTanW, Nw);
    Nv = PerturbB3Normal(nTS, 1.0, vTanV, Nv);
  }

  vec3 base = mix(DecodeB3Albedo(sRock.rgb), DecodeB3Albedo(sSand.rgb), blendW);
  float metal = clamp(mix(g_RockMetallic, g_SandMetallic, blendW), 0.0, 1.0);
  float rough = clamp(mix(0.65, max(g_SandRoughness, 0.25), blendW), 0.08, 1.0);
  float specRate = mix(max(g_RockSpecIntensity, 0.5), max(g_SandSpecIntensity, 0.5), blendW);

  vec3 F0 = mix(vec3(0.03), base, metal);
  vec3 diffK = base * (1.0 - metal);
  vec3 V = normalize(-vEyeW);

  vec3 direct = EvaluateB3DirectPbr(Nw, V, diffK, rough, F0);
  vec3 ibl = EvaluateB3Ibl(Nw, V, diffK, rough, F0, 1.0, specRate, uTexEnv, uTexIrradiance);
  vec3 colour = direct + ibl + g_EmissiveColor * g_EmissiveIntensity * 0.25;

  gl_FragColor = FinishB3Fragment(colour, 1.0, Nw, Nv, vPosV);
}
`;

// ============================================================================
// 7. JWL00 / JWL01 / DMN00 / SPRT00 / Eye* — Dual-Height Parallax Crystal & Jewel
// (`SHADER_ALGORITHMS_DEEP.md` §3-4 lines 6860-7140)
// ============================================================================
const B3_JEWEL_FRAGMENT_SHADER = /* glsl */ `
${B3_PBR_COMMON_GLSL}

uniform sampler2D uTexAlbedo;
uniform sampler2D uTexMask;
uniform sampler2D uTexNormal;
uniform samplerCube uTexEnv;

uniform int uHasTexAlbedo;
uniform int uHasTexMask;
uniform int uHasTexNormal;

uniform vec3 g_AlbedoColor;
uniform vec3 g_AlbedoColor2;
uniform float g_Roughness;
uniform float g_EmissiveIntensity;
uniform float g_RefractionBias;
uniform float g_Height1;
uniform float g_Height2;
uniform vec2 g_FresnelMinMax;
uniform vec3 g_FresnelColor;
uniform vec3 g_EmissiveRoughnessParams; // x = Begin, y = Contrast, z = Blend

void main() {
  vec3 Nw = normalize(vNrmW);
  vec3 Nv = normalize(vNrmV);
  float erMask = 0.5;

  if (uHasTexNormal == 1) {
    vec4 sNorm = texture2D(uTexNormal, vUv.xy);
    erMask = sNorm.a;
    vec3 nTS = DecodeB3NormalTS(sNorm);
    Nw = PerturbB3Normal(nTS, 1.0, vTanW, Nw);
    Nv = PerturbB3Normal(nTS, 1.0, vTanV, Nv);
  }

  // Dual-height mirror-pair parallax UV offsets (SHADER_ALGORITHMS_DEEP.md §3 lines 6876-6884)
  vec2 uvBase = vUv.xy;
  vec2 uv1 = uvBase + vec2(Nv.x * g_Height1, -Nv.y * g_Height1);
  vec2 uv2 = uvBase + vec2(Nv.x * g_Height2, -Nv.y * g_Height2);

  vec4 lay1 = (uHasTexAlbedo == 1) ? texture2D(uTexAlbedo, uv1) : vec4(0.55, 0.78, 0.98, 1.0);
  vec4 lay2 = (uHasTexAlbedo == 1) ? texture2D(uTexAlbedo, uv2) : vec4(0.35, 0.62, 0.92, 1.0);
  vec3 alb2 = (uHasTexMask == 1) ? texture2D(uTexMask, uv2).rgb : vec3(0.0);

  // JWL01 Emissive-roughness ramp (§4 lines 7051-7065)
  float erBegin = g_EmissiveRoughnessParams.x;
  float erContrast = g_EmissiveRoughnessParams.y;
  float erBlend = g_EmissiveRoughnessParams.z;
  float tEr = saturate((erMask - erBegin) * 4.0);
  float er = saturate(((tEr - erBegin) * (1.0 + erContrast) + erBegin) * max(erBlend, 0.25));
  float invEr = 1.0 - er;
  float rough = clamp(mix(g_Roughness, er, 0.5), 0.03, 0.85);

  vec3 jewelTint = dot(g_AlbedoColor, g_AlbedoColor) > 1e-4
    ? DecodeB3Albedo(g_AlbedoColor)
    : vec3(1.0);
  vec3 sampledBase = DecodeB3Albedo(lay1.rgb * lay2.rgb + alb2);
  vec3 base = sampledBase * jewelTint;
  if (dot(g_AlbedoColor2, g_AlbedoColor2) > 1e-4) {
    base = mix(base, DecodeB3Albedo(g_AlbedoColor2), invEr * 0.45);
  }

  vec3 V = normalize(-vEyeW);

  // Fresnel smoothstep & g_RefractionBias colour mix (§3 lines 6890-6909)
  float fDenom = max(g_FresnelMinMax.y - g_FresnelMinMax.x, 1e-4);
  float tFres = saturate((1.0 - saturate(dot(Nw, V)) - g_FresnelMinMax.x) / fDenom);
  float fres = tFres * tFres * (3.0 - 2.0 * tFres);
  vec3 fCol = mix(base, DecodeB3Albedo(g_FresnelColor), clamp(g_RefractionBias, 0.0, 1.0) * fres);

  // Dual mirror environment cube taps: reflect(-V, N) and reflect(-V, -N) (§3 lines 6918-6923)
  vec3 envR = texCUBE_game(uTexEnv, reflect(-V, Nw)).rgb;
  vec3 envF = texCUBE_game(uTexEnv, reflect(-V, -Nw)).rgb;
  vec3 env = (envR * fres + envF * (1.0 - fres)) * (1.0 - rough * 0.65);

  vec3 F0 = mix(vec3(0.05), fCol, 0.35);
  vec3 lit = EvaluateB3DirectPbr(Nw, V, fCol * 0.65, rough, F0);
  float specK = 1.0 + max(g_EmissiveIntensity, 0.0) * 0.35;
  vec3 colour = lit * specK + env * fCol + fCol * invEr * max(g_EmissiveIntensity, 0.0) * 0.25;

  gl_FragColor = FinishB3Fragment(colour, 1.0, Nw, Nv, vPosV);
}
`;

// ============================================================================
// 8. CNS* / HMFRM* / OCL* / FOG* — Effect Cards, Flow Maps & Frame Shaders
// (`SHADER_ALGORITHMS_DEEP.md` §05a-05b lines 6360-6720)
// ============================================================================
const B3_EFFECT_FRAGMENT_SHADER = /* glsl */ `
${B3_PBR_COMMON_GLSL}

uniform sampler2D uTexAlbedo;
uniform sampler2D uTexMask;
uniform samplerCube uTexEnv;

uniform int uHasTexAlbedo;
uniform int uHasTexMask;
uniform float g_bAlbedoOverWrite;
uniform vec3 g_AlbedoColor;
uniform float g_EmissiveIntensity;
uniform vec2 g_Tile;

void main() {
  vec2 uv = vUv.xy * g_Tile;
  if (uHasTexMask == 1) {
    vec2 flow = texture2D(uTexMask, vUv.zw).rg * 2.0 - 1.0;
    uv += vec2(flow.x, -flow.y) * 0.08;
  }

  vec4 sAlb = (uHasTexAlbedo == 1) ? texture2D(uTexAlbedo, uv) : vec4(0.85, 0.88, 0.95, 1.0);
  vec3 base = (g_bAlbedoOverWrite > 0.5)
    ? DecodeB3Albedo(g_AlbedoColor)
    : DecodeB3Albedo(sAlb.rgb) * DecodeB3Albedo(g_AlbedoColor);

  vec3 Nw = normalize(vNrmW);
  vec3 Nv = normalize(vNrmV);
  vec3 V = normalize(-vEyeW);
  vec3 env = texCUBE_game(uTexEnv, reflect(-V, Nw)).rgb * 0.25;
  float intensity = max(g_EmissiveIntensity, 1.0);

  vec3 colour = base * intensity + env;
  gl_FragColor = FinishB3Fragment(colour, clamp(sAlb.a, 0.25, 1.0), Nw, Nv, vPosV);
}
`;

interface B3ResolvedFamily {
  id: string;
  fragmentShader: string;
  queue: RenderQueueBucket;
  transparent: boolean;
  depthWrite: boolean;
  pbsVariant: number;
  pbsHasDFlag: number;
  hairVariant: number;
  skinVariant: number;
  glassVariant: number;
}

/**
 * Classifies a Bayonetta 3 `shader_name` (up to 16-char prefix) into its implemented shader family,
 * or returns `null` if the shader name is unknown/unimplemented so the registry falls back to
 * `FALLBACK_FRAGMENT_SHADER` (`isFallback: true`, showing the `⚠` badge in `Hierarchy.tsx`).
 */
function resolveB3ShaderFamily(
  shaderName: string | null
): B3ResolvedFamily | null {
  const raw = (shaderName ?? "").trim();
  if (!raw) return null;
  const low = raw.toLowerCase();

  // Extract suffix flags after '_' to detect alpha-blend / cutout / D permutations
  const parts = raw.split("_");
  const flags = parts.length > 1 ? parts.slice(1).join("_") : "";
  const isAlphaBlendFlag = flags.includes("A") || flags.includes("T");
  // Position 5 'D' (without Da/Db) in PBS04_XXXXXDXXX selects albedo.a metallic
  const hasStandaloneD =
    flags.includes("D") && !flags.includes("Da") && !flags.includes("Db");

  // 1. PBS families
  if (low.startsWith("pbs00")) {
    const queue: RenderQueueBucket = isAlphaBlendFlag ? "blend" : "opaque";
    return {
      id: `b3:PBS00 (${raw})`,
      fragmentShader: B3_PBS_FRAGMENT_SHADER,
      queue,
      transparent: queue === "blend",
      depthWrite: queue !== "blend",
      pbsVariant: 0,
      pbsHasDFlag: 0,
      hairVariant: 0,
      skinVariant: 20,
      glassVariant: 0,
    };
  }
  if (low.startsWith("pbs01")) {
    return {
      id: `b3:PBS01 (${raw})`,
      fragmentShader: B3_PBS_FRAGMENT_SHADER,
      queue: "opaque",
      transparent: false,
      depthWrite: true,
      pbsVariant: 1,
      pbsHasDFlag: 0,
      hairVariant: 0,
      skinVariant: 20,
      glassVariant: 0,
    };
  }
  if (low.startsWith("pbs02")) {
    return {
      id: `b3:PBS02 (${raw})`,
      fragmentShader: B3_PBS_FRAGMENT_SHADER,
      queue: "opaque",
      transparent: false,
      depthWrite: true,
      pbsVariant: 2,
      pbsHasDFlag: 0,
      hairVariant: 0,
      skinVariant: 20,
      glassVariant: 0,
    };
  }
  if (low.startsWith("pbs03")) {
    return {
      id: `b3:PBS03 (${raw})`,
      fragmentShader: B3_PBS_FRAGMENT_SHADER,
      queue: "opaque",
      transparent: false,
      depthWrite: true,
      pbsVariant: 3,
      pbsHasDFlag: 0,
      hairVariant: 0,
      skinVariant: 20,
      glassVariant: 0,
    };
  }
  if (low.startsWith("pbs04")) {
    return {
      id: `b3:PBS04 (${raw})`,
      fragmentShader: B3_PBS_FRAGMENT_SHADER,
      queue: "opaque",
      transparent: false,
      depthWrite: true,
      pbsVariant: 4,
      pbsHasDFlag: hasStandaloneD ? 1 : 0,
      hairVariant: 0,
      skinVariant: 20,
      glassVariant: 0,
    };
  }
  if (low.startsWith("pbs21")) {
    return {
      id: `b3:PBS21 (${raw})`,
      fragmentShader: B3_PBS_FRAGMENT_SHADER,
      queue: "opaque",
      transparent: false,
      depthWrite: true,
      pbsVariant: 21,
      pbsHasDFlag: 0,
      hairVariant: 0,
      skinVariant: 20,
      glassVariant: 0,
    };
  }
  if (low.startsWith("pbs91")) {
    return {
      id: `b3:PBS91 (${raw})`,
      fragmentShader: B3_PBS_FRAGMENT_SHADER,
      queue: "opaque",
      transparent: false,
      depthWrite: true,
      pbsVariant: 91,
      pbsHasDFlag: 0,
      hairVariant: 0,
      skinVariant: 20,
      glassVariant: 0,
    };
  }
  if (low.startsWith("pbs99")) {
    return {
      id: `b3:PBS99 (${raw})`,
      fragmentShader: B3_PBS_FRAGMENT_SHADER,
      queue: "opaque",
      transparent: false,
      depthWrite: true,
      pbsVariant: 99,
      pbsHasDFlag: 0,
      hairVariant: 0,
      skinVariant: 20,
      glassVariant: 0,
    };
  }
  if (
    low.startsWith("pbs30") ||
    low.startsWith("pbs35") ||
    low.startsWith("pbs40") ||
    low.startsWith("pbs") ||
    low.startsWith("golem") ||
    low.startsWith("plt") ||
    low.startsWith("hmmny")
  ) {
    return {
      id: `b3:PBS (${raw})`,
      fragmentShader: B3_PBS_FRAGMENT_SHADER,
      queue: isAlphaBlendFlag ? "blend" : "opaque",
      transparent: isAlphaBlendFlag,
      depthWrite: !isAlphaBlendFlag,
      pbsVariant: 0,
      pbsHasDFlag: 0,
      hairVariant: 0,
      skinVariant: 20,
      glassVariant: 0,
    };
  }

  // 2. BAYO00
  if (low.startsWith("bayo00")) {
    return {
      id: `b3:BAYO00 (${raw})`,
      fragmentShader: B3_BAYO00_FRAGMENT_SHADER,
      queue: "opaque",
      transparent: false,
      depthWrite: true,
      pbsVariant: 0,
      pbsHasDFlag: 0,
      hairVariant: 0,
      skinVariant: 20,
      glassVariant: 0,
    };
  }

  // 3. Hair20 / Hair21 / Hair22 / FUR01 / FUR02 / FBR00 / CES00
  if (
    low.startsWith("hair20") ||
    low.startsWith("hair21") ||
    low.startsWith("hair22") ||
    low.startsWith("fur") ||
    low.startsWith("fbr") ||
    low.startsWith("ces")
  ) {
    const isFbr = low.startsWith("fbr");
    const isFur01 = low.startsWith("fur01");
    const hv = low.startsWith("hair22")
      ? 22
      : low.startsWith("hair21")
      ? 21
      : isFur01
      ? 30
      : isFbr
      ? 40
      : 20;
    const queue: RenderQueueBucket = isFbr || isFur01 ? "opaque" : "alphatest";
    return {
      id: isFbr ? `b3:FBR00 (${raw})` : isFur01 ? `b3:FUR01 (${raw})` : `b3:Hair${hv} (${raw})`,
      fragmentShader: isFbr ? B3_FABRIC_FRAGMENT_SHADER : B3_HAIR_FRAGMENT_SHADER,
      queue,
      transparent: false,
      depthWrite: true,
      pbsVariant: 0,
      pbsHasDFlag: 0,
      hairVariant: hv,
      skinVariant: 20,
      glassVariant: 0,
    };
  }

  // 4. Skin families: SKN20, SKN21, SKN25, SKN30, SKN31, HMSKN00, HMSKN01, HMSKN10
  if (low.startsWith("skn") || low.startsWith("hmskn")) {
    const sv = low.startsWith("skn21")
      ? 21
      : low.startsWith("skn25")
      ? 25
      : low.startsWith("skn30") || low.startsWith("skn31")
      ? 30
      : 20;
    return {
      id: `b3:Skin (${raw})`,
      fragmentShader: B3_SKIN_FRAGMENT_SHADER,
      queue: "opaque",
      transparent: false,
      depthWrite: true,
      pbsVariant: 0,
      pbsHasDFlag: 0,
      hairVariant: 0,
      skinVariant: sv,
      glassVariant: 0,
    };
  }

  // 5. Glass & Window families: Gls14, Gls15, Gls01, Gls02, Gls03, Gls20, Gls22, Gls31
  if (low.startsWith("gls")) {
    const isGls03 = low.startsWith("gls03");
    const isGls14Or15 = low.startsWith("gls14") || low.startsWith("gls15");
    const isInterior = isGls03 || isGls14Or15;
    const queue: RenderQueueBucket = isInterior ? "opaque" : "blend";
    return {
      id: `b3:Glass (${raw})`,
      fragmentShader: B3_GLASS_FRAGMENT_SHADER,
      queue,
      transparent: queue === "blend",
      depthWrite: queue !== "blend",
      pbsVariant: 0,
      pbsHasDFlag: 0,
      hairVariant: 0,
      skinVariant: 20,
      glassVariant: isGls03 ? 3 : isGls14Or15 ? 14 : 2,
    };
  }

  // 6. Ground / Terrain / Water / Lava / Foam: HZR00, Sand00, Wtr01, Wtr03, Wtr10, Lava00, VNS05/06/10
  if (
    low.startsWith("hzr00") ||
    low.startsWith("sand00") ||
    low.startsWith("wtr") ||
    low.startsWith("lava00") ||
    low.startsWith("vns")
  ) {
    const isWater = low.startsWith("wtr") || low.startsWith("vns");
    return {
      id: `b3:Ground (${raw})`,
      fragmentShader: B3_GROUND_FRAGMENT_SHADER,
      queue: isWater ? "blend" : "opaque",
      transparent: isWater,
      depthWrite: !isWater,
      pbsVariant: 0,
      pbsHasDFlag: 0,
      hairVariant: 0,
      skinVariant: 20,
      glassVariant: 0,
    };
  }

  // 7. Jewels / Crystals / Demon / Spirit / Eyes: JWL00, JWL01, DMN00, SPRT00, EYEBALL, Eye*, HMEYE
  if (
    low.startsWith("jwl") ||
    low.startsWith("dmn00") ||
    low.startsWith("sprt00") ||
    low.startsWith("eye") ||
    low.startsWith("hmeye")
  ) {
    return {
      id: `b3:Jewel (${raw})`,
      fragmentShader: B3_JEWEL_FRAGMENT_SHADER,
      queue: "opaque",
      transparent: false,
      depthWrite: true,
      pbsVariant: 0,
      pbsHasDFlag: 0,
      hairVariant: 0,
      skinVariant: 20,
      glassVariant: 0,
    };
  }

  // 8. Effects & Frame shaders: CNS*, HMFRM*, OCL*, FOG*, CSH00, TEX00
  if (
    low.startsWith("cns") ||
    low.startsWith("hmfrm") ||
    low.startsWith("ocl") ||
    low.startsWith("fog") ||
    low.startsWith("csh") ||
    low.startsWith("tex00")
  ) {
    const isCard = low.startsWith("cns") || low.startsWith("fog") || low.startsWith("csh");
    return {
      id: `b3:Effect (${raw})`,
      fragmentShader: B3_EFFECT_FRAGMENT_SHADER,
      queue: isCard ? "blend" : "opaque",
      transparent: isCard,
      depthWrite: !isCard,
      pbsVariant: 0,
      pbsHasDFlag: 0,
      hairVariant: 0,
      skinVariant: 20,
      glassVariant: 0,
    };
  }

  return null;
}

export function getB3ShaderSpec(shaderName: string | null): {
  id: string;
  vertexShader: string;
  fragmentShader: string;
  queue: RenderQueueBucket;
  transparent: boolean;
  depthWrite: boolean;
  isFallback: boolean;
  envUseNormal: boolean;
  alphaBlendVariant: boolean;
  pbsVariant: number;
  pbsHasDFlag: number;
  hairVariant: number;
  skinVariant: number;
  glassVariant: number;
} | null {
  const resolved = resolveB3ShaderFamily(shaderName);
  if (!resolved) return null;
  return {
    id: resolved.id,
    vertexShader: STUDIO_VERTEX_SHADER,
    fragmentShader: resolved.fragmentShader,
    queue: resolved.queue,
    transparent: resolved.transparent,
    depthWrite: resolved.depthWrite,
    isFallback: false,
    envUseNormal: true,
    alphaBlendVariant: resolved.transparent,
    pbsVariant: resolved.pbsVariant,
    pbsHasDFlag: resolved.pbsHasDFlag,
    hairVariant: resolved.hairVariant,
    skinVariant: resolved.skinVariant,
    glassVariant: resolved.glassVariant,
  };
}
