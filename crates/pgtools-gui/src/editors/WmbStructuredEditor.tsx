import React, { useMemo } from "react";
import { StudioColorPicker } from "../components/StudioColorPicker";
import { StudioRange } from "../components/StudioRange";
import { StudioSelect } from "../components/StudioSelect";
import { invoke } from "@tauri-apps/api/core";
import {
  CorePretextureInfo,
  DecodedWmbMaterial,
  MaterialBankTextureOption,
  MaterialClipboardState,
  MaterialPickerOption,
  ShaderOptionDesc,
  Wmb3Material,
  Wmb3NamedParameter,
  Wmb3TextureSlotView,
  WmbSectionView,
} from "../types";
import {
  hexToRgbFloat,
  isColorLikeParamName,
  isUnitRangeScalarName,
  rgbFloatToHex,
} from "../utils/color";
import { EditableNumberCell } from "../components/EditableNumberCell";
import { IconChevronDown, IconCube } from "../components/Icons";
import { convertMaterialBetweenGames } from "../wmb-studio/contract";
import {
  bankOptionsToTextureRefs,
  MaterialPickerPopover as StudioMaterialPickerPopover,
  materialOptionsToMaterialDescs,
  TextureSlotPickerPopover as StudioTextureSlotPickerPopover,
} from "../wmb-studio/Pickers";

function TextureSlotPickerPopover({
  value,
  isB1SlotMode,
  bankTextures,
  corePretextures,
  onChange,
}: {
  value: number;
  isB1SlotMode: boolean;
  bankTextures: MaterialBankTextureOption[];
  corePretextures: CorePretextureInfo[];
  onChange: (nextVal: number) => void;
}) {
  const textures = useMemo(() => bankOptionsToTextureRefs(bankTextures), [bankTextures]);
  const emptyRoles = useMemo(() => new Map<number, string>(), []);
  return (
    <StudioTextureSlotPickerPopover
      slotIndex={0}
      slotLabel="Sampler"
      currentTexId={value}
      textures={textures}
      materials={[]}
      corePretextures={corePretextures}
      textureRoles={emptyRoles}
      useSlotIndexValue={isB1SlotMode}
      onSelectTextureId={onChange}
    />
  );
}

function MaterialPickerPopover({
  value,
  options,
  onChange,
  onNavigateMaterial,
}: {
  value: number;
  options: MaterialPickerOption[];
  onChange: (nextMatIdx: number) => void;
  onNavigateMaterial: (nodeId: string) => void;
}) {
  const materials = useMemo(() => materialOptionsToMaterialDescs(options), [options]);
  return (
    <StudioMaterialPickerPopover
      materials={materials}
      textures={[]}
      currentMatIdx={value}
      onChange={onChange}
      onJumpToMaterial={(matIdx) => {
        const opt = options.find((o) => o.index === matIdx);
        if (opt) onNavigateMaterial(opt.node_id);
      }}
    />
  );
}

export interface WmbStructuredCenterProps {
  wmb: WmbSectionView;
  wmbMatDraft: DecodedWmbMaterial | null;
  setWmbMatDraft: React.Dispatch<React.SetStateAction<DecodedWmbMaterial | null>>;
  wmb3MatDraft: Wmb3Material | null;
  setWmb3MatDraft: React.Dispatch<React.SetStateAction<Wmb3Material | null>>;
  wmbRawMode: boolean;
  setWmbRawMode: React.Dispatch<React.SetStateAction<boolean>>;
  wmbB2Override: boolean | null;
  setWmbB2Override: React.Dispatch<React.SetStateAction<boolean | null>>;
  materialClipboard: MaterialClipboardState | null;
  setMaterialClipboard: React.Dispatch<React.SetStateAction<MaterialClipboardState | null>>;
  wmbHeaderFlagsDraft: string;
  setWmbHeaderFlagsDraft: React.Dispatch<React.SetStateAction<string>>;
  wmbBboxDraft: number[];
  setWmbBboxDraft: React.Dispatch<React.SetStateAction<number[]>>;
  expandedMeshGroups: Record<string, boolean>;
  setExpandedMeshGroups: React.Dispatch<React.SetStateAction<Record<string, boolean>>>;
  expandedBatchLods: Record<string, boolean>;
  setExpandedBatchLods: React.Dispatch<React.SetStateAction<Record<string, boolean>>>;
  draggedBatchInfo: {
    isWmb3: boolean;
    batchIdx: number;
    fromMeshIdx: number;
  } | null;
  setDraggedBatchInfo: React.Dispatch<
    React.SetStateAction<{
      isWmb3: boolean;
      batchIdx: number;
      fromMeshIdx: number;
    } | null>
  >;
  dragOverMeshIdx: number | null;
  setDragOverMeshIdx: React.Dispatch<React.SetStateAction<number | null>>;
  selectAndInspectNode: (
    nodeId: string,
    mip?: number,
    channel?: string,
    opaque?: boolean,
    b2Override?: boolean | null
  ) => Promise<void>;
  runWmbMutation: (nodeId: string, operation: string, payload?: any) => Promise<void>;
  handleCrossGameMaterialConvert: (wmb: WmbSectionView, dstGame: "b1" | "b2" | "b3") => Promise<void>;
  setStatusLeft: (msg: string) => void;
}

