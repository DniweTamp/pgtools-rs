import {
  BatchDesc,
  BoneDesc,
  LodDesc,
  MaterialDesc,
  MeshGroupDesc,
  Region,
  TextureRefDesc,
  TextureSlotRole,
  WmbSceneManifest,
  WmbScenePayload,
} from "./contract";

export type SelectionKind =
  | "root"
  | "texture"
  | "material"
  | "bone"
  | "meshGroup"
  | "lod"
  | "batch";

export interface SceneSelectionItem {
  kind: SelectionKind;
  id: string;
}

export type DebugChannelMode = "off" | "normals" | "slot";

export interface DebugSlotPreview {
  slotIndex: number;
  slotName: string;
  role: TextureSlotRole;
  channel: "RGB" | "R" | "G" | "B" | "A";
}

export interface CachedVertexBufferData {
  geometryHash: string;
  vertexCount: number;
  position: Float32Array;
  normal: Float32Array | null;
  tangent: Float32Array | null;
  uv0: Float32Array | null;
  uv1: Float32Array | null;
  uv2: Float32Array | null;
  color: Uint8Array | null;
}

export interface WmbGeometryCache {
  vertexBuffers: Map<string, CachedVertexBufferData>;
  batchIndices: Map<string, Uint32Array>;
}

export function createGeometryCache(): WmbGeometryCache {
  return {
    vertexBuffers: new Map<string, CachedVertexBufferData>(),
    batchIndices: new Map<string, Uint32Array>(),
  };
}

export function getKnownGeometryHashes(cache: WmbGeometryCache): string[] {
  const out = new Set<string>();
  cache.vertexBuffers.forEach((_v, k) => out.add(k));
  cache.batchIndices.forEach((_v, k) => out.add(k));
  return Array.from(out);
}

function readTypedRegion<T extends Float32Array | Uint8Array | Uint32Array>(
  binary: ArrayBuffer,
  region: Region | null | undefined,
  ctor: { new (buffer: ArrayBuffer): T }
): T | null {
  if (!region || region.omitted || region.byteLength <= 0) return null;
  if (region.offset + region.byteLength > binary.byteLength) return null;
  return new ctor(binary.slice(region.offset, region.offset + region.byteLength));
}

export function mergeScenePayloadIntoCache(
  payload: WmbScenePayload,
  cache: WmbGeometryCache
): {
  changedVbHashes: Set<string>;
  changedBatchHashes: Set<string>;
} {
  const { manifest, binary } = payload;
  const changedVbHashes = new Set<string>();
  const changedBatchHashes = new Set<string>();
  const activeVbHashes = new Set<string>();
  const activeBatchHashes = new Set<string>();

  for (const vb of manifest.vertexBuffers) {
    activeVbHashes.add(vb.geometryHash);
    if (!vb.position.omitted || !cache.vertexBuffers.has(vb.geometryHash)) {
      const pos =
        readTypedRegion(binary, vb.position, Float32Array) ??
        new Float32Array(vb.vertexCount * 3);
      const nrm = readTypedRegion(binary, vb.normal, Float32Array);
      const tan = readTypedRegion(binary, vb.tangent, Float32Array);
      const uv0 = readTypedRegion(binary, vb.uv0, Float32Array);
      const uv1 = readTypedRegion(binary, vb.uv1, Float32Array);
      const uv2 = readTypedRegion(binary, vb.uv2 ?? null, Float32Array);
      const col = readTypedRegion(binary, vb.color, Uint8Array);
      cache.vertexBuffers.set(vb.geometryHash, {
        geometryHash: vb.geometryHash,
        vertexCount: vb.vertexCount,
        position: pos,
        normal: nrm,
        tangent: tan,
        uv0,
        uv1,
        uv2,
        color: col,
      });
      changedVbHashes.add(vb.geometryHash);
    }
  }

  for (const batch of manifest.batches) {
    activeBatchHashes.add(batch.geometryHash);
    if (
      !batch.indices.omitted ||
      !cache.batchIndices.has(batch.geometryHash)
    ) {
      const idx =
        readTypedRegion(binary, batch.indices, Uint32Array) ??
        new Uint32Array(0);
      cache.batchIndices.set(batch.geometryHash, idx);
      changedBatchHashes.add(batch.geometryHash);
    }
  }

  for (const key of Array.from(cache.vertexBuffers.keys())) {
    if (!activeVbHashes.has(key)) {
      cache.vertexBuffers.delete(key);
    }
  }
  for (const key of Array.from(cache.batchIndices.keys())) {
    if (!activeBatchHashes.has(key)) {
      cache.batchIndices.delete(key);
    }
  }

  return { changedVbHashes, changedBatchHashes };
}

