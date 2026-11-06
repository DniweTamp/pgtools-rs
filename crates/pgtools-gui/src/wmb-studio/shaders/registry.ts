import * as THREE from "three";
import { MaterialDesc, TextureSlotDesc } from "../contract";
import { RenderQueueBucket, STUDIO_FRAGMENT_HEADER, STUDIO_VERTEX_SHADER } from "./common";
import { B1_SHADERS } from "./b1/shaders";
import {
  classifyB2Queue,
  getB2ShaderSpec,
  resolveB2SamplerSlots,
} from "./b2/shaders";
import {
  getB3ShaderSpec,
  resolveB3SamplerSlots,
} from "./b3/shaders";

export { resolveB2SamplerSlots, resolveB3SamplerSlots };

export interface ResolvedShaderSpec {
  id: string;
  vertexShader: string;
  fragmentShader: string;
  queue: RenderQueueBucket;
  transparent: boolean;
  depthWrite: boolean;
  isFallback: boolean;
  envUseNormal: boolean;
  alphaBlendVariant: boolean;
  useVertexAlpha?: boolean;
  pbsVariant?: number;
  pbsHasDFlag?: number;
  hairVariant?: number;
  skinVariant?: number;
  glassVariant?: number;
}

export const FALLBACK_FRAGMENT_SHADER = /* glsl */ `
${STUDIO_FRAGMENT_HEADER}

uniform sampler2D Color_1_sampler;
uniform sampler2D reliefmap_sampler;
uniform int uHasNormalMap;
uniform vec2 Color_1_Tile;
uniform vec2 Color_1_Offset;
uniform vec2 tile;
uniform float depth;
uniform vec4 uBaseTint;

void main() {
  vec4 albedo = texture2D(Color_1_sampler, vUv.xy * Color_1_Tile + Color_1_Offset) * uBaseTint;
  if (uDebugMode == 0 && albedo.a <= 0.3333) discard;

  vec3 Nw = normalize(vNrmW);
  vec3 Nv = normalize(vNrmV);
  if (uHasNormalMap == 1) {
    vec4 nm = texture2D(reliefmap_sampler, vUv.zw * tile);
    Nw = normalize(PerturbNormal(nm, depth, vTanW, Nw));
    Nv = normalize(PerturbNormal(nm, depth, normalize(vTanV), Nv));
  }

  float keyNdotL = saturate(dot(Nw, STUDIO_KEY_TO_LIGHT));
  float fillNdotL = saturate(dot(Nw, STUDIO_FILL_TO_LIGHT));
  float hemi = Nw.y * 0.5 + 0.5;
  vec3 hemiAmb = mix(vec3(0.22, 0.24, 0.28), vec3(0.38, 0.40, 0.44), hemi) * (STUDIO_AMBIENT_INTENSITY / 0.3);
  vec3 direct = vec3(keyNdotL * STUDIO_KEY_INTENSITY + fillNdotL * STUDIO_FILL_INTENSITY);

  vec3 V = normalize(-vPosV);
  vec3 H = normalize(vec3(0.35, 0.65, 0.68) + V);
  float spec = pow(saturate(dot(Nv, H)), 24.0) * 0.18;

  vec3 color = albedo.rgb * (hemiAmb + direct * 0.78) + vec3(spec);
  gl_FragColor = ApplyStudioPost(vec4(color, albedo.a), Nw, Nv, vPosV);
}
`;

export function getShader(
  game: "b1" | "b2" | "b3",
  shaderName: string | null,
  materialType: number | null
): ResolvedShaderSpec {
  if (game === "b1") {
    const cleanName = (shaderName ?? "").trim().toLowerCase();
    const spec = B1_SHADERS.find(
      (s) =>
        (materialType !== null && s.materialTypes.includes(materialType)) ||
        (cleanName.length > 0 &&
          s.shaderNames.some((n) => n.toLowerCase() === cleanName))
    );
    if (spec) {
      const isMVariant =
        materialType === 75 ||
        materialType === 76 ||
        materialType === 104 ||
        materialType === 109 ||
        materialType === 113 ||
        materialType === 151 ||
        materialType === 182 ||
        materialType === 183 ||
        materialType === 185 ||
        materialType === 186 ||
        cleanName.endsWith("mxx") ||
        cleanName.endsWith("mxa") ||
        cleanName.endsWith("mxb") ||
        cleanName.endsWith("mxy") ||
        cleanName.includes("_bxxmx") ||
        cleanName.includes("_bxnmx") ||
        cleanName.includes("_bxnma") ||
        cleanName.includes("_bxxma") ||
        cleanName.includes("_bxxmb") ||
        cleanName.includes("_bxxmy");
      const isBlendVariant =
        materialType === 59 ||
        materialType === 60 ||
        materialType === 88 ||
        materialType === 104 ||
        materialType === 149 ||
        materialType === 184 ||
        materialType === 185 ||
        materialType === 187 ||
        cleanName.endsWith("b");
      const isVertexAlphaVariant =
        materialType === 148 ||
        materialType === 149 ||
        cleanName.includes("pls07") ||
        cleanName.includes("pls08");
      const queue: RenderQueueBucket = isBlendVariant
        ? "blend"
        : materialType === 135 || materialType === 181
        ? "opaque"
        : spec.queue;
      return {
        id: spec.id,
        vertexShader: spec.vertexShader,
        fragmentShader: spec.fragmentShader,
        queue,
        transparent: queue === "blend",
        depthWrite: queue !== "blend",
        isFallback: false,
        envUseNormal: isMVariant,
        alphaBlendVariant: isBlendVariant,
        useVertexAlpha: isVertexAlphaVariant,
      };
    }
  }

  if (game === "b2") {
    return getB2ShaderSpec(shaderName, materialType);
  }

  if (game === "b3") {
    const b3Spec = getB3ShaderSpec(shaderName);
    if (b3Spec) {
      return b3Spec;
    }
  }

  return {
    id: "fallback",
    vertexShader: STUDIO_VERTEX_SHADER,
    fragmentShader: FALLBACK_FRAGMENT_SHADER,
    queue: "opaque",
    transparent: false,
    depthWrite: true,
    isFallback: true,
    envUseNormal: false,
    alphaBlendVariant: false,
  };
}

export function isB1GlobalCubeId(texId: number): boolean {
  const u = texId >>> 0;
  return (
    u === 0x80000001 ||
    u === 0x80000003 ||
    u === 0x80000007 ||
    u === 0x80000008 ||
    u === 0x80000009 ||
    u === 0x8000000a
  );
}

export function isB1GlobalSpecLutId(texId: number): boolean {
  return (texId >>> 0) === 0x80000006;
}

export interface B1SamplerSlotBinding {
  Color_1?: TextureSlotDesc;
  reliefmap?: TextureSlotDesc;
  envmap?: TextureSlotDesc;
  env_amb?: TextureSlotDesc;
  Spec_Pow?: TextureSlotDesc;
  Spec_Mask?: TextureSlotDesc;
  lightmap?: TextureSlotDesc;
}

/**
 * Resolves the sampler slots for a Bayonetta 1 material, handling both:
 * 1. Canonical WMB0 binary order (`Color_1 < lightmap < Spec_Mask < reliefmap < env_amb < envmap < Spec_Pow`),
 *    including the `pls01` (types 68/113/132/148/182/185/186/188), `obs00` (types 56/75), `etc01` (type 60),
 *    and `pls00` (type 67) slot swaps noted in `bayonetta_character_shaders.hlsl`.
 * 2. `MATERIAL_DATABASE` named slot layout (used by synthetic/demo materials or user-edited slots).
 */
