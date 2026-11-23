import * as THREE from "three";
import {
  BatchDesc,
  MaterialDesc,
  MaterialShaderStatus,
  TextureSlotDesc,
  WmbSceneManifest,
} from "../contract";
export type { MaterialShaderStatus };
import {
  createInitialUniforms,
  FALLBACK_FRAGMENT_SHADER,
  getShader,
  isB1GlobalCubeId,
  resolveB1SamplerSlots,
  resolveB2SamplerSlots,
  resolveB3BlendAndSort,
  resolveB3SamplerSlots,
  ResolvedShaderSpec,
  updateMaterialUniformsInPlace,
} from "../shaders/registry";
import { STUDIO_VERTEX_SHADER } from "../shaders/common";
import { DebugChannelMode, DebugSlotPreview, isNullShaderName, WmbGeometryCache } from "../store";
import { ViewerTextureCache } from "./textures";

const verifiedShaderPrograms = new WeakMap<
  WebGLRenderingContext | WebGL2RenderingContext,
  Map<string, { ok: boolean; errorLog?: string }>
>();

function hasNonZeroCoords(arr: Float32Array | null | undefined, minLen: number): arr is Float32Array {
  if (!arr || arr.length < minLen) return false;
  for (let i = 0; i < minLen; i++) {
    if (Math.abs(arr[i]) > 1e-6) return true;
  }
  return false;
}

interface SharedVbAttributeSet {
  position: THREE.BufferAttribute;
  normal: THREE.BufferAttribute;
  tangent: THREE.BufferAttribute;
  uv: THREE.BufferAttribute;
  uv1: THREE.BufferAttribute;
  uv2: THREE.BufferAttribute;
  color: THREE.BufferAttribute;
}

export function computeBatchRenderOrder(
  queue: "opaque" | "alphatest" | "blend",
  localSortNo: number,
  fileOrder: number
): number {
  const bucketBase =
    queue === "opaque"
      ? 1_000_000
      : queue === "alphatest"
      ? 2_000_000
      : 3_000_000;
  const clampedSort = Math.max(-50, Math.min(50, Math.trunc(localSortNo || 0)));
  const clampedOrder = Math.max(0, Math.min(9999, Math.trunc(fileOrder || 0)));
  return bucketBase + clampedSort * 10_000 + clampedOrder;
}

/**
 * Verifies that a raw Three.js ShaderMaterial vertex + fragment source compiles and links
 * on the active WebGLRenderingContext / WebGL2RenderingContext before assigning it.
 * Note: Three.js prepends standard uniforms/attributes (`projectionMatrix`, `modelViewMatrix`,
 * `modelMatrix`, `viewMatrix`, `normalMatrix`, `cameraPosition`, `position`, `normal`, `uv`),
 * while custom attributes (`tangent`, `uv1`, `uv2`, `color`) are declared inside `STUDIO_VERTEX_SHADER`.
 */
