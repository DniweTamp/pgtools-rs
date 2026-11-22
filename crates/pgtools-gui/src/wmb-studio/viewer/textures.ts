import * as THREE from "three";
import { buildThreeTextureFromGpuSurface } from "../../utils/gpuTexture";
import {
  DecodedWtx1Texture,
  getPreTexture,
  getWmbTexture,
  TextureRefDesc,
  TextureSlotDesc,
  TextureSlotRole,
} from "../contract";
import { isNeutralLightmapId, STUDIO_KEY_LIGHT_DIR } from "../shaders/common";
import { isB1GlobalCubeId, isB1GlobalSpecLutId } from "../shaders/registry";

const VIEWPORT_MAX_ALBEDO_DIM = 2048;
const VIEWPORT_MAX_SECONDARY_DIM = 1024;
const VIEWPORT_MAX_CUBE_DIM = 512;
const VIEWPORT_MAX_LUT_DIM = 512;
const MAX_2D_CACHE_ENTRIES = 64;
const MAX_CUBE_CACHE_ENTRIES = 24;

function makeSolidDataTexture(
  r: number,
  g: number,
  b: number,
  a: number
): THREE.DataTexture {
  const data = new Uint8Array([r, g, b, a]);
  const tex = new THREE.DataTexture(data, 1, 1, THREE.RGBAFormat);
  tex.flipY = false;
  tex.colorSpace = THREE.NoColorSpace;
  tex.wrapS = THREE.RepeatWrapping;
  tex.wrapT = THREE.RepeatWrapping;
  tex.magFilter = THREE.LinearFilter;
  tex.minFilter = THREE.LinearFilter;
  tex.needsUpdate = true;
  return tex;
}

function makeMissingCheckerTexture(): THREE.DataTexture {
  const size = 16;
  const data = new Uint8Array(size * size * 4);
  for (let y = 0; y < size; y++) {
    for (let x = 0; x < size; x++) {
      const i = (y * size + x) * 4;
      const isMagenta = ((x >> 2) + (y >> 2)) % 2 === 0;
      data[i + 0] = isMagenta ? 255 : 24;
      data[i + 1] = isMagenta ? 0 : 24;
      data[i + 2] = isMagenta ? 220 : 28;
      data[i + 3] = 255;
    }
  }
  const tex = new THREE.DataTexture(data, size, size, THREE.RGBAFormat);
  tex.flipY = false;
  tex.colorSpace = THREE.NoColorSpace;
  tex.wrapS = THREE.RepeatWrapping;
  tex.wrapT = THREE.RepeatWrapping;
  tex.magFilter = THREE.NearestFilter;
  tex.minFilter = THREE.NearestFilter;
  tex.needsUpdate = true;
  return tex;
}

/**
 * Generates a 256x32 procedural Spec_Pow LUT matching Bayonetta 1's 16 paired power curves
 * so specular highlights on `pls01`, `pls05`, and `pls09` work even when `core.dat` is unconfigured.
 */
function makeProceduralSpecLutTexture(): THREE.DataTexture {
  const width = 256;
  const height = 32;
  const data = new Uint8Array(width * height * 4);
  for (let y = 0; y < height; y++) {
    const curveIdx = Math.floor(y / 2);
    const exponent = 2.0 + curveIdx * 3.5;
    for (let x = 0; x < width; x++) {
      const u = x / (width - 1);
      const val = x === 0 ? 0.0 : Math.pow(u, exponent);
      const byte = Math.max(0, Math.min(255, Math.round(val * 255)));
      const idx = (y * width + x) * 4;
      data[idx + 0] = byte;
      data[idx + 1] = byte;
      data[idx + 2] = byte;
      data[idx + 3] = 255;
    }
  }
  const tex = new THREE.DataTexture(data, width, height, THREE.RGBAFormat);
  tex.flipY = false;
  tex.colorSpace = THREE.NoColorSpace;
  tex.wrapS = THREE.ClampToEdgeWrapping;
  tex.wrapT = THREE.ClampToEdgeWrapping;
  tex.magFilter = THREE.LinearFilter;
  tex.minFilter = THREE.LinearFilter;
  tex.generateMipmaps = false;
  tex.needsUpdate = true;
  return tex;
}

/**
 * Generates a 16x16x6 procedural studio CubeTexture with soft sky/ground gradient in RGB
 * and specular hotspot glints in Alpha (used by B1 `env.rgb * env.a * Env_Amb_Param`).
 */
