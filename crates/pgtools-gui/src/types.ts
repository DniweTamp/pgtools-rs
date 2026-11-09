export interface StudioSettings {
  theme: string;
  recent_files: string[];
  last_dir: string;
  hide_unsupported: boolean;
  file_sort_col: string;
  file_sort_asc: boolean;
  left_width: number;
  right_width: number;
  asset_ratio: number;
  oodle_dll_path: string;
  nvtt_dll_path: string;
  pkz_default_compression?: string;
  pkz_default_level?: number;
  core_dat_b1: string;
  core_dat_b2: string;
  core_dat_b3: string;
  wmb_hierarchy_ratio?: number;
  wmb_wireframe?: boolean;
  wmb_normals_view?: boolean;
  wmb_skeleton_overlay?: boolean;
  wmb_grid_visible?: boolean;
}

export interface OodleValidationReport {
  found: boolean;
  resolved_path: string;
  can_decompress: boolean;
  can_compress: boolean;
  message: string;
}

export interface PkzCompressionModalState {
  title: string;
  subtitle?: string;
  compression: "None" | "ZStandard" | "OodleKraken";
  level: number;
  forceCompression: boolean;
  onConfirm: (
    compression: "None" | "ZStandard" | "OodleKraken",
    level: number,
    forceCompression: boolean
  ) => Promise<void> | void;
}

export interface PkzConflictModalState {
  kind: "collision" | "extract_overwrite";
  title: string;
  message: string;
  paths?: string[];
  onChoice: (policy: "overwrite" | "skip") => Promise<void> | void;
}

export type PkzSortCol = "pkz_order" | "name" | "format" | "compression" | "size";

export interface FsEntry {
  name: string;
  path: string;
  is_dir: boolean;
  size: number;
  modified_epoch_ms: number;
  modified_str: string;
  extension: string;
  is_supported: boolean;
}

export interface AssetTreeNode {
  id: string;
  label: string;
  kind: string;
  format_badge: string;
  size: number;
  modified?: boolean;
  compression?: string | null;
  children: AssetTreeNode[];
}

export interface DocumentOutline {
  path: string;
  paired_path?: string | null;
  file_name: string;
  root_format: string;
  endian: string;
  summary_right: string;
  dirty: boolean;
  can_undo: boolean;
  can_redo: boolean;
  undo_label: string | null;
  redo_label: string | null;
  tree: AssetTreeNode[];
}

export interface MipLevelDesc {
  index: number;
  label: string;
  width: number;
  height: number;
}

export interface TextureNodeView {
  node_id: string;
  index: number;
  title: string;
  breadcrumb: string;
  spec_line: string;
  validation_ok: boolean;
  validation_msg: string;
  format_name: string;
  container_ext: string;
  game_profile: string;
  has_confirmed_xt1_flags: boolean;
  width: number;
  height: number;
  mip_count: number;
  mips: MipLevelDesc[];
  size_bytes: number;
  flags_hex: string;
  table2_bytes_str: string;
  has_crc32_flag: boolean;
  srgb_flag: boolean;
  cubemap_flag: boolean;
  texture_id_hex: string;
  preview_data_url: string | null;
  raw_header_hex: string;
}

export interface DecodedMaterialParameter {
  name: string;
  param_type: string;
  sampler_tex_id: number | null;
  sampler_flags?: number | null;
  float_values: [number, number, number, number] | null;
  role?: string | null;
  description?: string | null;
  component_labels?: [string, string, string, string] | null;
  pretexture_name?: string | null;
  is_color?: boolean;
}

export interface DecodedWmbMaterial {
  index: number;
  mat_type: number;
  flag: number;
  is_bayo2?: boolean;
  shader_name: string | null;
  shader_family?: string | null;
  shader_flags_notes?: string[];
  expected_byte_size: number | null;
  fields: DecodedMaterialParameter[];
  raw_dwords: number[];
}

export interface MaterialBankTextureOption {
  index: number;
  texture_id: number;
  texture_id_hex: string;
  label: string;
  format_name: string;
  width: number;
  height: number;
  node_id: string;
  thumbnail_data_url: string | null;
}

