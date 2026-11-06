import React from "react";
import { invoke } from "@tauri-apps/api/core";
import { ContextMenuState, MenuItemSpec, NodeInspectPayload } from "../types";
import { hexToRgbFloat, rgbFloatToHex } from "../utils/color";
import { formatBytes } from "../utils/format";
import { EditableNumberCell } from "../components/EditableNumberCell";
import { IconChevronDown, IconCube, IconImage } from "../components/Icons";

export interface StructCenterViewProps {
  nodePayload: NodeInspectPayload;
  setNodePayload: React.Dispatch<React.SetStateAction<NodeInspectPayload | null>>;
  centerTab: string;
  csvHeadersDraft: string[];
  setCsvHeadersDraft: React.Dispatch<React.SetStateAction<string[]>>;
  csvRowsDraft: string[][];
  setCsvRowsDraft: React.Dispatch<React.SetStateAction<string[][]>>;
  estDraft: any;
  setEstDraft: React.Dispatch<React.SetStateAction<any>>;
  structDraft: any;
  setStructDraft: React.Dispatch<React.SetStateAction<any>>;
  selectedMotTrackIdx: number;
  setSelectedMotTrackIdx: React.Dispatch<React.SetStateAction<number>>;
  pkzBulkCodec: string;
  setPkzBulkCodec: React.Dispatch<React.SetStateAction<string>>;
  runStructMutation: (nodeId: string, structKind: string, updatedData: any) => Promise<void>;
  runClothMutation: (nodeId: string, operation: string, preset?: string, updatedJson?: any) => Promise<void>;
  runContainerMutation: (nodeId: string, action: string, targetPath?: string, payload?: any) => Promise<void>;
  selectAndInspectNode: (nodeId: string) => Promise<void>;
  handlePickOodleDll: () => Promise<void>;
  loadDirectory: (target?: string) => Promise<void>;
  currentDirRef: React.MutableRefObject<string>;
  buildAssetNodeContextMenu: (nodeId: string, kind: string) => MenuItemSpec[];
  setContextMenu: React.Dispatch<React.SetStateAction<ContextMenuState | null>>;
  setHoverSubmenuIdx: React.Dispatch<React.SetStateAction<number | null>>;
  setStatusLeft: (msg: string) => void;
}

