// [wmb-studio BEGIN phase3]
import { STUDIO_FRAGMENT_HEADER } from "../common";

/**
 * Shared GLSL header and PBR / IBL / anisotropic strand / tonemap functions for Bayonetta 3.
 *
 * NOTE ON COLOR-SPACE SWITCHES (Section 3.4):
 * `B3_SRGB_ALBEDO` and `B3_ENCODE_OUTPUT` are declared at the top of the GLSL block below
 * so either can be flipped in one line if texture decode or canvas output gamma changes:
 * - `B3_SRGB_ALBEDO = false`: `get_wmb_texture` decodes RGBA8 texels directly; keep `false`
 *   to match Bayonetta 1/2 viewport parity, or set `true` to apply `pow(rgb, 2.2)` on sample.
 * - `B3_ENCODE_OUTPUT = true`: applies studio tone-mapping and gamma encoding before output.
 */
export const B3_PBR_COMMON_GLSL = /* glsl */ `
${STUDIO_FRAGMENT_HEADER}

const bool B3_SRGB_ALBEDO = false;
const bool B3_ENCODE_OUTPUT = true;

const float B3_PI = 3.14159265359;

vec3 DecodeB3Albedo(vec3 rawRgb) {
  vec3 c = max(rawRgb, vec3(0.0));
  return B3_SRGB_ALBEDO ? pow(c, vec3(2.2)) : c;
}

/**
 * Decodes a Bayonetta 3 normal map sample by ALWAYS reconstructing Z from the RG (XY)
 * components (xy = 2.0 * rg - 1.0, z = sqrt(max(0.0, 1.0 - dot(xy, xy)))), regardless of
 * what is packed in the B or A channels (AO, metallic, specular mask, roughness, or 0xFF).
 */
vec3 DecodeB3NormalTS(vec4 sampleNrm, bool forceRgReconstruct) {
  vec2 xy = sampleNrm.rg * 2.0 - 1.0;
  float z = sqrt(max(0.0, 1.0 - dot(xy, xy)));
  vec3 n = vec3(xy, z);
  if (dot(n, n) < 1e-4) {
    return vec3(0.0, 0.0, 1.0);
  }
  return normalize(n);
}

vec3 SafeOrthoTangentB3(vec3 N, vec3 tanVec) {
  vec3 ortho = tanVec - N * dot(N, tanVec);
  float lenSq = dot(ortho, ortho);
  if (lenSq > 1e-8) {
    return ortho * inversesqrt(lenSq);
  }
  vec3 fallbackAxis = abs(N.y) < 0.99 ? vec3(0.0, 1.0, 0.0) : vec3(1.0, 0.0, 0.0);
  return normalize(cross(fallbackAxis, N));
}

vec3 PerturbB3NormalW(vec3 nTS, float strength, vec3 tanW, vec3 nrmW) {
  vec3 N = normalize(nrmW);
  vec3 T = SafeOrthoTangentB3(N, tanW);
  vec3 B = cross(N, T);
  vec3 scaled = normalize(vec3(nTS.xy * strength, max(nTS.z, 1e-3)));
  return normalize(T * scaled.x + B * scaled.y + N * scaled.z);
}

vec3 PerturbB3NormalV(vec3 nTS, float strength, vec3 tanV, vec3 nrmV) {
  vec3 N = normalize(nrmV);
  vec3 T = SafeOrthoTangentB3(N, tanV);
  vec3 B = cross(N, T);
  vec3 scaled = normalize(vec3(nTS.xy * strength, max(nTS.z, 1e-3)));
  return normalize(T * scaled.x + B * scaled.y + N * scaled.z);
}

vec3 FresnelSchlickB3(float cosTheta, vec3 F0) {
  float t = clamp(1.0 - cosTheta, 0.0, 1.0);
  float t2 = t * t;
  return F0 + (vec3(1.0) - F0) * (t2 * t2 * t);
}

vec3 FresnelSchlickRoughnessB3(float cosTheta, vec3 F0, float roughness) {
  float t = clamp(1.0 - cosTheta, 0.0, 1.0);
  float t2 = t * t;
  return F0 + (max(vec3(1.0 - roughness), F0) - F0) * (t2 * t2 * t);
}

float DistributionGGXB3(vec3 N, vec3 H, float roughness) {
  float a = max(roughness * roughness, 0.002);
  float a2 = a * a;
  float NdotH = saturate(dot(N, H));
  float NdotH2 = NdotH * NdotH;
  float denom = NdotH2 * (a2 - 1.0) + 1.0;
  return a2 / max(B3_PI * denom * denom, 1e-5);
}

float GeometrySchlickGGXB3(float NdotV, float roughness) {
  float r = roughness + 1.0;
  float k = (r * r) / 8.0;
  return NdotV / max(NdotV * (1.0 - k) + k, 1e-4);
}

float GeometrySmithB3(vec3 N, vec3 V, vec3 L, float roughness) {
  float NdotV = max(dot(N, V), 1e-4);
  float NdotL = max(dot(N, L), 1e-4);
  return GeometrySchlickGGXB3(NdotV, roughness) * GeometrySchlickGGXB3(NdotL, roughness);
}

vec3 EvaluateSingleLightPbrB3(
  vec3 N,
  vec3 V,
  vec3 L,
  vec3 lightRadiance,
  vec3 diffK,
  float roughness,
  vec3 F0
) {
  float NdotL = saturate(dot(N, L));
  if (NdotL <= 0.0) return vec3(0.0);
  vec3 H = normalize(V + L);
  float NdotV = max(dot(N, V), 1e-4);
  float D = DistributionGGXB3(N, H, roughness);
  float G = GeometrySmithB3(N, V, L, roughness);
  vec3 F = FresnelSchlickB3(saturate(dot(H, V)), F0);

  vec3 spec = (D * G * F) / max(4.0 * NdotV * NdotL, 1e-4);
  vec3 kD = (vec3(1.0) - F);
  return (kD * diffK / B3_PI + spec) * lightRadiance * NdotL;
}

/**
 * Evaluates the studio 3-point analytic light rig using PBS00's metallic-roughness BRDF:
 * F0 = mix(vec3(0.03), baseColor, metallic), diffK = baseColor * (1.0 - metallic).
 */
vec3 EvaluateB3DirectPbr(
  vec3 N,
  vec3 V,
  vec3 diffK,
  float roughness,
  vec3 F0
) {
  vec3 keyRad = vec3(1.0, 0.98, 0.95) * (STUDIO_KEY_INTENSITY * 2.1);
  vec3 fillRad = vec3(0.82, 0.88, 1.0) * (STUDIO_FILL_INTENSITY * 1.9);
  vec3 rimDir = normalize(vec3(0.0, 0.45, -0.89));
  vec3 rimRad = vec3(0.90, 0.92, 1.0) * 0.45;

  vec3 acc = vec3(0.0);
  acc += EvaluateSingleLightPbrB3(N, V, STUDIO_KEY_TO_LIGHT, keyRad, diffK, roughness, F0);
  acc += EvaluateSingleLightPbrB3(N, V, STUDIO_FILL_TO_LIGHT, fillRad, diffK, roughness, F0);
  acc += EvaluateSingleLightPbrB3(N, V, rimDir, rimRad, diffK, roughness, F0);
  return acc;
}

/**
 * Evaluates Bayonetta 3 environment cube + irradiance cube + studio hemisphere IBL.
 */
vec3 EvaluateB3Ibl(
  vec3 N,
  vec3 V,
  vec3 diffK,
  float roughness,
  vec3 F0,
  float ao,
  float specScale,
  samplerCube envCube,
  samplerCube irradCube
) {
  float NdotV = saturate(dot(N, V));
  vec3 R = reflect(-V, N);

  vec3 envSharp = texCUBE_game(envCube, R).rgb;
  vec3 envSoft = texCUBE_game(irradCube, R).rgb;
  vec3 envSpec = mix(envSharp, envSoft, clamp(roughness * roughness, 0.0, 1.0));

  vec3 irrad = texCUBE_game(irradCube, N).rgb;
  float hemi = N.y * 0.5 + 0.5;
  vec3 studioHemi = mix(vec3(0.20, 0.22, 0.26), vec3(0.42, 0.44, 0.48), hemi);
  vec3 ambientIrr = mix(studioHemi, irrad, 0.45) * STUDIO_AMBIENT_INTENSITY * 1.55;

  vec3 kS = FresnelSchlickRoughnessB3(NdotV, F0, roughness);
  vec3 kD = (vec3(1.0) - kS);

  vec3 diffuseIbl = ambientIrr * diffK * kD * ao;
  float glossWeight = mix(1.0, 0.18, roughness);
  vec3 specIbl = envSpec * kS * glossWeight * max(specScale, 0.0) * mix(0.35, 1.0, ao);

  return diffuseIbl + specIbl;
}

/**
 * Rotates tangent vector T around normal N by angleRad (sincos(mat.g_HairOffset) in BAYO00 / Hair20).
 */
vec3 RotateStrandTangentB3(vec3 N, vec3 T, float angleRad) {
  vec3 orthoT = SafeOrthoTangentB3(N, T);
  vec3 B = cross(N, orthoT);
  float s = sin(angleRad);
  float c = cos(angleRad);
  return normalize(c * orthoT + s * B);
}

/**
 * Evaluates Kajiya-Kay / Marschner dual-lobe anisotropic strand specular (BAYO00 §1.5 & Hair20 §2.3).
 */
vec3 EvaluateB3HairDualLobes(
  vec3 N,
  vec3 Tstrand,
  vec3 V,
  vec3 L,
  vec3 priColor,
  float priRoughness,
  float priAlpha,
  vec3 secColor,
  float secRoughness,
  float secAlpha,
  float specPower
) {
  float NdotL = saturate(dot(N, L));
  if (NdotL <= 0.0) return vec3(0.0);

  vec3 H = normalize(L + V);
  // Primary shifted strand direction & secondary shifted strand direction
  vec3 T1 = normalize(Tstrand - 0.08 * N);
  vec3 T2 = normalize(Tstrand + 0.16 * N);

  float tdoth1 = dot(T1, H);
  float tdoth2 = dot(T2, H);
  float sinTH1 = sqrt(max(0.0, 1.0 - tdoth1 * tdoth1));
  float sinTH2 = sqrt(max(0.0, 1.0 - tdoth2 * tdoth2));

  float dirAtten1 = smoothstep(-1.0, 0.0, tdoth1);
  float dirAtten2 = smoothstep(-1.0, 0.0, tdoth2);

  float r1 = clamp(priRoughness, 0.05, 1.0);
  float r2 = clamp(secRoughness, 0.05, 1.0);
  float exp1 = clamp((2.0 / max(r1 * r1, 0.01)) * max(specPower, 0.15), 4.0, 256.0);
  float exp2 = clamp((2.0 / max(r2 * r2, 0.01)) * max(specPower * 0.5, 0.15), 2.0, 128.0);

  float lobe1 = dirAtten1 * pow(sinTH1, exp1) * 0.39894;
  float lobe2 = dirAtten2 * pow(sinTH2, exp2) * 0.07958;

  return (priColor * priAlpha * lobe1 + secColor * secAlpha * lobe2) * NdotL;
}

/**
 * Shared Bayonetta 3 luminance-preserving grade / tonemap epilogue + output encoding.
 */
vec4 FinishB3Fragment(
  vec3 linearColor,
  float alpha,
  vec3 Nw,
  vec3 Nv,
  vec3 posV
) {
  vec3 c = max(linearColor, vec3(0.0));
  // Soft shoulder roll-off so bright specular/emissive highlights never harsh-clip
  c = c / (vec3(1.0) + c * 0.22);
  if (B3_ENCODE_OUTPUT) {
    c = pow(max(c, vec3(0.0)), vec3(1.0 / 1.12));
  }
  return ApplyStudioPost(vec4(c, clamp(alpha, 0.0, 1.0)), Nw, Nv, posV);
}
`;
// [wmb-studio END]