export function resolveB1SamplerSlots(mat: MaterialDesc): B1SamplerSlotBinding {
  const slots = mat.textureSlots;
  const byName = new Map<string, TextureSlotDesc>();
  for (const s of slots) {
    byName.set(s.name, s);
  }

  const out: B1SamplerSlotBinding = {
    Color_1: byName.get("Color_1") ?? slots[0],
    reliefmap: byName.get("reliefmap"),
    envmap: byName.get("envmap"),
    env_amb: byName.get("env_amb"),
    Spec_Pow: byName.get("Spec_Pow"),
    Spec_Mask: byName.get("Spec_Mask"),
    lightmap: byName.get("lightmap"),
  };

  const mt = mat.materialType;

  // pls01 / pls07 (types 68, 113, 132, 148, 182, 185, 186, 188):
  // YAML order: [0]=Color_1, [1]=envmap, [2]=Spec_Mask, [3]=Spec_Pow
  // Binary order: [0]=Color_1, [1]=Spec_Mask, [2]=envmap, [3]=Spec_Pow
  if (
    (mt === 68 ||
      mt === 113 ||
      mt === 132 ||
      mt === 148 ||
      mt === 182 ||
      mt === 185 ||
      mt === 186 ||
      mt === 188) &&
    slots.length >= 3
  ) {
    const s1 = slots[1];
    const s2 = slots[2];
    if (isB1GlobalCubeId(s2.textureId) && !isB1GlobalCubeId(s1.textureId)) {
      out.Spec_Mask = s1;
      out.envmap = s2;
    } else {
      out.envmap = s1;
      out.Spec_Mask = s2;
    }
  }

  // Type 56/75 (obs00) & Type 60 (etc01) & Type 67 (pls00):
  // YAML order: [0]=Color_1, [1]=reliefmap, [2]=envmap, [3]=Spec_Mask (, [4]=Spec_Pow)
  // Binary order: [0]=Color_1, [1]=Spec_Mask, [2]=reliefmap, [3]=envmap (, [4]=Spec_Pow)
  if ((mt === 56 || mt === 60 || mt === 67 || mt === 75) && slots.length >= 4) {
    const s1 = slots[1];
    const s2 = slots[2];
    const s3 = slots[3];
    if (isB1GlobalCubeId(s3.textureId) && !isB1GlobalCubeId(s2.textureId)) {
      out.Spec_Mask = s1;
      out.reliefmap = s2;
      out.envmap = s3;
    }
  }

  // Type 151 (pls09):
  // YAML order: [0]=Color_1, [1]=envmap, [2]=Spec_Pow, [3]=env_amb
  // Binary order: [0]=Color_1, [1]=env_amb, [2]=envmap, [3]=Spec_Pow
  if (mt === 151 && slots.length >= 4) {
    const s1 = slots[1];
    const s2 = slots[2];
    const s3 = slots[3];
    if (
      isB1GlobalSpecLutId(s3.textureId) ||
      (!isB1GlobalCubeId(s3.textureId) && isB1GlobalCubeId(s2.textureId))
    ) {
      out.env_amb = s1;
      out.envmap = s2;
      out.Spec_Pow = s3;
    }
  }

  // Types with both envmap and env_amb (64, 110, 114, 135, 163, 178, 181, 184):
  // Binary order puts env_amb (soft cube 0x80000008 / 0x8000000A) before envmap (hard cube 0x80000007 / 0x80000009),
  // or if both cubes are identical, slot 2 is env_amb and slot 3 is envmap.
  if (
    (mt === 64 ||
      mt === 110 ||
      mt === 114 ||
      mt === 135 ||
      mt === 163 ||
      mt === 178 ||
      mt === 181 ||
      mt === 184) &&
    slots.length >= 4
  ) {
    const s2 = slots[2];
    const s3 = slots[3];
    const u2 = s2.textureId >>> 0;
    const u3 = s3.textureId >>> 0;
    const s2IsSoft =
      u2 === 0x80000008 || u2 === 0x8000000a || u2 === 0x80000001;
    const s3IsHard = u3 === 0x80000007 || u3 === 0x80000009;
    const s2IsHard = u2 === 0x80000007 || u2 === 0x80000009;
    const s3IsSoft = u3 === 0x80000008 || u3 === 0x8000000a;
    if ((s2IsSoft && s3IsHard) || (!s2IsHard && !s3IsSoft && isB1GlobalCubeId(u2) && isB1GlobalCubeId(u3))) {
      out.env_amb = s2;
      out.envmap = s3;
    }
  }

  return out;
}

function findParamValues(
  mat: MaterialDesc,
  name: string,
  fallback: number[]
): number[] {
  const low = name.toLowerCase();
  const found = mat.params.find(
    (p) => p.name.toLowerCase() === low || p.key.toLowerCase() === low
  );
  if (!found || found.values.length === 0) return fallback;
  return found.values;
}

function findScalarParam(
  mat: MaterialDesc,
  candidateNames: string[],
  fallback: number
): number {
  for (const name of candidateNames) {
    const low = name.toLowerCase();
    const found = mat.params.find(
      (p) =>
        p.name.toLowerCase() === low ||
        p.key.toLowerCase() === low ||
        p.key.toLowerCase().startsWith(`${low}_`)
    );
    if (found && found.values.length > 0 && Number.isFinite(found.values[0])) {
      return found.values[0];
    }
  }
  return fallback;
}

function findPositiveScalarParam(
  mat: MaterialDesc,
  candidateNames: string[],
  fallback: number
): number {
  const val = findScalarParam(mat, candidateNames, fallback);
  return val > 1e-5 ? val : fallback;
}

/**
 * Resolves a 3-component vector or RGB parameter from `MaterialDesc.params`, supporting both:
 * 1. Separate scalar parameters (`<prefix>_X`, `<prefix>_Y`, `<prefix>_Z` or `<prefix>R`, `<prefix>G`, `<prefix>B`)
 *    used by Bayonetta 3 `shader_params_bayo3.json`
 * 2. Packed vec3/vec4 parameters (`<prefix>`)
 */
function findVec3Param(
  mat: MaterialDesc,
  prefix: string,
  fallback: [number, number, number],
  rejectAllZero = false
): [number, number, number] {
  const vecMatch = mat.params.find(
    (p) =>
      (p.name.toLowerCase() === prefix.toLowerCase() ||
        p.key.toLowerCase() === prefix.toLowerCase()) &&
      p.values.length >= 3
  );
  if (vecMatch) {
    const rx = vecMatch.values[0] ?? fallback[0];
    const ry = vecMatch.values[1] ?? fallback[1];
    const rz = vecMatch.values[2] ?? fallback[2];
    if (
      rejectAllZero &&
      Math.abs(rx) < 1e-5 &&
      Math.abs(ry) < 1e-5 &&
      Math.abs(rz) < 1e-5
    ) {
      return fallback;
    }
    return [rx, ry, rz];
  }

  const x = findScalarParam(
    mat,
    [`${prefix}_X`, `${prefix}_x`, `${prefix}R`],
    NaN
  );
  const y = findScalarParam(
    mat,
    [`${prefix}_Y`, `${prefix}_y`, `${prefix}G`],
    NaN
  );
  const z = findScalarParam(
    mat,
    [`${prefix}_Z`, `${prefix}_z`, `${prefix}B`],
    NaN
  );
  if (Number.isFinite(x) || Number.isFinite(y) || Number.isFinite(z)) {
    const rx = Number.isFinite(x) ? x : fallback[0];
    const ry = Number.isFinite(y) ? y : fallback[1];
    const rz = Number.isFinite(z) ? z : fallback[2];
    if (
      rejectAllZero &&
      Math.abs(rx) < 1e-5 &&
      Math.abs(ry) < 1e-5 &&
      Math.abs(rz) < 1e-5
    ) {
      return fallback;
    }
    return [rx, ry, rz];
  }
  return fallback;
}

function resolveMinMaxBounds(
  mat: MaterialDesc,
  minName: string,
  maxName: string,
  defMin: number,
  defMax: number
): [number, number] {
  const mn = findScalarParam(mat, [minName], defMin);
  const mx = findScalarParam(mat, [maxName], defMax);
  if (mx <= mn + 1e-5 && Math.abs(mx) < 1e-5) {
    return [mn, defMax];
  }
  return [mn, mx];
}

/**
 * Remaps Bayonetta 2's raw `specular.y` (`0.96875` = row 0) to the uploaded GPU V coordinate
 * per `b2_shader_layouts.json` (`0xECB834`):
 * `1.0 - ((1.0 - y - 1/32) * 2.0 + 1/16)` -> mapped into `[0, 1]` where row 0 is at the bottom of `specLut2D`.
 */
function remapB2SpecLutRow(rawY: number): number {
  if (rawY > 0.5 && rawY <= 1.0) {
    const ecb834 = 1.0 - ((1.0 - rawY - 1.0 / 32.0) * 2.0 + 1.0 / 16.0);
    // In `makeProceduralSpecLutTexture`, row 0 (softest curve) is at V=0 and row 15 (sharpest) is at V=1:
    return Math.max(0.0, Math.min(1.0, 1.0 - ecb834));
  }
  return Math.max(0.0, Math.min(1.0, rawY));
}

