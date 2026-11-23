import { invoke } from "@tauri-apps/api/core";
import {
  BatchOperationReport,
  BatchReplaceScanResult,
  CubemapPreviewPayload,
  DocumentOutline,
  FsEntry,
  NodeInspectPayload,
  OodleValidationReport,
  SaveEncodeOptions,
  StudioSettings,
} from "../types";

export type BayoGame = "b1" | "b2" | "b3";

export interface PickFileDialogOptions {
  title?: string | null;
  extensions?: string[] | null;
}

export interface SaveFileDialogOptions {
  title?: string | null;
  defaultName?: string | null;
}

export interface PickFolderDialogOptions {
  title?: string | null;
}

export function pickFileDialog(
  options: PickFileDialogOptions = {}
): Promise<string | null> {
  return invoke<string | null>("pick_file_dialog", {
    title: options.title ?? null,
    extensions: options.extensions ?? null,
  });
}

export function pickMultipleFilesDialog(
  options: PickFileDialogOptions = {}
): Promise<string[]> {
  return invoke<string[]>("pick_multiple_files_dialog", {
    title: options.title ?? null,
    extensions: options.extensions ?? null,
  });
}

export function saveFileDialog(
  options: SaveFileDialogOptions = {}
): Promise<string | null> {
  return invoke<string | null>("save_file_dialog", {
    title: options.title ?? null,
    defaultName: options.defaultName ?? null,
  });
}

export function pickFolderDialog(
  options: PickFolderDialogOptions = {}
): Promise<string | null> {
  return invoke<string | null>("pick_folder_dialog", {
    title: options.title ?? null,
  });
}

export function getStudioSettings(): Promise<StudioSettings> {
  return invoke<StudioSettings>("get_studio_settings");
}

export function saveStudioSettings(
  settings: StudioSettings
): Promise<StudioSettings> {
  return invoke<StudioSettings>("save_studio_settings", { settings });
}

export function getRecentFiles(): Promise<string[]> {
  return invoke<string[]>("get_recent_files");
}

export function validateOodleDll(
  path: string | null = null
): Promise<OodleValidationReport> {
  return invoke<OodleValidationReport>("validate_oodle_dll", { path });
}

export function openDocument(path: string): Promise<DocumentOutline> {
  return invoke<DocumentOutline>("open_document", { path });
}

export function saveDocument(
  outputPath: string | null = null,
  encodeOptions: SaveEncodeOptions | null = null
): Promise<DocumentOutline> {
  return invoke<DocumentOutline>("save_document", {
    outputPath,
    encodeOptions,
  });
}

export function undoDocument(): Promise<DocumentOutline> {
  return invoke<DocumentOutline>("undo_document");
}

export function redoDocument(): Promise<DocumentOutline> {
  return invoke<DocumentOutline>("redo_document");
}

export function inspectAssetNode<T = NodeInspectPayload>(params: {
  nodeId: string;
  mipIndex: number;
  layerIndex?: number;
  channel: string;
  opaque: boolean;
  b2Override?: boolean | null;
}): Promise<T> {
  return invoke<T>("inspect_asset_node", params);
}

export function dragOutAssetNode(nodeId: string): Promise<string> {
  return invoke<string>("drag_out_asset_node", { nodeId });
}

export function packFolderToPkz(params: {
  sourceDir: string;
  destPath: string;
  compression: string;
  level: number;
  forceCompression: boolean;
  allowZstdFallback: boolean;
}): Promise<string> {
  return invoke<string>("pack_folder_to_pkz", params);
}

export function mutateWmbNode(
  nodeId: string,
  operation: string,
  payload: unknown = {}
): Promise<DocumentOutline> {
  return invoke<DocumentOutline>("mutate_wmb_node", {
    nodeId,
    operation,
    payload,
  });
}

export function mutateTextureNode(
  nodeId: string,
  action: string,
  payload: unknown = {}
): Promise<DocumentOutline> {
  return invoke<DocumentOutline>("mutate_texture_node", {
    nodeId,
    action,
    payload,
  });
}

export function mutateBxmNode(params: {
  nodeId: string;
  xmlContent: string;
  tag: string;
}): Promise<DocumentOutline> {
  return invoke<DocumentOutline>("mutate_bxm_node", params);
}

export function mutateStructNode(
  nodeId: string,
  structKind: string,
  updatedData: unknown
): Promise<DocumentOutline> {
  return invoke<DocumentOutline>("mutate_struct_node", {
    nodeId,
    structKind,
    updatedData,
  });
}

export function mutateClothNode(
  nodeId: string,
  operation: string,
  preset: string | null = null,
  updatedJson: unknown = null
): Promise<DocumentOutline> {
  return invoke<DocumentOutline>("mutate_cloth_node", {
    nodeId,
    operation,
    preset,
    updatedJson,
  });
}