function verifyShaderCompiles(
  gl: WebGLRenderingContext | WebGL2RenderingContext | null,
  spec: ResolvedShaderSpec
): { ok: boolean; errorLog?: string } {
  if (!gl || gl.isContextLost()) return { ok: true };
  let ctxMap = verifiedShaderPrograms.get(gl);
  if (!ctxMap) {
    ctxMap = new Map();
    verifiedShaderPrograms.set(gl, ctxMap);
  }
  const cacheKey = `${spec.vertexShader}\0${spec.fragmentShader}`;
  const cached = ctxMap.get(cacheKey);
  if (cached) return cached;

  const vsPreamble = `
precision highp float;
precision highp int;
uniform mat4 modelMatrix;
uniform mat4 modelViewMatrix;
uniform mat4 projectionMatrix;
uniform mat4 viewMatrix;
uniform mat3 normalMatrix;
uniform vec3 cameraPosition;
attribute vec3 position;
attribute vec3 normal;
attribute vec2 uv;
`;
  const fsPreamble = `
precision highp float;
precision highp int;
uniform mat4 viewMatrix;
uniform vec3 cameraPosition;
`;
  const vsSource = vsPreamble + spec.vertexShader;
  const fsSource = fsPreamble + spec.fragmentShader;

  const vs = gl.createShader(gl.VERTEX_SHADER);
  const fs = gl.createShader(gl.FRAGMENT_SHADER);
  if (!vs || !fs) {
    return { ok: true };
  }

  gl.shaderSource(vs, vsSource);
  gl.compileShader(vs);
  if (!gl.getShaderParameter(vs, gl.COMPILE_STATUS)) {
    const log = gl.getShaderInfoLog(vs) ?? "Unknown vertex shader compile error";
    console.error(`[wmb-studio] Vertex shader compile failed for ${spec.id}:`, log);
    gl.deleteShader(vs);
    gl.deleteShader(fs);
    if (gl.isContextLost()) return { ok: true };
    const res = { ok: false, errorLog: log };
    ctxMap.set(cacheKey, res);
    return res;
  }

  gl.shaderSource(fs, fsSource);
  gl.compileShader(fs);
  if (!gl.getShaderParameter(fs, gl.COMPILE_STATUS)) {
    const log = gl.getShaderInfoLog(fs) ?? "Unknown fragment shader compile error";
    console.error(`[wmb-studio] Fragment shader compile failed for ${spec.id}:`, log);
    gl.deleteShader(vs);
    gl.deleteShader(fs);
    if (gl.isContextLost()) return { ok: true };
    const res = { ok: false, errorLog: log };
    ctxMap.set(cacheKey, res);
    return res;
  }

  const prog = gl.createProgram();
  if (prog) {
    gl.attachShader(prog, vs);
    gl.attachShader(prog, fs);
    gl.linkProgram(prog);
    if (!gl.getProgramParameter(prog, gl.LINK_STATUS)) {
      const log = gl.getProgramInfoLog(prog) ?? "Unknown program link error";
      console.error(`[wmb-studio] Shader program link failed for ${spec.id}:`, log);
      gl.deleteProgram(prog);
      gl.deleteShader(vs);
      gl.deleteShader(fs);
      if (gl.isContextLost()) return { ok: true };
      const res = { ok: false, errorLog: log };
      ctxMap.set(cacheKey, res);
      return res;
    }
    gl.deleteProgram(prog);
  }

  gl.deleteShader(vs);
  gl.deleteShader(fs);
  const res = { ok: true };
  ctxMap.set(cacheKey, res);
  return res;
}

export class SceneMeshBuilder {
  public readonly rootGroup = new THREE.Group();
  public readonly batchMeshes = new Map<string, THREE.Mesh>();
  public readonly materialInstances = new Map<string, THREE.ShaderMaterial>();
  public readonly materialShaderStatus = new Map<string, MaterialShaderStatus>();

  private readonly batchGeometries = new Map<
    string,
    { vbHash: string; idxHash: string; geometry: THREE.BufferGeometry }
  >();
  private readonly sharedVbAttributes = new Map<string, SharedVbAttributeSet>();
  private readonly materialSlotTextures = new Map<
    string,
    Map<number, THREE.Texture | THREE.CubeTexture>
  >();
  private readonly materialSignature = new Map<string, string>();

  constructor(
    private readonly texCache: ViewerTextureCache,
    private readonly onNeedsRender: () => void
  ) {
    this.rootGroup.name = "wmb-batches-root";
  }