export interface MaterialPickerOption {
  index: number;
  label: string;
  shader_name: string;
  badge: string;
  node_id: string;
  first_tex_id_hex: string | null;
  thumbnail_data_url: string | null;
}

export interface CorePretextureInfo {
  id: number;
  id_hex: string;
  tex_id?: number;
  tex_id_hex?: string;
  name: string;
  role: string;
  description: string;
  game?: string;
}

export interface ShaderOptionDesc {
  key: string;
  id?: string;
  label: string;
  family: string;
  summary: string;
  mat_type?: number | null;
}

export interface Wmb3NamedParameter {
  group_index: number;
  param_index: number;
  key: string;
  name: string;
  value: number;
}

export interface Wmb3TextureSlotView {
  slot_index: number;
  name: string;
  texture_id: number;
  texture_id_hex: string;
  pretexture_name?: string | null;
  role?: string | null;
}

export interface Wmb3Material {
  date?: [number, number, number, number];
  name: string;
  shader_name: string;
  technique_name: string;
  u_a: number;
  textures: [string, number][];
  parameter_groups: [number, number[]][];
  variables: [string, number][];
}

export interface WmbSectionView {
  node_id: string;
  section: string;
  title: string;
  breadcrumb: string;
  spec_line: string;
  validation_msg: string;
  is_wmb3: boolean;
  is_bayo2: boolean;
  endian: string;
  data: any;
  material_decoded: DecodedWmbMaterial | null;
  wmb3_material?: Wmb3Material | null;
  wmb3_named_params?: Wmb3NamedParameter[];
  wmb3_texture_views?: Wmb3TextureSlotView[];
  bank_textures?: MaterialBankTextureOption[];
  core_pretextures?: CorePretextureInfo[];
  available_shaders?: ShaderOptionDesc[];
  material_options?: MaterialPickerOption[];
}

export interface BxmXmlNode {
  name: string;
  attributes: [string, string][];
  value: string | null;
  children: BxmXmlNode[];
}

export type ThemeMode = "dark" | "light";
export type FileSortCol = "name" | "type" | "modified" | "size";

export interface MenuItemSpec {
  label?: string;
  shortcut?: string;
  disabled?: boolean;
  separator?: boolean;
  action?: () => void;
  submenu?: MenuItemSpec[];
}

export interface ContextMenuState {
  x: number;
  y: number;
  items: MenuItemSpec[];
}

export interface PointerDragState {
  kind: "asset" | "bxm";
  sourceId: string;
  label: string;
  startX: number;
  startY: number;
  curX: number;
  curY: number;
  active: boolean;
  overId: string | null;
  insertPos: "before" | "after" | null;
  dragOutTriggered: boolean;
}

export interface TexModalState {
  nodeId: string;
  action: "replace_file" | "add_texture";
  path: string;
  fileName: string;
  encodeFormat: string;
  generateMips: boolean;
  solidify: boolean;
  makeOpaque: boolean;
}

export interface MaterialClipboardState {
  game: "b1" | "b2" | "b3";
  payload: any;
}

/** Result of inspect_asset_node — discriminated by view_type. */
export type NodeViewType =
  | "TEXTURE"
  | "WMB"
  | "BXM"
  | "EST"
  | "CSV"
  | "RAW_STRUCT"
  | "CLOTH"
  | "EMPTY"
  | string;

export interface NodeInspectPayload {
  node_id: string;
  view_type: NodeViewType;
  title?: string;
  tag?: string;
  texture?: TextureNodeView;
  wmb?: WmbSectionView;
  /** BXM-specific tree payload when view_type === "BXM" */
  bxm?: BxmXmlNode;
  est?: any;
  csv?: { headers: string[]; rows: string[][] };
  data?: any;
  [key: string]: any;
}


/** Opaque structured-editor records (WMB meshes/batches/bones) until fully typed. */
export type WmbLooseRecord = Record<string, unknown>;