function makeProceduralStudioCube(hardHotspot: boolean): THREE.CubeTexture {
  const size = 16;
  const faces: THREE.DataTexture[] = [];
  // Cube face directions: +X, -X, +Y, -Y, +Z, -Z
  const faceDirs: Array<(u: number, v: number) => [number, number, number]> = [
    (u, v) => [1, -v, -u],
    (u, v) => [-1, -v, u],
    (u, v) => [u, 1, v],
    (u, v) => [u, -1, -v],
    (u, v) => [u, -v, 1],
    (u, v) => [-u, -v, -1],
  ];

  const [lx, ly, posLz] = STUDIO_KEY_LIGHT_DIR;
  const lz = -posLz;

  for (let f = 0; f < 6; f++) {
    const data = new Uint8Array(size * size * 4);
    for (let y = 0; y < size; y++) {
      const v = (y / (size - 1)) * 2 - 1;
      for (let x = 0; x < size; x++) {
        const u = (x / (size - 1)) * 2 - 1;
        const [dx, dy, dz] = faceDirs[f](u, v);
        const len = Math.hypot(dx, dy, dz) || 1;
        const nx = dx / len;
        const ny = dy / len;
        const nz = dz / len;

        const skyT = Math.max(0, Math.min(1, ny * 0.5 + 0.5));
        const dotKey = Math.max(0, nx * lx + ny * ly + nz * lz);
        const specBoost = hardHotspot
          ? Math.pow(dotKey, 8.0) * 0.45
          : Math.pow(dotKey, 2.5) * 0.2;

        const r = Math.min(1, 0.34 + 0.48 * skyT + specBoost);
        const g = Math.min(1, 0.36 + 0.48 * skyT + specBoost);
        const b = Math.min(1, 0.4 + 0.5 * skyT + specBoost);
        const hotspotA = hardHotspot
          ? Math.pow(dotKey, 6.0)
          : Math.pow(dotKey, 3.0) * 0.65;

        const i = (y * size + x) * 4;
        data[i + 0] = Math.round(r * 255);
        data[i + 1] = Math.round(g * 255);
        data[i + 2] = Math.round(b * 255);
        data[i + 3] = Math.round(Math.max(0, Math.min(1, hotspotA)) * 255);
      }
    }
    const faceTex = new THREE.DataTexture(data, size, size, THREE.RGBAFormat);
    faceTex.flipY = false;
    faceTex.colorSpace = THREE.NoColorSpace;
    faceTex.needsUpdate = true;
    faces.push(faceTex);
  }

  const cube = new THREE.CubeTexture(faces as unknown as HTMLImageElement[]);
  cube.flipY = false;
  cube.colorSpace = THREE.NoColorSpace;
  cube.magFilter = THREE.LinearFilter;
  cube.minFilter = THREE.LinearFilter;
  cube.generateMipmaps = false;
  cube.needsUpdate = true;
  return cube;
}

function buildThreeTextureFromWtx1(
  decoded: DecodedWtx1Texture,
  isLut: boolean
): THREE.Texture | THREE.CubeTexture {
  return buildThreeTextureFromGpuSurface(decoded, isLut);
}

export class ViewerTextureCache {
  public readonly white2D: THREE.DataTexture;
  public readonly black2D: THREE.DataTexture;
  public readonly flatNormal2D: THREE.DataTexture;
  public readonly greyRelief2D: THREE.DataTexture;
  public readonly defaultMask2D: THREE.DataTexture;
  public readonly specLut2D: THREE.DataTexture;
  public readonly studioCubeHard: THREE.CubeTexture;
  public readonly studioCubeSoft: THREE.CubeTexture;
  public readonly missingChecker2D: THREE.DataTexture;

  private readonly tex2DCache = new Map<string, THREE.Texture>();
  private readonly cubeCache = new Map<string, THREE.CubeTexture>();
  private readonly inFlight = new Map<
    string,
    Promise<THREE.Texture | THREE.CubeTexture>
  >();
  private disposed = false;

  constructor() {
    this.white2D = makeSolidDataTexture(255, 255, 255, 255);
    this.black2D = makeSolidDataTexture(0, 0, 0, 255);
    this.flatNormal2D = makeSolidDataTexture(128, 128, 255, 255);
    this.greyRelief2D = makeSolidDataTexture(128, 128, 128, 255);
    // Neutral PBR mask map (R=0 metallic, G=115 ~0.45 roughness, B=255 AO=1, A=255)
    this.defaultMask2D = makeSolidDataTexture(0, 115, 255, 255);
    this.specLut2D = makeProceduralSpecLutTexture();
    this.studioCubeHard = makeProceduralStudioCube(true);
    this.studioCubeSoft = makeProceduralStudioCube(false);
    this.missingChecker2D = makeMissingCheckerTexture();
  }