function populateB2AndB3UniformValues(
  u: Record<string, THREE.IUniform>,
  mat: MaterialDesc,
  resolvedSpec: ResolvedShaderSpec
): void {
  // ---- Bayonetta 2 (`Bayo2.hlsl`) uniforms ----
  const b2Queue = classifyB2Queue(mat.shaderName, mat.materialType);
  if (u.uB2AlphaClip) {
    u.uB2AlphaClip.value = b2Queue.alphaClip ? 1 : 0;
  }

  const b2BaseCol = findParamValues(
    mat,
    "base_color",
    findParamValues(
      mat,
      "color",
      findParamValues(mat, "skin_color", [0.5, 0.5, 0.5, 1.0])
    )
  );
  const b2BaseScaleVec = findParamValues(mat, "base", []);
  const b2DiffuseScale =
    b2BaseScaleVec.length >= 4 && b2BaseScaleVec[3] > 0.01
      ? b2BaseScaleVec[3]
      : 1.0;

  const b2UvTileRaw = findParamValues(mat, "uv_tile", [1, 1, 0, 0]);
  const b2UvScrollRaw = findParamValues(
    mat,
    "uv_scroll",
    findParamValues(mat, "Color_1_Offset", [0, 0])
  );
  const b2SpecVec = findParamValues(mat, "specular", []);
  const b2ReflVec = findParamValues(mat, "reflection", []);
  const b2Tile = nonZeroVec2(
    [b2UvTileRaw[0] ?? 1, b2UvTileRaw[1] ?? 1],
    1,
    1
  );
  if (u.uB2UvTile) u.uB2UvTile.value.set(b2Tile.x, b2Tile.y);
  if (u.uB2UvOffset) {
    const offU =
      b2UvScrollRaw[0] !== undefined && b2UvScrollRaw[0] > -0.99
        ? b2UvScrollRaw[0]
        : 0.0;
    const offV =
      b2UvScrollRaw[1] !== undefined && b2UvScrollRaw[1] > -0.99
        ? b2UvScrollRaw[1]
        : 0.0;
    u.uB2UvOffset.value.set(offU, offV);
  }
  if (u.uB2BaseColor) {
    // In B2, base_color RGB uses 0.5 as neutral 1.0x multiplier, and .w is -1.0 when alpha is unused
    const rawR = b2BaseCol[0] ?? 0.5;
    const rawG = b2BaseCol[1] ?? 0.5;
    const rawB = b2BaseCol[2] ?? 0.5;
    const rawA = b2BaseCol[3] ?? 1.0;
    const scale = rawR <= 0.65 && rawG <= 0.65 && rawB <= 0.65 ? 2.0 : 1.0;
    const r = (rawR >= 0 ? rawR * scale : 1.0) * b2DiffuseScale;
    const g = (rawG >= 0 ? rawG * scale : 1.0) * b2DiffuseScale;
    const b = (rawB >= 0 ? rawB * scale : 1.0) * b2DiffuseScale;
    const a = rawA > 0 ? rawA : 1.0;
    u.uB2BaseColor.value.set(r, g, b, a);
  }
  // Bayo2.hlsl exact defaults: SpecularType = 0.375, SpecularPower = 0.8, SpecularStrength = 15.0, CubemapStrength = 15.0
  const b2SpecType =
    b2SpecVec.length >= 2 && b2SpecVec[1] > 0.01 && b2SpecVec[1] <= 1.0
      ? remapB2SpecLutRow(b2SpecVec[1])
      : findScalarParam(mat, ["SpecularType", "Spec_Type"], 0.375);
  const b2SpecPower =
    b2SpecVec.length >= 3 && b2SpecVec[2] > 0.05
      ? b2SpecVec[2]
      : findScalarParam(mat, ["SpecularPower"], 0.8);
  const b2SpecStrength =
    b2SpecVec.length >= 1 && b2SpecVec[0] >= 0
      ? b2SpecVec[0] * 10.0
      : findScalarParam(mat, ["SpecularStrength"], 15.0);
  const b2CubeStrength =
    b2ReflVec.length >= 1 && b2ReflVec[0] >= 0
      ? b2ReflVec[0]
      : findScalarParam(mat, ["CubemapStrength"], 15.0);
  const b2ShadowCol = findVec3Param(mat, "ShadowColor", [0.5, 0.5, 0.5], true);

  if (u.uB2SpecularType) u.uB2SpecularType.value = b2SpecType;
  if (u.uB2SpecularPower) u.uB2SpecularPower.value = b2SpecPower;
  if (u.uB2SpecularStrength) u.uB2SpecularStrength.value = b2SpecStrength;
  if (u.uB2CubemapStrength) u.uB2CubemapStrength.value = b2CubeStrength;
  if (u.uB2ShadowColor) {
    u.uB2ShadowColor.value.set(b2ShadowCol[0], b2ShadowCol[1], b2ShadowCol[2]);
  }

  // ---- Bayonetta 3 (`PBS00`..`PBS99`, `BAYO00`, `Hair20/21/22`, `SKN20/21/25/30/31`, `Gls*`, `HZR00`, `JWL*`) uniforms ----
  if (u.uB3PbsVariant) u.uB3PbsVariant.value = resolvedSpec.pbsVariant ?? 0;
  if (u.uB3PbsHasDFlag) u.uB3PbsHasDFlag.value = resolvedSpec.pbsHasDFlag ?? 0;
  if (u.uB3HairVariant) u.uB3HairVariant.value = resolvedSpec.hairVariant ?? 20;
  if (u.uB3SkinVariant) u.uB3SkinVariant.value = resolvedSpec.skinVariant ?? 20;
  if (u.uB3GlassVariant) u.uB3GlassVariant.value = resolvedSpec.glassVariant ?? 2;

  const hasNormalSlot = mat.textureSlots.some((s) => s.role === "normal");
  const useNormal = findScalarParam(
    mat,
    ["g_UseNormalMap"],
    hasNormalSlot ? 1.0 : 0.0
  );
  if (u.g_UseNormalMap) u.g_UseNormalMap.value = useNormal;

  if (u.g_bAlbedoOverWrite) {
    u.g_bAlbedoOverWrite.value = findScalarParam(
      mat,
      ["g_bAlbedoOverWrite"],
      0.0
    );
  }
  if (u.g_bRoughnessOverWrite) {
    u.g_bRoughnessOverWrite.value = findScalarParam(
      mat,
      ["g_bRoughnessOverWrite"],
      0.0
    );
  }
  if (u.g_bRoughnessOverWrite2) {
    u.g_bRoughnessOverWrite2.value = findScalarParam(
      mat,
      ["g_bRoughnessOverWrite2"],
      0.0
    );
  }
  if (u.g_bMetalicOverWrite) {
    u.g_bMetalicOverWrite.value = findScalarParam(
      mat,
      ["g_bMetalicOverWrite", "g_bMetallicOverWrite"],
      0.0
    );
  }
  if (u.g_1BitMask) {
    u.g_1BitMask.value = findScalarParam(mat, ["g_1BitMask"], 0.0);
  }

  const tileX = findPositiveScalarParam(
    mat,
    ["g_Tile_X", "g_Tile1_X"],
    1.0
  );
  const tileY = findPositiveScalarParam(
    mat,
    ["g_Tile_Y", "g_Tile1_Y", "g_Tile_X"],
    tileX
  );
  const b3Tile = nonZeroVec2([tileX, tileY], 1.0, 1.0);
  if (u.g_Tile) u.g_Tile.value.set(b3Tile.x, b3Tile.y);

  const tile2X = findPositiveScalarParam(
    mat,
    ["g_Tile2_X", "g_Layer2_Tiling_X"],
    1.0
  );
  const tile2Y = findPositiveScalarParam(
    mat,
    ["g_Tile2_Y", "g_Layer2_Tiling_Y", "g_Tile2_X", "g_Layer2_Tiling_X"],
    tile2X
  );
  if (u.g_Tile2) u.g_Tile2.value.set(tile2X, tile2Y);

  const detTileX = findPositiveScalarParam(
    mat,
    [
      "g_DetailNormalTile_X",
      "g_NormalTile_X",
      "g_Layer1_Tiling_X",
      "g_InnerTiling_X",
    ],
    1.0
  );
  const detTileY = findPositiveScalarParam(
    mat,
    [
      "g_DetailNormalTile_Y",
      "g_NormalTile_Y",
      "g_Layer1_Tiling_Y",
      "g_InnerTiling_Y",
      "g_DetailNormalTile_X",
      "g_NormalTile_X",
      "g_Layer1_Tiling_X",
    ],
    detTileX
  );
  if (u.g_DetailNormalTile) {
    u.g_DetailNormalTile.value.set(detTileX, detTileY);
  }

  if (u.g_UseDetailNormalMap) {
    u.g_UseDetailNormalMap.value = findScalarParam(
      mat,
      ["g_UseDetailNormalMap", "g_UseDetailNormal"],
      0.0
    );
  }
  if (u.g_DetailNormalIntensity) {
    u.g_DetailNormalIntensity.value = findPositiveScalarParam(
      mat,
      ["g_DetailNormalIntensity"],
      0.5
    );
  }
  if (u.g_AlbedoAMetallic) {
    u.g_AlbedoAMetallic.value = findScalarParam(
      mat,
      ["g_AlbedoAMetallic"],
      0.0
    );
  }
  if (u.g_OnlyLayer1) {
    u.g_OnlyLayer1.value = findScalarParam(
      mat,
      ["g_OnlyLayer1", "Group1_02", "1_02"],
      1.0
    );
  }
  if (u.g_EyeBlowMode) {
    u.g_EyeBlowMode.value = findScalarParam(
      mat,
      ["g_EyeBlowMode", "Group1_03", "1_03"],
      0.0
    );
  }
  if (u.g_UseAlphaEffect) {
    u.g_UseAlphaEffect.value = findScalarParam(
      mat,
      ["g_UseAlphaEffect", "Group1_04", "1_04"],
      0.0
    );
  }
  if (u.g_BumpHeight) {
    u.g_BumpHeight.value = findScalarParam(mat, ["g_BumpHeight"], 0.0);
  }
  if (u.g_ParallaxIntensity) {
    u.g_ParallaxIntensity.value = findScalarParam(
      mat,
      ["g_ParallaxIntensity"],
      0.0
    );
  }
  if (u.g_Layer1_UseNormalMap) {
    u.g_Layer1_UseNormalMap.value = findScalarParam(
      mat,
      ["g_Layer1_UseNormalMap"],
      1.0
    );
  }
  if (u.g_Layer1_UseSpecularMask) {
    u.g_Layer1_UseSpecularMask.value = findScalarParam(
      mat,
      ["g_Layer1_UseSpecularMask"],
      1.0
    );
  }

  if (u.g_SpecRefRate) {
    u.g_SpecRefRate.value = findScalarParam(mat, ["g_SpecRefRate"], 1.0);
  }
  if (u.g_UseOcclusionMap) {
    u.g_UseOcclusionMap.value = findScalarParam(
      mat,
      ["g_UseOcclusionMap"],
      1.0
    );
  }
  if (u.g_UV2Use) {
    u.g_UV2Use.value = findScalarParam(mat, ["g_UV2Use"], 0.0);
  }

  const albCol = findVec3Param(mat, "g_AlbedoColor", [1.0, 1.0, 1.0], true);
  if (u.g_AlbedoColor) {
    u.g_AlbedoColor.value.set(albCol[0], albCol[1], albCol[2]);
  }
  const albCol2 = findVec3Param(mat, "g_AlbedoColor2", [0.0, 0.0, 0.0], false);
  if (u.g_AlbedoColor2) {
    u.g_AlbedoColor2.value.set(albCol2[0], albCol2[1], albCol2[2]);
  }

  if (u.g_EmissiveIntensity) {
    u.g_EmissiveIntensity.value = findScalarParam(
      mat,
      ["g_EmissiveIntensity", "g_Intensity"],
      0.0
    );
  }
  if (u.g_EmissiveMaskIntensity) {
    u.g_EmissiveMaskIntensity.value = findPositiveScalarParam(
      mat,
      ["g_EmissiveMaskIntensity"],
      1.0
    );
  }
  const emCol = findVec3Param(
    mat,
    "g_EmissiveOverwriteColor",
    findVec3Param(mat, "g_EmissiveColor", [1.0, 1.0, 1.0], true),
    true
  );
  if (u.g_EmissiveColor) {
    u.g_EmissiveColor.value.set(emCol[0], emCol[1], emCol[2]);
  }
  if (u.g_bUseEmissiveColor) {
    u.g_bUseEmissiveColor.value = findScalarParam(
      mat,
      ["g_bUseEmissiveColor"],
      0.0
    );
  }

  // PBS00 /Variables: g_Roughness or 1.0 - g_Glossiness
  const glossOpt = findScalarParam(mat, ["g_Glossiness"], NaN);
  const defaultRough = Number.isFinite(glossOpt)
    ? Math.max(0.04, Math.min(1.0, 1.0 - glossOpt))
    : 0.42;
  if (u.g_Roughness) {
    u.g_Roughness.value = findPositiveScalarParam(
      mat,
      ["g_Roughness"],
      defaultRough
    );
  }
  if (u.g_Metallic) {
    u.g_Metallic.value = findScalarParam(
      mat,
      ["g_Metallic", "g_Metalic"],
      0.0
    );
  }

  // PBS01 & PBS99 remap bounds
  if (u.g_RoughnessMinMax) {
    const [rMin, rMax] = resolveMinMaxBounds(
      mat,
      "g_RoughnessMin",
      "g_RoughnessMax",
      0.05,
      0.95
    );
    u.g_RoughnessMinMax.value.set(rMin, rMax);
  }
  if (u.g_MetalMinMax) {
    const [mMin, mMax] = resolveMinMaxBounds(
      mat,
      "g_MetalMin",
      "g_MetalMax",
      0.0,
      1.0
    );
    u.g_MetalMinMax.value.set(mMin, mMax);
  }

  // PBS02 clear-coat & PBS99 Maziora
  if (u.g_ClearCoatRate) {
    u.g_ClearCoatRate.value = findScalarParam(mat, ["g_ClearCoatRate"], 0.5);
  }
  if (u.g_ClearCoatFresnel) {
    u.g_ClearCoatFresnel.value = findPositiveScalarParam(
      mat,
      ["g_ClearCoatFresnel"],
      1.5
    );
  }
  if (u.g_ClearCoatFresnelMinMax) {
    const [ccMin, ccMax] = resolveMinMaxBounds(
      mat,
      "g_ClearCoatFresnelMin",
      "g_ClearCoatFresnelMax",
      0.06,
      1.0
    );
    u.g_ClearCoatFresnelMinMax.value.set(ccMin, ccMax);
  }
  if (u.g_Roughness2) {
    u.g_Roughness2.value = findPositiveScalarParam(mat, ["g_Roughness2"], 0.12);
  }
  const botAlb = findVec3Param(
    mat,
    "g_BottomAlbedoColor",
    [0.0, 0.0, 0.0],
    false
  );
  if (u.g_BottomAlbedoColor) {
    u.g_BottomAlbedoColor.value.set(botAlb[0], botAlb[1], botAlb[2]);
  }
  if (u.g_MazioraParams) {
    u.g_MazioraParams.value.set(
      findPositiveScalarParam(mat, ["g_mazioraParam1"], 2.2),
      findPositiveScalarParam(mat, ["g_mazioraParam2"], 1.0),
      findPositiveScalarParam(mat, ["g_mazioraParam3"], 0.55)
    );
  }

  // BAYO00 & Hair20/21/22 parameters
  if (u.g_HairNormalIntensity) {
    u.g_HairNormalIntensity.value = findPositiveScalarParam(
      mat,
      ["g_HairNormalIntensity"],
      1.0
    );
  }
  if (u.g_HairMetallic) {
    u.g_HairMetallic.value = findScalarParam(mat, ["g_HairMetallic"], 0.15);
  }
  if (u.g_PriAlpha) {
    u.g_PriAlpha.value = findPositiveScalarParam(mat, ["g_PriAlpha"], 0.75);
  }
  if (u.g_SecAlpha) {
    u.g_SecAlpha.value = findPositiveScalarParam(mat, ["g_SecAlpha"], 0.55);
  }
  const priSpec = findVec3Param(
    mat,
    "g_PriSpecularColor",
    [0.55, 0.5, 0.68],
    true
  );
  if (u.g_PriSpecularColor) {
    u.g_PriSpecularColor.value.set(priSpec[0], priSpec[1], priSpec[2]);
  }
  if (u.g_PriRoughness) {
    u.g_PriRoughness.value = findPositiveScalarParam(
      mat,
      ["g_PriRoughness", "g_Layer1_Roughness"],
      0.28
    );
  }
  const secSpec = findVec3Param(
    mat,
    "g_SecSpecularColor",
    findVec3Param(mat, "g_Layer1_SheenColorMul", [0.38, 0.34, 0.52], true),
    true
  );
  if (u.g_SecSpecularColor) {
    u.g_SecSpecularColor.value.set(secSpec[0], secSpec[1], secSpec[2]);
  }
  if (u.g_SecRoughness) {
    u.g_SecRoughness.value = findPositiveScalarParam(
      mat,
      ["g_SecRoughness", "g_Layer2_Roughness"],
      0.42
    );
  }
  if (u.g_TransStrength) {
    u.g_TransStrength.value = findPositiveScalarParam(
      mat,
      ["g_TransStrength", "g_Layer1_SheenIntencity"],
      0.35
    );
  }
  if (u.g_SpecularPower) {
    u.g_SpecularPower.value = findPositiveScalarParam(
      mat,
      ["g_SpecularPower"],
      1.25
    );
  }
  if (u.g_HairOffset) {
    u.g_HairOffset.value = findScalarParam(mat, ["g_HairOffset"], 0.08);
  }
  const hairCol = findVec3Param(mat, "g_HairColor", [0.22, 0.18, 0.28], true);
  if (u.g_HairColor) {
    u.g_HairColor.value.set(hairCol[0], hairCol[1], hairCol[2]);
  }
  const layCol1 = findVec3Param(
    mat,
    "g_LayerColor1",
    findVec3Param(mat, "g_Layer1_BaseColor", [0.85, 0.82, 0.9], true),
    true
  );
  if (u.g_LayerColor1) {
    u.g_LayerColor1.value.set(layCol1[0], layCol1[1], layCol1[2]);
  }
  const layCol2 = findVec3Param(
    mat,
    "g_LayerColor2",
    findVec3Param(mat, "g_Layer2_BaseColor", [0.65, 0.6, 0.75], true),
    true
  );
  if (u.g_LayerColor2) {
    u.g_LayerColor2.value.set(layCol2[0], layCol2[1], layCol2[2]);
  }
  if (u.g_DiffuseIntensity) {
    u.g_DiffuseIntensity.value = findPositiveScalarParam(
      mat,
      ["g_DiffuseIntensity", "g_Layer1_DiffuseWeight"],
      1.0
    );
  }
  if (u.g_WrapDiffuseStrength) {
    u.g_WrapDiffuseStrength.value = findPositiveScalarParam(
      mat,
      ["g_WrapDiffuseStrength"],
      0.35
    );
  }
  if (u.g_NormalIntensity) {
    u.g_NormalIntensity.value = findPositiveScalarParam(
      mat,
      ["g_NormalIntensity"],
      0.5
    );
  }
  if (u.g_AmbientOcclusionMinMax) {
    const [aoMin, aoMax] = resolveMinMaxBounds(
      mat,
      "g_AmbientOcclusionMin",
      "g_AmbientOcclusionMax",
      0.15,
      1.0
    );
    u.g_AmbientOcclusionMinMax.value.set(aoMin, aoMax);
  }
  if (u.g_SpecularMaskMinMax) {
    const [smMin, smMax] = resolveMinMaxBounds(
      mat,
      "g_SpecularMaskMin",
      "g_SpecularMaskMax",
      0.15,
      1.0
    );
    u.g_SpecularMaskMinMax.value.set(smMin, smMax);
  }
  if (u.g_OcclusionMaskMinMax) {
    const [omMin, omMax] = resolveMinMaxBounds(
      mat,
      "g_OcclusionMaskMin",
      "g_OcclusionMaskMax",
      0.15,
      1.0
    );
    u.g_OcclusionMaskMinMax.value.set(omMin, omMax);
  }
  if (u.g_IridescenceStrength) {
    u.g_IridescenceStrength.value = findScalarParam(
      mat,
      ["g_IridescenceStrength"],
      0.0
    );
  }
  if (u.g_IridescenceCoefficient) {
    u.g_IridescenceCoefficient.value = findPositiveScalarParam(
      mat,
      ["g_IridescenceCoefficient"],
      1.2
    );
  }
  const iridCol = findVec3Param(mat, "g_Iridescence", [0.8, 0.55, 1.0], true);
  if (u.g_IridescenceColor) {
    u.g_IridescenceColor.value.set(iridCol[0], iridCol[1], iridCol[2]);
  }
  if (u.g_TilingMap1UV) {
    const t1u = findPositiveScalarParam(mat, ["g_TilingMap1U"], 1.0);
    const t1v = findPositiveScalarParam(mat, ["g_TilingMap1V", "g_TilingMap1U"], t1u);
    u.g_TilingMap1UV.value.set(t1u, t1v);
  }
  if (u.g_TilingMap2UV) {
    const t2u = findPositiveScalarParam(mat, ["g_TilingMap2U", "g_TilingMap2UV"], 1.0);
    const t2v = findPositiveScalarParam(mat, ["g_TilingMap2V", "g_TilingMap2U"], t2u);
    u.g_TilingMap2UV.value.set(t2u, t2v);
  }
  if (u.g_IridescenceStr) {
    u.g_IridescenceStr.value.set(
      findScalarParam(mat, ["g_IridescenceStrDiff"], 0.35),
      findScalarParam(mat, ["g_IridescenceStrPriSpec"], 0.65),
      findScalarParam(mat, ["g_IridescenceStrSecSpec"], 0.55)
    );
  }
  if (u.g_IridescenceFresnel) {
    const [ifMin, ifMax] = resolveMinMaxBounds(
      mat,
      "g_IridescenceFresnelMin",
      "g_IridescenceFresnelMax",
      0.05,
      1.0
    );
    u.g_IridescenceFresnel.value.set(
      findPositiveScalarParam(mat, ["g_IridescenceFresnelPow"], 2.0),
      ifMin,
      ifMax
    );
  }
  if (u.g_IridescenceCoord) {
    u.g_IridescenceCoord.value.set(
      findScalarParam(mat, ["g_IridescenceViewRate"], 1.0),
      findScalarParam(mat, ["g_IridescenceMapVRate"], 0.25),
      findPositiveScalarParam(mat, ["g_IridescenceVTile"], 1.0),
      findScalarParam(mat, ["g_IridescenceVOffset"], 0.0)
    );
  }

  // SKN20 / SKN21 / SKN25 / SKN30 / SKN31 parameters
  if (u.g_CurvatureScale) {
    u.g_CurvatureScale.value = findPositiveScalarParam(
      mat,
      ["g_CurvatureScale"],
      0.55
    );
  }
  if (u.g_SSSBias) {
    u.g_SSSBias.value = findPositiveScalarParam(mat, ["g_SSSBias"], 0.42);
  }
  if (u.g_BendNormalRate) {
    u.g_BendNormalRate.value = findPositiveScalarParam(
      mat,
      ["g_BendNormalRate"],
      0.28
    );
  }
  const sssColB3 = findVec3Param(mat, "g_SSSColor", [1.0, 0.38, 0.28], true);
  if (u.g_SSSColor) {
    u.g_SSSColor.value.set(sssColB3[0], sssColB3[1], sssColB3[2]);
  }
  if (u.g_SpecularShadowRate) {
    u.g_SpecularShadowRate.value = findPositiveScalarParam(
      mat,
      ["g_SpecularShadowRate"],
      0.35
    );
  }
  if (u.g_SpecularShadowClampMinMax) {
    const [ssMin, ssMax] = resolveMinMaxBounds(
      mat,
      "g_SpecularShadowClampMin",
      "g_SpecularShadowClampMax",
      0.2,
      0.85
    );
    u.g_SpecularShadowClampMinMax.value.set(ssMin, ssMax);
  }
  if (u.g_RamePower) {
    u.g_RamePower.value = findPositiveScalarParam(mat, ["g_RamePower"], 3.5);
  }
  if (u.g_RameLightStrength) {
    u.g_RameLightStrength.value = findScalarParam(
      mat,
      ["g_RameLightStrength"],
      0.0
    );
  }
  if (u.g_InnerRoughnessAdd) {
    u.g_InnerRoughnessAdd.value = findScalarParam(
      mat,
      ["g_InnerRoughnessAdd"],
      0.0
    );
  }
  if (u.g_OuterRoughnessAdd) {
    u.g_OuterRoughnessAdd.value = findScalarParam(
      mat,
      ["g_OuterRoughnessAdd"],
      0.0
    );
  }
  if (u.g_InnerAlbedoIntensity) {
    u.g_InnerAlbedoIntensity.value = findPositiveScalarParam(
      mat,
      ["g_InnerAlbedoIntensity"],
      0.85
    );
  }
  if (u.g_ParallaxDepth) {
    u.g_ParallaxDepth.value = findScalarParam(mat, ["g_ParallaxDepth"], -0.02);
  }
  if (u.g_ParallaxUVIntensity) {
    u.g_ParallaxUVIntensity.value.set(
      findPositiveScalarParam(mat, ["g_ParallaxUVIntensity_X"], 1.0),
      findPositiveScalarParam(mat, ["g_ParallaxUVIntensity_Y"], 1.0)
    );
  }
  if (u.g_AlbedoFresnelParams) {
    u.g_AlbedoFresnelParams.value.set(
      findPositiveScalarParam(mat, ["g_AlbedoFresnelIntensity"], 0.45),
      findPositiveScalarParam(mat, ["g_AlbedoFresnelExp"], 3.0),
      findScalarParam(mat, ["g_AlbedoFresnelBaseFrac"], 0.04)
    );
  }
  if (u.g_OuterFresnelMinMax) {
    const [ofMin, ofMax] = resolveMinMaxBounds(
      mat,
      "g_OuterFresnelMin",
      "g_OuterFresnelMax",
      0.05,
      1.0
    );
    u.g_OuterFresnelMinMax.value.set(ofMin, ofMax);
  }

  // Gls14 / Glass parameters
  const glassCol = findVec3Param(mat, "g_GlassColor", [0.78, 0.9, 1.0], true);
  if (u.g_GlassColor) {
    u.g_GlassColor.value.set(glassCol[0], glassCol[1], glassCol[2]);
  }
  if (u.g_SpaceDimensions) {
    u.g_SpaceDimensions.value.set(
      findPositiveScalarParam(mat, ["g_SpaceWidth"], 1.5),
      findPositiveScalarParam(mat, ["g_SpaceHeight"], 1.5),
      findPositiveScalarParam(mat, ["g_SpaceDepth"], 1.5)
    );
  }
  if (u.g_DivisionNum) {
    u.g_DivisionNum.value.set(
      findPositiveScalarParam(mat, ["g_DivisionNum_x"], 4.0),
      findPositiveScalarParam(mat, ["g_DivisionNum_y"], 4.0)
    );
  }
  if (u.g_InteriorBlendRate) {
    u.g_InteriorBlendRate.value = findPositiveScalarParam(
      mat,
      ["g_InteriorBlendRate"],
      0.55
    );
  }
  if (u.g_inLightIntensity) {
    u.g_inLightIntensity.value = findPositiveScalarParam(
      mat,
      ["g_inLightIntensity"],
      0.85
    );
  }

  // HZR00 / Ground parameters
  if (u.g_RockMetallic) {
    u.g_RockMetallic.value = findScalarParam(mat, ["g_RockMetallic"], 0.05);
  }
  if (u.g_RockSpecIntensity) {
    u.g_RockSpecIntensity.value = findPositiveScalarParam(
      mat,
      ["g_RockSpecIntensity"],
      0.6
    );
  }
  if (u.g_SandRoughness) {
    u.g_SandRoughness.value = findPositiveScalarParam(
      mat,
      ["g_SandRoughness"],
      0.72
    );
  }
  if (u.g_SandMetallic) {
    u.g_SandMetallic.value = findScalarParam(mat, ["g_SandMetallic"], 0.02);
  }
  if (u.g_SandSpecIntensity) {
    u.g_SandSpecIntensity.value = findPositiveScalarParam(
      mat,
      ["g_SandSpecIntensity"],
      0.45
    );
  }
  if (u.g_BlendThresholdMinMax) {
    const [bMin, bMax] = resolveMinMaxBounds(
      mat,
      "g_BlendThresholdMin",
      "g_BlendThresholdMax",
      0.25,
      0.75
    );
    u.g_BlendThresholdMinMax.value.set(bMin, bMax);
  }

  // JWL00 / JWL01 parameters
  if (u.g_RefractionBias) {
    u.g_RefractionBias.value = findPositiveScalarParam(
      mat,
      ["g_RefractionBias"],
      0.45
    );
  }
  if (u.g_Height1) {
    u.g_Height1.value = findScalarParam(mat, ["g_Height1"], 0.04);
  }
  if (u.g_Height2) {
    u.g_Height2.value = findScalarParam(mat, ["g_Height2"], 0.09);
  }
  if (u.g_FresnelMinMax) {
    const [fMin, fMax] = resolveMinMaxBounds(
      mat,
      "g_FresnelMin",
      "g_FresnelMax",
      0.1,
      0.9
    );
    u.g_FresnelMinMax.value.set(fMin, fMax);
  }
  const fresCol = findVec3Param(mat, "g_FresnelColor", [0.65, 0.85, 1.0], true);
  if (u.g_FresnelColor) {
    u.g_FresnelColor.value.set(fresCol[0], fresCol[1], fresCol[2]);
  }
  if (u.g_EmissiveRoughnessParams) {
    u.g_EmissiveRoughnessParams.value.set(
      findPositiveScalarParam(mat, ["g_EmissiveRoughnessBegin"], 0.2),
      findPositiveScalarParam(mat, ["g_EmissiveRoughnessContrast"], 0.5),
      findPositiveScalarParam(mat, ["g_EmissiveRoughnessBlend"], 0.65)
    );
  }
}

