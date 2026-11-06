/**
 * Bayonetta 1 (PC, 2017) character shaders ported from `bayonetta_character_shaders.hlsl`
 * and D3D9 SM3.0 disassemblies (`disasm_all/*.pso.asm`) to WebGL2 / Three.js `ShaderMaterial` GLSL.
 *
 * Port order (Section 2.5):
 *   1. pls03 (types 110, 178) & pls04 (type 114, identical to pls03)
 *   2. obs07 (types 107, 109, 172, 183)
 *   3. pls01 (types 68, 113, 132, 182, 185, 186, 188) & pls07 (type 148, vertex alpha variant)
 *   4. pls05 (types 127, 179, 187)
 *   5. pls06 (types 135, 181, 184)
 *   6. pls09 (type 151 — treats Spec_Limit as a percentage * 0.01)
 *   7. obs00 (types 56, 75)
 *   8. obs04 (types 66, 76)
 *   9. obs10 (type 163)
 *  10. etc01 (type 60)
 *  11. obs02 (type 64)
 *  12. etc00 (types 59, 104)
 *  13. pls02 (type 88) & pls08 (type 149, vertex alpha variant)
 *  14. pls00 (type 67 — normal-mapped Spec_Pow LUT + dot_min/dot_max/dot_color silhouette rim)
 */

import { RenderQueueBucket, STUDIO_FRAGMENT_HEADER, STUDIO_VERTEX_SHADER } from "../common";

export interface B1ShaderSpec {
  id: string;
  materialTypes: number[];
  shaderNames: string[];
  vertexShader: string;
  fragmentShader: string;
  queue: RenderQueueBucket;
  transparent: boolean;
  depthWrite: boolean;
}

const B1_UNIFORMS_BLOCK = /* glsl */ `
uniform sampler2D Color_1_sampler;
uniform sampler2D reliefmap_sampler;
uniform samplerCube envmap_sampler;
uniform samplerCube env_amb_sampler;
uniform sampler2D Spec_Pow_sampler;
uniform sampler2D Spec_Mask_sampler;
uniform sampler2D lightmap_sampler;

uniform vec2 Color_1_Tile;
uniform vec2 Color_1_Offset;
uniform vec2 tile;
uniform float depth;
uniform float blend;
uniform float Env_Amb_Param;
uniform float env_factor;
uniform vec3 specular;
uniform vec4 spelightpos;
uniform float Spec_Type;
uniform float Spec_Limit;
uniform float shine;
uniform float bias;
uniform vec3 uDiffuseParam;
uniform vec3 uAmbientParam;
uniform vec3 SSS_color;
uniform float SSS_value;
uniform float SSS_flag;
uniform vec2 SSS_norm;
uniform float dot_min;
uniform float dot_max;
uniform vec3 dot_color;
uniform vec4 lightpos;
uniform vec3 light_col;
uniform vec4 lightpos2;
uniform vec3 light_col2;
uniform int uEnvUseNormal;      // 1 for 'm' variants (bxxmx / bxnmx), 0 for reflection vector
uniform int uAlphaBlendVariant; // 1 for bxnxb / blended variants
uniform int uUseVertexAlpha;    // 1 for pls07 / pls08 vertex-color alpha variants
`;

// ============================================================================================
// 1. pls03_bxnxx / bxnxa (types 110, 178) & pls04_bxnxx (type 114)
// Original HLSL: PS_pls03(VS_OUT_NM i)
//   float4 albedo = tex2D(Color_1_sampler, i.uv.xy * Color_1_Tile + Color_1_Offset);
//   clip(albedo.a - 0.5);
//   float4 nm = tex2D(reliefmap_sampler, i.uv.zw * tile);
//   float3 Nv = normalize(PerturbNormal(nm, depth, normalize(i.tanV), normalize(i.nrmV)));
//   float3 diffuse = TwoLights(i.posV, Nv);
//   float3 Nw  = PerturbNormal(nm, depth, i.tanW, i.nrmW);
//   float3 R   = i.eyeW - 2 * dot(i.eyeW, Nw) * Nw;
//   float4 env = texCUBE_game(envmap_sampler,  R);
//   float4 amb = texCUBE_game(env_amb_sampler, Nw);
//   float4 E   = lerp(env, amb, saturate(nm.b * 4));
//   float3 color = albedo.rgb * (E.rgb * blend + diffuse) + E.rgb * E.a * nm.a * Env_Amb_Param;
// ============================================================================================
const FRAG_B1_PLS03 = /* glsl */ `
${STUDIO_FRAGMENT_HEADER}
${B1_UNIFORMS_BLOCK}

void main() {
  vec4 albedo = texture2D(Color_1_sampler, vUv.xy * Color_1_Tile + Color_1_Offset);
  if (uDebugMode == 0 && albedo.a <= 0.3333) discard;

  vec4 nm = texture2D(reliefmap_sampler, vUv.zw * tile);
  vec3 T0 = normalize(vTanV);
  vec3 N0 = normalize(vNrmV);
  vec3 Nv = normalize(PerturbNormal(nm, depth, T0, N0));
  vec3 Nw = PerturbNormal(nm, depth, vTanW, vNrmW);
  vec3 diffuse = TwoLights(vPosV, Nv, Nw, lightpos, light_col, lightpos2, light_col2);

  vec3 R = vEyeW - 2.0 * dot(vEyeW, Nw) * Nw;
  vec4 env = texCUBE_game(envmap_sampler, R);
  vec4 amb = texCUBE_game(env_amb_sampler, Nw);
  vec4 E = mix(env, amb, saturate(nm.b * 4.0));

  vec3 color = albedo.rgb * (E.rgb * blend + diffuse)
             + E.rgb * E.a * nm.a * Env_Amb_Param;
  gl_FragColor = ApplyStudioPost(vec4(color, 1.0), Nw, Nv, vPosV);
}
`;

