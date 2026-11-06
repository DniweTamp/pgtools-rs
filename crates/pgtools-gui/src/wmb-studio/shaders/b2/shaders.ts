// [wmb-studio BEGIN phase3]
import { MaterialDesc, TextureSlotDesc } from "../../contract";
import { STUDIO_FRAGMENT_HEADER, STUDIO_VERTEX_SHADER } from "../common";
import { RenderQueueBucket } from "../b1/shaders";

export interface B2SamplerSlotBinding {
  albedo?: TextureSlotDesc;
  normal?: TextureSlotDesc;
  rss?: TextureSlotDesc;
  cubemap?: TextureSlotDesc;
  lut?: TextureSlotDesc;
}

/**
 * Resolves the sampler slots for a Bayonetta 2 material (`Bayo2.hlsl` & `b2_shader_layouts.json`):
 * - `FLAG1_TEXTURE1` -> `albedo` (slot 0 or first `"albedo"` slot)
 * - `FLAG1_TEXTURE2` -> `normal` (first `"normal"` slot, typically slot 1 or 2)
 * - `FLAG1_TEXTURE3` -> `rss` (`RSSTexture`: `.g = CubemapMask`, `.b = SpecularMask`; first `"spec"` or non-lightmap `"mask"` slot)
 * - `cubemap` -> first `"env"` slot (optional; falls back to studio cubemap when `CubemapMask > 0`)
 * - `lut` -> first `"lut"` slot (optional; falls back to `specLut2D` / `pre_pow`)
 */
export function resolveB2SamplerSlots(mat: MaterialDesc): B2SamplerSlotBinding {
  const slots = mat.textureSlots;
  const albedo =
    slots.find((s) => s.role === "albedo") ??
    slots.find((s) => s.slot === 0) ??
    slots[0];

  const normal = slots.find(
    (s) =>
      s.role === "normal" ||
      s.name.toLowerCase().includes("normal") ||
      s.name.toLowerCase().includes("relief")
  );

  const rss = slots.find((s) => {
    if (s === albedo || s === normal) return false;
    const u = s.textureId >>> 0;
    const low = s.name.toLowerCase();
    // Exclude neutral white pre_lightmap (0x4E9C16F4 / 0x5864A24F) on Skn02/Skn03/Bgs* so it doesn't force CubemapMask=1 & SpecularMask=1
    const isLightmapSlot =
      u === 0x4e9c16f4 ||
      u === 0x5864a24f ||
      low.includes("lightmap");
    if (isLightmapSlot) return false;
    return (
      s.role === "spec" ||
      s.role === "mask" ||
      low.includes("rss") ||
      low.includes("mask") ||
      low.includes("spec")
    );
  });

  const cubemap = slots.find(
    (s) =>
      s.role === "env" ||
      s.name.toLowerCase().includes("cube") ||
      s.name.toLowerCase().includes("env")
  );

  const lut = slots.find(
    (s) =>
      s.role === "lut" ||
      s.name.toLowerCase().includes("lut") ||
      s.name.toLowerCase().includes("pow")
  );

  return {
    albedo,
    normal,
    rss,
    cubemap,
    lut,
  };
}