export function mutateContainerNode(
  nodeId: string,
  action: string,
  targetPath = "",
  payload: unknown = null
): Promise<DocumentOutline> {
  return invoke<DocumentOutline>("mutate_container_node", {
    nodeId,
    action,
    targetPath,
    payload,
  });
}

export function mutateUvdNode(
  nodeId: string,
  action: string,
  payload: unknown = {}
): Promise<DocumentOutline> {
  return invoke<DocumentOutline>("mutate_uvd_node", {
    nodeId,
    action,
    payload,
  });
}

export function extractUvdAction(params: {
  nodeIds: string[];
  destPath: string;
  writeJson: boolean;
}): Promise<string> {
  return invoke<string>("extract_uvd_action", params);
}

export function exportTextureAction(params: {
  nodeId: string;
  scope: string;
  format: string;
  destPath: string;
}): Promise<string> {
  return invoke<string>("export_texture_action", params);
}

export function convertMaterialBetweenGames<T = any>(
  srcGame: BayoGame,
  dstGame: BayoGame,
  materialPayload: unknown,
  targetShader: string | null = null
): Promise<{ converted: T; report: unknown }> {
  return invoke<{ converted: T; report: unknown }>("convert_material_action", {
    srcGame,
    dstGame,
    materialPayload,
    targetShader,
  });
}

export function getTextureGpuSurface(params: {
  nodeId: string;
  mipIndex: number;
  layerIndex: number;
}): Promise<ArrayBuffer> {
  return invoke<ArrayBuffer>("get_texture_gpu_surface", params);
}

export function getTextureGpuCubemap(params: {
  nodeId: string;
  mipIndex: number;
}): Promise<ArrayBuffer> {
  return invoke<ArrayBuffer>("get_texture_gpu_cubemap", params);
}

export function getCubemapFacesPreview(params: {
  nodeId: string;
  mipIndex: number;
  opaque: boolean;
}): Promise<CubemapPreviewPayload> {
  return invoke<CubemapPreviewPayload>("get_cubemap_faces_preview", params);
}

export function listDirectory(
  dirPath: string | null = null
): Promise<[string, FsEntry[]]> {
  return invoke<[string, FsEntry[]]>("list_directory", { dirPath });
}

export function runFileContextOperation(params: {
  filePath: string;
  operation: string;
  destPath: string | null;
}): Promise<string> {
  return invoke<string>("run_file_context_operation", params);
}

export function fsBatchOperation(params: {
  operation: string;
  sourcePaths: string[];
  destDir: string | null;
  newName: string | null;
}): Promise<string> {
  return invoke<string>("fs_batch_operation", params);
}

export function revealInExplorer(path: string): Promise<void> {
  return invoke<void>("reveal_in_explorer", { path });
}

export function runBatchRecompressAction(params: {
  sourceMode: string;
  folderPath: string | null;
  recursive: boolean;
  opaqueFormat: string;
  alphaFormat: string;
  opaqueQuality: string;
  alphaQuality: string;
  downscale: boolean;
  skipNoMips: boolean;
  syncFlags: boolean;
}): Promise<BatchOperationReport> {
  return invoke<BatchOperationReport>("run_batch_recompress_action", params);
}

export function runFolderTextureExport(params: {
  sourceFolder: string;
  recursive: boolean;
  outputFolder: string;
  subfolderPerBank: boolean;
  format: string;
  includeBntxName: boolean;
}): Promise<BatchOperationReport> {
  return invoke<BatchOperationReport>("run_folder_texture_export", params);
}

export function createEmptyBankDocument(params: {
  outputPath: string;
  isSplitWtaWtp: boolean;
  gameProfile: string;
}): Promise<DocumentOutline> {
  return invoke<DocumentOutline>("create_empty_bank_document", params);
}

export function runBatchDatAlignAction(params: {
  sourceMode: string;
  folderPath: string | null;
  filePaths: string[] | null;
  recursive: boolean;
  pairDtt: boolean;
  customAlign: number | null;
  perTypeDefault: boolean;
  alignNested: boolean;
  outputFolder: string | null;
}): Promise<BatchOperationReport> {
  return invoke<BatchOperationReport>("run_batch_dat_align_action", params);
}

export function scanBatchReplaceFolder(params: {
  nodeId: string;
  folder: string;
}): Promise<BatchReplaceScanResult> {
  return invoke<BatchReplaceScanResult>("scan_batch_replace_folder", params);
}

export function getTextureGpuThumbnailsBatch(
  nodeIds: string[]
): Promise<ArrayBuffer> {
  return invoke<ArrayBuffer>("get_texture_gpu_thumbnails_batch", { nodeIds });
}

export function getTextureThumbnailsBatch(
  nodeIds: string[]
): Promise<[string, string | null][]> {
  return invoke<[string, string | null][]>("get_texture_thumbnails_batch", {
    nodeIds,
  });
}
