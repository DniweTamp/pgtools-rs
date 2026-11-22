import React, { useEffect, useMemo, useState } from "react";
import { IconCheck } from "../components/Icons";
import { invoke } from "@tauri-apps/api/core";
import {
  BatchDesc,
  BoneDesc,
  LodDesc,
  MaterialDesc,
  MeshGroupDesc,
  resolveLegacyNode,
  TextureRefDesc,
  WmbSceneManifest,
} from "./contract";
import { LegacyActionKind } from "./contract";
import { MaterialEditor } from "./MaterialEditor";
import {
  IconChevronRight,
  IconDelete,
  IconDuplicate,
  IconShadow,
  inferTextureRoles,
  isSyntheticTexName,
  MaterialPickerPopover,
} from "./Pickers";
import {
  buildBoneHierarchy,
  DebugSlotPreview,
  findBatchById,
  findBoneIndexAndDescBySceneId,
  findLodById,
  findMaterialById,
  findMeshGroupById,
  findTextureBySceneId,
  getBoneSceneId,
  getMeshGroupBatchIds,
  SceneSelectionItem,
} from "./store";

export interface PropertiesProps {
  manifest: WmbSceneManifest;
  selection: SceneSelectionItem[];
  onSelect: (items: SceneSelectionItem[], additive?: boolean) => void;
  onLiveMaterialChange: (draftMat: MaterialDesc) => void;
  onMutateOp: (op: Record<string, unknown>) => Promise<void>;
  onDirectWmbMutate: (
    operation: string,
    payload: Record<string, unknown>
  ) => Promise<void>;
  onOpenLegacyFlow: (itemId: string, action: LegacyActionKind) => Promise<void>;
  debugSlot?: DebugSlotPreview | null;
  onChangeDebugSlot?: (slot: DebugSlotPreview | null) => void;
}

