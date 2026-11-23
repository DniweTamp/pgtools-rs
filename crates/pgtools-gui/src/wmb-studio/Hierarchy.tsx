import { AnimatedBranch } from "../components/AnimatedBranch";
import { useClosingTransition } from "../hooks/useClosingTransition";
import { ContextMenuState, HierarchyContextMenu } from "./HierarchyContextMenu";
import { HierarchyModals, MaterialDeleteModal, TextureSlotDropModal } from "./HierarchyModals";
import React, {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import {
  BatchDesc,
  BoneDesc,
  ClipboardStatus,
  LegacyActionKind,
  LodDesc,
  MaterialDesc,
  MaterialShaderStatus,
  MeshGroupDesc,
  TextureRefDesc,
  WmbSceneManifest,
} from "./contract";
export type { LegacyActionKind };
import { canPasteClipboard } from "./clipboard";
import {
  computeDropPositionAt,
  DragPayload,
  DropPosition,
  isDropCompatible,
} from "./dnd";
import {
  IconChevronRight,
  IconEye,
  IconShadow,
  inferTextureRoles,
  isSyntheticTexName,
  MaterialPickerPopover,
} from "./Pickers";
import {
  buildBoneHierarchy,
  getBoneSceneId,
  getMeshGroupBatchIds,
  parseTextureSceneId,
  SceneSelectionItem,
  SelectionKind,
} from "./store";
import { formatHexU8 } from "../utils/format";

interface HierarchyProps {
  manifest: WmbSceneManifest;
  selection: SceneSelectionItem[];
  onSelect: (items: SceneSelectionItem[], additive?: boolean) => void;
  shaderStatuses: Map<string, MaterialShaderStatus>;
  clipboardStatus: ClipboardStatus | null;
  hiddenBatchIds: Set<string>;
  hiddenMeshGroupIds: Set<string>;
  onToggleBatchVisibility: (batchId: string) => void;
  onToggleMeshGroupVisibility: (meshGroupId: string) => void;
  onMutateOp: (op: Record<string, unknown>) => Promise<void>;
  onCopyItems: (itemIds: string[]) => Promise<void>;
  onPasteInto: (targetContainerId: string | null) => Promise<void>;
  onConvertMaterialProfile: (
    materialIndex: number,
    targetGame: "b1" | "b2" | "b3"
  ) => Promise<void>;
  onOpenLegacyFlow: (itemId: string, action: LegacyActionKind) => Promise<void>;
}

type DomainFilter = "all" | "meshes" | "materials" | "bones" | "textures";

interface FlatTreeRow {
  rowKey: string;
  selectionItem: SceneSelectionItem | null;
  parentGroupKey: string;
  expandKey?: string;
}

interface OutlinerDropTarget {
  kind: SelectionKind;
  id: string;
  orderIndex?: number;
  position: DropPosition;
}

interface OutlinerPointerDrag {
  payload: DragPayload;
  label: string;
  startX: number;
  startY: number;
  curX: number;
  curY: number;
  active: boolean;
  target: OutlinerDropTarget | null;
}

export const Hierarchy: React.FC<HierarchyProps> = React.memo(function Hierarchy({
  manifest,
  selection,
  onSelect,
  shaderStatuses,
  clipboardStatus,
  hiddenBatchIds,
  hiddenMeshGroupIds,
  onToggleBatchVisibility,
  onToggleMeshGroupVisibility,
  onMutateOp,
  onCopyItems,
  onPasteInto,
  onConvertMaterialProfile,
  onOpenLegacyFlow,
}: HierarchyProps) {
  const treeContainerRef = useRef<HTMLDivElement | null>(null);
  const [filterText, setFilterText] = useState("");
  const [domainFilter, setDomainFilter] = useState<DomainFilter>("all");
  const [showShadowProxies, setShowShadowProxies] = useState(false);

  // Everything in Scene Outliner starts collapsed by default (except the root container node)
  const [expandedKeys, setExpandedKeys] = useState<Set<string>>(() => {
    return new Set(["root", "sec:model"]);
  });

  // Reset to collapsed-by-default synchronously when switching to a different model node
  const [prevNodeId, setPrevNodeId] = useState(manifest.nodeId);
  const isFiltering = filterText.trim().length > 0;
  const [prevFiltering, setPrevFiltering] = useState(isFiltering);
  const [suppressBranchAnim, setSuppressBranchAnim] = useState(false);

  if (manifest.nodeId !== prevNodeId) {
    setPrevNodeId(manifest.nodeId);
    setExpandedKeys(new Set(["root", "sec:model"]));
    setDomainFilter("all");
    setSuppressBranchAnim(true);
  }

  if (isFiltering !== prevFiltering) {
    setPrevFiltering(isFiltering);
    setSuppressBranchAnim(true);
  }

  const [renamingId, setRenamingId] = useState<string | null>(null);
  const [renameDraft, setRenameDraft] = useState<string>("");

  const [dragIndicator, setDragIndicator] = useState<{
    targetId: string;
    position: DropPosition;
  } | null>(null);
  // HTML5 drag-and-drop is unusable inside a Tauri WebView2 window on Windows:
  // Tauri installs its own OS drop handler, which prevents the webview from
  // receiving dragover/drop events. Use pointer events for the Scene Outliner
  // instead, while leaving Tauri's native file-drop handler enabled.
  const pointerDragRef = useRef<OutlinerPointerDrag | null>(null);
  const suppressClickUntilRef = useRef(0);
  const [dragGhost, setDragGhost] = useState<{
    x: number;
    y: number;
    label: string;
  } | null>(null);

  const [contextMenu, setContextMenu] = useState<ContextMenuState | null>(null);
  const [slotDropModal, setSlotDropModal] =
    useState<TextureSlotDropModal | null>(null);
  const [matDeleteModal, setMatDeleteModal] =
    useState<MaterialDeleteModal | null>(null);

  const { displayed: activeCtxMenu, isClosing: isCtxMenuClosing } =
    useClosingTransition(contextMenu);
  const { displayed: activeSlotDrop, isClosing: isSlotDropClosing } =
    useClosingTransition(slotDropModal);

  useEffect(() => {
    const closeMenus = () => {
      setContextMenu(null);
      setSlotDropModal(null);
    };
    window.addEventListener("click", closeMenus);
    return () => window.removeEventListener("click", closeMenus);
  }, []);

  const toggleExpand = useCallback((key: string, e?: React.MouseEvent) => {
    e?.stopPropagation();
    setExpandedKeys((prev) => {
      const next = new Set(prev);
      if (next.has(key)) next.delete(key);
      else next.add(key);
      return next;
    });
  }, []);

  const handleDomainSelect = useCallback((nextDomain: DomainFilter) => {
    setDomainFilter(nextDomain);
    if (nextDomain !== "all") {
      setExpandedKeys((prev) => {
        const next = new Set(prev);
        next.add("root");
        next.add("sec:model");
        if (nextDomain === "meshes") next.add("sec:meshes");
        if (nextDomain === "materials") next.add("sec:materials");
        if (nextDomain === "bones") next.add("sec:bones");
        if (nextDomain === "textures") next.add("sec:textures");
        return next;
      });
    }
  }, []);

  const collapseAll = useCallback(() => {
    setSuppressBranchAnim(true);
    setExpandedKeys(new Set(["root", "sec:model"]));
  }, []);

  useEffect(() => {
    if (!suppressBranchAnim) return;
    const id = requestAnimationFrame(() => setSuppressBranchAnim(false));
    return () => cancelAnimationFrame(id);
  }, [suppressBranchAnim]);

  const selectedSet = useMemo(() => {
    const s = new Set<string>();
    selection.forEach((item) => s.add(`${item.kind}:${item.id}`));
    return s;
  }, [selection]);

  const isSelected = useCallback(
    (kind: SelectionKind, id: string) => selectedSet.has(`${kind}:${id}`),
    [selectedSet]
  );

  const batchById = useMemo(() => {
    const m = new Map<string, BatchDesc>();
    manifest.batches.forEach((b) => m.set(b.id, b));
    return m;
  }, [manifest.batches]);

  const materialById = useMemo(() => {
    const m = new Map<string, MaterialDesc>();
    manifest.materials.forEach((mat) => m.set(mat.id, mat));
    return m;
  }, [manifest.materials]);

  const textureRoles = useMemo(() => inferTextureRoles(manifest), [manifest]);

  const boneChildrenMap = useMemo(() => buildBoneHierarchy(manifest.bones), [manifest.bones]);

  // Flat list of selectable rows in display order for Shift+click range selection and Up/Down arrows
  const visibleFlatRows = useMemo<FlatTreeRow[]>(() => {
    const rows: FlatTreeRow[] = [];
    const q = filterText.trim().toLowerCase();

    rows.push({
      rowKey: "root:root",
      selectionItem: { kind: "root", id: "root" },
      parentGroupKey: "root",
      expandKey: "root",
    });

    if (!expandedKeys.has("root")) return rows;

    const showTextures = domainFilter === "all" || domainFilter === "textures";
    const showMaterials = domainFilter === "all" || domainFilter === "materials";
    const showBones = domainFilter === "all" || domainFilter === "bones";
    const showMeshes = domainFilter === "all" || domainFilter === "meshes";

    if (showTextures && (expandedKeys.has("sec:textures") || q.length > 0)) {
      manifest.textures.forEach((tex, idx) => {
        const role = textureRoles.get(tex.textureId >>> 0) || "";
        const realName = !isSyntheticTexName(tex.name) ? tex.name.toLowerCase() : "";
        if (
          q &&
          !`#${idx}`.includes(q) &&
          !tex.idHex.toLowerCase().includes(q) &&
          !role.toLowerCase().includes(q) &&
          !realName.includes(q)
        ) {
          return;
        }
        rows.push({
          rowKey: `texture:tex:${tex.textureId}`,
          selectionItem: { kind: "texture", id: `tex:${tex.textureId}` },
          parentGroupKey: "sec:textures",
        });
      });
    }

    if (showMaterials && (expandedKeys.has("sec:materials") || q.length > 0)) {
      for (const mat of manifest.materials) {
        if (
          q &&
          !mat.name.toLowerCase().includes(q) &&
          !(mat.shaderName ?? "").toLowerCase().includes(q) &&
          !`#${mat.index}`.includes(q)
        ) {
          continue;
        }
        rows.push({
          rowKey: `material:${mat.id}`,
          selectionItem: { kind: "material", id: mat.id },
          parentGroupKey: "sec:materials",
        });
      }
    }

    if (showBones && (expandedKeys.has("sec:bones") || q.length > 0)) {
      const visitedBones = new Set<number>();
      const visitBone = (b: BoneDesc, idx: number) => {
        if (visitedBones.has(idx)) return;
        visitedBones.add(idx);
        const bId = getBoneSceneId(b, idx);
        const label = `#${idx} gid ${b.globalId} local ${b.localId ?? idx}`.toLowerCase();
        if (!q || label.includes(q)) {
          rows.push({
            rowKey: `bone:${bId}`,
            selectionItem: { kind: "bone", id: bId },
            parentGroupKey: "sec:bones",
            expandKey: bId,
          });
        }
        const kids = boneChildrenMap.childrenByParentIdx.get(idx) ?? [];
        if (expandedKeys.has(bId) || q.length > 0) {
          for (const child of kids) {
            visitBone(child.bone, child.idx);
          }
        }
      };
      const roots = boneChildrenMap.childrenByParentIdx.get(-1) ?? [];
      for (const rb of roots) {
        visitBone(rb.bone, rb.idx);
      }
    }

    if (showMeshes && (expandedKeys.has("sec:meshes") || q.length > 0)) {
      for (const mg of manifest.meshGroups) {
        rows.push({
          rowKey: `meshGroup:${mg.id}`,
          selectionItem: { kind: "meshGroup", id: mg.id },
          parentGroupKey: "sec:meshes",
          expandKey: mg.id,
        });
        if (expandedKeys.has(mg.id) || q.length > 0) {
          if (mg.lods.length > 0) {
            for (const lod of mg.lods) {
              rows.push({
                rowKey: `lod:${lod.id}`,
                selectionItem: { kind: "lod", id: lod.id },
                parentGroupKey: mg.id,
                expandKey: lod.id,
              });
              if (expandedKeys.has(lod.id) || q.length > 0) {
                for (const bid of lod.batchIds) {
                  const b = batchById.get(bid);
                  if (!b) continue;
                  if (b.isShadowProxy && !showShadowProxies) continue;
                  rows.push({
                    rowKey: `batch:${b.id}`,
                    selectionItem: { kind: "batch", id: b.id },
                    parentGroupKey: lod.id,
                  });
                }
              }
            }
          } else {
            for (const bid of mg.batchIds) {
              const b = batchById.get(bid);
              if (!b) continue;
              if (b.isShadowProxy && !showShadowProxies) continue;
              rows.push({
                rowKey: `batch:${b.id}`,
                selectionItem: { kind: "batch", id: b.id },
                parentGroupKey: mg.id,
              });
            }
          }
        }
      }
    }

    return rows;
  }, [
    manifest,
    expandedKeys,
    filterText,
    domainFilter,
    boneChildrenMap,
    batchById,
    showShadowProxies,
    textureRoles,
  ]);

  const handleRowClick = useCallback(
    (item: SceneSelectionItem, parentGroupKey: string, e: React.MouseEvent) => {
      e.stopPropagation();
      if (performance.now() < suppressClickUntilRef.current) {
        e.preventDefault();
        return;
      }
      if (e.shiftKey && selection.length > 0) {
        const anchor = selection[selection.length - 1];
        const anchorIdx = visibleFlatRows.findIndex(
          (r) =>
            r.selectionItem?.kind === anchor.kind &&
            r.selectionItem?.id === anchor.id
        );
        const targetIdx = visibleFlatRows.findIndex(
          (r) =>
            r.selectionItem?.kind === item.kind &&
            r.selectionItem?.id === item.id
        );
        if (anchorIdx >= 0 && targetIdx >= 0) {
          const start = Math.min(anchorIdx, targetIdx);
          const end = Math.max(anchorIdx, targetIdx);
          const rangeItems: SceneSelectionItem[] = [];
          for (let i = start; i <= end; i++) {
            const row = visibleFlatRows[i];
            if (
              row.selectionItem &&
              row.selectionItem.kind === item.kind &&
              row.parentGroupKey === parentGroupKey
            ) {
              rangeItems.push(row.selectionItem);
            }
          }
          if (rangeItems.length > 0) {
            onSelect(rangeItems, false);
            return;
          }
        }
      }
      const additive = e.ctrlKey || e.metaKey;
      onSelect([item], additive);
    },
    [onSelect, selection, visibleFlatRows]
  );

  const beginInlineRename = useCallback(
    (itemId: string, currentName: string) => {
      setRenamingId(itemId);
      setRenameDraft(currentName);
    },
    []
  );

  const commitInlineRename = useCallback(async () => {
    if (!renamingId) return;
    const targetId = renamingId;
    const nextName = renameDraft.trim();
    setRenamingId(null);
    if (!nextName) return;
    await onMutateOp({
      op: "rename_item",
      itemId: targetId,
      name: nextName,
    });
  }, [onMutateOp, renameDraft, renamingId]);

  const triggerDeleteSelected = useCallback(
    async (itemsToDelete: SceneSelectionItem[]) => {
      const deletable = itemsToDelete.filter(
        (s) =>
          s.kind === "material" ||
          s.kind === "batch" ||
          s.kind === "meshGroup" ||
          s.kind === "bone"
      );
      if (deletable.length === 0) return;

      if (deletable.length === 1 && deletable[0].kind === "material") {
        const mat = materialById.get(deletable[0].id);
        if (
          mat &&
          mat.usedByBatchIds.length > 0 &&
          manifest.materials.length > 1
        ) {
          const fallbackMat =
            manifest.materials.find((m) => m.index !== mat.index) ??
            manifest.materials[0];
          setMatDeleteModal({
            material: mat,
            reassignToIndex: fallbackMat.index,
          });
          return;
        }
      }

      await onMutateOp({
        op: "delete_items",
        items: deletable.map((d) => d.id),
      });
    },
    [manifest.materials, materialById, onMutateOp]
  );

  // Keyboard shortcuts when Hierarchy has focus
  const handleKeyDown = useCallback(
    async (e: React.KeyboardEvent<HTMLDivElement>) => {
      if (
        e.target instanceof HTMLInputElement ||
        e.target instanceof HTMLTextAreaElement ||
        e.target instanceof HTMLSelectElement
      ) {
        return;
      }

      const primary = selection[0];
      if (e.key === "Escape") {
        e.preventDefault();
        onSelect([{ kind: "root", id: "root" }], false);
        return;
      }

      if (e.key === "F2" && primary) {
        if (primary.kind === "meshGroup") {
          const mg = manifest.meshGroups.find((m) => m.id === primary.id);
          if (mg) {
            e.preventDefault();
            beginInlineRename(mg.id, mg.name);
          }
        } else if (primary.kind === "lod") {
          for (const mg of manifest.meshGroups) {
            const l = mg.lods.find((x) => x.id === primary.id);
            if (l) {
              e.preventDefault();
              beginInlineRename(l.id, l.name);
              break;
            }
          }
        }
        return;
      }

      if (e.key === "Delete") {
        e.preventDefault();
        await triggerDeleteSelected(selection);
        return;
      }

      if ((e.ctrlKey || e.metaKey) && (e.key === "c" || e.key === "C")) {
        const copyable = selection
          .filter((s) => s.kind === "batch" || s.kind === "material")
          .map((s) => s.id);
        if (copyable.length > 0) {
          e.preventDefault();
          await onCopyItems(copyable);
        }
        return;
      }

      if ((e.ctrlKey || e.metaKey) && (e.key === "v" || e.key === "V")) {
        if (clipboardStatus && canPasteClipboard(clipboardStatus, manifest.game).allowed) {
          e.preventDefault();
          let targetContainerId: string | null = null;
          if (primary?.kind === "meshGroup" || primary?.kind === "lod") {
            targetContainerId = primary.id;
          } else if (primary?.kind === "batch") {
            const b = batchById.get(primary.id);
            targetContainerId = b?.lodId ?? b?.meshId ?? null;
          }
          await onPasteInto(targetContainerId);
        }
        return;
      }

      if ((e.ctrlKey || e.metaKey) && (e.key === "d" || e.key === "D")) {
        if (
          primary &&
          (primary.kind === "batch" ||
            primary.kind === "material" ||
            primary.kind === "meshGroup")
        ) {
          e.preventDefault();
          await onMutateOp({
            op: "duplicate_item",
            itemId: primary.id,
          });
        }
        return;
      }

      if (e.key === "ArrowDown" || e.key === "ArrowUp") {
        e.preventDefault();
        if (visibleFlatRows.length === 0) return;
        const currIdx = primary
          ? visibleFlatRows.findIndex(
              (r) =>
                r.selectionItem?.kind === primary.kind &&
                r.selectionItem?.id === primary.id
            )
          : 0;
        const nextIdx =
          e.key === "ArrowDown"
            ? Math.min(visibleFlatRows.length - 1, Math.max(0, currIdx + 1))
            : Math.max(0, currIdx - 1);
        const nextRow = visibleFlatRows[nextIdx];
        if (nextRow?.selectionItem) {
          onSelect([nextRow.selectionItem], false);
        }
        return;
      }

      if (e.key === "ArrowRight" || e.key === "ArrowLeft") {
        if (!primary) return;
        const currRow = visibleFlatRows.find(
          (r) =>
            r.selectionItem?.kind === primary.kind &&
            r.selectionItem?.id === primary.id
        );
        if (currRow?.expandKey) {
          e.preventDefault();
          const key = currRow.expandKey;
          setExpandedKeys((prev) => {
            const next = new Set(prev);
            if (e.key === "ArrowRight") next.add(key);
            else next.delete(key);
            return next;
          });
        }
      }
    },
    [
      batchById,
      beginInlineRename,
      clipboardStatus,
      manifest,
      onCopyItems,
      onMutateOp,
      onPasteInto,
      onSelect,
      selection,
      triggerDeleteSelected,
      visibleFlatRows,
    ]
  );

  // Pointer-based drag & drop handlers.
  //
  // Do NOT reintroduce `draggable`/HTML5 drag handlers here: on Windows the
  // Tauri/wry drop target replaces WebView2's own handler, so `dragover` and
  // `drop` never reach the page and reordering silently does nothing.
  const beginPointerDrag = useCallback(
    (
      e: React.PointerEvent<HTMLElement>,
      kind: DragPayload["kind"],
      id: string,
      parentId?: string,
      label?: string
    ) => {
      if (e.button !== 0) return;
      const target = e.target as HTMLElement | null;
      if (target?.closest("button, input, textarea, select, a")) return;

      const ids =
        selection.some((s) => s.kind === kind && s.id === id) &&
        selection.filter((s) => s.kind === kind).length > 1
          ? selection.filter((s) => s.kind === kind).map((s) => s.id)
          : [id];

      pointerDragRef.current = {
        payload: { kind, ids, sourceParentId: parentId },
        label: label ?? id,
        startX: e.clientX,
        startY: e.clientY,
        curX: e.clientX,
        curY: e.clientY,
        active: false,
        target: null,
      };
    },
    [selection]
  );

  const resolveDropTarget = useCallback(
    (
      payload: DragPayload,
      targetKind: SelectionKind,
      targetId: string,
      clientY: number,
      rowEl: HTMLElement
    ): DropPosition | null => {
      if (
        payload.kind === "bone" &&
        targetKind === "bone" &&
        targetId !== "sec:bones"
      ) {
        const draggedIdx = boneChildrenMap.resolveBoneIdxFromId(
          payload.ids[0] ?? ""
        );
        const targetIdx = boneChildrenMap.resolveBoneIdxFromId(targetId);
        if (
          draggedIdx < 0 ||
          targetIdx < 0 ||
          targetIdx === draggedIdx ||
          boneChildrenMap.isBoneDescendantOf(targetIdx, draggedIdx)
        ) {
          return null;
        }
      }

      const allowOnto =
        (payload.kind === "material" &&
          (targetKind === "batch" || targetKind === "meshGroup")) ||
        (payload.kind === "texture" && targetKind === "material") ||
        (payload.kind === "batch" &&
          (targetKind === "meshGroup" || targetKind === "lod")) ||
        (payload.kind === "bone" && targetKind === "bone");
      const pos =
        payload.kind === "bone" && targetKind === "bone"
          ? "onto"
          : computeDropPositionAt(clientY, rowEl, allowOnto);

      return isDropCompatible(
        payload,
        targetKind,
        targetId,
        pos,
        (dragId, tgtId) => {
          const dIdx = boneChildrenMap.resolveBoneIdxFromId(dragId);
          const tIdx = boneChildrenMap.resolveBoneIdxFromId(tgtId);
          return (
            dIdx >= 0 &&
            tIdx >= 0 &&
            dIdx !== tIdx &&
            !boneChildrenMap.isBoneDescendantOf(tIdx, dIdx)
          );
        }
      )
        ? pos
        : null;
    },
    [boneChildrenMap]
  );

  const performDrop = useCallback(
    async (
      payload: DragPayload,
      dropPos: DropPosition,
      targetKind: SelectionKind,
      targetId: string,
      targetOrderIndex?: number,
      dropPoint?: { x: number; y: number }
    ) => {
      if (!payload || payload.ids.length === 0) return;

      const draggedId = payload.ids[0];

      // 1. Material -> Batch (`set_batch_material`)
      if (payload.kind === "material" && targetKind === "batch") {
        const mat = materialById.get(draggedId);
        if (mat) {
          await onMutateOp({
            op: "set_batch_material",
            batchId: targetId,
            materialIndex: mat.index,
          });
        }
        return;
      }

      // 2. Material -> Mesh Group (`set_mesh_group_material`)
      if (payload.kind === "material" && targetKind === "meshGroup") {
        const mat = materialById.get(draggedId);
        if (mat) {
          await onMutateOp({
            op: "set_mesh_group_material",
            meshId: targetId,
            materialIndex: mat.index,
          });
        }
        return;
      }

      if (payload.kind === "texture" && targetKind === "material") {
        const mat = materialById.get(targetId);
        const rawTid = parseTextureSceneId(draggedId);
        const texRef =
          rawTid !== null
            ? manifest.textures.find((t) => (t.textureId >>> 0) === rawTid)
            : undefined;
        if (mat && rawTid !== null) {
          setSlotDropModal({
            x: dropPoint?.x ?? 0,
            y: dropPoint?.y ?? 0,
            textureId: rawTid,
            textureName: texRef?.idHex ?? `0x${rawTid.toString(16)}`,
            material: mat,
          });
        }
        return;
      }

      // 4. Bone -> Bone (`reparent_bone`, with cycle check)
      if (payload.kind === "bone" && targetKind === "bone") {
        const draggedIdx = boneChildrenMap.resolveBoneIdxFromId(draggedId);
        if (draggedIdx < 0) return;

        if (targetId === "sec:bones") {
          // Drop onto Bones header -> make root bone (parentIndex = -1)
          await onMutateOp({
            op: "reparent_bone",
            boneIndex: draggedIdx,
            parentIndex: -1,
          });
          return;
        }

        const targetIdx = boneChildrenMap.resolveBoneIdxFromId(targetId);
        if (
          targetIdx >= 0 &&
          targetIdx !== draggedIdx &&
          !boneChildrenMap.isBoneDescendantOf(targetIdx, draggedIdx)
        ) {
          setExpandedKeys((prev) => new Set(prev).add(targetId));
          await onMutateOp({
            op: "reparent_bone",
            boneIndex: draggedIdx,
            parentIndex: targetIdx,
          });
        }
        return;
      }

      // 5. Batch -> Mesh Group / LOD (`move_batches` single atomic mutation — B-10)
      if (
        payload.kind === "batch" &&
        (targetKind === "meshGroup" || targetKind === "lod") &&
        dropPos === "onto"
      ) {
        setExpandedKeys((prev) => new Set(prev).add(targetId));
        await onMutateOp({
          op: "move_batches",
          batchIds: payload.ids,
          targetParentId: targetId,
        });
        return;
      }

      // 6. Reorder Materials / Mesh Groups / Batches (`reorder_item`)
      if (
        payload.kind === targetKind &&
        (dropPos === "before" || dropPos === "after") &&
        typeof targetOrderIndex === "number"
      ) {
        const toIndex =
          dropPos === "after" ? targetOrderIndex + 1 : targetOrderIndex;
        const targetBatch =
          targetKind === "batch" ? batchById.get(targetId) : undefined;
        await onMutateOp({
          op: "reorder_item",
          itemId: draggedId,
          itemIds: payload.ids,
          targetParentId: targetBatch
            ? targetBatch.lodId ?? targetBatch.meshId
            : undefined,
          toIndex,
        });
      }
    },
    [batchById, boneChildrenMap, manifest.textures, materialById, onMutateOp]
  );

  useEffect(() => {
    const finishPointerDrag = (commit: boolean) => {
      const drag = pointerDragRef.current;
      pointerDragRef.current = null;
      setDragGhost(null);
      setDragIndicator(null);
      if (!drag || !drag.active) return;

      // The pointerup that ends a drag must not also select/expand the row.
      suppressClickUntilRef.current = performance.now() + 300;
      if (commit && drag.target) {
        void performDrop(
          drag.payload,
          drag.target.position,
          drag.target.kind,
          drag.target.id,
          drag.target.orderIndex,
          { x: drag.curX, y: drag.curY }
        );
      }
    };

    const onPointerMove = (e: PointerEvent) => {
      const drag = pointerDragRef.current;
      if (!drag) return;

      const dist = Math.hypot(e.clientX - drag.startX, e.clientY - drag.startY);
      if (!drag.active) {
        if (dist < 6) return;
        drag.active = true;
      }

      drag.curX = e.clientX;
      drag.curY = e.clientY;
      setDragGhost({ x: e.clientX, y: e.clientY, label: drag.label });

      // Keep long outliner lists usable while dragging.
      const scroller =
        treeContainerRef.current?.querySelector<HTMLElement>(".wmb-tree-scroll");
      if (scroller) {
        const rect = scroller.getBoundingClientRect();
        if (e.clientY < rect.top + 22) {
          scroller.scrollTop = Math.max(0, scroller.scrollTop - 14);
        } else if (e.clientY > rect.bottom - 22) {
          scroller.scrollTop += 14;
        }
      }

      const el = document.elementFromPoint(
        e.clientX,
        e.clientY
      ) as HTMLElement | null;
      const rowEl = el?.closest<HTMLElement>("[data-outliner-id]") ?? null;

      let target: OutlinerDropTarget | null = null;
      if (rowEl) {
        const kind = rowEl.dataset.outlinerKind as SelectionKind | undefined;
        const id = rowEl.dataset.outlinerId;
        const orderRaw = rowEl.dataset.outlinerOrder;
        if (kind && id) {
          const position = resolveDropTarget(
            drag.payload,
            kind,
            id,
            e.clientY,
            rowEl
          );
          if (position) {
            target = {
              kind,
              id,
              position,
              orderIndex:
                orderRaw !== undefined && orderRaw !== ""
                  ? Number(orderRaw)
                  : undefined,
            };
          }
        }
      }

      drag.target = target;
      setDragIndicator((prev) => {
        if (!target) return prev === null ? prev : null;
        if (prev && prev.targetId === target.id && prev.position === target.position) {
          return prev;
        }
        return { targetId: target.id, position: target.position };
      });
    };

    const onPointerUp = () => finishPointerDrag(true);
    const onPointerCancel = () => finishPointerDrag(false);
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape" && pointerDragRef.current) {
        finishPointerDrag(false);
      }
    };

    window.addEventListener("pointermove", onPointerMove);
    window.addEventListener("pointerup", onPointerUp);
    window.addEventListener("pointercancel", onPointerCancel);
    window.addEventListener("keydown", onKeyDown);
    return () => {
      window.removeEventListener("pointermove", onPointerMove);
      window.removeEventListener("pointerup", onPointerUp);
      window.removeEventListener("pointercancel", onPointerCancel);
      window.removeEventListener("keydown", onKeyDown);
    };
  }, [performDrop, resolveDropTarget]);

  // Hierarchical Bone Subtree (displays #index, gid, and local ID — never fabricated names)
  const renderBoneSubtree = (
    bone: BoneDesc,
    boneIdx: number,
    depth: number,
    ancestors: Set<number> = new Set()
  ) => {
    if (ancestors.has(boneIdx)) return null;
    const nextAncestors = new Set(ancestors);
    nextAncestors.add(boneIdx);
    const bId = getBoneSceneId(bone, boneIdx);
    const kids = boneChildrenMap.childrenByParentIdx.get(boneIdx) ?? [];
    const hasKids = kids.length > 0;
    const q = filterText.trim().toLowerCase();
    const isExp = expandedKeys.has(bId) || q.length > 0;
    const sel = isSelected("bone", bId);
    const ind = dragIndicator?.targetId === bId ? dragIndicator.position : null;
    const localIdx = bone.localId ?? boneIdx;

    return (
      <div key={bId} className="wmb-tree-node-group">
        <div
          data-outliner-kind="bone"
          data-outliner-id={bId}
          data-outliner-order={boneIdx}
          onPointerDown={(e) =>
            beginPointerDrag(e, "bone", bId, "sec:bones", `Bone #${boneIdx}`)
          }
          className={`wmb-tree-row ${sel ? "selected" : ""} ${
            ind ? `drop-${ind}` : ""
          }`}
          style={{ paddingLeft: `${depth * 14 + 8}px` }}
          data-tooltip={`Bone #${boneIdx} · Global ID ${bone.globalId} · Local ID ${localIdx} · Parent ${
            bone.parent >= 0 ? `#${bone.parent}` : "None (Root)"
          } — Drag onto another bone to reparent`}
          onClick={(e) =>
            handleRowClick({ kind: "bone", id: bId }, "sec:bones", e)
          }
          onContextMenu={(e) => {
            e.preventDefault();
            e.stopPropagation();
            onSelect([{ kind: "bone", id: bId }], false);
            setContextMenu({
              x: e.clientX,
              y: e.clientY,
              targetType: "bone",
              bone,
            });
          }}
        >
          <span
            className={`wmb-tree-caret ${hasKids ? "clickable" : "leaf"}`}
            onClick={(e) => (hasKids ? toggleExpand(bId, e) : undefined)}
          >
            {hasKids ? <IconChevronRight expanded={isExp} /> : "·"}
          </span>
          <span className="tree-badge">#{boneIdx}</span>
          <span className="wmb-tree-label mono">
            gid {bone.globalId}{" "}
            <span className="wmb-tree-meta">
              · local {localIdx}
              {hasKids ? ` · ${kids.length}` : ""}
            </span>
          </span>
        </div>

        {hasKids && (
          <AnimatedBranch expanded={isExp} scrollSelector=".wmb-tree-scroll" animate={!q && !suppressBranchAnim}>
              {kids.map((child) =>
                renderBoneSubtree(child.bone, child.idx, depth + 1, nextAncestors)
              )}
            </AnimatedBranch>
        )}
      </div>
    );
  };

  const renderBatchRow = (
    b: BatchDesc,
    orderInParent: number,
    parentGroupKey: string,
    depth: number
  ) => {
    if (b.isShadowProxy && !showShadowProxies) return null;
    const q = filterText.trim().toLowerCase();
    const mat = materialById.get(b.materialId);
    const labelStr = `batch #${b.index} ${mat?.name ?? `mat${b.materialIndex}`}`;
    if (q && !labelStr.toLowerCase().includes(q)) return null;

    const sel = isSelected("batch", b.id);
    const triCount = Math.floor(b.indexCount / 3);
    const isHidden = hiddenBatchIds.has(b.id);
    const ind =
      dragIndicator?.targetId === b.id ? dragIndicator.position : null;

    return (
      <div
        key={b.id}
        data-outliner-kind="batch"
        data-outliner-id={b.id}
        data-outliner-order={orderInParent}
        onPointerDown={(e) =>
          beginPointerDrag(
            e,
            "batch",
            b.id,
            parentGroupKey,
            `Batch #${b.index}`
          )
        }
        className={`wmb-tree-row ${sel ? "selected" : ""} ${
          b.isShadowProxy ? "shadow-proxy" : ""
        } ${ind ? `drop-${ind}` : ""}`}
        style={{ paddingLeft: `${depth * 14 + 8}px` }}
        data-tooltip={`Batch #${b.index} (${triCount.toLocaleString()} tris) — Drag to reorder or move across meshes/LODs`}
        onClick={(e) =>
          handleRowClick({ kind: "batch", id: b.id }, parentGroupKey, e)
        }
        onContextMenu={(e) => {
          e.preventDefault();
          e.stopPropagation();
          if (!sel) onSelect([{ kind: "batch", id: b.id }], false);
          setContextMenu({
            x: e.clientX,
            y: e.clientY,
            targetType: "batch",
            batch: b,
          });
        }}
      >
        <span className="wmb-tree-caret leaf">·</span>
        <span className="tree-badge">#{b.index}</span>
        <span className="wmb-tree-label">
          Batch #{b.index}{" "}
          <span className="wmb-tree-meta">
            {triCount.toLocaleString()} tris
          </span>
          {b.isShadowProxy && (
            <span className="wmb-shadow-badge">shadow</span>
          )}
        </span>

        {(b.indexCount === 0 || b.vertexCount === 0) && (
          <span
            className="wmb-shader-warn-badge"
            data-tooltip="Batch has 0 valid triangles or degenerate indices — skipped in viewport"
          >
            ⚠
          </span>
        )}

        <MaterialPickerPopover
          compact
          materials={manifest.materials}
          textures={manifest.textures}
          currentMatIdx={b.materialIndex}
          onChange={(nextMatIdx) => {
            void onMutateOp({
              op: "set_batch_material",
              batchId: b.id,
              materialIndex: nextMatIdx,
            });
          }}
        />

        <button
          type="button"
          className={`wmb-tree-eye-btn ${isHidden ? "is-hidden" : ""}`}
          data-tooltip={isHidden ? "Show batch in viewport" : "Hide batch in viewport"}
          onClick={(e) => {
            e.stopPropagation();
            onToggleBatchVisibility(b.id);
          }}
        >
          <IconEye visible={!isHidden} />
        </button>
      </div>
    );
  };

  const shadowCount = useMemo(
    () => manifest.batches.filter((b) => b.isShadowProxy).length,
    [manifest.batches]
  );

  const qActive = filterText.trim().length > 0;
  const rootExpanded = expandedKeys.has("root") || qActive;
  const texExpanded = expandedKeys.has("sec:textures") || qActive;
  const matExpanded = expandedKeys.has("sec:materials") || qActive;
  const bonesExpanded = expandedKeys.has("sec:bones") || qActive;
  const meshesExpanded = expandedKeys.has("sec:meshes") || qActive;

  const showTexturesSection = domainFilter === "all" || domainFilter === "textures";
  const showMaterialsSection = domainFilter === "all" || domainFilter === "materials";
  const showBonesSection = domainFilter === "all" || domainFilter === "bones";
  const showMeshesSection = domainFilter === "all" || domainFilter === "meshes";

  return (
    <div
      ref={treeContainerRef}
      className="wmb-hierarchy-shell"
      tabIndex={0}
      onKeyDown={(e) => void handleKeyDown(e)}
      onClickCapture={(e) => {
        if (performance.now() < suppressClickUntilRef.current) {
          e.preventDefault();
          e.stopPropagation();
        }
      }}
    >
      {dragGhost && (
        <div
          className="pointer-drag-ghost"
          style={{ left: dragGhost.x + 14, top: dragGhost.y + 12 }}
        >
          <span>{dragGhost.label}</span>
          <span
            style={{
              color: "var(--brush-muted)",
              fontWeight: 400,
              fontSize: 11,
            }}
          >
            Drag to move
          </span>
        </div>
      )}
      {/* Top Panel Header + Search */}
      <div className="wmb-panel-header">
        <span className="wmb-panel-title">Scene Outliner</span>
        <div className="wmb-hierarchy-controls">
          <input
            type="text"
            className="wmb-hierarchy-search"
            placeholder="Filter scene…"
            value={filterText}
            onChange={(e) => setFilterText(e.target.value)}
          />
          <button
            type="button"
            className="wmb-mini-btn"
            onClick={collapseAll}
            data-tooltip="Collapse all sections, meshes, LODs, and bone subtrees"
          >
            Collapse
          </button>
        </div>
      </div>

      {/* Segmented Domain Filter Strip */}
      <div className="domain-filter-strip">
        <button
          type="button"
          className={`domain-pill ${domainFilter === "all" ? "active" : ""}`}
          onClick={() => handleDomainSelect("all")}
        >
          All
        </button>
        <button
          type="button"
          className={`domain-pill ${domainFilter === "meshes" ? "active" : ""}`}
          onClick={() => handleDomainSelect("meshes")}
        >
          Meshes <span className="cnt">{manifest.meshGroups.length}</span>
        </button>
        <button
          type="button"
          className={`domain-pill ${domainFilter === "materials" ? "active" : ""}`}
          onClick={() => handleDomainSelect("materials")}
        >
          Mats <span className="cnt">{manifest.materials.length}</span>
        </button>
        <button
          type="button"
          className={`domain-pill ${domainFilter === "bones" ? "active" : ""}`}
          onClick={() => handleDomainSelect("bones")}
        >
          Bones <span className="cnt">{manifest.bones.length}</span>
        </button>
        <button
          type="button"
          className={`domain-pill ${domainFilter === "textures" ? "active" : ""}`}
          onClick={() => handleDomainSelect("textures")}
        >
          Tex <span className="cnt">{manifest.textures.length}</span>
        </button>
      </div>

      {/* Clarified Shadow-Proxy Filter Bar */}
      {shadowCount > 0 && showMeshesSection && (
        <div className="outliner-subbar">
          <span className="outliner-subbar-label">
            Shadow proxies: <strong>{showShadowProxies ? "Shown" : "Hidden"}</strong>
          </span>
          <button
            type="button"
            className={`wmb-mini-btn shadow-toggle-btn ${
              showShadowProxies ? "active" : ""
            }`}
            onClick={() => setShowShadowProxies((p) => !p)}
            data-tooltip="PlatinumGames WMB models include low-poly duplicate shadow-caster meshes (isShadowProxy = true). Toggle whether they appear in the outliner."
          >
            <IconShadow />
            <span>
              {showShadowProxies
                ? `Hide Shadow Proxies (${shadowCount})`
                : `Show Shadow Proxies (${shadowCount})`}
            </span>
          </button>
        </div>
      )}

      <div key={manifest.nodeId} className="wmb-tree-scroll">
        {/* Root Node: <wmb name> */}
        <div
          className={`wmb-tree-row root-row ${
            isSelected("root", "root") ? "selected" : ""
          }`}
          onClick={(e) =>
            handleRowClick({ kind: "root", id: "root" }, "root", e)
          }
          onContextMenu={(e) => {
            e.preventDefault();
            e.stopPropagation();
            onSelect([{ kind: "root", id: "root" }], false);
            setContextMenu({
              x: e.clientX,
              y: e.clientY,
              targetType: "root",
            });
          }}
        >
          <span
            className="wmb-tree-caret clickable"
            onClick={(e) => toggleExpand("root", e)}
          >
            <IconChevronRight expanded={rootExpanded} />
          </span>
          <span className="tree-badge">{manifest.game.toUpperCase()}</span>
          <span className="wmb-tree-label">
            <strong>{manifest.fileName}</strong>{" "}
            <span className="wmb-tree-meta">
              ({manifest.batches.length} batches)
            </span>
          </span>
        </div>

        <AnimatedBranch expanded={rootExpanded} scrollSelector=".wmb-tree-scroll" animate={!qActive && !suppressBranchAnim}>
            {/* Section 1: Textures (N) */}
            {showTexturesSection && (
              <div className="wmb-tree-node-group">
                <div
                  className="wmb-tree-row section-row"
                  style={{ paddingLeft: "16px" }}
                  onClick={(e) => toggleExpand("sec:textures", e)}
                  onContextMenu={(e) => {
                    e.preventDefault();
                    e.stopPropagation();
                    setContextMenu({
                      x: e.clientX,
                      y: e.clientY,
                      targetType: "texturesHeader",
                    });
                  }}
                >
                  <span className="wmb-tree-caret clickable">
                    <IconChevronRight expanded={texExpanded} />
                  </span>
                  <span className="wmb-tree-label">
                    Textures{" "}
                    <span className="wmb-tree-meta">
                      ({manifest.textures.length})
                    </span>
                  </span>
                </div>

                <AnimatedBranch expanded={texExpanded} scrollSelector=".wmb-tree-scroll" animate={!qActive && !suppressBranchAnim}>
                    {manifest.textures.map((tex, texIdx) => {
                      const q = filterText.trim().toLowerCase();
                      const role =
                        textureRoles.get(tex.textureId >>> 0) ||
                        (tex.usedByMaterialIds.length > 0 ? "Bound" : "Unused");
                      const hasRealBntxName = !isSyntheticTexName(tex.name);
                      if (
                        q &&
                        !`#${texIdx}`.includes(q) &&
                        !tex.idHex.toLowerCase().includes(q) &&
                        !role.toLowerCase().includes(q) &&
                        !(hasRealBntxName && tex.name.toLowerCase().includes(q))
                      ) {
                        return null;
                      }
                      const tid = `tex:${tex.textureId}`;
                      const sel = isSelected("texture", tid);
                      return (
                        <div
                          key={tid}
                          data-outliner-kind="texture"
                          data-outliner-id={tid}
                          onPointerDown={(e) =>
                            beginPointerDrag(
                              e,
                              "texture",
                              tid,
                              "sec:textures",
                              `Texture ${tex.idHex}`
                            )
                          }
                          className={`wmb-tree-row ${sel ? "selected" : ""}`}
                          style={{ paddingLeft: "30px" }}
                          data-tooltip={`Texture #${texIdx} (${tex.idHex}) · ${tex.width}×${tex.height} ${tex.formatName} — Drag onto a material to bind`}
                          onClick={(e) =>
                            handleRowClick(
                              { kind: "texture", id: tid },
                              "sec:textures",
                              e
                            )
                          }
                          onContextMenu={(e) => {
                            e.preventDefault();
                            e.stopPropagation();
                            onSelect([{ kind: "texture", id: tid }], false);
                            setContextMenu({
                              x: e.clientX,
                              y: e.clientY,
                              targetType: "texture",
                              texture: tex,
                            });
                          }}
                        >
                          {tex.thumbnailDataUrl ? (
                            <img
                              src={tex.thumbnailDataUrl}
                              alt={tex.idHex}
                              className="wmb-tree-thumb"
                              draggable={false}
                            />
                          ) : (
                            <span className="wmb-tree-thumb-fallback">TX</span>
                          )}
                          <span className="tree-badge">#{texIdx}</span>
                          <span className="wmb-tree-label">
                            <span className="mono">{tex.idHex}</span>
                            {hasRealBntxName ? ` · ${tex.name}` : ""}
                            <span className="tex-role-pill">{role}</span>
                          </span>
                          <span className="wmb-tree-submeta">
                            {tex.width}×{tex.height} · {tex.formatName}
                          </span>
                        </div>
                      );
                    })}
                  </AnimatedBranch>
              </div>
            )}

            {/* Section 2: Materials (M) */}
            {showMaterialsSection && (
              <div className="wmb-tree-node-group">
                <div
                  className="wmb-tree-row section-row"
                  style={{ paddingLeft: "16px" }}
                  onClick={(e) => toggleExpand("sec:materials", e)}
                  onContextMenu={(e) => {
                    e.preventDefault();
                    e.stopPropagation();
                    setContextMenu({
                      x: e.clientX,
                      y: e.clientY,
                      targetType: "materialsHeader",
                    });
                  }}
                >
                  <span className="wmb-tree-caret clickable">
                    <IconChevronRight expanded={matExpanded} />
                  </span>
                  <span className="wmb-tree-label">
                    Materials{" "}
                    <span className="wmb-tree-meta">
                      ({manifest.materials.length})
                    </span>
                  </span>
                  <button
                    type="button"
                    className="wmb-tree-add-btn"
                    data-tooltip="Add new material"
                    onClick={(e) => {
                      e.stopPropagation();
                      void onMutateOp({ op: "add_material" });
                    }}
                  >
                    +
                  </button>
                </div>

                <AnimatedBranch expanded={matExpanded} scrollSelector=".wmb-tree-scroll" animate={!qActive && !suppressBranchAnim}>
                    {manifest.materials.map((mat, matIdx) => {
                      const q = filterText.trim().toLowerCase();
                      const shLabel =
                        mat.shaderName ??
                        (mat.materialType !== null
                          ? `type_${mat.materialType}`
                          : "unmapped");
                      if (
                        q &&
                        !mat.name.toLowerCase().includes(q) &&
                        !shLabel.toLowerCase().includes(q) &&
                        !`#${mat.index}`.includes(q)
                      ) {
                        return null;
                      }
                      const sel = isSelected("material", mat.id);
                      const status = shaderStatuses.get(mat.id);
                      const showWarn = status
                        ? status.isFallback || status.hasCompileError
                        : false;
                      const warnTitle = status?.hasCompileError
                        ? `Shader ${shLabel} failed GLSL compilation — rendered with fallback lighting`
                        : `Shader ${shLabel} not yet implemented — rendered with fallback lighting`;
                      const ind =
                        dragIndicator?.targetId === mat.id
                          ? dragIndicator.position
                          : null;
                      const hexBadge = `0x${mat.index
                        .toString(16)
                        .toUpperCase()
                        .padStart(2, "0")}`;

                      return (
                        <div
                          key={mat.id}
                          data-outliner-kind="material"
                          data-outliner-id={mat.id}
                          data-outliner-order={matIdx}
                          onPointerDown={(e) =>
                            beginPointerDrag(
                              e,
                              "material",
                              mat.id,
                              "sec:materials",
                              `Mat #${mat.index}`
                            )
                          }
                          className={`wmb-tree-row ${sel ? "selected" : ""} ${
                            ind ? `drop-${ind}` : ""
                          }`}
                          style={{ paddingLeft: "30px" }}
                          data-tooltip={`Material #${mat.index} (${shLabel}) — Drag to reorder or drop onto a batch/mesh group`}
                          onClick={(e) =>
                            handleRowClick(
                              { kind: "material", id: mat.id },
                              "sec:materials",
                              e
                            )
                          }
                          onContextMenu={(e) => {
                            e.preventDefault();
                            e.stopPropagation();
                            onSelect([{ kind: "material", id: mat.id }], false);
                            setContextMenu({
                              x: e.clientX,
                              y: e.clientY,
                              targetType: "material",
                              material: mat,
                            });
                          }}
                        >
                          {mat.thumbnailDataUrl ? (
                            <img
                              src={mat.thumbnailDataUrl}
                              alt={mat.name}
                              className="wmb-tree-thumb"
                              draggable={false}
                            />
                          ) : (
                            <span className="wmb-tree-thumb-fallback">
                              M{mat.index}
                            </span>
                          )}
                          <span className="tree-badge">{hexBadge}</span>
                          <span className="wmb-tree-label">
                            Mat #{mat.index}{" "}
                            <span className="wmb-tree-shader">({shLabel})</span>
                          </span>
                          <span className="wmb-tree-submeta">
                            {mat.usedByBatchIds.length}b
                          </span>
                          {showWarn && (
                            <span
                              className="wmb-shader-warn-badge"
                              data-tooltip={warnTitle}
                            >
                              ⚠
                            </span>
                          )}
                        </div>
                      );
                    })}
                  </AnimatedBranch>
              </div>
            )}

            {/* Section 3: Bones (K) — Hierarchical tree with drag-and-drop reparenting */}
            {showBonesSection && (
              <div className="wmb-tree-node-group">
                <div
                  data-outliner-kind="bone"
                  data-outliner-id="sec:bones"
                  className={`wmb-tree-row section-row ${
                    dragIndicator?.targetId === "sec:bones"
                      ? `drop-${dragIndicator.position}`
                      : ""
                  }`}
                  style={{ paddingLeft: "16px" }}
                  data-tooltip="Bones hierarchy — Drag any bone onto another bone to reparent, or drop here to make it a root bone"
                  onClick={(e) => toggleExpand("sec:bones", e)}
                  onContextMenu={(e) => {
                    e.preventDefault();
                    e.stopPropagation();
                    setContextMenu({
                      x: e.clientX,
                      y: e.clientY,
                      targetType: "bonesHeader",
                    });
                  }}
                >
                  <span className="wmb-tree-caret clickable">
                    <IconChevronRight expanded={bonesExpanded} />
                  </span>
                  <span className="wmb-tree-label">
                    Bones{" "}
                    <span className="wmb-tree-meta">
                      ({manifest.bones.length})
                    </span>
                  </span>
                  <button
                    type="button"
                    className="wmb-tree-add-btn"
                    data-tooltip="Add root bone"
                    onClick={(e) => {
                      e.stopPropagation();
                      void onMutateOp({
                        op: "add_bone",
                        parentGlobalId: -1,
                      });
                    }}
                  >
                    +
                  </button>
                </div>

                <AnimatedBranch expanded={bonesExpanded} scrollSelector=".wmb-tree-scroll" animate={!qActive && !suppressBranchAnim}>
                    {(boneChildrenMap.childrenByParentIdx.get(-1) ?? []).map(
                      (rootBone) =>
                        renderBoneSubtree(rootBone.bone, rootBone.idx, 2)
                    )}
                  </AnimatedBranch>
              </div>
            )}

            {/* Section 4: Mesh Groups (G) */}
            {showMeshesSection && (
              <div className="wmb-tree-node-group">
                <div
                  className="wmb-tree-row section-row"
                  style={{ paddingLeft: "16px" }}
                  onClick={(e) => toggleExpand("sec:meshes", e)}
                  onContextMenu={(e) => {
                    e.preventDefault();
                    e.stopPropagation();
                    setContextMenu({
                      x: e.clientX,
                      y: e.clientY,
                      targetType: "meshGroupsHeader",
                    });
                  }}
                >
                  <span className="wmb-tree-caret clickable">
                    <IconChevronRight expanded={meshesExpanded} />
                  </span>
                  <span className="wmb-tree-label">
                    Mesh Groups{" "}
                    <span className="wmb-tree-meta">
                      ({manifest.meshGroups.length})
                    </span>
                  </span>
                  <button
                    type="button"
                    className="wmb-tree-add-btn"
                    data-tooltip="Add new mesh group"
                    onClick={(e) => {
                      e.stopPropagation();
                      void onMutateOp({
                        op: "add_mesh_group",
                        name: `mesh_${manifest.meshGroups.length}`,
                      });
                    }}
                  >
                    +
                  </button>
                </div>

                <AnimatedBranch expanded={meshesExpanded} scrollSelector=".wmb-tree-scroll" animate={!qActive && !suppressBranchAnim}>
                    {manifest.meshGroups.map((mg, mgIdx) => {
                      const isExp = expandedKeys.has(mg.id) || qActive;
                      const sel = isSelected("meshGroup", mg.id);
                      const isRenaming = renamingId === mg.id;
                      const isHidden = hiddenMeshGroupIds.has(mg.id);
                      const ind =
                        dragIndicator?.targetId === mg.id
                          ? dragIndicator.position
                          : null;
                      const totalBatches =
                        mg.lods.length > 0
                          ? mg.lods.reduce(
                              (acc, l) => acc + l.batchIds.length,
                              0
                            )
                          : mg.batchIds.length;

                      return (
                        <div key={mg.id} className="wmb-tree-node-group">
                          <div
                            data-outliner-kind="meshGroup"
                            data-outliner-id={mg.id}
                            data-outliner-order={mgIdx}
                            onPointerDown={(e) =>
                              beginPointerDrag(
                                e,
                                "meshGroup",
                                mg.id,
                                "sec:meshes",
                                mg.name
                              )
                            }
                            className={`wmb-tree-row ${sel ? "selected" : ""} ${
                              ind ? `drop-${ind}` : ""
                            }`}
                            style={{ paddingLeft: "28px" }}
                            onClick={(e) =>
                              handleRowClick(
                                { kind: "meshGroup", id: mg.id },
                                "sec:meshes",
                                e
                              )
                            }
                            onDoubleClick={() =>
                              beginInlineRename(mg.id, mg.name)
                            }
                            onContextMenu={(e) => {
                              e.preventDefault();
                              e.stopPropagation();
                              onSelect(
                                [{ kind: "meshGroup", id: mg.id }],
                                false
                              );
                              setContextMenu({
                                x: e.clientX,
                                y: e.clientY,
                                targetType: "meshGroup",
                                meshGroup: mg,
                              });
                            }}
                          >
                            <span
                              className="wmb-tree-caret clickable"
                              onClick={(e) => toggleExpand(mg.id, e)}
                            >
                              <IconChevronRight expanded={isExp} />
                            </span>
                            <span className="tree-badge">#{mg.index}</span>
                            {isRenaming ? (
                              <input
                                type="text"
                                className="wmb-tree-rename-input"
                                value={renameDraft}
                                autoFocus
                                onChange={(e) =>
                                  setRenameDraft(e.target.value)
                                }
                                onBlur={() => void commitInlineRename()}
                                onKeyDown={(e) => {
                                  if (e.key === "Enter")
                                    void commitInlineRename();
                                  else if (e.key === "Escape")
                                    setRenamingId(null);
                                }}
                                onClick={(e) => e.stopPropagation()}
                              />
                            ) : (
                              <span className="wmb-tree-label">
                                <strong>{mg.name}</strong>{" "}
                                <span className="wmb-tree-meta">
                                  ({totalBatches} batches)
                                </span>
                              </span>
                            )}
                            <button
                              type="button"
                              className={`wmb-tree-eye-btn ${
                                isHidden ? "is-hidden" : ""
                              }`}
                              data-tooltip={
                                isHidden
                                  ? "Show mesh group in viewport"
                                  : "Hide mesh group in viewport"
                              }
                              onClick={(e) => {
                                e.stopPropagation();
                                onToggleMeshGroupVisibility(mg.id);
                              }}
                            >
                              <IconEye visible={!isHidden} />
                            </button>
                          </div>

                          <AnimatedBranch expanded={isExp} scrollSelector=".wmb-tree-scroll" animate={!qActive && !suppressBranchAnim}>
                              {mg.lods.length > 0
                                ? mg.lods.map((lod) => {
                                    const lodExp =
                                      expandedKeys.has(lod.id) || qActive;
                                    const lodSel = isSelected("lod", lod.id);
                                    const lodRenaming = renamingId === lod.id;
                                    const lodInd =
                                      dragIndicator?.targetId === lod.id
                                        ? dragIndicator.position
                                        : null;
                                    return (
                                      <div
                                        key={lod.id}
                                        className="wmb-tree-node-group"
                                      >
                                        <div
                                          data-outliner-kind="lod"
                                          data-outliner-id={lod.id}
                                          className={`wmb-tree-row ${
                                            lodSel ? "selected" : ""
                                          } ${lodInd ? `drop-${lodInd}` : ""}`}
                                          style={{ paddingLeft: "42px" }}
                                          onClick={(e) =>
                                            handleRowClick(
                                              { kind: "lod", id: lod.id },
                                              mg.id,
                                              e
                                            )
                                          }
                                          onDoubleClick={() =>
                                            beginInlineRename(lod.id, lod.name)
                                          }
                                          onContextMenu={(e) => {
                                            e.preventDefault();
                                            e.stopPropagation();
                                            onSelect(
                                              [{ kind: "lod", id: lod.id }],
                                              false
                                            );
                                            setContextMenu({
                                              x: e.clientX,
                                              y: e.clientY,
                                              targetType: "lod",
                                              meshGroup: mg,
                                              lod,
                                            });
                                          }}
                                        >
                                          <span
                                            className="wmb-tree-caret clickable"
                                            onClick={(e) =>
                                              toggleExpand(lod.id, e)
                                            }
                                          >
                                            <IconChevronRight
                                              expanded={lodExp}
                                            />
                                          </span>
                                          <span className="tree-badge">
                                            L{lod.level}
                                          </span>
                                          {lodRenaming ? (
                                            <input
                                              type="text"
                                              className="wmb-tree-rename-input"
                                              value={renameDraft}
                                              autoFocus
                                              onChange={(e) =>
                                                setRenameDraft(e.target.value)
                                              }
                                              onBlur={() =>
                                                void commitInlineRename()
                                              }
                                              onKeyDown={(e) => {
                                                if (e.key === "Enter")
                                                  void commitInlineRename();
                                                else if (e.key === "Escape")
                                                  setRenamingId(null);
                                              }}
                                              onClick={(e) =>
                                                e.stopPropagation()
                                              }
                                            />
                                          ) : (
                                            <span className="wmb-tree-label">
                                              LOD {lod.level} "{lod.name}"{" "}
                                              <span className="wmb-tree-meta">
                                                ({lod.batchIds.length} batches)
                                              </span>
                                            </span>
                                          )}
                                        </div>

                                        <AnimatedBranch expanded={lodExp} scrollSelector=".wmb-tree-scroll" animate={!qActive && !suppressBranchAnim}>
                                            {lod.batchIds.map((bid, bOrder) => {
                                              const b = batchById.get(bid);
                                              return b
                                                ? renderBatchRow(
                                                    b,
                                                    bOrder,
                                                    lod.id,
                                                    4
                                                  )
                                                : null;
                                            })}
                                          </AnimatedBranch>
                                      </div>
                                    );
                                  })
                                : mg.batchIds.map((bid, bOrder) => {
                                    const b = batchById.get(bid);
                                    return b
                                      ? renderBatchRow(b, bOrder, mg.id, 3)
                                      : null;
                                  })}
                            </AnimatedBranch>
                        </div>
                      );
                    })}
                  </AnimatedBranch>
              </div>
            )}
          </AnimatedBranch>
      </div>

      {/* Texture Drop onto Material -> Slot Picker Popover */}
      <HierarchyModals
        activeSlotDrop={activeSlotDrop}
        isSlotDropClosing={isSlotDropClosing}
        matDeleteModal={matDeleteModal}
        materials={manifest.materials}
        onCloseSlotDrop={() => setSlotDropModal(null)}
        onMutateOp={onMutateOp}
        onChangeReassignIndex={(nextIndex) =>
          setMatDeleteModal((prev) => (prev ? { ...prev, reassignToIndex: nextIndex } : null))
        }
        onCloseMatDelete={() => setMatDeleteModal(null)}
      />

      <HierarchyContextMenu
        menu={activeCtxMenu}
        isClosing={isCtxMenuClosing}
        manifest={manifest}
        selection={selection}
        clipboardStatus={clipboardStatus}
        onClose={() => setContextMenu(null)}
        onSelect={onSelect}
        onMutateOp={onMutateOp}
        onCopyItems={onCopyItems}
        onPasteInto={onPasteInto}
        onConvertMaterialProfile={onConvertMaterialProfile}
        onOpenLegacyFlow={onOpenLegacyFlow}
        beginInlineRename={beginInlineRename}
        triggerDeleteSelected={triggerDeleteSelected}
      />
    </div>
  );
});
