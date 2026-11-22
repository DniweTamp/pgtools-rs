import { invoke } from "@tauri-apps/api/core";
import { DecodedGpuSurface, decodeWtxBlob } from "../utils/gpuTexture";

export interface Region {
  offset: number;
  byteLength: number;
  omitted?: boolean;
}

export interface WmbSceneManifest {
  schema: 1;
  structureVersion: number;
  nodeId: string;
  fileName: string;
  game: "b1" | "b2" | "b3";
  isWmb3: boolean;
  boundingBox: number[];
  lodLevels: number[];
  meshGroups: MeshGroupDesc[];
  batches: BatchDesc[];
  materials: MaterialDesc[];
  bones: BoneDesc[];
  textures: TextureRefDesc[];
  vertexBuffers: VertexBufferDesc[];
}

export interface LodDesc {
  id: string;
  name: string;
  level: number;
  batchIds: string[];
}

export interface MeshGroupDesc {
  id: string;
  index: number;
  name: string;
  batchIds: string[];
  lods: LodDesc[];
}

export interface BatchDesc {
  id: string;
  index: number;
  meshId: string;
  lodId: string | null;
  fileOrder: number;
  materialIndex: number;
  materialId: string;
  isShadowProxy: boolean;
  vertexBufferIndex: number;
  indices: Region;
  indexCount: number;
  vertexCount: number;
  geometryHash: string;
  bbox: number[];
  hasDanglingMaterial?: boolean;
  flags?: number;
  exMatId?: number;
  rawMaterialId?: number;
  hasBoneRefs?: number;
  uE1?: number;
  uE2?: number;
  primitiveType?: number;
  vertexStart?: number;
  vertexEnd?: number;
  vertexOffset?: number;
  uF?: number[];
  boneRefs?: number[];
  unknownNoBones?: number[];
  boneSetIndex?: number;
  callTreeNodeIndex?: number;
  meshMaterialIndex?: number;
  uB?: number;
}

export interface VertexBufferDesc {
  index: number;
  vertexCount: number;
  geometryHash: string;
  position: Region;
  normal: Region | null;
  tangent: Region | null;
  uv0: Region | null;
  uv1: Region | null;
  uv2?: Region | null;
  color: Region | null;
}

export interface MaterialParamDesc {
  key: string;
  name: string;
  values: number[];
  isColor: boolean;
}

export interface MaterialDesc {
  id: string;
  index: number;
  name: string;
  game: "b1" | "b2" | "b3";
  shaderName: string | null;
  materialType: number | null;
  technique: string | null;
  textureSlots: TextureSlotDesc[];
  params: MaterialParamDesc[];
  usedByBatchIds: string[];
  thumbnailDataUrl: string | null;
}

export type TextureSlotRole =
  | "albedo"
  | "normal"
  | "spec"
  | "mask"
  | "env"
  | "lut"
  | "unknown";

export interface TextureSlotDesc {
  slot: number;
  name: string;
  role: TextureSlotRole;
  textureId: number;
  textureIdHex: string;
  global: boolean;
  present: boolean;
}

export interface TextureRefDesc {
  textureId: number;
  idHex: string;
  name: string;
  width: number;
  height: number;
  formatName: string;
  contentHash: string;
  usedByMaterialIds: string[];
  thumbnailDataUrl: string | null;
}

export interface BoneDesc {
  globalId: number;
  localId: number | null;
  parent: number;
  name: string | null;
  localPosition: number[];
  worldPosition: number[];
  rotation: number[] | null;
  scale: number[] | null;
}

export interface ClipboardStatus {
  game: "b1" | "b2" | "b3";
  kind: "batches" | "materials";
  count: number;
}

export interface WmbScenePayload {
  manifest: WmbSceneManifest;
  binary: ArrayBuffer;
}

export type DecodedWtx1Texture = DecodedGpuSurface;


function align4(n: number): number {
  const rem = n % 4;
  return rem === 0 ? n : n + (4 - rem);
}