// ============================================================================================
// 2. obs07_bxnxx / bxnmx / bxnxa / bxnma (types 107, 109, 172, 183)
// Original HLSL: PS_obs07(VS_OUT_NM i)
//   float4 albedo = tex2D(Color_1_sampler, i.uv.xy * Color_1_Tile + Color_1_Offset);
//   clip(albedo.a - 0.5);
//   float4 nm = tex2D(reliefmap_sampler, i.uv.zw * tile);
//   float3 Nv = normalize(PerturbNormal(nm, depth, normalize(i.tanV), normalize(i.nrmV)));
//   float3 Nw = PerturbNormal(nm, depth, i.tanW, i.nrmW);
//   float3 envDir = uEnvUseNormal ? Nw : (i.eyeW - 2 * dot(i.eyeW, Nw) * Nw);
//   float4 env = texCUBE_game(envmap_sampler, envDir);
//   float3 color = albedo.rgb * (env.rgb * blend + TwoLights(i.posV, Nv))
//                + env.rgb * env.a * nm.a * Env_Amb_Param;
// ============================================================================================
const FRAG_B1_OBS07 = /* glsl */ `
${STUDIO_FRAGMENT_HEADER}
${B1_UNIFORMS_BLOCK}

void main() {
  vec4 albedo = texture2D(Color_1_sampler, vUv.xy * Color_1_Tile + Color_1_Offset);
  if (uDebugMode == 0 && albedo.a <= 0.3333) discard;

  vec4 nm = texture2D(reliefmap_sampler, vUv.zw * tile);
  vec3 Nv = normalize(PerturbNormal(nm, depth, normalize(vTanV), normalize(vNrmV)));
  vec3 Nw = PerturbNormal(nm, depth, vTanW, vNrmW);
  vec3 envDir = uEnvUseNormal == 1 ? Nw : (vEyeW - 2.0 * dot(vEyeW, Nw) * Nw);
  vec4 env = texCUBE_game(envmap_sampler, envDir);
  vec3 color = albedo.rgb * (env.rgb * blend + TwoLights(vPosV, Nv, Nw, lightpos, light_col, lightpos2, light_col2))
             + env.rgb * env.a * nm.a * Env_Amb_Param;
  gl_FragColor = ApplyStudioPost(vec4(color, 1.0), Nw, Nv, vPosV);
}
`;

// ============================================================================================
// 3. pls01 (types 68, 113, 132, 182, 185, 186, 188) & pls07 (type 148) — hair (aniso-ish spec LUT)
// Original HLSL: PS_pls01(float2 uv, float3 posV, float3 tanV, float3 nrmV, float3 envDirW)
//   float2 st     = uv + Color_1_Offset;
//   float4 albedo = tex2D(Color_1_sampler, st);
//   float  mask   = tex2D(Spec_Mask_sampler, st).r;
//   float  alpha  = uUseVertexAlpha ? (albedo.a * vColor.a) : albedo.a;
//   clip(alpha - 0.001);
//   float3 B = cross(normalize(tanV), normalize(nrmV));
//   float3 V = normalize(posV);
//   float3 d1 = lightpos.xyz  - posV;  float att1 = max(lightpos.w  - length(d1), 0) / lightpos.w;
//   float3 d2 = lightpos2.xyz - posV;  float att2 = max(lightpos2.w - length(d2), 0) / lightpos2.w;
//   float3 L1 = normalize(d1), L2 = normalize(d2);
//   float3 H1 = normalize(L1 - V),  H2 = normalize(L2 - V);
//   float aniso = 1 - abs(dot(H1, B)) * shine;
//   float inv   = 1 / (1 - Spec_Limit);
//   float s1 = (saturate(dot(H1, nrmV)) * att1 * aniso - Spec_Limit) * inv;
//   float s2 = (saturate(dot(H2, nrmV)) * att2 * aniso - Spec_Limit) * inv;
//   float3 spec = mask * specular * (tex2D(Spec_Pow_sampler, float2(s1, Spec_Type)).rgb +
//                                    tex2D(Spec_Pow_sampler, float2(s2, Spec_Type)).rgb);
//   float4 env     = texCUBE(envmap_sampler, envDirW);
//   float3 diffuse = saturate(dot(L1, nrmV)) * light_col * att1 + saturate(dot(L2, nrmV)) * light_col2 * att2;
//   float3 color = albedo.rgb * (env.rgb * blend + diffuse)
//                + mask * env.rgb * env.a * Env_Amb_Param
//                + spec * env.rgb * blend;
// ============================================================================================
const FRAG_B1_PLS01 = /* glsl */ `
${STUDIO_FRAGMENT_HEADER}
${B1_UNIFORMS_BLOCK}

void main() {
  vec2 st = vUv.xy + Color_1_Offset;
  vec4 albedo = texture2D(Color_1_sampler, st);
  float mask = texture2D(Spec_Mask_sampler, st).r;
  float alpha = uUseVertexAlpha == 1 ? (albedo.a * vColor.a) : albedo.a;
  if (uDebugMode == 0 && alpha < 0.001) discard;

  vec3 Tv = normalize(vTanV);
  vec3 Nv = normalize(vNrmV);
  vec3 B = cross(Tv, Nv);
  vec3 V = normalize(vPosV);

  // Runtime engine sets lightpos = spelightpos (5, 0, 10, 10000) for pls01/pls07 hair draws
  vec4 lp1 = length(lightpos.xyz) > 1e-4 ? lightpos : vec4(5.0, 0.0, 10.0, 10000.0);
  vec4 lp2 = length(lightpos2.xyz) > 1e-4 ? lightpos2 : vec4(-5.0, 2.0, 10.0, 10000.0);
  vec3 d1 = lp1.xyz - vPosV;
  float r1 = max(abs(lp1.w), 1.0);
  float att1 = max(r1 - length(d1), 0.0) / r1;
  vec3 d2 = lp2.xyz - vPosV;
  float r2 = max(abs(lp2.w), 1.0);
  float att2 = max(r2 - length(d2), 0.0) / r2;

  vec3 L1 = normalize(d1);
  vec3 L2 = normalize(d2);
  vec3 H1 = normalize(L1 - V);
  vec3 H2 = normalize(L2 - V);

  float aniso = 1.0 - abs(dot(H1, B)) * shine;
  float denom = max(1.0 - Spec_Limit, 1e-4);
  float inv = 1.0 / denom;
  float s1 = clamp((saturate(dot(H1, Nv)) * att1 * aniso - Spec_Limit) * inv, 0.0, 1.0);
  float s2 = clamp((saturate(dot(H2, Nv)) * att2 * aniso - Spec_Limit) * inv, 0.0, 1.0);
  float lutY = clamp(Spec_Type, 0.0, 1.0);
  vec3 spec = mask * specular * (
    texture2D(Spec_Pow_sampler, vec2(s1, lutY)).rgb +
    texture2D(Spec_Pow_sampler, vec2(s2, lutY)).rgb
  );

  vec3 Nw = ReorthoWorldNormal(vTanW, vNrmW);
  vec3 envDirW = uEnvUseNormal == 1 ? Nw : reflect(vEyeW, Nw);
  vec4 env = texCUBE_game(envmap_sampler, envDirW);
  vec3 diffuse = TwoLights(vPosV, Nv, Nw, lightpos, light_col, lightpos2, light_col2);

  vec3 color = albedo.rgb * (env.rgb * blend + diffuse)
             + mask * env.rgb * env.a * Env_Amb_Param
             + spec * env.rgb * blend;
  gl_FragColor = ApplyStudioPost(vec4(color, alpha), Nw, Nv, vPosV);
}
`;

