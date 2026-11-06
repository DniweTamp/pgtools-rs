import React, { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  MaterialDesc,
  MaterialParamDesc,
  resolveLegacyNode,
  TextureSlotDesc,
  WmbSceneManifest,
} from "./contract";
import {
  inferTextureRoles,
  TextureSlotPickerPopover,
} from "./Pickers";
import { IconChevronRight } from "../components/Icons";
import { dwordToFloat, hexToRgbFloats, rgbFloatsToHex } from "../utils/color";
import { formatHexU8 } from "../utils/format";
import {
  CorePretextureInfo,
  DecodedWmbMaterial,
  ShaderOptionDesc,
  Wmb3Material,
} from "../types";
import { DebugSlotPreview, SceneSelectionItem } from "./store";

export interface MaterialEditorProps {
  manifest: WmbSceneManifest;
  material: MaterialDesc;
  onSelect: (items: SceneSelectionItem[], additive?: boolean) => void;
  onLiveMaterialChange: (draftMat: MaterialDesc) => void;
  onCommitMaterialTexture: (
    materialIndex: number,
    slotIndex: number,
    textureId: number
  ) => Promise<void>;
  onCommitMaterialParam: (
    materialIndex: number,
    key: string,
    values: number[]
  ) => Promise<void>;
  onSwitchMaterialShader: (
    materialIndex: number,
    shaderKey: string,
    b2Override?: boolean
  ) => Promise<void>;
  onDirectWmbMutate?: (
    operation: string,
    payload: Record<string, unknown>
  ) => Promise<void>;
  onMutateOp?: (op: Record<string, unknown>) => Promise<void>;
  debugSlot?: DebugSlotPreview | null;
  onChangeDebugSlot?: (slot: DebugSlotPreview | null) => void;
}

