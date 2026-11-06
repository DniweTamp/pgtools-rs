// [wmb-studio BEGIN phase2]
import {
  BatchDesc,
  BoneDesc,
  ClipboardStatus,
  LodDesc,
  MaterialDesc,
  MeshGroupDesc,
  Region,
  TextureRefDesc,
  TextureSlotRole,
  VertexBufferDesc,
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

function readFloat32Region(
  binary: ArrayBuffer,
  region: Region | null
): Float32Array | null {
  if (!region || region.omitted || region.byteLength <= 0) return null;
  if (region.offset + region.byteLength > binary.byteLength) return null;
  const slice = binary.slice(region.offset, region.offset + region.byteLength);
  return new Float32Array(slice);
}

function readUint8Region(
  binary: ArrayBuffer,
  region: Region | null
): Uint8Array | null {
  if (!region || region.omitted || region.byteLength <= 0) return null;
  if (region.offset + region.byteLength > binary.byteLength) return null;
  const slice = binary.slice(region.offset, region.offset + region.byteLength);
  return new Uint8Array(slice);
}

function readUint32Region(
  binary: ArrayBuffer,
  region: Region | null
): Uint32Array | null {
  if (!region || region.omitted || region.byteLength <= 0) return null;
  if (region.offset + region.byteLength > binary.byteLength) return null;
  const slice = binary.slice(region.offset, region.offset + region.byteLength);
  return new Uint32Array(slice);
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
        readFloat32Region(binary, vb.position) ??
        new Float32Array(vb.vertexCount * 3);
      const nrm = readFloat32Region(binary, vb.normal);
      const tan = readFloat32Region(binary, vb.tangent);
      const uv0 = readFloat32Region(binary, vb.uv0);
      const uv1 = readFloat32Region(binary, vb.uv1);
      const uv2 = readFloat32Region(binary, vb.uv2 ?? null);
      const col = readUint8Region(binary, vb.color);
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
        readUint32Region(binary, batch.indices) ?? new Uint32Array(0);
      cache.batchIndices.set(batch.geometryHash, idx);
      changedBatchHashes.add(batch.geometryHash);
    }
  }

  // Prune unreferenced geometry hashes
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

export function isItemPresentInManifest(
  manifest: WmbSceneManifest | null,
  item: SceneSelectionItem
): boolean {
  if (!manifest) return false;
  switch (item.kind) {
    case "root":
      return item.id === "root";
    case "texture":
      return manifest.textures.some(
        (t) => `tex:${t.textureId}` === item.id || t.idHex === item.id
      );
    case "material":
      return manifest.materials.some((m) => m.id === item.id);
    case "bone":
      return findBoneBySceneId(manifest, item.id) !== undefined;
    case "meshGroup":
      return manifest.meshGroups.some((mg) => mg.id === item.id);
    case "lod":
      return manifest.meshGroups.some((mg) =>
        mg.lods.some((l) => l.id === item.id)
      );
    case "batch":
      return manifest.batches.some((b) => b.id === item.id);
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
  if (valid.length === 0) {
    return [{ kind: "root", id: "root" }];
  }
  return valid;
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

export function getBoneSceneId(bone: BoneDesc, index: number): string {
  return `bone:${index}:${bone.globalId}`;
}

export function findBoneIndexAndDescBySceneId(
  manifest: WmbSceneManifest | null,
  id: string
): { bone: BoneDesc; index: number } | undefined {
  if (!manifest) return undefined;
  const raw = id.replace(/^bone:/, "");
  if (raw.includes(":")) {
    const [idxStr, gidStr] = raw.split(":");
    const idx = parseInt(idxStr, 10);
    const gid = parseInt(gidStr, 10);
    if (!Number.isNaN(idx) && idx >= 0 && idx < manifest.bones.length) {
      const candidate = manifest.bones[idx];
      if (Number.isNaN(gid) || candidate.globalId === gid) {
        return { bone: candidate, index: idx };
      }
    }
  }
  const gid = parseInt(raw, 10);
  if (Number.isNaN(gid)) return undefined;
  const foundIdx = manifest.bones.findIndex((b) => b.globalId === gid);
  if (foundIdx >= 0) {
    return { bone: manifest.bones[foundIdx], index: foundIdx };
  }
  if (gid >= 0 && gid < manifest.bones.length) {
    return { bone: manifest.bones[gid], index: gid };
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
  const raw = id.replace(/^tex:/, "");
  const tid = raw.toLowerCase().startsWith("0x")
    ? parseInt(raw.slice(2), 16)
    : parseInt(raw, 10);
  if (Number.isNaN(tid)) return undefined;
  return manifest.textures.find((t) => (t.textureId >>> 0) === (tid >>> 0));
}

export function computeSelectionBoundingBox(
  manifest: WmbSceneManifest | null,
  selection: SceneSelectionItem[]
): number[] | null {
  if (!manifest || selection.length === 0) return null;
  const batchIds = new Set<string>();
  const bonePoints: number[][] = [];

  for (const sel of selection) {
    if (sel.kind === "root") {
      return manifest.boundingBox.length === 6 ? manifest.boundingBox : null;
    } else if (sel.kind === "batch") {
      batchIds.add(sel.id);
    } else if (sel.kind === "material") {
      const mat = findMaterialById(manifest, sel.id);
      mat?.usedByBatchIds.forEach((bid) => batchIds.add(bid));
    } else if (sel.kind === "meshGroup") {
      const mg = findMeshGroupById(manifest, sel.id);
      if (mg) {
        mg.batchIds.forEach((bid) => batchIds.add(bid));
        mg.lods.forEach((l) => l.batchIds.forEach((bid) => batchIds.add(bid)));
      }
    } else if (sel.kind === "lod") {
      const found = findLodById(manifest, sel.id);
      found?.lod.batchIds.forEach((bid) => batchIds.add(bid));
    } else if (sel.kind === "bone") {
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
  manifest: WmbSceneManifest | null
): TextureSlotDebugRow[] {
  if (!manifest) return [];
  const bySlot = new Map<number, TextureSlotDebugRow>();
  const texThumbById = new Map<number, string | null>();
  for (const t of manifest.textures) {
    texThumbById.set(t.textureId >>> 0, t.thumbnailDataUrl);
  }
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

export type {
  BatchDesc,
  BoneDesc,
  ClipboardStatus,
  LodDesc,
  MaterialDesc,
  MeshGroupDesc,
  TextureRefDesc,
  TextureSlotRole,
  VertexBufferDesc,
  WmbSceneManifest,
  WmbScenePayload,
};
// [wmb-studio END]