// ============================================================================================
// 4. pls05_bxnxx / bxnxa / bxnxb (types 127 / 179 / 187) — BAYONETTA'S HAIR SUIT
// Original HLSL: PS_pls05(VS_OUT_NM i)
//   float4 albedo = tex2D(Color_1_sampler, i.uv.xy * Color_1_Tile + Color_1_Offset);
//   clip(albedo.a - 0.5);
//   float  smask  = albedo.a * 2 - 1;
//   float4 nm = tex2D(reliefmap_sampler, i.uv.zw * tile);
//   float3 T0 = normalize(i.tanV), N0 = normalize(i.nrmV);
//   float3 Nv = normalize(PerturbNormal(nm, depth, T0, N0));
//   float3 diffuse = TwoLights(i.posV, Nv);
//   float3 Nw  = PerturbNormal(nm, depth, i.tanW, i.nrmW);
//   float3 R   = i.eyeW - 2 * dot(i.eyeW, Nw) * Nw;
//   float  t   = saturate(nm.b * 4);
//   float4 E   = lerp(texCUBE_game(envmap_sampler, R), texCUBE_game(envmap_sampler, Nw), t);
//   float3 base = albedo.rgb * (E.rgb * blend + diffuse);
//   float3 refl = smask * E.rgb * E.a * Env_Amb_Param;
//   float3 V   = normalize(i.posV);
//   float  k   = 1 + dot(N0, V);
//   float  rim = (0.9 - t >= 0) ? 0 : saturate(k * k);
//   float3 H     = normalize(normalize(spelightpos.xyz - i.posV) - V);
//   float  aniso = 1 - abs(dot(H, cross(T0, N0))) * shine;
//   float  x     = (saturate(dot(H, N0)) * aniso - Spec_Limit) / (1 - Spec_Limit);
//   float3 spec  = smask * specular * tex2D(Spec_Pow_sampler, float2(x, Spec_Type)).rgb * nm.a;
//   float3 color = base * (1 + rim) + refl + E.rgb * spec * blend;
// ============================================================================================
const FRAG_B1_PLS05 = /* glsl */ `
${STUDIO_FRAGMENT_HEADER}
${B1_UNIFORMS_BLOCK}

void main() {
  vec4 albedo = texture2D(Color_1_sampler, vUv.xy * Color_1_Tile + Color_1_Offset);
  if (uDebugMode == 0 && albedo.a <= 0.3333) discard;
  float smask = albedo.a * 2.0 - 1.0;

  vec4 nm = texture2D(reliefmap_sampler, vUv.zw * tile);
  vec3 T0 = normalize(vTanV);
  vec3 N0 = normalize(vNrmV);
  vec3 Nv = normalize(PerturbNormal(nm, depth, T0, N0));
  vec3 Nw = PerturbNormal(nm, depth, vTanW, vNrmW);
  vec3 diffuse = TwoLights(vPosV, Nv, Nw, lightpos, light_col, lightpos2, light_col2);

  vec3 R = vEyeW - 2.0 * dot(vEyeW, Nw) * Nw;
  float t = saturate(nm.b * 4.0);
  vec4 E = mix(texCUBE_game(envmap_sampler, R), texCUBE_game(envmap_sampler, Nw), t);

  vec3 base = albedo.rgb * (E.rgb * blend + diffuse);
  vec3 refl = smask * E.rgb * E.a * Env_Amb_Param;

  vec3 V = normalize(vPosV);
  float k = 1.0 + dot(N0, V);
  float rim = (0.9 - t >= 0.0) ? 0.0 : saturate(k * k);

  vec3 spePos = length(spelightpos.xyz) > 1e-4 ? spelightpos.xyz : vec3(5.0, 0.0, 10.0);
  vec3 H = normalize(normalize(spePos - vPosV) - V);
  float aniso = 1.0 - abs(dot(H, cross(T0, N0))) * shine;
  float denom = max(1.0 - Spec_Limit, 1e-4);
  float x = clamp((saturate(dot(H, N0)) * aniso - Spec_Limit) / denom, 0.0, 1.0);
  vec3 spec = smask * specular * texture2D(Spec_Pow_sampler, vec2(x, clamp(Spec_Type, 0.0, 1.0))).rgb * nm.a;

  vec3 color = base * (1.0 + rim) + refl + E.rgb * spec * blend;
  float outA = uAlphaBlendVariant == 1 ? saturate(albedo.a) : 1.0;
  gl_FragColor = ApplyStudioPost(vec4(color, outA), Nw, Nv, vPosV);
}
`;