/**
 * Direct translation of `Bayo2.hlsl` into Three.js GLSL for the Bayonetta 2 viewport.
 *
 * Key equations from `Bayo2.hlsl`:
 * - `AlbedoTexture = vec4(1, 1, 1, 1)` if `FLAG1_TEXTURE1` is unset, else `texture2D(uAlbedoMap, uv)`
 * - `RSSTexture = vec4(1, 0, 0, 1)` if `FLAG1_TEXTURE3` is unset, else `texture2D(uRssMap, uv)`
 *   (`CubemapMask = RSSTexture.g`, `SpecularMask = RSSTexture.b`)
 * - `NormalTexture = vec4(0.5, 0.5, 1, 1)` if `FLAG1_TEXTURE2` is unset, else `texture2D(uNormalMap, uv)`
 *   (Normal Z is always reconstructed from XY via `PerturbNormal`)
 * - `SpecularType = 0.375`, `SpecularPower = 0.8`, `SpecularStrength = 15.0`, `CubemapStrength = 15.0`,
 *   `ShadowColor = vec3(0.5, 0.5, 0.5)`
 * - `H = normalize(L + viewDir); specular = pow(saturate(dot(N, H)), 8.0);`
 * - `L' = normalize(vec3(-L.x, L.y, -L.z)); H' = normalize(L' + viewDir); specular += pow(saturate(dot(N, H')), 8.0);`
 * - `fakeAnisotropic = texture2D(uMultipleLut, vec2(clamp(specular, 0.0, 1.0), SpecularType)).rgb * SpecularStrength * SpecularMask;`
 * - `CubemapReflection = CubemapTexture.rgb * CubemapStrength * CubemapMask;`
 * - `color.rgb *= 1.0 + fakeAnisotropic; color.rgb *= 1.0 + CubemapReflection;`
 */
export const B2_GENERIC_FRAGMENT_SHADER = /* glsl */ `
${STUDIO_FRAGMENT_HEADER}

uniform sampler2D uB2AlbedoMap;
uniform sampler2D uB2NormalMap;
uniform sampler2D uB2RssMap;
uniform samplerCube uB2CubeMap;
uniform sampler2D uB2MultipleLut;

uniform int uB2HasAlbedoMap;
uniform int uB2HasNormalMap;
uniform int uB2HasRssMap;
uniform int uB2HasCubeMap;
uniform int uB2AlphaClip;

uniform vec2 uB2UvTile;
uniform vec2 uB2UvOffset;
uniform vec4 uB2BaseColor;
uniform float uB2SpecularType;
uniform float uB2SpecularPower;
uniform float uB2SpecularStrength;
uniform float uB2CubemapStrength;
uniform vec3 uB2ShadowColor;

void main() {
  vec2 uv = vUv.xy * uB2UvTile + uB2UvOffset;

  // 1. AlbedoTexture (FLAG1_TEXTURE1)
  vec4 AlbedoTexture = vec4(1.0, 1.0, 1.0, 1.0);
  if (uB2HasAlbedoMap == 1) {
    AlbedoTexture = texture2D(uB2AlbedoMap, uv);
  }
  AlbedoTexture *= uB2BaseColor;

  if (uDebugMode == 0 && uB2AlphaClip == 1 && AlbedoTexture.a <= 0.3333) {
    discard;
  }

  // 2. RSSTexture (FLAG1_TEXTURE3): .g = CubemapMask, .b = SpecularMask
  vec4 RSSTexture = vec4(1.0, 0.0, 0.0, 1.0);
  if (uB2HasRssMap == 1) {
    RSSTexture = texture2D(uB2RssMap, uv);
  }

  // 3. NormalTexture (FLAG1_TEXTURE2): always reconstructs Z from XY via PerturbNormal
  vec4 NormalTexture = vec4(0.5, 0.5, 1.0, 1.0);
  vec3 Nw = normalize(vNrmW);
  vec3 Nv = normalize(vNrmV);
  if (uB2HasNormalMap == 1) {
    NormalTexture = texture2D(uB2NormalMap, uv);
    Nw = normalize(PerturbNormal(NormalTexture, 1.0, vTanW, Nw));
    Nv = normalize(PerturbNormal(NormalTexture, 1.0, normalize(vTanV), Nv));
  }

  // 4. Bayo2.hlsl Constants & Masks
  float CubemapMask = RSSTexture.g;
  float SpecularMask = RSSTexture.b;
  float SpecularType = uB2SpecularType;
  float SpecularPower = uB2SpecularPower;
  float SpecularStrength = uB2SpecularStrength;
  float CubemapStrength = uB2CubemapStrength;
  vec3 ShadowColor = uB2ShadowColor;

  // 5. Primary & Mirrored Directional Light Specular (Bayo2.hlsl lines 27-38)
  vec3 viewDir = normalize(-vEyeW);
  vec3 L = normalize(STUDIO_KEY_TO_LIGHT);
  vec3 H = normalize(L + viewDir);
  float specular = pow(saturate(dot(Nw, H)), 8.0 * max(SpecularPower, 0.125));

  vec3 L2 = normalize(vec3(-L.x, L.y, -L.z));
  vec3 H2 = normalize(L2 + viewDir);
  specular += pow(saturate(dot(Nw, H2)), 8.0 * max(SpecularPower, 0.125));

  // 6. Fake Anisotropic LUT Sample & Cubemap Reflection (Bayo2.hlsl lines 39-43)
  vec3 fakeAnisotropic =
    texture2D(uB2MultipleLut, vec2(clamp(specular, 0.0, 1.0), SpecularType)).rgb *
    SpecularStrength *
    SpecularMask;

  vec4 CubemapTexture = vec4(0.0, 0.0, 0.0, 1.0);
  if (uB2HasCubeMap == 1 || CubemapMask > 0.001) {
    vec3 R = reflect(-viewDir, Nw);
    CubemapTexture = vec4(texCUBE_game(uB2CubeMap, R).rgb * 0.08, 1.0);
  }
  vec3 CubemapReflection = CubemapTexture.rgb * CubemapStrength * CubemapMask;

  // 7. Diffuse Shading & Multiplicative Energy Combine
  float ndotl = saturate(dot(Nw, STUDIO_KEY_TO_LIGHT)) * 0.75 +
                saturate(dot(Nw, STUDIO_FILL_TO_LIGHT)) * 0.25;
  vec3 diffuseShade = mix(ShadowColor, vec3(1.0), smoothstep(0.0, 0.85, ndotl + 0.2));

  vec4 color = vec4(AlbedoTexture.rgb * diffuseShade, AlbedoTexture.a);
  color.rgb *= (1.0 + fakeAnisotropic);
  color.rgb *= (1.0 + CubemapReflection);

  gl_FragColor = ApplyStudioPost(color, Nw, Nv, vPosV);
}
`;

