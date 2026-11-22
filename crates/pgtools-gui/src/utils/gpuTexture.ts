import * as THREE from "three";

export const WTX1_MAGIC = 0x31585457; // "WTX1"
export const WTX2_MAGIC = 0x32585457; // "WTX2"
export const WTB2_MAGIC = 0x32425457; // "WTB2"

export const GPU_FMT = {
  RGBA8: 0,
  R8: 2,
  RG8: 3,
  RGBA16F: 4,
  BC1: 10,
  BC2: 11,
  BC3: 12,
  BC4: 13,
  BC5: 14,
  BC6H_UF16: 15,
  BC6H_SF16: 16,
  BC7: 17,
} as const;

export interface DecodedGpuSurface {
  width: number;
  height: number;
  cube: boolean;
  srgb: boolean;
  gpuFormat: number;
  faceByteLen: number;
  data: Uint8Array<ArrayBuffer>;
}

export function decodeWtxBlob(raw: ArrayBuffer): DecodedGpuSurface {
  if (raw.byteLength < 16) {
    throw new Error("WTX texture buffer shorter than 16 bytes");
  }
  const view = new DataView(raw);
  const magic = view.getUint32(0, true);

  if (magic === WTX2_MAGIC) {
    if (raw.byteLength < 24) {
      throw new Error("WTX2 texture buffer shorter than 24-byte header");
    }
    const width = view.getUint32(4, true);
    const height = view.getUint32(8, true);
    const flags = view.getUint32(12, true);
    const gpuFormat = view.getUint32(16, true);
    const faceByteLen = view.getUint32(20, true);
    const cube = (flags & 1) !== 0;
    const srgb = (flags & 2) !== 0;
    if (width === 0 || height === 0 || width > 16384 || height > 16384) {
      throw new Error(`Invalid WTX2 dimensions: ${width}x${height}`);
    }
    const data = new Uint8Array(raw, 24);
    const expected = cube ? faceByteLen * 6 : faceByteLen;
    if (data.byteLength < expected) {
      throw new Error(
        `WTX2 payload truncated: expected ${expected} bytes, got ${data.byteLength}`
      );
    }
    return { width, height, cube, srgb, gpuFormat, faceByteLen, data };
  }

  if (magic === WTX1_MAGIC) {
    const width = view.getUint32(4, true);
    const height = view.getUint32(8, true);
    const flags = view.getUint32(12, true);
    const cube = (flags & 1) !== 0;
    const faceByteLen = width * height * 4;
    const data = new Uint8Array(raw, 16);
    return {
      width,
      height,
      cube,
      srgb: false,
      gpuFormat: GPU_FMT.RGBA8,
      faceByteLen,
      data,
    };
  }

  throw new Error(`Unknown WTX magic: 0x${magic.toString(16)}`);
}

export function decodeWtb2Batch(
  raw: ArrayBuffer
): Array<{ id: string; surface: DecodedGpuSurface | null }> {
  if (raw.byteLength < 8) return [];
  const view = new DataView(raw);
  if (view.getUint32(0, true) !== WTB2_MAGIC) return [];
  const count = view.getUint32(4, true);
  const dec = new TextDecoder();
  const out: Array<{ id: string; surface: DecodedGpuSurface | null }> = [];
  let off = 8;
  for (let i = 0; i < count && off + 8 <= raw.byteLength; i++) {
    const idLen = view.getUint32(off, true);
    off += 4;
    if (off + idLen + 4 > raw.byteLength) break;
    const id = dec.decode(new Uint8Array(raw, off, idLen));
    off += idLen;
    const blobLen = view.getUint32(off, true);
    off += 4;
    if (off + blobLen > raw.byteLength) break;
    let surface: DecodedGpuSurface | null = null;
    if (blobLen >= 16) {
      try {
        surface = decodeWtxBlob(raw.slice(off, off + blobLen));
      } catch {
        surface = null;
      }
    }
    off += blobLen;
    out.push({ id, surface });
  }
  return out;
}