function nonZeroVec2(v: number[], defX: number, defY: number): THREE.Vector2 {
  const x = v[0] ?? defX;
  const y = v[1] ?? defY;
  if (Math.abs(x) < 1e-6 && Math.abs(y) < 1e-6) {
    return new THREE.Vector2(defX, defY);
  }
  return new THREE.Vector2(x, y);
}

function populateB1UniformValues(
  u: Record<string, THREE.IUniform>,
  mat: MaterialDesc
): void {
  const c1Tile = nonZeroVec2(findParamValues(mat, "Color_1_Tile", [1, 1]), 1, 1);
  const c1Off = findParamValues(mat, "Color_1_Offset", [0, 0]);
  const nmTile = nonZeroVec2(findParamValues(mat, "tile", [1, 1]), 1, 1);
  if (u.Color_1_Tile) u.Color_1_Tile.value.set(c1Tile.x, c1Tile.y);
  if (u.Color_1_Offset) u.Color_1_Offset.value.set(c1Off[0] ?? 0, c1Off[1] ?? 0);
  if (u.tile) u.tile.value.set(nmTile.x, nmTile.y);

  const depthVal = findParamValues(mat, "depth", [1.0])[0] ?? 1.0;
  if (u.depth) u.depth.value = depthVal;

  const blendRaw = findParamValues(mat, "blend", [0.85])[0] ?? 0.85;
  const blendVal = Math.abs(blendRaw) < 1e-6 ? 0.85 : blendRaw;
  if (u.blend) u.blend.value = blendVal;

  const envAmbParam = findParamValues(mat, "Env_Amb_Param", [0.5])[0] ?? 0.5;
  if (u.Env_Amb_Param) u.Env_Amb_Param.value = envAmbParam;

  const envFactor = findParamValues(mat, "env_factor", [0.0])[0] ?? 0.0;
  if (u.env_factor) u.env_factor.value = envFactor;

  const specVec = findParamValues(mat, "specular", [0.8, 0.8, 0.8]);
  if (u.specular) {
    u.specular.value.set(
      specVec[0] ?? 0.8,
      specVec[1] ?? 0.8,
      specVec[2] ?? 0.8
    );
  }

  const speLight = findParamValues(mat, "spelightpos", [5.0, 0.0, 10.0, 10000.0]);
  if (u.spelightpos) {
    u.spelightpos.value.set(
      speLight[0] ?? 5.0,
      speLight[1] ?? 0.0,
      speLight[2] ?? 10.0,
      speLight[3] ?? 10000.0
    );
  }

  const specType = findParamValues(mat, "Spec_Type", [0.25])[0] ?? 0.25;
  if (u.Spec_Type) u.Spec_Type.value = specType;

  const specLimit =
    findParamValues(
      mat,
      "Spec_Limit",
      mat.materialType === 151 ? [95.0] : [0.5]
    )[0] ?? (mat.materialType === 151 ? 95.0 : 0.5);
  if (u.Spec_Limit) u.Spec_Limit.value = specLimit;

  const shineVal = findParamValues(mat, "shine", [0.4])[0] ?? 0.4;
  if (u.shine) u.shine.value = shineVal;

  const biasRaw = findParamValues(mat, "bias", [1.0])[0] ?? 1.0;
  const biasVal = Math.abs(biasRaw) < 1e-6 ? 1.0 : biasRaw;
  if (u.bias) u.bias.value = biasVal;

  const diffVec = findParamValues(mat, "diffuse", [1.0, 1.0, 1.0]);
  if (u.uDiffuseParam) {
    u.uDiffuseParam.value.set(
      diffVec[0] ?? 1.0,
      diffVec[1] ?? 1.0,
      diffVec[2] ?? 1.0
    );
  }

  const ambVec = findParamValues(mat, "ambient", [0.2, 0.2, 0.2]);
  if (u.uAmbientParam) {
    u.uAmbientParam.value.set(
      ambVec[0] ?? 0.2,
      ambVec[1] ?? 0.2,
      ambVec[2] ?? 0.2
    );
  }

  const sssColVec = findParamValues(mat, "SSS_color", [1.0, 1.0, 1.0]);
  if (u.SSS_color) {
    u.SSS_color.value.set(
      sssColVec[0] ?? 1.0,
      sssColVec[1] ?? 1.0,
      sssColVec[2] ?? 1.0
    );
  }
  const sssVal = findParamValues(mat, "SSS_value", [1.0])[0] ?? 1.0;
  if (u.SSS_value) u.SSS_value.value = sssVal;
  const sssFlag = findParamValues(mat, "SSS_flag", [1.0])[0] ?? 1.0;
  if (u.SSS_flag) u.SSS_flag.value = sssFlag;
  const sssNormVec = findParamValues(mat, "SSS_norm", [0.0, 1.0]);
  if (u.SSS_norm) {
    u.SSS_norm.value.set(sssNormVec[0] ?? 0.0, sssNormVec[1] ?? 1.0);
  }

  const dotMinVal = findParamValues(mat, "dot_min", [0.0])[0] ?? 0.0;
  if (u.dot_min) u.dot_min.value = dotMinVal;
  const dotMaxVal = findParamValues(mat, "dot_max", [0.5])[0] ?? 0.5;
  if (u.dot_max) u.dot_max.value = dotMaxVal;
  const dotColVec = findParamValues(mat, "dot_color", [0.0, 0.0, 0.0]);
  if (u.dot_color) {
    u.dot_color.value.set(
      dotColVec[0] ?? 0.0,
      dotColVec[1] ?? 0.0,
      dotColVec[2] ?? 0.0
    );
  }

  const lp1 = findParamValues(mat, "lightpos", [0, 0, 0, 1]);
  if (u.lightpos) {
    u.lightpos.value.set(lp1[0] ?? 0, lp1[1] ?? 0, lp1[2] ?? 0, lp1[3] ?? 1);
  }
  const lc1 = findParamValues(mat, "light_col", [0, 0, 0]);
  if (u.light_col) {
    u.light_col.value.set(lc1[0] ?? 0, lc1[1] ?? 0, lc1[2] ?? 0);
  }
  const lp2 = findParamValues(mat, "lightpos2", [0, 0, 0, 1]);
  if (u.lightpos2) {
    u.lightpos2.value.set(lp2[0] ?? 0, lp2[1] ?? 0, lp2[2] ?? 0, lp2[3] ?? 1);
  }
  const lc2 = findParamValues(mat, "light_col2", [0, 0, 0]);
  if (u.light_col2) {
    u.light_col2.value.set(lc2[0] ?? 0, lc2[1] ?? 0, lc2[2] ?? 0);
  }

  const b23Color = mat.params.find((p) => p.isColor && p.values.length >= 3);
  if (b23Color && u.uBaseTint) {
    u.uBaseTint.value.set(
      b23Color.values[0] ?? 1,
      b23Color.values[1] ?? 1,
      b23Color.values[2] ?? 1,
      b23Color.values[3] !== undefined && b23Color.values[3] > 0
        ? b23Color.values[3]
        : 1
    );
  }
}

