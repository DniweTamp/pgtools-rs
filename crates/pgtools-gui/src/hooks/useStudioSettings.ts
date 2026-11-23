import React, { useCallback, useEffect, useRef, useState } from "react";
import { FileSortCol, StudioSettings, ThemeMode } from "../types";

import {
  DEFAULT_CUSTOM_DAT_ALIGNMENT,
  sanitizeCustomDatAlignment,
} from "../constants/datAlignment";
export {
  DEFAULT_CUSTOM_DAT_ALIGNMENT,
  parseCustomDatAlignmentInput,
  sanitizeCustomDatAlignment,
} from "../constants/datAlignment";

export interface StudioSettingsState {
  settingsLoaded: boolean;
  theme: ThemeMode;
  leftWidth: number;
  rightWidth: number;
  assetRatio: number;
  oodleDllPath: string;
  nvttDllPath: string;
  oodleConfigured: boolean;
  nvttConfigured: boolean;
  pkzDefaultCompression: "None" | "ZStandard" | "OodleKraken";
  pkzDefaultLevel: number;
  forceCustomDatAlignment: boolean;
  customDatAlignment: number;
  coreDatB1: string;
  coreDatB2: string;
  coreDatB3: string;
  wmbHierarchyRatio: number;
  wmbWireframe: boolean;
  wmbNormalsView: boolean;
  wmbSkeletonOverlay: boolean;
  wmbGridVisible: boolean;
  recentFiles: string[];
  hideUnsupported: boolean;
  fileSortCol: FileSortCol;
  fileSortAsc: boolean;
  defaultImportFormat: string;
  defaultEncodeQuality: string;
  syncXt1FlagsOnImport: boolean;
  assetBrowserZoom: number;
  uvdPadding: number;
  uvdMaxSize: number;
}

export interface StudioSettingsApi extends StudioSettingsState {
  setTheme: React.Dispatch<React.SetStateAction<ThemeMode>>;
  setLeftWidth: React.Dispatch<React.SetStateAction<number>>;
  setRightWidth: React.Dispatch<React.SetStateAction<number>>;
  setAssetRatio: React.Dispatch<React.SetStateAction<number>>;
  setOodleDllPath: React.Dispatch<React.SetStateAction<string>>;
  setNvttDllPath: React.Dispatch<React.SetStateAction<string>>;
  setOodleConfigured: React.Dispatch<React.SetStateAction<boolean>>;
  setNvttConfigured: React.Dispatch<React.SetStateAction<boolean>>;
  setPkzDefaultCompression: React.Dispatch<
    React.SetStateAction<"None" | "ZStandard" | "OodleKraken">
  >;
  setPkzDefaultLevel: React.Dispatch<React.SetStateAction<number>>;
  setForceCustomDatAlignment: React.Dispatch<React.SetStateAction<boolean>>;
  setCustomDatAlignment: React.Dispatch<React.SetStateAction<number>>;
  setCoreDatB1: React.Dispatch<React.SetStateAction<string>>;
  setCoreDatB2: React.Dispatch<React.SetStateAction<string>>;
  setCoreDatB3: React.Dispatch<React.SetStateAction<string>>;
  setWmbHierarchyRatio: React.Dispatch<React.SetStateAction<number>>;
  setWmbWireframe: React.Dispatch<React.SetStateAction<boolean>>;
  setWmbNormalsView: React.Dispatch<React.SetStateAction<boolean>>;
  setWmbSkeletonOverlay: React.Dispatch<React.SetStateAction<boolean>>;
  setWmbGridVisible: React.Dispatch<React.SetStateAction<boolean>>;
  setRecentFiles: (f: string[] | ((prev: string[]) => string[])) => void;
  setHideUnsupported: React.Dispatch<React.SetStateAction<boolean>>;
  setFileSortCol: React.Dispatch<React.SetStateAction<FileSortCol>>;
  setFileSortAsc: React.Dispatch<React.SetStateAction<boolean>>;
  setDefaultImportFormat: React.Dispatch<React.SetStateAction<string>>;
  setDefaultEncodeQuality: React.Dispatch<React.SetStateAction<string>>;
  setSyncXt1FlagsOnImport: React.Dispatch<React.SetStateAction<boolean>>;
  setAssetBrowserZoom: React.Dispatch<React.SetStateAction<number>>;
  setUvdPadding: React.Dispatch<React.SetStateAction<number>>;
  setUvdMaxSize: React.Dispatch<React.SetStateAction<number>>;
  buildStudioSettingsPayload: (
    currentDir: string,
    overrides?: Partial<StudioSettings>
  ) => StudioSettings;
  applyLoadedSettings: (s: StudioSettings) => void;
  markLoaded: () => void;
}