function gpuFormatToThreeCompressed(
  gpuFormat: number
): THREE.CompressedPixelFormat | null {
  switch (gpuFormat) {
    case GPU_FMT.BC1:
      return THREE.RGBA_S3TC_DXT1_Format;
    case GPU_FMT.BC2:
      return THREE.RGBA_S3TC_DXT3_Format;
    case GPU_FMT.BC3:
      return THREE.RGBA_S3TC_DXT5_Format;
    case GPU_FMT.BC4:
      return THREE.RED_RGTC1_Format;
    case GPU_FMT.BC5:
      return THREE.RED_GREEN_RGTC2_Format;
    case GPU_FMT.BC6H_UF16:
      return THREE.RGB_BPTC_UNSIGNED_Format;
    case GPU_FMT.BC6H_SF16:
      return THREE.RGB_BPTC_SIGNED_Format;
    case GPU_FMT.BC7:
      return THREE.RGBA_BPTC_Format;
    default:
      return null;
  }
}

function makeSingleFaceThreeTexture(
  slice: Uint8Array<ArrayBuffer>,
  width: number,
  height: number,
  gpuFormat: number
): THREE.Texture {
  const compFmt = gpuFormatToThreeCompressed(gpuFormat);
  if (compFmt !== null) {
    const mip = { data: slice, width, height };
    const tex = new THREE.CompressedTexture(
      [mip as unknown as ImageData],
      width,
      height,
      compFmt
    );
    tex.flipY = false;
    tex.colorSpace = THREE.NoColorSpace;
    tex.generateMipmaps = false;
    tex.needsUpdate = true;
    return tex;
  }

  if (gpuFormat === GPU_FMT.RGBA16F) {
    const aligned = slice.byteOffset % 2 === 0 ? slice : slice.slice();
    const u16 = new Uint16Array(
      aligned.buffer as ArrayBuffer,
      aligned.byteOffset,
      Math.floor(aligned.byteLength / 2)
    );
    const tex = new THREE.DataTexture(
      u16,
      width,
      height,
      THREE.RGBAFormat,
      THREE.HalfFloatType
    );
    tex.flipY = false;
    tex.colorSpace = THREE.NoColorSpace;
    tex.generateMipmaps = false;
    tex.needsUpdate = true;
    return tex;
  }

  if (gpuFormat === GPU_FMT.R8) {
    const tex = new THREE.DataTexture(
      slice,
      width,
      height,
      THREE.RedFormat,
      THREE.UnsignedByteType
    );
    tex.flipY = false;
    tex.colorSpace = THREE.NoColorSpace;
    tex.needsUpdate = true;
    return tex;
  }

  if (gpuFormat === GPU_FMT.RG8) {
    const tex = new THREE.DataTexture(
      slice,
      width,
      height,
      THREE.RGFormat,
      THREE.UnsignedByteType
    );
    tex.flipY = false;
    tex.colorSpace = THREE.NoColorSpace;
    tex.needsUpdate = true;
    return tex;
  }

  const expected = width * height * 4;
  const rgba =
    slice.byteLength >= expected ? slice.slice(0, expected) : new Uint8Array(expected);
  const tex = new THREE.DataTexture(rgba, width, height, THREE.RGBAFormat);
  tex.flipY = false;
  tex.colorSpace = THREE.NoColorSpace;
  tex.needsUpdate = true;
  return tex;
}