// ============================================================================================
// 5. pls06_bxnxx / bxnxa (types 135, 181) & pls06_bxnxb (type 184) — pls03 + fake subsurface / rim tint
// Original HLSL: PS_pls06(VS_OUT_NM i)
//   float4 nm = tex2D(reliefmap_sampler, i.uv.zw * tile);
//   float3 N0 = normalize(i.nrmV);
//   float3 Nv = normalize(PerturbNormal(nm, depth, normalize(i.tanV), N0));
//   float3 diffuse = TwoLights(i.posV, Nv);
//   float3 Nw  = PerturbNormal(nm, depth, i.tanW, i.nrmW);
//   float3 R   = i.eyeW - 2 * dot(i.eyeW, Nw) * Nw;
//   float4 E   = lerp(texCUBE_game(envmap_sampler, R), texCUBE_game(env_amb_sampler, Nw), saturate(nm.b * 4));
//   float3 lit = E.rgb * blend + diffuse;
//   float4 albedo = tex2D(Color_1_sampler, i.uv.xy * Color_1_Tile + Color_1_Offset);
//   float facing = -dot(N0, normalize(i.posV));
//   float k      = 1 - lerp(SSS_norm.x, SSS_norm.y, facing);
//   float sss    = (SSS_flag > 0 ? saturate(k * k) : saturate(k)) * SSS_value;
//   float3 tint  = 1 + (albedo.a * sss) * SSS_color;
//   float3 color = albedo.rgb * lit * tint + E.rgb * E.a * nm.a * Env_Amb_Param;
// ============================================================================================
const FRAG_B1_PLS06 = /* glsl */ `
${STUDIO_FRAGMENT_HEADER}
${B1_UNIFORMS_BLOCK}

void main() {
  vec4 albedo = texture2D(Color_1_sampler, vUv.xy * Color_1_Tile + Color_1_Offset);
  if (uDebugMode == 0 && uAlphaBlendVariant == 1 && albedo.a < 0.001) discard;

  vec4 nm = texture2D(reliefmap_sampler, vUv.zw * tile);
  vec3 N0 = normalize(vNrmV);
  vec3 Nv = normalize(PerturbNormal(nm, depth, normalize(vTanV), N0));
  vec3 Nw = PerturbNormal(nm, depth, vTanW, vNrmW);
  vec3 diffuse = TwoLights(vPosV, Nv, Nw, lightpos, light_col, lightpos2, light_col2);

  vec3 R = vEyeW - 2.0 * dot(vEyeW, Nw) * Nw;
  vec4 E = mix(texCUBE_game(envmap_sampler, R), texCUBE_game(env_amb_sampler, Nw), saturate(nm.b * 4.0));
  vec3 lit = E.rgb * blend + diffuse;

  float facing = -dot(N0, normalize(vPosV));
  float k = 1.0 - mix(SSS_norm.x, SSS_norm.y, facing);
  float sss = (SSS_flag > 0.0 ? saturate(k * k) : saturate(k)) * SSS_value;
  vec3 tint = vec3(1.0) + (albedo.a * sss) * SSS_color;

  vec3 color = albedo.rgb * lit * tint + E.rgb * E.a * nm.a * Env_Amb_Param;
  float outA = uAlphaBlendVariant == 1 ? albedo.a : 1.0;
  gl_FragColor = ApplyStudioPost(vec4(color, outA), Nw, Nv, vPosV);
}
`;

// ============================================================================================
// 6. pls09_bxxmx (type 151) — face (spec LUT, no normal map; Spec_Limit is a percentage * 0.01)
// Original HLSL: PS_pls09(float2 uv, float3 posV, float3 nrmV, float3 reflW, float3 nrmW)
//   float4 albedo = tex2D(Color_1_sampler, uv + Color_1_Offset);
//   clip(albedo.a - 0.5);
//   float3 N = normalize(nrmV);
//   float3 lit  = texCUBE(env_amb_sampler, nrmW).rgb * blend + TwoLights(posV, N);
//   float4 env  = texCUBE(envmap_sampler, reflW);
//   float3 refl = env.rgb * (env.a * Env_Amb_Param + env_factor);
//   float3 H  = normalize(normalize(spelightpos.xyz - posV) - normalize(posV));
//   float  lim = Spec_Limit * 0.01;
//   float  x  = (saturate(dot(H, N)) - lim) / (1 - lim);
//   float3 sp = specular * tex2D(Spec_Pow_sampler, float2(x, Spec_Type)).rgb;
//   return FinalOut(albedo.rgb * lit + refl + sp, posV.z);
// ============================================================================================
const FRAG_B1_PLS09 = /* glsl */ `
${STUDIO_FRAGMENT_HEADER}
${B1_UNIFORMS_BLOCK}

void main() {
  vec4 albedo = texture2D(Color_1_sampler, vUv.xy + Color_1_Offset);
  if (uDebugMode == 0 && albedo.a <= 0.3333) discard;

  vec3 N = normalize(vNrmV);
  vec3 Nw = ReorthoWorldNormal(vTanW, vNrmW);
  vec3 reflW = reflect(vEyeW, Nw);

  vec3 lit = texCUBE_game(env_amb_sampler, Nw).rgb * blend
           + TwoLights(vPosV, N, Nw, lightpos, light_col, lightpos2, light_col2);
  vec4 env = texCUBE_game(envmap_sampler, reflW);
  vec3 refl = env.rgb * (env.a * Env_Amb_Param + env_factor);

  // Runtime engine scales file spelightpos by (5, -5, 1)
  vec3 spePos = length(spelightpos.xyz) > 1e-4
    ? spelightpos.xyz * vec3(5.0, -5.0, 1.0)
    : vec3(3.65, 18.25, 100.0);
  vec3 H = normalize(normalize(spePos - vPosV) - normalize(vPosV));
  float lim = Spec_Limit * 0.01;
  float denom = max(1.0 - lim, 1e-4);
  float x = clamp((saturate(dot(H, N)) - lim) / denom, 0.0, 1.0);
  vec3 sp = specular * texture2D(Spec_Pow_sampler, vec2(x, clamp(Spec_Type, 0.0, 1.0))).rgb;

  vec3 color = albedo.rgb * lit + refl + sp;
  gl_FragColor = ApplyStudioPost(vec4(color, 1.0), Nw, N, vPosV);
}
`;

