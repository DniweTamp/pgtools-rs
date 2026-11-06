// [wmb-studio BEGIN phase2]
/**
 * Shared GLSL vertex and fragment shader building blocks for the PGTools Studio 3D Viewport.
 *
 * Implements:
 * - Studio lighting rig (Section 2.5):
 *     Key directional light: intensity 1.0, direction (-0.5, -0.8, -0.6) normalized (Y-up world space)
 *     Fill directional light: intensity 0.4, direction (0.6, -0.2, 0.4) normalized
 *     Ambient: intensity 0.3
 * - Original Bayonetta 1 HLSL helpers (`PerturbNormal`, `texCUBE_game`, `PointLight`, `TwoLights`, `FinalOut`)
 * - Yellow fresnel selection rim (`#facc15` at 0.35 strength)
 * - Normals view and Texture slot channel debug modes (`RGB | R | G | B | A`)
 */

export const STUDIO_VERTEX_SHADER = /* glsl */ `
attribute vec4 tangent;
attribute vec2 uv1;
attribute vec2 uv2;
attribute vec4 color;

varying vec4 vUv;       // xy = UV0 (mapping), zw = UV1 (mapping2)
varying vec2 vUv2;      // xy = UV2 (mapping3)
varying vec3 vPosV;     // view-space position
varying vec3 vPosW;     // world-space position
varying vec3 vTanV;     // view-space tangent
varying vec3 vNrmV;     // view-space normal
varying vec3 vTanW;     // world-space tangent (normalized)
varying vec3 vNrmW;     // world-space normal (normalized)
varying vec3 vEyeW;     // world-space (pos - cameraPosition), unnormalized
varying vec4 vColor;    // vertex color RGBA

void main() {
  vec4 worldPos = modelMatrix * vec4(position, 1.0);
  vec4 viewPos = modelViewMatrix * vec4(position, 1.0);
  gl_Position = projectionMatrix * viewPos;

  mat3 worldMat3 = mat3(modelMatrix);
  mat3 viewMat3 = mat3(modelViewMatrix);

  vec3 nrm = length(normal) > 1e-5 ? normal : vec3(0.0, 1.0, 0.0);
  vec3 tanXYZ = length(tangent.xyz) > 2e-2 ? tangent.xyz : vec3(1.0, 0.0, 0.0);

  vPosV = viewPos.xyz;
  vPosW = worldPos.xyz;
  vTanV = viewMat3 * tanXYZ;
  vNrmV = viewMat3 * nrm;
  vTanW = normalize(worldMat3 * tanXYZ);
  vNrmW = normalize(worldMat3 * nrm);
  vEyeW = worldPos.xyz - cameraPosition;
  vUv = vec4(uv, uv1);
  vUv2 = uv2;
  vColor = color;
}
`;

