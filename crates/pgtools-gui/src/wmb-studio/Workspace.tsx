// [wmb-studio BEGIN phase2]
import React, {
  useCallback,
  useEffect,
  useRef,
  useState,
} from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  ClipboardStatus,
  getWmbScene,
  MaterialDesc,
  resolveLegacyNode,
  wmbClipboardCopy,
  WmbSceneManifest,
} from "./contract";
import { canPasteClipboard, refreshClipboardStatus } from "./clipboard";
import { Hierarchy, LegacyActionKind } from "./Hierarchy";
import { Properties } from "./Properties";
import {
  createGeometryCache,
  DebugSlotPreview,
  filterValidSelection,
  findBatchById,
  findMaterialById,
  getKnownGeometryHashes,
  mergeScenePayloadIntoCache,
  SceneSelectionItem,
} from "./store";
import { Viewport, ViewportHandle } from "./Viewport";
import { MaterialShaderStatus } from "./viewer/sceneBuilder";
import { DocumentOutline } from "../types";

export interface WorkspaceProps {
  nodeId: string;
  docGeneration: number;
  theme: string;
  rightPanelWidth: number;
  onStartResizeRightPanel: (e: React.MouseEvent) => void;
  hierarchyRatio: number;
  onChangeHierarchyRatio: (ratio: number) => void;
  wireframe: boolean;
  onToggleWireframe: () => void;
  normalsView: boolean;
  onToggleNormalsView: () => void;
  skeletonOverlay: boolean;
  onToggleSkeletonOverlay: () => void;
  gridVisible: boolean;
  onToggleGridVisible: () => void;
  onOutlineUpdated: (outline: DocumentOutline, statusMsg?: string) => void;
  onStatusError: (msg: string) => void;
  onTriggerLegacyAction: (
    legacyNodeId: string,
    action: LegacyActionKind
  ) => Promise<void>;
}