export const WmbStructuredCenterView: React.FC<WmbStructuredCenterProps> = ({
  wmb,
  wmbMatDraft,
  setWmbMatDraft,
  wmb3MatDraft,
  setWmb3MatDraft,
  wmbRawMode,
  setWmbRawMode,
  wmbB2Override,
  setWmbB2Override,
  materialClipboard,
  setMaterialClipboard,
  wmbHeaderFlagsDraft,
  setWmbHeaderFlagsDraft,
  wmbBboxDraft,
  setWmbBboxDraft,
  expandedMeshGroups,
  setExpandedMeshGroups,
  expandedBatchLods,
  setExpandedBatchLods,
  draggedBatchInfo,
  setDraggedBatchInfo,
  dragOverMeshIdx,
  setDragOverMeshIdx,
  selectAndInspectNode,
  runWmbMutation,
  handleCrossGameMaterialConvert,
  setStatusLeft,
}) => {
  function resolveSamplerMatches(
    texId: number,
    matchBySlot: boolean,
    bankTextures: MaterialBankTextureOption[],
    corePretextures: CorePretextureInfo[]
  ): {
    bankMatch: MaterialBankTextureOption | undefined;
    pretexMatch: CorePretextureInfo | undefined;
  } {
    const unsigned = texId >>> 0;
    const bankMatch = matchBySlot
      ? bankTextures.find((b) => b.index === unsigned) ||
        bankTextures.find((b) => (b.texture_id >>> 0) === unsigned)
      : bankTextures.find((b) => (b.texture_id >>> 0) === unsigned);
    const pretexMatch = corePretextures.find((p) => ((p.id ?? p.tex_id ?? 0) >>> 0) === unsigned);
    return { bankMatch, pretexMatch };
  }

  // Render clean structured tables & Material Editor for WMB1 / WMB2 / WMB3
  function renderWmbStructuredCenter(wmb: WmbSectionView) {
    const bankTextures: MaterialBankTextureOption[] = Array.isArray(wmb.bank_textures)
      ? wmb.bank_textures
      : [];
    const corePretextures: CorePretextureInfo[] = Array.isArray(wmb.core_pretextures)
      ? wmb.core_pretextures
      : [];
    const availableShaders: ShaderOptionDesc[] = Array.isArray(wmb.available_shaders)
      ? wmb.available_shaders
      : [];

    // 1. Single Active Material Editor (WMB1 / WMB2 or WMB3)
    if (wmb.section.startsWith("mat:")) {
      // 1A. WMB3 Material Editor (Bayonetta 3 / NieR:Automata / Astral Chain)
      if (wmb.is_wmb3 && wmb3MatDraft) {
        const matIdx = parseInt(wmb.section.replace("mat:", ""), 10) || 0;
        const texViews = Array.isArray(wmb.wmb3_texture_views) ? wmb.wmb3_texture_views : [];
        const namedParams = Array.isArray(wmb.wmb3_named_params) ? wmb.wmb3_named_params : [];

        return (
          <div style={{ display: "flex", flexDirection: "column", gap: 16 }}>
            {/* Top Material Action Bar */}
            <div
              style={{
                display: "flex",
                flexWrap: "wrap",
                justifyContent: "space-between",
                alignItems: "center",
                gap: 10,
                padding: "10px 12px",
                borderRadius: 8,
                backgroundColor: "var(--brush-card)",
                border: "1px solid var(--brush-border)",
              }}
            >
              <div style={{ display: "flex", alignItems: "center", gap: 10, flexWrap: "wrap" }}>
                <span style={{ fontWeight: 600, color: "var(--brush-heading)" }}>Shader:</span>
                <StudioSelect
                  className="pill-select mono"
                  style={{ minWidth: 230 }}
                  value={wmb3MatDraft.shader_name}
                  options={[
                    { value: wmb3MatDraft.shader_name, label: wmb3MatDraft.shader_name },
                    ...availableShaders
                      .filter((s) => (s.key ?? s.id) !== wmb3MatDraft.shader_name)
                      .map((s) => ({
                        value: s.key ?? s.id ?? s.label,
                        label: s.label,
                      })),
                  ]}
                  onChange={(nextShader) => {
                    runWmbMutation(wmb.node_id, "switch_shader", {
                      index: matIdx,
                      shader: nextShader,
                    });
                  }}
                />

                <span style={{ color: "var(--brush-muted)" }}>Technique:</span>
                <input
                  type="text"
                  className="field-box mono"
                  style={{ width: 145, height: 28 }}
                  value={wmb3MatDraft.technique_name}
                  onChange={(e) => {
                    const copy = structuredClone(wmb3MatDraft);
                    copy.technique_name = e.target.value;
                    setWmb3MatDraft(copy);
                  }}
                />
              </div>

              <div style={{ display: "flex", alignItems: "center", gap: 6, flexWrap: "wrap" }}>
                <button
                  className="pill-button"
                  onClick={() => {
                    setMaterialClipboard({ game: "b3", payload: structuredClone(wmb3MatDraft) });
                    setStatusLeft(`Copied WMB3 material '${wmb3MatDraft.name}' to clipboard`);
                  }}
                >
                  Copy
                </button>
                <button
                  className="pill-button"
                  disabled={!materialClipboard}
                  onClick={async () => {
                    if (!materialClipboard) return;
                    try {
                      if (materialClipboard.game === "b3") {
                        const nextMat = {
                          ...structuredClone(materialClipboard.payload),
                          name: wmb3MatDraft.name,
                        };
                        setWmb3MatDraft(nextMat);
                        await runWmbMutation(wmb.node_id, "update_wmb3_material", {
                          index: matIdx,
                          material: nextMat,
                        });
                      } else {
                        const res = await invoke<{ converted: any }>("convert_material_action", {
                          srcGame: materialClipboard.game,
                          dstGame: "b3",
                          materialPayload: materialClipboard.payload,
                          targetShader: null,
                        });
                        const nextMat = {
                          ...res.converted,
                          name: wmb3MatDraft.name || res.converted?.name,
                        };
                        setWmb3MatDraft(nextMat);
                        await runWmbMutation(wmb.node_id, "update_wmb3_material", {
                          index: matIdx,
                          material: nextMat,
                        });
                      }
                    } catch (err) {
                      setStatusLeft(`Material paste error: ${String(err)}`);
                    }
                  }}
                  data-tooltip="Paste material from clipboard"
                >
                  Paste
                </button>
                <button
                  className="pill-button"
                  onClick={() => handleCrossGameMaterialConvert(wmb, "b2")}
                  data-tooltip="Convert this WMB3 material to Bayonetta 2 format and copy to clipboard"
                >
                  → B2
                </button>
                <button
                  className="pill-button"
                  onClick={() => handleCrossGameMaterialConvert(wmb, "b1")}
                  data-tooltip="Convert this WMB3 material to Bayonetta 1 format and copy to clipboard"
                >
                  → B1
                </button>
                <button
                  className="pill-button accent"
                  onClick={() =>
                    runWmbMutation(wmb.node_id, "update_wmb3_material", {
                      index: matIdx,
                      material: wmb3MatDraft,
                    })
                  }
                >
                  Apply WMB3 Material
                </button>
              </div>
            </div>

            {/* WMB3 Texture Samplers with Live Thumbnails, Bank Picker & Core Pretextures */}
            <div>
              <div style={{ fontWeight: 600, color: "var(--brush-heading)", marginBottom: 8 }}>
                Texture Samplers ({wmb3MatDraft.textures.length})
              </div>
              <div className="mat-sampler-grid">
                {wmb3MatDraft.textures.map(([samplerName, texId], ti) => {
                  const viewSlot = texViews[ti];
                  const { bankMatch, pretexMatch } = resolveSamplerMatches(
                    texId,
                    false,
                    bankTextures,
                    corePretextures
                  );
                  return (
                    <div key={ti} className="mat-sampler-card">
                      <div className="mat-sampler-thumb">
                        {bankMatch?.thumbnail_data_url ? (
                          <img src={bankMatch.thumbnail_data_url} alt={samplerName} />
                        ) : (
                          <span className="mono" style={{ fontSize: 10, color: "var(--brush-muted)" }}>
                            {pretexMatch ? "PRE" : "TEX"}
                          </span>
                        )}
                      </div>
                      <div style={{ flex: 1, minWidth: 0, display: "flex", flexDirection: "column", gap: 4 }}>
                        <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", gap: 6 }}>
                          <span className="mono" style={{ fontWeight: 600, fontSize: 12 }}>
                            {samplerName}
                          </span>
                          <span style={{ color: "var(--brush-muted)", fontSize: 11 }}>
                            {viewSlot?.pretexture_name || viewSlot?.role || `Slot #${ti}`}
                          </span>
                        </div>

                        {pretexMatch && (
                          <div
                            className="mono"
                            style={{
                              fontSize: 10.5,
                              color: "var(--brush-accent)",
                            }}
                          >
                            Core Pretexture: {pretexMatch.name}
                          </div>
                        )}

                        <TextureSlotPickerPopover
                          value={texId >>> 0}
                          isB1SlotMode={false}
                          bankTextures={bankTextures}
                          corePretextures={corePretextures}
                          onChange={(nextVal) => {
                            const copy = structuredClone(wmb3MatDraft);
                            copy.textures[ti][1] = nextVal >>> 0;
                            setWmb3MatDraft(copy);
                          }}
                        />

                        <div style={{ display: "flex", alignItems: "center", gap: 6 }}>
                          <input
                            type="text"
                            className="field-box mono"
                            style={{ flex: 1, height: 24, fontSize: 11.5 }}
                            value={`0x${(texId >>> 0).toString(16).toUpperCase().padStart(8, "0")}`}
                            onChange={(e) => {
                              const clean = e.target.value.replace(/^0x/i, "");
                              const parsed = parseInt(clean || "0", 16);
                              if (!Number.isNaN(parsed)) {
                                const copy = structuredClone(wmb3MatDraft);
                                copy.textures[ti][1] = parsed >>> 0;
                                setWmb3MatDraft(copy);
                              }
                            }}
                          />
                          {bankMatch && (
                            <button
                              className="pill-button"
                              style={{ height: 24, padding: "0 8px", fontSize: 11 }}
                              data-tooltip={`Jump to ${bankMatch.label}`}
                              onClick={() => selectAndInspectNode(bankMatch.node_id)}
                            >
                              ↗ Open
                            </button>
                          )}
                        </div>
                      </div>
                    </div>
                  );
                })}
              </div>
            </div>

            {/* WMB3 Shader Parameters (Auto-grouped _X/_Y/_Z/_W vectors + Color Swatches + 0.0..1.0 Sliders) */}
            <div>
              <div style={{ fontWeight: 600, color: "var(--brush-heading)", marginBottom: 8 }}>
                Shader Parameters ({namedParams.length} floats across {(wmb3MatDraft.parameter_groups || []).length} groups)
              </div>
              <table className="struct-table">
                <thead>
                  <tr>
                    <th style={{ width: 95 }}>Group / Idx</th>
                    <th style={{ width: 230 }}>Parameter Name</th>
                    <th>Value(s), Slider &amp; Color Swatch</th>
                  </tr>
                </thead>
                <tbody>
                  {(() => {
                    const paramGroups = Array.isArray(wmb3MatDraft.parameter_groups)
                      ? wmb3MatDraft.parameter_groups
                      : [];
                    const rows: React.ReactNode[] = [];

                    paramGroups.forEach((grpTuple, gi) => {
                      const floats: number[] = Array.isArray(grpTuple?.[1]) ? grpTuple[1] : [];
                      const getName = (pi: number) =>
                        namedParams.find((np) => np.group_index === gi && np.param_index === pi)
                          ?.name || `Group${gi}_${String(pi).padStart(2, "0")}`;

                      let pi = 0;
                      while (pi < floats.length) {
                        const rawName = getName(pi);
                        const matchVec = rawName.match(/^(.*?)_([XYZW])$/i);
                        if (matchVec && matchVec[2].toUpperCase() === "X") {
                          const baseName = matchVec[1];
                          const compIndices: number[] = [pi];
                          const compLabels: string[] = ["X"];
                          const expectedSuffixes = ["Y", "Z", "W"];
                          for (const suf of expectedSuffixes) {
                            const nextIdx = pi + compIndices.length;
                            if (
                              nextIdx < floats.length &&
                              getName(nextIdx).toLowerCase() === `${baseName}_${suf}`.toLowerCase()
                            ) {
                              compIndices.push(nextIdx);
                              compLabels.push(suf);
                            } else {
                              break;
                            }
                          }
                          if (compIndices.length >= 2) {
                            const startPi = pi;
                            const endPi = pi + compIndices.length - 1;
                            const vecVals = compIndices.map((idx) => floats[idx] ?? 0);
                            const isColor =
                              compIndices.length >= 3 && isColorLikeParamName(baseName);
                            const showUnitSlider = isUnitRangeScalarName(baseName);
                            rows.push(
                              <tr key={`g${gi}:v${startPi}`}>
                                <td className="mono" style={{ color: "var(--brush-muted)" }}>
                                  G{gi} [{startPi}..{endPi}]
                                </td>
                                <td className="mono" style={{ fontWeight: 600 }} data-tooltip={baseName}>
                                  {baseName}
                                </td>
                                <td>
                                  <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
                                    {isColor && (
                                      <StudioColorPicker
                                        value={rgbFloatToHex(vecVals)}
                                        onChange={(hex) => {
                                          const [r, g, b] = hexToRgbFloat(hex);
                                          const copy = structuredClone(wmb3MatDraft);
                                          copy.parameter_groups[gi][1][compIndices[0]] = r;
                                          copy.parameter_groups[gi][1][compIndices[1]] = g;
                                          copy.parameter_groups[gi][1][compIndices[2]] = b;
                                          setWmb3MatDraft(copy);
                                        }}
                                        style={{ width: 26, height: 26 }}
                                        data-tooltip="Pick RGB color"
                                      />
                                    )}
                                    <div
                                      style={{
                                        flex: 1,
                                        display: "grid",
                                        gridTemplateColumns: `repeat(${compIndices.length}, minmax(115px, 1fr))`,
                                        gap: 6,
                                      }}
                                    >
                                      {compIndices.map((cIdx, ci) => {
                                        const fv = floats[cIdx] ?? 0;
                                        return (
                                          <div
                                            key={cIdx}
                                            style={{ display: "flex", flexDirection: "column", gap: 2 }}
                                          >
                                            <span
                                              className="mono"
                                              style={{ fontSize: 10, color: "var(--brush-muted)" }}
                                            >
                                              {compLabels[ci]}
                                            </span>
                                            <div className="param-slider-group">
                                              {showUnitSlider && fv >= 0.0 && fv <= 1.0 && (
                                                <StudioRange
                                                  min={0}
                                                  max={1}
                                                  step={0.01}
                                                  className="param-range"
                                                  value={fv}
                                                  onChange={(v) => {
                                                    const copy = structuredClone(wmb3MatDraft);
                                                    copy.parameter_groups[gi][1][cIdx] = v;
                                                    setWmb3MatDraft(copy);
                                                  }}
                                                />
                                              )}
                                              <input
                                                type="number"
                                                step="0.01"
                                                className="field-box mono"
                                                style={{ width: "100%", height: 25 }}
                                                value={Number(fv.toFixed(4))}
                                                onChange={(e) => {
                                                  const v = parseFloat(e.target.value);
                                                  if (!Number.isNaN(v)) {
                                                    const copy = structuredClone(wmb3MatDraft);
                                                    copy.parameter_groups[gi][1][cIdx] = v;
                                                    setWmb3MatDraft(copy);
                                                  }
                                                }}
                                              />
                                            </div>
                                          </div>
                                        );
                                      })}
                                    </div>
                                  </div>
                                </td>
                              </tr>
                            );
                            pi += compIndices.length;
                            continue;
                          }
                        }

                        // Standalone scalar parameter row
                        const curPi = pi;
                        const scalarVal = floats[curPi] ?? 0;
                        const useSlider =
                          isUnitRangeScalarName(rawName) &&
                          scalarVal >= 0.0 &&
                          scalarVal <= 1.0;
                        rows.push(
                          <tr key={`g${gi}:s${curPi}`}>
                            <td className="mono" style={{ color: "var(--brush-muted)" }}>
                              G{gi} [{curPi}]
                            </td>
                            <td className="mono" style={{ fontWeight: 600 }} data-tooltip={rawName}>
                              {rawName}
                            </td>
                            <td>
                              <div className="param-slider-group" style={{ maxWidth: 280 }}>
                                {useSlider && (
                                  <StudioRange
                                    min={0}
                                    max={1}
                                    step={0.01}
                                    className="param-range"
                                    style={{ maxWidth: 120 }}
                                    value={scalarVal}
                                    onChange={(v) => {
                                      const copy = structuredClone(wmb3MatDraft);
                                      copy.parameter_groups[gi][1][curPi] = v;
                                      setWmb3MatDraft(copy);
                                    }}
                                  />
                                )}
                                <input
                                  type="number"
                                  step="0.01"
                                  className="field-box mono"
                                  style={{ width: 130, height: 25 }}
                                  value={Number(scalarVal.toFixed(4))}
                                  onChange={(e) => {
                                    const v = parseFloat(e.target.value);
                                    if (!Number.isNaN(v)) {
                                      const copy = structuredClone(wmb3MatDraft);
                                      copy.parameter_groups[gi][1][curPi] = v;
                                      setWmb3MatDraft(copy);
                                    }
                                  }}
                                />
                              </div>
                            </td>
                          </tr>
                        );
                        pi += 1;
                      }
                    });
                    return rows;
                  })()}
                </tbody>
              </table>
            </div>

            {/* WMB3 Scalar Variables */}
            {wmb3MatDraft.variables.length > 0 && (
              <div>
                <div style={{ fontWeight: 600, color: "var(--brush-heading)", marginBottom: 8 }}>
                  Scalar Material Variables ({wmb3MatDraft.variables.length})
                </div>
                <table className="struct-table">
                  <thead>
                    <tr>
                      <th>Variable Name</th>
                      <th>Float Value</th>
                    </tr>
                  </thead>
                  <tbody>
                    {wmb3MatDraft.variables.map(([vName, vVal], vi) => (
                      <tr key={vi}>
                        <td className="mono" style={{ fontWeight: 600 }}>
                          {vName}
                        </td>
                        <td>
                          <input
                            type="number"
                            step="0.01"
                            className="field-box mono"
                            style={{ width: 150, height: 25 }}
                            value={Number((vVal ?? 0).toFixed(4))}
                            onChange={(e) => {
                              const v = parseFloat(e.target.value);
                              if (!Number.isNaN(v)) {
                                const copy = structuredClone(wmb3MatDraft);
                                copy.variables[vi][1] = v;
                                setWmb3MatDraft(copy);
                              }
                            }}
                          />
                        </td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            )}
          </div>
        );
      }

      // 1B. WMB1 / WMB2 Material Editor (Bayonetta 1 & Bayonetta 2)
      if (wmbMatDraft) {
        const fields = Array.isArray(wmbMatDraft.fields) ? wmbMatDraft.fields : [];
        const rawDwords = Array.isArray(wmbMatDraft.raw_dwords) ? wmbMatDraft.raw_dwords : [];
        const effectiveB2 = wmbB2Override ?? Boolean(wmbMatDraft.is_bayo2 ?? wmb.is_bayo2);
        const samplerFields = fields
          .map((f, idx) => ({ f, idx }))
          .filter((item) => item.f.sampler_tex_id !== null && item.f.sampler_tex_id !== undefined);
        const vectorFields = fields
          .map((f, idx) => ({ f, idx }))
          .filter((item) => item.f.float_values !== null && item.f.float_values !== undefined);

        const currentShaderVal = effectiveB2
          ? wmbMatDraft.shader_name || ""
          : String(wmbMatDraft.mat_type ?? 0);

        return (
          <div style={{ display: "flex", flexDirection: "column", gap: 16 }}>
            {/* Top Material Action Bar: B1/B2 Mode Toggle, Typed/Raw Toggle, Shader Dropdown, Copy/Paste/Convert */}
            <div
              style={{
                display: "flex",
                flexWrap: "wrap",
                justifyContent: "space-between",
                alignItems: "center",
                gap: 10,
                padding: "10px 12px",
                borderRadius: 8,
                backgroundColor: "var(--brush-card)",
                border: "1px solid var(--brush-border)",
              }}
            >
              <div style={{ display: "flex", alignItems: "center", gap: 8, flexWrap: "wrap" }}>
                {/* B1 vs B2 Game Mode Toggle (`WMB0`) */}
                <button
                  className={`pill-button ${!effectiveB2 ? "accent" : ""}`}
                  style={{ height: 27, padding: "0 9px", fontSize: 11.5 }}
                  onClick={() => {
                    setWmbB2Override(false);
                    selectAndInspectNode(wmb.node_id, undefined, undefined, undefined, false);
                  }}
                  data-tooltip="Interpret material using Bayonetta 1 layout database (0x24..0xBC)"
                >
                  Bayonetta 1
                </button>
                <button
                  className={`pill-button ${effectiveB2 ? "accent" : ""}`}
                  style={{ height: 27, padding: "0 9px", fontSize: 11.5 }}
                  onClick={() => {
                    setWmbB2Override(true);
                    selectAndInspectNode(wmb.node_id, undefined, undefined, undefined, true);
                  }}
                  data-tooltip="Interpret material using Bayonetta 2 shader database (843 shaders)"
                >
                  Bayonetta 2
                </button>

                <div className="toolbar-spacer" />

                {/* Typed View vs Raw Vec4 Words Toggle */}
                <button
                  className={`pill-button ${!wmbRawMode ? "accent" : ""}`}
                  style={{ height: 27, padding: "0 9px", fontSize: 11.5 }}
                  onClick={() => setWmbRawMode(false)}
                >
                  Typed View
                </button>
                <button
                  className={`pill-button ${wmbRawMode ? "accent" : ""}`}
                  style={{ height: 27, padding: "0 9px", fontSize: 11.5 }}
                  onClick={() => setWmbRawMode(true)}
                >
                  Raw Words ({rawDwords.length})
                </button>

                <div className="toolbar-spacer" />

                {/* Shader / Material Type Dropdown */}
                <span style={{ fontWeight: 600, color: "var(--brush-heading)" }}>
                  {effectiveB2 ? "Shader:" : "Mat Type:"}
                </span>
                <StudioSelect
                  className="pill-select mono"
                  style={{ minWidth: 230, height: 28 }}
                  value={currentShaderVal}
                  options={[
                    {
                      value: currentShaderVal,
                      label: wmbMatDraft.shader_name
                        ? `${wmbMatDraft.shader_name} (0x${wmbMatDraft.mat_type.toString(16).toUpperCase()})`
                        : `Type ${wmbMatDraft.mat_type ?? 0} (0x${(wmbMatDraft.mat_type ?? 0).toString(16).toUpperCase().padStart(2, "0")})`,
                    },
                    ...availableShaders
                      .filter((s) => (s.key ?? s.id) !== currentShaderVal)
                      .map((s) => ({
                        value: s.key ?? s.id ?? s.label,
                        label: s.label,
                      })),
                  ]}
                  onChange={(targetShader) => {
                    runWmbMutation(wmb.node_id, "switch_shader", {
                      index: wmbMatDraft.index,
                      shader: targetShader,
                      is_bayo2: effectiveB2,
                    });
                  }}
                />
              </div>

              <div style={{ display: "flex", alignItems: "center", gap: 6, flexWrap: "wrap" }}>
                <button
                  className="pill-button"
                  style={{ height: 27 }}
                  onClick={() => {
                    setMaterialClipboard({
                      game: effectiveB2 ? "b2" : "b1",
                      payload: structuredClone(wmbMatDraft),
                    });
                    setStatusLeft(`Copied Material #${wmbMatDraft.index} to clipboard`);
                  }}
                >
                  Copy
                </button>
                <button
                  className="pill-button"
                  style={{ height: 27 }}
                  disabled={!materialClipboard}
                  onClick={async () => {
                    if (!materialClipboard) return;
                    const dstGame = effectiveB2 ? "b2" : "b1";
                    if (materialClipboard.game === dstGame) {
                      const nextMat = {
                        ...materialClipboard.payload,
                        index: wmbMatDraft.index,
                      };
                      setWmbMatDraft(nextMat);
                      await runWmbMutation(wmb.node_id, "update_material", nextMat);
                    } else {
                      const res = await invoke<{ converted: any }>("convert_material_action", {
                        srcGame: materialClipboard.game,
                        dstGame,
                        materialPayload: materialClipboard.payload,
                        targetShader: null,
                      });
                      const nextMat = {
                        ...res.converted,
                        index: wmbMatDraft.index,
                      };
                      setWmbMatDraft(nextMat);
                      await runWmbMutation(wmb.node_id, "update_material", nextMat);
                    }
                  }}
                >
                  Paste
                </button>
                <button
                  className="pill-button"
                  style={{ height: 27 }}
                  onClick={() =>
                    handleCrossGameMaterialConvert(wmb, effectiveB2 ? "b1" : "b2")
                  }
                  data-tooltip={`Convert Material to ${effectiveB2 ? "Bayonetta 1" : "Bayonetta 2"}`}
                >
                  {effectiveB2 ? "→ B1" : "→ B2"}
                </button>
                <button
                  className="pill-button"
                  style={{ height: 27 }}
                  onClick={() => handleCrossGameMaterialConvert(wmb, "b3")}
                  data-tooltip="Convert Material to Bayonetta 3 (WMB3) and copy to clipboard"
                >
                  → B3
                </button>
                <button
                  className="pill-button accent"
                  style={{ height: 27 }}
                  onClick={() => runWmbMutation(wmb.node_id, "update_material", wmbMatDraft)}
                >
                  Apply Material Changes
                </button>
              </div>
            </div>

            {/* Shader Family & Decoded Flags Banner */}
            {(wmbMatDraft.shader_family ||
              (wmbMatDraft.shader_flags_notes && wmbMatDraft.shader_flags_notes.length > 0)) && (
              <div
                style={{
                  display: "flex",
                  flexWrap: "wrap",
                  gap: 8,
                  alignItems: "center",
                  padding: "6px 10px",
                  borderRadius: 7,
                  backgroundColor: "var(--brush-control)",
                  border: "1px solid var(--brush-border)",
                  fontSize: 11.5,
                }}
              >
                {wmbMatDraft.shader_family && (
                  <span className="mono" style={{ fontWeight: 600, color: "var(--brush-accent)" }}>
                    Family: {wmbMatDraft.shader_family}
                  </span>
                )}
                {wmbMatDraft.shader_flags_notes?.map((note, ni) => (
                  <span key={ni} className="tree-badge">
                    {note}
                  </span>
                ))}
              </div>
            )}

            {/* Typed View: Texture Sampler Cards + Vector Parameters Table */}
            {!wmbRawMode && fields.length > 0 && (
              <>
                {samplerFields.length > 0 && (
                  <div>
                    <div style={{ fontWeight: 600, color: "var(--brush-heading)", marginBottom: 8 }}>
                      Texture Samplers ({samplerFields.length})
                    </div>
                    <div className="mat-sampler-grid">
                      {samplerFields.map(({ f: param, idx: pIdx }) => {
                        const texVal = (param.sampler_tex_id ?? 0) >>> 0;
                        const { bankMatch, pretexMatch } = resolveSamplerMatches(
                          texVal,
                          !effectiveB2,
                          bankTextures,
                          corePretextures
                        );
                        return (
                          <div key={pIdx} className="mat-sampler-card">
                            <div className="mat-sampler-thumb">
                              {bankMatch?.thumbnail_data_url ? (
                                <img src={bankMatch.thumbnail_data_url} alt={param.name} />
                              ) : (
                                <span
                                  className="mono"
                                  style={{ fontSize: 10, color: "var(--brush-muted)" }}
                                >
                                  {pretexMatch || param.pretexture_name ? "PRE" : "TEX"}
                                </span>
                              )}
                            </div>

                            <div
                              style={{
                                flex: 1,
                                minWidth: 0,
                                display: "flex",
                                flexDirection: "column",
                                gap: 4,
                              }}
                            >
                              <div
                                style={{
                                  display: "flex",
                                  justifyContent: "space-between",
                                  alignItems: "center",
                                  gap: 6,
                                }}
                              >
                                <span className="mono" style={{ fontWeight: 600, fontSize: 12 }}>
                                  {param.name}
                                </span>
                                <span style={{ color: "var(--brush-muted)", fontSize: 11 }}>
                                  {param.role || param.param_type}
                                </span>
                              </div>

                              {(pretexMatch || param.pretexture_name) && (
                                <div
                                  className="mono"
                                  style={{ fontSize: 10.5, color: "var(--brush-accent)" }}
                                >
                                  Core Pretexture: {pretexMatch?.name || param.pretexture_name}
                                </div>
                              )}

                              {/* Bank & Core Pretexture Custom Popover with Live Thumbnails */}
                              <TextureSlotPickerPopover
                                value={texVal}
                                isB1SlotMode={!effectiveB2}
                                bankTextures={bankTextures}
                                corePretextures={corePretextures}
                                onChange={(nextVal) => {
                                  const copy = structuredClone(wmbMatDraft);
                                  copy.fields[pIdx].sampler_tex_id = nextVal >>> 0;
                                  setWmbMatDraft(copy);
                                }}
                              />

                              <div style={{ display: "flex", alignItems: "center", gap: 6 }}>
                                <input
                                  type="text"
                                  className="field-box mono"
                                  style={{ flex: 1, height: 24, fontSize: 11.5 }}
                                  value={
                                    effectiveB2 || texVal > 255
                                      ? `0x${texVal.toString(16).toUpperCase().padStart(8, "0")}`
                                      : String(texVal)
                                  }
                                  onChange={(e) => {
                                    const rawStr = e.target.value.trim();
                                    const parsed = rawStr.toLowerCase().startsWith("0x")
                                      ? parseInt(rawStr.slice(2) || "0", 16)
                                      : parseInt(rawStr || "0", 10);
                                    if (!Number.isNaN(parsed)) {
                                      const copy = structuredClone(wmbMatDraft);
                                      copy.fields[pIdx].sampler_tex_id = parsed >>> 0;
                                      setWmbMatDraft(copy);
                                    }
                                  }}
                                />
                                {bankMatch && (
                                  <button
                                    className="pill-button"
                                    style={{ height: 24, padding: "0 8px", fontSize: 11 }}
                                    data-tooltip={`Open ${bankMatch.label} in Texture Viewer`}
                                    onClick={() => selectAndInspectNode(bankMatch.node_id)}
                                  >
                                    ↗ Open
                                  </button>
                                )}
                              </div>
                            </div>
                          </div>
                        );
                      })}
                    </div>
                  </div>
                )}

                {vectorFields.length > 0 && (
                  <div>
                    <div style={{ fontWeight: 600, color: "var(--brush-heading)", marginBottom: 8 }}>
                      Shader Vector &amp; Color Parameters ({vectorFields.length})
                    </div>
                    <table className="struct-table">
                      <thead>
                        <tr>
                          <th style={{ width: 230 }}>Parameter Name</th>
                          <th>Components (X, Y, Z, W), Sliders &amp; Color Swatch</th>
                        </tr>
                      </thead>
                      <tbody>
                        {vectorFields.map(({ f: param, idx: pIdx }) => {
                          const flts = param.float_values || [0, 0, 0, 0];
                          const isColor = isColorLikeParamName(param.name, param.is_color);
                          const showSlider = isColor || isUnitRangeScalarName(param.name);
                          const labels = param.component_labels || ["X", "Y", "Z", "W"];
                          return (
                            <tr key={pIdx}>
                              <td
                                className="mono"
                                style={{ fontWeight: 600 }}
                                data-tooltip={param.description || param.role || param.param_type}
                              >
                                {param.name}
                              </td>
                              <td>
                                <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
                                  {isColor && (
                                    <StudioColorPicker
                                      value={rgbFloatToHex(flts)}
                                      onChange={(hex) => {
                                        const [r, g, b] = hexToRgbFloat(hex);
                                        const copy = structuredClone(wmbMatDraft);
                                        if (copy.fields[pIdx].float_values) {
                                          copy.fields[pIdx].float_values![0] = r;
                                          copy.fields[pIdx].float_values![1] = g;
                                          copy.fields[pIdx].float_values![2] = b;
                                          setWmbMatDraft(copy);
                                        }
                                      }}
                                      style={{ width: 26, height: 26 }}
                                      data-tooltip="Pick RGB color"
                                    />
                                  )}
                                  <div
                                    style={{
                                      flex: 1,
                                      display: "grid",
                                      gridTemplateColumns: "repeat(4, minmax(125px, 1fr))",
                                      gap: 6,
                                    }}
                                  >
                                    {flts.map((fv, fi) => (
                                      <div
                                        key={fi}
                                        style={{ display: "flex", flexDirection: "column", gap: 2 }}
                                      >
                                        <span
                                          className="mono"
                                          style={{ fontSize: 10, color: "var(--brush-muted)" }}
                                        >
                                          {labels[fi] || `C${fi}`}
                                        </span>
                                        <div className="param-slider-group">
                                          {showSlider && (fv ?? 0) >= 0.0 && (fv ?? 0) <= 1.0 && (
                                            <StudioRange
                                              min={0}
                                              max={1}
                                              step={0.01}
                                              className="param-range"
                                              value={fv ?? 0}
                                              onChange={(val) => {
                                                const copy = structuredClone(wmbMatDraft);
                                                if (copy.fields[pIdx].float_values) {
                                                  copy.fields[pIdx].float_values![fi] = val;
                                                  setWmbMatDraft(copy);
                                                }
                                              }}
                                            />
                                          )}
                                          <input
                                            type="number"
                                            step="0.01"
                                            className="field-box mono"
                                            style={{ width: "100%", height: 25 }}
                                            value={Number((fv ?? 0).toFixed(4))}
                                            onChange={(e) => {
                                              const val = parseFloat(e.target.value);
                                              if (!Number.isNaN(val)) {
                                                const copy = structuredClone(wmbMatDraft);
                                                if (copy.fields[pIdx].float_values) {
                                                  copy.fields[pIdx].float_values![fi] = val;
                                                  setWmbMatDraft(copy);
                                                }
                                              }
                                            }}
                                          />
                                        </div>
                                      </div>
                                    ))}
                                  </div>
                                </div>
                              </td>
                            </tr>
                          );
                        })}
                      </tbody>
                    </table>
                  </div>
                )}
              </>
            )}

            {/* Raw Parameter Words View (shown when Raw Words toggle is active OR when shader has no typed fields) */}
            {(wmbRawMode || fields.length === 0) && (
              <div>
                <div
                  style={{
                    display: "flex",
                    justifyContent: "space-between",
                    alignItems: "center",
                    marginBottom: 8,
                  }}
                >
                  <span style={{ fontWeight: 600, color: "var(--brush-heading)" }}>
                    Raw Material Parameter Words ({rawDwords.length} × u32 / f32)
                  </span>
                </div>
                <table className="struct-table">
                  <thead>
                    <tr>
                      <th style={{ width: 90 }}>Word #</th>
                      <th style={{ width: 110 }}>Byte Offset</th>
                      <th>Hex Value (u32)</th>
                      <th>Float Value (f32)</th>
                    </tr>
                  </thead>
                  <tbody>
                    {rawDwords.map((dw, di) => {
                      const buf = new ArrayBuffer(4);
                      const view = new DataView(buf);
                      view.setUint32(0, dw >>> 0, true);
                      const fVal = view.getFloat32(0, true);
                      return (
                        <tr key={di}>
                          <td className="mono">#{di}</td>
                          <td className="mono" style={{ color: "var(--brush-muted)" }}>
                            +0x{(4 + di * 4).toString(16).toUpperCase().padStart(2, "0")}
                          </td>
                          <td>
                            <input
                              type="text"
                              className="field-box mono"
                              style={{ width: 140, height: 25 }}
                              value={`0x${(dw >>> 0).toString(16).toUpperCase().padStart(8, "0")}`}
                              onChange={(e) => {
                                const clean = e.target.value.replace(/^0x/i, "");
                                const parsed = parseInt(clean || "0", 16);
                                if (!Number.isNaN(parsed)) {
                                  const copy = structuredClone(wmbMatDraft);
                                  copy.raw_dwords[di] = parsed >>> 0;
                                  copy.fields = [];
                                  setWmbMatDraft(copy);
                                }
                              }}
                            />
                          </td>
                          <td>
                            <input
                              type="number"
                              step="0.01"
                              className="field-box mono"
                              style={{ width: 140, height: 25 }}
                              value={
                                Number.isFinite(fVal) && Math.abs(fVal) < 1e7
                                  ? Number(fVal.toFixed(5))
                                  : 0
                              }
                              onChange={(e) => {
                                const fv = parseFloat(e.target.value);
                                if (!Number.isNaN(fv)) {
                                  const b2 = new ArrayBuffer(4);
                                  const v2 = new DataView(b2);
                                  v2.setFloat32(0, fv, true);
                                  const copy = structuredClone(wmbMatDraft);
                                  copy.raw_dwords[di] = v2.getUint32(0, true);
                                  copy.fields = [];
                                  setWmbMatDraft(copy);
                                }
                              }}
                            />
                          </td>
                        </tr>
                      );
                    })}
                  </tbody>
                </table>
              </div>
            )}
          </div>
        );
      }
    }

    // 2. Materials Table Summary
    if (wmb.section === "materials" && Array.isArray(wmb.data)) {
      const parentBase = wmb.node_id.replace(/:materials$/, "");
      return (
        <table className="struct-table">
          <thead>
            <tr>
              <th style={{ width: 70 }}>Mat #</th>
              <th>Shader / Material Name</th>
              <th>Type / Technique</th>
              <th>Parameters</th>
              <th style={{ width: 110 }}>Action</th>
            </tr>
          </thead>
          <tbody>
            {(wmb.data as any[]).map((m, mi) => (
              <tr key={mi}>
                <td className="mono">#{mi}</td>
                <td className="mono" style={{ fontWeight: 600 }}>
                  {m.name || m.shader_name || `Material_${mi}`}
                </td>
                <td className="mono" style={{ color: "var(--brush-muted)" }}>
                  {m.technique_name
                    ? `${m.shader_name} (${m.technique_name})`
                    : m.mat_type !== undefined
                    ? `0x${Number(m.mat_type).toString(16).toUpperCase().padStart(2, "0")}${
                        m.shader_family ? ` · ${m.shader_family}` : ""
                      }`
                    : "—"}
                </td>
                <td className="mono">
                  {Array.isArray(m.fields)
                    ? `${m.fields.length} params (${(m.raw_dwords?.length ?? 0) * 4 + 4} B)`
                    : Array.isArray(m.textures)
                    ? `${m.textures.length} tex, ${
                        Array.isArray(m.parameter_groups)
                          ? m.parameter_groups.reduce((acc: number, g: any[]) => acc + (Array.isArray(g?.[1]) ? g[1].length : 0), 0)
                          : Array.isArray(m.parameters)
                          ? m.parameters.reduce((acc: number, g: any[]) => acc + (g?.length ?? 0), 0)
                          : 0
                      } floats`
                    : "—"}
                </td>
                <td>
                  <button
                    className="pill-button"
                    style={{ height: 24, padding: "0 10px", fontSize: 11.5 }}
                    onClick={() => selectAndInspectNode(`${parentBase}:mat:${mi}`)}
                  >
                    Edit Material
                  </button>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      );
    }

    // 3. Bones Table (Editable for both WMB1/2 and WMB3!)
    if (wmb.section === "bones") {
      const boneList: any[] = Array.isArray(wmb.data?.bones)
        ? wmb.data.bones
        : Array.isArray(wmb.data)
        ? wmb.data
        : [];
      return (
        <div style={{ display: "flex", flexDirection: "column", gap: 10 }}>
          {wmb.is_wmb3 && (
            <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center" }}>
              <span style={{ fontWeight: 600, color: "var(--brush-heading)" }}>
                WMB3 Skeleton ({boneList.length} bones)
              </span>
              <button
                className="pill-button accent"
                onClick={() => runWmbMutation(wmb.node_id, "set_tpose")}
              >
                Bake Skeleton &amp; Vertices to T-Pose
              </button>
            </div>
          )}
          <table className="struct-table">
            <thead>
              <tr>
                <th style={{ width: 65 }}>Bone #</th>
                <th style={{ width: 95 }}>Global ID</th>
                <th style={{ width: 95 }}>Parent #</th>
                <th>World Position (X, Y, Z)</th>
                <th>Relative / Local Offset (X, Y, Z)</th>
              </tr>
            </thead>
            <tbody>
              {boneList.map((b, bi) => {
                const pos = Array.isArray(b.position)
                  ? b.position
                  : b.position
                  ? [b.position.x, b.position.y, b.position.z]
                  : b.world_position
                  ? [b.world_position.x, b.world_position.y, b.world_position.z]
                  : [0, 0, 0];
                const rel = Array.isArray(b.relative_position)
                  ? b.relative_position
                  : b.relative_position
                  ? [b.relative_position.x, b.relative_position.y, b.relative_position.z]
                  : b.local_position
                  ? [b.local_position.x, b.local_position.y, b.local_position.z]
                  : [0, 0, 0];
                return (
                  <tr key={bi}>
                    <td className="mono">#{b.index ?? bi}</td>
                    <td className="mono" style={{ fontWeight: 600 }}>
                      {b.global_id ?? b.id ?? bi}
                    </td>
                    <td>
                      <input
                        type="number"
                        className="field-box mono"
                        style={{ width: 85, height: 24 }}
                        defaultValue={b.parent_index}
                        onBlur={(e) => {
                          const pVal = parseInt(e.target.value, 10);
                          if (!Number.isNaN(pVal) && pVal !== b.parent_index) {
                            if (pVal < -1 || pVal >= boneList.length || pVal === bi) {
                              setStatusLeft(
                                `Invalid parent bone #${pVal} for bone #${bi} (must be -1..${boneList.length - 1} and not self)`
                              );
                              e.target.value = String(b.parent_index);
                              return;
                            }
                            let cur = pVal;
                            const seen = new Set<number>([bi]);
                            let hasCycle = false;
                            while (cur >= 0 && cur < boneList.length) {
                              if (seen.has(cur)) {
                                hasCycle = true;
                                break;
                              }
                              seen.add(cur);
                              cur = Number(boneList[cur]?.parent_index ?? -1);
                            }
                            if (hasCycle) {
                              setStatusLeft(
                                `Cannot reparent bone #${bi} to descendant #${pVal} (cycle detected)`
                              );
                              e.target.value = String(b.parent_index);
                              return;
                            }
                            runWmbMutation(wmb.node_id, "update_bone", {
                              index: bi,
                              parent_index: pVal,
                            });
                          }
                        }}
                      />
                    </td>
                    <td className="mono">
                      {Number(pos[0] ?? 0).toFixed(3)}, {Number(pos[1] ?? 0).toFixed(3)},{" "}
                      {Number(pos[2] ?? 0).toFixed(3)}
                    </td>
                    <td className="mono">
                      {Number(rel[0] ?? 0).toFixed(3)}, {Number(rel[1] ?? 0).toFixed(3)},{" "}
                      {Number(rel[2] ?? 0).toFixed(3)}
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        </div>
      );
    }

    // 4. Interactive Meshes -> Batches (-> LODs) Tree with Drag-and-Drop & Custom Material Picker
    if (wmb.section === "meshes" || wmb.section.startsWith("mesh:")) {
      const matOptions: MaterialPickerOption[] = Array.isArray(wmb.material_options)
        ? wmb.material_options
        : [];

      // 4A. WMB3 Meshes -> Batches -> LODs Tree
      if (wmb.is_wmb3 && wmb.data) {
        const meshes3: any[] = Array.isArray(wmb.data.meshes) ? wmb.data.meshes : [];
        const batches3: any[] = Array.isArray(wmb.data.batches) ? wmb.data.batches : [];
        const lods3: any[] = Array.isArray(wmb.data.lods) ? wmb.data.lods : [];

        // Build per-batch metadata & LOD sub-rows using `inf.mesh_index` (NOT `mesh_group_index`!)
        interface Wmb3BatchResolved {
          batchIdx: number;
          batch: any;
          meshIndex: number;
          materialIndex: number;
          lodRows: {
            lodIndex: number;
            lodName: string;
            lodLevel: number;
            meshMatIndex: number;
            materialIndex: number;
            meshIndex: number;
            vertexGroupIndex: number;
            callTreeNodeIndex: number;
            uB: number;
          }[];
        }

        const resolvedBatches: Wmb3BatchResolved[] = batches3.map((b, bi) => {
          let primaryMeshIdx = 0;
          let primaryMatIdx = 0;
          let foundPrimary = false;
          const lodRows: Wmb3BatchResolved["lodRows"] = [];

          lods3.forEach((lod, li) => {
            const start = lod.batch_start ?? 0;
            const infos: any[] = Array.isArray(lod.batch_infos) ? lod.batch_infos : [];
            if (bi >= start && bi < start + infos.length) {
              const inf = infos[bi - start];
              const mIdx = inf?.mesh_index ?? inf?.mesh_group_index ?? 0;
              const matIdx = inf?.material_index ?? 0;
              if (!foundPrimary) {
                primaryMeshIdx = mIdx;
                primaryMatIdx = matIdx;
                foundPrimary = true;
              }
              lodRows.push({
                lodIndex: li,
                lodName: lod.name || `lod${li}`,
                lodLevel: lod.lod_level ?? li,
                meshMatIndex: inf?.mesh_material_index ?? 0,
                materialIndex: matIdx,
                meshIndex: mIdx,
                vertexGroupIndex: inf?.vertex_group_index ?? b.vertex_group_index ?? 0,
                callTreeNodeIndex: inf?.call_tree_node_index ?? -1,
                uB: inf?.u_b ?? 0,
              });
            }
          });

          return {
            batchIdx: bi,
            batch: b,
            meshIndex: primaryMeshIdx,
            materialIndex: primaryMatIdx,
            lodRows,
          };
        });

        const meshGroupCount = Math.max(
          meshes3.length,
          resolvedBatches.reduce((acc, r) => Math.max(acc, r.meshIndex + 1), 1)
        );

        return (
          <div style={{ display: "flex", flexDirection: "column", gap: 10 }}>
            <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", marginBottom: 4 }}>
              <div>
                <div style={{ fontWeight: 600, color: "var(--brush-heading)" }}>
                  WMB3 Mesh Groups ({meshes3.length}) → Batches ({batches3.length}) → LODs ({lods3.length})
                </div>
                <div style={{ color: "var(--brush-muted)", fontSize: 11.5 }}>
                  Drag any Batch row onto another Mesh Group to reassign it, or click ⚙ Header / LODs to edit all batch header fields.
                </div>
              </div>
            </div>

            {Array.from({ length: meshGroupCount }).map((_, mi) => {
              const mg = meshes3[mi] || { name: `MeshGroup_${mi}`, materials: [], bone_indices: [] };
              const groupBatches = resolvedBatches.filter((rb) => rb.meshIndex === mi);
              const mgKey = `wmb3_mg_${mi}`;
              const isExpanded = expandedMeshGroups[mgKey] ?? true;
              const isDropTarget = dragOverMeshIdx === mi && draggedBatchInfo?.isWmb3;

              return (
                <div
                  key={mi}
                  className={`mesh-tree-card ${isDropTarget ? "drag-over-target" : ""}`}
                  onDragOver={(e) => {
                    if (draggedBatchInfo?.isWmb3) {
                      e.preventDefault();
                      e.dataTransfer.dropEffect = "move";
                      if (dragOverMeshIdx !== mi) setDragOverMeshIdx(mi);
                    }
                  }}
                  onDragLeave={() => {
                    if (dragOverMeshIdx === mi) setDragOverMeshIdx(null);
                  }}
                  onDrop={(e) => {
                    e.preventDefault();
                    setDragOverMeshIdx(null);
                    if (draggedBatchInfo?.isWmb3 && draggedBatchInfo.fromMeshIdx !== mi) {
                      runWmbMutation(wmb.node_id, "update_wmb3_batch", {
                        batch_index: draggedBatchInfo.batchIdx,
                        mesh_index: mi,
                      });
                    }
                    setDraggedBatchInfo(null);
                  }}
                >
                  <div
                    className="mesh-tree-header"
                    onClick={() =>
                      setExpandedMeshGroups((p) => ({ ...p, [mgKey]: !isExpanded }))
                    }
                  >
                    <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
                      <span className={`tree-arrow ${isExpanded ? "expanded" : ""}`}>
                        <svg width="6" height="9" viewBox="0 0 6 10" fill="none" stroke="currentColor" strokeWidth="1.5">
                          <path d="M1 1L5 5L1 9" strokeLinecap="round" strokeLinejoin="round" />
                        </svg>
                      </span>
                      <span className="tree-badge">MESH #{mi}</span>
                      <span style={{ fontWeight: 600, color: "var(--brush-heading)" }}>
                        {mg.name || `MeshGroup_${mi}`}
                      </span>
                      <span className="mono" style={{ color: "var(--brush-muted)", fontSize: 11.5 }}>
                        ({groupBatches.length} batches · mats: [{(mg.materials || []).join(", ")}])
                      </span>
                    </div>
                    <span className="mono" style={{ color: "var(--brush-muted)", fontSize: 11 }}>
                      {Array.isArray(mg.bone_indices) ? `${mg.bone_indices.length} bones` : ""}
                    </span>
                  </div>

                  {isExpanded && (
                    <div>
                      {groupBatches.length === 0 ? (
                        <div style={{ padding: "10px 24px", color: "var(--brush-muted)", fontSize: 12 }}>
                          (No batches assigned to Mesh Group #{mi} — drag a batch here to assign)
                        </div>
                      ) : (
                        groupBatches.map((rb) => {
                          const lodKey = `wmb3_b_${rb.batchIdx}`;
                          const hdrKey = `wmb3_hdr_${rb.batchIdx}`;
                          const lodExpanded = expandedBatchLods[lodKey] ?? false;
                          const hdrExpanded = expandedBatchLods[hdrKey] ?? false;
                          const hasShadowLod = rb.lodRows.some(
                            (lr) => lr.lodLevel === -1 || String(lr.lodName).toUpperCase().includes("SHADOW")
                          );
                          return (
                            <div key={rb.batchIdx}>
                              <div
                                className={`mesh-batch-row ${
                                  draggedBatchInfo?.batchIdx === rb.batchIdx ? "dragging-batch" : ""
                                }`}
                                draggable
                                onDragStart={(e) => {
                                  e.dataTransfer.effectAllowed = "move";
                                  e.dataTransfer.setData("text/plain", String(rb.batchIdx));
                                  setDraggedBatchInfo({
                                    isWmb3: true,
                                    batchIdx: rb.batchIdx,
                                    fromMeshIdx: mi,
                                  });
                                }}
                                onDragEnd={() => {
                                  setDraggedBatchInfo(null);
                                  setDragOverMeshIdx(null);
                                }}
                              >
                                <div style={{ display: "flex", alignItems: "center", gap: 8, flexWrap: "wrap" }}>
                                  <span
                                    style={{ cursor: "grab", color: "var(--brush-muted)", fontWeight: 700 }}
                                    data-tooltip="Drag to another Mesh Group"
                                  >
                                    ⋮⋮
                                  </span>
                                  <button
                                    type="button"
                                    className="pill-button"
                                    style={{ height: 24, padding: "0 7px", fontSize: 11 }}
                                    onClick={() =>
                                      setExpandedBatchLods((p) => ({ ...p, [lodKey]: !lodExpanded }))
                                    }
                                    data-tooltip="Toggle LOD sub-rows"
                                  >
                                    <span>{lodExpanded ? "▾" : "▸"}</span>
                                    <span className="mono">Batch #{rb.batchIdx}</span>
                                    <span style={{ color: "var(--brush-muted)" }}>
                                      ({rb.lodRows.length} LOD{rb.lodRows.length === 1 ? "" : "s"})
                                    </span>
                                  </button>

                                  {hasShadowLod && (
                                    <span
                                      className="tree-badge"
                                      style={{ background: "rgba(180, 130, 255, 0.16)", color: "#c49bff" }}
                                      data-tooltip="Includes -1 SHADOW LOD in WMB3"
                                    >
                                      SHADOW LOD
                                    </span>
                                  )}

                                  <MaterialPickerPopover
                                    value={rb.materialIndex}
                                    options={matOptions}
                                    onChange={(nextMat) =>
                                      runWmbMutation(wmb.node_id, "update_wmb3_batch", {
                                        batch_index: rb.batchIdx,
                                        material_index: nextMat,
                                      })
                                    }
                                    onNavigateMaterial={(matNodeId) => selectAndInspectNode(matNodeId)}
                                  />
                                </div>

                                <div style={{ display: "flex", alignItems: "center", gap: 8, flexWrap: "wrap" }}>
                                  <label style={{ display: "inline-flex", alignItems: "center", gap: 4, fontSize: 11.5 }}>
                                    <span style={{ color: "var(--brush-muted)" }}>Group:</span>
                                    <StudioSelect
                                      className="pill-select mono"
                                      style={{ height: 26, fontSize: 11.5 }}
                                      value={String(rb.meshIndex)}
                                      options={Array.from({ length: meshGroupCount }).map((__, mgi) => ({
                                        value: String(mgi),
                                        label: `#${mgi} ${meshes3[mgi]?.name || `Group_${mgi}`}`,
                                      }))}
                                      onChange={(val) => {
                                        const nextMg = parseInt(val, 10);
                                        if (!Number.isNaN(nextMg) && nextMg !== rb.meshIndex) {
                                          runWmbMutation(wmb.node_id, "update_wmb3_batch", {
                                            batch_index: rb.batchIdx,
                                            mesh_index: nextMg,
                                          });
                                        }
                                      }}
                                    />
                                  </label>

                                  <label style={{ display: "inline-flex", alignItems: "center", gap: 4, fontSize: 11.5 }}>
                                    <span style={{ color: "var(--brush-muted)" }}>VtxGrp:</span>
                                    <input
                                      type="number"
                                      className="field-box mono"
                                      style={{ width: 54, height: 24 }}
                                      defaultValue={rb.batch.vertex_group_index ?? 0}
                                      onBlur={(e) => {
                                        const vgi = parseInt(e.target.value, 10);
                                        if (!Number.isNaN(vgi) && vgi !== rb.batch.vertex_group_index) {
                                          runWmbMutation(wmb.node_id, "update_wmb3_batch", {
                                            batch_index: rb.batchIdx,
                                            vertex_group_index: vgi,
                                          });
                                        }
                                      }}
                                    />
                                  </label>

                                  <label style={{ display: "inline-flex", alignItems: "center", gap: 4, fontSize: 11.5 }}>
                                    <span style={{ color: "var(--brush-muted)" }}>BoneSet:</span>
                                    <input
                                      type="number"
                                      className="field-box mono"
                                      style={{ width: 54, height: 24 }}
                                      defaultValue={rb.batch.bone_set_index ?? -1}
                                      onBlur={(e) => {
                                        const bsi = parseInt(e.target.value, 10);
                                        if (!Number.isNaN(bsi) && bsi !== rb.batch.bone_set_index) {
                                          runWmbMutation(wmb.node_id, "update_wmb3_batch", {
                                            batch_index: rb.batchIdx,
                                            bone_set_index: bsi,
                                          });
                                        }
                                      }}
                                    />
                                  </label>

                                  <button
                                    type="button"
                                    className={`pill-button ${hdrExpanded ? "accent" : ""}`}
                                    style={{ height: 24, padding: "0 8px", fontSize: 11 }}
                                    onClick={() =>
                                      setExpandedBatchLods((p) => ({ ...p, [hdrKey]: !hdrExpanded }))
                                    }
                                    data-tooltip="Edit all WMB3 Batch Header fields"
                                  >
                                    ⚙ Header
                                  </button>

                                  <span className="mono" style={{ color: "var(--brush-muted)", fontSize: 11 }}>
                                    {rb.batch.num_primitives} tris ({rb.batch.num_indices} idx)
                                  </span>
                                </div>
                              </div>

                              {/* Expandable WMB3 Batch Header Editor */}
                              {hdrExpanded && (
                                <div
                                  className="mesh-lod-subrow"
                                  style={{
                                    display: "grid",
                                    gridTemplateColumns: "repeat(auto-fill, minmax(150px, 1fr))",
                                    gap: 8,
                                    padding: "8px 16px 8px 36px",
                                    background: "var(--brush-panel)",
                                  }}
                                >
                                  {(
                                    [
                                      ["vertex_group_index", "vertex_group_index (u32)"],
                                      ["bone_set_index", "bone_set_index (i32)"],
                                      ["vertex_start", "vertex_start (u32)"],
                                      ["index_start", "index_start (u32)"],
                                      ["num_vertexes", "num_vertexes (u32)"],
                                      ["num_indices", "num_indices (u32)"],
                                      ["num_primitives", "num_primitives (u32)"],
                                    ] as [string, string][]
                                  ).map(([fKey, fLabel]) => (
                                    <label
                                      key={fKey}
                                      style={{ display: "flex", flexDirection: "column", gap: 3, fontSize: 11 }}
                                    >
                                      <span className="mono" style={{ color: "var(--brush-muted)" }}>
                                        {fLabel}
                                      </span>
                                      <input
                                        type="number"
                                        className="field-box mono"
                                        style={{ height: 24 }}
                                        defaultValue={rb.batch[fKey] ?? 0}
                                        onBlur={(e) => {
                                          const val = parseInt(e.target.value, 10);
                                          if (!Number.isNaN(val) && val !== rb.batch[fKey]) {
                                            runWmbMutation(wmb.node_id, "update_wmb3_batch", {
                                              batch_index: rb.batchIdx,
                                              [fKey]: val,
                                            });
                                          }
                                        }}
                                      />
                                    </label>
                                  ))}
                                </div>
                              )}

                              {/* Non-draggable LOD sub-rows under this Batch (with editable LOD BatchInfo fields) */}
                              {lodExpanded &&
                                rb.lodRows.map((lr) => (
                                  <div
                                    key={lr.lodIndex}
                                    className="mesh-lod-subrow"
                                    style={{ display: "flex", alignItems: "center", gap: 10, flexWrap: "wrap" }}
                                  >
                                    <span className="tree-badge">LOD #{lr.lodLevel}</span>
                                    <span className="mono" style={{ fontWeight: 600, color: "var(--brush-text)" }}>
                                      {lr.lodName}
                                    </span>
                                    <label style={{ display: "inline-flex", alignItems: "center", gap: 4, fontSize: 11 }}>
                                      <span className="mono" style={{ color: "var(--brush-muted)" }}>mesh_idx:</span>
                                      <input
                                        type="number"
                                        className="field-box mono"
                                        style={{ width: 52, height: 22 }}
                                        defaultValue={lr.meshIndex}
                                        onBlur={(e) => {
                                          const v = parseInt(e.target.value, 10);
                                          if (!Number.isNaN(v) && v !== lr.meshIndex) {
                                            runWmbMutation(wmb.node_id, "update_wmb3_batch", {
                                              batch_index: rb.batchIdx,
                                              lod_index: lr.lodIndex,
                                              mesh_index: v,
                                            });
                                          }
                                        }}
                                      />
                                    </label>
                                    <label style={{ display: "inline-flex", alignItems: "center", gap: 4, fontSize: 11 }}>
                                      <span className="mono" style={{ color: "var(--brush-muted)" }}>mat_idx:</span>
                                      <input
                                        type="number"
                                        className="field-box mono"
                                        style={{ width: 52, height: 22 }}
                                        defaultValue={lr.materialIndex}
                                        onBlur={(e) => {
                                          const v = parseInt(e.target.value, 10);
                                          if (!Number.isNaN(v) && v !== lr.materialIndex) {
                                            runWmbMutation(wmb.node_id, "update_wmb3_batch", {
                                              batch_index: rb.batchIdx,
                                              lod_index: lr.lodIndex,
                                              material_index: v,
                                            });
                                          }
                                        }}
                                      />
                                    </label>
                                    <label style={{ display: "inline-flex", alignItems: "center", gap: 4, fontSize: 11 }}>
                                      <span className="mono" style={{ color: "var(--brush-muted)" }}>mesh_mat_idx:</span>
                                      <input
                                        type="number"
                                        className="field-box mono"
                                        style={{ width: 52, height: 22 }}
                                        defaultValue={lr.meshMatIndex}
                                        onBlur={(e) => {
                                          const v = parseInt(e.target.value, 10);
                                          if (!Number.isNaN(v) && v !== lr.meshMatIndex) {
                                            runWmbMutation(wmb.node_id, "update_wmb3_batch", {
                                              batch_index: rb.batchIdx,
                                              lod_index: lr.lodIndex,
                                              mesh_material_index: v,
                                            });
                                          }
                                        }}
                                      />
                                    </label>
                                    <label style={{ display: "inline-flex", alignItems: "center", gap: 4, fontSize: 11 }}>
                                      <span className="mono" style={{ color: "var(--brush-muted)" }}>col_tree_idx:</span>
                                      <input
                                        type="number"
                                        className="field-box mono"
                                        style={{ width: 52, height: 22 }}
                                        defaultValue={lr.callTreeNodeIndex}
                                        onBlur={(e) => {
                                          const v = parseInt(e.target.value, 10);
                                          if (!Number.isNaN(v) && v !== lr.callTreeNodeIndex) {
                                            runWmbMutation(wmb.node_id, "update_wmb3_batch", {
                                              batch_index: rb.batchIdx,
                                              lod_index: lr.lodIndex,
                                              call_tree_node_index: v,
                                            });
                                          }
                                        }}
                                      />
                                    </label>
                                    <label style={{ display: "inline-flex", alignItems: "center", gap: 4, fontSize: 11 }}>
                                      <span className="mono" style={{ color: "var(--brush-muted)" }}>u_b:</span>
                                      <input
                                        type="number"
                                        className="field-box mono"
                                        style={{ width: 52, height: 22 }}
                                        defaultValue={lr.uB}
                                        onBlur={(e) => {
                                          const v = parseInt(e.target.value, 10);
                                          if (!Number.isNaN(v) && v !== lr.uB) {
                                            runWmbMutation(wmb.node_id, "update_wmb3_batch", {
                                              batch_index: rb.batchIdx,
                                              lod_index: lr.lodIndex,
                                              u_b: v,
                                            });
                                          }
                                        }}
                                      />
                                    </label>
                                  </div>
                                ))}
                            </div>
                          );
                        })
                      )}
                    </div>
                  )}
                </div>
              );
            })}
          </div>
        );
      }

      // 4B. WMB0 (Bayonetta 1 / Bayonetta 2) Meshes -> Batches Tree
      const isB2BatchMode = wmbB2Override !== null ? wmbB2Override : wmb.is_bayo2;
      const getWmb0EffectiveMatId = (hdr: any): number => {
        if (!hdr) return 0;
        if (isB2BatchMode) {
          return hdr.ex_mat_id !== undefined && hdr.ex_mat_id >= 0
            ? hdr.ex_mat_id
            : hdr.material_id ?? 0;
        }
        return hdr.material_id ?? 0;
      };
      const isWmb0BatchShadowProxy = (hdr: any): boolean => {
        if (!hdr) return false;
        return isB2BatchMode ? hdr.u_e1 === 48 : hdr.u_e1 === 32 && hdr.u_e2 === 15;
      };

      if (wmb.section.includes(":batch:") && wmb.data?.batch) {
        const b = wmb.data.batch;
        const hdr = b.header || {};
        const mIdx = wmb.data.mesh_index ?? 0;
        const bIdx = wmb.data.batch_index ?? 0;
        const curMatId = getWmb0EffectiveMatId(hdr);
        const isShadow = isWmb0BatchShadowProxy(hdr);
        return (
          <div style={{ display: "flex", flexDirection: "column", gap: 14 }}>
            <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", flexWrap: "wrap", gap: 8 }}>
              <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
                <span style={{ fontWeight: 600, color: "var(--brush-heading)" }}>
                  Mesh #{mIdx} ({wmb.data.mesh_name}) → Batch #{bIdx}
                </span>
                {isShadow && (
                  <span
                    className="tree-badge"
                    style={{ background: "rgba(180, 130, 255, 0.18)", color: "#c49bff" }}
                  >
                    SHADOW PROXY
                  </span>
                )}
              </div>
              <div style={{ display: "flex", gap: 6 }}>
                <button
                  type="button"
                  className={`pill-button ${isShadow ? "accent" : ""}`}
                  onClick={() =>
                    runWmbMutation(wmb.node_id, "update_batch", {
                      mesh_index: mIdx,
                      batch_index: bIdx,
                      shadow_proxy: !isShadow,
                    })
                  }
                  data-tooltip={
                    isB2BatchMode
                      ? "Toggle Bayonetta 2 Shadow Proxy (u_e1 = 48 / 0x30)"
                      : "Toggle Bayonetta 1 Shadow Proxy (u_e1 = 32, u_e2 = 15)"
                  }
                >
                  ☁ {isShadow ? "Shadow Proxy: ON" : "Shadow Proxy: OFF"}
                </button>
                <button
                  type="button"
                  className="pill-button"
                  onClick={() =>
                    runWmbMutation(wmb.node_id, "duplicate_wmb_batch", {
                      mesh_index: mIdx,
                      batch_index: bIdx,
                    })
                  }
                >
                  ⧉ Duplicate Batch
                </button>
              </div>
            </div>
            <table className="struct-table">
              <thead>
                <tr>
                  <th>Batch Header Field</th>
                  <th>Value (Editable)</th>
                </tr>
              </thead>
              <tbody>
                <tr>
                  <td className="mono">Assigned Material ({isB2BatchMode ? "Bayo 2 ex_mat_id" : "Bayo 1 material_id"})</td>
                  <td>
                    <MaterialPickerPopover
                      value={curMatId}
                      options={matOptions}
                      onChange={(nextMat) =>
                        runWmbMutation(wmb.node_id, "update_batch", {
                          mesh_index: mIdx,
                          batch_index: bIdx,
                          material_id: nextMat,
                        })
                      }
                      onNavigateMaterial={(matNodeId) => selectAndInspectNode(matNodeId)}
                    />
                  </td>
                </tr>
                <tr>
                  <td className="mono">Shadow Caster Proxy (u_e1 / u_e2)</td>
                  <td>
                    <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
                      <button
                        type="button"
                        className={`pill-button ${isShadow ? "accent" : ""}`}
                        style={{ height: 24, padding: "0 10px", fontSize: 11.5 }}
                        onClick={() =>
                          runWmbMutation(wmb.node_id, "update_batch", {
                            mesh_index: mIdx,
                            batch_index: bIdx,
                            shadow_proxy: !isShadow,
                          })
                        }
                      >
                        {isShadow ? "✓ Enabled (Shadow Proxy)" : "Disabled (Visible Mesh)"}
                      </button>
                      <span className="mono" style={{ fontSize: 11, color: "var(--brush-muted)" }}>
                        u_e1={hdr.u_e1 ?? 0}, u_e2={hdr.u_e2 ?? 0}
                      </span>
                    </div>
                  </td>
                </tr>
                {(
                  [
                    ["batch_id", "batch_id (i16)"],
                    ["mesh_id", "mesh_id (i16)"],
                    ["ex_mat_id", "ex_mat_id (i16 — Bayo 2 Material ID)"],
                    ["raw_material_id", "material_id (u8 — Bayo 1 Material ID)", hdr.material_id ?? 0],
                    ["has_bone_refs", "has_bone_refs (u8)"],
                    ["u_e1", "u_e1 (u8 — Shadow LOD Flag 1)"],
                    ["u_e2", "u_e2 (u8 — Shadow LOD Flag 2)"],
                    ["vertex_start", "vertex_start (u32)"],
                    ["vertex_end", "vertex_end (u32)"],
                    ["primitive_type", "primitive_type (4=Triangle List, 5=Strip)"],
                    ["offset_indices", "offset_indices (u32)"],
                    ["num_indices", "num_indices (i32)"],
                    ["vertex_offset", "vertex_offset (i32)"],
                  ] as [string, string, number?][]
                ).map(([fKey, fLabel, overrideVal]) => {
                  const curVal = overrideVal !== undefined ? overrideVal : hdr[fKey] ?? 0;
                  return (
                    <tr key={`${mIdx}_${bIdx}_${fKey}_${curVal}`}>
                      <td className="mono">{fLabel}</td>
                      <td>
                        <input
                          type="number"
                          size={1}
                          className="field-box mono"
                          style={{ height: 24, width: 120 }}
                          defaultValue={curVal}
                          onBlur={(e) => {
                            const parsed = parseInt(e.target.value, 10);
                            if (!Number.isNaN(parsed) && parsed !== curVal) {
                              runWmbMutation(wmb.node_id, "update_batch", {
                                mesh_index: mIdx,
                                batch_index: bIdx,
                                [fKey]: parsed,
                              });
                            }
                          }}
                        />
                      </td>
                    </tr>
                  );
                })}
                <tr key={`${mIdx}_${bIdx}_flags_${hdr.flags}`}>
                  <td className="mono">flags (u16 hex)</td>
                  <td>
                    <input
                      type="text"
                      size={1}
                      className="field-box mono"
                      style={{ height: 24, width: 120 }}
                      defaultValue={`0x${(hdr.flags ?? 0).toString(16).toUpperCase().padStart(4, "0")}`}
                      onBlur={(e) => {
                        runWmbMutation(wmb.node_id, "update_batch", {
                          mesh_index: mIdx,
                          batch_index: bIdx,
                          flags_hex: e.target.value,
                        });
                      }}
                    />
                  </td>
                </tr>
                <tr>
                  <td className="mono">u_f (7 × i32)</td>
                  <td>
                    <div style={{ display: "flex", gap: 4, flexWrap: "wrap" }}>
                      {(Array.isArray(hdr.u_f) ? hdr.u_f : [0, 0, 0, 0, 0, 0, 0]).map(
                        (uv: number, ui: number) => (
                          <input
                            key={`${ui}_${uv}`}
                            type="number"
                            size={1}
                            className="field-box mono"
                            style={{ width: 64, height: 24 }}
                            defaultValue={uv}
                            onBlur={(e) => {
                              const parsed = parseInt(e.target.value, 10);
                              if (!Number.isNaN(parsed) && parsed !== uv) {
                                const nextUf = [...(hdr.u_f || [0, 0, 0, 0, 0, 0, 0])];
                                nextUf[ui] = parsed;
                                runWmbMutation(wmb.node_id, "update_batch", {
                                  mesh_index: mIdx,
                                  batch_index: bIdx,
                                  u_f: nextUf,
                                });
                              }
                            }}
                          />
                        )
                      )}
                    </div>
                  </td>
                </tr>
                <tr key={`${mIdx}_${bIdx}_bonerefs_${(b.bone_refs || []).join(",")}`}>
                  <td className="mono">Bone Palette Refs ({b.bone_refs?.length ?? 0})</td>
                  <td>
                    <input
                      type="text"
                      size={1}
                      className="field-box mono"
                      style={{ height: 24, width: "100%", minWidth: 220 }}
                      defaultValue={Array.isArray(b.bone_refs) ? b.bone_refs.join(", ") : ""}
                      onBlur={(e) => {
                        const nums = e.target.value
                          .split(",")
                          .map((s) => parseInt(s.trim(), 10))
                          .filter((n) => !Number.isNaN(n) && n >= 0 && n <= 255);
                        runWmbMutation(wmb.node_id, "update_batch", {
                          mesh_index: mIdx,
                          batch_index: bIdx,
                          bone_refs: nums,
                        });
                      }}
                    />
                  </td>
                </tr>
              </tbody>
            </table>
          </div>
        );
      }

      const meshes: any[] = Array.isArray(wmb.data)
        ? wmb.data
        : Array.isArray(wmb.data?.meshes)
        ? wmb.data.meshes
        : wmb.data?.batches
        ? [wmb.data]
        : [];

      return (
        <div style={{ display: "flex", flexDirection: "column", gap: 10 }}>
          <div>
            <div style={{ fontWeight: 600, color: "var(--brush-heading)" }}>
              WMB0 Meshes ({meshes.length}) → Batches Tree ({isB2BatchMode ? "Bayonetta 2" : "Bayonetta 1"})
            </div>
            <div style={{ color: "var(--brush-muted)", fontSize: 11.5, marginBottom: 4 }}>
              Drag any Batch row onto another Mesh to move it, toggle ☁ Shadow Proxy on a Mesh or Batch, or click ⚙ Header to edit batch header fields.
            </div>
          </div>

          {meshes.map((mesh, mi) => {
            const mKey = `wmb0_m_${mi}`;
            const isExpanded = expandedMeshGroups[mKey] ?? true;
            const batches: any[] = Array.isArray(mesh.batches) ? mesh.batches : [];
            const isDropTarget = dragOverMeshIdx === mi && draggedBatchInfo && !draggedBatchInfo.isWmb3;
            const allShadowInMesh =
              batches.length > 0 && batches.every((b) => isWmb0BatchShadowProxy(b.header));
            const someShadowInMesh = batches.some((b) => isWmb0BatchShadowProxy(b.header));

            return (
              <div
                key={mi}
                className={`mesh-tree-card ${isDropTarget ? "drag-over-target" : ""}`}
                onDragOver={(e) => {
                  if (draggedBatchInfo && !draggedBatchInfo.isWmb3) {
                    e.preventDefault();
                    e.dataTransfer.dropEffect = "move";
                    if (dragOverMeshIdx !== mi) setDragOverMeshIdx(mi);
                  }
                }}
                onDragLeave={() => {
                  if (dragOverMeshIdx === mi) setDragOverMeshIdx(null);
                }}
                onDrop={(e) => {
                  e.preventDefault();
                  setDragOverMeshIdx(null);
                  if (
                    draggedBatchInfo &&
                    !draggedBatchInfo.isWmb3 &&
                    draggedBatchInfo.fromMeshIdx !== mi
                  ) {
                    runWmbMutation(wmb.node_id, "move_wmb_batch", {
                      from_mesh_index: draggedBatchInfo.fromMeshIdx,
                      from_batch_index: draggedBatchInfo.batchIdx,
                      to_mesh_index: mi,
                    });
                  }
                  setDraggedBatchInfo(null);
                }}
              >
                <div
                  className="mesh-tree-header"
                  onClick={() =>
                    setExpandedMeshGroups((p) => ({ ...p, [mKey]: !isExpanded }))
                  }
                >
                  <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
                    <span className={`tree-arrow ${isExpanded ? "expanded" : ""}`}>
                      <svg width="6" height="9" viewBox="0 0 6 10" fill="none" stroke="currentColor" strokeWidth="1.5">
                        <path d="M1 1L5 5L1 9" strokeLinecap="round" strokeLinejoin="round" />
                      </svg>
                    </span>
                    <span className="tree-badge">MESH #{mesh.id ?? mesh.header?.id ?? mi}</span>
                    <span style={{ fontWeight: 600, color: "var(--brush-heading)" }}>
                      {mesh.name || `Mesh_${mi}`}
                    </span>
                    <span className="mono" style={{ color: "var(--brush-muted)", fontSize: 11.5 }}>
                      ({batches.length} batches)
                    </span>
                    {someShadowInMesh && (
                      <span
                        className="tree-badge"
                        style={{ background: "rgba(180, 130, 255, 0.18)", color: "#c49bff" }}
                      >
                        {allShadowInMesh ? "SHADOW PROXY MESH" : "PARTIAL SHADOW"}
                      </span>
                    )}
                  </div>
                  <div
                    style={{ display: "flex", alignItems: "center", gap: 6 }}
                    onClick={(e) => e.stopPropagation()}
                  >
                    <button
                      type="button"
                      className={`pill-button ${allShadowInMesh ? "accent" : ""}`}
                      style={{ height: 24, padding: "0 8px", fontSize: 11 }}
                      data-tooltip={
                        isB2BatchMode
                          ? "Toggle Shadow Proxy for all batches in this Mesh (Bayo 2: u_e1=48)"
                          : "Toggle Shadow Proxy for all batches in this Mesh (Bayo 1: u_e1=32, u_e2=15)"
                      }
                      onClick={() =>
                        runWmbMutation(wmb.node_id, "toggle_wmb_mesh_shadow_proxy", {
                          mesh_index: mi,
                          enabled: !allShadowInMesh,
                        })
                      }
                    >
                      ☁ {allShadowInMesh ? "Shadow Mesh: ON" : "Shadow Mesh"}
                    </button>
                  </div>
                </div>

                {isExpanded && (
                  <div>
                    {batches.map((b: any, bi: number) => {
                      const hdr = b.header || {};
                      const curMatId = getWmb0EffectiveMatId(hdr);
                      const isShadow = isWmb0BatchShadowProxy(hdr);
                      const hdrKey = `wmb0_hdr_${mi}_${bi}`;
                      const hdrExpanded = expandedBatchLods[hdrKey] ?? false;

                      return (
                        <div key={bi}>
                          <div
                            className={`mesh-batch-row ${
                              draggedBatchInfo &&
                              !draggedBatchInfo.isWmb3 &&
                              draggedBatchInfo.fromMeshIdx === mi &&
                              draggedBatchInfo.batchIdx === bi
                                ? "dragging-batch"
                                : ""
                            }`}
                            draggable
                            onDragStart={(e) => {
                              e.dataTransfer.effectAllowed = "move";
                              e.dataTransfer.setData("text/plain", `${mi}:${bi}`);
                              setDraggedBatchInfo({
                                isWmb3: false,
                                batchIdx: bi,
                                fromMeshIdx: mi,
                              });
                            }}
                            onDragEnd={() => {
                              setDraggedBatchInfo(null);
                              setDragOverMeshIdx(null);
                            }}
                          >
                            <div style={{ display: "flex", alignItems: "center", gap: 8, flexWrap: "wrap" }}>
                              <span
                                style={{ cursor: "grab", color: "var(--brush-muted)", fontWeight: 700 }}
                                data-tooltip="Drag to move batch to another Mesh"
                              >
                                ⋮⋮
                              </span>
                              <span className="mono" style={{ fontWeight: 600 }}>
                                Batch #{bi}
                              </span>

                              {isShadow && (
                                <span
                                  className="tree-badge"
                                  style={{ background: "rgba(180, 130, 255, 0.18)", color: "#c49bff" }}
                                  data-tooltip={`Shadow Proxy (u_e1=${hdr.u_e1}, u_e2=${hdr.u_e2})`}
                                >
                                  SHADOW
                                </span>
                              )}

                              <MaterialPickerPopover
                                value={curMatId}
                                options={matOptions}
                                onChange={(nextMat) =>
                                  runWmbMutation(wmb.node_id, "update_batch", {
                                    mesh_index: mi,
                                    batch_index: bi,
                                    material_id: nextMat,
                                  })
                                }
                                onNavigateMaterial={(matNodeId) => selectAndInspectNode(matNodeId)}
                              />
                            </div>

                            <div style={{ display: "flex", alignItems: "center", gap: 6, flexWrap: "wrap" }}>
                              {meshes.length > 1 && (
                                <label style={{ display: "inline-flex", alignItems: "center", gap: 4, fontSize: 11.5 }}>
                                  <span style={{ color: "var(--brush-muted)" }}>Mesh:</span>
                                  <StudioSelect
                                    className="pill-select mono"
                                    style={{ height: 26, fontSize: 11.5 }}
                                    value={String(mi)}
                                    options={meshes.map((mOpt, mOptIdx) => ({
                                      value: String(mOptIdx),
                                      label: `#${mOptIdx} ${mOpt.name || `Mesh_${mOptIdx}`}`,
                                    }))}
                                    onChange={(val) => {
                                      const dstMesh = parseInt(val, 10);
                                      if (!Number.isNaN(dstMesh) && dstMesh !== mi) {
                                        runWmbMutation(wmb.node_id, "move_wmb_batch", {
                                          from_mesh_index: mi,
                                          from_batch_index: bi,
                                          to_mesh_index: dstMesh,
                                        });
                                      }
                                    }}
                                  />
                                </label>
                              )}

                              <button
                                type="button"
                                className={`pill-button ${isShadow ? "accent" : ""}`}
                                style={{ height: 24, padding: "0 7px", fontSize: 11 }}
                                data-tooltip={
                                  isB2BatchMode
                                    ? `Shadow Proxy toggle (Bayo 2: u_e1=48, currently u_e1=${hdr.u_e1 ?? 0}, u_e2=${hdr.u_e2 ?? 0})`
                                    : `Shadow Proxy toggle (Bayo 1: u_e1=32, u_e2=15, currently u_e1=${hdr.u_e1 ?? 0}, u_e2=${hdr.u_e2 ?? 0})`
                                }
                                onClick={() =>
                                  runWmbMutation(wmb.node_id, "update_batch", {
                                    mesh_index: mi,
                                    batch_index: bi,
                                    shadow_proxy: !isShadow,
                                  })
                                }
                              >
                                ☁ Shadow
                              </button>

                              <button
                                type="button"
                                className={`pill-button ${hdrExpanded ? "accent" : ""}`}
                                style={{ height: 24, padding: "0 7px", fontSize: 11 }}
                                data-tooltip="Edit WMB0 Batch Header fields (batch_id, flags, u_e1, u_e2, vertex range, bone_refs...)"
                                onClick={() =>
                                  setExpandedBatchLods((p) => ({ ...p, [hdrKey]: !hdrExpanded }))
                                }
                              >
                                ⚙ Header
                              </button>

                              <span className="mono" style={{ fontSize: 11, color: "var(--brush-muted)" }}>
                                vtx {hdr.vertex_start}..{hdr.vertex_end} ·{" "}
                                {b.indices?.length ?? hdr.num_indices ?? 0} idx ·{" "}
                                {hdr.primitive_type === 4 ? "Tris" : "Strip"}
                              </span>

                              <button
                                type="button"
                                className="pill-button"
                                style={{ height: 24, padding: "0 7px", fontSize: 11 }}
                                data-tooltip="Duplicate Batch"
                                onClick={() =>
                                  runWmbMutation(wmb.node_id, "duplicate_wmb_batch", {
                                    mesh_index: mi,
                                    batch_index: bi,
                                  })
                                }
                              >
                                ⧉
                              </button>
                              <button
                                type="button"
                                className="pill-button"
                                style={{ height: 24, padding: "0 7px", fontSize: 11 }}
                                disabled={batches.length <= 1}
                                data-tooltip="Delete Batch"
                                onClick={() =>
                                  runWmbMutation(wmb.node_id, "delete_wmb_batch", {
                                    mesh_index: mi,
                                    batch_index: bi,
                                  })
                                }
                              >
                                ✕
                              </button>
                            </div>
                          </div>

                          {/* Expandable WMB0 Batch Header Editor */}
                          {hdrExpanded && (
                            <div
                              className="mesh-lod-subrow"
                              style={{
                                display: "grid",
                                gridTemplateColumns: "repeat(auto-fill, minmax(145px, 1fr))",
                                gap: 8,
                                padding: "8px 16px 8px 36px",
                                background: "var(--brush-panel)",
                              }}
                            >
                              {(
                                [
                                  ["batch_id", "batch_id (i16)", hdr.batch_id ?? bi],
                                  ["mesh_id", "mesh_id (i16)", hdr.mesh_id ?? mi],
                                  ["ex_mat_id", "ex_mat_id (i16 B2)", hdr.ex_mat_id ?? 0],
                                  ["raw_material_id", "material_id (u8 B1)", hdr.material_id ?? 0],
                                  ["has_bone_refs", "has_bone_refs (u8)", hdr.has_bone_refs ?? 1],
                                  ["u_e1", "u_e1 (u8 Shadow 1)", hdr.u_e1 ?? 0],
                                  ["u_e2", "u_e2 (u8 Shadow 2)", hdr.u_e2 ?? 0],
                                  ["vertex_start", "vertex_start (u32)", hdr.vertex_start ?? 0],
                                  ["vertex_end", "vertex_end (u32)", hdr.vertex_end ?? 0],
                                  ["primitive_type", "primitive_type (4/5)", hdr.primitive_type ?? 4],
                                  ["offset_indices", "offset_indices (u32)", hdr.offset_indices ?? 0],
                                  ["num_indices", "num_indices (i32)", hdr.num_indices ?? 0],
                                  ["vertex_offset", "vertex_offset (i32)", hdr.vertex_offset ?? 0],
                                ] as [string, string, number][]
                              ).map(([fKey, fLabel, curVal]) => (
                                <label
                                  key={`${mi}_${bi}_${fKey}_${curVal}`}
                                  style={{ display: "flex", flexDirection: "column", gap: 3, fontSize: 11 }}
                                >
                                  <span className="mono" style={{ color: "var(--brush-muted)" }}>
                                    {fLabel}
                                  </span>
                                  <input
                                    type="number"
                                    size={1}
                                    className="field-box mono"
                                    style={{ height: 24 }}
                                    defaultValue={curVal}
                                    onBlur={(e) => {
                                      const parsed = parseInt(e.target.value, 10);
                                      if (!Number.isNaN(parsed) && parsed !== curVal) {
                                        runWmbMutation(wmb.node_id, "update_batch", {
                                          mesh_index: mi,
                                          batch_index: bi,
                                          [fKey]: parsed,
                                        });
                                      }
                                    }}
                                  />
                                </label>
                              ))}
                              <label
                                key={`${mi}_${bi}_flags_${hdr.flags}`}
                                style={{ display: "flex", flexDirection: "column", gap: 3, fontSize: 11 }}
                              >
                                <span className="mono" style={{ color: "var(--brush-muted)" }}>
                                  flags (u16 hex)
                                </span>
                                <input
                                  type="text"
                                  size={1}
                                  className="field-box mono"
                                  style={{ height: 24 }}
                                  defaultValue={`0x${(hdr.flags ?? 0).toString(16).toUpperCase().padStart(4, "0")}`}
                                  onBlur={(e) => {
                                    runWmbMutation(wmb.node_id, "update_batch", {
                                      mesh_index: mi,
                                      batch_index: bi,
                                      flags_hex: e.target.value,
                                    });
                                  }}
                                />
                              </label>
                            </div>
                          )}
                        </div>
                      );
                    })}
                  </div>
                )}
              </div>
            );
          })}
        </div>
      );
    }

    // 5. Vertexes Table
    if (wmb.section === "vertexes" && Array.isArray(wmb.data)) {
      return (
        <table className="struct-table">
          <thead>
            <tr>
              <th>#</th>
              <th>Position (X, Y, Z)</th>
              <th>UV (U, V)</th>
              <th>Normal (Nx, Ny, Nz)</th>
            </tr>
          </thead>
          <tbody>
            {(wmb.data as any[]).map((v, vi) => (
              <tr key={vi}>
                <td className="mono">{vi}</td>
                <td className="mono">
                  {v.position
                    ? `${v.position.x.toFixed(3)}, ${v.position.y.toFixed(3)}, ${v.position.z.toFixed(3)}`
                    : "—"}
                </td>
                <td className="mono">
                  {v.mapping ? `${v.mapping[0].toFixed(4)}, ${v.mapping[1].toFixed(4)}` : "—"}
                </td>
                <td className="mono">
                  {v.normal?.normal
                    ? `${v.normal.normal[0].toFixed(3)}, ${v.normal.normal[1].toFixed(3)}, ${v.normal.normal[2].toFixed(3)}`
                    : "—"}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      );
    }

    // 6. WMB / WMB3 Header View
    const hdr = wmb.data || {};
    return (
      <div style={{ display: "flex", flexDirection: "column", gap: 16 }}>
        <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center" }}>
          <span style={{ fontWeight: 600, color: "var(--brush-heading)" }}>
            Model Header &amp; Bounding Box
          </span>
          {!wmb.is_wmb3 && (
            <button
              className="pill-button accent"
              onClick={() =>
                runWmbMutation(wmb.node_id, "update_header", {
                  vertex_flags_hex: wmbHeaderFlagsDraft,
                  bounding_box: wmbBboxDraft,
                })
              }
            >
              <span>Apply Header Changes</span>
            </button>
          )}
        </div>
        <table className="struct-table">
          <thead>
            <tr>
              <th style={{ width: 220 }}>Header Property</th>
              <th>Value</th>
            </tr>
          </thead>
          <tbody>
            {!wmb.is_wmb3 && (
              <tr>
                <td className="mono" style={{ fontWeight: 600 }}>
                  Vertex Format Flags
                </td>
                <td>
                  <input
                    type="text"
                    className="field-box mono"
                    style={{ width: 160, height: 26 }}
                    value={wmbHeaderFlagsDraft}
                    onChange={(e) => setWmbHeaderFlagsDraft(e.target.value)}
                  />
                </td>
              </tr>
            )}
            <tr>
              <td className="mono" style={{ fontWeight: 600 }}>
                Bounding Box (Min / Max XYZ)
              </td>
              <td>
                <div style={{ display: "grid", gridTemplateColumns: "repeat(3, 1fr)", gap: 6 }}>
                  {wmbBboxDraft.map((bv, bi) => (
                    <input
                      key={bi}
                      type="number"
                      step="0.05"
                      className="field-box mono"
                      style={{ height: 26 }}
                      value={Number((bv ?? 0).toFixed(4))}
                      disabled={wmb.is_wmb3}
                      onChange={(e) => {
                        const v = parseFloat(e.target.value);
                        if (!Number.isNaN(v)) {
                          const next = [...wmbBboxDraft];
                          next[bi] = v;
                          setWmbBboxDraft(next);
                        }
                      }}
                    />
                  ))}
                </div>
              </td>
            </tr>
            {Object.entries(hdr)
              .filter(([k]) => k !== "bounding_box" && k !== "vertex_flags" && k !== "id")
              .map(([k, v]) => (
                <tr key={k}>
                  <td className="mono" style={{ color: "var(--brush-muted)" }}>
                    {k}
                  </td>
                  <td className="mono">
                    {typeof v === "number" && k.startsWith("offset_")
                      ? `0x${v.toString(16).toUpperCase()} (${v.toLocaleString()} B)`
                      : typeof v === "object"
                      ? JSON.stringify(v)
                      : String(v)}
                  </td>
                </tr>
              ))}
          </tbody>
        </table>
      </div>
    );
  }

  return renderWmbStructuredCenter(wmb);
};

export interface WmbInspectorViewProps {
  activeWmb: WmbSectionView;
  openCards: Record<string, boolean>;
  toggleCard: (key: string) => void;
  wmbB2Override: boolean | null;
  wmbMatDraft: DecodedWmbMaterial | null;
  setWmbMatDraft: React.Dispatch<React.SetStateAction<DecodedWmbMaterial | null>>;
  runWmbMutation: (nodeId: string, operation: string, payload?: any) => Promise<void>;
}

export const WmbInspectorView: React.FC<WmbInspectorViewProps> = ({
  activeWmb,
  openCards,
  toggleCard,
  wmbB2Override,
  wmbMatDraft,
  setWmbMatDraft,
  runWmbMutation,
}) => {
  return (
            <div>
              <div className="inspector-header">
                <div className="inspector-icon-box">
                  <IconCube size={20} />
                </div>
                <div>
                  <div className="inspector-title">{activeWmb.title}</div>
                  <div className="inspector-subtitle">{activeWmb.breadcrumb}</div>
                </div>
              </div>

              <div className="inspector-spec">{activeWmb.spec_line}</div>

              <div className="validation-banner">
                <span>✓</span>
                <span>{activeWmb.validation_msg}</span>
              </div>

              <div className="inspector-card">
                <button className="card-header-toggle" onClick={() => toggleCard("wmb_props")}>
                  <span className="card-header-left">
                    <span style={{ color: "var(--brush-muted)" }}>#</span>
                    <span>WMB Model Operations</span>
                  </span>
                  <IconChevronDown collapsed={!openCards.wmb_props} />
                </button>
                {openCards.wmb_props && (
                  <div className="card-body">
                    <div className="inspector-row">
                      <span className="inspector-row-label">Format</span>
                      <span className="mono">
                        {activeWmb.is_wmb3
                          ? "WMB3 (Bayonetta 3 / NieR)"
                          : (wmbB2Override ?? activeWmb.is_bayo2)
                          ? "WMB0 (Bayonetta 2)"
                          : "WMB0 (Bayonetta 1)"}
                      </span>
                    </div>
                    <div className="inspector-row">
                      <span className="inspector-row-label">Endianness</span>
                      <span className="mono">{activeWmb.endian}</span>
                    </div>
                    {wmbMatDraft && (
                      <>
                        <div className="inspector-row">
                          <span className="inspector-row-label">Shader</span>
                          <span className="mono">{wmbMatDraft.shader_name ?? "Custom"}</span>
                        </div>
                        <div className="inspector-row">
                          <span className="inspector-row-label">Material Type</span>
                          <input
                            type="text"
                            className="field-box mono"
                            style={{ height: 25 }}
                            value={`0x${wmbMatDraft.mat_type.toString(16).toUpperCase()}`}
                            onChange={(e) => {
                              const clean = e.target.value.replace(/^0x/i, "");
                              const parsed = parseInt(clean || "0", 16);
                              if (!Number.isNaN(parsed)) {
                                setWmbMatDraft({
                                  ...wmbMatDraft,
                                  mat_type: parsed,
                                });
                              }
                            }}
                          />
                        </div>
                        <div className="inspector-row">
                          <span className="inspector-row-label">Flags (u16)</span>
                          <input
                            type="text"
                            className="field-box mono"
                            style={{ height: 25 }}
                            value={`0x${wmbMatDraft.flag.toString(16).toUpperCase()}`}
                            onChange={(e) => {
                              const clean = e.target.value.replace(/^0x/i, "");
                              const parsed = parseInt(clean || "0", 16);
                              if (!Number.isNaN(parsed)) {
                                setWmbMatDraft({
                                  ...wmbMatDraft,
                                  flag: parsed,
                                });
                              }
                            }}
                          />
                        </div>
                        <button
                          className="pill-button accent"
                          style={{ marginTop: 6 }}
                          onClick={() =>
                            runWmbMutation(activeWmb.node_id, "update_material", wmbMatDraft)
                          }
                        >
                          <span>Apply Material Changes</span>
                        </button>
                      </>
                    )}
                    <div style={{ display: "flex", flexWrap: "wrap", gap: 6, marginTop: 8 }}>
                      <button
                        className="pill-button"
                        onClick={async () => {
                          const dest = await invoke<string | null>("save_file_dialog", {
                            title: "Export WMB Model to Wavefront .OBJ",
                            defaultName: "model.obj",
                          });
                          if (dest) await runWmbMutation(activeWmb.node_id, "export_obj", { path: dest });
                        }}
                      >
                        Export .OBJ...
                      </button>
                      {activeWmb.is_wmb3 ? (
                        <button
                          className="pill-button"
                          onClick={() => runWmbMutation(activeWmb.node_id, "set_tpose")}
                        >
                          Bake T-Pose
                        </button>
                      ) : (
                        <>
                          <button
                            className="pill-button"
                            onClick={() => runWmbMutation(activeWmb.node_id, "cleanup_vertexes")}
                          >
                            Cleanup Verts
                          </button>
                          <button
                            className="pill-button"
                            onClick={() => runWmbMutation(activeWmb.node_id, "cleanup_materials")}
                          >
                            Cleanup Mats
                          </button>
                          <button
                            className="pill-button"
                            onClick={() => runWmbMutation(activeWmb.node_id, "swap_endian")}
                          >
                            Swap Endian
                          </button>
                        </>
                      )}
                    </div>
                  </div>
                )}
              </div>
            </div>
  );
};
