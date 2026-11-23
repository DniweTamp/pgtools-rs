import React from "react";
import { canPasteClipboard } from "./clipboard";
import {
  BatchDesc,
  BoneDesc,
  ClipboardStatus,
  LegacyActionKind,
  LodDesc,
  MaterialDesc,
  MeshGroupDesc,
  TextureRefDesc,
  WmbSceneManifest,
} from "./contract";
import { getBoneSceneId, SceneSelectionItem } from "./store";

export interface ContextMenuState {
  x: number;
  y: number;
  targetType:
    | "root"
    | "texturesHeader"
    | "texture"
    | "materialsHeader"
    | "material"
    | "bonesHeader"
    | "bone"
    | "meshGroupsHeader"
    | "meshGroup"
    | "lod"
    | "batch";
  texture?: TextureRefDesc;
  material?: MaterialDesc;
  bone?: BoneDesc;
  meshGroup?: MeshGroupDesc;
  lod?: LodDesc;
  batch?: BatchDesc;
}

interface HierarchyContextMenuProps {
  menu: ContextMenuState | null;
  isClosing: boolean;
  manifest: WmbSceneManifest;
  selection: SceneSelectionItem[];
  clipboardStatus: ClipboardStatus | null;
  onClose: () => void;
  onSelect: (items: SceneSelectionItem[], additive?: boolean) => void;
  onMutateOp: (op: Record<string, unknown>) => Promise<void>;
  onCopyItems: (itemIds: string[]) => Promise<void>;
  onPasteInto: (targetContainerId: string | null) => Promise<void>;
  onConvertMaterialProfile: (
    materialIndex: number,
    targetGame: "b1" | "b2" | "b3"
  ) => Promise<void>;
  onOpenLegacyFlow: (itemId: string, action: LegacyActionKind) => Promise<void>;
  beginInlineRename: (itemId: string, currentName: string) => void;
  triggerDeleteSelected: (itemsToDelete: SceneSelectionItem[]) => Promise<void>;
}