/**
 * Decode a binary scene response (`u32 LE manifest_byte_length | UTF-8 JSON | 4B pad | binary region`)
 * into `{ manifest, binary }`.
 */
export function decodeWmbSceneBlob(raw: ArrayBuffer): WmbScenePayload {
  if (raw.byteLength < 4) {
    throw new Error("Scene response buffer is shorter than 4-byte header");
  }
  const view = new DataView(raw);
  const manifestByteLength = view.getUint32(0, true);
  if (4 + manifestByteLength > raw.byteLength) {
    throw new Error("Scene response buffer is truncated inside manifest JSON");
  }
  const jsonBytes = new Uint8Array(raw, 4, manifestByteLength);
  const jsonStr = new TextDecoder("utf-8").decode(jsonBytes);
  const manifest = JSON.parse(jsonStr) as WmbSceneManifest;
  const binaryStart = Math.min(align4(4 + manifestByteLength), raw.byteLength);
  const binary = raw.slice(binaryStart);
  return { manifest, binary };
}

/**
 * Decode a `WTX1` binary texture response (`u32 magic 0x31585457 | u32 width | u32 height | u32 flags | RGBA8`)
 * into `{ width, height, cube, rgba }`.
 */
export function decodeWtx1TextureBlob(raw: ArrayBuffer): DecodedWtx1Texture {
  return decodeWtxBlob(raw);
}

export async function getWmbScene(
  nodeId: string,
  knownGeometryHashes: string[] = []
): Promise<WmbScenePayload> {
  const buf = await invoke<ArrayBuffer>("get_wmb_scene", {
    nodeId,
    knownGeometryHashes,
  });
  return decodeWmbSceneBlob(buf);
}

export async function getWmbTexture(
  nodeId: string,
  textureId: number,
  maxDim = 0
): Promise<DecodedWtx1Texture> {
  const buf = await invoke<ArrayBuffer>("get_wmb_texture", {
    nodeId,
    textureId: textureId >>> 0,
    maxDim: maxDim >>> 0,
  });
  return decodeWtx1TextureBlob(buf);
}

export async function getPreTexture(
  game: "b1" | "b2" | "b3",
  index: number,
  maxDim = 0
): Promise<DecodedWtx1Texture> {
  const buf = await invoke<ArrayBuffer>("get_pre_texture", {
    game,
    index: index >>> 0,
    maxDim: maxDim >>> 0,
  });
  return decodeWtx1TextureBlob(buf);
}

export async function resolveLegacyNode(
  nodeId: string,
  itemId: string
): Promise<string> {
  return invoke<string>("resolve_legacy_node", { nodeId, itemId });
}

export async function createDemoModel(
  game: "b1" | "b2" | "b3"
): Promise<string> {
  return invoke<string>("create_demo_model", { game });
}

export async function wmbClipboardCopy(
  nodeId: string,
  items: string[]
): Promise<ClipboardStatus> {
  return invoke<ClipboardStatus>("wmb_clipboard_copy", { nodeId, items });
}

export async function wmbClipboardStatus(): Promise<ClipboardStatus | null> {
  return invoke<ClipboardStatus | null>("wmb_clipboard_status");
}

export type LegacyActionKind =
  | "export_obj"
  | "import_obj"
  | "import_gltf"
  | "export_gltf"
  | "cleanup_vertexes"
  | "cleanup_materials"
  | "recompute_relative_positions"
  | "set_tpose"
  | "bone_preset"
  | "export_texture"
  | "replace_texture"
  | "import_texture";

export interface MaterialShaderStatus {
  materialId: string;
  shaderId: string;
  isFallback: boolean;
  hasCompileError: boolean;
  compileErrorLog?: string;
}

export async function convertMaterialBetweenGames<T = any>(
  srcGame: "b1" | "b2" | "b3",
  dstGame: "b1" | "b2" | "b3",
  materialPayload: unknown,
  targetShader: string | null = null
): Promise<{ converted: T; report: any }> {
  return invoke<{ converted: T; report: any }>("convert_material_action", {
    srcGame,
    dstGame,
    materialPayload,
    targetShader,
  });
}