export function getBoneSceneId(bone: BoneDesc, index: number): string {
  return `bone:${index}:${bone.globalId}`;
}

export function parseBoneSceneId(id: string): {
  index: number | null;
  globalId: number | null;
} {
  const raw = id.replace(/^bone:/, "").trim();
  if (raw.includes(":")) {
    const [idxStr, gidStr] = raw.split(":");
    const idx = parseInt(idxStr, 10);
    const gid = parseInt(gidStr, 10);
    return {
      index: Number.isNaN(idx) ? null : idx,
      globalId: Number.isNaN(gid) ? null : gid,
    };
  }
  const gid = parseInt(raw, 10);
  return {
    index: null,
    globalId: Number.isNaN(gid) ? null : gid,
  };
}

export function parseTextureSceneId(id: string): number | null {
  const raw = id.replace(/^tex:/, "").trim();
  if (!raw) return null;
  const parsed = raw.toLowerCase().startsWith("0x")
    ? parseInt(raw.slice(2), 16)
    : parseInt(raw, 10);
  return Number.isNaN(parsed) ? null : parsed >>> 0;
}

export function findMaterialById(
  manifest: WmbSceneManifest | null,
  id: string
): MaterialDesc | undefined {
  return manifest?.materials.find((m) => m.id === id);
}

export function findBatchById(
  manifest: WmbSceneManifest | null,
  id: string
): BatchDesc | undefined {
  return manifest?.batches.find((b) => b.id === id);
}

export function findMeshGroupById(
  manifest: WmbSceneManifest | null,
  id: string
): MeshGroupDesc | undefined {
  return manifest?.meshGroups.find((mg) => mg.id === id);
}

export function findLodById(
  manifest: WmbSceneManifest | null,
  id: string
): { meshGroup: MeshGroupDesc; lod: LodDesc } | undefined {
  if (!manifest) return undefined;
  for (const mg of manifest.meshGroups) {
    const lod = mg.lods.find((l) => l.id === id);
    if (lod) return { meshGroup: mg, lod };
  }
  return undefined;
}

export function getMeshGroupBatchIds(mg: MeshGroupDesc): string[] {
  return mg.lods.length > 0 ? mg.lods.flatMap((l) => l.batchIds) : mg.batchIds;
}

export function findBoneIndexAndDescBySceneId(
  manifest: WmbSceneManifest | null,
  id: string
): { bone: BoneDesc; index: number } | undefined {
  if (!manifest) return undefined;
  const { index, globalId } = parseBoneSceneId(id);
  if (index !== null && index >= 0 && index < manifest.bones.length) {
    const candidate = manifest.bones[index];
    if (globalId === null || candidate.globalId === globalId) {
      return { bone: candidate, index };
    }
  }
  if (globalId === null) return undefined;
  const foundIdx = manifest.bones.findIndex((b) => b.globalId === globalId);
  if (foundIdx >= 0) {
    return { bone: manifest.bones[foundIdx], index: foundIdx };
  }
  if (globalId >= 0 && globalId < manifest.bones.length) {
    return { bone: manifest.bones[globalId], index: globalId };
  }
  return undefined;
}

export function findBoneBySceneId(
  manifest: WmbSceneManifest | null,
  id: string
): BoneDesc | undefined {
  return findBoneIndexAndDescBySceneId(manifest, id)?.bone;
}

export function findTextureBySceneId(
  manifest: WmbSceneManifest | null,
  id: string
): TextureRefDesc | undefined {
  if (!manifest) return undefined;
  const tid = parseTextureSceneId(id);
  if (tid === null) return undefined;
  return manifest.textures.find((t) => (t.textureId >>> 0) === tid);
}

function isItemPresentInManifest(
  manifest: WmbSceneManifest | null,
  item: SceneSelectionItem
): boolean {
  if (!manifest) return false;
  switch (item.kind) {
    case "root":
      return item.id === "root";
    case "texture":
      return findTextureBySceneId(manifest, item.id) !== undefined;
    case "material":
      return findMaterialById(manifest, item.id) !== undefined;
    case "bone":
      return findBoneBySceneId(manifest, item.id) !== undefined;
    case "meshGroup":
      return findMeshGroupById(manifest, item.id) !== undefined;
    case "lod":
      return findLodById(manifest, item.id) !== undefined;
    case "batch":
      return findBatchById(manifest, item.id) !== undefined;
    default:
      return false;
  }
}