  public syncScene(
    manifest: WmbSceneManifest,
    geomCache: WmbGeometryCache,
    gl: WebGLRenderingContext | WebGL2RenderingContext | null,
    options: {
      wireframe: boolean;
      debugMode: DebugChannelMode;
      debugSlot: DebugSlotPreview | null;
      activeLodLevel: number | null;
      hiddenBatchIds: Set<string>;
      hiddenMeshGroupIds: Set<string>;
      secondaryNormalsEnabled?: boolean;
    }
  ): void {
    const activeMatIds = new Set<string>();
    const texHashById = new Map<number, string>();
    for (const t of manifest.textures) {
      texHashById.set(t.textureId >>> 0, t.contentHash);
    }
    for (const matDesc of manifest.materials) {
      activeMatIds.add(matDesc.id);
      const sig = `${matDesc.game}:${matDesc.shaderName ?? ""}:${
        matDesc.materialType ?? ""
      }:${matDesc.textureSlots
        .map(
          (s) =>
            `${s.slot}=${s.textureIdHex}:${s.present ? 1 : 0}:${
              texHashById.get(s.textureId >>> 0) ?? ""
            }`
        )
        .join(",")}`;
      const existingMat = this.materialInstances.get(matDesc.id);
      const prevSig = this.materialSignature.get(matDesc.id);

      if (!existingMat || prevSig !== sig) {
        existingMat?.dispose();
        const created = this.createShaderMaterial(manifest, matDesc, gl);
        this.materialInstances.set(matDesc.id, created);
        this.materialSignature.set(matDesc.id, sig);
      } else {
        updateMaterialUniformsInPlace(existingMat, matDesc);
      }
    }

    for (const [matId, matInst] of Array.from(this.materialInstances.entries())) {
      if (!activeMatIds.has(matId)) {
        matInst.dispose();
        this.materialInstances.delete(matId);
        this.materialSignature.delete(matId);
        this.materialShaderStatus.delete(matId);
        this.materialSlotTextures.delete(matId);
      }
    }

    const activeBatchIds = new Set<string>();
    const activeVbHashes = new Set<string>();
    const vbByIndex = new Map<number, string>();
    for (const vb of manifest.vertexBuffers) {
      vbByIndex.set(vb.index, vb.geometryHash);
      activeVbHashes.add(vb.geometryHash);
    }
    for (const key of Array.from(this.sharedVbAttributes.keys())) {
      if (!activeVbHashes.has(key)) {
        this.sharedVbAttributes.delete(key);
      }
    }

    const lodLevelById = new Map<string, number>();
    for (const mg of manifest.meshGroups) {
      for (const l of mg.lods) {
        lodLevelById.set(l.id, l.level);
      }
    }

    for (const batch of manifest.batches) {
      activeBatchIds.add(batch.id);
      const vbHash = vbByIndex.get(batch.vertexBufferIndex) ?? "";
      const idxHash = batch.geometryHash;

      let geomRecord = this.batchGeometries.get(batch.id);
      if (
        !geomRecord ||
        geomRecord.vbHash !== vbHash ||
        geomRecord.idxHash !== idxHash
      ) {
        geomRecord?.geometry.dispose();
        const builtGeom = this.buildBatchGeometry(batch, vbHash, geomCache);
        geomRecord = { vbHash, idxHash, geometry: builtGeom };
        this.batchGeometries.set(batch.id, geomRecord);
      }

      const shaderMat =
        this.materialInstances.get(batch.materialId) ??
        this.getOrCreateDefaultMaterial(manifest, gl);

      let mesh = this.batchMeshes.get(batch.id);
      if (!mesh) {
        mesh = new THREE.Mesh(geomRecord.geometry, shaderMat);
        mesh.frustumCulled = false;
        this.rootGroup.add(mesh);
        this.batchMeshes.set(batch.id, mesh);
      } else {
        if (mesh.geometry !== geomRecord.geometry) {
          mesh.geometry = geomRecord.geometry;
        }
        if (mesh.material !== shaderMat) {
          mesh.material = shaderMat;
        }
      }

      mesh.name = batch.id;
      mesh.userData = {
        batchId: batch.id,
        materialId: batch.materialId,
        meshId: batch.meshId,
        lodId: batch.lodId,
        fileOrder: batch.fileOrder,
      };

      // Assign renderOrder across opaque -> alphatest -> blend buckets (+ B3 g_LocalSortNo offset)
      const matDesc = manifest.materials.find((m) => m.id === batch.materialId);
      const spec = getShader(
        manifest.game,
        matDesc?.shaderName ?? null,
        matDesc?.materialType ?? null
      );
      const blendSort = resolveB3BlendAndSort(matDesc, spec);
      mesh.renderOrder = computeBatchRenderOrder(
        blendSort.queue,
        blendSort.localSortNo,
        batch.fileOrder
      );

      // Visibility filtering: shadow proxies, NULL00 shader meshes, LOD level filter, and manual visibility toggles
      let visible =
        !batch.isShadowProxy &&
        !isNullShaderName(matDesc?.shaderName) &&
        batch.indexCount > 0 &&
        batch.vertexCount > 0;
      if (
        visible &&
        options.activeLodLevel !== null &&
        batch.lodId &&
        lodLevelById.has(batch.lodId)
      ) {
        visible = lodLevelById.get(batch.lodId) === options.activeLodLevel;
      }
      if (
        visible &&
        (options.hiddenBatchIds.has(batch.id) ||
          options.hiddenMeshGroupIds.has(batch.meshId))
      ) {
        visible = false;
      }
      mesh.visible = visible;
    }

    for (const [batchId, mesh] of Array.from(this.batchMeshes.entries())) {
      if (!activeBatchIds.has(batchId)) {
        this.rootGroup.remove(mesh);
        this.batchMeshes.delete(batchId);
        const g = this.batchGeometries.get(batchId);
        g?.geometry.dispose();
        this.batchGeometries.delete(batchId);
      }
    }

    this.applyDisplayOptions(
      manifest,
      options.wireframe,
      options.debugMode,
      options.debugSlot,
      options.secondaryNormalsEnabled ?? false
    );
  }