export function classifyB2Queue(
  shaderName: string | null,
  materialType: number | null
): { queue: RenderQueueBucket; alphaClip: boolean } {
  const clean = (shaderName ?? "").trim().toLowerCase();
  if (
    clean.startsWith("gla") ||
    clean.startsWith("gls") ||
    clean.startsWith("wtr") ||
    clean.startsWith("vfx") ||
    clean.startsWith("eff") ||
    clean.startsWith("fod") ||
    clean.startsWith("eye01") ||
    materialType === 60 ||
    materialType === 88 ||
    materialType === 104 ||
    materialType === 184
  ) {
    return { queue: "blend", alphaClip: false };
  }
  if (
    clean.startsWith("alp") ||
    clean.startsWith("har") ||
    clean.includes("_a")
  ) {
    return { queue: "alphatest", alphaClip: true };
  }
  return { queue: "opaque", alphaClip: false };
}

export function getB2ShaderSpec(
  shaderName: string | null,
  materialType: number | null
): {
  id: string;
  vertexShader: string;
  fragmentShader: string;
  queue: RenderQueueBucket;
  transparent: boolean;
  depthWrite: boolean;
  isFallback: boolean;
  envUseNormal: boolean;
  alphaBlendVariant: boolean;
} {
  const { queue, alphaClip } = classifyB2Queue(shaderName, materialType);
  const clean = (shaderName ?? "").trim();
  return {
    id: clean.length > 0 ? `b2:${clean}` : "b2:generic",
    vertexShader: STUDIO_VERTEX_SHADER,
    fragmentShader: B2_GENERIC_FRAGMENT_SHADER,
    queue,
    transparent: queue === "blend",
    depthWrite: queue !== "blend",
    isFallback: false,
    envUseNormal: true,
    alphaBlendVariant: alphaClip || queue === "blend",
  };
}
// [wmb-studio END]