export const MaterialEditor: React.FC<MaterialEditorProps> = ({
  manifest,
  material,
  onSelect,
  onLiveMaterialChange,
  onCommitMaterialTexture,
  onCommitMaterialParam,
  onSwitchMaterialShader,
  onDirectWmbMutate,
  onMutateOp,
  debugSlot,
  onChangeDebugSlot,
}) => {
  const [localParams, setLocalParams] = useState<MaterialParamDesc[]>(
    material.params
  );
  const localParamsRef = useRef<MaterialParamDesc[]>(material.params);
  const lastCommittedSigRef = useRef<Record<string, string>>({});
  const [availableShaders, setAvailableShaders] = useState<ShaderOptionDesc[]>(
    []
  );
  const [corePretextures, setCorePretextures] = useState<CorePretextureInfo[]>([]);
  const colorCommitTimersRef = useRef<Record<string, ReturnType<typeof setTimeout>>>({});
  const [decodedMat, setDecodedMat] = useState<DecodedWmbMaterial | null>(null);
  const [wmb3Mat, setWmb3Mat] = useState<Wmb3Material | null>(null);
  const [b2LayoutOverride, setB2LayoutOverride] = useState<boolean>(
    manifest.game === "b2"
  );
  const [headerOpen, setHeaderOpen] = useState(true);
  const [samplersOpen, setSamplersOpen] = useState(true);
  const [paramsOpen, setParamsOpen] = useState(true);
  const [batchesOpen, setBatchesOpen] = useState(true);

  const [paramViewMode, setParamViewMode] = useState<"typed" | "raw">("typed");

  useEffect(() => {
    setB2LayoutOverride(manifest.game === "b2");
  }, [manifest.game, manifest.nodeId]);

  useEffect(() => {
    localParamsRef.current = material.params;
    setLocalParams(material.params);
    const sigs: Record<string, string> = {};
    for (const p of material.params) {
      sigs[p.key] = JSON.stringify(p.values.map((v) => Number((v ?? 0).toFixed(5))));
    }
    lastCommittedSigRef.current = sigs;
  }, [material]);

  useEffect(() => {
    return () => {
      Object.values(colorCommitTimersRef.current).forEach((t) => clearTimeout(t));
      colorCommitTimersRef.current = {};
    };
  }, []);

  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const legacyId = await resolveLegacyNode(manifest.nodeId, material.id);
        const res = await invoke<{
          wmb?: {
            available_shaders?: ShaderOptionDesc[];
            core_pretextures?: CorePretextureInfo[];
            material_decoded?: DecodedWmbMaterial | null;
            wmb3_material?: Wmb3Material | null;
          };
          wmb3?: {
            available_shaders?: ShaderOptionDesc[];
            core_pretextures?: CorePretextureInfo[];
          };
        }>("inspect_asset_node", {
          nodeId: legacyId,
          mipIndex: 0,
          channel: "RGBA",
          opaque: false,
          b2Override: manifest.isWmb3 ? null : b2LayoutOverride,
        });
        if (cancelled) return;
        const details = res.wmb ?? res.wmb3;
        if (details) {
          setAvailableShaders(details.available_shaders ?? []);
          setCorePretextures(details.core_pretextures ?? []);
        }
        if (res.wmb?.material_decoded) {
          setDecodedMat(res.wmb.material_decoded);
        } else {
          setDecodedMat(null);
        }
        if (res.wmb?.wmb3_material) {
          setWmb3Mat(res.wmb.wmb3_material);
        } else {
          setWmb3Mat(null);
        }
      } catch {
        // Fallback if inspect_asset_node is unavailable
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [
    b2LayoutOverride,
    manifest.game,
    manifest.isWmb3,
    manifest.nodeId,
    material.id,
    material.params,
    material.textureSlots,
  ]);

  const textureRoles = useMemo(() => inferTextureRoles(manifest), [manifest]);

  const handleParamLiveChange = useCallback(
    (paramKey: string, nextValues: number[]) => {
      const updated = localParamsRef.current.map((p) =>
        p.key === paramKey ? { ...p, values: nextValues } : p
      );
      localParamsRef.current = updated;
      setLocalParams(updated);
      onLiveMaterialChange({
        ...material,
        params: updated,
      });
    },
    [material, onLiveMaterialChange]
  );

  const handleParamCommit = useCallback(
    async (paramKey: string, values: number[]) => {
      if (colorCommitTimersRef.current[paramKey]) {
        clearTimeout(colorCommitTimersRef.current[paramKey]);
        delete colorCommitTimersRef.current[paramKey];
      }
      const sig = JSON.stringify(values.map((v) => Number((v ?? 0).toFixed(5))));
      if (lastCommittedSigRef.current[paramKey] === sig) {
        return;
      }
      lastCommittedSigRef.current[paramKey] = sig;
      await onCommitMaterialParam(material.index, paramKey, values);
    },
    [material.index, onCommitMaterialParam]
  );

  const colorParams = useMemo(
    () => localParams.filter((p) => p.isColor),
    [localParams]
  );
  const scalarVectorParams = useMemo(
    () => localParams.filter((p) => !p.isColor),
    [localParams]
  );

  const rawDwords = useMemo(() => {
    if (decodedMat?.raw_dwords && decodedMat.raw_dwords.length > 0) {
      return decodedMat.raw_dwords;
    }
    return [];
  }, [decodedMat]);

  const currentShaderSelectValue = useMemo(() => {
    if (!manifest.isWmb3 && !b2LayoutOverride && material.materialType !== null) {
      return String(material.materialType);
    }
    return material.shaderName ?? "";
  }, [
    b2LayoutOverride,
    manifest.isWmb3,
    material.materialType,
    material.shaderName,
  ]);

  const hexBadge = formatHexU8(material.index);
  const shaderBadgeLabel =
    material.shaderName ??
    (material.materialType !== null ? `type ${material.materialType}` : "standard");

  const channels: Array<"RGB" | "R" | "G" | "B" | "A"> = [
    "RGB",
    "R",
    "G",
    "B",
    "A",
  ];

  const renderSlotRow = (slot: TextureSlotDesc) => {
    const u = slot.textureId >>> 0;

    return (
      <div
        key={slot.slot}
        className="sampler-card"
      >
        <div className="sampler-card-top">
          <span className="sampler-slot-name">
            s{slot.slot} · {slot.name}
            {!slot.present && (
              <span
                className="wmb-missing-tex-badge"
                title="Texture not found in paired bank or pre_texture"
              >
                missing
              </span>
            )}
          </span>

          {onChangeDebugSlot && (
            <div
              className="chan-strip"
              title="Solo this sampler channel on the 3D model in the viewport"
            >
              {channels.map((ch) => {
                const isActive =
                  debugSlot?.slotIndex === slot.slot &&
                  debugSlot?.channel === ch;
                return (
                  <button
                    key={ch}
                    type="button"
                    className={`chan-btn ${isActive ? "active" : ""}`}
                    onClick={(e) => {
                      e.stopPropagation();
                      if (isActive) {
                        onChangeDebugSlot(null);
                      } else {
                        onChangeDebugSlot({
                          slotIndex: slot.slot,
                          slotName: slot.name,
                          role: slot.role,
                          channel: ch,
                        });
                      }
                    }}
                  >
                    {ch}
                  </button>
                );
              })}
            </div>
          )}
        </div>

        <div className="sampler-card-body">
          <TextureSlotPickerPopover
            slotIndex={slot.slot}
            slotLabel={slot.name}
            slotRole={slot.role}
            currentTexId={u}
            textures={manifest.textures}
            materials={manifest.materials}
            corePretextures={corePretextures}
            textureRoles={textureRoles}
            onSelectTextureId={(nextTexId) => {
              void onCommitMaterialTexture(
                material.index,
                slot.slot,
                nextTexId
              );
            }}
            onJumpToTexture={(tid) => {
              onSelect([{ kind: "texture", id: `tex:${tid}` }], false);
            }}
          />
        </div>
      </div>
    );
  };

  return (
    <div className="wmb-material-editor">
      <div className="insp-card">
        <div
          className="insp-card-header"
          onClick={() => setHeaderOpen((v) => !v)}
        >
          <div className="insp-card-header-left">
            <IconChevronRight expanded={headerOpen} />
            <span className="tree-badge">{hexBadge}</span>
            <span className="insp-card-title">
              Material #{material.index} ({shaderBadgeLabel})
            </span>
          </div>
          <div
            className="insp-card-header-actions"
            onClick={(e) => e.stopPropagation()}
          >
            {onMutateOp && (
              <button
                type="button"
                className="secondary-btn compact-btn"
                title="Duplicate this material"
                onClick={() =>
                  void onMutateOp({ op: "duplicate_item", itemId: material.id })
                }
              >
                Duplicate
              </button>
            )}
          </div>
        </div>

        <div className={`collapse-grid${headerOpen ? " expanded" : ""}`}>
          <div className="collapse-inner">
            <div className="insp-card-body">
              {!manifest.isWmb3 && (
                <div className="wmb-field-row">
                  <span>Decode Layout:</span>
                  <div className="seg-toggle-group">
                    <button
                      type="button"
                      className={`seg-toggle-btn ${
                        !b2LayoutOverride ? "active" : ""
                      }`}
                      onClick={() => setB2LayoutOverride(false)}
                    >
                      Bayonetta 1
                    </button>
                    <button
                      type="button"
                      className={`seg-toggle-btn ${
                        b2LayoutOverride ? "active" : ""
                      }`}
                      onClick={() => setB2LayoutOverride(true)}
                    >
                      Bayonetta 2
                    </button>
                  </div>
                </div>
              )}

              <div className="wmb-field-row">
                <span>Shader / Type:</span>
                {availableShaders.length > 0 ? (
                  <select
                    className="wmb-prop-select"
                    value={currentShaderSelectValue}
                    onChange={(e) => {
                      void onSwitchMaterialShader(
                        material.index,
                        e.target.value,
                        b2LayoutOverride
                      );
                    }}
                  >
                    {availableShaders.map((sh) => (
                      <option key={sh.key} value={sh.key}>
                        {sh.label}
                      </option>
                    ))}
                    {!availableShaders.some(
                      (sh) => sh.key === currentShaderSelectValue
                    ) && (
                      <option value={currentShaderSelectValue}>
                        {material.shaderName ??
                          `Type ${material.materialType ?? 0}`}
                      </option>
                    )}
                  </select>
                ) : (
                  <input
                    type="text"
                    className="wmb-prop-input"
                    defaultValue={
                      material.shaderName ?? String(material.materialType ?? 0)
                    }
                    onBlur={(e) => {
                      const next = e.target.value.trim();
                      if (next) {
                        void onSwitchMaterialShader(
                          material.index,
                          next,
                          b2LayoutOverride
                        );
                      }
                    }}
                  />
                )}
              </div>

              {manifest.isWmb3 ? (
                <div className="wmb-field-row">
                  <span>Technique:</span>
                  <input
                    type="text"
                    className="wmb-prop-input mono"
                    value={material.technique ?? "Default"}
                    readOnly
                  />
                </div>
              ) : (
                decodedMat && (
                  <div className="batch-grid-4">
                    <span className="lbl">Mat Type</span>
                    <input
                      type="number"
                      className="wmb-prop-input mono"
                      key={`mt:${material.id}:${decodedMat.mat_type}`}
                      defaultValue={decodedMat.mat_type}
                      onBlur={(e) => {
                        const nextType = parseInt(e.target.value, 10);
                        if (
                          !Number.isNaN(nextType) &&
                          nextType !== decodedMat.mat_type &&
                          onDirectWmbMutate
                        ) {
                          void onDirectWmbMutate("update_material", {
                            ...decodedMat,
                            mat_type: nextType,
                          });
                        }
                      }}
                    />
                    <span className="lbl">Flag</span>
                    <input
                      type="text"
                      className="wmb-prop-input mono"
                      key={`mf:${material.id}:${decodedMat.flag}`}
                      defaultValue={`0x${(decodedMat.flag >>> 0)
                        .toString(16)
                        .toUpperCase()
                        .padStart(8, "0")}`}
                      onBlur={(e) => {
                        const raw = e.target.value
                          .trim()
                          .replace(/^0x/i, "");
                        const nextFlag = parseInt(raw, 16);
                        if (
                          !Number.isNaN(nextFlag) &&
                          (nextFlag >>> 0) !== (decodedMat.flag >>> 0) &&
                          onDirectWmbMutate
                        ) {
                          void onDirectWmbMutate("update_material", {
                            ...decodedMat,
                            flag: nextFlag >>> 0,
                          });
                        }
                      }}
                    />
                  </div>
                )
              )}
            </div>
          </div>
        </div>
      </div>

      <div className="insp-card">
        <div
          className="insp-card-header"
          onClick={() => setSamplersOpen((v) => !v)}
        >
          <div className="insp-card-header-left">
            <IconChevronRight expanded={samplersOpen} />
            <span className="insp-card-title">
              Texture Samplers ({material.textureSlots.length})
            </span>
          </div>
          {debugSlot && onChangeDebugSlot && (
            <div
              className="insp-card-header-actions"
              onClick={(e) => e.stopPropagation()}
            >
              <button
                type="button"
                className="secondary-btn compact-btn active-solo-btn"
                onClick={() => onChangeDebugSlot(null)}
                title="Clear channel soloing and return to shaded 3D view"
              >
                Solo: s{debugSlot.slotIndex}.{debugSlot.channel} ✕
              </button>
            </div>
          )}
        </div>

        <div className={`collapse-grid${samplersOpen ? " expanded" : ""}`}>
          <div className="collapse-inner">
            <div className="insp-card-body">
              {material.textureSlots.length === 0 ? (
                <div className="wmb-prop-empty">
                  No texture samplers on this material.
                </div>
              ) : (
                <div className="wmb-mat-slots-list">
                  {material.textureSlots.map(renderSlotRow)}
                </div>
              )}
            </div>
          </div>
        </div>
      </div>

      <div className="insp-card">
        <div
          className="insp-card-header"
          onClick={() => setParamsOpen((v) => !v)}
        >
          <div className="insp-card-header-left">
            <IconChevronRight expanded={paramsOpen} />
            <span className="insp-card-title">
              Parameters ({localParams.length})
            </span>
          </div>
          <div
            className="insp-card-header-actions"
            onClick={(e) => e.stopPropagation()}
          >
            <div className="seg-toggle-group">
              <button
                type="button"
                className={`seg-toggle-btn ${
                  paramViewMode === "typed" ? "active" : ""
                }`}
                onClick={() => setParamViewMode("typed")}
              >
                Typed
              </button>
              <button
                type="button"
                className={`seg-toggle-btn ${
                  paramViewMode === "raw" ? "active" : ""
                }`}
                onClick={() => setParamViewMode("raw")}
              >
                Raw
                {rawDwords.length > 0 ? ` (${rawDwords.length})` : ""}
              </button>
            </div>
          </div>
        </div>

        <div className={`collapse-grid${paramsOpen ? " expanded" : ""}`}>
          <div className="collapse-inner">
            <div className="insp-card-body">
              {paramViewMode === "typed" ? (
                localParams.length === 0 ? (
                  <div className="wmb-prop-empty">No uniform parameters.</div>
                ) : (
                  <div className="param-list-stack">
                          {colorParams.map((param) => {
                      const vals = param.values;
                      const r = vals[0] ?? 1;
                      const g = vals[1] ?? 1;
                      const b = vals[2] ?? 1;
                      const a = vals[3] ?? 1;
                      const hexVal = rgbFloatsToHex(r, g, b);

                      return (
                        <div key={param.key} className="param-row-color">
                          <div className="param-label-stack">
                            <span
                              className="param-label"
                              title={param.name}
                            >
                              {param.name}
                            </span>
                            <span className="param-hex-sub mono">
                              {hexVal.toUpperCase()}
                            </span>
                          </div>

                          <label
                            className="color-swatch-btn"
                            style={{ backgroundColor: hexVal }}
                            title={`Pick color for ${param.name} (${hexVal.toUpperCase()})`}
                          >
                            <input
                              type="color"
                              value={hexVal}
                              onChange={(e) => {
                                const [nr, ng, nb] = hexToRgbFloats(
                                  e.target.value
                                );
                                const next =
                                  vals.length >= 4
                                    ? [nr, ng, nb, a]
                                    : [nr, ng, nb];
                                handleParamLiveChange(param.key, next);
                                if (colorCommitTimersRef.current[param.key]) {
                                  clearTimeout(
                                    colorCommitTimersRef.current[param.key]
                                  );
                                }
                                colorCommitTimersRef.current[param.key] =
                                  setTimeout(() => {
                                    delete colorCommitTimersRef.current[
                                      param.key
                                    ];
                                    const latest =
                                      localParamsRef.current.find(
                                        (p) => p.key === param.key
                                      )?.values ?? next;
                                    void handleParamCommit(param.key, latest);
                                  }, 180);
                              }}
                              onBlur={() => {
                                if (colorCommitTimersRef.current[param.key]) {
                                  clearTimeout(
                                    colorCommitTimersRef.current[param.key]
                                  );
                                  delete colorCommitTimersRef.current[
                                    param.key
                                  ];
                                }
                                const latest =
                                  localParamsRef.current.find(
                                    (p) => p.key === param.key
                                  )?.values ?? vals;
                                void handleParamCommit(param.key, latest);
                              }}
                            />
                          </label>

                          <div className="vec4-compact-grid">
                            {(vals.length >= 4
                              ? vals.slice(0, 4)
                              : [r, g, b, a]
                            ).map((v, compIdx) => (
                              <label key={compIdx} className="vec-compact-cell">
                                <span className="axis">
                                  {["R", "G", "B", "A"][compIdx]}
                                </span>
                                <input
                                  type="number"
                                  step="0.02"
                                  value={Number((v ?? 0).toFixed(3))}
                                  onChange={(e) => {
                                    const num = parseFloat(e.target.value);
                                    if (Number.isFinite(num)) {
                                      const next =
                                        vals.length >= 4
                                          ? [...vals]
                                          : [r, g, b, a];
                                      next[compIdx] = num;
                                      handleParamLiveChange(param.key, next);
                                    }
                                  }}
                                  onBlur={() => {
                                    const latest =
                                      localParamsRef.current.find(
                                        (p) => p.key === param.key
                                      )?.values ?? vals;
                                    void handleParamCommit(param.key, latest);
                                  }}
                                  onKeyDown={(e) => {
                                    if (e.key === "Enter") {
                                      (e.target as HTMLInputElement).blur();
                                    }
                                  }}
                                />
                              </label>
                            ))}
                          </div>
                        </div>
                      );
                    })}

                          {scalarVectorParams.map((param) => {
                      const vals = param.values;
                      const isSingle = vals.length === 1;
                      const v0 = vals[0] ?? 0;
                      const sliderMax =
                        param.name === "Spec_Limit" &&
                        material.materialType === 151
                          ? 100
                          : Math.max(2, Math.ceil(Math.abs(v0) * 2));
                      const sliderMin = v0 < 0 ? -sliderMax : 0;

                      if (isSingle) {
                        return (
                          <div key={param.key} className="param-row-scalar">
                            <span
                              className="param-label"
                              title={param.name}
                            >
                              {param.name}
                            </span>
                            <input
                              type="range"
                              min={sliderMin}
                              max={sliderMax}
                              step={(sliderMax - sliderMin) / 200}
                              value={v0}
                              onChange={(e) => {
                                const num = parseFloat(e.target.value);
                                if (Number.isFinite(num)) {
                                  handleParamLiveChange(param.key, [num]);
                                }
                              }}
                              onPointerUp={() => {
                                const latest =
                                  localParamsRef.current.find(
                                    (p) => p.key === param.key
                                  )?.values ?? vals;
                                void handleParamCommit(param.key, latest);
                              }}
                            />
                            <input
                              type="number"
                              step="0.05"
                              className="wmb-slider-num mono"
                              value={Number(v0.toFixed(4))}
                              onChange={(e) => {
                                const num = parseFloat(e.target.value);
                                if (Number.isFinite(num)) {
                                  handleParamLiveChange(param.key, [num]);
                                }
                              }}
                              onBlur={() => {
                                const latest =
                                  localParamsRef.current.find(
                                    (p) => p.key === param.key
                                  )?.values ?? vals;
                                void handleParamCommit(param.key, latest);
                              }}
                              onKeyDown={(e) => {
                                if (e.key === "Enter") {
                                  (e.target as HTMLInputElement).blur();
                                }
                              }}
                            />
                          </div>
                        );
                      }

                      return (
                        <div key={param.key} className="param-row-vec">
                          <span
                            className="param-label"
                            title={param.name}
                          >
                            {param.name}
                          </span>
                          <div className="vec4-compact-grid">
                            {vals.map((v, compIdx) => (
                              <label key={compIdx} className="vec-compact-cell">
                                <span className="axis">
                                  {["X", "Y", "Z", "W"][compIdx] ?? compIdx}
                                </span>
                                <input
                                  type="number"
                                  step="0.05"
                                  value={Number(v.toFixed(3))}
                                  onChange={(e) => {
                                    const num = parseFloat(e.target.value);
                                    if (Number.isFinite(num)) {
                                      const next = [...vals];
                                      next[compIdx] = num;
                                      handleParamLiveChange(param.key, next);
                                    }
                                  }}
                                  onBlur={() => {
                                    const latest =
                                      localParamsRef.current.find(
                                        (p) => p.key === param.key
                                      )?.values ?? vals;
                                    void handleParamCommit(param.key, latest);
                                  }}
                                  onKeyDown={(e) => {
                                    if (e.key === "Enter") {
                                      (e.target as HTMLInputElement).blur();
                                    }
                                  }}
                                />
                              </label>
                            ))}
                          </div>
                        </div>
                      );
                    })}
                  </div>
                )
              ) : rawDwords.length > 0 && decodedMat ? (
                <div className="raw-dword-grid">
                  {rawDwords.map((dw, dwIdx) => {
                    const hexStr = `0x${(dw >>> 0)
                      .toString(16)
                      .toUpperCase()
                      .padStart(8, "0")}`;
                    const flt = dwordToFloat(dw);
                    return (
                      <div key={dwIdx} className="raw-dword-cell">
                        <div className="raw-dword-meta">
                          <span className="idx">
                            #{dwIdx.toString().padStart(2, "0")}
                          </span>
                          <span className="flt" title="Float32 interpretation">
                            {flt}f
                          </span>
                        </div>
                        <input
                          type="text"
                          className="wmb-prop-input mono raw-dword-input"
                          key={`dw:${material.id}:${dwIdx}:${dw}`}
                          defaultValue={hexStr}
                          onBlur={(e) => {
                            const clean = e.target.value
                              .trim()
                              .replace(/^0x/i, "");
                            const parsed = parseInt(clean, 16);
                            if (
                              !Number.isNaN(parsed) &&
                              (parsed >>> 0) !== (dw >>> 0) &&
                              onDirectWmbMutate
                            ) {
                              const nextDwords = [...rawDwords];
                              nextDwords[dwIdx] = parsed >>> 0;
                              void onDirectWmbMutate("update_material", {
                                ...decodedMat,
                                raw_dwords: nextDwords,
                              });
                            }
                          }}
                          onKeyDown={(e) => {
                            if (e.key === "Enter") {
                              (e.target as HTMLInputElement).blur();
                            }
                          }}
                        />
                      </div>
                    );
                  })}
                </div>
              ) : wmb3Mat && wmb3Mat.parameter_groups.length > 0 ? (
                <div className="param-list-stack">
                  {wmb3Mat.parameter_groups.map(([grpIdx, vals], gOrder) => (
                    <div key={`${grpIdx}_${gOrder}`} className="raw-group-block">
                      <div className="picker-group-label">
                        Parameter Group #{grpIdx} ({vals.length} floats)
                      </div>
                      <div className="raw-dword-grid">
                        {vals.map((fv, fIdx) => (
                          <div key={fIdx} className="raw-dword-cell">
                            <div className="raw-dword-meta">
                              <span className="idx">[{fIdx}]</span>
                            </div>
                            <input
                              type="number"
                              step="0.05"
                              className="wmb-prop-input mono raw-dword-input"
                              key={`w3p:${material.id}:${gOrder}:${fIdx}:${fv}`}
                              defaultValue={Number(fv.toFixed(4))}
                              onBlur={(e) => {
                                const num = parseFloat(e.target.value);
                                if (
                                  Number.isFinite(num) &&
                                  num !== fv &&
                                  onDirectWmbMutate
                                ) {
                                  const nextGroups =
                                    wmb3Mat.parameter_groups.map(
                                      ([gi, arr], idx) =>
                                        idx === gOrder
                                          ? ([
                                              gi,
                                              arr.map((v, i) =>
                                                i === fIdx ? num : v
                                              ),
                                            ] as [number, number[]])
                                          : ([gi, [...arr]] as [
                                              number,
                                              number[],
                                            ])
                                    );
                                  void onDirectWmbMutate(
                                    "update_wmb3_material",
                                    {
                                      index: material.index,
                                      material: {
                                        ...wmb3Mat,
                                        parameter_groups: nextGroups,
                                      },
                                    }
                                  );
                                }
                              }}
                              onKeyDown={(e) => {
                                if (e.key === "Enter") {
                                  (e.target as HTMLInputElement).blur();
                                }
                              }}
                            />
                          </div>
                        ))}
                      </div>
                    </div>
                  ))}
                </div>
              ) : (
                <div className="wmb-prop-empty">
                  No raw parameter words available.
                </div>
              )}
            </div>
          </div>
        </div>
      </div>

      <div className="insp-card">
        <div
          className="insp-card-header"
          onClick={() => setBatchesOpen((v) => !v)}
        >
          <div className="insp-card-header-left">
            <IconChevronRight expanded={batchesOpen} />
            <span className="insp-card-title">
              Used by Batches ({material.usedByBatchIds.length})
            </span>
          </div>
        </div>

        <div className={`collapse-grid${batchesOpen ? " expanded" : ""}`}>
          <div className="collapse-inner">
            <div className="insp-card-body">
              {material.usedByBatchIds.length === 0 ? (
                <div className="wmb-prop-empty">
                  Not currently assigned to any batches.
                </div>
              ) : (
                <div className="wmb-chip-list">
                  {material.usedByBatchIds.map((bid) => {
                    const b = manifest.batches.find((x) => x.id === bid);
                    return (
                      <button
                        key={bid}
                        type="button"
                        className="wmb-link-chip"
                        onClick={() =>
                          onSelect([{ kind: "batch", id: bid }], false)
                        }
                      >
                        Batch #{b?.index ?? bid}
                      </button>
                    );
                  })}
                </div>
              )}
            </div>
          </div>
        </div>
      </div>
    </div>
  );
};