export function filterValidSelection(
  manifest: WmbSceneManifest | null,
  selection: SceneSelectionItem[]
): SceneSelectionItem[] {
  if (!manifest) return [];
  const valid = selection.filter((s) => isItemPresentInManifest(manifest, s));
  return valid.length === 0 ? [{ kind: "root", id: "root" }] : valid;
}

export interface BoneHierarchyGraph {
  childrenByParentIdx: Map<number, Array<{ bone: BoneDesc; idx: number }>>;
  indexByGlobalId: Map<number, number>;
  parentIdxByBoneIdx: Map<number, number>;
  resolveBoneIdxFromId: (boneSceneId: string) => number;
  isBoneDescendantOf: (candidateIdx: number, ancestorIdx: number) => boolean;
}

export function buildBoneHierarchy(bones: BoneDesc[]): BoneHierarchyGraph {
  const childrenByParentIdx = new Map<number, Array<{ bone: BoneDesc; idx: number }>>();
  const indexByGlobalId = new Map<number, number>();
  const parentIdxByBoneIdx = new Map<number, number>();

  bones.forEach((b, idx) => {
    if (!indexByGlobalId.has(b.globalId)) {
      indexByGlobalId.set(b.globalId, idx);
    }
  });

  for (let i = 0; i < bones.length; i++) {
    const b = bones[i];
    let parentIdx = b.parent;
    if (parentIdx >= bones.length && indexByGlobalId.has(parentIdx)) {
      parentIdx = indexByGlobalId.get(parentIdx)!;
    }
    if (parentIdx < 0 || parentIdx >= bones.length || parentIdx === i) {
      parentIdx = -1;
    }
    parentIdxByBoneIdx.set(i, parentIdx);
  }

  for (let i = 0; i < bones.length; i++) {
    let cur = parentIdxByBoneIdx.get(i) ?? -1;
    const seen = new Set<number>([i]);
    while (cur >= 0) {
      if (seen.has(cur)) {
        parentIdxByBoneIdx.set(i, -1);
        break;
      }
      seen.add(cur);
      cur = parentIdxByBoneIdx.get(cur) ?? -1;
    }
    const finalParent = parentIdxByBoneIdx.get(i) ?? -1;
    const list = childrenByParentIdx.get(finalParent) ?? [];
    list.push({ bone: bones[i], idx: i });
    childrenByParentIdx.set(finalParent, list);
  }

  const resolveBoneIdxFromId = (boneSceneId: string): number => {
    const { index, globalId } = parseBoneSceneId(boneSceneId);
    if (index !== null && index >= 0 && index < bones.length) {
      return index;
    }
    return globalId !== null ? indexByGlobalId.get(globalId) ?? -1 : -1;
  };

  const isBoneDescendantOf = (candidateIdx: number, ancestorIdx: number): boolean => {
    let cur = candidateIdx;
    const visited = new Set<number>();
    while (cur >= 0 && !visited.has(cur)) {
      if (cur === ancestorIdx) return true;
      visited.add(cur);
      cur = parentIdxByBoneIdx.get(cur) ?? -1;
    }
    return false;
  };

  return {
    childrenByParentIdx,
    indexByGlobalId,
    parentIdxByBoneIdx,
    resolveBoneIdxFromId,
    isBoneDescendantOf,
  };
}

export function collectBatchIdsForSelection(
  manifest: WmbSceneManifest | null,
  selection: SceneSelectionItem[],
  includeTextureUsers = false
): Set<string> {
  const batchIds = new Set<string>();
  if (!manifest) return batchIds;

  for (const sel of selection) {
    if (sel.kind === "batch") {
      batchIds.add(sel.id);
    } else if (sel.kind === "material") {
      findMaterialById(manifest, sel.id)?.usedByBatchIds.forEach((bid) =>
        batchIds.add(bid)
      );
    } else if (sel.kind === "meshGroup") {
      const mg = findMeshGroupById(manifest, sel.id);
      if (mg) {
        mg.batchIds.forEach((bid) => batchIds.add(bid));
        mg.lods.forEach((l) => l.batchIds.forEach((bid) => batchIds.add(bid)));
      }
    } else if (sel.kind === "lod") {
      findLodById(manifest, sel.id)?.lod.batchIds.forEach((bid) =>
        batchIds.add(bid)
      );
    } else if (includeTextureUsers && sel.kind === "texture") {
      const texRef = findTextureBySceneId(manifest, sel.id);
      texRef?.usedByMaterialIds.forEach((mid) => {
        findMaterialById(manifest, mid)?.usedByBatchIds.forEach((bid) =>
          batchIds.add(bid)
        );
      });
    }
  }
  return batchIds;
}