// ============================================================================================
// 7. obs00_bxnxx / bxnmx (types 56, 75) — obs07 whose reflection mask is Spec_Mask.r instead of nm.a
// Original HLSL: PS_obs00(VS_OUT_NM i)
//   float2 st     = i.uv.xy * Color_1_Tile + Color_1_Offset;
//   float4 albedo = tex2D(Color_1_sampler, st);
//   float  mask   = tex2D(Spec_Mask_sampler, st).r;
//   clip(albedo.a - 0.5);
//   float4 nm = tex2D(reliefmap_sampler, i.uv.zw * tile);
//   float3 Nv = normalize(PerturbNormal(nm, depth, normalize(i.tanV), normalize(i.nrmV)));
//   float3 Nw = PerturbNormal(nm, depth, i.tanW, i.nrmW);
//   float3 envDir = uEnvUseNormal ? Nw : (i.eyeW - 2 * dot(i.eyeW, Nw) * Nw);
//   float4 env = texCUBE_game(envmap_sampler, envDir);
//   float3 color = albedo.rgb * (env.rgb * blend + TwoLights(i.posV, Nv)) + env.rgb * env.a * mask * Env_Amb_Param;
// ============================================================================================
const FRAG_B1_OBS00 = /* glsl */ `
${STUDIO_FRAGMENT_HEADER}
${B1_UNIFORMS_BLOCK}

void main() {
  vec2 st = vUv.xy * Color_1_Tile + Color_1_Offset;
  vec4 albedo = texture2D(Color_1_sampler, st);
  float mask = texture2D(Spec_Mask_sampler, st).r;
  if (uDebugMode == 0 && albedo.a <= 0.3333) discard;

  vec4 nm = texture2D(reliefmap_sampler, vUv.zw * tile);
  vec3 Nv = normalize(PerturbNormal(nm, depth, normalize(vTanV), normalize(vNrmV)));
  vec3 Nw = PerturbNormal(nm, depth, vTanW, vNrmW);
  vec3 envDir = uEnvUseNormal == 1 ? Nw : (vEyeW - 2.0 * dot(vEyeW, Nw) * Nw);
  vec4 env = texCUBE_game(envmap_sampler, envDir);
  vec3 color = albedo.rgb * (env.rgb * blend + TwoLights(vPosV, Nv, Nw, lightpos, light_col, lightpos2, light_col2))
             + env.rgb * env.a * mask * Env_Amb_Param;
  gl_FragColor = ApplyStudioPost(vec4(color, 1.0), Nw, Nv, vPosV);
}
`;

// ============================================================================================
// 8. obs04_bxxxx / obs04_bxxmx (types 66 / 76) — simple env-mapped (no normal map, alpha test 0.5)
// Original HLSL: PS_obs04(float2 uv, float3 posV, float3 nrmV, float3 envDirW)
//   float4 albedo = tex2D(Color_1_sampler, uv * Color_1_Tile + Color_1_Offset);
//   clip(albedo.a - 0.5);
//   float4 env = texCUBE(envmap_sampler, envDirW);
//   float3 color = albedo.rgb * (env.rgb * blend + TwoLights(posV, normalize(nrmV)))
//                + env.rgb * env.a * Env_Amb_Param;
// ============================================================================================
const FRAG_B1_OBS04 = /* glsl */ `
${STUDIO_FRAGMENT_HEADER}
${B1_UNIFORMS_BLOCK}

void main() {
  vec4 albedo = texture2D(Color_1_sampler, vUv.xy * Color_1_Tile + Color_1_Offset);
  if (uDebugMode == 0 && albedo.a <= 0.3333) discard;

  vec3 Nv = normalize(vNrmV);
  vec3 Nw = ReorthoWorldNormal(vTanW, vNrmW);
  vec3 envDirW = uEnvUseNormal == 1 ? Nw : reflect(vEyeW, Nw);
  vec4 env = texCUBE_game(envmap_sampler, envDirW);
  vec3 color = albedo.rgb * (env.rgb * blend + TwoLights(vPosV, Nv, Nw, lightpos, light_col, lightpos2, light_col2))
             + env.rgb * env.a * Env_Amb_Param;
  gl_FragColor = ApplyStudioPost(vec4(color, 1.0), Nw, Nv, vPosV);
}
`;

// ============================================================================================
// 9. obs10_bxnxx (type 163) — normal-mapped + env_amb ambient, unmasked reflection + env_factor
// Original HLSL: PS_obs10(VS_OUT_NM i)
//   float4 albedo = tex2D(Color_1_sampler, i.uv.xy * Color_1_Tile + Color_1_Offset);
//   clip(albedo.a - 0.5);
//   float4 nm = tex2D(reliefmap_sampler, i.uv.zw * tile);
//   float3 Nv = normalize(PerturbNormal(nm, depth, normalize(i.tanV), normalize(i.nrmV)));
//   float3 Nw = PerturbNormal(nm, depth, i.tanW, i.nrmW);
//   float3 amb = texCUBE_game(env_amb_sampler, Nw).rgb;
//   float4 env = texCUBE_game(envmap_sampler, i.eyeW - 2 * dot(i.eyeW, Nw) * Nw);
//   float3 color = albedo.rgb * (amb * blend + TwoLights(i.posV, Nv))
//                + env.rgb * (env.a * Env_Amb_Param + env_factor);
// ============================================================================================
const FRAG_B1_OBS10 = /* glsl */ `
${STUDIO_FRAGMENT_HEADER}
${B1_UNIFORMS_BLOCK}

void main() {
  vec4 albedo = texture2D(Color_1_sampler, vUv.xy * Color_1_Tile + Color_1_Offset);
  if (uDebugMode == 0 && albedo.a <= 0.3333) discard;

  vec4 nm = texture2D(reliefmap_sampler, vUv.zw * tile);
  vec3 Nv = normalize(PerturbNormal(nm, depth, normalize(vTanV), normalize(vNrmV)));
  vec3 Nw = PerturbNormal(nm, depth, vTanW, vNrmW);
  vec3 amb = texCUBE_game(env_amb_sampler, Nw).rgb;
  vec4 env = texCUBE_game(envmap_sampler, vEyeW - 2.0 * dot(vEyeW, Nw) * Nw);
  vec3 color = albedo.rgb * (amb * blend + TwoLights(vPosV, Nv, Nw, lightpos, light_col, lightpos2, light_col2))
             + env.rgb * (env.a * Env_Amb_Param + env_factor);
  gl_FragColor = ApplyStudioPost(vec4(color, 1.0), Nw, Nv, vPosV);
}
`;