export const StructCenterView: React.FC<StructCenterViewProps> = ({
  nodePayload,
  setNodePayload,
  centerTab,
  csvHeadersDraft,
  setCsvHeadersDraft,
  csvRowsDraft,
  setCsvRowsDraft,
  estDraft,
  setEstDraft,
  structDraft,
  setStructDraft,
  selectedMotTrackIdx,
  setSelectedMotTrackIdx,
  pkzBulkCodec,
  setPkzBulkCodec,
  runStructMutation,
  runClothMutation,
  runContainerMutation,
  selectAndInspectNode,
  handlePickOodleDll,
  loadDirectory,
  currentDirRef,
  buildAssetNodeContextMenu,
  setContextMenu,
  setHoverSubmenuIdx,
  setStatusLeft,
}) => {
  function renderRawStructCenter() {
    const kind = nodePayload?.struct_kind;
    if (!structDraft) return null;

    if (kind === "COS" && Array.isArray(structDraft.entries)) {
      return (
        <div style={{ display: "flex", flexDirection: "column", gap: 12 }}>
          <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center" }}>
            <span style={{ fontWeight: 600, color: "var(--brush-heading)" }}>
              Costume Color Palettes ({structDraft.entries.length} entries)
            </span>
            <button
              className="pill-button accent"
              onClick={() => runStructMutation(nodePayload.node_id, "COS", structDraft)}
            >
              <span>Commit COS Changes</span>
            </button>
          </div>
          <table className="struct-table">
            <thead>
              <tr>
                <th style={{ width: 55 }}>#</th>
                <th style={{ width: 180 }}>Costume Name</th>
                <th>7 RGB Swatches (Click to Edit)</th>
              </tr>
            </thead>
            <tbody>
              {structDraft.entries.map((e: any, ei: number) => (
                <tr key={ei}>
                  <td className="mono">#{ei}</td>
                  <td>
                    <input
                      type="text"
                      className="field-box mono"
                      style={{ width: "100%", height: 25 }}
                      value={e.name}
                      onChange={(ev) => {
                        const copy = structuredClone(structDraft);
                        copy.entries[ei].name = ev.target.value;
                        setStructDraft(copy);
                      }}
                    />
                  </td>
                  <td>
                    <div style={{ display: "flex", gap: 8, flexWrap: "wrap" }}>
                      {(e.colors as number[][]).map((rgb, ci) => (
                        <label
                          key={ci}
                          style={{ display: "flex", alignItems: "center", gap: 4, fontSize: 11 }}
                          className="mono"
                        >
                          <input
                            type="color"
                            value={rgbFloatToHex(rgb)}
                            onChange={(ev) => {
                              const [r, g, b] = hexToRgbFloat(ev.target.value);
                              const copy = structuredClone(structDraft);
                              copy.entries[ei].colors[ci] = [r, g, b];
                              setStructDraft(copy);
                            }}
                            style={{
                              width: 24,
                              height: 24,
                              padding: 0,
                              border: "1px solid var(--brush-border)",
                              borderRadius: 4,
                              cursor: "pointer",
                            }}
                          />
                          <span>C{ci}</span>
                        </label>
                      ))}
                    </div>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      );
    }

    if (kind === "ATT" && Array.isArray(structDraft.points)) {
      return (
        <div style={{ display: "flex", flexDirection: "column", gap: 12 }}>
          <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center" }}>
            <span style={{ fontWeight: 600, color: "var(--brush-heading)" }}>
              Bone Attachment Points ({structDraft.points.length})
            </span>
            <div style={{ display: "flex", gap: 8 }}>
              <button
                className="pill-button"
                onClick={() => {
                  const copy = structuredClone(structDraft);
                  copy.points.push({ source_bone: 0, target_bone: 0, option: 0 });
                  setStructDraft(copy);
                }}
              >
                + Add Point
              </button>
              <button
                className="pill-button accent"
                onClick={() => runStructMutation(nodePayload.node_id, "ATT", structDraft)}
              >
                Commit ATT Changes
              </button>
            </div>
          </div>
          <table className="struct-table">
            <thead>
              <tr>
                <th style={{ width: 65 }}>#</th>
                <th>Source Bone</th>
                <th>Target Bone</th>
                <th>Option Flag</th>
                <th style={{ width: 80 }}>Remove</th>
              </tr>
            </thead>
            <tbody>
              {structDraft.points.map((pt: any, pi: number) => (
                <tr key={pi}>
                  <td className="mono">#{pi}</td>
                  <td>
                    <EditableNumberCell
                      style={{ width: 110, height: 25 }}
                      value={pt.source_bone ?? 0}
                      onChangeValue={(v) => {
                        const copy = structuredClone(structDraft);
                        copy.points[pi].source_bone = v;
                        setStructDraft(copy);
                      }}
                    />
                  </td>
                  <td>
                    <EditableNumberCell
                      style={{ width: 110, height: 25 }}
                      value={pt.target_bone ?? 0}
                      onChangeValue={(v) => {
                        const copy = structuredClone(structDraft);
                        copy.points[pi].target_bone = v;
                        setStructDraft(copy);
                      }}
                    />
                  </td>
                  <td>
                    <EditableNumberCell
                      style={{ width: 110, height: 25 }}
                      value={pt.option ?? 0}
                      onChangeValue={(v) => {
                        const copy = structuredClone(structDraft);
                        copy.points[pi].option = v;
                        setStructDraft(copy);
                      }}
                    />
                  </td>
                  <td>
                    <button
                      className="pill-button"
                      style={{ height: 24, padding: "0 8px" }}
                      onClick={() => {
                        const copy = structuredClone(structDraft);
                        copy.points.splice(pi, 1);
                        setStructDraft(copy);
                      }}
                    >
                      ✕
                    </button>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      );
    }

    if (kind === "OBI" && Array.isArray(structDraft.entries)) {
      return (
        <div style={{ display: "flex", flexDirection: "column", gap: 12 }}>
          <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center" }}>
            <span style={{ fontWeight: 600, color: "var(--brush-heading)" }}>
              Object Placement Instances ({structDraft.entries.length})
            </span>
            <button
              className="pill-button accent"
              onClick={() => runStructMutation(nodePayload.node_id, "OBI", structDraft)}
            >
              Commit OBI Changes
            </button>
          </div>
          <table className="struct-table">
            <thead>
              <tr>
                <th>#</th>
                <th>Object ID</th>
                <th>Position (X, Y, Z)</th>
                <th>Rotation (X, Y, Z)</th>
                <th>Scale (X, Y, Z)</th>
                <th>Difficulty</th>
              </tr>
            </thead>
            <tbody>
              {structDraft.entries.map((e: any, ei: number) => (
                <tr key={ei}>
                  <td className="mono">#{ei}</td>
                  <td className="mono" style={{ fontWeight: 600 }}>
                    {e.object_name} (0x{Number(e.object_id).toString(16).toUpperCase()})
                  </td>
                  <td>
                    <div style={{ display: "grid", gridTemplateColumns: "repeat(3, 1fr)", gap: 4 }}>
                      {(["x", "y", "z"] as const).map((axis) => (
                        <EditableNumberCell
                          key={axis}
                          isFloat
                          decimals={3}
                          step="0.1"
                          style={{ height: 24 }}
                          value={e.position?.[axis] ?? 0}
                          onChangeValue={(v) => {
                            const copy = structuredClone(structDraft);
                            copy.entries[ei].position[axis] = v;
                            setStructDraft(copy);
                          }}
                        />
                      ))}
                    </div>
                  </td>
                  <td>
                    <div style={{ display: "grid", gridTemplateColumns: "repeat(3, 1fr)", gap: 4 }}>
                      {(["x", "y", "z"] as const).map((axis) => (
                        <EditableNumberCell
                          key={axis}
                          isFloat
                          decimals={3}
                          step="0.05"
                          style={{ height: 24 }}
                          value={e.orientation?.[axis] ?? 0}
                          onChangeValue={(v) => {
                            const copy = structuredClone(structDraft);
                            copy.entries[ei].orientation[axis] = v;
                            setStructDraft(copy);
                          }}
                        />
                      ))}
                    </div>
                  </td>
                  <td>
                    <div style={{ display: "grid", gridTemplateColumns: "repeat(3, 1fr)", gap: 4 }}>
                      {(["x", "y", "z"] as const).map((axis) => (
                        <EditableNumberCell
                          key={axis}
                          isFloat
                          decimals={3}
                          step="0.05"
                          style={{ height: 24 }}
                          value={e.scale?.[axis] ?? 1}
                          onChangeValue={(v) => {
                            const copy = structuredClone(structDraft);
                            copy.entries[ei].scale[axis] = v;
                            setStructDraft(copy);
                          }}
                        />
                      ))}
                    </div>
                  </td>
                  <td>
                    <input
                      type="number"
                      className="field-box mono"
                      style={{ width: 85, height: 24 }}
                      value={e.difficulty_selector}
                      onChange={(ev) => {
                        const copy = structuredClone(structDraft);
                        copy.entries[ei].difficulty_selector = parseInt(ev.target.value || "0", 10);
                        setStructDraft(copy);
                      }}
                    />
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      );
    }

    if (kind === "MSQ" && Array.isArray(structDraft.events)) {
      return (
        <div style={{ display: "flex", flexDirection: "column", gap: 12 }}>
          <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center" }}>
            <span style={{ fontWeight: 600, color: "var(--brush-heading)" }}>
              Subtitle / Sequence Events ({structDraft.events.length})
            </span>
            <button
              className="pill-button accent"
              onClick={() => runStructMutation(nodePayload.node_id, "MSQ", structDraft)}
            >
              Commit MSQ Changes
            </button>
          </div>
          <table className="struct-table">
            <thead>
              <tr>
                <th>#</th>
                <th>Event Name</th>
                <th>ID</th>
                <th>Event Type</th>
                <th>Line / Group</th>
                <th>Duration (Frames)</th>
              </tr>
            </thead>
            <tbody>
              {structDraft.events.map((ev: any, ei: number) => (
                <tr key={ei}>
                  <td className="mono">#{ei}</td>
                  <td>
                    <input
                      type="text"
                      className="field-box mono"
                      style={{ width: "100%", height: 25 }}
                      value={ev.name}
                      onChange={(e) => {
                        const copy = structuredClone(structDraft);
                        copy.events[ei].name = e.target.value;
                        setStructDraft(copy);
                      }}
                    />
                  </td>
                  <td>
                    <input
                      type="number"
                      className="field-box mono"
                      style={{ width: 95, height: 25 }}
                      value={ev.id}
                      onChange={(e) => {
                        const copy = structuredClone(structDraft);
                        copy.events[ei].id = parseInt(e.target.value || "0", 10);
                        setStructDraft(copy);
                      }}
                    />
                  </td>
                  <td>
                    <input
                      type="number"
                      className="field-box mono"
                      style={{ width: 95, height: 25 }}
                      value={ev.event_type}
                      onChange={(e) => {
                        const copy = structuredClone(structDraft);
                        copy.events[ei].event_type = parseInt(e.target.value || "0", 10);
                        setStructDraft(copy);
                      }}
                    />
                  </td>
                  <td>
                    <input
                      type="number"
                      className="field-box mono"
                      style={{ width: 95, height: 25 }}
                      value={ev.line_number_per_group}
                      onChange={(e) => {
                        const copy = structuredClone(structDraft);
                        copy.events[ei].line_number_per_group = parseInt(e.target.value || "0", 10);
                        setStructDraft(copy);
                      }}
                    />
                  </td>
                  <td>
                    <input
                      type="number"
                      className="field-box mono"
                      style={{ width: 95, height: 25 }}
                      value={ev.event_duration}
                      onChange={(e) => {
                        const copy = structuredClone(structDraft);
                        copy.events[ei].event_duration = parseInt(e.target.value || "0", 10);
                        setStructDraft(copy);
                      }}
                    />
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      );
    }

    if (kind === "MOT" && Array.isArray(structDraft.records)) {
      const records: any[] = structDraft.records;
      const selTrack = records[selectedMotTrackIdx] ?? null;
      return (
        <div style={{ display: "flex", flexDirection: "column", gap: 14 }}>
          <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center" }}>
            <div style={{ display: "flex", alignItems: "center", gap: 12 }}>
              <span style={{ fontWeight: 600, color: "var(--brush-heading)" }}>
                Motion Header — Frame Count:
              </span>
              <input
                type="number"
                className="field-box mono"
                style={{ width: 95, height: 26 }}
                value={structDraft.frame_count ?? 0}
                onChange={(e) => {
                  const copy = structuredClone(structDraft);
                  copy.frame_count = parseInt(e.target.value || "0", 10);
                  setStructDraft(copy);
                }}
              />
            </div>
            <button
              className="pill-button accent"
              onClick={() => runStructMutation(nodePayload.node_id, "MOT", structDraft)}
            >
              Commit MOT Changes
            </button>
          </div>
          <div style={{ display: "grid", gridTemplateColumns: "1.2fr 1fr", gap: 14 }}>
            <table className="struct-table">
              <thead>
                <tr>
                  <th>#</th>
                  <th>Bone Index</th>
                  <th>Channel</th>
                  <th>Mode</th>
                  <th>Value / Keys</th>
                </tr>
              </thead>
              <tbody>
                {records.map((r, ri) => (
                  <tr
                    key={ri}
                    className={`file-row ${selectedMotTrackIdx === ri ? "selected" : ""}`}
                    onClick={() => setSelectedMotTrackIdx(ri)}
                  >
                    <td className="mono">#{ri}</td>
                    <td>
                      <input
                        type="number"
                        className="field-box mono"
                        style={{ width: 85, height: 23 }}
                        value={r.bone_index}
                        onClick={(e) => e.stopPropagation()}
                        onChange={(e) => {
                          const copy = structuredClone(structDraft);
                          copy.records[ri].bone_index = parseInt(e.target.value || "0", 10);
                          setStructDraft(copy);
                        }}
                      />
                    </td>
                    <td className="mono">{r.value_index}</td>
                    <td className="mono">{r.flag}</td>
                    <td>
                      {r.flag === 0 || !r.interpolation ? (
                        <input
                          type="number"
                          step="0.01"
                          className="field-box mono"
                          style={{ width: 105, height: 23 }}
                          value={Number((r.value ?? 0).toFixed(5))}
                          onClick={(e) => e.stopPropagation()}
                          onChange={(e) => {
                            const copy = structuredClone(structDraft);
                            copy.records[ri].value = parseFloat(e.target.value || "0");
                            setStructDraft(copy);
                          }}
                        />
                      ) : (
                        <span className="mono" style={{ color: "var(--brush-accent)" }}>
                          {r.elem_number} keyframes →
                        </span>
                      )}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>

            <div>
              <div style={{ fontWeight: 600, color: "var(--brush-heading)", marginBottom: 6 }}>
                {selTrack
                  ? `Track #${selectedMotTrackIdx} (Bone ${selTrack.bone_index}, Ch ${selTrack.value_index})`
                  : "Select a track"}
              </div>
              {selTrack?.interpolation ? (
                <table className="struct-table">
                  <thead>
                    <tr>
                      <th>Key #</th>
                      <th>Value / Quantized</th>
                    </tr>
                  </thead>
                  <tbody>
                    {(() => {
                      const interp = selTrack.interpolation;
                      const variantKey = Object.keys(interp)[0];
                      const payload = interp[variantKey];
                      const keys: any[] = payload?.keys ?? (Array.isArray(payload) ? payload : []);
                      return keys.map((k: any, ki: number) => (
                        <tr key={ki}>
                          <td className="mono">
                            {k.index !== undefined ? `Frame ${k.index}` : `#${ki}`}
                          </td>
                          <td className="mono">
                            {typeof k === "number"
                              ? k.toFixed(5)
                              : k.p !== undefined
                              ? `p=${Number(k.p).toFixed(4)} (m0=${k.m0}, m1=${k.m1})`
                              : JSON.stringify(k)}
                          </td>
                        </tr>
                      ));
                    })()}
                  </tbody>
                </table>
              ) : (
                <div style={{ color: "var(--brush-muted)", padding: 12 }}>
                  Constant value track ({selTrack?.value ?? 0})
                </div>
              )}
            </div>
          </div>
        </div>
      );
    }

    // COL (Editable Collision Groups)
    if (kind === "COL" && Array.isArray(structDraft.groups)) {
      return (
        <div style={{ display: "flex", flexDirection: "column", gap: 12 }}>
          <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center" }}>
            <span style={{ fontWeight: 600, color: "var(--brush-heading)" }}>
              Collision Groups ({structDraft.groups.length}) · {structDraft.num_vertices} vertices
            </span>
            <button
              className="pill-button accent"
              onClick={() => runStructMutation(nodePayload.node_id, "COL", structDraft)}
            >
              Commit COL Changes
            </button>
          </div>
          <table className="struct-table">
            <thead>
              <tr>
                <th>Group #</th>
                <th>Collision Group Name</th>
                <th>Flags 1 (Hex)</th>
                <th>Flags 2 (Hex)</th>
              </tr>
            </thead>
            <tbody>
              {structDraft.groups.map((g: any, gi: number) => (
                <tr key={gi}>
                  <td className="mono">#{g.index ?? gi}</td>
                  <td>
                    <input
                      type="text"
                      className="field-box mono"
                      style={{ height: 24, fontWeight: 600 }}
                      value={g.name}
                      onChange={(e) => {
                        const copy = structuredClone(structDraft);
                        copy.groups[gi].name = e.target.value;
                        setStructDraft(copy);
                      }}
                    />
                  </td>
                  <td>
                    <input
                      type="text"
                      className="field-box mono"
                      style={{ height: 24 }}
                      value={`0x${Number(g.flags >>> 0).toString(16).toUpperCase().padStart(8, "0")}`}
                      onChange={(e) => {
                        const clean = e.target.value.replace(/^0x/i, "");
                        const parsed = parseInt(clean || "0", 16);
                        if (!Number.isNaN(parsed)) {
                          const copy = structuredClone(structDraft);
                          copy.groups[gi].flags = parsed >>> 0;
                          setStructDraft(copy);
                        }
                      }}
                    />
                  </td>
                  <td>
                    <input
                      type="text"
                      className="field-box mono"
                      style={{ height: 24 }}
                      value={`0x${Number(g.flags2 >>> 0).toString(16).toUpperCase().padStart(8, "0")}`}
                      onChange={(e) => {
                        const clean = e.target.value.replace(/^0x/i, "");
                        const parsed = parseInt(clean || "0", 16);
                        if (!Number.isNaN(parsed)) {
                          const copy = structuredClone(structDraft);
                          copy.groups[gi].flags2 = parsed >>> 0;
                          setStructDraft(copy);
                        }
                      }}
                    />
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      );
    }

    // FLS (Stage Lighting Filters & 16 Fog Banks)
    if (kind === "FLS" && Array.isArray(structDraft.filters)) {
      return (
        <div style={{ display: "flex", flexDirection: "column", gap: 16 }}>
          <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center" }}>
            <span style={{ fontWeight: 600, color: "var(--brush-heading)" }}>
              FLS Stage Filters ({structDraft.filters.length}) &amp; Fog Banks ({structDraft.fogs?.length ?? 0})
            </span>
            <button
              className="pill-button accent"
              onClick={() => runStructMutation(nodePayload.node_id, "FLS", structDraft)}
            >
              Commit FLS Changes
            </button>
          </div>
          <table className="struct-table">
            <thead>
              <tr>
                <th>Filter #</th>
                <th>Position / Direction</th>
                <th>Color 1 (RGB)</th>
                <th>Color 2 (RGB)</th>
              </tr>
            </thead>
            <tbody>
              {structDraft.filters.map((f: any, fi: number) => (
                <tr key={fi}>
                  <td className="mono">#{fi}</td>
                  <td className="mono">
                    {Array.isArray(f.pos) ? f.pos.map((v: number) => Number(v).toFixed(2)).join(", ") : "—"}
                  </td>
                  <td>
                    <div style={{ display: "flex", alignItems: "center", gap: 6 }}>
                      <input
                        type="color"
                        value={rgbFloatToHex(f.color1 || [1, 1, 1])}
                        onChange={(e) => {
                          const [r, g, b] = hexToRgbFloat(e.target.value);
                          const copy = structuredClone(structDraft);
                          copy.filters[fi].color1 = [r, g, b, f.color1?.[3] ?? 1];
                          setStructDraft(copy);
                        }}
                        style={{ width: 24, height: 24, padding: 0, borderRadius: 4, cursor: "pointer" }}
                      />
                      <span className="mono" style={{ fontSize: 11 }}>
                        {(f.color1 || []).slice(0, 3).map((v: number) => Number(v).toFixed(2)).join(", ")}
                      </span>
                    </div>
                  </td>
                  <td>
                    <div style={{ display: "flex", alignItems: "center", gap: 6 }}>
                      <input
                        type="color"
                        value={rgbFloatToHex(f.color2 || [1, 1, 1])}
                        onChange={(e) => {
                          const [r, g, b] = hexToRgbFloat(e.target.value);
                          const copy = structuredClone(structDraft);
                          copy.filters[fi].color2 = [r, g, b, f.color2?.[3] ?? 1];
                          setStructDraft(copy);
                        }}
                        style={{ width: 24, height: 24, padding: 0, borderRadius: 4, cursor: "pointer" }}
                      />
                      <span className="mono" style={{ fontSize: 11 }}>
                        {(f.color2 || []).slice(0, 3).map((v: number) => Number(v).toFixed(2)).join(", ")}
                      </span>
                    </div>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>

          {Array.isArray(structDraft.fogs) && structDraft.fogs.length > 0 && (
            <table className="struct-table">
              <thead>
                <tr>
                  <th>Fog #</th>
                  <th>Start Distance</th>
                  <th>End Distance</th>
                  <th>Fog Color (RGB)</th>
                </tr>
              </thead>
              <tbody>
                {structDraft.fogs.map((fg: any, fi: number) => (
                  <tr key={fi}>
                    <td className="mono">#{fi}</td>
                    <td>
                      <input
                        type="number"
                        step="0.5"
                        className="field-box mono"
                        style={{ height: 24 }}
                        value={Number((fg.start ?? 0).toFixed(2))}
                        onChange={(e) => {
                          const copy = structuredClone(structDraft);
                          copy.fogs[fi].start = parseFloat(e.target.value || "0");
                          setStructDraft(copy);
                        }}
                      />
                    </td>
                    <td>
                      <input
                        type="number"
                        step="0.5"
                        className="field-box mono"
                        style={{ height: 24 }}
                        value={Number((fg.end ?? 0).toFixed(2))}
                        onChange={(e) => {
                          const copy = structuredClone(structDraft);
                          copy.fogs[fi].end = parseFloat(e.target.value || "0");
                          setStructDraft(copy);
                        }}
                      />
                    </td>
                    <td>
                      <div style={{ display: "flex", alignItems: "center", gap: 6 }}>
                        <input
                          type="color"
                          value={rgbFloatToHex(fg.color || [1, 1, 1])}
                          onChange={(e) => {
                            const [r, g, b] = hexToRgbFloat(e.target.value);
                            const copy = structuredClone(structDraft);
                            copy.fogs[fi].color = [r, g, b];
                            setStructDraft(copy);
                          }}
                          style={{ width: 24, height: 24, padding: 0, borderRadius: 4, cursor: "pointer" }}
                        />
                        <span className="mono" style={{ fontSize: 11 }}>
                          {(fg.color || []).map((v: number) => Number(v).toFixed(2)).join(", ")}
                        </span>
                      </div>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
        </div>
      );
    }

    // XAD (`.fla` / `.cta` Trigger Areas)
    if (kind === "XAD" && Array.isArray(structDraft.entries)) {
      return (
        <div style={{ display: "flex", flexDirection: "column", gap: 12 }}>
          <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center" }}>
            <span style={{ fontWeight: 600, color: "var(--brush-heading)" }}>
              XAD (.fla / .cta) Trigger Areas ({structDraft.entries.length})
            </span>
            <button
              className="pill-button accent"
              onClick={() => runStructMutation(nodePayload.node_id, "XAD", structDraft)}
            >
              Commit XAD Changes
            </button>
          </div>
          <table className="struct-table">
            <thead>
              <tr>
                <th>#</th>
                <th>Area ID</th>
                <th>Priority</th>
                <th>Target Index</th>
                <th>Center (X, Y, Z)</th>
                <th>Radius / Extent</th>
                <th>Interp Time</th>
              </tr>
            </thead>
            <tbody>
              {structDraft.entries.map((en: any, ei: number) => (
                <tr key={ei}>
                  <td className="mono">#{ei}</td>
                  <td className="mono">{en.id}</td>
                  <td>
                    <input
                      type="number"
                      className="field-box mono"
                      style={{ height: 24 }}
                      value={en.priority ?? 0}
                      onChange={(e) => {
                        const copy = structuredClone(structDraft);
                        copy.entries[ei].priority = parseInt(e.target.value || "0", 10);
                        setStructDraft(copy);
                      }}
                    />
                  </td>
                  <td>
                    <input
                      type="number"
                      className="field-box mono"
                      style={{ height: 24 }}
                      value={en.target_index ?? 0}
                      onChange={(e) => {
                        const copy = structuredClone(structDraft);
                        copy.entries[ei].target_index = parseInt(e.target.value || "0", 10);
                        setStructDraft(copy);
                      }}
                    />
                  </td>
                  <td>
                    <div style={{ display: "grid", gridTemplateColumns: "repeat(3, 1fr)", gap: 4 }}>
                      {[0, 1, 2].map((ai) => (
                        <input
                          key={ai}
                          type="number"
                          step="0.5"
                          className="field-box mono"
                          style={{ height: 24 }}
                          value={Number((en.position?.[ai] ?? 0).toFixed(2))}
                          onChange={(e) => {
                            const copy = structuredClone(structDraft);
                            copy.entries[ei].position[ai] = parseFloat(e.target.value || "0");
                            setStructDraft(copy);
                          }}
                        />
                      ))}
                    </div>
                  </td>
                  <td>
                    <input
                      type="number"
                      step="0.5"
                      className="field-box mono"
                      style={{ height: 24 }}
                      value={Number((en.radius ?? 0).toFixed(2))}
                      onChange={(e) => {
                        const copy = structuredClone(structDraft);
                        copy.entries[ei].radius = parseFloat(e.target.value || "0");
                        setStructDraft(copy);
                      }}
                    />
                  </td>
                  <td>
                    <input
                      type="number"
                      step="0.1"
                      className="field-box mono"
                      style={{ height: 24 }}
                      value={Number((en.interp_time ?? 0).toFixed(2))}
                      onChange={(e) => {
                        const copy = structuredClone(structDraft);
                        copy.entries[ei].interp_time = parseFloat(e.target.value || "0");
                        setStructDraft(copy);
                      }}
                    />
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      );
    }

    // LIT (Stage Lights & Shadow/Sun Lights)
    if (kind === "LIT" && Array.isArray(structDraft.lights)) {
      return (
        <div style={{ display: "flex", flexDirection: "column", gap: 12 }}>
          <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center" }}>
            <span style={{ fontWeight: 600, color: "var(--brush-heading)" }}>
              LIT Stage Lights ({structDraft.lights.length})
            </span>
            <button
              className="pill-button accent"
              onClick={() => runStructMutation(nodePayload.node_id, "LIT", structDraft)}
            >
              Commit LIT Changes
            </button>
          </div>
          <table className="struct-table">
            <thead>
              <tr>
                <th>Light #</th>
                <th>Type</th>
                <th>Position / Direction (X, Y, Z)</th>
                <th>Color (RGBA)</th>
                <th>Intensity</th>
              </tr>
            </thead>
            <tbody>
              {structDraft.lights.map((lt: any, li: number) => (
                <tr key={li}>
                  <td className="mono">#{li}</td>
                  <td className="mono">{lt.light_type}</td>
                  <td>
                    <div style={{ display: "grid", gridTemplateColumns: "repeat(3, 1fr)", gap: 4 }}>
                      {[0, 1, 2].map((ai) => (
                        <input
                          key={ai}
                          type="number"
                          step="0.25"
                          className="field-box mono"
                          style={{ height: 24 }}
                          value={Number((lt.position?.[ai] ?? 0).toFixed(3))}
                          onChange={(e) => {
                            const copy = structuredClone(structDraft);
                            copy.lights[li].position[ai] = parseFloat(e.target.value || "0");
                            setStructDraft(copy);
                          }}
                        />
                      ))}
                    </div>
                  </td>
                  <td>
                    <div style={{ display: "flex", alignItems: "center", gap: 6 }}>
                      <input
                        type="color"
                        value={rgbFloatToHex(lt.color || [1, 1, 1])}
                        onChange={(e) => {
                          const [r, g, b] = hexToRgbFloat(e.target.value);
                          const copy = structuredClone(structDraft);
                          copy.lights[li].color[0] = r;
                          copy.lights[li].color[1] = g;
                          copy.lights[li].color[2] = b;
                          setStructDraft(copy);
                        }}
                        style={{ width: 24, height: 24, padding: 0, borderRadius: 4, cursor: "pointer" }}
                      />
                      <span className="mono" style={{ fontSize: 11 }}>
                        {(lt.color || []).slice(0, 3).map((v: number) => Number(v).toFixed(2)).join(", ")}
                      </span>
                    </div>
                  </td>
                  <td>
                    <input
                      type="number"
                      step="0.05"
                      className="field-box mono"
                      style={{ height: 24 }}
                      value={Number((lt.intensity ?? 1).toFixed(3))}
                      onChange={(e) => {
                        const copy = structuredClone(structDraft);
                        copy.lights[li].intensity = parseFloat(e.target.value || "1");
                        setStructDraft(copy);
                      }}
                    />
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      );
    }

    // MDB (MadWorld `.mdb` Models)
    if (kind === "MDB" && Array.isArray(structDraft.meshes)) {
      return (
        <div style={{ display: "flex", flexDirection: "column", gap: 12 }}>
          <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center" }}>
            <span style={{ fontWeight: 600, color: "var(--brush-heading)" }}>
              MDB Model ({structDraft.num_positions} vertices · {structDraft.num_bones} bones ·{" "}
              {structDraft.meshes.length} meshes)
            </span>
            <button
              className="pill-button accent"
              onClick={() => runStructMutation(nodePayload.node_id, "MDB", structDraft)}
            >
              Commit MDB Changes
            </button>
          </div>
          <table className="struct-table">
            <thead>
              <tr>
                <th>Mesh #</th>
                <th>Material ID</th>
                <th>Batch Count</th>
                <th>Total Triangles</th>
              </tr>
            </thead>
            <tbody>
              {structDraft.meshes.map((m: any, mi: number) => (
                <tr key={mi}>
                  <td className="mono">#{mi}</td>
                  <td>
                    <input
                      type="number"
                      className="field-box mono"
                      style={{ height: 24 }}
                      value={m.material_id ?? 0}
                      onChange={(e) => {
                        const copy = structuredClone(structDraft);
                        copy.meshes[mi].material_id = parseInt(e.target.value || "0", 10);
                        setStructDraft(copy);
                      }}
                    />
                  </td>
                  <td className="mono">{m.num_batches ?? 0}</td>
                  <td className="mono">{m.num_triangles ?? 0}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      );
    }

    // EXP (Bayo1 / Bayo2 Expression Constraints) or generic structured record list
    const expVariantKey = structDraft.Bayo1
      ? "Bayo1"
      : structDraft.Bayo2
      ? "Bayo2"
      : structDraft.V1
      ? "V1"
      : structDraft.V2
      ? "V2"
      : null;
    const expData = expVariantKey ? structDraft[expVariantKey] : structDraft;
    if (expData && Array.isArray(expData.records)) {
      return (
        <div style={{ display: "flex", flexDirection: "column", gap: 12 }}>
          <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center" }}>
            <span style={{ fontWeight: 600, color: "var(--brush-heading)" }}>
              Expression Constraint Tracks ({expData.records.length})
            </span>
            <button
              className="pill-button accent"
              onClick={() => runStructMutation(nodePayload.node_id, "EXP", structDraft)}
            >
              Commit EXP Changes
            </button>
          </div>
          <table className="struct-table">
            <thead>
              <tr>
                <th>#</th>
                <th>Bone Index</th>
                <th>Animation Track</th>
                <th>Flags / Type</th>
                <th>Operations / Points</th>
              </tr>
            </thead>
            <tbody>
              {expData.records.map((r: any, ri: number) => (
                <tr key={ri}>
                  <td className="mono">#{ri}</td>
                  <td>
                    <input
                      type="number"
                      className="field-box mono"
                      style={{ width: 85, height: 24 }}
                      value={r.bone_index ?? 0}
                      onChange={(e) => {
                        const copy = structuredClone(structDraft);
                        const target = expVariantKey ? copy[expVariantKey] : copy;
                        target.records[ri].bone_index = parseInt(e.target.value || "0", 10);
                        setStructDraft(copy);
                      }}
                    />
                  </td>
                  <td>
                    <input
                      type="number"
                      className="field-box mono"
                      style={{ width: 85, height: 24 }}
                      value={r.animation_track ?? 0}
                      onChange={(e) => {
                        const copy = structuredClone(structDraft);
                        const target = expVariantKey ? copy[expVariantKey] : copy;
                        target.records[ri].animation_track = parseInt(e.target.value || "0", 10);
                        setStructDraft(copy);
                      }}
                    />
                  </td>
                  <td className="mono">{r.flags ?? r.entry_type ?? 0}</td>
                  <td className="mono">
                    {Array.isArray(r.operations)
                      ? `${r.operations.length} bytecode ops`
                      : Array.isArray(r.points)
                      ? `${r.points.length} spline points`
                      : "—"}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      );
    }

    return null;
  }


  return (
    <>
      {nodePayload?.view_type === "CSV" && centerTab === "primary" ? (
              <div className="struct-table-container">
                <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", marginBottom: 10 }}>
                  <span style={{ fontWeight: 600, color: "var(--brush-heading)" }}>
                    CSV Table ({csvRowsDraft.length} rows × {csvHeadersDraft.length} columns)
                  </span>
                  <div style={{ display: "flex", gap: 8 }}>
                    <button
                      className="pill-button"
                      onClick={() => {
                        setCsvRowsDraft([
                          ...csvRowsDraft,
                          new Array(Math.max(1, csvHeadersDraft.length)).fill(""),
                        ]);
                      }}
                    >
                      + Add Row
                    </button>
                    <button
                      className="pill-button accent"
                      onClick={() =>
                        runStructMutation(nodePayload.node_id, "CSV", {
                          headers: csvHeadersDraft,
                          rows: csvRowsDraft,
                        })
                      }
                    >
                      Commit CSV Changes
                    </button>
                  </div>
                </div>
                <table className="struct-table">
                  <thead>
                    <tr>
                      <th>#</th>
                      {csvHeadersDraft.map((h, hi) => (
                        <th key={hi}>
                          <input
                            type="text"
                            size={1}
                            className="field-box mono"
                            style={{ width: "100%", height: 24, fontWeight: 600 }}
                            value={h}
                            onChange={(e) => {
                              const next = [...csvHeadersDraft];
                              next[hi] = e.target.value;
                              setCsvHeadersDraft(next);
                            }}
                          />
                        </th>
                      ))}
                      <th style={{ width: 65 }}>Del</th>
                    </tr>
                  </thead>
                  <tbody>
                    {csvRowsDraft.map((row, ri) => (
                      <tr key={ri}>
                        <td className="mono">#{ri + 1}</td>
                        {csvHeadersDraft.map((_, ci) => (
                          <td key={ci}>
                            <input
                              type="text"
                              size={1}
                              className="field-box mono"
                              style={{ width: "100%", height: 24 }}
                              value={row[ci] ?? ""}
                              onChange={(e) => {
                                const next = csvRowsDraft.map((r, idx) => {
                                  if (idx !== ri) return r;
                                  const rowCopy = [...r];
                                  while (rowCopy.length <= ci) rowCopy.push("");
                                  rowCopy[ci] = e.target.value;
                                  return rowCopy;
                                });
                                setCsvRowsDraft(next);
                              }}
                            />
                          </td>
                        ))}
                        <td>
                          <button
                            className="pill-button"
                            style={{ height: 23, padding: "0 8px" }}
                            onClick={() => {
                              const next = [...csvRowsDraft];
                              next.splice(ri, 1);
                              setCsvRowsDraft(next);
                            }}
                          >
                            ✕
                          </button>
                        </td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            ) : nodePayload?.view_type === "EST" && centerTab === "primary" && estDraft ? (
              <div className="struct-table-container">
                <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", marginBottom: 10 }}>
                  <span style={{ fontWeight: 600, color: "var(--brush-heading)" }}>
                    {nodePayload.spec_line}
                  </span>
                  <button
                    className="pill-button accent"
                    onClick={() => runStructMutation(nodePayload.node_id, "EST", estDraft)}
                  >
                    Commit EST Particle Changes
                  </button>
                </div>
                <table className="struct-table">
                  <thead>
                    <tr>
                      <th style={{ width: 55 }}>#</th>
                      <th style={{ width: 85 }}>Tag</th>
                      <th style={{ width: 85 }}>Flag</th>
                      <th style={{ width: 85 }}>Bone</th>
                      <th>Offset (X, Y, Z)</th>
                      <th style={{ width: 95 }}>Scale</th>
                      <th>RGBA Color / Intensity</th>
                    </tr>
                  </thead>
                  <tbody>
                    {(estDraft.records as any[]).map((rec, ri) => (
                      <tr key={ri}>
                        <td className="mono">#{rec.index}</td>
                        <td className="mono" style={{ fontWeight: 600 }}>
                          {rec.group_tag}
                        </td>
                        <td>
                          <input
                            type="number"
                            className="field-box mono"
                            style={{ width: 70, height: 24 }}
                            value={rec.enabled_flag}
                            onChange={(e) => {
                              const copy = structuredClone(estDraft);
                              copy.records[ri].enabled_flag = parseInt(e.target.value || "0", 10);
                              setEstDraft(copy);
                            }}
                          />
                        </td>
                        <td>
                          <input
                            type="number"
                            className="field-box mono"
                            style={{ width: 70, height: 24 }}
                            value={rec.attach_bone}
                            onChange={(e) => {
                              const copy = structuredClone(estDraft);
                              copy.records[ri].attach_bone = parseInt(e.target.value || "0", 10);
                              setEstDraft(copy);
                            }}
                          />
                        </td>
                        <td>
                          <div style={{ display: "grid", gridTemplateColumns: "repeat(3, 1fr)", gap: 4 }}>
                            {(["x", "y", "z"] as const).map((axis) => (
                              <input
                                key={axis}
                                type="number"
                                step="0.05"
                                className="field-box mono"
                                style={{ height: 24 }}
                                value={Number((rec.offset?.[axis] ?? 0).toFixed(3))}
                                onChange={(e) => {
                                  const copy = structuredClone(estDraft);
                                  copy.records[ri].offset[axis] = parseFloat(e.target.value || "0");
                                  setEstDraft(copy);
                                }}
                              />
                            ))}
                          </div>
                        </td>
                        <td>
                          <input
                            type="number"
                            step="0.05"
                            className="field-box mono"
                            style={{ width: 85, height: 24 }}
                            value={Number((rec.scale ?? 1).toFixed(3))}
                            onChange={(e) => {
                              const copy = structuredClone(estDraft);
                              copy.records[ri].scale = parseFloat(e.target.value || "1");
                              setEstDraft(copy);
                            }}
                          />
                        </td>
                        <td>
                          <div style={{ display: "flex", alignItems: "center", gap: 6 }}>
                            <input
                              type="color"
                              value={rgbFloatToHex(rec.color_rgba ?? [1, 1, 1, 1])}
                              onChange={(e) => {
                                const [r, g, b] = hexToRgbFloat(e.target.value);
                                const copy = structuredClone(estDraft);
                                copy.records[ri].color_rgba[0] = r;
                                copy.records[ri].color_rgba[1] = g;
                                copy.records[ri].color_rgba[2] = b;
                                setEstDraft(copy);
                              }}
                              style={{
                                width: 24,
                                height: 24,
                                padding: 0,
                                border: "1px solid var(--brush-border)",
                                borderRadius: 4,
                                cursor: "pointer",
                              }}
                            />
                            <div style={{ display: "grid", gridTemplateColumns: "repeat(4, 1fr)", gap: 4, flex: 1 }}>
                              {[0, 1, 2, 3].map((ci) => (
                                <input
                                  key={ci}
                                  type="number"
                                  step="0.05"
                                  className="field-box mono"
                                  style={{ height: 24 }}
                                  value={Number((rec.color_rgba?.[ci] ?? 1).toFixed(3))}
                                  onChange={(e) => {
                                    const copy = structuredClone(estDraft);
                                    copy.records[ri].color_rgba[ci] = parseFloat(e.target.value || "0");
                                    setEstDraft(copy);
                                  }}
                                />
                              ))}
                            </div>
                          </div>
                        </td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            ) : nodePayload?.view_type === "CLOTH" && centerTab === "primary" ? (
              <div
                className="struct-table-container"
                onContextMenu={(e) => {
                  e.preventDefault();
                  e.stopPropagation();
                  setHoverSubmenuIdx(null);
                  setContextMenu({
                    x: Math.min(e.clientX, window.innerWidth - 250),
                    y: Math.min(e.clientY, window.innerHeight - 240),
                    items: buildAssetNodeContextMenu(nodePayload.node_id, nodePayload.cloth_kind),
                  });
                }}
              >
                <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", marginBottom: 10 }}>
                  <span style={{ fontWeight: 600, color: "var(--brush-heading)" }}>
                    {nodePayload.cloth_kind} Cloth Physics Nodes
                  </span>
                  <button
                    className="pill-button accent"
                    onClick={() =>
                      runClothMutation(
                        nodePayload.node_id,
                        "update_values",
                        undefined,
                        nodePayload.data
                      )
                    }
                  >
                    Commit {nodePayload.cloth_kind} Changes
                  </button>
                </div>
                {nodePayload.cloth_kind === "CLP" && Array.isArray(nodePayload.data?.cloth) ? (
                  <table className="struct-table">
                    <thead>
                      <tr>
                        <th>#</th>
                        <th>no</th>
                        <th>no_up</th>
                        <th>no_down</th>
                        <th>no_side</th>
                        <th>no_poly</th>
                        <th>no_fix</th>
                        <th>rot_limit</th>
                        <th>offset.x</th>
                        <th>offset.y</th>
                        <th>offset.z</th>
                      </tr>
                    </thead>
                    <tbody>
                      {(nodePayload.data.cloth as any[]).map((row, ri) => (
                        <tr key={ri}>
                          <td className="mono">#{ri}</td>
                          {(["no", "no_up", "no_down", "no_side", "no_poly", "no_fix"] as const).map((fk) => (
                            <td key={fk}>
                              <input
                                type="number"
                                size={1}
                                className="field-box mono"
                                style={{ height: 24 }}
                                value={row[fk] ?? 0}
                                onChange={(e) => {
                                  const copy = structuredClone(nodePayload);
                                  copy.data.cloth[ri][fk] = parseInt(e.target.value || "0", 10);
                                  setNodePayload(copy);
                                }}
                              />
                            </td>
                          ))}
                          <td>
                            <input
                              type="number"
                              step="0.01"
                              size={1}
                              className="field-box mono"
                              style={{ height: 24 }}
                              value={Number((row.rot_limit ?? 0).toFixed(4))}
                              onChange={(e) => {
                                const copy = structuredClone(nodePayload);
                                copy.data.cloth[ri].rot_limit = parseFloat(e.target.value || "0");
                                setNodePayload(copy);
                              }}
                            />
                          </td>
                          {(["x", "y", "z"] as const).map((axis) => (
                            <td key={`off_${axis}`}>
                              <input
                                type="number"
                                step="0.001"
                                size={1}
                                className="field-box mono"
                                style={{ height: 24 }}
                                value={Number((row.offset?.[axis] ?? 0).toFixed(4))}
                                onChange={(e) => {
                                  const copy = structuredClone(nodePayload);
                                  if (!copy.data.cloth[ri].offset) {
                                    copy.data.cloth[ri].offset = { x: 0, y: 0, z: 0 };
                                  }
                                  copy.data.cloth[ri].offset[axis] = parseFloat(e.target.value || "0");
                                  setNodePayload(copy);
                                }}
                              />
                            </td>
                          ))}
                        </tr>
                      ))}
                    </tbody>
                  </table>
                ) : nodePayload.cloth_kind === "CLH" &&
                  Array.isArray(nodePayload.data?.entries ?? nodePayload.data?.cloth_wk) ? (
                  <table className="struct-table">
                    <thead>
                      <tr>
                        <th>#</th>
                        <th>Bone 1 (p1)</th>
                        <th>Bone 2 (p2)</th>
                        <th>Weight</th>
                        <th>Radius</th>
                        <th>P1.x</th>
                        <th>P1.y</th>
                        <th>P1.z</th>
                        <th>P2.x</th>
                        <th>P2.y</th>
                        <th>P2.z</th>
                      </tr>
                    </thead>
                    <tbody>
                      {((nodePayload.data.entries ?? nodePayload.data.cloth_wk) as any[]).map(
                        (row, ri) => {
                          const listKey = Array.isArray(nodePayload.data.entries)
                            ? "entries"
                            : "cloth_wk";
                          return (
                            <tr key={ri}>
                              <td className="mono">#{ri}</td>
                              <td>
                                <input
                                  type="number"
                                  size={1}
                                  className="field-box mono"
                                  style={{ height: 24 }}
                                  value={row.p1 ?? row.no1 ?? 0}
                                  onChange={(e) => {
                                    const copy = structuredClone(nodePayload);
                                    const v = parseInt(e.target.value || "0", 10);
                                    copy.data[listKey][ri].p1 = v;
                                    setNodePayload(copy);
                                  }}
                                />
                              </td>
                              <td>
                                <input
                                  type="number"
                                  size={1}
                                  className="field-box mono"
                                  style={{ height: 24 }}
                                  value={row.p2 ?? row.no2 ?? 0}
                                  onChange={(e) => {
                                    const copy = structuredClone(nodePayload);
                                    const v = parseInt(e.target.value || "0", 10);
                                    copy.data[listKey][ri].p2 = v;
                                    setNodePayload(copy);
                                  }}
                                />
                              </td>
                              <td>
                                <input
                                  type="number"
                                  step="0.01"
                                  size={1}
                                  className="field-box mono"
                                  style={{ height: 24 }}
                                  value={Number((row.weight ?? 0).toFixed(4))}
                                  onChange={(e) => {
                                    const copy = structuredClone(nodePayload);
                                    copy.data[listKey][ri].weight = parseFloat(e.target.value || "0");
                                    setNodePayload(copy);
                                  }}
                                />
                              </td>
                              <td>
                                <input
                                  type="number"
                                  step="0.01"
                                  size={1}
                                  className="field-box mono"
                                  style={{ height: 24 }}
                                  value={Number((row.radius ?? row.r ?? 0).toFixed(4))}
                                  onChange={(e) => {
                                    const copy = structuredClone(nodePayload);
                                    copy.data[listKey][ri].radius = parseFloat(e.target.value || "0");
                                    setNodePayload(copy);
                                  }}
                                />
                              </td>
                              {(["x", "y", "z"] as const).map((axis) => (
                                <td key={`o1_${axis}`}>
                                  <input
                                    type="number"
                                    step="0.001"
                                    size={1}
                                    className="field-box mono"
                                    style={{ height: 24 }}
                                    value={Number((row.offset1?.[axis] ?? 0).toFixed(4))}
                                    onChange={(e) => {
                                      const copy = structuredClone(nodePayload);
                                      if (!copy.data[listKey][ri].offset1) {
                                        copy.data[listKey][ri].offset1 = { x: 0, y: 0, z: 0 };
                                      }
                                      copy.data[listKey][ri].offset1[axis] = parseFloat(e.target.value || "0");
                                      setNodePayload(copy);
                                    }}
                                  />
                                </td>
                              ))}
                              {(["x", "y", "z"] as const).map((axis) => (
                                <td key={`o2_${axis}`}>
                                  <input
                                    type="number"
                                    step="0.001"
                                    size={1}
                                    className="field-box mono"
                                    style={{ height: 24 }}
                                    value={Number((row.offset2?.[axis] ?? 0).toFixed(4))}
                                    onChange={(e) => {
                                      const copy = structuredClone(nodePayload);
                                      if (!copy.data[listKey][ri].offset2) {
                                        copy.data[listKey][ri].offset2 = { x: 0, y: 0, z: 0 };
                                      }
                                      copy.data[listKey][ri].offset2[axis] = parseFloat(e.target.value || "0");
                                      setNodePayload(copy);
                                    }}
                                  />
                                </td>
                              ))}
                            </tr>
                          );
                        }
                      )}
                    </tbody>
                  </table>
                ) : Array.isArray(nodePayload.data?.entries ?? nodePayload.data?.cloth_wind_wk) ? (
                  <table className="struct-table">
                    <thead>
                      <tr>
                        <th>#</th>
                        <th>Wind Type</th>
                        <th>Parts No</th>
                        <th>Offset1.x</th>
                        <th>Offset1.y</th>
                        <th>Offset1.z</th>
                        <th>Offset2.x</th>
                        <th>Offset2.y</th>
                        <th>Offset2.z</th>
                        <th>Radius</th>
                        <th>Power</th>
                        <th>Timer</th>
                        <th>Swing Rate</th>
                        <th>Swing Spd</th>
                        <th>Delay Max</th>
                      </tr>
                    </thead>
                    <tbody>
                      {((nodePayload.data.entries ?? nodePayload.data.cloth_wind_wk) as any[]).map(
                        (row, ri) => {
                          const listKey = Array.isArray(nodePayload.data.entries)
                            ? "entries"
                            : "cloth_wind_wk";
                          return (
                            <tr key={ri}>
                              <td className="mono">#{ri}</td>
                              <td>
                                <input
                                  type="number"
                                  size={1}
                                  className="field-box mono"
                                  style={{ height: 24 }}
                                  value={row.wind_type ?? row.ent_type ?? 0}
                                  onChange={(e) => {
                                    const copy = structuredClone(nodePayload);
                                    copy.data[listKey][ri].wind_type = parseInt(e.target.value || "0", 10);
                                    setNodePayload(copy);
                                  }}
                                />
                              </td>
                              <td>
                                <input
                                  type="number"
                                  size={1}
                                  className="field-box mono"
                                  style={{ height: 24 }}
                                  value={row.parts_no ?? row.no ?? 0}
                                  onChange={(e) => {
                                    const copy = structuredClone(nodePayload);
                                    copy.data[listKey][ri].parts_no = parseInt(e.target.value || "0", 10);
                                    setNodePayload(copy);
                                  }}
                                />
                              </td>
                              {(["x", "y", "z"] as const).map((axis) => (
                                <td key={`w1_${axis}`}>
                                  <input
                                    type="number"
                                    step="0.001"
                                    size={1}
                                    className="field-box mono"
                                    style={{ height: 24 }}
                                    value={Number((row.offset?.[axis] ?? 0).toFixed(4))}
                                    onChange={(e) => {
                                      const copy = structuredClone(nodePayload);
                                      if (!copy.data[listKey][ri].offset) {
                                        copy.data[listKey][ri].offset = { x: 0, y: 0, z: 0 };
                                      }
                                      copy.data[listKey][ri].offset[axis] = parseFloat(e.target.value || "0");
                                      setNodePayload(copy);
                                    }}
                                  />
                                </td>
                              ))}
                              {(["x", "y", "z"] as const).map((axis) => (
                                <td key={`w2_${axis}`}>
                                  <input
                                    type="number"
                                    step="0.001"
                                    size={1}
                                    className="field-box mono"
                                    style={{ height: 24 }}
                                    value={Number((row.offset2?.[axis] ?? 0).toFixed(4))}
                                    onChange={(e) => {
                                      const copy = structuredClone(nodePayload);
                                      if (!copy.data[listKey][ri].offset2) {
                                        copy.data[listKey][ri].offset2 = { x: 0, y: 0, z: 0 };
                                      }
                                      copy.data[listKey][ri].offset2[axis] = parseFloat(e.target.value || "0");
                                      setNodePayload(copy);
                                    }}
                                  />
                                </td>
                              ))}
                              {(
                                [
                                  "radius",
                                  "power",
                                  "timer",
                                  "swing_rate",
                                  "swing_spd",
                                  "delay_max",
                                ] as const
                              ).map((fKey) => (
                                <td key={fKey}>
                                  <input
                                    type="number"
                                    step="0.01"
                                    size={1}
                                    className="field-box mono"
                                    style={{ height: 24 }}
                                    value={Number((row[fKey] ?? 0).toFixed(4))}
                                    onChange={(e) => {
                                      const copy = structuredClone(nodePayload);
                                      copy.data[listKey][ri][fKey] = parseFloat(e.target.value || "0");
                                      setNodePayload(copy);
                                    }}
                                  />
                                </td>
                              ))}
                            </tr>
                          );
                        }
                      )}
                    </tbody>
                  </table>
                ) : null}
              </div>
            ) : nodePayload?.view_type === "RAW_STRUCT" && centerTab === "primary" ? (
              <div className="struct-table-container">{renderRawStructCenter()}</div>
            ) : nodePayload?.view_type === "CONTAINER" && centerTab === "primary" ? (
              <div className="struct-table-container">
                {nodePayload.pkz_info?.uses_oodle && nodePayload.pkz_info?.oodle_fallback_active && (
                  <div className="validation-banner" style={{ marginBottom: 12 }}>
                    <span>✓</span>
                    <div style={{ flex: 1 }}>
                      Using built-in pure-Rust Oodle Kraken (<span className="mono">ooz</span>) fallback
                      decompressor. You can also optionally configure external{" "}
                      <span className="mono">oo2core_*_win64.dll</span>.
                    </div>
                    <button className="pill-button" onClick={handlePickOodleDll}>
                      Locate Oodle DLL...
                    </button>
                  </div>
                )}

                <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", flexWrap: "wrap", gap: 10, marginBottom: 10 }}>
                  <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
                    <span style={{ fontWeight: 600, color: "var(--brush-heading)" }}>
                      {nodePayload.title} ({nodePayload.format} · {formatBytes(nodePayload.size_bytes)})
                    </span>
                    {nodePayload.folder_prefix && (
                      <button
                        className="pill-button"
                        style={{ height: 24, padding: "0 8px", fontSize: 11 }}
                        onClick={() => {
                          const rootId = nodePayload.node_id.includes("/pkzfolder:")
                            ? nodePayload.node_id.split("/pkzfolder:")[0]
                            : "root";
                          selectAndInspectNode(rootId);
                        }}
                      >
                        ✕ Clear Folder Filter ({nodePayload.folder_prefix}/)
                      </button>
                    )}
                  </div>
                  <div style={{ display: "flex", alignItems: "center", gap: 8, flexWrap: "wrap" }}>
                    {nodePayload.format === "PKZ" && (
                      <div style={{ display: "inline-flex", alignItems: "center", gap: 6 }}>
                        <span style={{ color: "var(--brush-muted)", fontSize: 11.5 }}>Set All Codec:</span>
                        <select
                          className="pill-select mono"
                          style={{ height: 28, fontSize: 11.5 }}
                          value={pkzBulkCodec}
                          onChange={(e) => setPkzBulkCodec(e.target.value)}
                        >
                          <option value="ZStandard">ZStandard</option>
                          <option value="OodleKraken">OodleKraken</option>
                          <option value="None">None (Uncompressed)</option>
                        </select>
                        <button
                          className="pill-button"
                          style={{ height: 28 }}
                          onClick={() =>
                            runContainerMutation(
                              nodePayload.node_id,
                              "set_pkz_compression",
                              pkzBulkCodec,
                              { compression: pkzBulkCodec }
                            )
                          }
                        >
                          Apply All
                        </button>
                      </div>
                    )}
                    <button
                      className="pill-button"
                      onClick={async () => {
                        const paths = await invoke<string[]>("pick_multiple_files_dialog", {
                          title: "Add File(s) to Container",
                          extensions: null,
                        });
                        if (paths && paths.length > 0) {
                          await runContainerMutation(nodePayload.node_id, "add_files", "", {
                            paths,
                            folder_prefix: nodePayload.folder_prefix ?? null,
                          });
                        }
                      }}
                    >
                      + Add File(s)...
                    </button>
                    <button
                      className="pill-button accent"
                      onClick={async () => {
                        const folder = await invoke<string | null>("pick_folder_dialog", {
                          title: "Extract All Entries to Folder",
                        });
                        if (folder) {
                          await runContainerMutation(nodePayload.node_id, "extract_all", folder);
                          setStatusLeft(`Extracted all entries to ${folder}`);
                          loadDirectory(currentDirRef.current);
                        }
                      }}
                    >
                      Extract All...
                    </button>
                  </div>
                </div>

                {Array.isArray(nodePayload.entries) && nodePayload.entries.length > 0 ? (
                  <table className="struct-table">
                    <thead>
                      <tr>
                        <th style={{ width: 55 }}>#</th>
                        <th>Entry Name</th>
                        <th>Type / Compression Codec</th>
                        <th style={{ textAlign: "right" }}>Size</th>
                        <th style={{ width: 170, textAlign: "right" }}>Actions</th>
                      </tr>
                    </thead>
                    <tbody>
                      {nodePayload.entries
                        .filter((e: any) => {
                          if (!nodePayload.folder_prefix) return true;
                          const norm = String(e.name || "").replace(/\\/g, "/").replace(/^\/+/, "");
                          const fp = String(nodePayload.folder_prefix).replace(/^\/+|\/+$/g, "");
                          return norm === fp || norm.startsWith(`${fp}/`);
                        })
                        .map((e: any, ei: number) => {
                        const baseContainerId = nodePayload.node_id.includes("/pkzfolder:")
                          ? nodePayload.node_id.split("/pkzfolder:")[0]
                          : nodePayload.node_id.startsWith("pkzfolder:")
                          ? "root"
                          : nodePayload.node_id;
                        const prefix =
                          baseContainerId === "root" || !baseContainerId
                            ? ""
                            : `${baseContainerId}/`;
                        const childNodeId =
                          nodePayload.format === "PKZ"
                            ? `${prefix}pkz:${e.index}`
                            : nodePayload.format === "EFF"
                            ? `${prefix}effdir:${e.dir_index}/efffile:${e.index}`
                            : `${prefix}dat:${e.index}`;
                        return (
                          <tr key={ei}>
                            <td className="mono">#{e.index}</td>
                            <td className="mono" style={{ fontWeight: 600 }}>
                              {e.dir_name ? `${e.dir_name} / ${e.name}` : e.name}
                            </td>
                            <td className="mono" style={{ color: "var(--brush-muted)" }}>
                              {nodePayload.format === "PKZ" ? (
                                <select
                                  className="pill-select mono"
                                  style={{ height: 25, fontSize: 11.5 }}
                                  value={
                                    String(e.compression || "").toLowerCase().includes("oodle")
                                      ? "OodleKraken"
                                      : String(e.compression || "").toLowerCase().includes("zst")
                                      ? "ZStandard"
                                      : "None"
                                  }
                                  onChange={(ev) => {
                                    const nextCodec = ev.target.value;
                                    runContainerMutation(
                                      baseContainerId,
                                      "set_pkz_compression",
                                      nextCodec,
                                      {
                                        compression: nextCodec,
                                        entry_indices: [e.index],
                                      }
                                    );
                                  }}
                                >
                                  <option value="ZStandard">ZStandard</option>
                                  <option value="OodleKraken">OodleKraken</option>
                                  <option value="None">None</option>
                                </select>
                              ) : (
                                e.compression || e.extension || "BIN"
                              )}
                            </td>
                            <td className="mono" style={{ textAlign: "right" }}>
                              {formatBytes(e.size ?? 0)}
                              {e.compressed_size !== undefined && e.compressed_size !== e.size
                                ? ` (${formatBytes(e.compressed_size)} comp)`
                                : ""}
                            </td>
                            <td style={{ textAlign: "right" }}>
                              <div style={{ display: "inline-flex", gap: 4 }}>
                                <button
                                  className="pill-button"
                                  style={{ height: 23, padding: "0 8px", fontSize: 11 }}
                                  onClick={() => selectAndInspectNode(childNodeId)}
                                >
                                  Open
                                </button>
                                <button
                                  className="pill-button"
                                  style={{ height: 23, padding: "0 7px", fontSize: 11 }}
                                  onClick={() => runContainerMutation(childNodeId, "remove_entry")}
                                  title="Delete Entry from Container"
                                >
                                  ✕
                                </button>
                              </div>
                            </td>
                          </tr>
                        );
                      })}
                    </tbody>
                  </table>
                ) : (
                  <pre className="mono" style={{ margin: 0, lineHeight: 1.55 }}>
                    {nodePayload.hex_preview}
                  </pre>
                )}
              </div>
            ) : nodePayload ? (
              <div className="struct-table-container">
                {nodePayload.hex_preview ? (
                  <pre className="mono" style={{ margin: 0, lineHeight: 1.55 }}>
                    {nodePayload.hex_preview}
                  </pre>
                ) : (
                  <pre className="mono" style={{ margin: 0, lineHeight: 1.45 }}>
                    {JSON.stringify(nodePayload.data ?? nodePayload, null, 2)}
                  </pre>
                )}
              </div>
            ) : (
              <div className="checkerboard-canvas">
                <div className="viewport-empty">
                  <IconImage size={44} />
                  <div className="viewport-empty-title">No preview</div>
                  <div className="viewport-empty-note">
                    Open a container or select an item from the Asset Browser.
                  </div>
                </div>
              </div>
            )}
    </>
  );
};

export interface ClothInspectorViewProps {
  nodePayload: NodeInspectPayload;
  setNodePayload: React.Dispatch<React.SetStateAction<NodeInspectPayload | null>>;
  openCards: Record<string, boolean>;
  toggleCard: (key: string) => void;
  runClothMutation: (nodeId: string, operation: string, preset?: string, updatedJson?: any) => Promise<void>;
}

export const ClothInspectorView: React.FC<ClothInspectorViewProps> = ({
  nodePayload,
  setNodePayload,
  openCards,
  toggleCard,
  runClothMutation,
}) => {
  return (
            <div>
              <div className="inspector-header">
                <div className="inspector-icon-box">
                  <IconCube size={20} />
                </div>
                <div>
                  <div className="inspector-title">{nodePayload.title}</div>
                  <div className="inspector-subtitle">
                    {nodePayload.cloth_kind} Physics · {nodePayload.endian} Endian
                  </div>
                </div>
              </div>

              <div className="validation-banner">
                <span>✓</span>
                <span>Right-click the cloth node or table to remap bones or swap Wii U ↔ PC endianness.</span>
              </div>

              {nodePayload.cloth_kind === "CLP" && nodePayload.data?.header && (
                <div className="inspector-card">
                  <button className="card-header-toggle" onClick={() => toggleCard("cloth_header")}>
                    <span className="card-header-left">
                      <span style={{ color: "var(--brush-muted)" }}>⚙</span>
                      <span>Spring, Gravity &amp; Wind Header</span>
                    </span>
                    <IconChevronDown collapsed={!openCards.cloth_header} />
                  </button>
                  {openCards.cloth_header && (
                    <div className="card-body">
                      {(
                        [
                          ["limit_spring_rate", "Spring Rate"],
                          ["spd_rate", "Speed Rate"],
                          ["stretchy", "Stretchy"],
                          ["thick", "Thickness"],
                          ["first_bundle_rate", "First Bundle"],
                          ["wind_sin", "Wind Sine"],
                          ["hit_adjust_rate", "Hit Adjust"],
                        ] as [string, string][]
                      ).map(([fieldKey, label]) => (
                        <div key={fieldKey} className="inspector-row">
                          <span className="inspector-row-label">{label}</span>
                          <input
                            type="number"
                            step="0.01"
                            className="field-box mono"
                            value={nodePayload.data.header[fieldKey]}
                            onChange={(e) => {
                              const v = parseFloat(e.target.value);
                              if (!Number.isNaN(v)) {
                                const copy = structuredClone(nodePayload);
                                copy.data.header[fieldKey] = v;
                                setNodePayload(copy);
                              }
                            }}
                          />
                        </div>
                      ))}
                      {(
                        [
                          ["gravity_vec", "Gravity Vec"],
                          ["wind_vec", "Wind Vec"],
                          ["wind_offset", "Wind Offset"],
                        ] as [string, string][]
                      ).map(([vecKey, vecLabel]) => (
                        <div key={vecKey} className="inspector-row">
                          <span className="inspector-row-label">{vecLabel}</span>
                          <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr 1fr", gap: 4 }}>
                            {(["x", "y", "z"] as const).map((axis) => (
                              <input
                                key={axis}
                                type="number"
                                step="0.01"
                                size={1}
                                className="field-box mono"
                                title={`${vecLabel}.${axis}`}
                                value={Number((nodePayload.data.header[vecKey]?.[axis] ?? 0).toFixed(4))}
                                onChange={(e) => {
                                  const v = parseFloat(e.target.value || "0");
                                  if (!Number.isNaN(v)) {
                                    const copy = structuredClone(nodePayload);
                                    if (!copy.data.header[vecKey]) {
                                      copy.data.header[vecKey] = { x: 0, y: 0, z: 0 };
                                    }
                                    copy.data.header[vecKey][axis] = v;
                                    setNodePayload(copy);
                                  }
                                }}
                              />
                            ))}
                          </div>
                        </div>
                      ))}
                      <button
                        className="pill-button accent"
                        style={{ marginTop: 6 }}
                        onClick={() =>
                          runClothMutation(
                            nodePayload.node_id,
                            "update_values",
                            undefined,
                            nodePayload.data
                          )
                        }
                      >
                        <span>Apply Header Parameters</span>
                      </button>
                    </div>
                  )}
                </div>
              )}
            </div>
  );
};