export function buildThreeTextureFromGpuSurface(
  surface: DecodedGpuSurface,
  isLut: boolean
): THREE.Texture | THREE.CubeTexture {
  const { width, height, cube, gpuFormat, faceByteLen, data } = surface;
  const w = Math.max(1, width);
  const h = Math.max(1, height);

  if (cube) {
    const faces: THREE.Texture[] = [];
    for (let f = 0; f < 6; f++) {
      const start = f * faceByteLen;
      const slice =
        start + faceByteLen <= data.byteLength
          ? data.slice(start, start + faceByteLen)
          : new Uint8Array(faceByteLen);
      faces.push(makeSingleFaceThreeTexture(slice, w, h, gpuFormat));
    }
    const cubeTex = new THREE.CubeTexture(faces as unknown as HTMLImageElement[]);
    const compFmt = gpuFormatToThreeCompressed(gpuFormat);
    if (compFmt !== null) {
      cubeTex.format = compFmt as unknown as THREE.PixelFormat;
    }
    cubeTex.flipY = false;
    cubeTex.colorSpace = THREE.NoColorSpace;
    cubeTex.magFilter = THREE.LinearFilter;
    cubeTex.minFilter = THREE.LinearFilter;
    cubeTex.generateMipmaps = false;
    cubeTex.needsUpdate = true;
    return cubeTex;
  }

  const slice =
    faceByteLen > 0 && faceByteLen <= data.byteLength
      ? data.slice(0, faceByteLen)
      : data.slice();
  const tex = makeSingleFaceThreeTexture(slice, w, h, gpuFormat);
  if (isLut) {
    tex.wrapS = THREE.ClampToEdgeWrapping;
    tex.wrapT = THREE.ClampToEdgeWrapping;
    tex.magFilter = THREE.LinearFilter;
    tex.minFilter = THREE.LinearFilter;
    tex.generateMipmaps = false;
  } else {
    tex.wrapS = THREE.RepeatWrapping;
    tex.wrapT = THREE.RepeatWrapping;
    tex.magFilter = THREE.LinearFilter;
    const isCompressed = gpuFormat >= 10 || gpuFormat === GPU_FMT.RGBA16F;
    const canMip =
      !isCompressed &&
      w >= 4 &&
      h >= 4 &&
      (w & (w - 1)) === 0 &&
      (h & (h - 1)) === 0;
    tex.minFilter = canMip ? THREE.LinearMipmapLinearFilter : THREE.LinearFilter;
    tex.generateMipmaps = canMip;
  }
  tex.needsUpdate = true;
  return tex;
}

// ============================================================================
// Shared WebGL2 2D Upload + Shader Helper (used by GpuTextureCanvas & Thumbnails)
// ============================================================================

export interface Gl2TextureExts {
  s3tc: WEBGL_compressed_texture_s3tc | null;
  rgtc: {
    COMPRESSED_RED_RGTC1_EXT: number;
    COMPRESSED_RED_GREEN_RGTC2_EXT: number;
  } | null;
  bptc: {
    COMPRESSED_RGBA_BPTC_UNORM_EXT: number;
    COMPRESSED_RGB_BPTC_SIGNED_FLOAT_EXT: number;
    COMPRESSED_RGB_BPTC_UNSIGNED_FLOAT_EXT: number;
  } | null;
}

export function queryGl2Exts(gl: WebGL2RenderingContext): Gl2TextureExts {
  return {
    s3tc: gl.getExtension("WEBGL_compressed_texture_s3tc"),
    rgtc: gl.getExtension("EXT_texture_compression_rgtc"),
    bptc: gl.getExtension("EXT_texture_compression_bptc"),
  };
}