// ============================================================================================
// 10. etc01_bxnxx (type 60) — lens ("renz"), reflection-driven alpha
// Original HLSL: PS_etc01(VS_OUT_NM i)
//   float2 st     = i.uv.xy * Color_1_Tile + Color_1_Offset;
//   float4 albedo = tex2D(Color_1_sampler, st);
//   float  mask   = tex2D(Spec_Mask_sampler, st).r;
//   clip(albedo.a - 0.001);
//   float4 nm = tex2D(reliefmap_sampler, i.uv.zw * tile);
//   float3 Nv = normalize(PerturbNormal(nm, depth, normalize(i.tanV), normalize(i.nrmV)));
//   float3 Nw = PerturbNormal(nm, depth, i.tanW, i.nrmW);
//   float4 env = texCUBE_game(envmap_sampler, i.eyeW - 2 * dot(i.eyeW, Nw) * Nw);
//   float  r   = mask * env.a * Env_Amb_Param;
//   float3 color = albedo.rgb * (env.rgb * blend + TwoLights(i.posV, Nv)) + env.rgb * r;
//   o.a = max(albedo.a, r) * prefogcolor_enhance.w * finalcolor_enhance.a;
// ============================================================================================
const FRAG_B1_ETC01 = /* glsl */ `
${STUDIO_FRAGMENT_HEADER}
${B1_UNIFORMS_BLOCK}

void main() {
  vec2 st = vUv.xy * Color_1_Tile + Color_1_Offset;
  vec4 albedo = texture2D(Color_1_sampler, st);
  float mask = texture2D(Spec_Mask_sampler, st).r;
  if (uDebugMode == 0 && albedo.a < 0.001) discard;

  vec4 nm = texture2D(reliefmap_sampler, vUv.zw * tile);
  vec3 Nv = normalize(PerturbNormal(nm, depth, normalize(vTanV), normalize(vNrmV)));
  vec3 Nw = PerturbNormal(nm, depth, vTanW, vNrmW);
  vec4 env = texCUBE_game(envmap_sampler, vEyeW - 2.0 * dot(vEyeW, Nw) * Nw);
  float r = mask * env.a * Env_Amb_Param;
  vec3 color = albedo.rgb * (env.rgb * blend + TwoLights(vPosV, Nv, Nw, lightpos, light_col, lightpos2, light_col2))
             + env.rgb * r;
  float outA = clamp(max(albedo.a, r), 0.0, 1.0);
  gl_FragColor = ApplyStudioPost(vec4(color, outA), Nw, Nv, vPosV);
}
`;

// ============================================================================================
// 11. obs02_bxxxx (type 64) — pls09 without spec, times a lightmap on UV2
// Original HLSL: PS_obs02(float4 uv, float3 posV, float3 nrmV, float3 reflW, float3 nrmW)
//   float4 albedo = tex2D(Color_1_sampler, uv.xy * Color_1_Tile + Color_1_Offset);
//   clip(albedo.a - 0.5);
//   float3 lit  = texCUBE(env_amb_sampler, nrmW).rgb * blend + TwoLights(posV, nrmV);
//   float4 env  = texCUBE(envmap_sampler, reflW);
//   float3 color = albedo.rgb * lit + env.rgb * (env.a * Env_Amb_Param + env_factor);
//   color *= tex2D(lightmap_sampler, uv.zw * Color_1_Tile + Color_1_Offset).rgb;
// ============================================================================================
const FRAG_B1_OBS02 = /* glsl */ `
${STUDIO_FRAGMENT_HEADER}
${B1_UNIFORMS_BLOCK}

void main() {
  vec4 albedo = texture2D(Color_1_sampler, vUv.xy * Color_1_Tile + Color_1_Offset);
  if (uDebugMode == 0 && albedo.a <= 0.3333) discard;

  vec3 Nv = normalize(vNrmV);
  vec3 Nw = ReorthoWorldNormal(vTanW, vNrmW);
  vec3 reflW = reflect(vEyeW, Nw);

  vec3 lit = texCUBE_game(env_amb_sampler, Nw).rgb * blend
           + TwoLights(vPosV, Nv, Nw, lightpos, light_col, lightpos2, light_col2);
  vec4 env = texCUBE_game(envmap_sampler, reflW);
  vec3 color = albedo.rgb * lit + env.rgb * (env.a * Env_Amb_Param + env_factor);
  color *= texture2D(lightmap_sampler, vUv.zw * Color_1_Tile + Color_1_Offset).rgb;
  gl_FragColor = ApplyStudioPost(vec4(color, 1.0), Nw, Nv, vPosV);
}
`;

// ============================================================================================
// 12. etc00_bxxxx / etc00_bxxmx (types 59, 104) — eye shadow ("eye_kage"), obs04 with alpha test 0.001 (blended)
// Original HLSL: PS_obs04 with clip(albedo.a - 0.001) and envDirW = uEnvUseNormal ? Nw : reflect(vEyeW, Nw)
// ============================================================================================
const FRAG_B1_ETC00 = /* glsl */ `
${STUDIO_FRAGMENT_HEADER}
${B1_UNIFORMS_BLOCK}

void main() {
  vec4 albedo = texture2D(Color_1_sampler, vUv.xy * Color_1_Tile + Color_1_Offset);
  if (uDebugMode == 0 && albedo.a < 0.001) discard;

  vec3 Nv = normalize(vNrmV);
  vec3 Nw = ReorthoWorldNormal(vTanW, vNrmW);
  vec3 envDirW = uEnvUseNormal == 1 ? Nw : reflect(vEyeW, Nw);
  vec4 env = texCUBE_game(envmap_sampler, envDirW);
  vec3 color = albedo.rgb * (env.rgb * blend + TwoLights(vPosV, Nv, Nw, lightpos, light_col, lightpos2, light_col2))
             + env.rgb * env.a * Env_Amb_Param;
  gl_FragColor = ApplyStudioPost(vec4(color, albedo.a), Nw, Nv, vPosV);
}
`;