  public applyDisplayOptions(
    manifest: WmbSceneManifest | null,
    wireframe: boolean,
    debugMode: DebugChannelMode,
    debugSlot: DebugSlotPreview | null,
    secondaryNormalsEnabled = false
  ): void {
    const channelIdx =
      debugSlot?.channel === "R"
        ? 1
        : debugSlot?.channel === "G"
        ? 2
        : debugSlot?.channel === "B"
        ? 3
        : debugSlot?.channel === "A"
        ? 4
        : 0;
    const modeInt =
      debugMode === "normals" ? 1 : debugMode === "slot" && debugSlot ? 2 : 0;

    this.materialInstances.forEach((mat, matId) => {
      mat.wireframe = wireframe;
      if (mat.uniforms.uUseSecondaryNormals) {
        mat.uniforms.uUseSecondaryNormals.value = secondaryNormalsEnabled ? 1 : 0;
      }
      if (mat.uniforms.uDebugMode) {
        mat.uniforms.uDebugMode.value = modeInt;
      }
      if (mat.uniforms.uDebugChannel) {
        mat.uniforms.uDebugChannel.value = channelIdx;
      }
      if (modeInt === 2 && debugSlot && manifest) {
        const slotMap = this.materialSlotTextures.get(matId);
        const tex = slotMap?.get(debugSlot.slotIndex);
        if (tex instanceof THREE.CubeTexture) {
          mat.uniforms.uDebugIsCube.value = 1;
          mat.uniforms.uDebugTexCube.value = tex;
        } else if (tex instanceof THREE.Texture) {
          mat.uniforms.uDebugIsCube.value = 0;
          mat.uniforms.uDebugTex2D.value = tex;
        } else {
          mat.uniforms.uDebugIsCube.value = 0;
          mat.uniforms.uDebugTex2D.value = this.texCache.white2D;
        }

        // Determine the active UV slot and tiling for this texture slot in Tex Debug mode
        let debugUvMode = 0;
        let tileX = 1.0;
        let tileY = 1.0;
        const matDesc = manifest.materials.find((m) => m.id === matId);
        if (matDesc && matDesc.game === "b3") {
          const b3Slots = resolveB3SamplerSlots(matDesc);
          const hairVar = mat.uniforms.uB3HairVariant?.value ?? 0;
          const pbsVar = mat.uniforms.uB3PbsVariant?.value ?? -1;
          const matSlot = matDesc.textureSlots.find(
            (s) => s.slot === debugSlot.slotIndex
          );
          const slotLow = (matSlot?.name ?? "").toLowerCase();

          if (hairVar === 20) {
            const onlyL1 = mat.uniforms.g_OnlyLayer1?.value ?? 1.0;
            const eyeBlow = mat.uniforms.g_EyeBlowMode?.value ?? 0.0;
            const alphaEff = mat.uniforms.g_UseAlphaEffect?.value ?? 0.0;
            const useLmAlbedo = onlyL1 < 0.5 && eyeBlow < 0.5 && alphaEff < 0.5;
            if (
              (b3Slots.albedo && b3Slots.albedo.slot === debugSlot.slotIndex) ||
              slotLow === "g_albedomap1"
            ) {
              if (useLmAlbedo) debugUvMode = 1;
            } else if (
              (b3Slots.normal && b3Slots.normal.slot === debugSlot.slotIndex) ||
              slotLow === "g_normalmap"
            ) {
              // Hair20 g_NormalMap uses UV0 for RG (strand normal) and Lightmap UV (UV1/UV2) for B (AO) & A
              debugUvMode = 2;
            }
          } else if (hairVar === 21) {
            if (b3Slots.normal && b3Slots.normal.slot === debugSlot.slotIndex) {
              tileX = mat.uniforms.g_TilingMap1UV?.value?.x ?? 1.0;
              tileY = mat.uniforms.g_TilingMap1UV?.value?.y ?? 1.0;
            } else if (b3Slots.normal2 && b3Slots.normal2.slot === debugSlot.slotIndex) {
              tileX = mat.uniforms.g_TilingMap2UV?.value?.x ?? 1.0;
              tileY = mat.uniforms.g_TilingMap2UV?.value?.y ?? 1.0;
            }
          } else if (hairVar === 22) {
            if (b3Slots.normal && b3Slots.normal.slot === debugSlot.slotIndex) {
              tileX = mat.uniforms.g_TilingMap1UV?.value?.x ?? 1.0;
              tileY = mat.uniforms.g_TilingMap1UV?.value?.y ?? 1.0;
            } else if (b3Slots.normal2 && b3Slots.normal2.slot === debugSlot.slotIndex) {
              debugUvMode = 1;
              tileX = mat.uniforms.g_TilingMap2UV?.value?.x ?? 1.0;
              tileY = mat.uniforms.g_TilingMap2UV?.value?.y ?? 1.0;
            }
          } else if (pbsVar === 4 && b3Slots.normal2 && b3Slots.normal2.slot === debugSlot.slotIndex) {
            debugUvMode = 1;
            tileX = mat.uniforms.g_DetailNormalTile?.value?.x ?? 1.0;
            tileY = mat.uniforms.g_DetailNormalTile?.value?.y ?? 1.0;
          }
        }
        if (mat.uniforms.uDebugUvMode) {
          mat.uniforms.uDebugUvMode.value = debugUvMode;
        }
        if (mat.uniforms.uDebugUvTile) {
          mat.uniforms.uDebugUvTile.value.set(tileX, tileY);
        }
      }
    });
  }