  private get2DLru(key: string): THREE.Texture | undefined {
    const existing = this.tex2DCache.get(key);
    if (existing) {
      this.tex2DCache.delete(key);
      this.tex2DCache.set(key, existing);
    }
    return existing;
  }

  private set2DLru(key: string, tex: THREE.Texture): void {
    const prev = this.tex2DCache.get(key);
    if (prev && prev !== tex) {
      prev.dispose();
    }
    this.tex2DCache.delete(key);
    while (this.tex2DCache.size >= MAX_2D_CACHE_ENTRIES) {
      const oldestKey = this.tex2DCache.keys().next().value;
      if (oldestKey === undefined) break;
      const oldest = this.tex2DCache.get(oldestKey);
      this.tex2DCache.delete(oldestKey);
      oldest?.dispose();
    }
    this.tex2DCache.set(key, tex);
  }

  private getCubeLru(key: string): THREE.CubeTexture | undefined {
    const existing = this.cubeCache.get(key);
    if (existing) {
      this.cubeCache.delete(key);
      this.cubeCache.set(key, existing);
    }
    return existing;
  }

  private setCubeLru(key: string, cube: THREE.CubeTexture): void {
    const prev = this.cubeCache.get(key);
    if (prev && prev !== cube) {
      prev.dispose();
    }
    this.cubeCache.delete(key);
    while (this.cubeCache.size >= MAX_CUBE_CACHE_ENTRIES) {
      const oldestKey = this.cubeCache.keys().next().value;
      if (oldestKey === undefined) break;
      const oldest = this.cubeCache.get(oldestKey);
      this.cubeCache.delete(oldestKey);
      oldest?.dispose();
    }
    this.cubeCache.set(key, cube);
  }

  public invalidatePreTextures(): void {
    for (const [k, tex] of Array.from(this.tex2DCache.entries())) {
      if (k.startsWith("pre:")) {
        tex.dispose();
        this.tex2DCache.delete(k);
      }
    }
    for (const [k, cube] of Array.from(this.cubeCache.entries())) {
      if (k.startsWith("pre:")) {
        cube.dispose();
        this.cubeCache.delete(k);
      }
    }
  }

  public getFallback2D(
    role: TextureSlotRole,
    slot?: TextureSlotDesc
  ): THREE.Texture {
    if (slot) {
      const u = slot.textureId >>> 0;
      if (u === 0x80000000 || u === 0x80000004 || isNeutralLightmapId(u)) {
        return this.white2D;
      }
      if (u === 0x132ed5c3) {
        return this.black2D;
      }
      if (
        u === 0x80000005 ||
        u === 0x8000000b ||
        u === 0x7fd4929a ||
        u === 0x3176288c
      ) {
        return this.flatNormal2D;
      }
      if (u === 0x80000002 || u === 0x62fc2788) {
        return this.greyRelief2D;
      }
      if (u === 0x6a9bfb55) {
        return this.defaultMask2D;
      }
      if (isB1GlobalSpecLutId(u) || u === 0x0da7bff1 || u === 0x50e2db38) {
        return this.specLut2D;
      }
      if (!slot.present && !slot.global) {
        if (role === "normal") return this.flatNormal2D;
        if (role === "lut") return this.specLut2D;
        if (role === "mask" || role === "spec") return this.defaultMask2D;
        return this.missingChecker2D;
      }
    }
    if (role === "normal") return this.flatNormal2D;
    if (role === "lut") return this.specLut2D;
    if (role === "mask") return this.defaultMask2D;
    return this.white2D;
  }

  public getFallbackCube(slot?: TextureSlotDesc): THREE.CubeTexture {
    if (slot) {
      const u = slot.textureId >>> 0;
      if (
        u === 0x80000008 ||
        u === 0x8000000a ||
        u === 0x215c9d15 ||
        u === 0x1d418ce6 ||
        u === 0x5e5574b1 ||
        slot.name === "env_amb" ||
        slot.name.toLowerCase() === "g_irradiancemap"
      ) {
        return this.studioCubeSoft;
      }
    }
    return this.studioCubeHard;
  }