// ============================================================================================
// 13. pls02_bxxxx (type 88) & pls08_bxxxx (type 149) — unlit hair cards
// Original HLSL: PS_pls02(float2 uv, float3 posV)
//   float4 albedo = tex2D(Color_1_sampler, uv * Color_1_Tile + Color_1_Offset);
//   float  alpha  = uUseVertexAlpha ? (albedo.a * vColor.a) : albedo.a;
//   clip(alpha - 0.001);
//   return FinalOut((diffuse * albedo.rgb + ambient) * bias, posV.z);
// ============================================================================================
const FRAG_B1_PLS02 = /* glsl */ `
${STUDIO_FRAGMENT_HEADER}
${B1_UNIFORMS_BLOCK}

void main() {
  vec4 albedo = texture2D(Color_1_sampler, vUv.xy * Color_1_Tile + Color_1_Offset);
  float alpha = uUseVertexAlpha == 1 ? (albedo.a * vColor.a) : albedo.a;
  if (uDebugMode == 0 && alpha < 0.001) discard;

  vec3 Nw = normalize(vNrmW);
  vec3 Nv = normalize(vNrmV);
  vec3 color = (uDiffuseParam * albedo.rgb + uAmbientParam) * bias;
  gl_FragColor = ApplyStudioPost(vec4(color, alpha), Nw, Nv, vPosV);
}
`;

// ============================================================================================
// 14. pls00_bxnxx (type 67) — normal-mapped character suit/skin with Spec_Pow LUT + dot_color rim
// Reconstructed from `disasm_all/modelshaderpls00_bxnxx.pso.asm`:
//   float2 st     = i.uv.xy * Color_1_Tile + Color_1_Offset;
//   float4 albedo = tex2D(Color_1_sampler, st);
//   float  mask   = tex2D(Spec_Mask_sampler, st).r;
//   clip(albedo.a - 0.5);
//   float4 nm = tex2D(reliefmap_sampler, i.uv.zw * tile);
//   float3 T0 = normalize(i.tanV), N0 = normalize(i.nrmV);
//   float3 Nv = normalize(PerturbNormal(nm, depth, T0, N0));
//   float3 Nw = PerturbNormal(nm, depth, i.tanW, i.nrmW);
//   float4 env = texCUBE_game(envmap_sampler, i.eyeW - 2 * dot(i.eyeW, Nw) * Nw);
//   float3 diffuse = TwoLights(i.posV, Nv);
//   float3 base = albedo.rgb * (env.rgb * blend + diffuse) + mask * env.rgb * env.a * Env_Amb_Param;
//   // Two point-light specular lookups into Spec_Pow_sampler
//   float3 V = normalize(i.posV);
//   // Rim band between dot_min and dot_max: triangle wave 1 - abs(2 * t - 1)
// ============================================================================================
const FRAG_B1_PLS00 = /* glsl */ `
${STUDIO_FRAGMENT_HEADER}
${B1_UNIFORMS_BLOCK}

void main() {
  vec2 st = vUv.xy * Color_1_Tile + Color_1_Offset;
  vec4 albedo = texture2D(Color_1_sampler, st);
  float mask = texture2D(Spec_Mask_sampler, st).r;
  if (uDebugMode == 0 && albedo.a <= 0.3333) discard;

  vec4 nm = texture2D(reliefmap_sampler, vUv.zw * tile);
  vec3 T0 = normalize(vTanV);
  vec3 N0 = normalize(vNrmV);
  vec3 Nv = normalize(PerturbNormal(nm, depth, T0, N0));
  vec3 Nw = PerturbNormal(nm, depth, vTanW, vNrmW);
  vec4 env = texCUBE_game(envmap_sampler, vEyeW - 2.0 * dot(vEyeW, Nw) * Nw);
  vec3 diffuse = TwoLights(vPosV, Nv, Nw, lightpos, light_col, lightpos2, light_col2);

  vec3 V = normalize(vPosV);
  vec4 lp1 = length(lightpos.xyz) > 1e-4 ? lightpos : vec4(5.0, 0.0, 10.0, 10000.0);
  vec4 lp2 = length(lightpos2.xyz) > 1e-4 ? lightpos2 : vec4(-5.0, 2.0, 10.0, 10000.0);
  vec3 d1 = lp1.xyz - vPosV;
  float r1 = max(abs(lp1.w), 1.0);
  float att1 = max(r1 - length(d1), 0.0) / r1;
  vec3 d2 = lp2.xyz - vPosV;
  float r2 = max(abs(lp2.w), 1.0);
  float att2 = max(r2 - length(d2), 0.0) / r2;

  vec3 H1 = normalize(normalize(d1) - V);
  vec3 H2 = normalize(normalize(d2) - V);
  float denom = max(1.0 - Spec_Limit, 1e-4);
  float s1 = clamp((saturate(dot(H1, Nv)) * att1 - Spec_Limit) / denom, 0.0, 1.0);
  float s2 = clamp((saturate(dot(H2, Nv)) * att2 - Spec_Limit) / denom, 0.0, 1.0);
  float lutY = clamp(Spec_Type, 0.0, 1.0);
  vec3 spec = mask * specular * (
    texture2D(Spec_Pow_sampler, vec2(s1, lutY)).rgb +
    texture2D(Spec_Pow_sampler, vec2(s2, lutY)).rgb
  );

  float facing = -dot(N0, V);
  float dRange = max(dot_max - dot_min, 1e-4);
  float dt = clamp((clamp(facing, dot_min, dot_max) - dot_min) / dRange, 0.0, 1.0);
  float dotBand = 1.0 - abs(2.0 * dt - 1.0);
  vec3 dotTerm = mask * dot_color * dotBand;

  vec3 color = albedo.rgb * (env.rgb * blend + diffuse)
             + mask * env.rgb * env.a * Env_Amb_Param
             + spec
             + dotTerm;
  gl_FragColor = ApplyStudioPost(vec4(color, 1.0), Nw, Nv, vPosV);
}
`;