export function uploadSurfaceToGl2(
  gl: WebGL2RenderingContext,
  tex: WebGLTexture,
  surface: DecodedGpuSurface,
  exts: Gl2TextureExts
): boolean {
  gl.bindTexture(gl.TEXTURE_2D, tex);
  gl.pixelStorei(gl.UNPACK_ALIGNMENT, 1);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.LINEAR);

  const slice =
    surface.faceByteLen > 0 && surface.faceByteLen <= surface.data.byteLength
      ? surface.data.subarray(0, surface.faceByteLen)
      : surface.data;

  const w = Math.max(1, surface.width);
  const h = Math.max(1, surface.height);

  switch (surface.gpuFormat) {
    case GPU_FMT.RGBA8:
      gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA8, w, h, 0, gl.RGBA, gl.UNSIGNED_BYTE, slice);
      return true;
    case GPU_FMT.R8:
      gl.texImage2D(gl.TEXTURE_2D, 0, gl.R8, w, h, 0, gl.RED, gl.UNSIGNED_BYTE, slice);
      return true;
    case GPU_FMT.RG8:
      gl.texImage2D(gl.TEXTURE_2D, 0, gl.RG8, w, h, 0, gl.RG, gl.UNSIGNED_BYTE, slice);
      return true;
    case GPU_FMT.RGBA16F: {
      const aligned = slice.byteOffset % 2 === 0 ? slice : slice.slice();
      const u16 = new Uint16Array(
        aligned.buffer as ArrayBuffer,
        aligned.byteOffset,
        Math.floor(aligned.byteLength / 2)
      );
      gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA16F, w, h, 0, gl.RGBA, gl.HALF_FLOAT, u16);
      return true;
    }
    case GPU_FMT.BC1:
      if (!exts.s3tc) return false;
      gl.compressedTexImage2D(gl.TEXTURE_2D, 0, exts.s3tc.COMPRESSED_RGBA_S3TC_DXT1_EXT, w, h, 0, slice);
      return true;
    case GPU_FMT.BC2:
      if (!exts.s3tc) return false;
      gl.compressedTexImage2D(gl.TEXTURE_2D, 0, exts.s3tc.COMPRESSED_RGBA_S3TC_DXT3_EXT, w, h, 0, slice);
      return true;
    case GPU_FMT.BC3:
      if (!exts.s3tc) return false;
      gl.compressedTexImage2D(gl.TEXTURE_2D, 0, exts.s3tc.COMPRESSED_RGBA_S3TC_DXT5_EXT, w, h, 0, slice);
      return true;
    case GPU_FMT.BC4:
      if (!exts.rgtc) return false;
      gl.compressedTexImage2D(gl.TEXTURE_2D, 0, exts.rgtc.COMPRESSED_RED_RGTC1_EXT, w, h, 0, slice);
      return true;
    case GPU_FMT.BC5:
      if (!exts.rgtc) return false;
      gl.compressedTexImage2D(gl.TEXTURE_2D, 0, exts.rgtc.COMPRESSED_RED_GREEN_RGTC2_EXT, w, h, 0, slice);
      return true;
    case GPU_FMT.BC6H_UF16:
      if (!exts.bptc) return false;
      gl.compressedTexImage2D(gl.TEXTURE_2D, 0, exts.bptc.COMPRESSED_RGB_BPTC_UNSIGNED_FLOAT_EXT, w, h, 0, slice);
      return true;
    case GPU_FMT.BC6H_SF16:
      if (!exts.bptc) return false;
      gl.compressedTexImage2D(gl.TEXTURE_2D, 0, exts.bptc.COMPRESSED_RGB_BPTC_SIGNED_FLOAT_EXT, w, h, 0, slice);
      return true;
    case GPU_FMT.BC7:
      if (!exts.bptc) return false;
      gl.compressedTexImage2D(gl.TEXTURE_2D, 0, exts.bptc.COMPRESSED_RGBA_BPTC_UNORM_EXT, w, h, 0, slice);
      return true;
    default:
      return false;
  }
}