export function createInitialUniforms(
  mat: MaterialDesc,
  resolvedSpec: ResolvedShaderSpec,
  defaultTex2D: THREE.Texture,
  defaultNormal2D: THREE.Texture,
  defaultSpecLut2D: THREE.Texture,
  defaultCube: THREE.CubeTexture,
  defaultSoftCube?: THREE.CubeTexture
): Record<string, THREE.IUniform> {
  const hasNormalSlot = mat.textureSlots.some((s) => s.role === "normal");
  const irradCube = defaultSoftCube ?? defaultCube;

  const uniforms: Record<string, THREE.IUniform> = {
    uSelectionHighlight: { value: 0.0 },
    uDebugMode: { value: 0 },
    uDebugChannel: { value: 0 },
    uDebugIsCube: { value: 0 },
    uDebugUvMode: { value: 0 },
    uDebugUvTile: { value: new THREE.Vector2(1, 1) },
    uUseSecondaryNormals: { value: 0 },
    uDebugTex2D: { value: defaultTex2D },
    uDebugTexCube: { value: defaultCube },

    Color_1_sampler: { value: defaultTex2D },
    reliefmap_sampler: { value: defaultNormal2D },
    envmap_sampler: { value: defaultCube },
    env_amb_sampler: { value: irradCube },
    Spec_Pow_sampler: { value: defaultSpecLut2D },
    Spec_Mask_sampler: { value: defaultTex2D },
    lightmap_sampler: { value: defaultTex2D },

    uHasNormalMap: { value: hasNormalSlot ? 1 : 0 },
    uBaseTint: { value: new THREE.Vector4(1, 1, 1, 1) },

    Color_1_Tile: { value: new THREE.Vector2(1, 1) },
    Color_1_Offset: { value: new THREE.Vector2(0, 0) },
    tile: { value: new THREE.Vector2(1, 1) },
    depth: { value: 1.0 },
    blend: { value: 0.85 },
    Env_Amb_Param: { value: 0.5 },
    env_factor: { value: 0.0 },
    specular: { value: new THREE.Vector3(0.8, 0.8, 0.8) },
    spelightpos: { value: new THREE.Vector4(5.0, 0.0, 10.0, 10000.0) },
    Spec_Type: { value: 0.25 },
    Spec_Limit: { value: mat.materialType === 151 ? 95.0 : 0.5 },
    shine: { value: 0.4 },
    bias: { value: 1.0 },
    uDiffuseParam: { value: new THREE.Vector3(1.0, 1.0, 1.0) },
    uAmbientParam: { value: new THREE.Vector3(0.2, 0.2, 0.2) },
    SSS_color: { value: new THREE.Vector3(1.0, 1.0, 1.0) },
    SSS_value: { value: 1.0 },
    SSS_flag: { value: 1.0 },
    SSS_norm: { value: new THREE.Vector2(0.0, 1.0) },
    dot_min: { value: 0.0 },
    dot_max: { value: 0.5 },
    dot_color: { value: new THREE.Vector3(0.0, 0.0, 0.0) },
    lightpos: { value: new THREE.Vector4(0, 0, 0, 1) },
    light_col: { value: new THREE.Vector3(0, 0, 0) },
    lightpos2: { value: new THREE.Vector4(0, 0, 0, 1) },
    light_col2: { value: new THREE.Vector3(0, 0, 0) },
    uEnvUseNormal: { value: resolvedSpec.envUseNormal ? 1 : 0 },
    uAlphaBlendVariant: { value: resolvedSpec.alphaBlendVariant ? 1 : 0 },
    uUseVertexAlpha: { value: resolvedSpec.useVertexAlpha ? 1 : 0 },

    uB2AlbedoMap: { value: defaultTex2D },
    uB2NormalMap: { value: defaultNormal2D },
    uB2RssMap: { value: defaultTex2D },
    uB2CubeMap: { value: defaultCube },
    uB2MultipleLut: { value: defaultSpecLut2D },
    uB2HasAlbedoMap: { value: 0 },
    uB2HasNormalMap: { value: 0 },
    uB2HasRssMap: { value: 0 },
    uB2HasCubeMap: { value: 0 },
    uB2AlphaClip: { value: 0 },
    uB2UvTile: { value: new THREE.Vector2(1, 1) },
    uB2UvOffset: { value: new THREE.Vector2(0, 0) },
    uB2BaseColor: { value: new THREE.Vector4(1, 1, 1, 1) },
    uB2SpecularType: { value: 0.375 },
    uB2SpecularPower: { value: 0.8 },
    uB2SpecularStrength: { value: 15.0 },
    uB2CubemapStrength: { value: 15.0 },
    uB2ShadowColor: { value: new THREE.Vector3(0.5, 0.5, 0.5) },

    uTexAlbedo: { value: defaultTex2D },
    uTexMask: { value: defaultTex2D },
    uTexNormal: { value: defaultNormal2D },
    uTexNormal2: { value: defaultNormal2D },
    uTexNormal3: { value: defaultNormal2D },
    uTexIndex: { value: defaultTex2D },
    uTexEnv: { value: defaultCube },
    uTexIrradiance: { value: irradCube },
    uHasTexAlbedo: { value: 0 },
    uHasTexMask: { value: 0 },
    uHasTexNormal: { value: 0 },
    uHasTexNormal2: { value: 0 },
    uHasTexNormal3: { value: 0 },
    uHasTexIndex: { value: 0 },
    uB3PbsVariant: { value: resolvedSpec.pbsVariant ?? 0 },
    uB3PbsHasDFlag: { value: resolvedSpec.pbsHasDFlag ?? 0 },
    uB3HairVariant: { value: resolvedSpec.hairVariant ?? 20 },
    uB3SkinVariant: { value: resolvedSpec.skinVariant ?? 20 },
    uB3GlassVariant: { value: resolvedSpec.glassVariant ?? 2 },

    g_UseNormalMap: { value: hasNormalSlot ? 1.0 : 0.0 },
    g_bAlbedoOverWrite: { value: 0.0 },
    g_bRoughnessOverWrite: { value: 0.0 },
    g_bRoughnessOverWrite2: { value: 0.0 },
    g_bMetalicOverWrite: { value: 0.0 },
    g_1BitMask: { value: 0.0 },
    g_OnlyLayer1: { value: 1.0 },
    g_EyeBlowMode: { value: 0.0 },
    g_UseAlphaEffect: { value: 0.0 },
    g_Layer1_UseNormalMap: { value: 1.0 },
    g_Layer1_UseSpecularMask: { value: 1.0 },
    g_Tile: { value: new THREE.Vector2(1.0, 1.0) },
    g_Tile2: { value: new THREE.Vector2(1.0, 1.0) },
    g_DetailNormalTile: { value: new THREE.Vector2(1.0, 1.0) },
    g_UseDetailNormalMap: { value: 0.0 },
    g_DetailNormalIntensity: { value: 0.5 },
    g_AlbedoAMetallic: { value: 0.0 },
    g_SpecRefRate: { value: 1.0 },
    g_UseOcclusionMap: { value: 1.0 },
    g_UV2Use: { value: 0.0 },
    g_AlbedoColor: { value: new THREE.Vector3(1.0, 1.0, 1.0) },
    g_AlbedoColor2: { value: new THREE.Vector3(0.0, 0.0, 0.0) },
    g_EmissiveIntensity: { value: 0.0 },
    g_EmissiveMaskIntensity: { value: 1.0 },
    g_EmissiveColor: { value: new THREE.Vector3(1.0, 1.0, 1.0) },
    g_bUseEmissiveColor: { value: 0.0 },
    g_Roughness: { value: 0.42 },
    g_Metallic: { value: 0.0 },
    g_RoughnessMinMax: { value: new THREE.Vector2(0.05, 0.95) },
    g_MetalMinMax: { value: new THREE.Vector2(0.0, 1.0) },
    g_ClearCoatRate: { value: 0.5 },
    g_ClearCoatFresnel: { value: 1.5 },
    g_ClearCoatFresnelMinMax: { value: new THREE.Vector2(0.06, 1.0) },
    g_Roughness2: { value: 0.12 },
    g_BottomAlbedoColor: { value: new THREE.Vector3(0.0, 0.0, 0.0) },
    g_MazioraParams: { value: new THREE.Vector3(2.2, 1.0, 0.55) },

    g_HairNormalIntensity: { value: 1.0 },
    g_HairMetallic: { value: 0.15 },
    g_PriAlpha: { value: 0.75 },
    g_SecAlpha: { value: 0.55 },
    g_PriSpecularColor: { value: new THREE.Vector3(0.55, 0.50, 0.68) },
    g_PriRoughness: { value: 0.28 },
    g_SecSpecularColor: { value: new THREE.Vector3(0.38, 0.34, 0.52) },
    g_SecRoughness: { value: 0.42 },
    g_TransStrength: { value: 0.35 },
    g_SpecularPower: { value: 1.25 },
    g_HairOffset: { value: 0.08 },
    g_HairColor: { value: new THREE.Vector3(0.22, 0.18, 0.28) },
    g_LayerColor1: { value: new THREE.Vector3(0.85, 0.82, 0.90) },
    g_LayerColor2: { value: new THREE.Vector3(0.65, 0.60, 0.75) },
    g_DiffuseIntensity: { value: 1.0 },
    g_WrapDiffuseStrength: { value: 0.35 },
    g_NormalIntensity: { value: 0.5 },
    g_AmbientOcclusionMinMax: { value: new THREE.Vector2(0.15, 1.0) },
    g_SpecularMaskMinMax: { value: new THREE.Vector2(0.15, 1.0) },
    g_OcclusionMaskMinMax: { value: new THREE.Vector2(0.15, 1.0) },
    g_IridescenceStrength: { value: 0.0 },
    g_IridescenceCoefficient: { value: 1.2 },
    g_IridescenceColor: { value: new THREE.Vector3(0.8, 0.55, 1.0) },
    g_TilingMap1UV: { value: new THREE.Vector2(1.0, 1.0) },
    g_TilingMap2UV: { value: new THREE.Vector2(1.0, 1.0) },
    g_IridescenceStr: { value: new THREE.Vector3(0.35, 0.65, 0.55) },
    g_IridescenceFresnel: { value: new THREE.Vector3(2.0, 0.05, 1.0) },
    g_IridescenceCoord: { value: new THREE.Vector4(1.0, 0.25, 1.0, 0.0) },

    g_CurvatureScale: { value: 0.55 },
    g_SSSBias: { value: 0.42 },
    g_BendNormalRate: { value: 0.28 },
    g_SSSColor: { value: new THREE.Vector3(1.0, 0.38, 0.28) },
    g_SpecularShadowRate: { value: 0.35 },
    g_SpecularShadowClampMinMax: { value: new THREE.Vector2(0.2, 0.85) },
    g_RamePower: { value: 3.5 },
    g_RameLightStrength: { value: 0.0 },
    g_InnerRoughnessAdd: { value: 0.0 },
    g_OuterRoughnessAdd: { value: 0.0 },
    g_InnerAlbedoIntensity: { value: 0.85 },
    g_ParallaxDepth: { value: -0.02 },
    g_ParallaxUVIntensity: { value: new THREE.Vector2(1.0, 1.0) },
    g_AlbedoFresnelParams: { value: new THREE.Vector3(0.45, 3.0, 0.04) },
    g_OuterFresnelMinMax: { value: new THREE.Vector2(0.05, 1.0) },

    g_GlassColor: { value: new THREE.Vector3(0.78, 0.9, 1.0) },
    g_SpaceDimensions: { value: new THREE.Vector3(1.5, 1.5, 1.5) },
    g_DivisionNum: { value: new THREE.Vector2(4.0, 4.0) },
    g_InteriorBlendRate: { value: 0.55 },
    g_inLightIntensity: { value: 0.85 },

    g_RockMetallic: { value: 0.05 },
    g_RockSpecIntensity: { value: 0.6 },
    g_SandRoughness: { value: 0.72 },
    g_SandMetallic: { value: 0.02 },
    g_SandSpecIntensity: { value: 0.45 },
    g_BlendThresholdMinMax: { value: new THREE.Vector2(0.25, 0.75) },

    g_RefractionBias: { value: 0.45 },
    g_Height1: { value: 0.04 },
    g_Height2: { value: 0.09 },
    g_FresnelMinMax: { value: new THREE.Vector2(0.1, 0.9) },
    g_FresnelColor: { value: new THREE.Vector3(0.65, 0.85, 1.0) },
    g_EmissiveRoughnessParams: { value: new THREE.Vector3(0.2, 0.5, 0.65) },
  };

  populateB1UniformValues(uniforms, mat);
  populateB2AndB3UniformValues(uniforms, mat, resolvedSpec);

  return uniforms;
}

