import React, { useCallback, useEffect, useRef, useState } from "react";
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
  NodeInspectPayload,
  OodleValidationReport,
  PkzCompressionModalState,
  PkzConflictModalState,
  PointerDragState,
  StudioSettings,
  TexModalState,
  TextureNodeView,
  ThemeMode,
  Wmb3Material,
  WmbSectionView,
} from "./types";
import {
  isVirtualSubNode,
  parseReorderableNode,
} from "./utils/format";
import {
  bxmTreeToXmlDocument,
  getBxmNodeByPath,
} from "./bxm/xml";
import { TextureImportModal } from "./editors/TextureEditor";
import {
  BatchDatAlignModal,
  BatchRecompressModal,
  BatchReplaceModal,
  DatAlignmentSettingsModal,
  ExportTexturesModal,
  NewBankModal,
} from "./modals/TextureBatchModals";
import { createMenuBuilders } from "./menus/contextMenus";
import {
  IconCheck,
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
import {
  clampPkzLevel,
  normalizePkzCodec,
  useStudioSettings,
} from "./hooks/useStudioSettings";
import { useResizableTableColumns } from "./hooks/useResizableTableColumns";
import { useFileBrowser } from "./hooks/useFileBrowser";
import { useDocumentSession } from "./hooks/useDocumentSession";
import { useEditorDrafts } from "./hooks/useEditorDrafts";
import { useMeshUi } from "./hooks/useMeshUi";
import { FileBrowserPanel } from "./panels/FileBrowserPanel";
import { AssetBrowserPanel } from "./panels/AssetBrowserPanel";
import { DocumentEditors } from "./panels/DocumentEditors";

export default function App() {
  const settings = useStudioSettings();
  const {
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
    assetBrowserZoom,
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
    setAssetBrowserZoom,
    buildStudioSettingsPayload,
    applyLoadedSettings,
    markLoaded,
  } = settings;

  const [draggingSplit, setDraggingSplit] = useState<"left" | "right" | "left-row" | null>(null);
  const leftColRef = useRef<HTMLDivElement | null>(null);
  const assetBrowserCardRef = useRef<HTMLDivElement | null>(null);
  const assetSearchInputRef = useRef<HTMLInputElement | null>(null);

  const [wmbDocGeneration, setWmbDocGeneration] = useState<number>(0);
  const [coreDatModalOpen, setCoreDatModalOpen] = useState<boolean>(false);
  const [batchReplaceNodeId, setBatchReplaceNodeId] = useState<string | null>(null);
  const [batchRecompressOpen, setBatchRecompressOpen] = useState<boolean>(false);
  const [batchDatAlignOpen, setBatchDatAlignOpen] = useState<boolean>(false);
  const [datAlignSettingsOpen, setDatAlignSettingsOpen] = useState<boolean>(false);
  const [exportFolderModalOpen, setExportFolderModalOpen] = useState<boolean>(false);
  const [newBankModalOpen, setNewBankModalOpen] = useState<boolean>(false);
  const [pkzCompModal, setPkzCompModal] = useState<PkzCompressionModalState | null>(null);
  const [pkzConflictModal, setPkzConflictModal] = useState<PkzConflictModalState | null>(null);
  const [oodleValidation, setOodleValidation] = useState<OodleValidationReport | null>(null);
  const [busyLabel, setBusyLabel] = useState<string | null>(null);

  const [statusLeft, setStatusLeft] = useState<string>("Ready");
  const inspectSeqRef = useRef<number>(0);

  const fb = useFileBrowser({
    hideUnsupported,
    fileSortCol,
    fileSortAsc,
    setFileSortCol,
    setFileSortAsc,
    onStatus: setStatusLeft,
  });
  const {
    currentDir,
    currentDirRef,
    selectedFilePath,
    setSelectedFilePath,
    selectedFilePaths,
    setSelectedFilePaths,
    fsClipboard,
    setFsClipboard,
    filteredFiles,
    loadDirectory,
  } = fb;

  // Texture Replacement & Compression Modal Dialog state
  const [texModalState, setTexModalState] = useState<TexModalState | null>(null);

  // PKZ Bulk Compression & WMB Mesh->Batch->LOD Tree state
  const meshUi = useMeshUi();
  const {
    pkzBulkCodec,
    setPkzBulkCodec,
    pkzSelectedIndices,
    setPkzSelectedIndices,
    setPkzAnchorIndex,
  } = meshUi;

  const docSession = useDocumentSession();
  const {
    outline,
    expandedNodes,
    selectedNodeId,
    assetSearch,
    outlineRef,
    setOutline,
    setExpandedNodes,
    setSelectedNodeId,
    setAssetSearch,
    confirmDiscardIfDirty,
    expandForOpen,
  } = docSession;

  const selectedNodeIdRef = useRef<string>(selectedNodeId);
  selectedNodeIdRef.current = selectedNodeId;

  useEffect(() => {
    const onBeforeUnload = (e: BeforeUnloadEvent) => {
      if (outlineRef.current?.dirty) {
        e.preventDefault();
        e.returnValue = "";
      }
    };
    window.addEventListener("beforeunload", onBeforeUnload);
    return () => window.removeEventListener("beforeunload", onBeforeUnload);
  }, [outlineRef]);

  // Pointer-based Drag-and-Drop (reordering + drag-out to Explorer) & External File Drop
  const [pointerDrag, setPointerDrag] = useState<PointerDragState | null>(null);
  const [externalDragHover, setExternalDragHover] = useState<boolean>(false);

  const [nodePayload, setNodePayload] = useState<NodeInspectPayload | null>(null);
  const nodePayloadRef = useRef<NodeInspectPayload | null>(null);
  nodePayloadRef.current = nodePayload;
  const panStartRef = useRef<{ x: number; y: number }>({ x: 0, y: 0 });
  const [materialClipboard, setMaterialClipboard] =
    useState<MaterialClipboardState | null>(null);

  const drafts = useEditorDrafts();
  const {
    hashDraft,
    setHashDraft,
    flagsDraft,
    setFlagsDraft,
    mipIndex,
    setMipIndex,
    layerIndex,
    setLayerIndex,
    channelMode,
    setChannelMode,
    opaquePreview,
    setOpaquePreview,
    texEncodeFormat,
    setTexEncodeFormat,
    texQuality,
    setTexQuality,
    texGenerateMips,
    setTexGenerateMips,
    texSolidify,
    setTexSolidify,
    texMakeOpaque,
    setTexMakeOpaque,
    texSyncFlags,
    setTexSyncFlags,
    zoom,
    setZoom,
    pan,
    setPan,
    isPanning,
    setIsPanning,
    wmbMatDraft,
    setWmbMatDraft,
    wmb3MatDraft,
    setWmb3MatDraft,
    wmbRawMode,
    setWmbRawMode,
    wmbB2Override,
    setWmbB2Override,
    wmbHeaderFlagsDraft,
    setWmbHeaderFlagsDraft,
    wmbBboxDraft,
    setWmbBboxDraft,
    bxmTreeDraft,
    setBxmTreeDraft,
    bxmXmlDraft,
    setBxmXmlDraft,
    selectedBxmPath,
    setSelectedBxmPath,
    bxmViewMode,
    setBxmViewMode,
    bxmExpandedPaths,
    setBxmExpandedPaths,
    updateBxmDraftTree,
    switchBxmViewMode: switchBxmViewModeInner,
    estDraft,
    setEstDraft,
    csvHeadersDraft,
    setCsvHeadersDraft,
    csvRowsDraft,
    setCsvRowsDraft,
    structDraft,
    setStructDraft,
    selectedMotTrackIdx,
    setSelectedMotTrackIdx,
    centerTab,
    setCenterTab,
    openCards,
    setOpenCards,
    toggleCard,
    hydrateFromInspect,
  } = drafts;

  function switchBxmViewMode(nextMode: "tree" | "table" | "xml") {
    const err = switchBxmViewModeInner(nextMode);
    if (err) setStatusLeft(err);
  }

  // Menus & Status (with appearing & disappearing animation transitions)
  const [activeTopMenu, setActiveTopMenu] = useState<"file" | "tools" | "help" | null>(null);
  const [closingTopMenu, setClosingTopMenu] = useState<"file" | "tools" | "help" | null>(null);
  const prevTopMenuRef = useRef<"file" | "tools" | "help" | null>(null);

  const [contextMenu, setContextMenu] = useState<ContextMenuState | null>(null);
  const [closingContextMenu, setClosingContextMenu] = useState<ContextMenuState | null>(null);
  const prevContextMenuRef = useRef<ContextMenuState | null>(null);

  const [hoverSubmenuIdx, setHoverSubmenuIdx] = useState<number | null>(null);
  const [closingSubmenuIdx, setClosingSubmenuIdx] = useState<number | null>(null);
  const prevSubmenuIdxRef = useRef<number | null>(null);

  useEffect(() => {
    if (hoverSubmenuIdx !== null) {
      setClosingSubmenuIdx(null);
      prevSubmenuIdxRef.current = hoverSubmenuIdx;
    } else if (prevSubmenuIdxRef.current !== null) {
      const prev = prevSubmenuIdxRef.current;
      prevSubmenuIdxRef.current = null;
      setClosingSubmenuIdx(prev);
      const timer = window.setTimeout(() => {
        setClosingSubmenuIdx((cur) => (cur === prev ? null : cur));
      }, 115);
      return () => window.clearTimeout(timer);
    }
  }, [hoverSubmenuIdx]);

  const displayedSubmenuIdx = hoverSubmenuIdx ?? closingSubmenuIdx;
  const isSubmenuClosing = hoverSubmenuIdx === null && closingSubmenuIdx !== null;

  useEffect(() => {
    if (activeTopMenu !== null) {
      setClosingTopMenu(null);
      prevTopMenuRef.current = activeTopMenu;
      setHoverSubmenuIdx(null);
    } else if (prevTopMenuRef.current !== null) {
      const prev = prevTopMenuRef.current;
      prevTopMenuRef.current = null;
      setClosingTopMenu(prev);
      const timer = window.setTimeout(() => {
        setClosingTopMenu((cur) => (cur === prev ? null : cur));
      }, 115);
      return () => window.clearTimeout(timer);
    }
  }, [activeTopMenu]);

  useEffect(() => {
    if (contextMenu !== null) {
      setClosingContextMenu(null);
      prevContextMenuRef.current = contextMenu;
    } else if (prevContextMenuRef.current !== null) {
      const prev = prevContextMenuRef.current;
      prevContextMenuRef.current = null;
      setClosingContextMenu(prev);
      const timer = window.setTimeout(() => {
        setClosingContextMenu((cur) => (cur === prev ? null : cur));
      }, 115);
      return () => window.clearTimeout(timer);
    }
  }, [contextMenu]);

  const displayedTopMenu = activeTopMenu ?? closingTopMenu;
  const isTopMenuClosing = activeTopMenu === null && closingTopMenu !== null;
  const displayedContextMenu = contextMenu ?? closingContextMenu;
  const isContextMenuClosing = contextMenu === null && closingContextMenu !== null;

  // Suppress generic browser right-click context menu globally
  useEffect(() => {
    const suppressNativeContextMenu = (e: MouseEvent) => {
      e.preventDefault();
    };
    window.addEventListener("contextmenu", suppressNativeContextMenu);
    return () => window.removeEventListener("contextmenu", suppressNativeContextMenu);
  }, []);

  useResizableTableColumns([
    nodePayload?.node_id,
    nodePayload?.view_type,
    nodePayload?.wmb?.section,
    bxmViewMode,
    selectedBxmPath,
    centerTab,
  ]);

  const refreshOodleValidation = useCallback(async (customPath?: string) => {
    try {
      const report = await invoke<OodleValidationReport>("validate_oodle_dll", {
        path: customPath ?? null,
      });
      setOodleValidation(report);
      return report;
    } catch {
      return null;
    }
  }, []);

  const openPkzCompressionModal = useCallback(
    (
      title: string,
      subtitle: string,
      initialCodec: string | undefined,
      onConfirm: (
        compression: "None" | "ZStandard" | "OodleKraken",
        level: number,
        forceCompression: boolean
      ) => Promise<void> | void
    ) => {
      const codec = normalizePkzCodec(initialCodec ?? pkzDefaultCompression);
      const level = clampPkzLevel(
        codec,
        codec === pkzDefaultCompression
          ? pkzDefaultLevel
          : codec === "OodleKraken"
          ? 4
          : 3
      );
      void refreshOodleValidation();
      setPkzCompModal({
        title,
        subtitle,
        compression: codec,
        level,
        forceCompression: false,
        onConfirm,
      });
    },
    [pkzDefaultCompression, pkzDefaultLevel, refreshOodleValidation]
  );

  const handleExpandAll = useCallback(() => {
    if (!outlineRef.current) return;
    const next: Record<string, boolean> = {
      root: true,
      "root:dat": true,
      "root:dtt": true,
    };
    const walk = (nodes: AssetTreeNode[]) => {
      for (const n of nodes) {
        if (n.children && n.children.length > 0) {
          next[n.id] = true;
          walk(n.children);
        }
      }
    };
    walk(outlineRef.current.tree);
    setExpandedNodes(next);
  }, [outlineRef, setExpandedNodes]);

  const handleCollapseAll = useCallback(() => {
    setExpandedNodes({
      root: true,
      "root:dat": true,
      "root:dtt": true,
    });
  }, [setExpandedNodes]);

  // Listen for external OS file drops (`tauri://drag-drop`) anywhere on the window
  const dropHandlerRef = useRef<(paths: string[]) => Promise<void>>(async () => {});
  useEffect(() => {
    let unlistenDrop: (() => void) | undefined;
    let unlistenEnter: (() => void) | undefined;
    let unlistenLeave: (() => void) | undefined;

    (async () => {
      try {
        unlistenDrop = await listen<{ paths?: string[] }>("tauri://drag-drop", (event) => {
          setExternalDragHover(false);
          const paths = event.payload?.paths;
          if (Array.isArray(paths) && paths.length > 0) {
            void dropHandlerRef.current(paths);
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
        applyLoadedSettings(s);
        markLoaded();
        await loadDirectory(s.last_dir || undefined);
      } catch {
        markLoaded();
        await loadDirectory();
      }
    })();
    // loadDirectory is stable enough for mount-only; omit from deps intentionally
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [applyLoadedSettings, markLoaded]);

  // Persist StudioSettings whenever preferences change (debounced)
  useEffect(() => {
    if (!settingsLoaded) return;
    const payload = buildStudioSettingsPayload(currentDir);
    const timer = window.setTimeout(() => {
      invoke<StudioSettings>("save_studio_settings", { settings: payload })
        .then((saved) => {
          setOodleConfigured(Boolean(saved.oodle_configured));
          setNvttConfigured(Boolean(saved.nvtt_configured));
        })
        .catch(() => {
          setOodleConfigured(false);
          setNvttConfigured(false);
        });
    }, 250);
    return () => window.clearTimeout(timer);
  }, [
    buildStudioSettingsPayload,
    settingsLoaded,
    currentDir,
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
    assetBrowserZoom,
  ]);

  const shortcutHandlersRef = useRef<{
    openFile: () => void;
    save: () => void;
    saveAs: () => void;
    undo: () => void;
    redo: () => void;
    focusSearch: () => void;
    selectAllPkz: () => void;
    renameSelected: () => void;
    deleteSelected: () => void;
  }>({
    openFile: () => {},
    save: () => {},
    saveAs: () => {},
    undo: () => {},
    redo: () => {},
    focusSearch: () => {},
    selectAllPkz: () => {},
    renameSelected: () => {},
    deleteSelected: () => {},
  });

  // Global keyboard shortcuts (Ctrl+O, Ctrl+S, Ctrl+Z, Ctrl+Y, Ctrl+F, Ctrl+A, F2, Delete)
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
      } else if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "f") {
        e.preventDefault();
        shortcutHandlersRef.current.focusSearch();
      } else if (!inEditableField && (e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "a") {
        if (nodePayloadRef.current?.format === "PKZ") {
          e.preventDefault();
          shortcutHandlersRef.current.selectAllPkz();
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
      } else if (!inEditableField && e.key === "F2") {
        e.preventDefault();
        shortcutHandlersRef.current.renameSelected();
      } else if (!inEditableField && e.key === "Delete") {
        e.preventDefault();
        shortcutHandlersRef.current.deleteSelected();
      } else if (e.key === "Escape") {
        setContextMenu(null);
        setActiveTopMenu(null);
        setPointerDrag(null);
        setPkzCompModal(null);
        setPkzConflictModal(null);
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
  }, [draggingSplit, setAssetRatio, setLeftWidth, setRightWidth]);

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
            void runContainerMutation(
              pointerDrag.sourceId,
              "move_pkz_entry_to_folder",
              folderPrefix,
              {
                target_folder: folderPrefix,
              }
            );
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

  async function openDocumentByPath(path: string, skipDirtyPrompt = false) {
    if (!skipDirtyPrompt && !confirmDiscardIfDirty(`open "${path.split(/[\\/]/).pop() || path}"`)) {
      return;
    }
    try {
      setBusyLabel(`Loading ${path.split(/[\\/]/).pop() || path}...`);
      setStatusLeft(`Loading ${path}...`);
      const opened = await invoke<DocumentOutline>("open_document", { path });
      setOutline(opened);
      setSelectedFilePath(path);
      setPkzSelectedIndices([]);
      setWmbB2Override(null);
      refreshRecents();

      const { nextExpanded, defaultTargetId } = expandForOpen(opened);
      setWmbDocGeneration((g) => g + 1);
      setExpandedNodes(nextExpanded);
      setMipIndex(0);
      setLayerIndex(0);
      await selectAndInspectNode(
        defaultTargetId,
        0,
        channelMode,
        opaquePreview,
        null,
        opened.file_name
      );
      setStatusLeft(`Loaded ${opened.file_name}`);
    } catch (err) {
      setStatusLeft(`Failed to open: ${String(err)}`);
    } finally {
      setBusyLabel(null);
    }
  }

  dropHandlerRef.current = async (paths: string[]) => {
    const pkzArchives = paths.filter((p) => p.toLowerCase().endsWith(".pkz"));
    const looseItems = paths.filter((p) => !p.toLowerCase().endsWith(".pkz"));

    if (
      pkzArchives.length === 0 &&
      looseItems.length > 0 &&
      (outlineRef.current?.root_format === "PKZ" ||
        nodePayloadRef.current?.format === "PKZ")
    ) {
      const curSel = selectedNodeIdRef.current;
      const curPayload = nodePayloadRef.current;
      const folderPrefix = curSel.includes("pkzfolder:")
        ? curSel.split("pkzfolder:").pop() || ""
        : curPayload?.folder_prefix ?? "";
      const rawTargetNode =
        curPayload?.format === "PKZ" ? curPayload.node_id : "root";
      const basePkzNodeId = rawTargetNode.includes("/pkzfolder:")
        ? rawTargetNode.split("/pkzfolder:")[0]
        : rawTargetNode.startsWith("pkzfolder:")
        ? "root"
        : rawTargetNode;
      openPkzCompressionModal(
        `Add ${looseItems.length} Dropped Item(s) as`,
        folderPrefix ? `Adding into '${folderPrefix}/'` : "Adding into PKZ root",
        pkzDefaultCompression,
        async (compression, level, forceCompression) => {
          await runContainerMutation(basePkzNodeId, "add_files", "", {
            paths: looseItems,
            folder_prefix: folderPrefix || null,
            compression,
            level,
            force_compression: forceCompression,
          });
        }
      );
      return;
    }

    if (paths[0]) {
      await openDocumentByPath(paths[0]);
    }
  };

  async function selectAndInspectNode(
    nodeId: string,
    mip = mipIndex,
    channel = channelMode,
    opaque = opaquePreview,
    b2Override: boolean | null = wmbB2Override,
    docLabel?: string,
    layer = layerIndex
  ) {
    const reqSeq = ++inspectSeqRef.current;
    if (nodeId !== selectedNodeId) {
      setLayerIndex(0);
      layer = 0;
    }
    setSelectedNodeId(nodeId);
    try {
      const res = await invoke<NodeInspectPayload>("inspect_asset_node", {
        nodeId,
        mipIndex: mip,
        layerIndex: layer,
        channel,
        opaque,
        b2Override,
      });
      if (reqSeq !== inspectSeqRef.current) return;
      setNodePayload(res);
      hydrateFromInspect(res);
      if (docLabel) {
        setStatusLeft(`Loaded ${docLabel}`);
      }
    } catch (err) {
      if (reqSeq === inspectSeqRef.current) {
        setStatusLeft(`Inspect error: ${String(err)}`);
      }
    }
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
    if (nodePayload?.view_type === "TEXTURE" && nodePayload.texture) {
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

  async function handlePackFolderToPkz(sourceFolder?: string) {
    setActiveTopMenu(null);
    const srcFolder =
      sourceFolder ??
      (await invoke<string | null>("pick_folder_dialog", {
        title: "Choose the folder to pack into a .PKZ archive",
      }));
    if (!srcFolder) return;

    const folderStem =
      srcFolder.replace(/[\\/]+$/, "").split(/[\\/]/).pop() || "archive";

    openPkzCompressionModal(
      "Pack Folder as .PKZ",
      `Source folder: ${srcFolder}`,
      pkzDefaultCompression,
      async (compression, level, forceCompression) => {
        const dest = await invoke<string | null>("save_file_dialog", {
          title: "Save the new .PKZ archive",
          defaultName: `${folderStem}.pkz`,
        });
        if (!dest) return;

        const runPack = async (
          codecToUse: "None" | "ZStandard" | "OodleKraken",
          allowFallback = false
        ) => {
          try {
            setBusyLabel(`Packing ${folderStem}.pkz...`);
            setStatusLeft(`Packing folder ${srcFolder} into ${dest}...`);
            const msg = await invoke<string>("pack_folder_to_pkz", {
              sourceDir: srcFolder,
              destPath: dest,
              compression: codecToUse,
              level,
              forceCompression,
              allowZstdFallback: allowFallback,
            });
            setPkzDefaultCompression(codecToUse);
            setPkzDefaultLevel(level);
            await loadDirectory(currentDirRef.current);
            await openDocumentByPath(dest, true);
            setStatusLeft(msg);
          } catch (err) {
            const errStr = String(err);
            if (errStr.includes("OODLE_DLL_MISSING")) {
              const useZstd = window.confirm(
                "OodleKraken compression requires oo2core_*_win64.dll with OodleLZ_Compress.\n\nClick OK to pack using ZStandard instead, or Cancel to locate oo2core_*_win64.dll."
              );
              if (useZstd) {
                await runPack("ZStandard", true);
              } else {
                await handlePickOodleDll();
              }
              return;
            }
            setStatusLeft(`Pack folder failed: ${errStr}`);
          } finally {
            setBusyLabel(null);
          }
        };

        await runPack(compression);
      }
    );
  }

  async function handleMenuSave() {
    setActiveTopMenu(null);
    if (!outline) return;
    try {
      setBusyLabel(`Saving ${outline.file_name}...`);
      const updated = await invoke<DocumentOutline>("save_document", { outputPath: null });
      setOutline(updated);
      await selectAndInspectNode(selectedNodeId);
      setStatusLeft(`Saved ${updated.file_name}`);
      loadDirectory(currentDirRef.current);
    } catch (err) {
      setStatusLeft(`Save error: ${String(err)}`);
    } finally {
      setBusyLabel(null);
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
      setBusyLabel(`Saving to ${dest.split(/[\\/]/).pop() || dest}...`);
      const updated = await invoke<DocumentOutline>("save_document", { outputPath: dest });
      setOutline(updated);
      await selectAndInspectNode(selectedNodeId);
      setStatusLeft(`Saved to ${dest}`);
      loadDirectory(currentDirRef.current);
    } catch (err) {
      setStatusLeft(`Save As error: ${String(err)}`);
    } finally {
      setBusyLabel(null);
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
    focusSearch: () => {
      assetSearchInputRef.current?.focus();
      assetSearchInputRef.current?.select();
    },
    selectAllPkz: () => {
      if (nodePayload?.format === "PKZ" && Array.isArray(nodePayload.entries)) {
        setPkzSelectedIndices(nodePayload.entries.map((e: any) => e.index));
      }
    },
    renameSelected: () => {
      const leafSeg = selectedNodeId.split("/").pop() || "";
      if (selectedNodeId.includes("pkzfolder:")) {
        const folderPrefix = selectedNodeId.split("pkzfolder:").pop() || "";
        const nextPrefix = window.prompt("Rename PKZ folder:", folderPrefix);
        if (nextPrefix !== null && nextPrefix.trim() !== folderPrefix) {
          void runContainerMutation(selectedNodeId, "rename_pkz_folder", nextPrefix.trim(), {
            folder_prefix: folderPrefix,
            new_prefix: nextPrefix.trim(),
          });
        }
      } else if (leafSeg.startsWith("pkz:")) {
        const nextName = window.prompt("Rename PKZ entry:", nodePayload?.title || "");
        if (nextName && nextName.trim()) {
          void runContainerMutation(selectedNodeId, "rename_entry", nextName.trim(), {
            new_name: nextName.trim(),
          });
        }
      }
    },
    deleteSelected: () => {
      const leafSeg = selectedNodeId.split("/").pop() || "";
      if (nodePayload?.format === "PKZ" && pkzSelectedIndices.length > 0) {
        const baseContainerId = nodePayload.node_id.includes("/pkzfolder:")
          ? nodePayload.node_id.split("/pkzfolder:")[0]
          : nodePayload.node_id.startsWith("pkzfolder:")
          ? "root"
          : nodePayload.node_id;
        const ok = window.confirm(
          `Delete ${pkzSelectedIndices.length} selected file(s) from the PKZ archive?\n\nThe change is staged until you save.`
        );
        if (!ok) return;
        void runContainerMutation(baseContainerId, "remove_pkz_entries", "", {
          entry_indices: pkzSelectedIndices,
        });
      } else if (selectedNodeId.includes("pkzfolder:")) {
        const folderPrefix = selectedNodeId.split("pkzfolder:").pop() || "";
        const ok = window.confirm(
          `Delete the folder '${folderPrefix}' and all files under it?\n\nThe change is staged until you save.`
        );
        if (ok) {
          void runContainerMutation(selectedNodeId, "remove_pkz_folder", "", {
            folder_prefix: folderPrefix,
          });
        }
      } else if (leafSeg.startsWith("pkz:")) {
        const ok = window.confirm(
          `Delete '${nodePayload?.title || selectedNodeId}'?\n\nThe change is staged until you save.`
        );
        if (ok) {
          void runContainerMutation(selectedNodeId, "remove_entry");
        }
      }
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
      try {
        const saved = await invoke<StudioSettings>("save_studio_settings", {
          settings: buildStudioSettingsPayload(currentDir, { oodle_dll_path: picked }),
        });
        setOodleConfigured(Boolean(saved.oodle_configured ?? true));
      } catch {
        setOodleConfigured(true);
      }
      await refreshOodleValidation(picked);
      setStatusLeft(`Configured Oodle DLL: ${picked}`);
      if (outline && !outline.dirty) {
        await openDocumentByPath(outline.path, true);
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
      try {
        const saved = await invoke<StudioSettings>("save_studio_settings", {
          settings: buildStudioSettingsPayload(currentDir, { nvtt_dll_path: picked }),
        });
        setNvttConfigured(Boolean(saved.nvtt_configured ?? true));
      } catch {
        setNvttConfigured(true);
      }
      setStatusLeft(`Configured NVTT DLL: ${picked}`);
    }
  }

  async function runTextureMutation(nodeId: string, action: string, payload: any = {}) {
    try {
      const fullPayload = {
        encode_format: texEncodeFormat,
        quality: texQuality,
        generate_mips: texGenerateMips,
        solidify: texSolidify,
        make_opaque: texMakeOpaque,
        sync_flags: texSyncFlags,
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
    const defaultStem = (path.split(/[\\/]/).pop() || "")
      .replace(/\.[^.]+$/, "")
      .replace(/^tex\d+_0x[0-9a-fA-F]+$/i, "");
    setTexModalState({
      nodeId,
      action,
      path,
      fileName,
      encodeFormat: action === "add_texture" && texEncodeFormat === "Auto" ? "BC3_UNORM" : texEncodeFormat,
      quality: texQuality,
      generateMips: texGenerateMips,
      solidify: texSolidify,
      makeOpaque: texMakeOpaque,
      syncFlags: texSyncFlags,
      textureName: action === "add_texture" ? defaultStem : undefined,
    });
  }

  async function handleTextureExport(
    nodeId: string,
    scope: "single" | "bank" | "container",
    format: "png" | "dds" | "raw"
  ) {
    setContextMenu(null);
    try {
      if (scope === "single") {
        const rawExt = (activeTexture?.container_ext || ".xt1").replace(/^\./, "");
        const ext = format === "png" ? "png" : format === "dds" ? "dds" : rawExt;
        const hashSuffix =
          activeTexture?.has_crc32_flag && activeTexture?.texture_id_hex
            ? `_${activeTexture.texture_id_hex}`
            : "";
        const defName = activeTexture
          ? `tex${activeTexture.index}${hashSuffix}.${ext}`
          : `texture.${ext}`;
        const dest = await invoke<string | null>("save_file_dialog", {
          title: `Export Texture (.${ext})`,
          defaultName: defName,
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
      setBusyLabel(`Working (${action})...`);
      const updated = await invoke<DocumentOutline>("mutate_container_node", {
        nodeId,
        action,
        targetPath,
        payload,
      });
      setOutline(updated);
      if (
        action === "remove_entry" ||
        action === "remove_pkz_entries" ||
        action === "remove_pkz_folder" ||
        action === "revert_pkz_entries" ||
        action === "add_files"
      ) {
        setPkzSelectedIndices([]);
        setPkzAnchorIndex(null);
      }
      if (action === "remove_pkz_folder") {
        const parent = nodeId.includes("/pkzfolder:")
          ? nodeId.split("/pkzfolder:")[0]
          : "root";
        await selectAndInspectNode(parent || "root");
      } else if (action === "remove_entry") {
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
          "OodleKraken compression requires oo2core_*_win64.dll with OodleLZ_Compress.\n\nClick OK to compress using ZStandard instead, or Cancel to locate oo2core_*_win64.dll."
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
      if (msg.includes("PKZ_COLLISION:")) {
        const jsonPart = msg.slice(msg.indexOf("PKZ_COLLISION:") + "PKZ_COLLISION:".length);
        try {
          const info = JSON.parse(jsonPart);
          const count = Number(info.count ?? 1);
          const paths: string[] = Array.isArray(info.paths) ? info.paths : [];
          const newCount = Number(info.new_count ?? 0);
          setPkzConflictModal({
            kind: "collision",
            title: "Name Collision in PKZ Archive",
            message:
              count === 1
                ? `1 entry already exists in the PKZ archive:\n${paths[0] ?? ""}\n\nOverwrite it?`
                : `${count} entries already exist in the PKZ archive.${
                    newCount > 0
                      ? ` Choosing Skip Existing adds only the ${newCount} new file(s).`
                      : ""
                  }`,
            paths: paths.slice(0, 10),
            onChoice: async (policy) => {
              setPkzConflictModal(null);
              await runContainerMutation(nodeId, action, targetPath, {
                ...(payload || {}),
                collision_policy: policy,
              });
            },
          });
          return;
        } catch {
          // fall through
        }
      }
      if (msg.includes("EXTRACT_OVERWRITE:")) {
        const countStr = msg
          .slice(msg.indexOf("EXTRACT_OVERWRITE:") + "EXTRACT_OVERWRITE:".length)
          .trim();
        const count = parseInt(countStr || "1", 10) || 1;
        setPkzConflictModal({
          kind: "extract_overwrite",
          title: "Files Already Exist in Destination Folder",
          message:
            count === 1
              ? "1 file in the destination folder already exists."
              : `${count} files in the destination folder already exist.`,
          onChoice: async (policy) => {
            setPkzConflictModal(null);
            await runContainerMutation(nodeId, action, targetPath, {
              ...(payload || {}),
              overwrite_policy: policy,
            });
            setStatusLeft(`Extraction completed (${policy}) to ${targetPath}`);
            loadDirectory(currentDirRef.current);
          },
        });
        return;
      }
      setStatusLeft(`Container operation failed: ${msg}`);
    } finally {
      setBusyLabel(null);
    }
  }

  const activeTexture: TextureNodeView | null =
    nodePayload?.view_type === "TEXTURE" ? (nodePayload.texture ?? null) : null;
  const activeWmb: WmbSectionView | null =
    nodePayload?.view_type === "WMB" ? (nodePayload.wmb ?? null) : null;

  const { buildAssetNodeContextMenu, buildFileBrowserContextMenu } = createMenuBuilders(
    {
      activeTexture,
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
      openBatchReplaceModal: (targetNodeId: string) => setBatchReplaceNodeId(targetNodeId),
      runTextureMutation,
      runClothMutation,
      openPkzCompressionModal,
      onExpandAll: handleExpandAll,
      onCollapseAll: handleCollapseAll,
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
      onPackFolderToPkz: handlePackFolderToPkz,
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
        <div className="external-drop-overlay">
          {outline?.root_format === "PKZ"
            ? "Drop .pkz to open, or drop files/folders to add into current PKZ"
            : "Drop PlatinumGames file to open in Studio"}
        </div>
      )}
      <header className="studio-menubar" onClick={(e) => e.stopPropagation()}>
        <div
          style={{ position: "relative" }}
          onMouseEnter={() => {
            if (activeTopMenu !== null && activeTopMenu !== "file") {
              setActiveTopMenu("file");
            }
          }}
        >
          <button
            className={`menu-top-button ${activeTopMenu === "file" ? "active" : ""}`}
            onClick={() => {
              setContextMenu(null);
              setActiveTopMenu(activeTopMenu === "file" ? null : "file");
            }}
            onMouseEnter={() => {
              if (activeTopMenu !== null && activeTopMenu !== "file") {
                setActiveTopMenu("file");
              }
            }}
          >
            File
          </button>
          {displayedTopMenu === "file" && (
            <div
              className={`context-menu top-menu-dropdown ${isTopMenuClosing ? "closing" : ""}`}
              style={{ top: 28, left: 0 }}
            >
              <button className="context-menu-item" onClick={handleMenuOpenFile}>
                <span>Open File...</span>
                <span className="context-menu-shortcut">Ctrl+O</span>
              </button>
              <button className="context-menu-item" onClick={handleMenuOpenFolder}>
                <span>Open Folder...</span>
              </button>
              <button
                className="context-menu-item"
                onClick={() => {
                  setActiveTopMenu(null);
                  setNewBankModalOpen(true);
                }}
              >
                <span>New empty bank...</span>
              </button>
              <div
                className="context-menu-item"
                onMouseEnter={() => setHoverSubmenuIdx(99)}
                onMouseLeave={() => setHoverSubmenuIdx(null)}
              >
                <span>Open Recent</span>
                <span className="context-menu-shortcut">›</span>
                {displayedSubmenuIdx === 99 && (
                  <div className={`context-submenu ${isSubmenuClosing ? "closing" : ""}`}>
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
              <div className="context-menu-sep" />
              <button
                className="context-menu-item"
                onClick={() => void handlePackFolderToPkz()}
              >
                <span>PKZ from Folder...</span>
              </button>
            </div>
          )}
        </div>

        <div
          style={{ position: "relative" }}
          onMouseEnter={() => {
            if (activeTopMenu !== null && activeTopMenu !== "tools") {
              setActiveTopMenu("tools");
            }
          }}
        >
          <button
            className={`menu-top-button ${activeTopMenu === "tools" ? "active" : ""}`}
            onClick={() => {
              setContextMenu(null);
              setActiveTopMenu(activeTopMenu === "tools" ? null : "tools");
            }}
            onMouseEnter={() => {
              if (activeTopMenu !== null && activeTopMenu !== "tools") {
                setActiveTopMenu("tools");
              }
            }}
          >
            Tools
          </button>
          {displayedTopMenu === "tools" && (
            <div
              className={`context-menu top-menu-dropdown ${isTopMenuClosing ? "closing" : ""}`}
              style={{ top: 28, left: 0, minWidth: 280 }}
            >
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
              <button
                className="context-menu-item"
                onClick={() => {
                  setActiveTopMenu(null);
                  setExportFolderModalOpen(true);
                }}
              >
                <span>Export textures from folder...</span>
              </button>
              <button
                className="context-menu-item"
                onClick={() => {
                  setActiveTopMenu(null);
                  setBatchRecompressOpen(true);
                }}
              >
                <span>Batch recompress (ASTC to BC)...</span>
              </button>
              <button
                className="context-menu-item"
                onClick={() => {
                  setActiveTopMenu(null);
                  setBatchDatAlignOpen(true);
                }}
              >
                <span>Batch DAT custom align...</span>
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
                title={oodleDllPath ? `Configured: ${oodleDllPath}` : oodleConfigured ? "Oodle DLL detected" : "Not configured"}
                onClick={() => {
                  setActiveTopMenu(null);
                  handlePickOodleDll();
                }}
              >
                <span>Configure Oodle Kraken DLL (oo2core)...</span>
                {oodleConfigured && (
                  <span className="menu-check-badge" aria-label="Configured">
                    <IconCheck />
                  </span>
                )}
              </button>
              <button
                className="context-menu-item"
                title={nvttDllPath ? `Configured: ${nvttDllPath}` : nvttConfigured ? "NVTT DLL detected" : "Not configured"}
                onClick={() => {
                  setActiveTopMenu(null);
                  handlePickNvttDll();
                }}
              >
                <span>Configure NVIDIA Texture Tools DLL (nvtt30205)...</span>
                {nvttConfigured && (
                  <span className="menu-check-badge" aria-label="Configured">
                    <IconCheck />
                  </span>
                )}
              </button>
              <div className="context-menu-sep" />
              <button
                className="context-menu-item"
                title={
                  forceCustomDatAlignment
                    ? `Forcing custom DAT alignment: ${customDatAlignment} B (0x${customDatAlignment
                        .toString(16)
                        .toUpperCase()})`
                    : `Default per-file-type alignment (Custom override preset: ${customDatAlignment} B)`
                }
                onClick={() => {
                  setActiveTopMenu(null);
                  setDatAlignSettingsOpen(true);
                }}
              >
                <span>Configure DAT save alignment...</span>
                {forceCustomDatAlignment && (
                  <span className="menu-check-badge" aria-label="Custom DAT alignment active">
                    <IconCheck />
                  </span>
                )}
              </button>
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

        <div
          style={{ position: "relative" }}
          onMouseEnter={() => {
            if (activeTopMenu !== null && activeTopMenu !== "help") {
              setActiveTopMenu("help");
            }
          }}
        >
          <button
            className={`menu-top-button ${activeTopMenu === "help" ? "active" : ""}`}
            onClick={() => {
              setContextMenu(null);
              setActiveTopMenu(activeTopMenu === "help" ? null : "help");
            }}
            onMouseEnter={() => {
              if (activeTopMenu !== null && activeTopMenu !== "help") {
                setActiveTopMenu("help");
              }
            }}
          >
            Help
          </button>
          {displayedTopMenu === "help" && (
            <div
              className={`context-menu top-menu-dropdown ${isTopMenuClosing ? "closing" : ""}`}
              style={{ top: 28, left: 0, minWidth: 280 }}
            >
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

          {busyLabel && (
            <span
              className="mono"
              style={{
                marginLeft: 10,
                fontSize: 11.5,
                color: "var(--brush-accent)",
                fontWeight: 600,
              }}
            >
              ⏳ {busyLabel}
            </span>
          )}
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
          <AssetBrowserPanel
            cardRef={assetBrowserCardRef}
            searchInputRef={assetSearchInputRef}
            outline={outline}
            expandedNodes={expandedNodes}
            selectedNodeId={selectedNodeId}
            assetSearch={assetSearch}
            pointerDrag={pointerDrag}
            assetRatio={assetRatio}
            zoomLevel={assetBrowserZoom}
            docGeneration={wmbDocGeneration}
            onChangeZoomLevel={setAssetBrowserZoom}
            onSearchChange={setAssetSearch}
            onToggleExpand={(nodeId) =>
              setExpandedNodes((p) => ({ ...p, [nodeId]: !p[nodeId] }))
            }
            onExpandAll={handleExpandAll}
            onCollapseAll={handleCollapseAll}
            onSelectNode={(nodeId) => void selectAndInspectNode(nodeId)}
            onContextMenu={(e, nodeId, kind) => {
              setHoverSubmenuIdx(null);
              const items = buildAssetNodeContextMenu(nodeId, kind);
              if (items.length > 0) {
                setContextMenu({
                  x: Math.min(e.clientX, window.innerWidth - 260),
                  y: Math.min(e.clientY, window.innerHeight - 340),
                  items,
                });
              }
            }}
            onPointerDownNode={(e, node) => {
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
          />

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

          <div style={{ flex: `${1 - assetRatio} 1 0%`, minHeight: 110, display: "flex", flexDirection: "column" }}>
            <FileBrowserPanel
              fb={fb}
              hideUnsupported={hideUnsupported}
              setHideUnsupported={setHideUnsupported}
              fileSortCol={fileSortCol}
              fileSortAsc={fileSortAsc}
              onOpenFolder={handleMenuOpenFolder}
              onOpenFile={(path) => void openDocumentByPath(path)}
              onContextMenu={(e, entry) => {
                setHoverSubmenuIdx(null);
                setContextMenu({
                  x: Math.min(e.clientX, window.innerWidth - 250),
                  y: Math.min(e.clientY, window.innerHeight - 360),
                  items: buildFileBrowserContextMenu(entry),
                });
              }}
              setContextMenu={setContextMenu}
            />
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
        <DocumentEditors
          selection={{
            nodePayload,
            setNodePayload,
            activeTexture,
            activeWmb,
            outline,
            assetSearch,
          }}
          drafts={drafts}
          meshUi={meshUi}
          layout={{
            rightWidth,
            draggingSplit,
            setDraggingSplit,
            setRightWidth,
            panStartRef,
          }}
          dll={{
            oodleDllPath,
            setOodleDllPath,
            oodleConfigured,
            nvttDllPath,
            setNvttDllPath,
            nvttConfigured,
            handlePickOodleDll,
            handlePickNvttDll,
          }}
          actions={{
            selectAndInspectNode,
            runWmbMutation,
            runTextureMutation,
            runStructMutation,
            runClothMutation,
            runContainerMutation,
            openPkzCompressionModal,
            handleCrossGameMaterialConvert,
            promptAndRunTextureImport,
            handleTextureExport,
            handleResetView,
            commitBxmDraft,
            loadDirectory,
            currentDirRef,
            buildAssetNodeContextMenu,
            setContextMenu,
            setHoverSubmenuIdx,
            setStatusLeft,
            switchBxmViewMode,
          }}
          materialClipboard={materialClipboard}
          setMaterialClipboard={setMaterialClipboard}
          pointerDrag={pointerDrag}
          setPointerDrag={setPointerDrag}
        />
        </>
        )}
      </div>

      <footer className="studio-statusbar">
        <div className="status-left">
          {outline?.dirty ? "● " : ""}
          {statusLeft}
        </div>
        <div className="status-right">
          {forceCustomDatAlignment && (
            <span
              className="mono"
              style={{ color: "var(--brush-accent)", fontWeight: 600 }}
              title="Forced custom byte alignment on DAT save is enabled"
            >
              DAT Align: {customDatAlignment} B (0x{customDatAlignment.toString(16).toUpperCase()})
            </span>
          )}
          {activeTexture && <span title="Preview zoom">{Math.round(zoom * 100)}%</span>}
          <span>{outline ? outline.summary_right : "PGTools Studio"}</span>
        </div>
      </footer>

      {displayedContextMenu && (
        <div
          className="context-menu-backdrop"
          style={isContextMenuClosing ? { pointerEvents: "none" } : undefined}
          onClick={() => setContextMenu(null)}
          onContextMenu={(e) => {
            e.preventDefault();
            setContextMenu(null);
          }}
        >
          <div
            key={`${displayedContextMenu.x}-${displayedContextMenu.y}`}
            className={`context-menu ${isContextMenuClosing ? "closing" : ""}`}
            style={{ left: displayedContextMenu.x, top: displayedContextMenu.y }}
            onClick={(e) => e.stopPropagation()}
          >
            {displayedContextMenu.items.map((item, idx) =>
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
                  {item.submenu && displayedSubmenuIdx === idx && (
                    <div className={`context-submenu ${isSubmenuClosing ? "closing" : ""}`}>
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
          setTexQuality={setTexQuality}
          setTexGenerateMips={setTexGenerateMips}
          setTexSolidify={setTexSolidify}
          setTexMakeOpaque={setTexMakeOpaque}
          setTexSyncFlags={setTexSyncFlags}
          runTextureMutation={runTextureMutation}
        />
      )}

      {batchReplaceNodeId && (
        <BatchReplaceModal
          nodeId={batchReplaceNodeId}
          initialFolder={currentDir}
          onClose={() => setBatchReplaceNodeId(null)}
          onRunReplace={async (targetNodeId, folder, quality, solidify, syncFlags) => {
            await runTextureMutation(targetNodeId, "batch_replace_folder", {
              folder,
              quality,
              solidify,
              sync_flags: syncFlags,
            });
          }}
        />
      )}

      {batchRecompressOpen && (
        <BatchRecompressModal
          hasOpenDocument={Boolean(outline)}
          defaultFolder={currentDir}
          onClose={() => setBatchRecompressOpen(false)}
          onOutlineUpdated={(updated, statusMsg) => {
            setOutline(updated);
            setWmbDocGeneration((g) => g + 1);
            void selectAndInspectNode(selectedNodeId);
            setStatusLeft(statusMsg);
          }}
        />
      )}

      {batchDatAlignOpen && (
        <BatchDatAlignModal
          hasOpenDocument={Boolean(outline)}
          defaultFolder={currentDir}
          onClose={() => setBatchDatAlignOpen(false)}
          onOutlineUpdated={(updated, statusMsg) => {
            setOutline(updated);
            setWmbDocGeneration((g) => g + 1);
            void selectAndInspectNode(selectedNodeId);
            setStatusLeft(statusMsg);
          }}
          onDirectoryMutated={() => {
            void loadDirectory(currentDirRef.current);
          }}
        />
      )}

      {datAlignSettingsOpen && (
        <DatAlignmentSettingsModal
          forceCustomDatAlignment={forceCustomDatAlignment}
          customDatAlignment={customDatAlignment}
          onChangeForceCustomDatAlignment={(val) => {
            setForceCustomDatAlignment(val);
            setStatusLeft(
              val
                ? `Enabled forced custom alignment on DAT save (${customDatAlignment} B / 0x${customDatAlignment
                    .toString(16)
                    .toUpperCase()})`
                : "Restored default per-file-type alignment on DAT save"
            );
          }}
          onChangeCustomDatAlignment={(val) => {
            setCustomDatAlignment(val);
          }}
          onClose={() => setDatAlignSettingsOpen(false)}
        />
      )}

      {exportFolderModalOpen && (
        <ExportTexturesModal
          defaultSourceFolder={currentDir}
          onClose={() => setExportFolderModalOpen(false)}
        />
      )}

      {newBankModalOpen && (
        <NewBankModal
          onClose={() => setNewBankModalOpen(false)}
          onCreated={(opened) => {
            setOutline(opened);
            const { nextExpanded, defaultTargetId } = expandForOpen(opened);
            setExpandedNodes(nextExpanded);
            void selectAndInspectNode(defaultTargetId);
            loadDirectory(currentDirRef.current);
          }}
          onStatus={setStatusLeft}
        />
      )}

      {/* PKZ Compression Type & Level Modal (matching PKZ Explorer CompressionDialog) */}
      {pkzCompModal && (
        <div
          className="context-menu-backdrop"
          style={{
            display: "flex",
            alignItems: "center",
            justifyContent: "center",
            backgroundColor: "rgba(0, 0, 0, 0.55)",
          }}
          onClick={() => setPkzCompModal(null)}
        >
          <div
            className="studio-card"
            style={{
              width: 470,
              maxWidth: "94vw",
              padding: 16,
              display: "flex",
              flexDirection: "column",
              gap: 12,
            }}
            onClick={(e) => e.stopPropagation()}
          >
            <div>
              <div style={{ fontWeight: 700, fontSize: 14, color: "var(--brush-heading)" }}>
                {pkzCompModal.title}
              </div>
              {pkzCompModal.subtitle && (
                <div style={{ fontSize: 11.5, color: "var(--brush-muted)", marginTop: 3 }}>
                  {pkzCompModal.subtitle}
                </div>
              )}
            </div>

            <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
              {(
                [
                  [
                    "None",
                    "None (Uncompressed)",
                    "Store every file as it is. Fastest loading, largest archives.",
                  ],
                  [
                    "ZStandard",
                    "ZStandard",
                    "Built in, no extra DLL needed. Fast and widely supported.",
                  ],
                  [
                    "OodleKraken",
                    "Oodle Kraken",
                    "What Bayonetta 3 and Astral Chain use natively. Requires oo2core_*_win64.dll for encoding.",
                  ],
                ] as const
              ).map(([codecKey, codecLabel, codecDesc]) => {
                const checked = pkzCompModal.compression === codecKey;
                return (
                  <label
                    key={codecKey}
                    style={{
                      display: "flex",
                      alignItems: "flex-start",
                      gap: 9,
                      padding: "8px 10px",
                      borderRadius: 6,
                      border: `1px solid ${
                        checked ? "var(--brush-accent)" : "var(--brush-border)"
                      }`,
                      cursor: "pointer",
                    }}
                  >
                    <input
                      type="radio"
                      name="pkz_codec_choice"
                      checked={checked}
                      onChange={() => {
                        const nextLvl = clampPkzLevel(
                          codecKey,
                          codecKey === "OodleKraken"
                            ? 4
                            : codecKey === "ZStandard"
                            ? 3
                            : 0
                        );
                        setPkzCompModal((prev) =>
                          prev
                            ? { ...prev, compression: codecKey, level: nextLvl }
                            : null
                        );
                      }}
                      style={{ marginTop: 2 }}
                    />
                    <div>
                      <div style={{ fontWeight: 600, fontSize: 12.5 }}>{codecLabel}</div>
                      <div style={{ fontSize: 11.5, color: "var(--brush-muted)", marginTop: 2 }}>
                        {codecDesc}
                      </div>
                    </div>
                  </label>
                );
              })}
            </div>

            {pkzCompModal.compression !== "None" && (
              <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
                <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center" }}>
                  <span style={{ fontSize: 12, fontWeight: 600 }}>
                    Compression Level ({pkzCompModal.compression === "OodleKraken" ? "1–9" : "1–22"})
                  </span>
                  <span className="mono" style={{ fontWeight: 700, fontSize: 12.5 }}>
                    {pkzCompModal.level}
                  </span>
                </div>
                <input
                  type="range"
                  min={1}
                  max={pkzCompModal.compression === "OodleKraken" ? 9 : 22}
                  value={pkzCompModal.level}
                  onChange={(e) => {
                    const next = parseInt(e.target.value || "3", 10);
                    setPkzCompModal((prev) =>
                      prev ? { ...prev, level: next } : null
                    );
                  }}
                />
                <label
                  style={{
                    display: "flex",
                    alignItems: "center",
                    gap: 7,
                    fontSize: 11.5,
                    color: "var(--brush-muted)",
                    marginTop: 2,
                    cursor: "pointer",
                  }}
                >
                  <input
                    type="checkbox"
                    checked={pkzCompModal.forceCompression}
                    onChange={(e) =>
                      setPkzCompModal((prev) =>
                        prev ? { ...prev, forceCompression: e.target.checked } : null
                      )
                    }
                  />
                  <span>
                    Force compression even if file stays at ≥ 90% of original size
                  </span>
                </label>
                <div style={{ fontSize: 11, color: "var(--brush-muted)", lineHeight: 1.4 }}>
                  {pkzCompModal.forceCompression
                    ? "Every file will be stored compressed even if compression does not reduce size by 10%."
                    : "Files that stay at 90% or more of their original size are automatically stored uncompressed (None) for faster in-game loading."}
                </div>
              </div>
            )}

            {pkzCompModal.compression === "OodleKraken" && (
              <div
                style={{
                  padding: "8px 10px",
                  borderRadius: 6,
                  border: "1px solid var(--brush-border)",
                  fontSize: 11.5,
                  display: "flex",
                  justifyContent: "space-between",
                  alignItems: "center",
                  gap: 8,
                }}
              >
                <div style={{ color: "var(--brush-muted)", flex: 1 }}>
                  {oodleValidation?.can_compress
                    ? `✓ ${oodleValidation.message}`
                    : oodleValidation?.message ??
                      "Oodle Kraken compression requires oo2core_*_win64.dll."}
                </div>
                <button
                  type="button"
                  className="pill-button"
                  onClick={() => void handlePickOodleDll()}
                >
                  Browse DLL...
                </button>
              </div>
            )}

            <div style={{ display: "flex", justifyContent: "flex-end", gap: 8, marginTop: 4 }}>
              <button
                type="button"
                className="pill-button"
                onClick={() => setPkzCompModal(null)}
              >
                Cancel
              </button>
              <button
                type="button"
                className="pill-button accent"
                onClick={() => {
                  const { compression, level, forceCompression, onConfirm } = pkzCompModal;
                  setPkzDefaultCompression(compression);
                  if (compression !== "None") {
                    setPkzDefaultLevel(level);
                  }
                  setPkzCompModal(null);
                  void onConfirm(compression, level, forceCompression);
                }}
              >
                OK
              </button>
            </div>
          </div>
        </div>
      )}

      {/* PKZ Collision & Extract Overwrite Modal (matching PKZ Explorer OverwriteDialog) */}
      {pkzConflictModal && (
        <div
          className="context-menu-backdrop"
          style={{
            display: "flex",
            alignItems: "center",
            justifyContent: "center",
            backgroundColor: "rgba(0, 0, 0, 0.55)",
          }}
          onClick={() => setPkzConflictModal(null)}
        >
          <div
            className="studio-card"
            style={{
              width: 440,
              maxWidth: "94vw",
              padding: 16,
              display: "flex",
              flexDirection: "column",
              gap: 12,
            }}
            onClick={(e) => e.stopPropagation()}
          >
            <div style={{ fontWeight: 700, fontSize: 14, color: "var(--brush-heading)" }}>
              {pkzConflictModal.title}
            </div>
            <div
              style={{
                fontSize: 12,
                color: "var(--brush-muted)",
                whiteSpace: "pre-line",
                lineHeight: 1.45,
              }}
            >
              {pkzConflictModal.message}
            </div>
            {pkzConflictModal.paths && pkzConflictModal.paths.length > 0 && (
              <div
                className="mono"
                style={{
                  fontSize: 11,
                  padding: "6px 8px",
                  borderRadius: 4,
                  border: "1px solid var(--brush-border)",
                  maxHeight: 110,
                  overflowY: "auto",
                }}
              >
                {pkzConflictModal.paths.map((p, i) => (
                  <div key={i}>{p}</div>
                ))}
              </div>
            )}
            <div style={{ display: "flex", justifyContent: "flex-end", gap: 8, marginTop: 4 }}>
              <button
                type="button"
                className="pill-button"
                onClick={() => setPkzConflictModal(null)}
              >
                Cancel
              </button>
              <button
                type="button"
                className="pill-button"
                onClick={() => void pkzConflictModal.onChoice("skip")}
              >
                Skip Existing
              </button>
              <button
                type="button"
                className="pill-button accent"
                onClick={() => void pkzConflictModal.onChoice("overwrite")}
              >
                Overwrite
              </button>
            </div>
          </div>
        </div>
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