export const B1_SHADERS: B1ShaderSpec[] = [
  {
    id: "b1:pls03",
    materialTypes: [110, 114, 178],
    shaderNames: [
      "modelshaderpls03_bxnxx",
      "modelshaderpls03_bxnxa",
      "modelshaderpls04_bxnxx",
      "pls03_bxnxx",
      "pls03_bxnxa",
      "pls04_bxnxx",
    ],
    vertexShader: STUDIO_VERTEX_SHADER,
    fragmentShader: FRAG_B1_PLS03,
    queue: "alphatest",
    transparent: false,
    depthWrite: true,
  },
  {
    id: "b1:obs07",
    materialTypes: [107, 109, 172, 183],
    shaderNames: [
      "modelshaderobs07_bxnxx",
      "modelshaderobs07_bxnmx",
      "modelshaderobs07_bxnxa",
      "modelshaderobs07_bxnma",
      "obs07_bxnxx",
      "obs07_bxnmx",
      "obs07_bxnxa",
      "obs07_bxnma",
    ],
    vertexShader: STUDIO_VERTEX_SHADER,
    fragmentShader: FRAG_B1_OBS07,
    queue: "alphatest",
    transparent: false,
    depthWrite: true,
  },
  {
    id: "b1:pls01",
    materialTypes: [68, 113, 132, 148, 182, 185, 186, 188],
    shaderNames: [
      "modelshaderpls01_bxxxx",
      "modelshaderpls01_bxxmx",
      "modelshaderpls01_bxxxa",
      "modelshaderpls01_bxxma",
      "modelshaderpls01_bxxmb",
      "modelshaderpls01_bxxmy",
      "modelshaderpls01_bxxxz",
      "modelshaderpls07_bxxxa",
      "pls01_bxxxx",
      "pls01_bxxmx",
      "pls01_bxxxa",
      "pls01_bxxma",
      "pls01_bxxmb",
      "pls01_bxxmy",
      "pls01_bxxxz",
      "pls07_bxxxa",
    ],
    vertexShader: STUDIO_VERTEX_SHADER,
    fragmentShader: FRAG_B1_PLS01,
    queue: "alphatest",
    transparent: false,
    depthWrite: true,
  },
  {
    id: "b1:pls05",
    materialTypes: [127, 179, 187],
    shaderNames: [
      "modelshaderpls05_bxnxx",
      "modelshaderpls05_bxnxa",
      "modelshaderpls05_bxnxb",
      "pls05_bxnxx",
      "pls05_bxnxa",
      "pls05_bxnxb",
    ],
    vertexShader: STUDIO_VERTEX_SHADER,
    fragmentShader: FRAG_B1_PLS05,
    queue: "alphatest",
    transparent: false,
    depthWrite: true,
  },
  {
    id: "b1:pls06",
    materialTypes: [135, 181, 184],
    shaderNames: [
      "modelshaderpls06_bxnxx",
      "modelshaderpls06_bxnxx2",
      "modelshaderpls06_bxnxa",
      "modelshaderpls06_bxnxb",
      "pls06_bxnxx",
      "pls06_bxnxx2",
      "pls06_bxnxa",
      "pls06_bxnxb",
    ],
    vertexShader: STUDIO_VERTEX_SHADER,
    fragmentShader: FRAG_B1_PLS06,
    queue: "opaque",
    transparent: false,
    depthWrite: true,
  },
  {
    id: "b1:pls09",
    materialTypes: [151],
    shaderNames: ["modelshaderpls09_bxxmx", "pls09_bxxmx"],
    vertexShader: STUDIO_VERTEX_SHADER,
    fragmentShader: FRAG_B1_PLS09,
    queue: "alphatest",
    transparent: false,
    depthWrite: true,
  },
  {
    id: "b1:obs00",
    materialTypes: [56, 75],
    shaderNames: [
      "modelshaderobs00_bxnxx",
      "modelshaderobs00_bxnmx",
      "obs00_bxnxx",
      "obs00_bxnmx",
    ],
    vertexShader: STUDIO_VERTEX_SHADER,
    fragmentShader: FRAG_B1_OBS00,
    queue: "alphatest",
    transparent: false,
    depthWrite: true,
  },
  {
    id: "b1:obs04",
    materialTypes: [66, 76],
    shaderNames: ["modelshaderobs04_bxxxx", "modelshaderobs04_bxxmx", "obs04_bxxxx", "obs04_bxxmx"],
    vertexShader: STUDIO_VERTEX_SHADER,
    fragmentShader: FRAG_B1_OBS04,
    queue: "alphatest",
    transparent: false,
    depthWrite: true,
  },
  {
    id: "b1:obs10",
    materialTypes: [163],
    shaderNames: ["modelshaderobs10_bxnxx", "obs10_bxnxx"],
    vertexShader: STUDIO_VERTEX_SHADER,
    fragmentShader: FRAG_B1_OBS10,
    queue: "alphatest",
    transparent: false,
    depthWrite: true,
  },
  {
    id: "b1:etc01",
    materialTypes: [60],
    shaderNames: ["modelshaderetc01_bxnxx", "etc01_bxnxx"],
    vertexShader: STUDIO_VERTEX_SHADER,
    fragmentShader: FRAG_B1_ETC01,
    queue: "blend",
    transparent: true,
    depthWrite: false,
  },
  {
    id: "b1:obs02",
    materialTypes: [64],
    shaderNames: ["modelshaderobs02_bxxxx", "obs02_bxxxx"],
    vertexShader: STUDIO_VERTEX_SHADER,
    fragmentShader: FRAG_B1_OBS02,
    queue: "alphatest",
    transparent: false,
    depthWrite: true,
  },
  {
    id: "b1:etc00",
    materialTypes: [59, 104],
    shaderNames: [
      "modelshaderetc00_bxxxx",
      "modelshaderetc00_bxxmx",
      "etc00_bxxxx",
      "etc00_bxxmx",
    ],
    vertexShader: STUDIO_VERTEX_SHADER,
    fragmentShader: FRAG_B1_ETC00,
    queue: "blend",
    transparent: true,
    depthWrite: false,
  },
  {
    id: "b1:pls02",
    materialTypes: [88, 149],
    shaderNames: [
      "modelshaderpls02_bxxxx",
      "modelshaderpls08_bxxxx",
      "pls02_bxxxx",
      "pls08_bxxxx",
    ],
    vertexShader: STUDIO_VERTEX_SHADER,
    fragmentShader: FRAG_B1_PLS02,
    queue: "blend",
    transparent: true,
    depthWrite: false,
  },
  {
    id: "b1:pls00",
    materialTypes: [67],
    shaderNames: ["modelshaderpls00_bxnxx", "pls00_bxnxx"],
    vertexShader: STUDIO_VERTEX_SHADER,
    fragmentShader: FRAG_B1_PLS00,
    queue: "alphatest",
    transparent: false,
    depthWrite: true,
  },
];