export const Workspace: React.FC<WorkspaceProps> = React.memo(function Workspace({
  nodeId,
  docGeneration,
  theme,
  rightPanelWidth,
  onStartResizeRightPanel,
  hierarchyRatio,
  onChangeHierarchyRatio,
  wireframe,
  onToggleWireframe,
  normalsView,
  onToggleNormalsView,
  skeletonOverlay,
  onToggleSkeletonOverlay,
  gridVisible,
  onToggleGridVisible,
  onOutlineUpdated,
  onStatusError,
  onTriggerLegacyAction,
}: WorkspaceProps) {
  const viewportRef = useRef<ViewportHandle | null>(null);
  const rightColRef = useRef<HTMLDivElement | null>(null);
  const geomCacheRef = useRef(createGeometryCache());
  const prevNodeIdRef = useRef<string>("");
  const activeNodeIdRef = useRef<string>(nodeId);
  const refreshSeqRef = useRef<number>(0);

  const [manifest, setManifest] = useState<WmbSceneManifest | null>(null);
  const manifestRef = useRef<WmbSceneManifest | null>(null);
  manifestRef.current = manifest;

  const [loading, setLoading] = useState<boolean>(false);
  const [selection, setSelection] = useState<SceneSelectionItem[]>([
    { kind: "root", id: "root" },
  ]);
  const [shaderStatuses, setShaderStatuses] = useState<
    Map<string, MaterialShaderStatus>
  >(new Map());
  const [clipboardStatus, setClipboardStatus] =
    useState<ClipboardStatus | null>(null);
  const [activeLodLevel, setActiveLodLevel] = useState<number | null>(null);
  const [debugSlot, setDebugSlot] = useState<DebugSlotPreview | null>(null);
  const [hiddenBatchIds, setHiddenBatchIds] = useState<Set<string>>(
    () => new Set()
  );
  const [hiddenMeshGroupIds, setHiddenMeshGroupIds] = useState<Set<string>>(
    () => new Set()
  );

  // Clear geometry cache and invalidate in-flight requests when switching to a different model node
  useEffect(() => {
    activeNodeIdRef.current = nodeId;
    if (prevNodeIdRef.current !== nodeId) {
      prevNodeIdRef.current = nodeId;
      refreshSeqRef.current += 1;
      geomCacheRef.current = createGeometryCache();
      setActiveLodLevel(null);
      setDebugSlot(null);
      setHiddenBatchIds(new Set());
      setHiddenMeshGroupIds(new Set());
      setSelection([{ kind: "root", id: "root" }]);
    }
  }, [nodeId]);

  const refreshScene = useCallback(async () => {
    if (!nodeId) return;
    const reqId = ++refreshSeqRef.current;
    const reqNodeId = nodeId;
    setLoading((prev) => prev || manifestRef.current === null);
    try {
      const known = getKnownGeometryHashes(geomCacheRef.current);
      const payload = await getWmbScene(reqNodeId, known);
      if (
        reqId !== refreshSeqRef.current ||
        reqNodeId !== activeNodeIdRef.current
      ) {
        return;
      }
      mergeScenePayloadIntoCache(payload, geomCacheRef.current);
      setManifest(payload.manifest);
      setSelection((prev) => filterValidSelection(payload.manifest, prev));
      const clip = await refreshClipboardStatus();
      if (
        reqId === refreshSeqRef.current &&
        reqNodeId === activeNodeIdRef.current
      ) {
        setClipboardStatus(clip);
      }
    } catch (err) {
      if (
        reqId === refreshSeqRef.current &&
        reqNodeId === activeNodeIdRef.current
      ) {
        onStatusError(String(err));
      }
    } finally {
      if (
        reqId === refreshSeqRef.current &&
        reqNodeId === activeNodeIdRef.current
      ) {
        setLoading(false);
      }
    }
  }, [nodeId, onStatusError]);

  useEffect(() => {
    void refreshScene();
  }, [nodeId, docGeneration, refreshScene]);

  // Vertical splitter drag inside right column (Hierarchy top / Properties bottom)
  const handleStartVerticalSplitDrag = useCallback(
    (e: React.MouseEvent) => {
      e.preventDefault();
      const col = rightColRef.current;
      if (!col) return;
      const rect = col.getBoundingClientRect();

      const onMove = (ev: MouseEvent) => {
        if (rect.height <= 0) return;
        const ratio = (ev.clientY - rect.top) / rect.height;
        const clamped = Math.max(0.18, Math.min(0.82, ratio));
        onChangeHierarchyRatio(clamped);
      };
      const onUp = () => {
        window.removeEventListener("mousemove", onMove);
        window.removeEventListener("mouseup", onUp);
      };
      window.addEventListener("mousemove", onMove);
      window.addEventListener("mouseup", onUp);
    },
    [onChangeHierarchyRatio]
  );

  const handleSelect = useCallback(
    (items: SceneSelectionItem[], additive = false) => {
      setSelection((prev) => {
        if (!additive) {
          return items.length > 0 ? items : [{ kind: "root", id: "root" }];
        }
        const next = [...prev];
        for (const item of items) {
          const idx = next.findIndex(
            (x) => x.kind === item.kind && x.id === item.id
          );
          if (idx >= 0) {
            if (next.length > 1) next.splice(idx, 1);
          } else {
            next.push(item);
          }
        }
        return next;
      });
    },
    []
  );

  const handleDirectWmbMutate = useCallback(
    async (operation: string, payload: Record<string, unknown>) => {
      try {
        const outline = await invoke<DocumentOutline>("mutate_wmb_node", {
          nodeId,
          operation,
          payload,
        });
        // Note: onOutlineUpdated increments docGeneration, which triggers the refreshScene effect (#15)
        onOutlineUpdated(outline, `${operation} applied`);
      } catch (err) {
        onStatusError(String(err));
        void refreshScene();
      }
    },
    [nodeId, onOutlineUpdated, onStatusError, refreshScene]
  );

  const handleMutateOp = useCallback(
    async (opObj: Record<string, unknown>) => {
      if (!manifest) return;
      const op = String(opObj.op ?? "");

      try {
        if (op === "set_batch_material") {
          const batchId = String(opObj.batchId ?? "");
          const materialIndex = Number(opObj.materialIndex ?? 0);
          if (!batchId) return;
          await handleDirectWmbMutate("set_batches_material", {
            items: [batchId],
            material_index: materialIndex,
          });
          return;
        }

        if (op === "set_batches_material") {
          const batchIds = (opObj.batchIds as string[]) ?? [];
          const materialIndex = Number(opObj.materialIndex ?? 0);
          if (batchIds.length === 0) return;
          await handleDirectWmbMutate("set_batches_material", {
            items: batchIds,
            material_index: materialIndex,
          });
          return;
        }

        if (op === "set_mesh_group_material") {
          const meshId = String(opObj.meshId ?? "");
          const materialIndex = Number(opObj.materialIndex ?? 0);
          const mg = manifest.meshGroups.find((m) => m.id === meshId);
          if (!mg) return;
          const allBids =
            mg.lods.length > 0
              ? mg.lods.flatMap((l) => l.batchIds)
              : mg.batchIds;
          if (allBids.length === 0) return;
          await handleDirectWmbMutate("set_batches_material", {
            items: allBids,
            material_index: materialIndex,
          });
          return;
        }

        if (op === "set_material_texture") {
          const materialId = String(opObj.materialId ?? "");
          const mat = findMaterialById(manifest, materialId);
          if (!mat) return;
          await handleDirectWmbMutate("set_material_texture", {
            index: mat.index,
            slot: Number(opObj.slot ?? 0),
            texture_id: Number(opObj.textureId ?? 0) >>> 0,
          });
          return;
        }

        if (op === "move_batch" || op === "move_batches") {
          const batchIds =
            Array.isArray(opObj.batchIds) && opObj.batchIds.length > 0
              ? (opObj.batchIds as string[])
              : opObj.batchId
              ? [String(opObj.batchId)]
              : [];
          const targetParentId = String(opObj.targetParentId ?? "");
          if (batchIds.length === 0 || !targetParentId) return;
          await handleDirectWmbMutate("move_items", {
            items: batchIds,
            target: { id: targetParentId },
          });
          return;
        }

        if (op === "reparent_bone") {
          const boneIndex = Number(opObj.boneIndex ?? -1);
          const parentIndex = Number(opObj.parentIndex ?? -1);
          if (boneIndex < 0) return;
          await handleDirectWmbMutate("update_bone", {
            index: boneIndex,
            parent_index: parentIndex,
          });
          return;
        }

        if (op === "reorder_item") {
          const itemId = String(opObj.itemId ?? "");
          const itemIds =
            Array.isArray(opObj.itemIds) && opObj.itemIds.length > 0
              ? (opObj.itemIds as string[])
              : [itemId];
          const toIndex = Number(opObj.toIndex ?? 0);
          if (itemId.startsWith("mesh:")) {
            await handleDirectWmbMutate("reorder_mesh_groups", {
              ids: itemIds,
              index: toIndex,
            });
          } else if (itemId.startsWith("batch:")) {
            const batch = findBatchById(manifest, itemId);
            const targetParentId =
              typeof opObj.targetParentId === "string" && opObj.targetParentId
                ? opObj.targetParentId
                : batch?.lodId ?? batch?.meshId;
            if (targetParentId) {
              await handleDirectWmbMutate("move_items", {
                items: itemIds,
                target: {
                  id: targetParentId,
                  index: toIndex,
                },
              });
            }
          } else if (itemId.startsWith("mat:")) {
            const mat = findMaterialById(manifest, itemId);
            if (mat) {
              await handleDirectWmbMutate("reorder_materials", {
                from_index: mat.index,
                to_index: toIndex,
              });
            }
          }
          return;
        }

        if (op === "rename_item") {
          const itemId = String(opObj.itemId ?? "");
          const name = String(opObj.name ?? "");
          if (itemId.startsWith("mesh:")) {
            await handleDirectWmbMutate("mesh_group_rename", {
              id: itemId,
              name,
            });
          } else if (itemId.startsWith("lod:")) {
            await handleDirectWmbMutate("update_wmb3_lod", {
              id: itemId,
              name,
            });
          } else if (itemId.startsWith("mat:")) {
            await handleDirectWmbMutate("material_rename", {
              id: itemId,
              name,
            });
          }
          return;
        }

        if (op === "delete_items") {
          const items = (opObj.items as string[]) ?? [];
          if (items.length === 0) return;
          const batchIds = items.filter((id) => id.startsWith("batch:"));
          const matIds = items.filter((id) => id.startsWith("mat:"));
          const meshIds = items.filter((id) => id.startsWith("mesh:"));
          const boneIds = items.filter((id) => id.startsWith("bone:"));

          if (batchIds.length > 0) {
            await handleDirectWmbMutate("delete_items", { items: batchIds });
          }
          if (matIds.length > 0) {
            const reassignIdx = opObj.reassignMaterialTo as number | undefined;
            const reassignMat =
              reassignIdx !== undefined
                ? manifest.materials[reassignIdx]
                : undefined;
            for (const matId of matIds) {
              await handleDirectWmbMutate("material_delete", {
                id: matId,
                reassignToId: reassignMat?.id,
              });
            }
          }
          if (meshIds.length > 0) {
            for (const meshId of meshIds) {
              await handleDirectWmbMutate("mesh_group_delete", { id: meshId });
            }
          }
          if (boneIds.length > 0) {
            for (const boneId of boneIds) {
              const raw = boneId.replace(/^bone:/, "");
              const gidStr = raw.includes(":") ? raw.split(":")[1] : raw;
              const gid = parseInt(gidStr, 10);
              if (!Number.isNaN(gid)) {
                await handleDirectWmbMutate("delete_bone", { global_id: gid });
              }
            }
          }
          return;
        }

        if (op === "duplicate_item") {
          const itemId = String(opObj.itemId ?? "");
          if (itemId.startsWith("mat:")) {
            await handleDirectWmbMutate("material_duplicate", { id: itemId });
          } else if (itemId.startsWith("mesh:")) {
            await handleDirectWmbMutate("mesh_group_duplicate", { id: itemId });
          } else if (itemId.startsWith("batch:")) {
            await handleDirectWmbMutate("duplicate_items", {
              items: [itemId],
            });
          }
          return;
        }

        if (op === "add_material") {
          await handleDirectWmbMutate("material_add", {});
          return;
        }

        if (op === "add_mesh_group") {
          await handleDirectWmbMutate("mesh_group_add", {
            name: String(opObj.name ?? `mesh_${manifest.meshGroups.length}`),
          });
          return;
        }

        if (op === "add_bone") {
          await handleDirectWmbMutate("add_bone", {
            parent_global_id: Number(opObj.parentGlobalId ?? -1),
          });
          return;
        }
      } catch (err) {
        onStatusError(String(err));
      }
    },
    [
      handleDirectWmbMutate,
      manifest,
      nodeId,
      onStatusError,
      refreshScene,
    ]
  );

  const handleCopyItems = useCallback(
    async (itemIds: string[]) => {
      try {
        const status = await wmbClipboardCopy(nodeId, itemIds);
        setClipboardStatus(status);
      } catch (err) {
        onStatusError(String(err));
      }
    },
    [nodeId, onStatusError]
  );

  const handlePasteInto = useCallback(
    async (targetContainerId: string | null) => {
      if (!manifest || !clipboardStatus) return;
      const check = canPasteClipboard(clipboardStatus, manifest.game);
      if (!check.allowed) {
        if (check.reason) onStatusError(check.reason);
        return;
      }
      const targetKind =
        clipboardStatus.kind === "materials"
          ? "materials"
          : targetContainerId?.startsWith("lod:")
          ? "lod"
          : "mesh";
      const fallbackMeshId =
        targetContainerId ??
        manifest.meshGroups[0]?.lods[0]?.id ??
        manifest.meshGroups[0]?.id;
      await handleDirectWmbMutate("paste_items", {
        target: {
          kind: targetKind,
          id: fallbackMeshId,
        },
      });
    },
    [clipboardStatus, handleDirectWmbMutate, manifest]
  );

  const handleConvertMaterialProfile = useCallback(
    async (materialIndex: number, targetGame: "b1" | "b2" | "b3") => {
      if (!manifest) return;
      try {
        const legacyId = await resolveLegacyNode(
          nodeId,
          `mat:${materialIndex}`
        );
        const inspected = await invoke<{
          wmb?: {
            is_wmb3: boolean;
            is_bayo2: boolean;
            material_decoded?: Record<string, unknown>;
            wmb3_material?: Record<string, unknown>;
          };
        }>("inspect_asset_node", {
          nodeId: legacyId,
          mipIndex: 0,
          channel: "RGBA",
          opaque: false,
        });
        const wmbView = inspected.wmb;
        const srcPayload = wmbView?.is_wmb3
          ? wmbView.wmb3_material
          : wmbView?.material_decoded;
        if (wmbView && srcPayload) {
          const srcGame: "b1" | "b2" | "b3" = wmbView.is_wmb3
            ? "b3"
            : wmbView.is_bayo2
            ? "b2"
            : "b1";
          const convertedRes = await invoke<{
            converted: Record<string, unknown>;
          }>("convert_material_action", {
            srcGame,
            dstGame: targetGame,
            materialPayload: srcPayload,
            targetShader: null,
          });
          if (
            !manifest.isWmb3 &&
            (targetGame === "b1" || targetGame === "b2") &&
            convertedRes.converted
          ) {
            await handleDirectWmbMutate("update_material", {
              ...convertedRes.converted,
              index: materialIndex,
            });
            return;
          } else if (
            manifest.isWmb3 &&
            targetGame === "b3" &&
            convertedRes.converted
          ) {
            await handleDirectWmbMutate("update_wmb3_material", {
              index: materialIndex,
              material: convertedRes.converted,
            });
            return;
          }
        }
      } catch {
        // Fall back to switch_shader if full cross-game conversion is cross-format
      }
      const defaultShaderByGame: Record<"b1" | "b2" | "b3", string> = {
        b1: "110",
        b2: "sis00_bxxxn",
        b3: "PBS00_XXXXX",
      };
      await handleDirectWmbMutate("switch_shader", {
        index: materialIndex,
        shader: defaultShaderByGame[targetGame],
        is_bayo2: targetGame === "b2",
      });
    },
    [handleDirectWmbMutate, manifest, nodeId]
  );

  const handleOpenLegacyFlow = useCallback(
    async (itemId: string, action: LegacyActionKind) => {
      try {
        const legacyNodeId = await resolveLegacyNode(nodeId, itemId);
        await onTriggerLegacyAction(legacyNodeId, action);
      } catch (err) {
        onStatusError(String(err));
      }
    },
    [nodeId, onStatusError, onTriggerLegacyAction]
  );

  return (
    <div className="wmb-workspace-root">
      {/* Center 3D Viewport */}
      <div className="wmb-workspace-center">
        {loading && !manifest ? (
          <div className="wmb-workspace-loading">Loading 3D scene…</div>
        ) : (
          <Viewport
            ref={viewportRef}
            manifest={manifest}
            geomCache={geomCacheRef.current}
            selection={selection}
            onSelect={handleSelect}
            theme={theme}
            wireframe={wireframe}
            onToggleWireframe={onToggleWireframe}
            normalsView={normalsView}
            onToggleNormalsView={onToggleNormalsView}
            skeletonOverlay={skeletonOverlay}
            onToggleSkeletonOverlay={onToggleSkeletonOverlay}
            gridVisible={gridVisible}
            onToggleGridVisible={onToggleGridVisible}
            activeLodLevel={activeLodLevel}
            onChangeLodLevel={setActiveLodLevel}
            hiddenBatchIds={hiddenBatchIds}
            hiddenMeshGroupIds={hiddenMeshGroupIds}
            onShaderStatusesChanged={setShaderStatuses}
            debugSlot={debugSlot}
            onChangeDebugSlot={setDebugSlot}
          />
        )}
      </div>

      {/* Splitter between Center Viewport and Right Column */}
      <div
        className="splitter splitter-v"
        onMouseDown={onStartResizeRightPanel}
        title="Drag to resize right inspector column"
      />

      {/* Right Column: Hierarchy (top) + Properties (bottom) */}
      <div
        ref={rightColRef}
        className="wmb-workspace-right"
        style={{ width: `${rightPanelWidth}px` }}
      >
        {manifest ? (
          <>
            <div
              className="wmb-workspace-hierarchy-pane"
              style={{ height: `${Math.round(hierarchyRatio * 100)}%` }}
            >
              <Hierarchy
                manifest={manifest}
                selection={selection}
                onSelect={handleSelect}
                shaderStatuses={shaderStatuses}
                clipboardStatus={clipboardStatus}
                hiddenBatchIds={hiddenBatchIds}
                hiddenMeshGroupIds={hiddenMeshGroupIds}
                onToggleBatchVisibility={(bid) => {
                  setHiddenBatchIds((prev) => {
                    const next = new Set(prev);
                    if (next.has(bid)) next.delete(bid);
                    else next.add(bid);
                    return next;
                  });
                }}
                onToggleMeshGroupVisibility={(mgId) => {
                  setHiddenMeshGroupIds((prev) => {
                    const next = new Set(prev);
                    if (next.has(mgId)) next.delete(mgId);
                    else next.add(mgId);
                    return next;
                  });
                }}
                onMutateOp={handleMutateOp}
                onCopyItems={handleCopyItems}
                onPasteInto={handlePasteInto}
                onConvertMaterialProfile={handleConvertMaterialProfile}
                onOpenLegacyFlow={handleOpenLegacyFlow}
              />
            </div>

            <div
              className="splitter splitter-h"
              onMouseDown={handleStartVerticalSplitDrag}
              title="Drag to resize Hierarchy / Properties split"
            />

            <div
              className="wmb-workspace-properties-pane"
              style={{
                height: `${Math.max(10, 100 - Math.round(hierarchyRatio * 100))}%`,
              }}
            >
              <Properties
                manifest={manifest}
                selection={selection}
                onSelect={handleSelect}
                onLiveMaterialChange={(draftMat: MaterialDesc) => {
                  viewportRef.current?.updateMaterialLive(draftMat);
                }}
                onMutateOp={handleMutateOp}
                onDirectWmbMutate={handleDirectWmbMutate}
                onOpenLegacyFlow={handleOpenLegacyFlow}
                debugSlot={debugSlot}
                onChangeDebugSlot={setDebugSlot}
              />
            </div>
          </>
        ) : (
          <div className="wmb-workspace-loading">No scene manifest loaded.</div>
        )}
      </div>
    </div>
  );
});
// [wmb-studio END]