export function uploadCubemapToGl2(
  gl: WebGL2RenderingContext,
  tex: WebGLTexture,
  surface: DecodedGpuSurface,
  exts: Gl2TextureExts
): boolean {
  if (!surface.cube || surface.faceByteLen <= 0) return false;
  gl.bindTexture(gl.TEXTURE_CUBE_MAP, tex);
  gl.pixelStorei(gl.UNPACK_ALIGNMENT, 1);
  gl.pixelStorei(gl.UNPACK_FLIP_Y_WEBGL, 0);
  gl.texParameteri(gl.TEXTURE_CUBE_MAP, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
  gl.texParameteri(gl.TEXTURE_CUBE_MAP, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
  gl.texParameteri(gl.TEXTURE_CUBE_MAP, gl.TEXTURE_WRAP_R, gl.CLAMP_TO_EDGE);
  gl.texParameteri(gl.TEXTURE_CUBE_MAP, gl.TEXTURE_MIN_FILTER, gl.LINEAR);
  gl.texParameteri(gl.TEXTURE_CUBE_MAP, gl.TEXTURE_MAG_FILTER, gl.LINEAR);

  const w = Math.max(1, surface.width);
  const h = Math.max(1, surface.height);
  const faceLen = surface.faceByteLen;

  for (let f = 0; f < 6; f++) {
    const target = gl.TEXTURE_CUBE_MAP_POSITIVE_X + f;
    const start = f * faceLen;
    if (start + faceLen > surface.data.byteLength) return false;
    const slice = surface.data.subarray(start, start + faceLen);

    switch (surface.gpuFormat) {
      case GPU_FMT.RGBA8:
        gl.texImage2D(target, 0, gl.RGBA8, w, h, 0, gl.RGBA, gl.UNSIGNED_BYTE, slice);
        break;
      case GPU_FMT.R8:
        gl.texImage2D(target, 0, gl.R8, w, h, 0, gl.RED, gl.UNSIGNED_BYTE, slice);
        break;
      case GPU_FMT.RG8:
        gl.texImage2D(target, 0, gl.RG8, w, h, 0, gl.RG, gl.UNSIGNED_BYTE, slice);
        break;
      case GPU_FMT.RGBA16F: {
        const aligned = slice.byteOffset % 2 === 0 ? slice : slice.slice();
        const u16 = new Uint16Array(
          aligned.buffer as ArrayBuffer,
          aligned.byteOffset,
          Math.floor(aligned.byteLength / 2)
        );
        gl.texImage2D(target, 0, gl.RGBA16F, w, h, 0, gl.RGBA, gl.HALF_FLOAT, u16);
        break;
      }
      case GPU_FMT.BC1:
        if (!exts.s3tc) return false;
        gl.compressedTexImage2D(target, 0, exts.s3tc.COMPRESSED_RGBA_S3TC_DXT1_EXT, w, h, 0, slice);
        break;
      case GPU_FMT.BC2:
        if (!exts.s3tc) return false;
        gl.compressedTexImage2D(target, 0, exts.s3tc.COMPRESSED_RGBA_S3TC_DXT3_EXT, w, h, 0, slice);
        break;
      case GPU_FMT.BC3:
        if (!exts.s3tc) return false;
        gl.compressedTexImage2D(target, 0, exts.s3tc.COMPRESSED_RGBA_S3TC_DXT5_EXT, w, h, 0, slice);
        break;
      case GPU_FMT.BC4:
        if (!exts.rgtc) return false;
        gl.compressedTexImage2D(target, 0, exts.rgtc.COMPRESSED_RED_RGTC1_EXT, w, h, 0, slice);
        break;
      case GPU_FMT.BC5:
        if (!exts.rgtc) return false;
        gl.compressedTexImage2D(target, 0, exts.rgtc.COMPRESSED_RED_GREEN_RGTC2_EXT, w, h, 0, slice);
        break;
      case GPU_FMT.BC6H_UF16:
        if (!exts.bptc) return false;
        gl.compressedTexImage2D(target, 0, exts.bptc.COMPRESSED_RGB_BPTC_UNSIGNED_FLOAT_EXT, w, h, 0, slice);
        break;
      case GPU_FMT.BC6H_SF16:
        if (!exts.bptc) return false;
        gl.compressedTexImage2D(target, 0, exts.bptc.COMPRESSED_RGB_BPTC_SIGNED_FLOAT_EXT, w, h, 0, slice);
        break;
      case GPU_FMT.BC7:
        if (!exts.bptc) return false;
        gl.compressedTexImage2D(target, 0, exts.bptc.COMPRESSED_RGBA_BPTC_UNORM_EXT, w, h, 0, slice);
        break;
      default:
        return false;
    }
  }
  return true;
}

export const GL2_CUBEMAP_SKY_VS = `#version 300 es
out vec2 vNdc;
void main() {
  vec2 pos = vec2((gl_VertexID == 1) ? 3.0 : -1.0, (gl_VertexID == 2) ? 3.0 : -1.0);
  vNdc = pos;
  gl_Position = vec4(pos, 0.0, 1.0);
}
`;

export const GL2_CUBEMAP_SKY_FS = `#version 300 es
precision highp float;
in vec2 vNdc;
uniform samplerCube uCube;
uniform vec3 uForward;
uniform vec3 uRight;
uniform vec3 uUp;
uniform float uAspect;
uniform float uTanHalfFov;
uniform int uFormatMode;
uniform int uChannelMode;
uniform int uOpaque;
uniform float uExposure;
uniform float uGamma;
out vec4 outColor;

void main() {
  vec3 dir = normalize(
    uForward + uRight * (vNdc.x * uAspect * uTanHalfFov) + uUp * (vNdc.y * uTanHalfFov)
  );
  vec4 s = texture(uCube, dir);
  vec3 col = s.rgb;
  float alpha = clamp(s.a, 0.0, 1.0);
  if (uFormatMode == 1) {
    col = vec3(s.r);
    alpha = 1.0;
  } else if (uFormatMode == 2) {
    col = vec3(s.r, s.g, 0.0);
    alpha = 1.0;
  } else if (uFormatMode == 4) {
    float nx = s.r * 2.0 - 1.0;
    float ny = s.g * 2.0 - 1.0;
    float nz = sqrt(max(1.0 - (nx * nx + ny * ny), 0.0));
    col = vec3(s.r, s.g, nz * 0.5 + 0.5);
    alpha = 1.0;
  }
  col = max(col, vec3(0.0)) * uExposure;
  if (uFormatMode == 3) {
    col = col / (vec3(1.0) + col);
  }
  col = pow(max(col, vec3(0.0)), vec3(1.0 / max(uGamma, 0.1)));

  if (uChannelMode == 2) {
    col = vec3(col.r);
  } else if (uChannelMode == 3) {
    col = vec3(col.g);
  } else if (uChannelMode == 4) {
    col = vec3(col.b);
  } else if (uChannelMode == 5) {
    col = vec3(uOpaque != 0 ? 1.0 : alpha);
  }
  outColor = vec4(col, 1.0);
}
`;

export const GL2_QUAD_VS = `#version 300 es
out vec2 vUv;
void main() {
  vec2 pos = vec2((gl_VertexID == 1) ? 3.0 : -1.0, (gl_VertexID == 2) ? 3.0 : -1.0);
  vUv = vec2((pos.x + 1.0) * 0.5, 1.0 - (pos.y + 1.0) * 0.5);
  gl_Position = vec4(pos, 0.0, 1.0);
}
`;

// uFormatMode: 0=RGBA, 1=R-only grayscale (R8/BC4), 2=RG8, 3=HDR Reinhard (RGBA16F/BC6H), 4=BC5 normal Z reconstruct
// uChannelMode: 0=RGBA, 1=RGB, 2=R, 3=G, 4=B, 5=A
export const GL2_QUAD_FS = `#version 300 es
precision highp float;
in vec2 vUv;
uniform sampler2D uTex;
uniform int uFormatMode;
uniform int uChannelMode;
uniform int uOpaque;
out vec4 outColor;

vec3 reinhard(vec3 v) {
  vec3 c = max(v, vec3(0.0));
  return c / (vec3(1.0) + c);
}

void main() {
  vec4 s = texture(uTex, vUv);
  if (uFormatMode == 1) {
    s = vec4(s.r, s.r, s.r, 1.0);
  } else if (uFormatMode == 2) {
    s = vec4(s.r, s.g, 0.0, 1.0);
  } else if (uFormatMode == 3) {
    s = vec4(reinhard(s.rgb), clamp(s.a, 0.0, 1.0));
  } else if (uFormatMode == 4) {
    float nx = s.r * 2.0 - 1.0;
    float ny = s.g * 2.0 - 1.0;
    float nz = sqrt(max(1.0 - (nx * nx + ny * ny), 0.0));
    s = vec4(s.r, s.g, nz * 0.5 + 0.5, 1.0);
  }

  vec4 c = s;
  if (uChannelMode == 1) {
    c = vec4(s.rgb, 1.0);
  } else if (uChannelMode == 2) {
    c = vec4(s.r, s.r, s.r, 1.0);
  } else if (uChannelMode == 3) {
    c = vec4(s.g, s.g, s.g, 1.0);
  } else if (uChannelMode == 4) {
    c = vec4(s.b, s.b, s.b, 1.0);
  } else if (uChannelMode == 5) {
    c = vec4(s.a, s.a, s.a, 1.0);
  }
  if (uOpaque != 0) {
    c.a = 1.0;
  }
  outColor = c;
}
`;

export function formatModeForGpuFormat(gpuFormat: number): number {
  if (gpuFormat === GPU_FMT.R8 || gpuFormat === GPU_FMT.BC4) return 1;
  if (gpuFormat === GPU_FMT.RG8) return 2;
  if (gpuFormat === GPU_FMT.BC5) return 4;
  if (
    gpuFormat === GPU_FMT.RGBA16F ||
    gpuFormat === GPU_FMT.BC6H_UF16 ||
    gpuFormat === GPU_FMT.BC6H_SF16
  ) {
    return 3;
  }
  return 0;
}

export function channelModeToInt(mode: string): number {
  switch (mode) {
    case "RGB":
      return 1;
    case "R":
      return 2;
    case "G":
      return 3;
    case "B":
      return 4;
    case "A":
      return 5;
    default:
      return 0;
  }
}

export function compileGl2Program(
  gl: WebGL2RenderingContext,
  vsSrc: string,
  fsSrc: string
): WebGLProgram | null {
  const vs = gl.createShader(gl.VERTEX_SHADER);
  const fs = gl.createShader(gl.FRAGMENT_SHADER);
  if (!vs || !fs) return null;
  gl.shaderSource(vs, vsSrc);
  gl.compileShader(vs);
  gl.shaderSource(fs, fsSrc);
  gl.compileShader(fs);
  const prog = gl.createProgram();
  if (!prog) return null;
  gl.attachShader(prog, vs);
  gl.attachShader(prog, fs);
  gl.linkProgram(prog);
  gl.deleteShader(vs);
  gl.deleteShader(fs);
  if (!gl.getProgramParameter(prog, gl.LINK_STATUS)) {
    gl.deleteProgram(prog);
    return null;
  }
  return prog;
}

// Persistent offscreen WebGL2 renderer for instant GPU-direct Asset Browser thumbnails
interface SharedThumbRenderer {
  canvas: HTMLCanvasElement;
  gl: WebGL2RenderingContext;
  exts: Gl2TextureExts;
  program: WebGLProgram;
  tex: WebGLTexture;
  uTex: WebGLUniformLocation | null;
  uFormatMode: WebGLUniformLocation | null;
  uChannelMode: WebGLUniformLocation | null;
  uOpaque: WebGLUniformLocation | null;
}

let sharedThumbRenderer: SharedThumbRenderer | null = null;

function getThumbRenderer(): SharedThumbRenderer | null {
  if (sharedThumbRenderer) return sharedThumbRenderer;
  const canvas = document.createElement("canvas");
  canvas.width = 128;
  canvas.height = 128;
  const gl = canvas.getContext("webgl2", {
    alpha: true,
    premultipliedAlpha: false,
    preserveDrawingBuffer: true,
    antialias: false,
  });
  if (!gl) return null;
  const program = compileGl2Program(gl, GL2_QUAD_VS, GL2_QUAD_FS);
  const tex = gl.createTexture();
  if (!program || !tex) return null;
  sharedThumbRenderer = {
    canvas,
    gl,
    exts: queryGl2Exts(gl),
    program,
    tex,
    uTex: gl.getUniformLocation(program, "uTex"),
    uFormatMode: gl.getUniformLocation(program, "uFormatMode"),
    uChannelMode: gl.getUniformLocation(program, "uChannelMode"),
    uOpaque: gl.getUniformLocation(program, "uOpaque"),
  };
  return sharedThumbRenderer;
}

export function renderGpuSurfaceToDataUrl(
  surface: DecodedGpuSurface,
  maxSize = 128
): string | null {
  const r = getThumbRenderer();
  if (!r) return null;
  const aspect = surface.width / Math.max(1, surface.height);
  let outW = maxSize;
  let outH = maxSize;
  if (aspect > 1) {
    outH = Math.max(1, Math.round(maxSize / aspect));
  } else if (aspect < 1) {
    outW = Math.max(1, Math.round(maxSize * aspect));
  }
  if (r.canvas.width !== outW || r.canvas.height !== outH) {
    r.canvas.width = outW;
    r.canvas.height = outH;
  }

  const { gl } = r;
  if (!uploadSurfaceToGl2(gl, r.tex, surface, r.exts)) {
    return null;
  }
  gl.viewport(0, 0, outW, outH);
  gl.clearColor(0, 0, 0, 0);
  gl.clear(gl.COLOR_BUFFER_BIT);
  gl.useProgram(r.program);
  gl.activeTexture(gl.TEXTURE0);
  gl.bindTexture(gl.TEXTURE_2D, r.tex);
  gl.uniform1i(r.uTex, 0);
  gl.uniform1i(r.uFormatMode, formatModeForGpuFormat(surface.gpuFormat));
  gl.uniform1i(r.uChannelMode, 0);
  gl.uniform1i(r.uOpaque, 0);
  gl.drawArrays(gl.TRIANGLES, 0, 3);
  return r.canvas.toDataURL("image/png");
}