  public async resolveSlotTexture(
    nodeId: string,
    game: "b1" | "b2" | "b3",
    slot: TextureSlotDesc,
    expectCube: boolean,
    bankTextures: TextureRefDesc[]
  ): Promise<THREE.Texture | THREE.CubeTexture> {
    if (this.disposed) {
      return expectCube
        ? this.getFallbackCube(slot)
        : this.getFallback2D(slot.role, slot);
    }

    const u = slot.textureId >>> 0;
    const isLut =
      slot.role === "lut" ||
      slot.name === "Spec_Pow" ||
      isB1GlobalSpecLutId(u);
    const maxDim = expectCube
      ? VIEWPORT_MAX_CUBE_DIM
      : isLut
      ? VIEWPORT_MAX_LUT_DIM
      : slot.role === "albedo"
      ? VIEWPORT_MAX_ALBEDO_DIM
      : VIEWPORT_MAX_SECONDARY_DIM;

    // Check if global pre_texture
    if (slot.global || (game === "b1" && (u & 0x80000000) !== 0)) {
      const cacheKey = `pre:${game}:${u}:${maxDim}`;
      if (expectCube) {
        const hit = this.getCubeLru(cacheKey);
        if (hit) return hit;
      } else {
        const hit = this.get2DLru(cacheKey);
        if (hit) return hit;
      }
      if (this.inFlight.has(cacheKey)) {
        return this.inFlight.get(cacheKey)!;
      }

      const promise = (async () => {
        try {
          const decoded = await getPreTexture(game, u, maxDim);
          if (this.disposed) {
            return expectCube
              ? this.getFallbackCube(slot)
              : this.getFallback2D(slot.role, slot);
          }
          const built = buildThreeTextureFromWtx1(decoded, isLut);
          if (built instanceof THREE.CubeTexture) {
            this.setCubeLru(cacheKey, built);
          } else {
            this.set2DLru(cacheKey, built);
          }
          return built;
        } catch {
          // Graceful fallback when core.dat is not configured or slot is missing
          return expectCube || isB1GlobalCubeId(u)
            ? this.getFallbackCube(slot)
            : this.getFallback2D(slot.role, slot);
        } finally {
          this.inFlight.delete(cacheKey);
        }
      })();

      this.inFlight.set(cacheKey, promise);
      return promise;
    }

    // Local WMB texture: check if present in bankTextures
    if (!slot.present) {
      return expectCube
        ? this.getFallbackCube(slot)
        : this.getFallback2D(slot.role, slot);
    }

    const bankEntry =
      bankTextures.find((bt) => (bt.textureId >>> 0) === u) ??
      bankTextures[u];
    const hashKey = bankEntry?.contentHash ?? `id:${u}`;
    const cacheKey = `wmb:${nodeId}:${u}:${hashKey}:${maxDim}`;

    if (expectCube) {
      const hit = this.getCubeLru(cacheKey);
      if (hit) return hit;
    } else {
      const hit = this.get2DLru(cacheKey);
      if (hit) return hit;
    }
    if (this.inFlight.has(cacheKey)) {
      return this.inFlight.get(cacheKey)!;
    }

    const promise = (async () => {
      try {
        const decoded = await getWmbTexture(nodeId, u, maxDim);
        if (this.disposed) {
          return expectCube
            ? this.getFallbackCube(slot)
            : this.getFallback2D(slot.role, slot);
        }
        const built = buildThreeTextureFromWtx1(decoded, isLut);
        if (built instanceof THREE.CubeTexture) {
          this.setCubeLru(cacheKey, built);
        } else {
          this.set2DLru(cacheKey, built);
        }
        return built;
      } catch {
        return expectCube
          ? this.getFallbackCube(slot)
          : this.getFallback2D(slot.role, slot);
      } finally {
        this.inFlight.delete(cacheKey);
      }
    })();

    this.inFlight.set(cacheKey, promise);
    return promise;
  }

  public dispose(): void {
    this.disposed = true;
    this.inFlight.clear();
    this.tex2DCache.forEach((t) => t.dispose());
    this.tex2DCache.clear();
    this.cubeCache.forEach((c) => c.dispose());
    this.cubeCache.clear();
    this.white2D.dispose();
    this.black2D.dispose();
    this.flatNormal2D.dispose();
    this.greyRelief2D.dispose();
    this.defaultMask2D.dispose();
    this.specLut2D.dispose();
    this.studioCubeHard.dispose();
    this.studioCubeSoft.dispose();
    this.missingChecker2D.dispose();
  }
}
