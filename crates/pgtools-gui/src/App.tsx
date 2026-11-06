import React, { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import {
  BxmXmlNode,
  ContextMenuState,
  DecodedWmbMaterial,
  DocumentOutline,
  FileSortCol,
  FsEntry,
  MaterialClipboardState,
  NodeInspectPayload,
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
import { useStudioSettings } from "./hooks/useStudioSettings";
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
    setTheme,
    setLeftWidth,
    setRightWidth,
    setAssetRatio,
    setOodleDllPath,
    setNvttDllPath,
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
    buildStudioSettingsPayload,
    applyLoadedSettings,
    markLoaded,
  } = settings;

  const [draggingSplit, setDraggingSplit] = useState<"left" | "right" | "left-row" | null>(null);
  const leftColRef = useRef<HTMLDivElement | null>(null);
  const assetBrowserCardRef = useRef<HTMLDivElement | null>(null);

  const [wmbDocGeneration, setWmbDocGeneration] = useState<number>(0);
  const [coreDatModalOpen, setCoreDatModalOpen] = useState<boolean>(false);

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
  const meshUi = useMeshUi();
  const {
    expandedMeshGroups,
    setExpandedMeshGroups,
    expandedBatchLods,
    setExpandedBatchLods,
    draggedBatchInfo,
    setDraggedBatchInfo,
    dragOverMeshIdx,
    setDragOverMeshIdx,
    pkzBulkCodec,
    setPkzBulkCodec,
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
    channelMode,
    setChannelMode,
    opaquePreview,
    setOpaquePreview,
    texEncodeFormat,
    setTexEncodeFormat,
    texGenerateMips,
    setTexGenerateMips,
    texSolidify,
    setTexSolidify,
    texMakeOpaque,
    setTexMakeOpaque,
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

  // Menus & Status
  const [activeTopMenu, setActiveTopMenu] = useState<"file" | "tools" | "help" | null>(null);
  const [contextMenu, setContextMenu] = useState<ContextMenuState | null>(null);
  const [hoverSubmenuIdx, setHoverSubmenuIdx] = useState<number | null>(null);

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
      invoke("save_studio_settings", { settings: payload }).catch(() => {});
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

  async function openDocumentByPath(path: string, skipDirtyPrompt = false) {
    if (!skipDirtyPrompt && !confirmDiscardIfDirty(`open "${path.split(/[\\/]/).pop() || path}"`)) {
      return;
    }
    try {
      setStatusLeft(`Loading ${path}...`);
      const opened = await invoke<DocumentOutline>("open_document", { path });
      setOutline(opened);
      setSelectedFilePath(path);
      setWmbB2Override(null);
      refreshRecents();

      const { nextExpanded, defaultTargetId } = expandForOpen(opened);
      setWmbDocGeneration((g) => g + 1);
      setExpandedNodes(nextExpanded);
      setMipIndex(0);
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
      const res = await invoke<NodeInspectPayload>("inspect_asset_node", {
        nodeId,
        mipIndex: mip,
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
        settings: buildStudioSettingsPayload(currentDir, { oodle_dll_path: picked }),
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
        settings: buildStudioSettingsPayload(currentDir, { nvtt_dll_path: picked }),
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

  const activeTexture: TextureNodeView | null =
    nodePayload?.view_type === "TEXTURE" ? (nodePayload.texture ?? null) : null;
  const activeWmb: WmbSectionView | null =
    nodePayload?.view_type === "WMB" ? (nodePayload.wmb ?? null) : null;

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
          <AssetBrowserPanel
            cardRef={assetBrowserCardRef}
            outline={outline}
            expandedNodes={expandedNodes}
            selectedNodeId={selectedNodeId}
            assetSearch={assetSearch}
            pointerDrag={pointerDrag}
            assetRatio={assetRatio}
            onSearchChange={setAssetSearch}
            onToggleExpand={(nodeId) =>
              setExpandedNodes((p) => ({ ...p, [nodeId]: !p[nodeId] }))
            }
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
            nvttDllPath,
            setNvttDllPath,
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