export const STUDIO_FRAGMENT_HEADER = /* glsl */ `
precision highp float;

varying vec4 vUv;
varying vec2 vUv2;
varying vec3 vPosV;
varying vec3 vPosW;
varying vec3 vTanV;
varying vec3 vNrmV;
varying vec3 vTanW;
varying vec3 vNrmW;
varying vec3 vEyeW;
varying vec4 vColor;

// Studio & viewport debug uniforms
uniform float uSelectionHighlight; // 0.0 or 1.0
uniform int uDebugMode;            // 0 = shaded, 1 = normals, 2 = texture slot channel preview
uniform int uDebugChannel;         // 0 = RGB, 1 = R, 2 = G, 3 = B, 4 = A
uniform int uDebugIsCube;          // 0 = 2D, 1 = Cube
uniform int uDebugUvMode;          // 0 = UV0, 1 = Lightmap UV (UV1/UV2), 2 = Split (RG on UV0, BA on Lightmap UV)
uniform vec2 uDebugUvTile;         // Tiling applied to debug 2D preview
uniform int uUseSecondaryNormals;  // 0 = off (default), 1 = NormalMap2/3 affect surface normals
uniform sampler2D uDebugTex2D;
uniform samplerCube uDebugTexCube;

// Resolves the secondary / lightmap UV coordinate from UV1 (mapping2) or UV2 (mapping3),
// falling back to UV0 when neither attribute carries distinct non-zero coordinates.
vec2 ResolveLightMapUv(vec4 uv01, vec2 uv2) {
  vec2 u0 = uv01.xy;
  vec2 u1 = uv01.zw;
  bool u1Valid = dot(u1, u1) > 1e-6 && distance(u1, u0) > 1e-4;
  bool u2Valid = dot(uv2, uv2) > 1e-6 && distance(uv2, u0) > 1e-4;
  return u1Valid ? u1 : (u2Valid ? uv2 : u0);
}

// Studio lighting constants (Section 2.5: Y-up world space, neutral white)
const vec3 STUDIO_KEY_TO_LIGHT = vec3(0.42640143, 0.68224229, 0.59696201);   // normalize(-(-0.5, -0.8, -0.6))
const vec3 STUDIO_FILL_TO_LIGHT = vec3(-0.80178373, 0.26726124, -0.53452248); // normalize(-(0.6, -0.2, 0.4))
const float STUDIO_KEY_INTENSITY = 1.0;
const float STUDIO_FILL_INTENSITY = 0.4;
const float STUDIO_AMBIENT_INTENSITY = 0.3;

// HLSL compatibility helpers
float saturate(float x) {
  return clamp(x, 0.0, 1.0);
}
vec2 saturate(vec2 x) {
  return clamp(x, vec2(0.0), vec2(1.0));
}
vec3 saturate(vec3 x) {
  return clamp(x, vec3(0.0), vec3(1.0));
}
vec4 saturate(vec4 x) {
  return clamp(x, vec4(0.0), vec4(1.0));
}

// Original HLSL:
// float4 texCUBE_game(samplerCUBE s, float3 v) { return texCUBE(s, float3(v.x, v.y, -v.z)); }
vec4 texCUBE_game(samplerCube s, vec3 v) {
  vec3 dir = length(v) > 1e-5 ? v : vec3(0.0, 1.0, 0.0);
  return textureCube(s, vec3(dir.x, dir.y, -dir.z));
}

// Original HLSL:
// float3 PerturbNormal(float4 nm, float depth, float3 T, float3 N) {
//     float2 xy = (nm.xy * 2 - 1) * depth;
//     float  z  = sqrt(abs(1 - dot(xy, xy)));
//     return xy.x * T + xy.y * cross(N, T) + z * N;
// }
vec3 PerturbNormal(vec4 nm, float depth, vec3 T, vec3 N) {
  vec2 xy = (nm.xy * 2.0 - 1.0) * depth;
  float z = sqrt(abs(1.0 - dot(xy, xy)));
  return xy.x * T + xy.y * cross(N, T) + z * N;
}

// Original HLSL:
// float3 PointLight(float3 posV, float3 N, float4 lp, float3 col) {
//     float3 d    = lp.xyz - posV;
//     float  dist = length(d);
//     float  att  = max(lp.w - dist, 0) / lp.w;
//     return saturate(dot(d / dist, N)) * col * att;
// }
vec3 PointLight(vec3 posV, vec3 N, vec4 lp, vec3 col) {
  vec3 d = lp.xyz - posV;
  float dist = max(length(d), 1e-5);
  float radius = max(abs(lp.w), 1e-5);
  float att = max(radius - dist, 0.0) / radius;
  return saturate(dot(d / dist, N)) * col * att;
}

// Evaluates the two point lights from the WMB material (when non-zero) plus a subtle
// studio directional fill so models remain readable even when material point lights are (0,0,0).
vec3 TwoLights(vec3 posV, vec3 Nv, vec3 Nw, vec4 lp1, vec3 lc1, vec4 lp2, vec3 lc2) {
  vec3 pt = PointLight(posV, Nv, lp1, lc1) + PointLight(posV, Nv, lp2, lc2);
  vec3 nW = normalize(Nw);
  float keyNdotL = saturate(dot(nW, STUDIO_KEY_TO_LIGHT));
  float fillNdotL = saturate(dot(nW, STUDIO_FILL_TO_LIGHT));
  vec3 studioDirect = vec3(keyNdotL * 0.32 * STUDIO_KEY_INTENSITY + fillNdotL * 0.25 * STUDIO_FILL_INTENSITY);
  return pt + studioDirect;
}

// Re-orthogonalized world normal used by non-normal-mapped B1 shaders (pls01, pls09, obs04, etc00, obs02):
// Nw = normalize(cross(Tw, normalize(W * cross(T, N))))
vec3 ReorthoWorldNormal(vec3 tanW, vec3 nrmW) {
  vec3 T = normalize(tanW);
  vec3 N = normalize(nrmW);
  vec3 B = cross(T, N);
  if (length(B) < 1e-5) {
    return N;
  }
  return normalize(cross(T, normalize(B)));
}

// Applies selection rim highlight (#facc15 at 0.35 strength) and debug overrides.
vec4 ApplyStudioPost(vec4 shadedColor, vec3 worldNormal, vec3 viewNormal, vec3 posV) {
  if (uDebugMode == 1) {
    vec3 n = normalize(worldNormal) * 0.5 + 0.5;
    return vec4(n, 1.0);
  }
  if (uDebugMode == 2) {
    vec2 lmUv = ResolveLightMapUv(vUv, vUv2);
    vec2 tile = dot(uDebugUvTile, uDebugUvTile) > 1e-6 ? uDebugUvTile : vec2(1.0, 1.0);
    vec4 samp;
    if (uDebugIsCube == 1) {
      samp = texCUBE_game(uDebugTexCube, normalize(worldNormal));
    } else if (uDebugUvMode == 1) {
      samp = texture2D(uDebugTex2D, lmUv * tile);
    } else if (uDebugUvMode == 2) {
      vec4 sampUv0 = texture2D(uDebugTex2D, vUv.xy * tile);
      vec4 sampLm  = texture2D(uDebugTex2D, lmUv * tile);
      samp = vec4(sampUv0.rg, sampLm.ba);
    } else {
      samp = texture2D(uDebugTex2D, vUv.xy * tile);
    }
    vec3 ch = samp.rgb;
    if (uDebugChannel == 1) ch = vec3(samp.r);
    else if (uDebugChannel == 2) ch = vec3(samp.g);
    else if (uDebugChannel == 3) ch = vec3(samp.b);
    else if (uDebugChannel == 4) ch = vec3(samp.a);
    return vec4(ch, 1.0);
  }

  vec3 outRgb = shadedColor.rgb;
  if (uSelectionHighlight > 0.5) {
    vec3 V = normalize(-posV);
    vec3 Nv = normalize(viewNormal);
    float fresnel = pow(1.0 - saturate(abs(dot(Nv, V))), 2.2);
    vec3 rimColor = vec3(0.98039, 0.80000, 0.08235); // #facc15
    outRgb = mix(outRgb, rimColor, saturate(0.18 + 0.55 * fresnel) * 0.35);
  }
  return vec4(outRgb, shadedColor.a);
}
`;
// [wmb-studio END]