export function normalizePkzCodec(
  raw?: string | null
): "None" | "ZStandard" | "OodleKraken" {
  const low = (raw ?? "").trim().toLowerCase();
  if (low.includes("oodle") || low.includes("kraken")) return "OodleKraken";
  if (low === "none" || low === "uncompressed") return "None";
  return "ZStandard";
}

export function clampPkzLevel(
  codec: "None" | "ZStandard" | "OodleKraken",
  level: number
): number {
  if (codec === "None") return 0;
  if (codec === "OodleKraken") return Math.max(1, Math.min(9, Math.round(level || 4)));
  return Math.max(1, Math.min(22, Math.round(level || 3)));
}

export function useStudioSettings(): StudioSettingsApi {
  const [settingsLoaded, setSettingsLoaded] = useState(false);
  const [theme, setTheme] = useState<ThemeMode>("dark");
  const [leftWidth, setLeftWidth] = useState(310);
  const [rightWidth, setRightWidth] = useState(380);
  const [assetRatio, setAssetRatio] = useState(0.5);
  const [oodleDllPath, setOodleDllPath] = useState("");
  const [nvttDllPath, setNvttDllPath] = useState("");
  const [oodleConfigured, setOodleConfigured] = useState(false);
  const [nvttConfigured, setNvttConfigured] = useState(false);
  const [pkzDefaultCompression, setPkzDefaultCompression] = useState<
    "None" | "ZStandard" | "OodleKraken"
  >("ZStandard");
  const [pkzDefaultLevel, setPkzDefaultLevel] = useState<number>(3);
  const [forceCustomDatAlignment, setForceCustomDatAlignment] = useState<boolean>(false);
  const [customDatAlignment, setCustomDatAlignment] = useState<number>(
    DEFAULT_CUSTOM_DAT_ALIGNMENT
  );
  const [coreDatB1, setCoreDatB1] = useState("");
  const [coreDatB2, setCoreDatB2] = useState("");
  const [coreDatB3, setCoreDatB3] = useState("");
  const [wmbHierarchyRatio, setWmbHierarchyRatio] = useState(0.48);
  const [wmbWireframe, setWmbWireframe] = useState(false);
  const [wmbNormalsView, setWmbNormalsView] = useState(false);
  const [wmbSkeletonOverlay, setWmbSkeletonOverlay] = useState(false);
  const [wmbGridVisible, setWmbGridVisible] = useState(true);
  const [recentFiles, setRecentFiles] = useState<string[]>([]);
  const [hideUnsupported, setHideUnsupported] = useState(true);
  const [fileSortCol, setFileSortCol] = useState<FileSortCol>("name");
  const [fileSortAsc, setFileSortAsc] = useState(true);
  const [defaultImportFormat, setDefaultImportFormat] = useState<string>("Auto");
  const [defaultEncodeQuality, setDefaultEncodeQuality] = useState<string>("Normal");
  const [syncXt1FlagsOnImport, setSyncXt1FlagsOnImport] = useState<boolean>(true);
  const [assetBrowserZoom, setAssetBrowserZoom] = useState<number>(0);
  const [uvdPadding, setUvdPadding] = useState<number>(4);
  const [uvdMaxSize, setUvdMaxSize] = useState<number>(4096);

  const stateRef = useRef({
    theme,
    recentFiles,
    hideUnsupported,
    fileSortCol,
    fileSortAsc,
    leftWidth,
    rightWidth,
    assetRatio,
    oodleDllPath,
    nvttDllPath,
    pkzDefaultCompression,
    pkzDefaultLevel,
    forceCustomDatAlignment,
    customDatAlignment,
    coreDatB1,
    coreDatB2,
    coreDatB3,
    wmbHierarchyRatio,
    wmbWireframe,
    wmbNormalsView,
    wmbSkeletonOverlay,
    wmbGridVisible,
    defaultImportFormat,
    defaultEncodeQuality,
    syncXt1FlagsOnImport,
    assetBrowserZoom,
    uvdPadding,
    uvdMaxSize,
  });
  stateRef.current = {
    theme,
    recentFiles,
    hideUnsupported,
    fileSortCol,
    fileSortAsc,
    leftWidth,
    rightWidth,
    assetRatio,
    oodleDllPath,
    nvttDllPath,
    pkzDefaultCompression,
    pkzDefaultLevel,
    forceCustomDatAlignment,
    customDatAlignment,
    coreDatB1,
    coreDatB2,
    coreDatB3,
    wmbHierarchyRatio,
    wmbWireframe,
    wmbNormalsView,
    wmbSkeletonOverlay,
    wmbGridVisible,
    defaultImportFormat,
    defaultEncodeQuality,
    syncXt1FlagsOnImport,
    assetBrowserZoom,
    uvdPadding,
    uvdMaxSize,
  };

  const applyLoadedSettings = useCallback((s: StudioSettings) => {
    const loadedTheme: ThemeMode = s.theme?.toLowerCase() === "light" ? "light" : "dark";
    setTheme(loadedTheme);
    document.documentElement.setAttribute("data-theme", loadedTheme);
    setRecentFiles(s.recent_files ?? []);
    setHideUnsupported(s.hide_unsupported ?? true);
    if (["name", "type", "modified", "size"].includes(s.file_sort_col)) {
      setFileSortCol(s.file_sort_col as FileSortCol);
    }
    setFileSortAsc(s.file_sort_asc ?? true);
    if (s.left_width >= 200 && s.left_width <= 650) setLeftWidth(s.left_width);
    if (s.right_width >= 260 && s.right_width <= 700) setRightWidth(s.right_width);
    if (s.asset_ratio >= 0.18 && s.asset_ratio <= 0.82) setAssetRatio(s.asset_ratio);
    setOodleDllPath(s.oodle_dll_path ?? "");
    setNvttDllPath(s.nvtt_dll_path ?? "");
    setOodleConfigured(Boolean(s.oodle_configured ?? (s.oodle_dll_path && s.oodle_dll_path.trim().length > 0)));
    setNvttConfigured(Boolean(s.nvtt_configured ?? (s.nvtt_dll_path && s.nvtt_dll_path.trim().length > 0)));
    const normCodec = normalizePkzCodec(s.pkz_default_compression);
    setPkzDefaultCompression(normCodec);
    setPkzDefaultLevel(clampPkzLevel(normCodec, s.pkz_default_level ?? 3));
    setForceCustomDatAlignment(Boolean(s.force_custom_dat_alignment ?? false));
    setCustomDatAlignment(sanitizeCustomDatAlignment(s.custom_dat_alignment));
    setCoreDatB1(s.core_dat_b1 ?? "");
    setCoreDatB2(s.core_dat_b2 ?? "");
    setCoreDatB3(s.core_dat_b3 ?? "");
    if (
      typeof s.wmb_hierarchy_ratio === "number" &&
      s.wmb_hierarchy_ratio >= 0.15 &&
      s.wmb_hierarchy_ratio <= 0.85
    ) {
      setWmbHierarchyRatio(s.wmb_hierarchy_ratio);
    }
    if (typeof s.wmb_wireframe === "boolean") setWmbWireframe(s.wmb_wireframe);
    if (typeof s.wmb_normals_view === "boolean") setWmbNormalsView(s.wmb_normals_view);
    if (typeof s.wmb_skeleton_overlay === "boolean") setWmbSkeletonOverlay(s.wmb_skeleton_overlay);
    if (typeof s.wmb_grid_visible === "boolean") setWmbGridVisible(s.wmb_grid_visible);
    if (typeof s.default_import_format === "string" && s.default_import_format.trim()) {
      setDefaultImportFormat(s.default_import_format);
    }
    if (typeof s.default_encode_quality === "string" && s.default_encode_quality.trim()) {
      setDefaultEncodeQuality(s.default_encode_quality);
    }
    if (typeof s.sync_xt1_flags_on_import === "boolean") {
      setSyncXt1FlagsOnImport(s.sync_xt1_flags_on_import);
    }
    if (typeof s.asset_browser_zoom === "number" && s.asset_browser_zoom >= 0 && s.asset_browser_zoom <= 4) setAssetBrowserZoom(s.asset_browser_zoom);
    if (typeof s.uvd_padding === "number" && s.uvd_padding >= 0 && s.uvd_padding <= 64) {
      setUvdPadding(Math.round(s.uvd_padding));
    }
    if (typeof s.uvd_max_size === "number" && s.uvd_max_size >= 64 && s.uvd_max_size <= 16384) {
      setUvdMaxSize(Math.round(s.uvd_max_size));
    }
  }, []);

  const markLoaded = useCallback(() => setSettingsLoaded(true), []);

  useEffect(() => {
    document.documentElement.setAttribute("data-theme", theme);
  }, [theme]);

  const buildStudioSettingsPayload = useCallback(
    (currentDir: string, overrides?: Partial<StudioSettings>): StudioSettings => {
      const s = stateRef.current;
      return {
        theme: s.theme === "light" ? "Light" : "Dark",
        recent_files: s.recentFiles,
        last_dir: currentDir,
        hide_unsupported: s.hideUnsupported,
        file_sort_col: s.fileSortCol,
        file_sort_asc: s.fileSortAsc,
        left_width: Math.round(s.leftWidth),
        right_width: Math.round(s.rightWidth),
        asset_ratio: s.assetRatio,
        oodle_dll_path: s.oodleDllPath,
        nvtt_dll_path: s.nvttDllPath,
        pkz_default_compression: s.pkzDefaultCompression,
        pkz_default_level: s.pkzDefaultLevel,
        force_custom_dat_alignment: s.forceCustomDatAlignment,
        custom_dat_alignment: sanitizeCustomDatAlignment(s.customDatAlignment),
        core_dat_b1: s.coreDatB1,
        core_dat_b2: s.coreDatB2,
        core_dat_b3: s.coreDatB3,
        wmb_hierarchy_ratio: s.wmbHierarchyRatio,
        wmb_wireframe: s.wmbWireframe,
        wmb_normals_view: s.wmbNormalsView,
        wmb_skeleton_overlay: s.wmbSkeletonOverlay,
        wmb_grid_visible: s.wmbGridVisible,
        default_import_format: s.defaultImportFormat,
        default_encode_quality: s.defaultEncodeQuality,
        sync_xt1_flags_on_import: s.syncXt1FlagsOnImport,
        asset_browser_zoom: s.assetBrowserZoom,
        uvd_padding: s.uvdPadding,
        uvd_max_size: s.uvdMaxSize,
        ...overrides,
      };
    },
    []
  );

  return {
    settingsLoaded,
    theme,
    leftWidth,
    rightWidth,
    assetRatio,
    oodleDllPath,
    nvttDllPath,
    oodleConfigured,
    nvttConfigured,
    pkzDefaultCompression,
    pkzDefaultLevel,
    forceCustomDatAlignment,
    customDatAlignment,
    coreDatB1,
    coreDatB2,
    coreDatB3,
    wmbHierarchyRatio,
    wmbWireframe,
    wmbNormalsView,
    wmbSkeletonOverlay,
    wmbGridVisible,
    recentFiles,
    hideUnsupported,
    fileSortCol,
    fileSortAsc,
    defaultImportFormat,
    defaultEncodeQuality,
    syncXt1FlagsOnImport,
    assetBrowserZoom,
    uvdPadding,
    uvdMaxSize,
    setTheme,
    setLeftWidth,
    setRightWidth,
    setAssetRatio,
    setOodleDllPath,
    setNvttDllPath,
    setOodleConfigured,
    setNvttConfigured,
    setPkzDefaultCompression,
    setPkzDefaultLevel,
    setForceCustomDatAlignment,
    setCustomDatAlignment,
    setCoreDatB1,
    setCoreDatB2,
    setCoreDatB3,
    setWmbHierarchyRatio,
    setWmbWireframe,
    setWmbNormalsView,
    setWmbSkeletonOverlay,
    setWmbGridVisible,
    setRecentFiles,
    setHideUnsupported,
    setFileSortCol,
    setFileSortAsc,
    setDefaultImportFormat,
    setDefaultEncodeQuality,
    setSyncXt1FlagsOnImport,
    setAssetBrowserZoom,
    setUvdPadding,
    setUvdMaxSize,
    buildStudioSettingsPayload,
    applyLoadedSettings,
    markLoaded,
  };
}