export function computeSelectionBoundingBox(
  manifest: WmbSceneManifest | null,
  selection: SceneSelectionItem[]
): number[] | null {
  if (!manifest || selection.length === 0) return null;
  if (selection.some((s) => s.kind === "root")) {
    return manifest.boundingBox.length === 6 ? manifest.boundingBox : null;
  }

  const batchIds = collectBatchIdsForSelection(manifest, selection, false);
  const bonePoints: number[][] = [];
  for (const sel of selection) {
    if (sel.kind === "bone") {
      const bone = findBoneBySceneId(manifest, sel.id);
      if (bone && bone.worldPosition.length === 3) {
        bonePoints.push(bone.worldPosition);
      }
    }
  }

  let minX = Infinity;
  let minY = Infinity;
  let minZ = Infinity;
  let maxX = -Infinity;
  let maxY = -Infinity;
  let maxZ = -Infinity;
  let anyFound = false;

  for (const b of manifest.batches) {
    if (batchIds.has(b.id) && !b.isShadowProxy && b.bbox.length === 6) {
      minX = Math.min(minX, b.bbox[0]);
      minY = Math.min(minY, b.bbox[1]);
      minZ = Math.min(minZ, b.bbox[2]);
      maxX = Math.max(maxX, b.bbox[3]);
      maxY = Math.max(maxY, b.bbox[4]);
      maxZ = Math.max(maxZ, b.bbox[5]);
      anyFound = true;
    }
  }

  for (const pt of bonePoints) {
    const r = 0.15;
    minX = Math.min(minX, pt[0] - r);
    minY = Math.min(minY, pt[1] - r);
    minZ = Math.min(minZ, pt[2] - r);
    maxX = Math.max(maxX, pt[0] + r);
    maxY = Math.max(maxY, pt[1] + r);
    maxZ = Math.max(maxZ, pt[2] + r);
    anyFound = true;
  }

  if (!anyFound) {
    return manifest.boundingBox.length === 6 ? manifest.boundingBox : null;
  }
  return [minX, minY, minZ, maxX, maxY, maxZ];
}

export interface TextureSlotDebugRow {
  slotIndex: number;
  name: string;
  role: TextureSlotRole;
  thumbnailDataUrl: string | null;
}

export function collectDebugTextureSlots(
  manifest: WmbSceneManifest | null,
  targetMaterial?: MaterialDesc
): TextureSlotDebugRow[] {
  if (!manifest) return [];
  const texThumbById = new Map<number, string | null>();
  for (const t of manifest.textures) {
    texThumbById.set(t.textureId >>> 0, t.thumbnailDataUrl);
  }
  if (targetMaterial && targetMaterial.textureSlots.length > 0) {
    return targetMaterial.textureSlots.map((slot) => ({
      slotIndex: slot.slot,
      name: slot.name,
      role: slot.role,
      thumbnailDataUrl:
        texThumbById.get(slot.textureId >>> 0) ??
        (slot.slot === 0 ? targetMaterial.thumbnailDataUrl : null) ??
        null,
    }));
  }
  const bySlot = new Map<number, TextureSlotDebugRow>();
  for (const mat of manifest.materials) {
    for (const slot of mat.textureSlots) {
      const existing = bySlot.get(slot.slot);
      const thumb =
        texThumbById.get(slot.textureId >>> 0) ??
        (slot.slot === 0 ? mat.thumbnailDataUrl : null) ??
        null;
      if (!existing) {
        bySlot.set(slot.slot, {
          slotIndex: slot.slot,
          name: slot.name,
          role: slot.role,
          thumbnailDataUrl: thumb,
        });
      } else if (!existing.thumbnailDataUrl && thumb) {
        existing.thumbnailDataUrl = thumb;
      }
    }
  }
  return Array.from(bySlot.values()).sort((a, b) => a.slotIndex - b.slotIndex);
}
