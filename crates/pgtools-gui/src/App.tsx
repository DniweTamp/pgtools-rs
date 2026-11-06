import React, { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import {
  AssetTreeNode,
  BxmXmlNode,
  ContextMenuState,
  DecodedWmbMaterial,
  DocumentOutline,
  FileSortCol,
  FsEntry,
  MaterialClipboardState,
  PointerDragState,
  StudioSettings,
  TexModalState,
  TextureNodeView,
  ThemeMode,
  Wmb3Material,
  WmbSectionView,
} from "./types";
import {
  compileGlobFilter,
  formatBytes,
  isVirtualSubNode,
  parseReorderableNode,
} from "./utils/format";
import {
  bxmTreeToXmlDocument,
  detectBxmTabular,
  getBxmNodeByPath,
  parseXmlToBxmNode,
} from "./bxm/xml";
import { BxmCenterView, BxmInspectorView } from "./bxm/BxmEditor";
import {
  TextureCenterView,
  TextureImportModal,
  TextureInspectorView,
} from "./editors/TextureEditor";
import { ClothInspectorView, StructCenterView } from "./editors/StructEditors";
import {
  WmbInspectorView,
  WmbStructuredCenterView,
} from "./editors/WmbStructuredEditor";
import { createMenuBuilders } from "./menus/contextMenus";
import {
  IconCheck,
  IconChevronDown,
  IconCode,
  IconCube,
  IconFolder,
  IconImage,
  IconRedo,
  IconRefresh,
  IconSunMoon,
  IconUndo,
} from "./components/Icons";
import { Workspace } from "./wmb-studio/Workspace";
import {
  convertMaterialBetweenGames,
  createDemoModel,
  LegacyActionKind,
} from "./wmb-studio/contract";

export default function App() {
  const [settingsLoaded, setSettingsLoaded] = useState<boolean>(false);
  const [theme, setTheme] = useState<ThemeMode>("dark");

  // Resizable pane dimensions (persisted in StudioSettings)
  const [leftWidth, setLeftWidth] = useState<number>(310);
  const [rightWidth, setRightWidth] = useState<number>(380);
  const [assetRatio, setAssetRatio] = useState<number>(0.5);
  const [draggingSplit, setDraggingSplit] = useState<"left" | "right" | "left-row" | null>(null);
  const leftColRef = useRef<HTMLDivElement | null>(null);
  const assetBrowserCardRef = useRef<HTMLDivElement | null>(null);

  // External DLL paths (persisted in StudioSettings)
  const [oodleDllPath, setOodleDllPath] = useState<string>("");
  const [nvttDllPath, setNvttDllPath] = useState<string>("");
  const [coreDatB1, setCoreDatB1] = useState<string>("");
  const [coreDatB2, setCoreDatB2] = useState<string>("");
  const [coreDatB3, setCoreDatB3] = useState<string>("");
  const [wmbHierarchyRatio, setWmbHierarchyRatio] = useState<number>(0.48);
  const [wmbWireframe, setWmbWireframe] = useState<boolean>(false);
  const [wmbNormalsView, setWmbNormalsView] = useState<boolean>(false);
  const [wmbSkeletonOverlay, setWmbSkeletonOverlay] = useState<boolean>(false);
  const [wmbGridVisible, setWmbGridVisible] = useState<boolean>(true);
  const [wmbDocGeneration, setWmbDocGeneration] = useState<number>(0);
  const [coreDatModalOpen, setCoreDatModalOpen] = useState<boolean>(false);

  // Bottom-Left File Browser state
  const [currentDir, setCurrentDir] = useState<string>("");
  const currentDirRef = useRef<string>("");
  currentDirRef.current = currentDir;
  const inspectSeqRef = useRef<number>(0);
  const [dirInput, setDirInput] = useState<string>("");
  const [fsEntries, setFsEntries] = useState<FsEntry[]>([]);
  const [fileGlobFilter, setFileGlobFilter] = useState<string>("");
  const [hideUnsupported, setHideUnsupported] = useState<boolean>(true);
  const [fileSortCol, setFileSortCol] = useState<FileSortCol>("name");
  const [fileSortAsc, setFileSortAsc] = useState<boolean>(true);
  const [selectedFilePath, setSelectedFilePath] = useState<string | null>(null);
  const [selectedFilePaths, setSelectedFilePaths] = useState<string[]>([]);
  const [fsClipboard, setFsClipboard] = useState<{ mode: "copy" | "cut"; paths: string[] } | null>(
    null
  );
  const [recentFiles, setRecentFiles] = useState<string[]>([]);

  // Texture Replacement & Compression Modal Dialog state
  const [texModalState, setTexModalState] = useState<{
    nodeId: string;
    action: "replace_file" | "add_texture";
    path: string;
    fileName: string;
    encodeFormat: string;
    generateMips: boolean;
    solidify: boolean;
    makeOpaque: boolean;
  } | null>(null);

  // PKZ Bulk Compression & WMB Mesh->Batch->LOD Tree state
  const [pkzBulkCodec, setPkzBulkCodec] = useState<string>("ZStandard");
  const [expandedMeshGroups, setExpandedMeshGroups] = useState<Record<string, boolean>>({});
  const [expandedBatchLods, setExpandedBatchLods] = useState<Record<string, boolean>>({});
  const [draggedBatchInfo, setDraggedBatchInfo] = useState<{
    isWmb3: boolean;
    batchIdx: number;
    fromMeshIdx: number;
  } | null>(null);
  const [dragOverMeshIdx, setDragOverMeshIdx] = useState<number | null>(null);

  // Top-Left Asset Browser & Document state
  const [outline, setOutline] = useState<DocumentOutline | null>(null);
  const outlineRef = useRef<DocumentOutline | null>(null);
  outlineRef.current = outline;

  function confirmDiscardIfDirty(actionLabel = "open another document"): boolean {
    if (!outlineRef.current?.dirty) return true;
    return window.confirm(
      `You have unsaved changes in "${outlineRef.current.file_name}".\n\nDiscard unsaved changes and ${actionLabel}?`
    );
  }

  useEffect(() => {
    const onBeforeUnload = (e: BeforeUnloadEvent) => {
      if (outlineRef.current?.dirty) {
        e.preventDefault();
        e.returnValue = "";
      }
    };
    window.addEventListener("beforeunload", onBeforeUnload);
    return () => window.removeEventListener("beforeunload", onBeforeUnload);
  }, []);
  const [expandedNodes, setExpandedNodes] = useState<Record<string, boolean>>({
    root: true,
    "root:dat": true,
    "root:dtt": true,
  });
  const [selectedNodeId, setSelectedNodeId] = useState<string>("root");
  const [assetSearch, setAssetSearch] = useState<string>("");

  // Pointer-based Drag-and-Drop (reordering + drag-out to Explorer) & External File Drop
  const [pointerDrag, setPointerDrag] = useState<PointerDragState | null>(null);
  const [externalDragHover, setExternalDragHover] = useState<boolean>(false);

  // Active Node View & Toolbar Controls
  const [nodePayload, setNodePayload] = useState<any>(null);
  const [mipIndex, setMipIndex] = useState<number>(0);
  const [channelMode, setChannelMode] = useState<string>("RGBA");
  const [opaquePreview, setOpaquePreview] = useState<boolean>(false);

  // Custom Texture Encoding Options (XT1 Studio parity)
  const [texEncodeFormat, setTexEncodeFormat] = useState<string>("Auto");
  const [texGenerateMips, setTexGenerateMips] = useState<boolean>(true);
  const [texSolidify, setTexSolidify] = useState<boolean>(true);
  const [texMakeOpaque, setTexMakeOpaque] = useState<boolean>(false);

  // 2D Viewport Pan & Zoom
  const [zoom, setZoom] = useState<number>(1.0);
  const [pan, setPan] = useState<{ x: number; y: number }>({ x: 0, y: 0 });
  const [isPanning, setIsPanning] = useState<boolean>(false);
  const panStartRef = useRef<{ x: number; y: number }>({ x: 0, y: 0 });
  const [centerTab, setCenterTab] = useState<string>("primary");

  // Inspector collapsible cards state
  const [openCards, setOpenCards] = useState<Record<string, boolean>>({
    hash: true,
    flags: true,
    encoding: true,
    advanced: false,
    wmb_props: true,
    wmb_params: true,
    bxm_node: true,
    cloth_header: true,
    dll_config: true,
  });

  // Editable Inspector & Center Structure Drafts
  const [hashDraft, setHashDraft] = useState<string>("");
  const [flagsDraft, setFlagsDraft] = useState<string>("");
  const [wmbMatDraft, setWmbMatDraft] = useState<DecodedWmbMaterial | null>(null);
  const [wmb3MatDraft, setWmb3MatDraft] = useState<Wmb3Material | null>(null);
  const [wmbRawMode, setWmbRawMode] = useState<boolean>(false);
  const [wmbB2Override, setWmbB2Override] = useState<boolean | null>(null);
  const [materialClipboard, setMaterialClipboard] = useState<{
    game: "b1" | "b2" | "b3";
    payload: any;
  } | null>(null);

  const [wmbHeaderFlagsDraft, setWmbHeaderFlagsDraft] = useState<string>("");
  const [wmbBboxDraft, setWmbBboxDraft] = useState<number[]>([0, 0, 0, 0, 0, 0]);

  // BXM Interactive Tree + Table + Raw XML state
  const [bxmTreeDraft, setBxmTreeDraft] = useState<BxmXmlNode | null>(null);
  const [bxmXmlDraft, setBxmXmlDraft] = useState<string>("");
  const [selectedBxmPath, setSelectedBxmPath] = useState<string>("0");
  const [bxmViewMode, setBxmViewMode] = useState<"tree" | "table" | "xml">("tree");
  const [bxmExpandedPaths, setBxmExpandedPaths] = useState<Record<string, boolean>>({ "0": true });

  const [estDraft, setEstDraft] = useState<any>(null);
  const [csvHeadersDraft, setCsvHeadersDraft] = useState<string[]>([]);
  const [csvRowsDraft, setCsvRowsDraft] = useState<string[][]>([]);
  const [structDraft, setStructDraft] = useState<any>(null);
  const [selectedMotTrackIdx, setSelectedMotTrackIdx] = useState<number>(0);

  // Menus & Status
  const [activeTopMenu, setActiveTopMenu] = useState<"file" | "tools" | "help" | null>(null);
  const [contextMenu, setContextMenu] = useState<ContextMenuState | null>(null);
  const [hoverSubmenuIdx, setHoverSubmenuIdx] = useState<number | null>(null);
  const [statusLeft, setStatusLeft] = useState<string>("Ready");

  // 1. Suppress generic browser right-click context menu globally
  useEffect(() => {
    const suppressNativeContextMenu = (e: MouseEvent) => {
      e.preventDefault();
    };
    window.addEventListener("contextmenu", suppressNativeContextMenu);
    return () => window.removeEventListener("contextmenu", suppressNativeContextMenu);
  }, []);

  // Helper to measure and auto-fit a single column or all columns of a table
  const autoFitColumn = (table: HTMLTableElement, th: HTMLTableCellElement, colIdx: number) => {
    const thInp = th.querySelector("input") as HTMLInputElement | null;
    const headerText = thInp ? thInp.value || "" : (th.textContent || "").trim();
    let maxW = Math.max(34, headerText.length * 7.2 + 16);
    const rows = table.querySelectorAll("tbody tr");
    const limit = Math.min(rows.length, 250);
    for (let i = 0; i < limit; i++) {
      const cell = rows[i].children[colIdx] as HTMLElement | undefined;
      if (!cell) continue;
      const inputs = cell.querySelectorAll("input, select");
      if (inputs.length > 1) {
        let multiW = 12;
        inputs.forEach((el) => {
          const inpEl = el as HTMLInputElement;
          if (inpEl.type === "color" || inpEl.type === "checkbox") {
            multiW += 30;
          } else if (inpEl.type === "range") {
            multiW += 80;
          } else {
            const vLen = (inpEl.value || "").length;
            multiW += Math.max(46, vLen * 7.2 + 16);
          }
        });
        if (multiW > maxW) maxW = Math.min(440, multiW);
      } else if (inputs.length === 1) {
        const inp = inputs[0] as HTMLInputElement;
        const valStr = inp.value || inp.placeholder || "";
        const w = Math.max(36, valStr.length * 7.3 + 20);
        if (w > maxW) maxW = Math.min(380, w);
      } else {
        const txt = (cell.textContent || "").trim();
        const w = Math.max(34, txt.length * 7.0 + 16);
        if (w > maxW) maxW = Math.min(380, w);
      }
    }
    const finalW = Math.round(maxW);
    th.style.width = `${finalW}px`;
    th.style.minWidth = `${finalW}px`;
    th.style.maxWidth = "";
  };

  // Auto-fit all visible struct-tables whenever a file/section/table view is first opened or switched
  useEffect(() => {
    const timer = window.requestAnimationFrame(() => {
      const tables = document.querySelectorAll("table.struct-table");
      tables.forEach((tbl) => {
        const tableEl = tbl as HTMLTableElement;
        const ths = tableEl.querySelectorAll("thead tr th");
        ths.forEach((thEl, idx) => {
          autoFitColumn(tableEl, thEl as HTMLTableCellElement, idx);
        });
      });
    });
    return () => window.cancelAnimationFrame(timer);
  }, [
    nodePayload?.node_id,
    nodePayload?.view_type,
    nodePayload?.wmb?.section,
    bxmViewMode,
    selectedBxmPath,
    centerTab,
  ]);

  // 1B. Universal Resizable Table Columns (drag right 8px edge of any <th> or double-click to auto-fit)
  useEffect(() => {
    let activeTh: HTMLTableCellElement | null = null;
    let startX = 0;
    let startWidth = 0;

    const onMouseMove = (e: MouseEvent) => {
      if (activeTh) {
        const nextW = Math.max(28, startWidth + (e.clientX - startX));
        activeTh.style.width = `${nextW}px`;
        activeTh.style.minWidth = `${nextW}px`;
        activeTh.style.maxWidth = `${nextW}px`;
        document.body.style.cursor = "col-resize";
        return;
      }
      const target = e.target as HTMLElement | null;
      const th = target?.closest("table.struct-table th, table.file-table th") as HTMLTableCellElement | null;
      if (th) {
        const r = th.getBoundingClientRect();
        if (r.right - e.clientX <= 8) {
          th.style.cursor = "col-resize";
        } else {
          th.style.cursor = "";
        }
      }
    };

    const onMouseDown = (e: MouseEvent) => {
      if (e.button !== 0) return;
      const target = e.target as HTMLElement | null;
      const th = target?.closest("table.struct-table th, table.file-table th") as HTMLTableCellElement | null;
      if (!th) return;
      const r = th.getBoundingClientRect();
      if (r.right - e.clientX <= 8) {
        e.preventDefault();
        e.stopPropagation();
        activeTh = th;
        startX = e.clientX;
        startWidth = r.width;
      }
    };

    const onDoubleClick = (e: MouseEvent) => {
      const target = e.target as HTMLElement | null;
      const th = target?.closest("table.struct-table th, table.file-table th") as HTMLTableCellElement | null;
      if (!th) return;
      const r = th.getBoundingClientRect();
      if (r.right - e.clientX <= 10) {
        e.preventDefault();
        e.stopPropagation();
        const tr = th.parentElement;
        const table = th.closest("table") as HTMLTableElement | null;
        if (!tr || !table) return;
        const colIdx = Array.from(tr.children).indexOf(th);
        autoFitColumn(table, th, colIdx);
      }
    };

    const onMouseUp = () => {
      if (activeTh) {
        activeTh = null;
        document.body.style.cursor = "";
      }
    };

    window.addEventListener("mousemove", onMouseMove, true);
    window.addEventListener("mousedown", onMouseDown, true);
    window.addEventListener("dblclick", onDoubleClick, true);
    window.addEventListener("mouseup", onMouseUp, true);
    return () => {
      window.removeEventListener("mousemove", onMouseMove, true);
      window.removeEventListener("mousedown", onMouseDown, true);
      window.removeEventListener("dblclick", onDoubleClick, true);
      window.removeEventListener("mouseup", onMouseUp, true);
    };
  }, []);

  // 2. Listen for external OS file drops (`tauri://drag-drop`) anywhere on the window
  const openDocRef = useRef<(p: string) => Promise<void>>(async () => {});
  useEffect(() => {
    let unlistenDrop: (() => void) | undefined;
    let unlistenEnter: (() => void) | undefined;
    let unlistenLeave: (() => void) | undefined;

    (async () => {
      try {
        unlistenDrop = await listen<{ paths?: string[] }>("tauri://drag-drop", (event) => {
          setExternalDragHover(false);
          const paths = event.payload?.paths;
          if (Array.isArray(paths) && paths.length > 0 && paths[0]) {
            openDocRef.current(paths[0]);
          }
        });
        unlistenEnter = await listen("tauri://drag-enter", () => {
          setExternalDragHover(true);
        });
        unlistenLeave = await listen("tauri://drag-leave", () => {
          setExternalDragHover(false);
        });
      } catch {
        // Fallback handled via DOM onDrop
      }
    })();

    return () => {
      if (unlistenDrop) unlistenDrop();
      if (unlistenEnter) unlistenEnter();
      if (unlistenLeave) unlistenLeave();
    };
  }, []);

  // Load persisted settings on mount
  useEffect(() => {
    (async () => {
      try {
        const s = await invoke<StudioSettings>("get_studio_settings");
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
        setCoreDatB1(s.core_dat_b1 ?? "");
        setCoreDatB2(s.core_dat_b2 ?? "");
        setCoreDatB3(s.core_dat_b3 ?? "");
        if (typeof s.wmb_hierarchy_ratio === "number" && s.wmb_hierarchy_ratio >= 0.15 && s.wmb_hierarchy_ratio <= 0.85) {
          setWmbHierarchyRatio(s.wmb_hierarchy_ratio);
        }
        if (typeof s.wmb_wireframe === "boolean") setWmbWireframe(s.wmb_wireframe);
        if (typeof s.wmb_normals_view === "boolean") setWmbNormalsView(s.wmb_normals_view);
        if (typeof s.wmb_skeleton_overlay === "boolean") setWmbSkeletonOverlay(s.wmb_skeleton_overlay);
        if (typeof s.wmb_grid_visible === "boolean") setWmbGridVisible(s.wmb_grid_visible);
        setSettingsLoaded(true);
        await loadDirectory(s.last_dir || undefined);
      } catch {
        setSettingsLoaded(true);
        await loadDirectory();
      }
    })();
  }, []);

  // Sync theme attribute on document root
  useEffect(() => {
    document.documentElement.setAttribute("data-theme", theme);
  }, [theme]);

  const buildStudioSettingsPayload = useCallback(
    (overrides?: Partial<StudioSettings>): StudioSettings => ({
      theme: theme === "light" ? "Light" : "Dark",
      recent_files: recentFiles,
      last_dir: currentDir,
      hide_unsupported: hideUnsupported,
      file_sort_col: fileSortCol,
      file_sort_asc: fileSortAsc,
      left_width: Math.round(leftWidth),
      right_width: Math.round(rightWidth),
      asset_ratio: assetRatio,
      oodle_dll_path: oodleDllPath,
      nvtt_dll_path: nvttDllPath,
      core_dat_b1: coreDatB1,
      core_dat_b2: coreDatB2,
      core_dat_b3: coreDatB3,
      wmb_hierarchy_ratio: wmbHierarchyRatio,
      wmb_wireframe: wmbWireframe,
      wmb_normals_view: wmbNormalsView,
      wmb_skeleton_overlay: wmbSkeletonOverlay,
      wmb_grid_visible: wmbGridVisible,
      ...overrides,
    }),
    [
      theme,
      recentFiles,
      currentDir,
      hideUnsupported,
      fileSortCol,
      fileSortAsc,
      leftWidth,
      rightWidth,
      assetRatio,
      oodleDllPath,
      nvttDllPath,
      coreDatB1,
      coreDatB2,
      coreDatB3,
      wmbHierarchyRatio,
      wmbWireframe,
      wmbNormalsView,
      wmbSkeletonOverlay,
      wmbGridVisible,
    ]
  );

  // Persist StudioSettings whenever preferences change (debounced to avoid IPC spam during splitter drag — B-15)
  useEffect(() => {
    if (!settingsLoaded) return;
    const payload = buildStudioSettingsPayload();
    const timer = window.setTimeout(() => {
      invoke("save_studio_settings", { settings: payload }).catch(() => {});
    }, 250);
    return () => window.clearTimeout(timer);
  }, [
    buildStudioSettingsPayload,
    settingsLoaded,
    theme,
    recentFiles,
    currentDir,
    hideUnsupported,
    fileSortCol,
    fileSortAsc,
    leftWidth,
    rightWidth,
    assetRatio,
    oodleDllPath,
    nvttDllPath,
    coreDatB1,
    coreDatB2,
    coreDatB3,
    wmbHierarchyRatio,
    wmbWireframe,
    wmbNormalsView,
    wmbSkeletonOverlay,
    wmbGridVisible,
  ]);

  const shortcutHandlersRef = useRef<{
    openFile: () => void;
    save: () => void;
    saveAs: () => void;
    undo: () => void;
    redo: () => void;
  }>({
    openFile: () => {},
    save: () => {},
    saveAs: () => {},
    undo: () => {},
    redo: () => {},
  });

  // Global keyboard shortcuts (Ctrl+O, Ctrl+S, Ctrl+Z, Ctrl+Y) with editable input guard (B-12)
  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      const target = e.target as HTMLElement | null;
      const inEditableField = Boolean(
        target &&
          (target.tagName === "INPUT" ||
            target.tagName === "TEXTAREA" ||
            target.tagName === "SELECT" ||
            target.isContentEditable)
      );
      if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "o") {
        e.preventDefault();
        shortcutHandlersRef.current.openFile();
      } else if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "s") {
        e.preventDefault();
        if (e.shiftKey) {
          shortcutHandlersRef.current.saveAs();
        } else {
          shortcutHandlersRef.current.save();
        }
      } else if (!inEditableField && (e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "z") {
        if (e.shiftKey) {
          e.preventDefault();
          shortcutHandlersRef.current.redo();
        } else {
          e.preventDefault();
          shortcutHandlersRef.current.undo();
        }
      } else if (!inEditableField && (e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "y") {
        e.preventDefault();
        shortcutHandlersRef.current.redo();
      } else if (e.key === "Escape") {
        setContextMenu(null);
        setActiveTopMenu(null);
        setPointerDrag(null);
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, []);

  // Splitter drag handler
  useEffect(() => {
    if (!draggingSplit) return;
    const onMove = (e: MouseEvent) => {
      if (draggingSplit === "left") {
        setLeftWidth(Math.max(220, Math.min(620, e.clientX - 10)));
      } else if (draggingSplit === "right") {
        setRightWidth(Math.max(290, Math.min(680, window.innerWidth - e.clientX - 10)));
      } else if (draggingSplit === "left-row" && leftColRef.current) {
        const rect = leftColRef.current.getBoundingClientRect();
        const rel = (e.clientY - rect.top) / Math.max(1, rect.height);
        setAssetRatio(Math.max(0.18, Math.min(0.82, rel)));
      }
    };
    const onUp = () => setDraggingSplit(null);
    window.addEventListener("mousemove", onMove);
    window.addEventListener("mouseup", onUp);
    return () => {
      window.removeEventListener("mousemove", onMove);
      window.removeEventListener("mouseup", onUp);
    };
  }, [draggingSplit]);

  useEffect(() => {
    if (!pointerDrag) return;
    const onPointerMove = (e: PointerEvent) => {
      const dx = e.clientX - pointerDrag.startX;
      const dy = e.clientY - pointerDrag.startY;
      const dist = Math.hypot(dx, dy);
      const isNowActive = pointerDrag.active || dist >= 6;
      if (!isNowActive) return;

      // Check if dragging an Asset Browser file node outside the Asset Browser card to Windows Explorer / Desktop
      if (
        pointerDrag.kind === "asset" &&
        !pointerDrag.dragOutTriggered &&
        assetBrowserCardRef.current &&
        !isVirtualSubNode(pointerDrag.sourceId) &&
        pointerDrag.sourceId !== "root"
      ) {
        const rect = assetBrowserCardRef.current.getBoundingClientRect();
        const outsideCard =
          e.clientX < rect.left - 28 ||
          e.clientX > rect.right + 28 ||
          e.clientY < rect.top - 28 ||
          e.clientY > rect.bottom + 28;
        if (outsideCard) {
          setPointerDrag(null);
          setStatusLeft(`Dragging ${pointerDrag.label} out to Windows Explorer...`);
          invoke<string>("drag_out_asset_node", { nodeId: pointerDrag.sourceId })
            .then((msg) => setStatusLeft(msg))
            .catch((err) => setStatusLeft(`Drag-out note: ${String(err)}`));
          return;
        }
      }

      // Hit-test row under cursor for sibling reordering
      const el = document.elementFromPoint(e.clientX, e.clientY) as HTMLElement | null;
      const rowEl = el?.closest(
        pointerDrag.kind === "asset" ? "[data-asset-node-id]" : "[data-bxm-path]"
      ) as HTMLElement | null;

      let overId: string | null = null;
      let insertPos: "before" | "after" | null = null;

      if (rowEl) {
        const candidateId =
          pointerDrag.kind === "asset"
            ? rowEl.getAttribute("data-asset-node-id")
            : rowEl.getAttribute("data-bxm-path");

        if (candidateId && candidateId !== pointerDrag.sourceId) {
          if (pointerDrag.kind === "asset") {
            const srcInfo = parseReorderableNode(pointerDrag.sourceId);
            const dstInfo = parseReorderableNode(candidateId);
            if (srcInfo && dstInfo && srcInfo[0] === dstInfo[0]) {
              overId = candidateId;
              const r = rowEl.getBoundingClientRect();
              insertPos = e.clientY < r.top + r.height / 2 ? "before" : "after";
            } else if (
              pointerDrag.sourceId.includes("pkz:") &&
              candidateId.includes("pkzfolder:")
            ) {
              overId = candidateId;
              insertPos = "after";
            }
          } else if (pointerDrag.kind === "bxm") {
            const srcParent = pointerDrag.sourceId.slice(0, pointerDrag.sourceId.lastIndexOf("."));
            const dstParent = candidateId.slice(0, candidateId.lastIndexOf("."));
            if (srcParent && srcParent === dstParent) {
              overId = candidateId;
              const r = rowEl.getBoundingClientRect();
              insertPos = e.clientY < r.top + r.height / 2 ? "before" : "after";
            }
          }
        }
      }

      setPointerDrag((prev) =>
        prev
          ? {
              ...prev,
              active: true,
              curX: e.clientX,
              curY: e.clientY,
              overId,
              insertPos,
            }
          : null
      );
    };

    const onPointerUp = () => {
      if (pointerDrag.active && pointerDrag.overId && pointerDrag.insertPos) {
        if (pointerDrag.kind === "asset") {
          if (
            pointerDrag.sourceId.includes("pkz:") &&
            pointerDrag.overId.includes("pkzfolder:")
          ) {
            const folderPrefix = pointerDrag.overId.split("pkzfolder:").pop() || "";
            runContainerMutation(pointerDrag.sourceId, "move_pkz_entry_to_folder", folderPrefix, {
              target_folder: folderPrefix,
            });
            setPointerDrag(null);
            return;
          }
          const srcInfo = parseReorderableNode(pointerDrag.sourceId);
          const dstInfo = parseReorderableNode(pointerDrag.overId);
          if (srcInfo && dstInfo && srcInfo[0] === dstInfo[0]) {
            const fromIdx = srcInfo[1];
            let toIdx = dstInfo[1];
            if (pointerDrag.insertPos === "after" && fromIdx < toIdx) {
              // removing fromIdx shifts subsequent items left by 1
            } else if (pointerDrag.insertPos === "before" && fromIdx > toIdx) {
              // target index stays toIdx
            } else if (pointerDrag.insertPos === "after" && fromIdx > toIdx) {
              toIdx += 1;
            } else if (pointerDrag.insertPos === "before" && fromIdx < toIdx) {
              toIdx = Math.max(0, toIdx - 1);
            }
            if (fromIdx !== toIdx) {
              runContainerMutation(pointerDrag.sourceId, "reorder_entry", "", {
                to_index: toIdx,
              });
            }
          }
        } else if (pointerDrag.kind === "bxm" && bxmTreeDraft) {
          const parentPath = pointerDrag.sourceId.slice(0, pointerDrag.sourceId.lastIndexOf("."));
          const fromIdx = parseInt(pointerDrag.sourceId.split(".").pop() || "0", 10);
          let toIdx = parseInt(pointerDrag.overId.split(".").pop() || "0", 10);
          if (pointerDrag.insertPos === "after" && fromIdx > toIdx) toIdx += 1;
          if (pointerDrag.insertPos === "before" && fromIdx < toIdx) toIdx = Math.max(0, toIdx - 1);
          if (fromIdx !== toIdx) {
            const nextTree = structuredClone(bxmTreeDraft);
            const parentNode = getBxmNodeByPath(nextTree, parentPath);
            if (parentNode && Array.isArray(parentNode.children)) {
              const [moved] = parentNode.children.splice(fromIdx, 1);
              parentNode.children.splice(
                Math.min(parentNode.children.length, Math.max(0, toIdx)),
                0,
                moved
              );
              updateBxmDraftTree(nextTree);
              setSelectedBxmPath(`${parentPath}.${toIdx}`);
            }
          }
        }
      }
      setPointerDrag(null);
    };

    window.addEventListener("pointermove", onPointerMove);
    window.addEventListener("pointerup", onPointerUp);
    return () => {
      window.removeEventListener("pointermove", onPointerMove);
      window.removeEventListener("pointerup", onPointerUp);
    };
  }, [pointerDrag, bxmTreeDraft]);

  async function refreshRecents() {
    try {
      const rec = await invoke<string[]>("get_recent_files");
      setRecentFiles(rec);
    } catch {
      // ignore
    }
  }

  async function loadDirectory(target?: string) {
    try {
      const [resolved, list] = await invoke<[string, FsEntry[]]>("list_directory", {
        dirPath: target ?? null,
      });
      setCurrentDir(resolved);
      setDirInput(resolved);
      setFsEntries(list);
    } catch (err) {
      setStatusLeft(`Directory error: ${String(err)}`);
    }
  }

  async function openDocumentByPath(path: string, skipDirtyPrompt = false) {
    if (!skipDirtyPrompt && !confirmDiscardIfDirty(`open "${path.split(/[\\/]/).pop() || path}"`)) {
      return;
    }
    try {
      setStatusLeft(`Loading ${path}...`);
      const doc = await invoke<DocumentOutline>("open_document", { path });
      setOutline(doc);
      setSelectedFilePath(path);
      setWmbB2Override(null);
      refreshRecents();

      const nextExpanded: Record<string, boolean> = {
        root: true,
        "root:dat": true,
        "root:dtt": true,
      };
      let defaultTargetId = doc.tree[0]?.id ?? "root";
      for (const rootNode of doc.tree) {
        nextExpanded[rootNode.id] = true;
        if (rootNode.format_badge === "WMB" || rootNode.format_badge === "WMB3") {
          defaultTargetId = rootNode.id;
          break;
        }
        const wmbChild = rootNode.children.find(
          (c) => (c.format_badge === "WMB" || c.format_badge === "WMB3") && !isVirtualSubNode(c.id)
        );
        if (wmbChild) {
          defaultTargetId = wmbChild.id;
          break;
        }
        if (rootNode.children.length > 0) {
          const firstChild = rootNode.children[0];
          nextExpanded[firstChild.id] = true;
          if (firstChild.children.length > 0) {
            defaultTargetId = firstChild.children[0].id;
            break;
          } else {
            defaultTargetId = firstChild.id;
            break;
          }
        }
      }
      setWmbDocGeneration((g) => g + 1);
      setExpandedNodes(nextExpanded);
      setMipIndex(0);
      await selectAndInspectNode(defaultTargetId, 0, channelMode, opaquePreview, null, doc.file_name);
      setStatusLeft(`Loaded ${doc.file_name}`);
    } catch (err) {
      setStatusLeft(`Failed to open: ${String(err)}`);
    }
  }
  openDocRef.current = openDocumentByPath;

  async function selectAndInspectNode(
    nodeId: string,
    mip = mipIndex,
    channel = channelMode,
    opaque = opaquePreview,
    b2Override: boolean | null = wmbB2Override,
    docLabel?: string
  ) {
    const reqSeq = ++inspectSeqRef.current;
    setSelectedNodeId(nodeId);
    try {
      const res = await invoke<any>("inspect_asset_node", {
        nodeId,
        mipIndex: mip,
        channel,
        opaque,
        b2Override,
      });
      if (reqSeq !== inspectSeqRef.current) return;
      setNodePayload(res);
      if (res.view_type === "TEXTURE" && res.texture) {
        const t: TextureNodeView = res.texture;
        setHashDraft(t.texture_id_hex);
        setFlagsDraft(t.flags_hex);
      } else if (res.view_type === "WMB" && res.wmb) {
        const w: WmbSectionView = res.wmb;
        if (w.material_decoded) {
          const norm: DecodedWmbMaterial = {
            ...w.material_decoded,
            fields: Array.isArray(w.material_decoded.fields) ? w.material_decoded.fields : [],
            raw_dwords: Array.isArray(w.material_decoded.raw_dwords)
              ? w.material_decoded.raw_dwords
              : [],
          };
          setWmbMatDraft(norm);
        } else {
          setWmbMatDraft(null);
        }
        if (w.wmb3_material) {
          setWmb3MatDraft(structuredClone(w.wmb3_material));
        } else {
          setWmb3MatDraft(null);
        }
        if (w.section === "header" && w.data) {
          const vf = w.data.vertex_flags;
          if (typeof vf === "number") {
            setWmbHeaderFlagsDraft(`0x${(vf >>> 0).toString(16).toUpperCase().padStart(8, "0")}`);
          }
          if (Array.isArray(w.data.bounding_box) && w.data.bounding_box.length === 6) {
            setWmbBboxDraft([...w.data.bounding_box]);
          }
        }
      } else if (res.view_type === "BXM") {
        const clonedRoot: BxmXmlNode | null = res.root ? structuredClone(res.root) : null;
        setBxmTreeDraft(clonedRoot);
        setBxmXmlDraft(res.xml ?? bxmTreeToXmlDocument(clonedRoot));
        setSelectedBxmPath("0");
        setBxmExpandedPaths({ "0": true, "0.0": true, "0.1": true });
        if (detectBxmTabular(clonedRoot)) {
          setBxmViewMode("table");
        } else {
          setBxmViewMode("tree");
        }
      } else if (res.view_type === "EST" && res.est) {
        setEstDraft(structuredClone(res.est));
      } else if (res.view_type === "CSV") {
        setCsvHeadersDraft(Array.isArray(res.headers) ? [...res.headers] : []);
        setCsvRowsDraft(Array.isArray(res.rows) ? structuredClone(res.rows) : []);
      } else if (res.view_type === "RAW_STRUCT") {
        setStructDraft(structuredClone(res.data));
        setSelectedMotTrackIdx(0);
      }
      if (docLabel) {
        setStatusLeft(`Loaded ${docLabel}`);
      }
    } catch (err) {
      if (reqSeq === inspectSeqRef.current) {
        setStatusLeft(`Inspect error: ${String(err)}`);
      }
    }
  }

  function updateBxmDraftTree(nextRoot: BxmXmlNode) {
    setBxmTreeDraft(nextRoot);
    setBxmXmlDraft(bxmTreeToXmlDocument(nextRoot));
  }

  function switchBxmViewMode(nextMode: "tree" | "table" | "xml") {
    if (bxmViewMode === "xml" && nextMode !== "xml") {
      const parsed = parseXmlToBxmNode(bxmXmlDraft);
      if (!parsed) {
        setStatusLeft("XML parse error: fix malformed XML syntax before switching to Tree/Table view");
        return;
      }
      setBxmTreeDraft(parsed);
    } else if (bxmViewMode !== "xml" && nextMode === "xml" && bxmTreeDraft) {
      setBxmXmlDraft(bxmTreeToXmlDocument(bxmTreeDraft));
    }
    setBxmViewMode(nextMode);
  }

  async function commitBxmDraft(customXml?: string) {
    if (!nodePayload || nodePayload.view_type !== "BXM") return;
    try {
      const xmlToSave =
        customXml ??
        (bxmViewMode === "xml" || !bxmTreeDraft
          ? bxmXmlDraft
          : bxmTreeToXmlDocument(bxmTreeDraft));
      const updated = await invoke<DocumentOutline>("mutate_bxm_node", {
        nodeId: nodePayload.node_id,
        xmlContent: xmlToSave,
        tag: nodePayload.tag || "BXM",
      });
      setOutline(updated);
      await selectAndInspectNode(nodePayload.node_id);
      setStatusLeft(`Committed ${nodePayload.title} changes to binary ${nodePayload.tag || "BXM"}`);
    } catch (err) {
      setStatusLeft(`BXM encode error: ${String(err)}`);
    }
  }

  async function handleTexturePreviewParamChange(newMip: number, newChannel: string, newOpaque: boolean) {
    setMipIndex(newMip);
    setChannelMode(newChannel);
    setOpaquePreview(newOpaque);
    if (nodePayload?.view_type === "TEXTURE") {
      await selectAndInspectNode(nodePayload.texture.node_id, newMip, newChannel, newOpaque);
    }
  }

  async function handleMenuOpenFile() {
    setActiveTopMenu(null);
    const picked = await invoke<string | null>("pick_file_dialog", {
      title: "Open PlatinumGames Archive or Asset",
      extensions: [
        "dat", "dtt", "evn", "pkz", "eff", "idd", "scr",
        "wtb", "wta", "wtp", "dds", "gtx", "xt1", "bntx",
        "wmb", "mdb", "bxm", "xml", "gad", "clp", "clh", "clw",
        "mot", "exp", "est", "cos", "att", "obi", "msq", "col",
        "fls", "fla", "cta", "lit", "csv",
      ],
    });
    if (picked) {
      const slash = Math.max(picked.lastIndexOf("\\"), picked.lastIndexOf("/"));
      if (slash > 0) {
        loadDirectory(picked.slice(0, slash));
      }
      await openDocumentByPath(picked);
    }
  }

  async function handleMenuOpenFolder() {
    setActiveTopMenu(null);
    const folder = await invoke<string | null>("pick_folder_dialog", {
      title: "Select Workspace Folder",
    });
    if (folder) {
      await loadDirectory(folder);
      setStatusLeft(`Opened folder ${folder}`);
    }
  }

  async function handleMenuSave() {
    setActiveTopMenu(null);
    if (!outline) return;
    try {
      const updated = await invoke<DocumentOutline>("save_document", { outputPath: null });
      setOutline(updated);
      setStatusLeft(`Saved ${updated.file_name}`);
      loadDirectory(currentDirRef.current);
    } catch (err) {
      setStatusLeft(`Save error: ${String(err)}`);
    }
  }

  async function handleMenuSaveAs() {
    setActiveTopMenu(null);
    if (!outline) return;
    const dest = await invoke<string | null>("save_file_dialog", {
      title: "Save Document As...",
      defaultName: outline.file_name.split(" + ")[0],
    });
    if (!dest) return;
    try {
      const updated = await invoke<DocumentOutline>("save_document", { outputPath: dest });
      setOutline(updated);
      setStatusLeft(`Saved to ${dest}`);
      loadDirectory(currentDirRef.current);
    } catch (err) {
      setStatusLeft(`Save As error: ${String(err)}`);
    }
  }

  async function handleUndo() {
    if (!outline?.can_undo) return;
    try {
      const label = outline.undo_label ?? "edit";
      const updated = await invoke<DocumentOutline>("undo_document");
      setOutline(updated);
      setWmbDocGeneration((g) => g + 1);
      await selectAndInspectNode(selectedNodeId);
      setStatusLeft(`Undid: ${label}`);
    } catch (err) {
      setStatusLeft(`Undo error: ${String(err)}`);
    }
  }

  async function handleRedo() {
    if (!outline?.can_redo) return;
    try {
      const label = outline.redo_label ?? "edit";
      const updated = await invoke<DocumentOutline>("redo_document");
      setOutline(updated);
      setWmbDocGeneration((g) => g + 1);
      await selectAndInspectNode(selectedNodeId);
      setStatusLeft(`Redid: ${label}`);
    } catch (err) {
      setStatusLeft(`Redo error: ${String(err)}`);
    }
  }

  shortcutHandlersRef.current = {
    openFile: () => {
      void handleMenuOpenFile();
    },
    save: () => {
      void handleMenuSave();
    },
    saveAs: () => {
      void handleMenuSaveAs();
    },
    undo: () => {
      void handleUndo();
    },
    redo: () => {
      void handleRedo();
    },
  };

  function handleResetView() {
    setZoom(1.0);
    setPan({ x: 0, y: 0 });
  }

  async function handlePickOodleDll() {
    const picked = await invoke<string | null>("pick_file_dialog", {
      title: "Locate Oodle Kraken Library (oo2core_*_win64.dll)",
      extensions: ["dll", "so", "dylib"],
    });
    if (picked) {
      setOodleDllPath(picked);
      await invoke("save_studio_settings", {
        settings: buildStudioSettingsPayload({ oodle_dll_path: picked }),
      }).catch(() => {});
      setStatusLeft(`Configured Oodle DLL: ${picked}`);
      if (outline) {
        await openDocumentByPath(outline.path);
      }
    }
  }

  async function handlePickNvttDll() {
    const picked = await invoke<string | null>("pick_file_dialog", {
      title: "Locate NVIDIA Texture Tools Library (nvtt30205.dll / nvtt.dll)",
      extensions: ["dll", "so", "dylib"],
    });
    if (picked) {
      setNvttDllPath(picked);
      await invoke("save_studio_settings", {
        settings: buildStudioSettingsPayload({ nvtt_dll_path: picked }),
      }).catch(() => {});
      setStatusLeft(`Configured NVTT DLL: ${picked}`);
    }
  }

  async function runTextureMutation(nodeId: string, action: string, payload: any = {}) {
    try {
      const fullPayload = {
        encode_format: texEncodeFormat,
        generate_mips: texGenerateMips,
        solidify: texSolidify,
        make_opaque: texMakeOpaque,
        ...payload,
      };
      const updated = await invoke<DocumentOutline>("mutate_texture_node", {
        nodeId,
        action,
        payload: fullPayload,
      });
      setOutline(updated);
      setWmbDocGeneration((g) => g + 1);
      await selectAndInspectNode(nodeId);
      setStatusLeft(updated.undo_label ?? `Updated texture`);
    } catch (err) {
      setStatusLeft(`Texture error: ${String(err)}`);
    }
  }

  async function promptAndRunTextureImport(
    nodeId: string,
    action: "replace_file" | "add_texture"
  ) {
    setContextMenu(null);
    const path = await invoke<string | null>("pick_file_dialog", {
      title:
        action === "replace_file"
          ? "Select Replacement Texture (PNG / TGA / BMP / JPG or Raw DDS / GTX / XT1 / BNTX)"
          : "Select Texture to Add to Bank",
      extensions: ["png", "dds", "gtx", "xt1", "bntx", "tga", "bmp", "jpg", "jpeg"],
    });
    if (!path) return;
    const low = path.toLowerCase();
    const isRawContainer =
      low.endsWith(".dds") || low.endsWith(".gtx") || low.endsWith(".xt1") || low.endsWith(".bntx");
    if (isRawContainer) {
      await runTextureMutation(nodeId, action, { path });
      return;
    }
    const fileName = path.split(/[\\/]/).pop() || path;
    setTexModalState({
      nodeId,
      action,
      path,
      fileName,
      encodeFormat: action === "add_texture" && texEncodeFormat === "Auto" ? "BC3_UNORM" : texEncodeFormat,
      generateMips: texGenerateMips,
      solidify: texSolidify,
      makeOpaque: texMakeOpaque,
    });
  }

  async function handleTextureExport(
    nodeId: string,
    scope: "single" | "bank",
    format: "png" | "dds" | "raw"
  ) {
    setContextMenu(null);
    try {
      if (scope === "single") {
        const ext = format === "png" ? "png" : format === "dds" ? "dds" : "bin";
        const dest = await invoke<string | null>("save_file_dialog", {
          title: `Export Texture (.${ext})`,
          defaultName: `texture.${ext}`,
        });
        if (!dest) return;
        const msg = await invoke<string>("export_texture_action", {
          nodeId,
          scope,
          format,
          destPath: dest,
        });
        setStatusLeft(msg);
      } else {
        const folder = await invoke<string | null>("pick_folder_dialog", {
          title: `Export All Textures as ${format.toUpperCase()} Folder`,
        });
        if (!folder) return;
        const msg = await invoke<string>("export_texture_action", {
          nodeId,
          scope,
          format,
          destPath: folder,
        });
        setStatusLeft(msg);
        loadDirectory(currentDirRef.current);
      }
    } catch (err) {
      setStatusLeft(`Export failed: ${String(err)}`);
    }
  }

  async function runWmbMutation(nodeId: string, operation: string, payload: any = {}) {
    setContextMenu(null);
    try {
      const updated = await invoke<DocumentOutline>("mutate_wmb_node", {
        nodeId,
        operation,
        payload,
      });
      setOutline(updated);
      setWmbDocGeneration((g) => g + 1);
      await selectAndInspectNode(nodeId);
      setStatusLeft(updated.undo_label ?? `Completed WMB ${operation}`);
    } catch (err) {
      setStatusLeft(`WMB error: ${String(err)}`);
    }
  }

  async function handleCrossGameMaterialConvert(
    wmb: WmbSectionView,
    dstGame: "b1" | "b2" | "b3"
  ) {
    setContextMenu(null);
    try {
      const srcGame: "b1" | "b2" | "b3" = wmb.is_wmb3
        ? "b3"
        : (wmbB2Override ?? wmb.is_bayo2)
        ? "b2"
        : "b1";
      const srcPayload = wmb.is_wmb3 ? wmb3MatDraft : wmbMatDraft;
      if (!srcPayload) return;

      const res = await convertMaterialBetweenGames(srcGame, dstGame, srcPayload, null);

      // Always store in clipboard so user can paste into another open model
      setMaterialClipboard({
        game: dstGame,
        payload: res.converted,
      });

      // If destination matches current model format (WMB0 for b1/b2, WMB3 for b3), apply in-place!
      if (!wmb.is_wmb3 && (dstGame === "b1" || dstGame === "b2") && res.converted?.fields) {
        const nextMat: DecodedWmbMaterial = {
          ...res.converted,
          index: wmbMatDraft?.index ?? 0,
        };
        setWmbB2Override(dstGame === "b2");
        setWmbMatDraft(nextMat);
        await runWmbMutation(wmb.node_id, "update_material", nextMat);
        setStatusLeft(
          `Converted material (${srcGame.toUpperCase()} → ${dstGame.toUpperCase()}) and applied in-place`
        );
      } else if (wmb.is_wmb3 && dstGame === "b3" && res.converted?.shader_name) {
        const matIdx = parseInt(wmb.section.replace("mat:", ""), 10) || 0;
        setWmb3MatDraft(res.converted);
        await runWmbMutation(wmb.node_id, "update_wmb3_material", {
          index: matIdx,
          material: res.converted,
        });
        setStatusLeft(`Converted material to Bayonetta 3 (${res.converted.shader_name})`);
      } else {
        setStatusLeft(
          `Converted material to ${dstGame.toUpperCase()} and copied to Material Clipboard (ready to paste)`
        );
      }
    } catch (err) {
      setStatusLeft(`Material conversion error: ${String(err)}`);
    }
  }

  async function runStructMutation(nodeId: string, structKind: string, updatedData: any) {
    try {
      const updated = await invoke<DocumentOutline>("mutate_struct_node", {
        nodeId,
        structKind,
        updatedData,
      });
      setOutline(updated);
      await selectAndInspectNode(nodeId);
      setStatusLeft(updated.undo_label ?? `Updated ${structKind}`);
    } catch (err) {
      setStatusLeft(`${structKind} update error: ${String(err)}`);
    }
  }

  async function runClothMutation(
    nodeId: string,
    operation: string,
    preset?: string,
    updatedJson?: any
  ) {
    setContextMenu(null);
    try {
      const updated = await invoke<DocumentOutline>("mutate_cloth_node", {
        nodeId,
        operation,
        preset: preset ?? null,
        updatedJson: updatedJson ?? null,
      });
      setOutline(updated);
      await selectAndInspectNode(nodeId);
      setStatusLeft(updated.undo_label ?? `Completed Cloth ${operation}`);
    } catch (err) {
      setStatusLeft(`Cloth error: ${String(err)}`);
    }
  }

  async function runContainerMutation(
    nodeId: string,
    action: string,
    targetPath = "",
    payload: any = null
  ) {
    setContextMenu(null);
    try {
      const updated = await invoke<DocumentOutline>("mutate_container_node", {
        nodeId,
        action,
        targetPath,
        payload,
      });
      setOutline(updated);
      if (action === "remove_entry") {
        const parent = nodeId.includes("/") ? nodeId.slice(0, nodeId.lastIndexOf("/")) : "root";
        await selectAndInspectNode(parent);
      } else {
        await selectAndInspectNode(nodeId);
      }
      setStatusLeft(updated.undo_label ?? `Completed ${action}`);
    } catch (err) {
      const msg = String(err);
      if (msg.includes("OODLE_DLL_MISSING")) {
        const useZstd = window.confirm(
          "OodleKraken compression requires oo2core_*_win64.dll.\n\nClick OK to compress using ZStandard instead, or Cancel to locate oo2core_*_win64.dll."
        );
        if (useZstd) {
          await runContainerMutation(nodeId, action, targetPath, {
            ...(payload || {}),
            compression: "ZStandard",
            allow_zstd_fallback: true,
          });
        } else {
          await handlePickOodleDll();
        }
        return;
      }
      setStatusLeft(`Container operation failed: ${msg}`);
    }
  }

  const filteredFiles = useMemo(() => {
    const regex = compileGlobFilter(fileGlobFilter);
    const list = fsEntries.filter((e) => {
      if (hideUnsupported && !e.is_dir && !e.is_supported) return false;
      if (!regex) return true;
      return regex.test(e.name);
    });
    list.sort((a, b) => {
      if (a.is_dir !== b.is_dir) return a.is_dir ? -1 : 1;
      let cmp = 0;
      if (fileSortCol === "name") {
        cmp = a.name.localeCompare(b.name, undefined, { numeric: true, sensitivity: "base" });
      } else if (fileSortCol === "type") {
        cmp =
          a.extension.localeCompare(b.extension, undefined, { sensitivity: "base" }) ||
          a.name.localeCompare(b.name, undefined, { numeric: true, sensitivity: "base" });
      } else if (fileSortCol === "modified") {
        cmp = a.modified_epoch_ms - b.modified_epoch_ms;
      } else {
        cmp = a.size - b.size;
      }
      return fileSortAsc ? cmp : -cmp;
    });
    return list;
  }, [fsEntries, fileGlobFilter, hideUnsupported, fileSortCol, fileSortAsc]);

  function toggleCard(key: string) {
    setOpenCards((prev) => ({ ...prev, [key]: !prev[key] }));
  }

  function toggleSort(col: FileSortCol) {
    if (fileSortCol === col) {
      setFileSortAsc(!fileSortAsc);
    } else {
      setFileSortCol(col);
      setFileSortAsc(true);
    }
  }

  // Recursive Asset Tree Renderer with Pointer-Based Drag Reordering + Drag-Out to Explorer
  function renderTreeNodes(nodes: AssetTreeNode[]) {
    const filterLow = assetSearch.trim().toLowerCase();
    return nodes.map((node) => {
      const hasChildren = node.children && node.children.length > 0;
      const isExpanded = !!expandedNodes[node.id];
      const isSelected = selectedNodeId === node.id;
      const canDragNode = node.id !== "root" && !isVirtualSubNode(node.id);
      const isDragOverBefore =
        pointerDrag?.active &&
        pointerDrag.kind === "asset" &&
        pointerDrag.overId === node.id &&
        pointerDrag.insertPos === "before";
      const isDragOverAfter =
        pointerDrag?.active &&
        pointerDrag.kind === "asset" &&
        pointerDrag.overId === node.id &&
        pointerDrag.insertPos === "after";

      const matchesFilter = (n: AssetTreeNode): boolean => {
        if (!filterLow) return true;
        if (
          n.label.toLowerCase().includes(filterLow) ||
          n.format_badge.toLowerCase().includes(filterLow)
        ) {
          return true;
        }
        return n.children?.some(matchesFilter) ?? false;
      };
      if (!matchesFilter(node)) return null;

      return (
        <div key={node.id}>
          <div
            data-asset-node-id={node.id}
            className={`tree-row ${isSelected ? "selected" : ""} ${
              isDragOverBefore ? "drag-insert-before" : ""
            } ${isDragOverAfter ? "drag-insert-after" : ""}`}
            onPointerDown={(e) => {
              if (e.button !== 0 || !canDragNode) return;
              setPointerDrag({
                kind: "asset",
                sourceId: node.id,
                label: node.label,
                startX: e.clientX,
                startY: e.clientY,
                curX: e.clientX,
                curY: e.clientY,
                active: false,
                overId: null,
                insertPos: null,
                dragOutTriggered: false,
              });
            }}
            onClick={() => {
              if (hasChildren && !expandedNodes[node.id]) {
                setExpandedNodes((p) => ({ ...p, [node.id]: true }));
              }
              selectAndInspectNode(node.id);
            }}
            onContextMenu={(e) => {
              e.preventDefault();
              e.stopPropagation();
              selectAndInspectNode(node.id);
              setHoverSubmenuIdx(null);
              const items = buildAssetNodeContextMenu(node.id, node.kind);
              if (items.length > 0) {
                setContextMenu({
                  x: Math.min(e.clientX, window.innerWidth - 260),
                  y: Math.min(e.clientY, window.innerHeight - 340),
                  items,
                });
              }
            }}
          >
            <span
              className={`tree-arrow ${isExpanded ? "expanded" : ""}`}
              style={{ visibility: hasChildren ? "visible" : "hidden" }}
              onClick={(e) => {
                e.stopPropagation();
                setExpandedNodes((p) => ({ ...p, [node.id]: !p[node.id] }));
              }}
            >
              <svg width="6" height="9" viewBox="0 0 6 10" fill="none" stroke="currentColor" strokeWidth="1.5">
                <path d="M1 1L5 5L1 9" strokeLinecap="round" strokeLinejoin="round" />
              </svg>
            </span>
            <span className="tree-badge">{node.format_badge}</span>
            <span style={{ overflow: "hidden", textOverflow: "ellipsis" }}>{node.label}</span>
          </div>
          {hasChildren && (isExpanded || !!filterLow) && (
            <div className="tree-children">{renderTreeNodes(node.children)}</div>
          )}
        </div>
      );
    });
  }

  const activeTexture: TextureNodeView | null =
    nodePayload?.view_type === "TEXTURE" ? nodePayload.texture : null;
  const activeWmb: WmbSectionView | null =
    nodePayload?.view_type === "WMB" ? nodePayload.wmb : null;

  const { buildAssetNodeContextMenu, buildFileBrowserContextMenu } = createMenuBuilders(
    {
      activeWmb,
      wmbMatDraft,
      wmb3MatDraft,
      wmbB2Override,
      materialClipboard,
      setMaterialClipboard,
      setContextMenu,
      setStatusLeft,
      runWmbMutation,
      handleCrossGameMaterialConvert,
      runContainerMutation,
      promptAndRunTextureImport,
      handleTextureExport,
      runTextureMutation,
      runClothMutation,
      loadDirectory,
      currentDirRef,
    },
    {
      selectedFilePaths,
      setSelectedFilePaths,
      filteredFiles,
      fsClipboard,
      setFsClipboard,
      currentDir,
      setContextMenu,
      setStatusLeft,
      loadDirectory,
      openDocumentByPath,
    }
  );

  return (
    <div
      className="studio-window"
      onClick={() => {
        setActiveTopMenu(null);
        setContextMenu(null);
      }}
      onContextMenu={(e) => {
        // Clicking empty space shows nothing and closes any open custom menu
        e.preventDefault();
        setContextMenu(null);
        setActiveTopMenu(null);
      }}
      onDragOver={(e) => {
        // External OS file drops are handled via Tauri's native tauri://drag-drop event listener (B-43)
        if (e.dataTransfer?.types?.includes("Files")) {
          e.preventDefault();
          e.dataTransfer.dropEffect = "copy";
        }
      }}
    >
      {/* Floating Pointer Drag Ghost Badge */}
      {pointerDrag?.active && (
        <div
          className="pointer-drag-ghost"
          style={{
            left: pointerDrag.curX + 14,
            top: pointerDrag.curY + 12,
          }}
        >
          <span className="mono">{pointerDrag.label}</span>
          <span style={{ color: "var(--brush-muted)", fontWeight: 400, fontSize: 11 }}>
            {pointerDrag.overId
              ? `Drop ${pointerDrag.insertPos} target`
              : pointerDrag.kind === "asset"
              ? "Drag outside to extract to Explorer"
              : "Drag to reorder"}
          </span>
        </div>
      )}

      {/* External OS File Drop Visual Overlay */}
      {externalDragHover && (
        <div className="external-drop-overlay">Drop PlatinumGames file to open in Studio</div>
      )}
      <header className="studio-menubar" onClick={(e) => e.stopPropagation()}>
        <div style={{ position: "relative" }}>
          <button
            className={`menu-top-button ${activeTopMenu === "file" ? "active" : ""}`}
            onClick={() => setActiveTopMenu(activeTopMenu === "file" ? null : "file")}
          >
            File
          </button>
          {activeTopMenu === "file" && (
            <div className="context-menu" style={{ top: 28, left: 0 }}>
              <button className="context-menu-item" onClick={handleMenuOpenFile}>
                <span>Open File...</span>
                <span className="context-menu-shortcut">Ctrl+O</span>
              </button>
              <button className="context-menu-item" onClick={handleMenuOpenFolder}>
                <span>Open Folder...</span>
              </button>
              <div
                className="context-menu-item"
                onMouseEnter={() => setHoverSubmenuIdx(99)}
                onMouseLeave={() => setHoverSubmenuIdx(null)}
              >
                <span>Open Recent</span>
                <span className="context-menu-shortcut">›</span>
                {hoverSubmenuIdx === 99 && (
                  <div className="context-submenu">
                    {recentFiles.length === 0 ? (
                      <button className="context-menu-item" disabled>
                        <span>(No recent files)</span>
                      </button>
                    ) : (
                      recentFiles.map((rf, idx) => (
                        <button
                          key={idx}
                          className="context-menu-item"
                          onClick={() => {
                            setActiveTopMenu(null);
                            openDocumentByPath(rf);
                          }}
                          title={rf}
                        >
                          <span style={{ overflow: "hidden", textOverflow: "ellipsis", maxWidth: 260 }}>
                            {rf.split(/[\\/]/).pop()}
                          </span>
                        </button>
                      ))
                    )}
                  </div>
                )}
              </div>
              <button
                className="context-menu-item"
                disabled={!outline}
                onClick={() => {
                  setActiveTopMenu(null);
                  if (outline) openDocumentByPath(outline.path);
                }}
              >
                <span>Reload</span>
              </button>
              <div className="context-menu-sep" />
              <button className="context-menu-item" disabled={!outline} onClick={handleMenuSave}>
                <span>Save</span>
                <span className="context-menu-shortcut">Ctrl+S</span>
              </button>
              <button className="context-menu-item" disabled={!outline} onClick={handleMenuSaveAs}>
                <span>Save As...</span>
                <span className="context-menu-shortcut">Ctrl+Shift+S</span>
              </button>
            </div>
          )}
        </div>

        <div style={{ position: "relative" }}>
          <button
            className={`menu-top-button ${activeTopMenu === "tools" ? "active" : ""}`}
            onClick={() => setActiveTopMenu(activeTopMenu === "tools" ? null : "tools")}
          >
            Tools
          </button>
          {activeTopMenu === "tools" && (
            <div className="context-menu" style={{ top: 28, left: 0, minWidth: 265 }}>
              <button
                className="context-menu-item"
                disabled={!activeTexture}
                onClick={() => {
                  setActiveTopMenu(null);
                  if (activeTexture) handleTextureExport(activeTexture.node_id, "bank", "png");
                }}
              >
                <span>Export all textures in bank to PNG...</span>
              </button>
              <button
                className="context-menu-item"
                disabled={!activeTexture}
                onClick={() => {
                  setActiveTopMenu(null);
                  if (activeTexture) handleTextureExport(activeTexture.node_id, "bank", "dds");
                }}
              >
                <span>Export all textures in bank to DDS...</span>
              </button>
              <div className="context-menu-sep" />
              <button
                className="context-menu-item"
                disabled={!activeWmb}
                onClick={() => {
                  setActiveTopMenu(null);
                  if (activeWmb) runWmbMutation(activeWmb.node_id, "cleanup_vertexes");
                }}
              >
                <span>WMB: Cleanup unreferenced vertexes</span>
              </button>
              <button
                className="context-menu-item"
                disabled={!activeWmb}
                onClick={() => {
                  setActiveTopMenu(null);
                  if (activeWmb) runWmbMutation(activeWmb.node_id, "cleanup_materials");
                }}
              >
                <span>WMB: Cleanup unused materials</span>
              </button>
              <div className="context-menu-sep" />
              <button
                className="context-menu-item"
                onClick={() => {
                  setActiveTopMenu(null);
                  handlePickOodleDll();
                }}
              >
                <span>Configure Oodle Kraken DLL (oo2core)...</span>
              </button>
              <button
                className="context-menu-item"
                onClick={() => {
                  setActiveTopMenu(null);
                  handlePickNvttDll();
                }}
              >
                <span>Configure NVIDIA Texture Tools DLL (nvtt30205)...</span>
              </button>
              <div className="context-menu-sep" />
              <button
                className="context-menu-item"
                onClick={() => {
                  setActiveTopMenu(null);
                  setCoreDatModalOpen(true);
                }}
              >
                <span>Configure core.dat (per game)...</span>
              </button>
              <div className="context-menu-sep" />
              <div style={{ padding: "4px 10px", fontSize: 11, color: "var(--brush-muted)", fontWeight: 600 }}>
                Open demo model
              </div>
              {(
                [
                  ["b1", "Bayonetta 1 (WMB0)"],
                  ["b2", "Bayonetta 2 (WMB0)"],
                  ["b3", "Bayonetta 3 (WMB3)"],
                ] as const
              ).map(([gameKey, gameLabel]) => (
                <button
                  key={gameKey}
                  className="context-menu-item"
                  onClick={async () => {
                    setActiveTopMenu(null);
                    try {
                      setStatusLeft(`Generating ${gameLabel} demo model...`);
                      const demoPath = await createDemoModel(gameKey);
                      await openDocumentByPath(demoPath);
                    } catch (err) {
                      setStatusLeft(`Demo model error: ${String(err)}`);
                    }
                  }}
                >
                  <span>• {gameLabel}</span>
                </button>
              ))}
            </div>
          )}
        </div>

        <div style={{ position: "relative" }}>
          <button
            className={`menu-top-button ${activeTopMenu === "help" ? "active" : ""}`}
            onClick={() => setActiveTopMenu(activeTopMenu === "help" ? null : "help")}
          >
            Help
          </button>
          {activeTopMenu === "help" && (
            <div className="context-menu" style={{ top: 28, left: 0, minWidth: 280 }}>
              <div style={{ padding: "8px 10px" }}>
                <div style={{ fontWeight: 600, color: "var(--brush-heading)" }}>PGTools Studio</div>
                <div style={{ color: "var(--brush-muted)", fontSize: 11.5, marginTop: 3 }}>
                  Pure Rust &amp; Tauri v2 suite for Bayonetta 1/2/3, NieR:Automata, MGR:R, Vanquish &amp; Astral Chain.
                </div>
              </div>
            </div>
          )}
        </div>
      </header>
      <div className="studio-toolbar">
        <div className="toolbar-group">
          {activeTexture && (
            <>
              <span className="toolbar-label">Mip</span>
              <select
                className="pill-select"
                style={{ width: 156 }}
                value={mipIndex}
                onChange={(e) =>
                  handleTexturePreviewParamChange(Number(e.target.value), channelMode, opaquePreview)
                }
              >
                {activeTexture.mips.map((m) => (
                  <option key={m.index} value={m.index}>
                    {m.label}
                  </option>
                ))}
              </select>

              <div className="toolbar-spacer" />

              <span className="toolbar-label">Channel</span>
              <select
                className="pill-select"
                style={{ width: 110 }}
                value={channelMode}
                onChange={(e) =>
                  handleTexturePreviewParamChange(mipIndex, e.target.value, opaquePreview)
                }
              >
                {["RGBA", "RGB", "R", "G", "B", "A"].map((ch) => (
                  <option key={ch} value={ch}>
                    {ch}
                  </option>
                ))}
              </select>

              <div className="toolbar-spacer" />

              <div
                className={`pill-checkbox ${opaquePreview ? "checked" : ""}`}
                onClick={() =>
                  handleTexturePreviewParamChange(mipIndex, channelMode, !opaquePreview)
                }
                title="Ignore the alpha channel in the 2D preview"
              >
                <span className="pill-checkbox-box">{opaquePreview && <IconCheck />}</span>
                <span>Opaque Preview</span>
              </div>

              <div className="toolbar-spacer" />

              <button
                className="pill-button"
                onClick={handleResetView}
                title="Fit preview to viewport (100%)"
              >
                <IconRefresh />
                <span>Reset View</span>
              </button>

              <div className="toolbar-spacer" />
            </>
          )}

          <button
            className="pill-button"
            disabled={!outline?.can_undo}
            onClick={handleUndo}
            title={outline?.undo_label ? `Undo ${outline.undo_label} (Ctrl+Z)` : "Undo (Ctrl+Z)"}
          >
            <IconUndo />
            <span>Undo</span>
          </button>

          <button
            className="pill-button"
            disabled={!outline?.can_redo}
            onClick={handleRedo}
            title={outline?.redo_label ? `Redo ${outline.redo_label} (Ctrl+Y)` : "Redo (Ctrl+Y)"}
          >
            <IconRedo />
            <span>Redo</span>
          </button>
        </div>

        <div className="toolbar-group">
          {outline?.dirty && (
            <button
              className="pill-button accent"
              onClick={handleMenuSave}
              title="Save modified container in place (Ctrl+S)"
            >
              <span>● Save {outline.file_name}</span>
            </button>
          )}
          <button
            className="pill-button"
            onClick={() => setTheme(theme === "dark" ? "light" : "dark")}
            title="Toggle light / dark theme"
          >
            <IconSunMoon />
            <span>Theme</span>
          </button>
        </div>
      </div>
      <div className="studio-main">
        {/* LEFT COLUMN: Stacked Asset Browser + Resizable File Browser */}
        <div className="left-column" ref={leftColRef} style={{ width: leftWidth }}>
          {/* Top-Left: Asset Browser */}
          <div
            ref={assetBrowserCardRef}
            className="studio-card"
            style={{ flex: `${assetRatio} 1 0%`, minHeight: 110 }}
          >
            <div className="pane-header">
              <span className="pane-title">Asset Browser</span>
              <input
                type="text"
                className="field-box"
                style={{ width: 135, height: 24, fontSize: 11.5 }}
                placeholder="Filter nodes..."
                value={assetSearch}
                onChange={(e) => setAssetSearch(e.target.value)}
              />
            </div>
            <div className="tree-scroll">
              {outline ? (
                renderTreeNodes(outline.tree)
              ) : (
                <div style={{ padding: "18px 10px", color: "var(--brush-muted)", textAlign: "center" }}>
                  Select a file in the File Browser below, drag &amp; drop a file onto the window, or press Ctrl+O.
                </div>
              )}
            </div>
          </div>

          {/* Horizontal Splitter between Asset Browser and File Browser */}
          <div
            className={`splitter-h ${draggingSplit === "left-row" ? "dragging" : ""}`}
            onMouseDown={(e) => {
              e.preventDefault();
              setDraggingSplit("left-row");
            }}
            onDoubleClick={() => setAssetRatio(0.5)}
            title="Drag to resize, double-click to reset"
          >
            <div className="splitter-h-grip" />
          </div>

          {/* Bottom-Left: File Browser with Sort by Type & Hide Unsupported Toggle */}
          <div className="studio-card" style={{ flex: `${1 - assetRatio} 1 0%`, minHeight: 110 }}>
            <div className="pane-header">
              <span className="pane-title">File Browser</span>
              <div style={{ display: "flex", gap: 4, alignItems: "center" }}>
                <button
                  className={`pill-button ${hideUnsupported ? "accent" : ""}`}
                  style={{ height: 24, padding: "0 8px", fontSize: 11 }}
                  onClick={() => setHideUnsupported(!hideUnsupported)}
                  title="Hide unsupported file extensions (On by default)"
                >
                  Supported Only
                </button>
                <button
                  className="pill-button"
                  style={{ height: 24, padding: "0 8px", fontSize: 11.5 }}
                  onClick={() => {
                    if (!currentDir) return;
                    const up = currentDir.replace(/[\\/][^\\/]+$/, "");
                    if (up && up !== currentDir) loadDirectory(up);
                  }}
                  title="Parent Directory"
                >
                  ↑ Up
                </button>
                <button
                  className="pill-button"
                  style={{ height: 24, padding: "0 8px", fontSize: 11.5 }}
                  onClick={handleMenuOpenFolder}
                  title="Browse Folder..."
                >
                  <IconFolder />
                </button>
              </div>
            </div>

            <div className="file-browser-controls">
              <div className="file-folder-bar">
                <input
                  type="text"
                  className="field-box mono"
                  style={{ flex: 1, height: 25, fontSize: 11 }}
                  value={dirInput}
                  onChange={(e) => setDirInput(e.target.value)}
                  onKeyDown={(e) => {
                    if (e.key === "Enter") loadDirectory(dirInput);
                  }}
                  placeholder="Folder path..."
                />
              </div>
              <input
                type="text"
                className="field-box mono"
                style={{ width: "100%", height: 25, fontSize: 11.5 }}
                value={fileGlobFilter}
                onChange={(e) => setFileGlobFilter(e.target.value)}
                placeholder="Glob filter (e.g. *.dat, pl00*.wmb, *.pkz)..."
              />
            </div>

            <div className="file-table-wrap">
              <table className="file-table">
                <thead>
                  <tr>
                    <th style={{ width: "42%" }} onClick={() => toggleSort("name")}>
                      Name {fileSortCol === "name" ? (fileSortAsc ? "▲" : "▼") : ""}
                    </th>
                    <th style={{ width: "16%" }} onClick={() => toggleSort("type")}>
                      Type {fileSortCol === "type" ? (fileSortAsc ? "▲" : "▼") : ""}
                    </th>
                    <th style={{ width: "22%" }} onClick={() => toggleSort("modified")}>
                      Modified {fileSortCol === "modified" ? (fileSortAsc ? "▲" : "▼") : ""}
                    </th>
                    <th style={{ width: "20%", textAlign: "right" }} onClick={() => toggleSort("size")}>
                      Size {fileSortCol === "size" ? (fileSortAsc ? "▲" : "▼") : ""}
                    </th>
                  </tr>
                </thead>
                <tbody>
                  {filteredFiles.map((entry, idx) => {
                    const isSelected =
                      selectedFilePath === entry.path || selectedFilePaths.includes(entry.path);
                    return (
                      <tr
                        key={entry.path}
                        className={`file-row ${isSelected ? "selected" : ""}`}
                        title={entry.path}
                        onClick={(e) => {
                          if (e.ctrlKey || e.metaKey) {
                            setSelectedFilePaths((prev) =>
                              prev.includes(entry.path)
                                ? prev.filter((p) => p !== entry.path)
                                : [...prev, entry.path]
                            );
                            setSelectedFilePath(entry.path);
                            return;
                          }
                          if (e.shiftKey && selectedFilePath) {
                            const anchorIdx = filteredFiles.findIndex(
                              (f) => f.path === selectedFilePath
                            );
                            if (anchorIdx !== -1) {
                              const lo = Math.min(anchorIdx, idx);
                              const hi = Math.max(anchorIdx, idx);
                              setSelectedFilePaths(filteredFiles.slice(lo, hi + 1).map((f) => f.path));
                              return;
                            }
                          }
                          setSelectedFilePaths([entry.path]);
                          setSelectedFilePath(entry.path);
                        }}
                        onDoubleClick={() => {
                          if (entry.is_dir) {
                            void loadDirectory(entry.path);
                          } else {
                            void openDocumentByPath(entry.path);
                          }
                        }}
                        onContextMenu={(e) => {
                          e.preventDefault();
                          e.stopPropagation();
                          if (!selectedFilePaths.includes(entry.path)) {
                            setSelectedFilePaths([entry.path]);
                          }
                          setSelectedFilePath(entry.path);
                          setHoverSubmenuIdx(null);
                          setContextMenu({
                            x: Math.min(e.clientX, window.innerWidth - 250),
                            y: Math.min(e.clientY, window.innerHeight - 360),
                            items: buildFileBrowserContextMenu(entry),
                          });
                        }}
                      >
                        <td>
                          {entry.is_dir ? "📁 " : ""}
                          {entry.name}
                        </td>
                        <td
                          className="mono"
                          style={{ color: isSelected ? "var(--brush-text)" : "var(--brush-muted)" }}
                        >
                          {entry.is_dir ? "DIR" : entry.extension ? `.${entry.extension}` : "—"}
                        </td>
                        <td style={{ color: isSelected ? "var(--brush-text)" : "var(--brush-muted)" }}>
                          {entry.modified_str}
                        </td>
                        <td
                          style={{
                            textAlign: "right",
                            color: isSelected ? "var(--brush-text)" : "var(--brush-muted)",
                          }}
                        >
                          {entry.is_dir ? "" : formatBytes(entry.size)}
                        </td>
                      </tr>
                    );
                  })}
                </tbody>
              </table>
            </div>
          </div>
        </div>

        {/* Left Vertical Splitter */}
        <div
          className={`splitter-v ${draggingSplit === "left" ? "dragging" : ""}`}
          onMouseDown={(e) => {
            e.preventDefault();
            setDraggingSplit("left");
          }}
          onDoubleClick={() => setLeftWidth(310)}
          title="Drag to resize, double-click to reset"
        >
          <div className="splitter-v-grip" />
        </div>

        {Boolean(activeWmb && !isVirtualSubNode(selectedNodeId)) ? (
          <Workspace
            nodeId={selectedNodeId}
            docGeneration={wmbDocGeneration}
            theme={theme}
            rightPanelWidth={rightWidth}
            onStartResizeRightPanel={(e) => {
              e.preventDefault();
              setDraggingSplit("right");
            }}
            hierarchyRatio={wmbHierarchyRatio}
            onChangeHierarchyRatio={setWmbHierarchyRatio}
            wireframe={wmbWireframe}
            onToggleWireframe={() => setWmbWireframe((v) => !v)}
            normalsView={wmbNormalsView}
            onToggleNormalsView={() => setWmbNormalsView((v) => !v)}
            skeletonOverlay={wmbSkeletonOverlay}
            onToggleSkeletonOverlay={() => setWmbSkeletonOverlay((v) => !v)}
            gridVisible={wmbGridVisible}
            onToggleGridVisible={() => setWmbGridVisible((v) => !v)}
            onOutlineUpdated={(updated, statusMsg) => {
              setOutline(updated);
              setWmbDocGeneration((g) => g + 1);
              if (statusMsg) setStatusLeft(updated.undo_label ?? statusMsg);
            }}
            onStatusError={(msg) => setStatusLeft(msg)}
            onTriggerLegacyAction={async (legacyNodeId: string, action: LegacyActionKind) => {
              if (action === "export_obj") {
                const dest = await invoke<string | null>("save_file_dialog", {
                  title: "Export WMB Model to Wavefront .OBJ",
                  defaultName: "model.obj",
                });
                if (dest) await runWmbMutation(selectedNodeId, "export_obj", { path: dest });
              } else if (action === "export_texture") {
                await handleTextureExport(legacyNodeId, "single", "png");
              } else if (action === "replace_texture") {
                await promptAndRunTextureImport(legacyNodeId, "replace_file");
              } else if (action === "import_texture") {
                await promptAndRunTextureImport(legacyNodeId, "add_texture");
              } else {
                await selectAndInspectNode(legacyNodeId);
              }
            }}
          />
        ) : (
        <>
        {/* CENTER COLUMN: Contextual Viewport / Structured Table Editor */}
        <div className="center-column">
          <div className="studio-card viewer-card">
            <div className="underline-tabbar">
              {activeTexture ? (
                <>
                  <button
                    className={`underline-tab ${centerTab === "primary" ? "active" : ""}`}
                    onClick={() => setCenterTab("primary")}
                  >
                    <IconImage size={12} />
                    <span>2D</span>
                  </button>
                  <button
                    className={`underline-tab ${centerTab === "secondary" ? "active" : ""}`}
                    onClick={() => setCenterTab("secondary")}
                  >
                    <IconCube size={12} />
                    <span>Header &amp; Surface Layout</span>
                  </button>
                </>
              ) : nodePayload?.view_type === "BXM" ? (
                <>
                  <button
                    className={`underline-tab ${bxmViewMode === "tree" ? "active" : ""}`}
                    onClick={() => switchBxmViewMode("tree")}
                  >
                    <IconCode size={12} />
                    <span>Interactive Tree</span>
                  </button>
                  <button
                    className={`underline-tab ${bxmViewMode === "table" ? "active" : ""}`}
                    onClick={() => switchBxmViewMode("table")}
                  >
                    <IconCube size={12} />
                    <span>Table View</span>
                  </button>
                  <button
                    className={`underline-tab ${bxmViewMode === "xml" ? "active" : ""}`}
                    onClick={() => switchBxmViewMode("xml")}
                  >
                    <IconCode size={12} />
                    <span>Raw XML</span>
                  </button>
                </>
              ) : (
                <>
                  <button
                    className={`underline-tab ${centerTab === "primary" ? "active" : ""}`}
                    onClick={() => setCenterTab("primary")}
                  >
                    <IconCube size={12} />
                    <span>Structured Editor</span>
                  </button>
                  <button
                    className={`underline-tab ${centerTab === "secondary" ? "active" : ""}`}
                    onClick={() => setCenterTab("secondary")}
                  >
                    <IconCode size={12} />
                    <span>Hex / Raw Inspector</span>
                  </button>
                </>
              )}
            </div>

            {/* Center Body */}
            {activeTexture ? (
              <TextureCenterView
                activeTexture={activeTexture}
                centerTab={centerTab}
                zoom={zoom}
                setZoom={setZoom}
                pan={pan}
                setPan={setPan}
                isPanning={isPanning}
                setIsPanning={setIsPanning}
                panStartRef={panStartRef}
                handleResetView={handleResetView}
                buildAssetNodeContextMenu={buildAssetNodeContextMenu}
                setContextMenu={setContextMenu}
                setHoverSubmenuIdx={setHoverSubmenuIdx}
              />
            ) : activeWmb && centerTab === "primary" ? (
              <div
                className="struct-table-container"
                onContextMenu={(e) => {
                  e.preventDefault();
                  e.stopPropagation();
                  setHoverSubmenuIdx(null);
                  const items = buildAssetNodeContextMenu(
                    activeWmb.node_id,
                    activeWmb.is_wmb3 ? "WMB3" : "WMB"
                  );
                  if (items.length > 0) {
                    setContextMenu({
                      x: Math.min(e.clientX, window.innerWidth - 250),
                      y: Math.min(e.clientY, window.innerHeight - 260),
                      items,
                    });
                  }
                }}
              >
                <WmbStructuredCenterView
                  wmb={activeWmb}
                  wmbMatDraft={wmbMatDraft}
                  setWmbMatDraft={setWmbMatDraft}
                  wmb3MatDraft={wmb3MatDraft}
                  setWmb3MatDraft={setWmb3MatDraft}
                  wmbRawMode={wmbRawMode}
                  setWmbRawMode={setWmbRawMode}
                  wmbB2Override={wmbB2Override}
                  setWmbB2Override={setWmbB2Override}
                  materialClipboard={materialClipboard}
                  setMaterialClipboard={setMaterialClipboard}
                  wmbHeaderFlagsDraft={wmbHeaderFlagsDraft}
                  setWmbHeaderFlagsDraft={setWmbHeaderFlagsDraft}
                  wmbBboxDraft={wmbBboxDraft}
                  setWmbBboxDraft={setWmbBboxDraft}
                  expandedMeshGroups={expandedMeshGroups}
                  setExpandedMeshGroups={setExpandedMeshGroups}
                  expandedBatchLods={expandedBatchLods}
                  setExpandedBatchLods={setExpandedBatchLods}
                  draggedBatchInfo={draggedBatchInfo}
                  setDraggedBatchInfo={setDraggedBatchInfo}
                  dragOverMeshIdx={dragOverMeshIdx}
                  setDragOverMeshIdx={setDragOverMeshIdx}
                  selectAndInspectNode={selectAndInspectNode}
                  runWmbMutation={runWmbMutation}
                  handleCrossGameMaterialConvert={handleCrossGameMaterialConvert}
                  setStatusLeft={setStatusLeft}
                />
              </div>
            ) : nodePayload?.view_type === "BXM" ? (
              <BxmCenterView
                nodePayload={nodePayload}
                bxmViewMode={bxmViewMode}
                bxmTreeDraft={bxmTreeDraft}
                bxmXmlDraft={bxmXmlDraft}
                setBxmXmlDraft={setBxmXmlDraft}
                selectedBxmPath={selectedBxmPath}
                setSelectedBxmPath={setSelectedBxmPath}
                bxmExpandedPaths={bxmExpandedPaths}
                setBxmExpandedPaths={setBxmExpandedPaths}
                pointerDrag={pointerDrag}
                setPointerDrag={setPointerDrag}
                updateBxmDraftTree={updateBxmDraftTree}
                commitBxmDraft={commitBxmDraft}
                runContainerMutation={runContainerMutation}
                setContextMenu={setContextMenu}
                setHoverSubmenuIdx={setHoverSubmenuIdx}
                setStatusLeft={setStatusLeft}
              />
            ) : (
              <StructCenterView
                nodePayload={nodePayload}
                setNodePayload={setNodePayload}
                centerTab={centerTab}
                csvHeadersDraft={csvHeadersDraft}
                setCsvHeadersDraft={setCsvHeadersDraft}
                csvRowsDraft={csvRowsDraft}
                setCsvRowsDraft={setCsvRowsDraft}
                estDraft={estDraft}
                setEstDraft={setEstDraft}
                structDraft={structDraft}
                setStructDraft={setStructDraft}
                selectedMotTrackIdx={selectedMotTrackIdx}
                setSelectedMotTrackIdx={setSelectedMotTrackIdx}
                pkzBulkCodec={pkzBulkCodec}
                setPkzBulkCodec={setPkzBulkCodec}
                runStructMutation={runStructMutation}
                runClothMutation={runClothMutation}
                runContainerMutation={runContainerMutation}
                selectAndInspectNode={selectAndInspectNode}
                handlePickOodleDll={handlePickOodleDll}
                loadDirectory={loadDirectory}
                currentDirRef={currentDirRef}
                buildAssetNodeContextMenu={buildAssetNodeContextMenu}
                setContextMenu={setContextMenu}
                setHoverSubmenuIdx={setHoverSubmenuIdx}
                setStatusLeft={setStatusLeft}
              />
            )}
          </div>
        </div>

        <div
          className={`splitter-v ${draggingSplit === "right" ? "dragging" : ""}`}
          onMouseDown={(e) => {
            e.preventDefault();
            setDraggingSplit("right");
          }}
          onDoubleClick={() => setRightWidth(380)}
          title="Drag to resize, double-click to reset"
        >
          <div className="splitter-v-grip" />
        </div>

        <div className="right-column" style={{ width: rightWidth }}>
          {activeTexture ? (
            <TextureInspectorView
              activeTexture={activeTexture}
              openCards={openCards}
              toggleCard={toggleCard}
              hashDraft={hashDraft}
              setHashDraft={setHashDraft}
              flagsDraft={flagsDraft}
              setFlagsDraft={setFlagsDraft}
              texEncodeFormat={texEncodeFormat}
              setTexEncodeFormat={setTexEncodeFormat}
              texGenerateMips={texGenerateMips}
              setTexGenerateMips={setTexGenerateMips}
              texSolidify={texSolidify}
              setTexSolidify={setTexSolidify}
              texMakeOpaque={texMakeOpaque}
              setTexMakeOpaque={setTexMakeOpaque}
              runTextureMutation={runTextureMutation}
              promptAndRunTextureImport={promptAndRunTextureImport}
              handleTextureExport={handleTextureExport}
            />
          ) : activeWmb ? (
            <WmbInspectorView
              activeWmb={activeWmb}
              openCards={openCards}
              toggleCard={toggleCard}
              wmbB2Override={wmbB2Override}
              wmbMatDraft={wmbMatDraft}
              setWmbMatDraft={setWmbMatDraft}
              runWmbMutation={runWmbMutation}
            />
          ) : nodePayload?.view_type === "BXM" ? (
            <BxmInspectorView
              nodePayload={nodePayload}
              bxmTreeDraft={bxmTreeDraft}
              selectedBxmPath={selectedBxmPath}
              openCards={openCards}
              toggleCard={toggleCard}
              updateBxmDraftTree={updateBxmDraftTree}
              commitBxmDraft={commitBxmDraft}
            />
          ) : nodePayload?.view_type === "CLOTH" ? (
            <ClothInspectorView
              nodePayload={nodePayload}
              setNodePayload={setNodePayload}
              openCards={openCards}
              toggleCard={toggleCard}
              runClothMutation={runClothMutation}
            />
          ) : (
            <div>
              <div className="inspector-header">
                <div className="inspector-icon-box">
                  <IconCube size={20} />
                </div>
                <div>
                  <div className="inspector-title">
                    {nodePayload?.title ?? "No asset selected"}
                  </div>
                  <div className="inspector-subtitle">
                    {nodePayload?.spec_line ??
                      (outline
                        ? `${outline.root_format} · ${outline.endian}`
                        : "Open a container from the File Browser and pick an item from the Asset Browser.")}
                  </div>
                </div>
              </div>

              <div className="inspector-card">
                <button className="card-header-toggle" onClick={() => toggleCard("dll_config")}>
                  <span className="card-header-left">
                    <span style={{ color: "var(--brush-muted)" }}>⚙</span>
                    <span>External Libraries (Oodle &amp; NVTT)</span>
                  </span>
                  <IconChevronDown collapsed={!openCards.dll_config} />
                </button>
                {openCards.dll_config && (
                  <div className="card-body">
                    <div className="inspector-row">
                      <span className="inspector-row-label">Oodle DLL</span>
                      <div style={{ display: "flex", gap: 6, flex: 1 }}>
                        <input
                          type="text"
                          className="field-box mono"
                          style={{ flex: 1, minWidth: 0, fontSize: 11 }}
                          value={oodleDllPath}
                          placeholder="oo2core_*_win64.dll (or pure-Rust ooz fallback)"
                          onChange={(e) => setOodleDllPath(e.target.value)}
                        />
                        <button className="pill-button" onClick={handlePickOodleDll}>
                          Browse
                        </button>
                      </div>
                    </div>
                    <div className="inspector-row">
                      <span className="inspector-row-label">NVTT DLL</span>
                      <div style={{ display: "flex", gap: 6, flex: 1 }}>
                        <input
                          type="text"
                          className="field-box mono"
                          style={{ flex: 1, minWidth: 0, fontSize: 11 }}
                          value={nvttDllPath}
                          placeholder="nvtt30205.dll (or pure-Rust BCn/ASTC fallback)"
                          onChange={(e) => setNvttDllPath(e.target.value)}
                        />
                        <button className="pill-button" onClick={handlePickNvttDll}>
                          Browse
                        </button>
                      </div>
                    </div>
                    <div style={{ color: "var(--brush-muted)", fontSize: 11, marginTop: 4 }}>
                      Includes built-in pure-Rust Oodle Kraken (<span className="mono">ooz</span>) fallback
                      decompressor when <span className="mono">oo2core_*_win64.dll</span> is not present.
                    </div>
                  </div>
                )}
              </div>
            </div>
          )}
        </div>
        </>
        )}
      </div>

      <footer className="studio-statusbar">
        <div className="status-left">
          {outline?.dirty ? "● " : ""}
          {statusLeft}
        </div>
        <div className="status-right">
          {activeTexture && <span title="Preview zoom">{Math.round(zoom * 100)}%</span>}
          <span>{outline ? outline.summary_right : "PGTools Studio"}</span>
        </div>
      </footer>

      {contextMenu && (
        <div
          className="context-menu-backdrop"
          onClick={() => setContextMenu(null)}
          onContextMenu={(e) => {
            e.preventDefault();
            setContextMenu(null);
          }}
        >
          <div
            className="context-menu"
            style={{ left: contextMenu.x, top: contextMenu.y }}
            onClick={(e) => e.stopPropagation()}
          >
            {contextMenu.items.map((item, idx) =>
              item.separator ? (
                <div key={idx} className="context-menu-sep" />
              ) : (
                <div
                  key={idx}
                  className="context-menu-item"
                  onMouseEnter={() => setHoverSubmenuIdx(idx)}
                  onClick={() => {
                    if (!item.disabled && item.action) {
                      item.action();
                    }
                  }}
                >
                  <span>{item.label}</span>
                  {item.shortcut && (
                    <span className="context-menu-shortcut">{item.shortcut}</span>
                  )}
                  {item.submenu && <span className="context-menu-shortcut">›</span>}
                  {item.submenu && hoverSubmenuIdx === idx && (
                    <div className="context-submenu">
                      {item.submenu.map((sub, sIdx) =>
                        sub.separator ? (
                          <div key={sIdx} className="context-menu-sep" />
                        ) : (
                          <button
                            key={sIdx}
                            className="context-menu-item"
                            disabled={sub.disabled}
                            onClick={() => {
                              if (sub.action) sub.action();
                            }}
                          >
                            <span>{sub.label}</span>
                          </button>
                        )
                      )}
                    </div>
                  )}
                </div>
              )
            )}
          </div>
        </div>
      )}

      {texModalState && (
        <TextureImportModal
          texModalState={texModalState}
          setTexModalState={setTexModalState}
          activeTexture={activeTexture}
          setTexEncodeFormat={setTexEncodeFormat}
          setTexGenerateMips={setTexGenerateMips}
          setTexSolidify={setTexSolidify}
          setTexMakeOpaque={setTexMakeOpaque}
          runTextureMutation={runTextureMutation}
        />
      )}

      {coreDatModalOpen && (
        <div
          className="context-menu-backdrop"
          style={{
            display: "flex",
            alignItems: "center",
            justifyContent: "center",
            backgroundColor: "rgba(0, 0, 0, 0.55)",
          }}
          onClick={() => setCoreDatModalOpen(false)}
        >
          <div
            className="studio-card"
            style={{
              width: 540,
              maxWidth: "94vw",
              padding: 16,
              display: "flex",
              flexDirection: "column",
              gap: 12,
            }}
            onClick={(e) => e.stopPropagation()}
          >
            <div style={{ fontWeight: 700, fontSize: 14, color: "var(--brush-heading)" }}>
              Configure Global Engine Textures (core.dat / pre_texture.wtb)
            </div>
            <div style={{ fontSize: 12, color: "var(--brush-muted)", lineHeight: 1.45 }}>
              Assign each game&apos;s <span className="mono">core.dat</span> (or extracted{" "}
              <span className="mono">pre_texture.wtb</span>) so global cubemaps and specular LUTs{" "}
              (<span className="mono">0x8000000N</span>) load with authentic stage reflections.
            </div>
            {(
              [
                ["Bayonetta 1 (core.dat / pre_texture.wtb)", coreDatB1, setCoreDatB1],
                ["Bayonetta 2 (core.dat / pre_texture.wta)", coreDatB2, setCoreDatB2],
                ["Bayonetta 3 (core.dat / pre_texture.wta)", coreDatB3, setCoreDatB3],
              ] as const
            ).map(([label, val, setter]) => (
              <div key={label} style={{ display: "flex", flexDirection: "column", gap: 4 }}>
                <span style={{ fontSize: 11.5, fontWeight: 600 }}>{label}</span>
                <div style={{ display: "flex", gap: 6 }}>
                  <input
                    type="text"
                    className="field-box mono"
                    style={{ flex: 1, fontSize: 11.5 }}
                    placeholder="Not configured (using studio procedural fallback)"
                    value={val}
                    onChange={(e) => setter(e.target.value)}
                  />
                  <button
                    type="button"
                    className="pill-button"
                    onClick={async () => {
                      const picked = await invoke<string | null>("pick_file_dialog", {
                        title: `Select ${label}`,
                        extensions: ["dat", "wtb", "wta"],
                      });
                      if (picked) {
                        setter(picked);
                        setWmbDocGeneration((g) => g + 1);
                      }
                    }}
                  >
                    Browse…
                  </button>
                  {val && (
                    <button
                      type="button"
                      className="pill-button"
                      onClick={() => {
                        setter("");
                        setWmbDocGeneration((g) => g + 1);
                      }}
                    >
                      Clear
                    </button>
                  )}
                </div>
              </div>
            ))}
            <div style={{ display: "flex", justifyContent: "flex-end", gap: 8, marginTop: 4 }}>
              <button
                type="button"
                className="pill-button accent"
                onClick={() => {
                  setCoreDatModalOpen(false);
                  setWmbDocGeneration((g) => g + 1);
                }}
              >
                Done
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