export const Properties: React.FC<PropertiesProps> = React.memo(function Properties({
  manifest,
  selection,
  onSelect,
  onLiveMaterialChange,
  onMutateOp,
  onDirectWmbMutate,
  onOpenLegacyFlow,
  debugSlot,
  onChangeDebugSlot,
}: PropertiesProps) {
  const primary = selection[0] ?? { kind: "root", id: "root" };
  const isMulti = selection.length > 1;

  const [batchHeaderOpen, setBatchHeaderOpen] = useState(true);
  const boneHierarchy = useMemo(() => buildBoneHierarchy(manifest.bones), [manifest.bones]);

  // Root header extra fields fetched via resolve_legacy_node("root")
  const [headerMeta, setHeaderMeta] = useState<{
    endian?: string;
    vertexFlagsHex?: string;
  }>({});
  const [bboxDraft, setBboxDraft] = useState<number[]>(manifest.boundingBox);
  const [scaleDraft, setScaleDraft] = useState<string>("1.0");

  const textureRoles = useMemo(() => inferTextureRoles(manifest), [manifest]);

  useEffect(() => {
    setBboxDraft(
      manifest.boundingBox.length === 6
        ? [...manifest.boundingBox]
        : [0, 0, 0, 0, 0, 0]
    );
  }, [manifest.boundingBox]);

  useEffect(() => {
    if (primary.kind !== "root" || isMulti) return;
    let cancelled = false;
    (async () => {
      try {
        const legacyId = await resolveLegacyNode(manifest.nodeId, "root");
        const res = await invoke<{
          wmb?: {
            endian: string;
            data?: {
              vertex_flags?: number;
              flags?: number;
            };
          };
        }>("inspect_asset_node", {
          nodeId: legacyId,
          mipIndex: 0,
          channel: "RGBA",
          opaque: false,
        });
        if (cancelled) return;
        if (res.wmb) {
          const rawFlags =
            res.wmb.data?.vertex_flags ?? res.wmb.data?.flags ?? 0;
          const hexWidth = manifest.isWmb3 ? 4 : 8;
          const masked = manifest.isWmb3 ? (rawFlags & 0xffff) >>> 0 : rawFlags >>> 0;
          setHeaderMeta({
            endian: res.wmb.endian,
            vertexFlagsHex: `0x${masked
              .toString(16)
              .toUpperCase()
              .padStart(hexWidth, "0")}`,
          });
        }
      } catch {
        // Ignore if unavailable
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [isMulti, manifest.nodeId, primary.kind]);


  const totalVertices = useMemo(
    () => manifest.vertexBuffers.reduce((acc, vb) => acc + vb.vertexCount, 0),
    [manifest.vertexBuffers]
  );

  const renderMultiSelect = () => {
    const selectedBatches = selection
      .filter((s) => s.kind === "batch")
      .map((s) => findBatchById(manifest, s.id))
      .filter((b): b is BatchDesc => Boolean(b));

    return (
      <div className="wmb-prop-body">
        <div className="insp-card">
          <div className="insp-card-header">
            <div className="insp-card-header-left">
              <span className="tree-badge">{selection.length}</span>
              <span className="insp-card-title">
                Multi-Selection ({selection.length} items)
              </span>
            </div>
          </div>
          <div className="insp-card-body">
            <div className="wmb-prop-kv">
              <span>Selected Batches:</span>
              <strong>{selectedBatches.length}</strong>
            </div>
            {selectedBatches.length > 0 && (
              <div className="wmb-field-row" style={{ marginTop: "6px" }}>
                <span>Assign Material:</span>
                <MaterialPickerPopover
                  allowReselect
                  materials={manifest.materials}
                  textures={manifest.textures}
                  currentMatIdx={selectedBatches[0].materialIndex}
                  onChange={(nextMatIdx) => {
                    void onMutateOp({
                      op: "set_batches_material",
                      batchIds: selectedBatches.map((b) => b.id),
                      materialIndex: nextMatIdx,
                    });
                  }}
                />
              </div>
            )}
          </div>
        </div>
      </div>
    );
  };

  const renderRootEditor = () => (
    <div className="wmb-prop-body">
      <div className="insp-card">
        <div className="insp-card-header">
          <div className="insp-card-header-left">
            <span className="tree-badge">{manifest.game.toUpperCase()}</span>
            <span className="insp-card-title">Model Header</span>
          </div>
        </div>
        <div className="insp-card-body">
          <div className="wmb-prop-kv">
            <span>File Name:</span>
            <strong className="mono">{manifest.fileName}</strong>
          </div>
          <div className="wmb-prop-kv">
            <span>Format / Game:</span>
            <strong>
              {manifest.isWmb3 ? "WMB3" : "WMB"} ({manifest.game.toUpperCase()})
            </strong>
          </div>
          {headerMeta.endian && (
            <div className="wmb-prop-kv">
              <span>Endianness:</span>
              <strong>{headerMeta.endian}</strong>
            </div>
          )}
          <div className="wmb-field-row">
            <span>{manifest.isWmb3 ? "Header Flags (16-bit):" : "Vertex Flags (Layout):"}</span>
            <input
              type="text"
              className="wmb-prop-input mono"
              key={headerMeta.vertexFlagsHex ?? "vf"}
              defaultValue={
                headerMeta.vertexFlagsHex ?? (manifest.isWmb3 ? "0x0000" : "0x00000000")
              }
              readOnly={!manifest.isWmb3}
              data-tooltip={
                manifest.isWmb3
                  ? "WMB3 16-bit model header flags (0x0000..0xFFFF)"
                  : "WMB0-2 vertex_flags defines the binary vertex stride and attribute layout (read-only to prevent vertex stream corruption)"
              }
              onBlur={(e) => {
                if (!manifest.isWmb3) return;
                const v = e.target.value.trim();
                if (v && v !== headerMeta.vertexFlagsHex) {
                  void onDirectWmbMutate("update_header", {
                    flags_hex: v,
                  });
                }
              }}
              onKeyDown={(e) => {
                if (e.key === "Enter") (e.target as HTMLInputElement).blur();
              }}
            />
          </div>
        </div>
      </div>

      <div className="insp-card">
        <div className="insp-card-header">
          <div className="insp-card-header-left">
            <span className="insp-card-title">
              Bounding Box (Min / Max){!manifest.isWmb3 ? " · Computed" : ""}
            </span>
          </div>
        </div>
        <div className="insp-card-body">
          <div className="wmb-vec-grid">
            {["Min X", "Min Y", "Min Z", "Max X", "Max Y", "Max Z"].map(
              (lbl, idx) => (
                <label key={lbl} className="wmb-vec-cell">
                  <span>{lbl}</span>
                  <input
                    type="number"
                    step="0.05"
                    readOnly={!manifest.isWmb3}
                    data-tooltip={
                      manifest.isWmb3
                        ? `Edit WMB3 header bounding box ${lbl}`
                        : "WMB0-2 model bounding box is computed from vertex positions (read-only)"
                    }
                    value={Number((bboxDraft[idx] ?? 0).toFixed(4))}
                    onChange={(e) => {
                      if (!manifest.isWmb3) return;
                      const num = parseFloat(e.target.value);
                      if (Number.isFinite(num)) {
                        setBboxDraft((prev) => {
                          const next = [...prev];
                          next[idx] = num;
                          return next;
                        });
                      }
                    }}
                    onBlur={() => {
                      if (!manifest.isWmb3) return;
                      const changed = bboxDraft.some(
                        (v, i) =>
                          Math.abs(v - (manifest.boundingBox[i] ?? 0)) > 1e-4
                      );
                      if (changed) {
                        void onDirectWmbMutate("update_header", {
                          bounding_box: bboxDraft,
                        });
                      }
                    }}
                    onKeyDown={(e) => {
                      if (e.key === "Enter")
                        (e.target as HTMLInputElement).blur();
                    }}
                  />
                </label>
              )
            )}
          </div>
        </div>
      </div>

      <div className="insp-card">
        <div className="insp-card-header">
          <div className="insp-card-header-left">
            <span className="insp-card-title">Scene Statistics</span>
          </div>
        </div>
        <div className="insp-card-body">
          <div className="batch-grid-4">
            <span className="lbl">Meshes</span>
            <strong className="mono">{manifest.meshGroups.length}</strong>
            <span className="lbl">Batches</span>
            <strong className="mono">{manifest.batches.length}</strong>
            <span className="lbl">Materials</span>
            <strong className="mono">{manifest.materials.length}</strong>
            <span className="lbl">Bones</span>
            <strong className="mono">{manifest.bones.length}</strong>
            <span className="lbl">Textures</span>
            <strong className="mono">{manifest.textures.length}</strong>
            <span className="lbl">Vertices</span>
            <strong className="mono">{totalVertices.toLocaleString()}</strong>
          </div>
        </div>
      </div>

      <div className="insp-card">
        <div className="insp-card-header">
          <div className="insp-card-header-left">
            <span className="insp-card-title">Model Operations</span>
          </div>
        </div>
        <div className="insp-card-body">
          {!manifest.isWmb3 ? (
            <>
              <div className="wmb-field-row">
                <span>Scale Model:</span>
                <div className="wmb-inline-group">
                  <input
                    type="number"
                    step="0.1"
                    className="wmb-prop-input mono"
                    value={scaleDraft}
                    onChange={(e) => setScaleDraft(e.target.value)}
                  />
                  <button
                    type="button"
                    className="wmb-vp-btn"
                    onClick={() => {
                      const s = parseFloat(scaleDraft);
                      if (Number.isFinite(s) && s > 0) {
                        void onDirectWmbMutate("scale", { scale: s });
                      }
                    }}
                  >
                    Apply Scale
                  </button>
                </div>
              </div>
              <div className="wmb-btn-grid">
                <button
                  type="button"
                  className="wmb-vp-btn"
                  onClick={() => void onDirectWmbMutate("swap_endian", {})}
                >
                  Swap Endianness
                </button>
                <button
                  type="button"
                  className="wmb-vp-btn"
                  onClick={() => void onDirectWmbMutate("cleanup_vertexes", {})}
                >
                  Cleanup Vertices
                </button>
                <button
                  type="button"
                  className="wmb-vp-btn"
                  onClick={() =>
                    void onDirectWmbMutate("cleanup_materials", {})
                  }
                >
                  Cleanup Materials
                </button>
                <button
                  type="button"
                  className="wmb-vp-btn"
                  onClick={() => void onDirectWmbMutate("check_normals", {})}
                >
                  Normalize Normals
                </button>
                <button
                  type="button"
                  className="wmb-vp-btn"
                  onClick={() =>
                    void onDirectWmbMutate("recompute_relative_positions", {})
                  }
                >
                  Recompute Bone Rel Pos
                </button>
              </div>
            </>
          ) : (
            <div className="wmb-btn-grid">
              <button
                type="button"
                className="wmb-vp-btn"
                onClick={() => void onDirectWmbMutate("set_tpose", {})}
              >
                Bake Skeleton & Mesh to T-Pose
              </button>
              <button
                type="button"
                className="wmb-vp-btn"
                onClick={() =>
                  void onDirectWmbMutate("recompute_relative_positions", {})
                }
              >
                Recompute Bone Rel Pos
              </button>
            </div>
          )}
        </div>
      </div>
    </div>
  );

  const renderTextureEditor = (tex: TextureRefDesc) => {
    const texIdx = manifest.textures.findIndex(
      (t) => (t.textureId >>> 0) === (tex.textureId >>> 0)
    );
    const role =
      textureRoles.get(tex.textureId >>> 0) ||
      (tex.usedByMaterialIds.length > 0 ? "Bound" : "Unused");
    const hasRealBntxName = !isSyntheticTexName(tex.name);

    return (
      <div className="wmb-prop-body">
        <div className="insp-card">
          <div className="insp-card-header">
            <div className="insp-card-header-left">
              <span className="tree-badge">
                {texIdx >= 0 ? `#${texIdx}` : "TEX"}
              </span>
              <span className="insp-card-title mono">{tex.idHex}</span>
              <span className="tex-role-pill">{role}</span>
            </div>
          </div>
          <div className="insp-card-body">
            {tex.thumbnailDataUrl && (
              <div className="wmb-tex-preview-box">
                <img
                  src={tex.thumbnailDataUrl}
                  alt={tex.idHex}
                  className="wmb-tex-preview-img"
                />
              </div>
            )}
            <div className="wmb-prop-kv">
              <span>Bank Index / Hex ID:</span>
              <strong className="mono">
                {texIdx >= 0 ? `#${texIdx} · ` : ""}
                {tex.idHex} ({tex.textureId >>> 0})
              </strong>
            </div>
            <div className="wmb-prop-kv">
              <span>Inferred Sampler Role:</span>
              <strong>{role}</strong>
            </div>
            {hasRealBntxName ? (
              <div className="wmb-prop-kv">
                <span>Embedded BNTX Name:</span>
                <strong className="mono">{tex.name}</strong>
              </div>
            ) : (
              <div className="wmb-prop-kv">
                <span>Container Name:</span>
                <span className="wmb-tree-meta">
                  WTB/WTA (ID-indexed; no string table)
                </span>
              </div>
            )}
            <div className="wmb-prop-kv">
              <span>Dimensions / Format:</span>
              <strong className="mono">
                {tex.width} × {tex.height} · {tex.formatName}
              </strong>
            </div>
            <div className="wmb-btn-grid" style={{ marginTop: "8px" }}>
              <button
                type="button"
                className="wmb-vp-btn"
                onClick={() =>
                  void onOpenLegacyFlow(
                    `tex:${tex.textureId}`,
                    "replace_texture"
                  )
                }
              >
                Replace Texture…
              </button>
              <button
                type="button"
                className="wmb-vp-btn"
                onClick={() =>
                  void onOpenLegacyFlow(
                    `tex:${tex.textureId}`,
                    "export_texture"
                  )
                }
              >
                Export DDS / PNG…
              </button>
            </div>
          </div>
        </div>

        <div className="insp-card">
          <div className="insp-card-header">
            <div className="insp-card-header-left">
              <span className="insp-card-title">
                Used by Materials ({tex.usedByMaterialIds.length})
              </span>
            </div>
          </div>
          <div className="insp-card-body">
            {tex.usedByMaterialIds.length === 0 ? (
              <div className="wmb-prop-empty">
                Not referenced by any material.
              </div>
            ) : (
              <div className="wmb-chip-list">
                {tex.usedByMaterialIds.map((mid) => {
                  const m = findMaterialById(manifest, mid);
                  return (
                    <button
                      key={mid}
                      type="button"
                      className="wmb-link-chip"
                      onClick={() =>
                        onSelect([{ kind: "material", id: mid }], false)
                      }
                    >
                      {m
                        ? `Material #${m.index} (${m.shaderName ?? m.name})`
                        : mid}
                    </button>
                  );
                })}
              </div>
            )}
          </div>
        </div>
      </div>
    );
  };

  // 3. Bone Editor (displays #index, gid, local ID — never fabricated names; reparenting via Outliner drag-and-drop)
  const renderBoneEditor = (bone: BoneDesc, boneIdx: number) => {
    const localIdx = bone.localId ?? boneIdx;
    const resolvedParentIdx = boneHierarchy.parentIdxByBoneIdx.get(boneIdx) ?? -1;
    const parentBone =
      resolvedParentIdx >= 0 && resolvedParentIdx < manifest.bones.length
        ? manifest.bones[resolvedParentIdx]
        : null;

    return (
      <div className="wmb-prop-body">
        <div className="insp-card">
          <div className="insp-card-header">
            <div className="insp-card-header-left">
              <span className="tree-badge">#{boneIdx}</span>
              <span className="insp-card-title mono">
                gid {bone.globalId} · local {localIdx}
              </span>
            </div>
          </div>
          <div className="insp-card-body">
            <div className="batch-grid-4">
              <span className="lbl">Table Index</span>
              <strong className="mono">#{boneIdx}</strong>
              <span className="lbl">Global ID</span>
              <strong className="mono">{bone.globalId}</strong>
              <span className="lbl">Local ID</span>
              <strong className="mono">{localIdx}</strong>
              <span className="lbl">Parent</span>
              {parentBone ? (
                <button
                  type="button"
                  className="wmb-link-chip mono"
                  data-tooltip="Jump to parent bone (drag bone in Scene Outliner to reparent)"
                  onClick={() =>
                    onSelect(
                      [
                        {
                          kind: "bone",
                          id: getBoneSceneId(parentBone, resolvedParentIdx),
                        },
                      ],
                      false
                    )
                  }
                >
                  #{bone.parent} (gid {parentBone.globalId})
                </button>
              ) : (
                <span className="mono wmb-tree-meta">None (-1 Root)</span>
              )}
            </div>
            <div className="bone-reparent-hint">
              Tip: Drag any bone onto another bone in the Scene Outliner to
              reparent it.
            </div>
          </div>
        </div>

        <div className="insp-card">
          <div className="insp-card-header">
            <div className="insp-card-header-left">
              <span className="insp-card-title">Local Position (Relative)</span>
            </div>
          </div>
          <div className="insp-card-body">
            <div className="wmb-vec-grid">
              {["X", "Y", "Z"].map((axis, compIdx) => (
                <label key={axis} className="wmb-vec-cell">
                  <span>{axis}</span>
                  <input
                    type="number"
                    step="0.01"
                    key={`lpos:${boneIdx}:${bone.globalId}:${compIdx}:${bone.localPosition[compIdx]}`}
                    defaultValue={Number(
                      (bone.localPosition[compIdx] ?? 0).toFixed(4)
                    )}
                    onBlur={(e) => {
                      const num = parseFloat(e.target.value);
                      const cur = Number(
                        (bone.localPosition[compIdx] ?? 0).toFixed(4)
                      );
                      if (
                        Number.isFinite(num) &&
                        boneIdx >= 0 &&
                        Math.abs(num - cur) > 1e-4
                      ) {
                        const next = [...bone.localPosition];
                        next[compIdx] = num;
                        void onDirectWmbMutate("update_bone", {
                          index: boneIdx,
                          relative_position: next,
                        });
                      }
                    }}
                    onKeyDown={(e) => {
                      if (e.key === "Enter")
                        (e.target as HTMLInputElement).blur();
                    }}
                  />
                </label>
              ))}
            </div>
          </div>
        </div>

        <div className="insp-card">
          <div className="insp-card-header">
            <div className="insp-card-header-left">
              <span className="insp-card-title">World Position (Bind)</span>
            </div>
          </div>
          <div className="insp-card-body">
            <div className="wmb-vec-grid">
              {["X", "Y", "Z"].map((axis, compIdx) => (
                <label key={axis} className="wmb-vec-cell">
                  <span>{axis}</span>
                  <input
                    type="number"
                    step="0.01"
                    key={`wpos:${boneIdx}:${bone.globalId}:${compIdx}:${bone.worldPosition[compIdx]}`}
                    defaultValue={Number(
                      (bone.worldPosition[compIdx] ?? 0).toFixed(4)
                    )}
                    onBlur={(e) => {
                      const num = parseFloat(e.target.value);
                      const cur = Number(
                        (bone.worldPosition[compIdx] ?? 0).toFixed(4)
                      );
                      if (
                        Number.isFinite(num) &&
                        boneIdx >= 0 &&
                        Math.abs(num - cur) > 1e-4
                      ) {
                        const next = [...bone.worldPosition];
                        next[compIdx] = num;
                        void onDirectWmbMutate("update_bone", {
                          index: boneIdx,
                          position: next,
                        });
                      }
                    }}
                    onKeyDown={(e) => {
                      if (e.key === "Enter")
                        (e.target as HTMLInputElement).blur();
                    }}
                  />
                </label>
              ))}
            </div>
          </div>
        </div>

        {manifest.isWmb3 && bone.rotation && (
          <div className="insp-card">
            <div className="insp-card-header">
              <div className="insp-card-header-left">
                <span className="insp-card-title">Rotation (Radians)</span>
              </div>
            </div>
            <div className="insp-card-body">
              <div className="wmb-vec-grid">
                {["X", "Y", "Z"].map((axis, compIdx) => (
                  <label key={axis} className="wmb-vec-cell">
                    <span>{axis}</span>
                    <input
                      type="number"
                      step="0.02"
                      key={`rot:${boneIdx}:${bone.globalId}:${compIdx}:${bone.rotation?.[compIdx]}`}
                      defaultValue={Number(
                        (bone.rotation?.[compIdx] ?? 0).toFixed(4)
                      )}
                      onBlur={(e) => {
                        const num = parseFloat(e.target.value);
                        const cur = Number(
                          (bone.rotation?.[compIdx] ?? 0).toFixed(4)
                        );
                        if (
                          Number.isFinite(num) &&
                          boneIdx >= 0 &&
                          bone.rotation &&
                          Math.abs(num - cur) > 1e-4
                        ) {
                          const next = [...bone.rotation];
                          next[compIdx] = num;
                          void onDirectWmbMutate("update_bone", {
                            index: boneIdx,
                            rotation: next,
                          });
                        }
                      }}
                      onKeyDown={(e) => {
                        if (e.key === "Enter")
                          (e.target as HTMLInputElement).blur();
                      }}
                    />
                  </label>
                ))}
              </div>
            </div>
          </div>
        )}

        {manifest.isWmb3 && bone.scale && (
          <div className="insp-card">
            <div className="insp-card-header">
              <div className="insp-card-header-left">
                <span className="insp-card-title">Scale</span>
              </div>
            </div>
            <div className="insp-card-body">
              <div className="wmb-vec-grid">
                {["X", "Y", "Z"].map((axis, compIdx) => (
                  <label key={axis} className="wmb-vec-cell">
                    <span>{axis}</span>
                    <input
                      type="number"
                      step="0.05"
                      key={`scl:${boneIdx}:${bone.globalId}:${compIdx}:${bone.scale?.[compIdx]}`}
                      defaultValue={Number(
                        (bone.scale?.[compIdx] ?? 1).toFixed(4)
                      )}
                      onBlur={(e) => {
                        const num = parseFloat(e.target.value);
                        const cur = Number(
                          (bone.scale?.[compIdx] ?? 1).toFixed(4)
                        );
                        if (
                          Number.isFinite(num) &&
                          boneIdx >= 0 &&
                          bone.scale &&
                          Math.abs(num - cur) > 1e-4
                        ) {
                          const next = [...bone.scale];
                          next[compIdx] = num;
                          void onDirectWmbMutate("update_bone", {
                            index: boneIdx,
                            scale: next,
                          });
                        }
                      }}
                      onKeyDown={(e) => {
                        if (e.key === "Enter")
                          (e.target as HTMLInputElement).blur();
                      }}
                    />
                  </label>
                ))}
              </div>
            </div>
          </div>
        )}
      </div>
    );
  };

  const renderMeshGroupEditor = (mg: MeshGroupDesc) => {
    const allBatchIds = getMeshGroupBatchIds(mg);
    const groupBatches = allBatchIds
      .map((id) => findBatchById(manifest, id))
      .filter((b): b is BatchDesc => Boolean(b));
    const totalTris = groupBatches.reduce(
      (acc, b) => acc + Math.floor(b.indexCount / 3),
      0
    );
    const primaryMatIdx = groupBatches[0]?.materialIndex ?? 0;

    return (
      <div className="wmb-prop-body">
        <div className="insp-card">
          <div className="insp-card-header">
            <div className="insp-card-header-left">
              <span className="tree-badge">#{mg.index}</span>
              <span className="insp-card-title">
                Mesh Group #{mg.index} "{mg.name}"
              </span>
            </div>
          </div>
          <div className="insp-card-body">
            <div className="wmb-field-row">
              <span>Name:</span>
              <input
                type="text"
                className="wmb-prop-input"
                key={`${mg.id}:${mg.name}`}
                defaultValue={mg.name}
                onBlur={(e) => {
                  const next = e.target.value.trim();
                  if (next && next !== mg.name) {
                    void onMutateOp({
                      op: "rename_item",
                      itemId: mg.id,
                      name: next,
                    });
                  }
                }}
                onKeyDown={(e) => {
                  if (e.key === "Enter") (e.target as HTMLInputElement).blur();
                }}
              />
            </div>

            <div className="batch-grid-4">
              <span className="lbl">Batches</span>
              <strong className="mono">{groupBatches.length}</strong>
              <span className="lbl">Triangles</span>
              <strong className="mono">{totalTris.toLocaleString()}</strong>
            </div>

            <div className="wmb-field-row" style={{ marginTop: "6px" }}>
              <span>Bulk Material:</span>
              <MaterialPickerPopover
                allowReselect
                materials={manifest.materials}
                textures={manifest.textures}
                currentMatIdx={primaryMatIdx}
                title="Assign material to all batches in this mesh group"
                onChange={(nextMatIdx) => {
                  void onMutateOp({
                    op: "set_mesh_group_material",
                    meshId: mg.id,
                    materialIndex: nextMatIdx,
                  });
                }}
              />
            </div>

            {!manifest.isWmb3 && (
              <button
                type="button"
                className="wmb-vp-btn shadow-toggle-btn"
                style={{ marginTop: "8px", width: "100%", justifyContent: "center" }}
                onClick={() =>
                  void onDirectWmbMutate("toggle_wmb_mesh_shadow_proxy", {
                    mesh_index: mg.index,
                  })
                }
              >
                <IconShadow />
                <span>Toggle Shadow Proxy on All Batches</span>
              </button>
            )}
          </div>
        </div>

        {/* Batches inside this Mesh Group */}
        <div className="insp-card">
          <div className="insp-card-header">
            <div className="insp-card-header-left">
              <span className="insp-card-title">
                Batches in "{mg.name}" ({groupBatches.length})
              </span>
            </div>
          </div>
          <div className="insp-card-body">
            <div className="wmb-chip-list">
              {groupBatches.map((b) => (
                <button
                  key={b.id}
                  type="button"
                  className="wmb-link-chip"
                  onClick={() =>
                    onSelect([{ kind: "batch", id: b.id }], false)
                  }
                >
                  Batch #{b.index} (Mat #{b.materialIndex} ·{" "}
                  {Math.floor(b.indexCount / 3)} tris)
                </button>
              ))}
            </div>
          </div>
        </div>
      </div>
    );
  };

  const renderLodEditor = (mg: MeshGroupDesc, lod: LodDesc) => (
    <div className="wmb-prop-body">
      <div className="insp-card">
        <div className="insp-card-header">
          <div className="insp-card-header-left">
            <span className="tree-badge">L{lod.level}</span>
            <span className="insp-card-title">
              {mg.name} / LOD {lod.level}
            </span>
          </div>
        </div>
        <div className="insp-card-body">
          <div className="wmb-field-row">
            <span>LOD Name:</span>
            <input
              type="text"
              className="wmb-prop-input"
              key={`${lod.id}:${lod.name}`}
              defaultValue={lod.name}
              onBlur={(e) => {
                const next = e.target.value.trim();
                if (next && next !== lod.name) {
                  void onDirectWmbMutate("update_wmb3_lod", {
                    id: lod.id,
                    name: next,
                  });
                }
              }}
              onKeyDown={(e) => {
                if (e.key === "Enter") (e.target as HTMLInputElement).blur();
              }}
            />
          </div>
          <div className="wmb-field-row">
            <span>LOD Level:</span>
            <input
              type="number"
              className="wmb-prop-input mono"
              key={`${lod.id}:lvl:${lod.level}`}
              defaultValue={lod.level}
              onBlur={(e) => {
                const nextLvl = parseInt(e.target.value, 10);
                if (!Number.isNaN(nextLvl) && nextLvl !== lod.level) {
                  void onDirectWmbMutate("update_wmb3_lod", {
                    id: lod.id,
                    level: nextLvl,
                  });
                }
              }}
              onKeyDown={(e) => {
                if (e.key === "Enter") (e.target as HTMLInputElement).blur();
              }}
            />
          </div>
          <div className="wmb-prop-kv">
            <span>Batches in LOD:</span>
            <strong className="mono">{lod.batchIds.length}</strong>
          </div>
        </div>
      </div>
    </div>
  );

  const renderBatchEditor = (batch: BatchDesc) => {
    const mg = findMeshGroupById(manifest, batch.meshId);
    const triCount = Math.floor(batch.indexCount / 3);
    const flagsHex = `0x${((batch.flags ?? 0x8001) >>> 0)
      .toString(16)
      .toUpperCase()
      .padStart(4, "0")}`;
    const uE1 = batch.uE1 ?? 0;
    const uE2 = batch.uE2 ?? 0;
    const primitiveType = batch.primitiveType ?? 4;
    const boneRefs = batch.boneRefs ?? [];
    const boneRefsStr = boneRefs.join(", ");
    const boneSetIndex = batch.boneSetIndex ?? -1;
    const callTreeNodeIndex = batch.callTreeNodeIndex ?? 0;
    const vtxSpanText =
      batch.vertexStart !== undefined && batch.vertexEnd !== undefined
        ? `vtx ${batch.vertexStart}..${batch.vertexEnd} · ${batch.indexCount} idx · ${triCount.toLocaleString()} Tris`
        : `${batch.vertexCount.toLocaleString()} vtx · ${batch.indexCount} idx · ${triCount.toLocaleString()} Tris`;

    return (
      <div className="wmb-prop-body">
        <div className="batch-card">
          {/* Batch Card Top Bar with animated `>` button */}
          <div
            className="batch-card-top"
            onClick={() => setBatchHeaderOpen((v) => !v)}
          >
            <div className="batch-card-title-group">
              <IconChevronRight expanded={batchHeaderOpen} />
              <span className="tree-badge">#{batch.index}</span>
              <span className="batch-card-title">Batch #{batch.index}</span>
              {mg && (
                <button
                  type="button"
                  className="tree-badge parent-mesh-badge"
                  data-tooltip={`Parent Mesh Group #${mg.index} "${mg.name}" — Click to inspect mesh group, or drag batch in Scene Outliner to move`}
                  onClick={(e) => {
                    e.stopPropagation();
                    onSelect([{ kind: "meshGroup", id: mg.id }], false);
                  }}
                >
                  MESH #{mg.index} · {mg.name}
                </button>
              )}
              <span className="wmb-tree-meta mono">{vtxSpanText}</span>
            </div>

            <div
              className="batch-card-actions"
              onClick={(e) => e.stopPropagation()}
            >
              <button
                type="button"
                className="icon-action-btn"
                data-tooltip="Duplicate Batch"
                onClick={() =>
                  void onMutateOp({ op: "duplicate_item", itemId: batch.id })
                }
              >
                <IconDuplicate />
              </button>
              <button
                type="button"
                className="icon-action-btn danger"
                data-tooltip="Delete Batch"
                onClick={() =>
                  void onMutateOp({ op: "delete_items", items: [batch.id] })
                }
              >
                <IconDelete />
              </button>
            </div>
          </div>

          <div className="batch-card-body">
            {/* Material Picker Row (standalone batch MaterialPickerPopover + ↗ Edit Mat) */}
            <div className="wmb-field-row">
              <span>Material:</span>
              <MaterialPickerPopover
                materials={manifest.materials}
                textures={manifest.textures}
                currentMatIdx={batch.materialIndex}
                onChange={(nextMatIdx) => {
                  void onMutateOp({
                    op: "set_batch_material",
                    batchId: batch.id,
                    materialIndex: nextMatIdx,
                  });
                }}
                onJumpToMaterial={(matIdx) => {
                  const targetMat = manifest.materials[matIdx];
                  if (targetMat) {
                    onSelect([{ kind: "material", id: targetMat.id }], false);
                  }
                }}
              />
            </div>

            {/* Collapsible Batch Header & Bone Palette (expanded by default, animated via .collapse-grid) */}
            <div
              className={`collapse-grid${batchHeaderOpen ? " expanded" : ""}`}
            >
              <div className="collapse-inner">
                <div className="batch-header-expanded-panel">
                  {!manifest.isWmb3 ? (
                    <>
                      <div className="batch-grid-4">
                        <span className="lbl">Flags (Hex)</span>
                        <input
                          type="text"
                          className="wmb-prop-input mono"
                          key={`bf:${batch.id}:${flagsHex}`}
                          defaultValue={flagsHex}
                          onBlur={(e) => {
                            const v = e.target.value.trim();
                            if (v && v.toUpperCase() !== flagsHex.toUpperCase()) {
                              void onDirectWmbMutate("update_batch", {
                                batch_item_id: batch.id,
                                flags_hex: v,
                              });
                            }
                          }}
                          onKeyDown={(e) => {
                            if (e.key === "Enter")
                              (e.target as HTMLInputElement).blur();
                          }}
                        />

                        <span className="lbl">Shadow Proxy</span>
                        <button
                          type="button"
                          role="checkbox"
                          aria-checked={batch.isShadowProxy}
                          className={`studio-inline-checkbox shadow-checkbox-cell${batch.isShadowProxy ? " checked" : ""}`}
                          data-tooltip="WMB shadow caster flag (B1: u_e1 = 32, u_e2 = 15; B2: u_e1 = 48)"
                          onClick={() => {
                            void onDirectWmbMutate("update_batch", {
                              batch_item_id: batch.id,
                              shadow_proxy: !batch.isShadowProxy,
                            });
                          }}
                        >
                          <span className="pill-checkbox-box">
                            {batch.isShadowProxy && <IconCheck />}
                          </span>
                          <span className="mono">
                            {batch.isShadowProxy
                              ? manifest.game === "b2"
                                ? "Yes (48)"
                                : "Yes (32/15)"
                              : `No (${uE1}/${uE2})`}
                          </span>
                        </button>

                        <span className="lbl">u_e1 (Layer)</span>
                        <input
                          type="number"
                          min={0}
                          max={255}
                          className="wmb-prop-input mono"
                          key={`bue1:${batch.id}:${uE1}`}
                          defaultValue={uE1}
                          onBlur={(e) => {
                            const num = parseInt(e.target.value, 10);
                            if (
                              !Number.isNaN(num) &&
                              num >= 0 &&
                              num <= 255 &&
                              num !== uE1
                            ) {
                              void onDirectWmbMutate("update_batch", {
                                batch_item_id: batch.id,
                                u_e1: num,
                              });
                            }
                          }}
                          onKeyDown={(e) => {
                            if (e.key === "Enter")
                              (e.target as HTMLInputElement).blur();
                          }}
                        />

                        <span className="lbl">u_e2 (Pass)</span>
                        <input
                          type="number"
                          min={0}
                          max={255}
                          className="wmb-prop-input mono"
                          key={`bue2:${batch.id}:${uE2}`}
                          defaultValue={uE2}
                          onBlur={(e) => {
                            const num = parseInt(e.target.value, 10);
                            if (
                              !Number.isNaN(num) &&
                              num >= 0 &&
                              num <= 255 &&
                              num !== uE2
                            ) {
                              void onDirectWmbMutate("update_batch", {
                                batch_item_id: batch.id,
                                u_e2: num,
                              });
                            }
                          }}
                          onKeyDown={(e) => {
                            if (e.key === "Enter")
                              (e.target as HTMLInputElement).blur();
                          }}
                        />

                        <span className="lbl">Prim Type</span>
                        <input
                          type="number"
                          min={4}
                          max={5}
                          className="wmb-prop-input mono"
                          key={`bp:${batch.id}:${primitiveType}`}
                          defaultValue={primitiveType}
                          onBlur={(e) => {
                            const num = parseInt(e.target.value, 10);
                            if (
                              !Number.isNaN(num) &&
                              (num === 4 || num === 5) &&
                              num !== primitiveType
                            ) {
                              void onDirectWmbMutate("update_batch", {
                                batch_item_id: batch.id,
                                primitive_type: num,
                              });
                            }
                          }}
                          onKeyDown={(e) => {
                            if (e.key === "Enter")
                              (e.target as HTMLInputElement).blur();
                          }}
                        />

                        <span className="lbl">Vtx Buffer</span>
                        <input
                          type="text"
                          className="wmb-prop-input mono"
                          value={`#${batch.vertexBufferIndex}`}
                          readOnly
                        />
                      </div>

                      <div className="bone-refs-block">
                        <div className="bone-refs-label">
                          <span>
                            Bone Refs ({boneRefs.length} joints)
                          </span>
                          <span className="wmb-tree-meta">CSV local IDs</span>
                        </div>
                        <input
                          type="text"
                          className="wmb-prop-input mono"
                          key={`bbr:${batch.id}:${boneRefsStr}`}
                          defaultValue={boneRefsStr}
                          onBlur={(e) => {
                            const maxBoneIdx = Math.max(0, manifest.bones.length - 1);
                            const nums = e.target.value
                              .split(",")
                              .map((s) => s.trim())
                              .filter((s) => s.length > 0)
                              .map((s) => parseInt(s, 10))
                              .filter(
                                (n) =>
                                  !Number.isNaN(n) &&
                                  n >= 0 &&
                                  n <= 255 &&
                                  (manifest.bones.length === 0 || n <= maxBoneIdx)
                              );
                            if (nums.join(", ") !== boneRefsStr) {
                              void onDirectWmbMutate("update_batch", {
                                batch_item_id: batch.id,
                                bone_refs: nums,
                              });
                            }
                          }}
                          onKeyDown={(e) => {
                            if (e.key === "Enter")
                              (e.target as HTMLInputElement).blur();
                          }}
                        />
                      </div>
                    </>
                  ) : (
                    <div className="batch-grid-4">
                      <span className="lbl">Bone Set Idx</span>
                      <input
                        type="number"
                        className="wmb-prop-input mono"
                        key={`bbsi:${batch.id}:${boneSetIndex}`}
                        defaultValue={boneSetIndex}
                        onBlur={(e) => {
                          const num = parseInt(e.target.value, 10);
                          if (!Number.isNaN(num) && num !== boneSetIndex) {
                            void onDirectWmbMutate("update_wmb3_batch", {
                              batch_id: batch.id,
                              batch_index: batch.index,
                              bone_set_index: num,
                            });
                          }
                        }}
                        onKeyDown={(e) => {
                          if (e.key === "Enter")
                            (e.target as HTMLInputElement).blur();
                        }}
                      />

                      <span className="lbl">Call Tree Node</span>
                      <input
                        type="number"
                        className="wmb-prop-input mono"
                        key={`bctn:${batch.id}:${callTreeNodeIndex}`}
                        defaultValue={callTreeNodeIndex}
                        onBlur={(e) => {
                          const num = parseInt(e.target.value, 10);
                          if (!Number.isNaN(num) && num !== callTreeNodeIndex) {
                            void onDirectWmbMutate("update_wmb3_batch", {
                              batch_id: batch.id,
                              batch_index: batch.index,
                              call_tree_node_index: num,
                            });
                          }
                        }}
                        onKeyDown={(e) => {
                          if (e.key === "Enter")
                            (e.target as HTMLInputElement).blur();
                        }}
                      />

                      <span className="lbl">Vtx Buffer</span>
                      <input
                        type="text"
                        className="wmb-prop-input mono"
                        value={`#${batch.vertexBufferIndex}`}
                        readOnly
                      />

                      <span className="lbl">Triangles</span>
                      <input
                        type="text"
                        className="wmb-prop-input mono"
                        value={triCount.toLocaleString()}
                        readOnly
                      />
                    </div>
                  )}
                </div>
              </div>
            </div>
          </div>
        </div>
      </div>
    );
  };

  return (
    <div className="wmb-properties-shell">
      <div className="wmb-panel-header">
        <span className="wmb-panel-title">Inspector</span>
        <span className="wmb-panel-subtitle mono">
          {isMulti
            ? `${selection.length} selected`
            : `${primary.kind}: ${primary.id}`}
        </span>
      </div>

      <div className="wmb-properties-scroll">
        {isMulti ? (
          renderMultiSelect()
        ) : primary.kind === "root" ? (
          renderRootEditor()
        ) : primary.kind === "texture" ? (
          (() => {
            const tex = findTextureBySceneId(manifest, primary.id);
            return tex ? (
              renderTextureEditor(tex)
            ) : (
              <div className="wmb-prop-empty">Texture not found.</div>
            );
          })()
        ) : primary.kind === "material" ? (
          (() => {
            const mat = findMaterialById(manifest, primary.id);
            return mat ? (
              <MaterialEditor
                manifest={manifest}
                material={mat}
                onSelect={onSelect}
                onLiveMaterialChange={onLiveMaterialChange}
                onCommitMaterialTexture={async (matIdx, slotIdx, texId) => {
                  await onDirectWmbMutate("set_material_texture", {
                    index: matIdx,
                    slot: slotIdx,
                    texture_id: texId,
                  });
                }}
                onCommitMaterialParam={async (matIdx, key, values) => {
                  await onDirectWmbMutate("set_material_param", {
                    index: matIdx,
                    key,
                    values,
                  });
                }}
                onSwitchMaterialShader={async (matIdx, shaderKey, b2Override) => {
                  await onDirectWmbMutate("switch_shader", {
                    index: matIdx,
                    shader: shaderKey,
                    is_bayo2:
                      b2Override !== undefined
                        ? b2Override
                        : manifest.game === "b2",
                  });
                }}
                onDirectWmbMutate={onDirectWmbMutate}
                onMutateOp={onMutateOp}
                debugSlot={debugSlot}
                onChangeDebugSlot={onChangeDebugSlot}
              />
            ) : (
              <div className="wmb-prop-empty">Material not found.</div>
            );
          })()
        ) : primary.kind === "bone" ? (
          (() => {
            const foundBone = findBoneIndexAndDescBySceneId(manifest, primary.id);
            return foundBone ? (
              renderBoneEditor(foundBone.bone, foundBone.index)
            ) : (
              <div className="wmb-prop-empty">Bone not found.</div>
            );
          })()
        ) : primary.kind === "meshGroup" ? (
          (() => {
            const mg = findMeshGroupById(manifest, primary.id);
            return mg ? (
              renderMeshGroupEditor(mg)
            ) : (
              <div className="wmb-prop-empty">Mesh group not found.</div>
            );
          })()
        ) : primary.kind === "lod" ? (
          (() => {
            const found = findLodById(manifest, primary.id);
            return found ? (
              renderLodEditor(found.meshGroup, found.lod)
            ) : (
              <div className="wmb-prop-empty">LOD not found.</div>
            );
          })()
        ) : primary.kind === "batch" ? (
          (() => {
            const batch = findBatchById(manifest, primary.id);
            return batch ? (
              renderBatchEditor(batch)
            ) : (
              <div className="wmb-prop-empty">Batch not found.</div>
            );
          })()
        ) : null}
      </div>
    </div>
  );
});