  public updateMaterialInPlace(matDesc: MaterialDesc): void {
    const existing = this.materialInstances.get(matDesc.id);
    if (existing) {
      updateMaterialUniformsInPlace(existing, matDesc);
      const spec = getShader(matDesc.game, matDesc.shaderName, matDesc.materialType);
      const blendSort = resolveB3BlendAndSort(matDesc, spec);
      if (
        existing.blending !== blendSort.blending ||
        existing.transparent !== blendSort.transparent ||
        existing.depthWrite !== blendSort.depthWrite
      ) {
        existing.blending = blendSort.blending;
        existing.transparent = blendSort.transparent;
        existing.depthWrite = blendSort.depthWrite;
        existing.needsUpdate = true;
      }
      this.batchMeshes.forEach((mesh) => {
        if (mesh.userData?.materialId === matDesc.id) {
          const fileOrder =
            typeof mesh.userData.fileOrder === "number" ? mesh.userData.fileOrder : 0;
          mesh.renderOrder = computeBatchRenderOrder(
            blendSort.queue,
            blendSort.localSortNo,
            fileOrder
          );
        }
      });
      this.onNeedsRender();
    }
  }

  private createShaderMaterial(
    manifest: WmbSceneManifest,
    matDesc: MaterialDesc,
    gl: WebGLRenderingContext | WebGL2RenderingContext | null
  ): THREE.ShaderMaterial {
    let spec = getShader(
      matDesc.game,
      matDesc.shaderName,
      matDesc.materialType
    );
    let hasCompileError = false;
    let compileErrorLog: string | undefined;

    if (!spec.isFallback) {
      const check = verifyShaderCompiles(gl, spec);
      if (!check.ok) {
        hasCompileError = true;
        compileErrorLog = check.errorLog;
        spec = {
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
    }

    this.materialShaderStatus.set(matDesc.id, {
      materialId: matDesc.id,
      shaderId: spec.id,
      isFallback: spec.isFallback,
      hasCompileError,
      compileErrorLog,
    });

    const uniforms = createInitialUniforms(
      matDesc,
      spec,
      this.texCache.white2D,
      this.texCache.flatNormal2D,
      this.texCache.specLut2D,
      this.texCache.studioCubeHard,
      this.texCache.studioCubeSoft
    );

    const blendSort = resolveB3BlendAndSort(matDesc, spec);
    const shaderMat = new THREE.ShaderMaterial({
      name: matDesc.id,
      vertexShader: spec.vertexShader,
      fragmentShader: spec.fragmentShader,
      uniforms,
      transparent: blendSort.transparent,
      depthWrite: blendSort.depthWrite,
      side: THREE.BackSide,
      blending: blendSort.blending,
    });

    const slotTexMap = new Map<number, THREE.Texture | THREE.CubeTexture>();
    this.materialSlotTextures.set(matDesc.id, slotTexMap);

      this.bindMaterialTextures(manifest, matDesc, shaderMat, slotTexMap);

    return shaderMat;
  }

  private bindMaterialTextures(
    manifest: WmbSceneManifest,
    matDesc: MaterialDesc,
    shaderMat: THREE.ShaderMaterial,
    slotTexMap: Map<number, THREE.Texture | THREE.CubeTexture>
  ): void {
    const loadAndAssign = (
      slot: TextureSlotDesc | undefined,
      uniformName: string,
      expectCube: boolean,
      presenceUniformName?: string
    ) => {
      if (!slot) return;
      // Immediate fallback before async decode completes
      const immediate = expectCube
        ? this.texCache.getFallbackCube(slot)
        : this.texCache.getFallback2D(slot.role, slot);
      if (shaderMat.uniforms[uniformName]) {
        shaderMat.uniforms[uniformName].value = immediate;
      }
      if (presenceUniformName && shaderMat.uniforms[presenceUniformName]) {
        shaderMat.uniforms[presenceUniformName].value = slot.present ? 1 : 0;
      }
      slotTexMap.set(slot.slot, immediate);

      void this.texCache
        .resolveSlotTexture(
          manifest.nodeId,
          manifest.game,
          slot,
          expectCube,
          manifest.textures
        )
        .then((resolved) => {
          slotTexMap.set(slot.slot, resolved);
          if (shaderMat.uniforms[uniformName]) {
            if (expectCube && resolved instanceof THREE.CubeTexture) {
              shaderMat.uniforms[uniformName].value = resolved;
            } else if (!expectCube && !(resolved instanceof THREE.CubeTexture)) {
              shaderMat.uniforms[uniformName].value = resolved;
            }
          }
          if (presenceUniformName && shaderMat.uniforms[presenceUniformName]) {
            shaderMat.uniforms[presenceUniformName].value = slot.present ? 1 : 0;
          }
          this.onNeedsRender();
        });
    };

    if (matDesc.game === "b1") {
      const b1Slots = resolveB1SamplerSlots(matDesc);
      loadAndAssign(b1Slots.Color_1, "Color_1_sampler", false);
      loadAndAssign(b1Slots.reliefmap, "reliefmap_sampler", false);
      loadAndAssign(b1Slots.envmap, "envmap_sampler", true);
      loadAndAssign(
        b1Slots.env_amb ?? b1Slots.envmap,
        "env_amb_sampler",
        true
      );
      loadAndAssign(b1Slots.Spec_Pow, "Spec_Pow_sampler", false);
      loadAndAssign(b1Slots.Spec_Mask, "Spec_Mask_sampler", false);
      loadAndAssign(b1Slots.lightmap, "lightmap_sampler", false);
    } else if (matDesc.game === "b2") {
      const b2Slots = resolveB2SamplerSlots(matDesc);
      loadAndAssign(
        b2Slots.albedo,
        "uB2AlbedoMap",
        false,
        "uB2HasAlbedoMap"
      );
      loadAndAssign(
        b2Slots.normal,
        "uB2NormalMap",
        false,
        "uB2HasNormalMap"
      );
      loadAndAssign(b2Slots.rss, "uB2RssMap", false, "uB2HasRssMap");
      loadAndAssign(b2Slots.cubemap, "uB2CubeMap", true, "uB2HasCubeMap");
      loadAndAssign(b2Slots.lut, "uB2MultipleLut", false);

      // Also populate fallback sampler names in case fallback shader is active
      loadAndAssign(b2Slots.albedo, "Color_1_sampler", false);
      if (b2Slots.normal) {
        loadAndAssign(b2Slots.normal, "reliefmap_sampler", false);
      }
    } else {
      // Bayonetta 3 (`PBS00`, `BAYO00`, `Hair20/21/22`, `SKN20`, `Gls*`, `HZR00`, `JWL*`, `CNS*`)
      const b3Slots = resolveB3SamplerSlots(matDesc);
      loadAndAssign(b3Slots.albedo, "uTexAlbedo", false, "uHasTexAlbedo");
      loadAndAssign(b3Slots.normal, "uTexNormal", false, "uHasTexNormal");
      loadAndAssign(b3Slots.mask, "uTexMask", false, "uHasTexMask");
      loadAndAssign(b3Slots.normal2, "uTexNormal2", false, "uHasTexNormal2");
      loadAndAssign(b3Slots.normal3, "uTexNormal3", false, "uHasTexNormal3");
      loadAndAssign(b3Slots.indexMap, "uTexIndex", false, "uHasTexIndex");
      loadAndAssign(b3Slots.envCube, "uTexEnv", true);
      loadAndAssign(b3Slots.irradCube, "uTexIrradiance", true);

      // Also populate fallback sampler names for unimplemented B3 shaders (e.g. CUSTOM_UNIMPLEMENTED_99)
      loadAndAssign(b3Slots.albedo, "Color_1_sampler", false);
      if (b3Slots.normal) {
        loadAndAssign(b3Slots.normal, "reliefmap_sampler", false);
      }
    }

    // Also populate slotTexMap for all remaining slots so texture debug preview works on every slot
    for (const slot of matDesc.textureSlots) {
      if (!slotTexMap.has(slot.slot)) {
        const expectCube =
          slot.role === "env" || isB1GlobalCubeId(slot.textureId);
        void this.texCache
          .resolveSlotTexture(
            manifest.nodeId,
            manifest.game,
            slot,
            expectCube,
            manifest.textures
          )
          .then((resolved) => {
            slotTexMap.set(slot.slot, resolved);
          });
      }
    }
  }

  private getOrCreateDefaultMaterial(
    manifest: WmbSceneManifest,
    gl: WebGLRenderingContext | WebGL2RenderingContext | null
  ): THREE.ShaderMaterial {
    const existing = this.materialInstances.get("__default__");
    if (existing) return existing;
    const dummyMat: MaterialDesc = {
      id: "__default__",
      index: 0,
      name: "Default",
      game: manifest.game,
      shaderName: null,
      materialType: null,
      technique: null,
      textureSlots: [],
      params: [],
      usedByBatchIds: [],
      thumbnailDataUrl: null,
    };
    const created = this.createShaderMaterial(manifest, dummyMat, gl);
    this.materialInstances.set("__default__", created);
    return created;
  }

  private buildBatchGeometry(
    batch: BatchDesc,
    vbHash: string,
    geomCache: WmbGeometryCache
  ): THREE.BufferGeometry {
    const geom = new THREE.BufferGeometry();
    const vb = geomCache.vertexBuffers.get(vbHash);
    const indices = geomCache.batchIndices.get(batch.geometryHash);

    // Malformed / empty batch guard: return a clean empty BufferGeometry with a zero-length position attribute
    if (
      !vb ||
      vb.vertexCount === 0 ||
      batch.indexCount === 0 ||
      batch.vertexCount === 0 ||
      !indices ||
      indices.length === 0
    ) {
      geom.setAttribute(
        "position",
        new THREE.BufferAttribute(new Float32Array(0), 3)
      );
      geom.boundingBox = new THREE.Box3(
        new THREE.Vector3(0, 0, 0),
        new THREE.Vector3(0, 0, 0)
      );
      return geom;
    }

    let sharedAttrs = this.sharedVbAttributes.get(vbHash);
    if (!sharedAttrs) {
      const vCount = vb.vertexCount;
      const position = new THREE.BufferAttribute(vb.position, 3);

      let normal: THREE.BufferAttribute;
      if (vb.normal && vb.normal.length >= vCount * 3) {
        normal = new THREE.BufferAttribute(vb.normal, 3);
      } else {
        const fallbackNrm = new Float32Array(vCount * 3);
        for (let i = 0; i < vCount; i++) fallbackNrm[i * 3 + 1] = 1.0;
        normal = new THREE.BufferAttribute(fallbackNrm, 3);
      }

      let tangent: THREE.BufferAttribute;
      if (vb.tangent && vb.tangent.length >= vCount * 4) {
        tangent = new THREE.BufferAttribute(vb.tangent, 4);
      } else {
        const fallbackTan = new Float32Array(vCount * 4);
        for (let i = 0; i < vCount; i++) {
          fallbackTan[i * 4 + 0] = 1.0;
          fallbackTan[i * 4 + 3] = 1.0;
        }
        tangent = new THREE.BufferAttribute(fallbackTan, 4);
      }

      const uv0Array =
        vb.uv0 && vb.uv0.length >= vCount * 2
          ? vb.uv0
          : new Float32Array(vCount * 2);
      const uv = new THREE.BufferAttribute(uv0Array, 2);

      const uv1 = hasNonZeroCoords(vb.uv1, vCount * 2)
        ? new THREE.BufferAttribute(vb.uv1, 2)
        : uv;

      const uv2 = hasNonZeroCoords(vb.uv2, vCount * 2)
        ? new THREE.BufferAttribute(vb.uv2, 2)
        : uv;

      let color: THREE.BufferAttribute;
      if (vb.color && vb.color.length >= vCount * 4) {
        color = new THREE.BufferAttribute(vb.color, 4, true);
      } else {
        const fallbackCol = new Uint8Array(vCount * 4);
        fallbackCol.fill(255);
        color = new THREE.BufferAttribute(fallbackCol, 4, true);
      }

      sharedAttrs = { position, normal, tangent, uv, uv1, uv2, color };
      this.sharedVbAttributes.set(vbHash, sharedAttrs);
    }

    geom.setAttribute("position", sharedAttrs.position);
    geom.setAttribute("normal", sharedAttrs.normal);
    geom.setAttribute("tangent", sharedAttrs.tangent);
    geom.setAttribute("uv", sharedAttrs.uv);
    geom.setAttribute("uv1", sharedAttrs.uv1);
    geom.setAttribute("uv2", sharedAttrs.uv2);
    geom.setAttribute("color", sharedAttrs.color);

    if (indices && indices.length > 0) {
      geom.setIndex(new THREE.BufferAttribute(indices, 1));
    }

    if (batch.bbox.length === 6) {
      geom.boundingBox = new THREE.Box3(
        new THREE.Vector3(batch.bbox[0], batch.bbox[1], batch.bbox[2]),
        new THREE.Vector3(batch.bbox[3], batch.bbox[4], batch.bbox[5])
      );
    } else {
      geom.computeBoundingBox();
    }

    return geom;
  }

  public dispose(): void {
    this.batchGeometries.forEach((g) => g.geometry.dispose());
    this.batchGeometries.clear();
    this.sharedVbAttributes.clear();
    this.batchMeshes.clear();
    this.materialInstances.forEach((m) => m.dispose());
    this.materialInstances.clear();
    this.materialSignature.clear();
    this.materialShaderStatus.clear();
    this.materialSlotTextures.clear();
    this.rootGroup.clear();
  }
}