export const HierarchyContextMenu: React.FC<HierarchyContextMenuProps> = ({
  menu,
  isClosing,
  manifest,
  selection,
  clipboardStatus,
  onClose,
  onSelect,
  onMutateOp,
  onCopyItems,
  onPasteInto,
  onConvertMaterialProfile,
  onOpenLegacyFlow,
  beginInlineRename,
  triggerDeleteSelected,
}) => (
  <>
      {menu && (
        <div
          key={`${menu.x}-${menu.y}`}
          className={`wmb-context-menu ${isClosing ? "closing" : ""}`}
          style={{
            left: Math.min(window.innerWidth - 260, menu.x),
            top: Math.min(window.innerHeight - 320, menu.y),
          }}
          onClick={(e) => e.stopPropagation()}
        >
          {menu.targetType === "root" && (
            <>
              <button
                type="button"
                className="wmb-ctx-item"
                disabled={!canPasteClipboard(clipboardStatus, manifest.game).allowed}
                data-tooltip={canPasteClipboard(clipboardStatus, manifest.game).reason}
                onClick={() => {
                  onClose();
                  void onPasteInto(null);
                }}
              >
                Paste{" "}
                {clipboardStatus
                  ? `(${clipboardStatus.count} ${clipboardStatus.kind})`
                  : ""}
              </button>
              <div className="wmb-ctx-sep" />
              <button
                type="button"
                className="wmb-ctx-item"
                onClick={() => {
                  onClose();
                  void onOpenLegacyFlow("root", "import_obj");
                }}
              >
                Import OBJ…
              </button>
              <button
                type="button"
                className="wmb-ctx-item"
                onClick={() => {
                  onClose();
                  void onOpenLegacyFlow("root", "export_obj");
                }}
              >
                Export OBJ…
              </button>
              <button
                type="button"
                className="wmb-ctx-item"
                onClick={() => {
                  onClose();
                  void onOpenLegacyFlow("root", "import_gltf");
                }}
              >
                Import GLTF…
              </button>
              <button
                type="button"
                className="wmb-ctx-item"
                onClick={() => {
                  onClose();
                  void onOpenLegacyFlow("root", "export_gltf");
                }}
              >
                Export GLTF…
              </button>
            </>
          )}

          {menu.targetType === "texturesHeader" && (
            <button
              type="button"
              className="wmb-ctx-item"
              onClick={() => {
                onClose();
                void onOpenLegacyFlow("textures", "import_texture");
              }}
            >
              Import texture…
            </button>
          )}

          {menu.targetType === "texture" && menu.texture && (
            <>
              <button
                type="button"
                className="wmb-ctx-item"
                onClick={() => {
                  const tex = menu.texture!;
                  onClose();
                  void onOpenLegacyFlow(
                    `tex:${tex.textureId}`,
                    "export_texture"
                  );
                }}
              >
                Export DDS / PNG…
              </button>
              <button
                type="button"
                className="wmb-ctx-item"
                onClick={() => {
                  const tex = menu.texture!;
                  onClose();
                  void onOpenLegacyFlow(
                    `tex:${tex.textureId}`,
                    "replace_texture"
                  );
                }}
              >
                Replace…
              </button>
              <div className="wmb-ctx-sep" />
              <button
                type="button"
                className="wmb-ctx-item"
                disabled={menu.texture.usedByMaterialIds.length === 0}
                onClick={() => {
                  const tex = menu.texture!;
                  onClose();
                  onSelect(
                    tex.usedByMaterialIds.map((id) => ({
                      kind: "material",
                      id,
                    })),
                    false
                  );
                }}
              >
                Select materials using this texture (
                {menu.texture.usedByMaterialIds.length})
              </button>
            </>
          )}

          {menu.targetType === "materialsHeader" && (
            <>
              <button
                type="button"
                className="wmb-ctx-item"
                onClick={() => {
                  onClose();
                  void onMutateOp({ op: "add_material" });
                }}
              >
                New material
              </button>
              <button
                type="button"
                className="wmb-ctx-item"
                disabled={!canPasteClipboard(clipboardStatus, manifest.game, "materials").allowed}
                data-tooltip={canPasteClipboard(clipboardStatus, manifest.game, "materials").reason}
                onClick={() => {
                  onClose();
                  void onPasteInto(null);
                }}
              >
                Paste material
              </button>
            </>
          )}

          {menu.targetType === "material" && menu.material && (
            <>
              <button
                type="button"
                className="wmb-ctx-item"
                onClick={() => {
                  const mat = menu.material!;
                  onClose();
                  void onCopyItems([mat.id]);
                }}
              >
                Copy
              </button>
              <button
                type="button"
                className="wmb-ctx-item"
                onClick={() => {
                  const mat = menu.material!;
                  onClose();
                  void onMutateOp({ op: "duplicate_item", itemId: mat.id });
                }}
              >
                Duplicate
              </button>
              <button
                type="button"
                className="wmb-ctx-item danger"
                disabled={manifest.materials.length <= 1}
                onClick={() => {
                  const mat = menu.material!;
                  onClose();
                  void triggerDeleteSelected([{ kind: "material", id: mat.id }]);
                }}
              >
                Delete…
              </button>
              <div className="wmb-ctx-sep" />
              <button
                type="button"
                className="wmb-ctx-item"
                disabled={menu.material.usedByBatchIds.length === 0}
                onClick={() => {
                  const mat = menu.material!;
                  onClose();
                  onSelect(
                    mat.usedByBatchIds.map((id) => ({ kind: "batch", id })),
                    false
                  );
                }}
              >
                Select batches using this material (
                {menu.material.usedByBatchIds.length})
              </button>
              <div className="wmb-ctx-sep" />
              <div className="wmb-ctx-header">
                Convert game profile (B1 ↔ B2 ↔ B3)
              </div>
              {(["b1", "b2", "b3"] as const).map((g) => (
                <button
                  key={g}
                  type="button"
                  className="wmb-ctx-item"
                  onClick={() => {
                    const mat = menu.material!;
                    onClose();
                    void onConvertMaterialProfile(mat.index, g);
                  }}
                >
                  Convert to {g.toUpperCase()} profile
                </button>
              ))}
            </>
          )}

          {menu.targetType === "bonesHeader" && (
            <>
              <button
                type="button"
                className="wmb-ctx-item"
                onClick={() => {
                  onClose();
                  void onMutateOp({ op: "add_bone", parentGlobalId: -1 });
                }}
              >
                Add bone
              </button>
              <button
                type="button"
                className="wmb-ctx-item"
                onClick={() => {
                  onClose();
                  void onOpenLegacyFlow("bones", "bone_preset");
                }}
              >
                Apply bone mapping preset…
              </button>
            </>
          )}

          {menu.targetType === "bone" && menu.bone && (
            <>
              <button
                type="button"
                className="wmb-ctx-item"
                onClick={() => {
                  const b = menu.bone!;
                  onClose();
                  void onMutateOp({
                    op: "add_bone",
                    parentGlobalId: b.globalId,
                  });
                }}
              >
                Add child bone
              </button>

              <button
                type="button"
                className="wmb-ctx-item danger"
                disabled={manifest.bones.length <= 1}
                onClick={() => {
                  const b = menu.bone!;
                  const bIdx = manifest.bones.indexOf(b);
                  onClose();
                  void onMutateOp({
                    op: "delete_items",
                    items: [getBoneSceneId(b, bIdx >= 0 ? bIdx : 0)],
                  });
                }}
              >
                Delete bone
              </button>
            </>
          )}

          {menu.targetType === "meshGroupsHeader" && (
            <>
              <button
                type="button"
                className="wmb-ctx-item"
                onClick={() => {
                  onClose();
                  void onMutateOp({
                    op: "add_mesh_group",
                    name: `mesh_${manifest.meshGroups.length}`,
                  });
                }}
              >
                New mesh group
              </button>
              <button
                type="button"
                className="wmb-ctx-item"
                disabled={
                  !canPasteClipboard(clipboardStatus, manifest.game, "batches").allowed ||
                  manifest.meshGroups.length === 0
                }
                data-tooltip={canPasteClipboard(clipboardStatus, manifest.game, "batches").reason}
                onClick={() => {
                  const firstTarget =
                    manifest.meshGroups[0]?.lods[0]?.id ??
                    manifest.meshGroups[0]?.id ??
                    null;
                  onClose();
                  void onPasteInto(firstTarget);
                }}
              >
                Paste batch
              </button>
            </>
          )}

          {menu.targetType === "meshGroup" && menu.meshGroup && (
            <>
              <button
                type="button"
                className="wmb-ctx-item"
                onClick={() => {
                  const mg = menu.meshGroup!;
                  onClose();
                  beginInlineRename(mg.id, mg.name);
                }}
              >
                Rename
              </button>
              <button
                type="button"
                className="wmb-ctx-item"
                onClick={() => {
                  const mg = menu.meshGroup!;
                  onClose();
                  void onMutateOp({ op: "duplicate_item", itemId: mg.id });
                }}
              >
                Duplicate
              </button>
              <button
                type="button"
                className="wmb-ctx-item danger"
                disabled={manifest.meshGroups.length <= 1}
                onClick={() => {
                  const mg = menu.meshGroup!;
                  onClose();
                  void onMutateOp({ op: "delete_items", items: [mg.id] });
                }}
              >
                Delete
              </button>
              <button
                type="button"
                className="wmb-ctx-item"
                disabled={!canPasteClipboard(clipboardStatus, manifest.game, "batches").allowed}
                data-tooltip={canPasteClipboard(clipboardStatus, manifest.game, "batches").reason}
                onClick={() => {
                  const mg = menu.meshGroup!;
                  onClose();
                  void onPasteInto(mg.lods[0]?.id ?? mg.id);
                }}
              >
                Paste batch here
              </button>
              <div className="wmb-ctx-sep" />
              <div className="wmb-ctx-header">
                Assign material to all batches ▸
              </div>
              <div className="wmb-ctx-subscroll">
                {manifest.materials.map((m) => (
                  <button
                    key={m.id}
                    type="button"
                    className="wmb-ctx-item"
                    onClick={() => {
                      const mg = menu.meshGroup!;
                      onClose();
                      void onMutateOp({
                        op: "set_mesh_group_material",
                        meshId: mg.id,
                        materialIndex: m.index,
                      });
                    }}
                  >
                    mat{m.index} — {m.shaderName ?? m.name}
                  </button>
                ))}
              </div>
            </>
          )}

          {menu.targetType === "lod" && menu.lod && (
            <>
              <button
                type="button"
                className="wmb-ctx-item"
                onClick={() => {
                  const l = menu.lod!;
                  onClose();
                  beginInlineRename(l.id, l.name);
                }}
              >
                Rename
              </button>
              <button
                type="button"
                className="wmb-ctx-item"
                disabled={!canPasteClipboard(clipboardStatus, manifest.game, "batches").allowed}
                data-tooltip={canPasteClipboard(clipboardStatus, manifest.game, "batches").reason}
                onClick={() => {
                  const l = menu.lod!;
                  onClose();
                  void onPasteInto(l.id);
                }}
              >
                Paste batch here
              </button>
            </>
          )}

          {menu.targetType === "batch" && menu.batch && (
            <>
              <button
                type="button"
                className="wmb-ctx-item"
                onClick={() => {
                  const b = menu.batch!;
                  const batchSelIds = selection
                    .filter((s) => s.kind === "batch")
                    .map((s) => s.id);
                  onClose();
                  void onCopyItems(
                    batchSelIds.includes(b.id) ? batchSelIds : [b.id]
                  );
                }}
              >
                Copy
              </button>
              <button
                type="button"
                className="wmb-ctx-item"
                onClick={() => {
                  const b = menu.batch!;
                  onClose();
                  void onMutateOp({ op: "duplicate_item", itemId: b.id });
                }}
              >
                Duplicate
              </button>
              <button
                type="button"
                className="wmb-ctx-item danger"
                onClick={() => {
                  const b = menu.batch!;
                  const batchSel = selection.filter((s) => s.kind === "batch");
                  onClose();
                  void triggerDeleteSelected(
                    batchSel.some((x) => x.id === b.id)
                      ? batchSel
                      : [{ kind: "batch", id: b.id }]
                  );
                }}
              >
                Delete
              </button>
              <div className="wmb-ctx-sep" />
              <div className="wmb-ctx-header">Assign material ▸</div>
              <div className="wmb-ctx-subscroll">
                {manifest.materials.map((m) => (
                  <button
                    key={m.id}
                    type="button"
                    className="wmb-ctx-item"
                    onClick={() => {
                      const b = menu.batch!;
                      onClose();
                      void onMutateOp({
                        op: "set_batch_material",
                        batchId: b.id,
                        materialIndex: m.index,
                      });
                    }}
                  >
                    mat{m.index} — {m.shaderName ?? m.name}
                  </button>
                ))}
              </div>
            </>
          )}
        </div>
      )}
  </>
);