export function updateMaterialUniformsInPlace(
  shaderMat: THREE.ShaderMaterial,
  mat: MaterialDesc
): void {
  const u = shaderMat.uniforms;
  if (!u) return;
  populateB1UniformValues(u, mat);
  const resolvedSpec = getShader(mat.game, mat.shaderName, mat.materialType);
  populateB2AndB3UniformValues(u, mat, resolvedSpec);
}

/**
 * Resolves Bayonetta 3 `g_BlendMode` (0 = opaque, 1 = transparent, 2 = modulate/multiply,
 * 3 = subtract, 4 = add) and `g_LocalSortNo` (`PG2[3]`) for Three.js material state & draw order.
 */
export function resolveB3BlendAndSort(
  mat: MaterialDesc | undefined,
  spec: ResolvedShaderSpec
): {
  blending: THREE.Blending;
  transparent: boolean;
  depthWrite: boolean;
  queue: RenderQueueBucket;
  localSortNo: number;
} {
  let blending: THREE.Blending = THREE.NormalBlending;
  let transparent = spec.transparent;
  let depthWrite = spec.depthWrite;
  let queue: RenderQueueBucket = spec.queue;
  let localSortNo = 0;

  if (mat && mat.game === "b3") {
    const rawSort = findScalarParam(mat, ["g_LocalSortNo", "g_SortNo"], 0.0);
    if (Number.isFinite(rawSort)) {
      localSortNo = Math.max(-32, Math.min(32, Math.round(rawSort)));
    }
    const rawBlend = findScalarParam(mat, ["g_BlendMode"], NaN);
    if (Number.isFinite(rawBlend)) {
      const bm = Math.round(rawBlend);
      // Apply explicit g_BlendMode 0..4 (note: Eye02/HMEYE use 7 for internal eye pass, keep spec default for >4)
      if (bm === 0) {
        blending = THREE.NormalBlending;
        transparent = false;
        depthWrite = true;
        queue = spec.queue === "alphatest" ? "alphatest" : "opaque";
      } else if (bm === 1) {
        blending = THREE.NormalBlending;
        transparent = true;
        depthWrite = false;
        queue = "blend";
      } else if (bm === 2) {
        blending = THREE.MultiplyBlending;
        transparent = true;
        depthWrite = false;
        queue = "blend";
      } else if (bm === 3) {
        blending = THREE.SubtractiveBlending;
        transparent = true;
        depthWrite = false;
        queue = "blend";
      } else if (bm === 4) {
        blending = THREE.AdditiveBlending;
        transparent = true;
        depthWrite = false;
        queue = "blend";
      }
    }
  }

  return { blending, transparent, depthWrite, queue, localSortNo };
}
