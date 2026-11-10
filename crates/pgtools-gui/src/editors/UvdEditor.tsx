import React, { useState } from "react";
import {
  PendingUvdSheetInfo,
  SaveEncodeOptions,
  UvdContextView,
  UvdGroupSummaryView,
  UvdItemSummaryView,
} from "../types";
import {
  ALL_TEXTURE_ENCODE_FORMATS,
  ENCODE_QUALITY_OPTIONS,
} from "./TextureEditor";
import {
  IconCheck,
  IconChevronDown,
  IconImage,
} from "../components/Icons";

export interface UvdActions {
  runUvdMutation: (nodeId: string, action: string, payload?: any) => Promise<void>;
  handleUvdExtract: (nodeIds: string[], writeJson: boolean) => Promise<void>;
  promptAddUvdGroup: (uvdNodeId: string) => Promise<void>;
  promptAddUvdSprite: (groupNodeId: string, groupIndex?: number) => Promise<void>;
  promptReplaceUvdSprite: (
    itemNodeId: string,
    groupIndex?: number,
    itemIndex?: number
  ) => Promise<void>;
  promptRenameUvdGroup: (
    groupNodeId: string,
    currentName: string,
    groupIndex?: number
  ) => Promise<void>;
  promptRenameUvdSprite: (
    itemNodeId: string,
    currentName: string,
    groupIndex?: number,
    itemIndex?: number
  ) => Promise<void>;
  selectAndInspectNode: (nodeId: string) => Promise<void>;
  resolveNodeName?: (nodeId: string, fallback: string) => string;
  uvdPadding: number;
  setUvdPadding: (val: number) => void;
  uvdMaxSize: number;
  setUvdMaxSize: (val: number) => void;
}

export interface UvdCenterViewProps {
  uvd: UvdContextView;
  specLine?: string;
  uvdActions: UvdActions;
}

export const UvdCenterView: React.FC<UvdCenterViewProps> = ({
  uvd,
  specLine,
  uvdActions,
}) => {
  return (
    <div
      className="struct-table-container"
      style={{ display: "flex", flexDirection: "column", gap: 14 }}
    >
      <div
        style={{
          display: "flex",
          justifyContent: "space-between",
          alignItems: "center",
          flexWrap: "wrap",
          gap: 10,
        }}
      >
        <div>
          <div style={{ fontWeight: 700, fontSize: 14, color: "var(--brush-heading)" }}>
            UVD Sprite Atlas — {uvd.uvd_name}
          </div>
          <div className="mono" style={{ fontSize: 11.5, color: "var(--brush-muted)", marginTop: 2 }}>
            {specLine ??
              `${uvd.endian} endian · ${uvd.groups.length} group(s) · Default padding ${uvd.uvd_padding}px`}
          </div>
        </div>
        <div style={{ display: "flex", gap: 8, flexWrap: "wrap" }}>
          <button
            type="button"
            className="pill-button accent"
            onClick={() => void uvdActions.promptAddUvdGroup(uvd.uvd_node_id)}
          >
            + Add Group...
          </button>
          <button
            type="button"
            className="pill-button"
            onClick={() => void uvdActions.handleUvdExtract([uvd.uvd_node_id], false)}
          >
            Extract All Groups...
          </button>
          <button
            type="button"
            className="pill-button"
            onClick={() => void uvdActions.handleUvdExtract([uvd.uvd_node_id], true)}
          >
            Extract All + .uvd.json...
          </button>
          <button
            type="button"
            className="pill-button"
            onClick={() => void uvdActions.runUvdMutation(uvd.uvd_node_id, "swap_endian")}
          >
            Swap Endian ({uvd.endian})
          </button>
        </div>
      </div>

      {uvd.groups.length === 0 ? (
        <div
          style={{
            padding: 24,
            borderRadius: 8,
            border: "1px dashed var(--brush-border)",
            color: "var(--brush-muted)",
            textAlign: "center",
          }}
        >
          This UVD file currently has no sprite-sheet groups. Click &ldquo;+ Add Group...&rdquo; to create one.
        </div>
      ) : (
        <table className="struct-table">
          <thead>
            <tr>
              <th style={{ width: 52 }}>#</th>
              <th>Group Name (≤31 chars)</th>
              <th style={{ width: 115 }}>Group ID</th>
              <th style={{ width: 85 }}>Sprites</th>
              <th style={{ width: 105 }}>Dimensions</th>
              <th>Matched WTA Texture</th>
              <th style={{ width: 280 }}>Actions</th>
            </tr>
          </thead>
          <tbody>
            {uvd.groups.map((grp: UvdGroupSummaryView) => (
              <tr
                key={grp.node_id}
                style={{ cursor: "pointer" }}
                onClick={() => void uvdActions.selectAndInspectNode(grp.node_id)}
              >
                <td className="mono">#{grp.index}</td>
                <td className="mono" style={{ fontWeight: 600 }}>
                  {grp.name}
                </td>
                <td className="mono">{grp.id_hex}</td>
                <td className="mono">{grp.item_count}</td>
                <td className="mono">
                  {grp.width} × {grp.height}
                </td>
                <td>
                  {grp.matched_wta_name ? (
                    <span
                      className="mono"
                      style={{
                        fontSize: 11,
                        color: grp.has_pixel_payload
                          ? "var(--brush-text)"
                          : "var(--brush-warn)",
                      }}
                    >
                      {grp.matched_wta_name} #{grp.matched_tex_index}
                      {!grp.has_pixel_payload ? " (header-only / no payload)" : ""}
                    </span>
                  ) : (
                    <span className="mono" style={{ fontSize: 11, color: "var(--brush-warn)" }}>
                      Unmatched (no texture with ID {grp.id_hex})
                    </span>
                  )}
                </td>
                <td onClick={(e) => e.stopPropagation()}>
                  <div style={{ display: "flex", gap: 4, flexWrap: "wrap" }}>
                    <button
                      type="button"
                      className="pill-button"
                      style={{ height: 22, padding: "0 7px", fontSize: 11 }}
                      onClick={() =>
                        void uvdActions.promptAddUvdSprite(grp.node_id, grp.index)
                      }
                    >
                      + Sprite
                    </button>
                    <button
                      type="button"
                      className="pill-button"
                      style={{ height: 22, padding: "0 7px", fontSize: 11 }}
                      onClick={() =>
                        void uvdActions.handleUvdExtract([grp.node_id], false)
                      }
                    >
                      Extract
                    </button>
                    <button
                      type="button"
                      className="pill-button"
                      style={{ height: 22, padding: "0 7px", fontSize: 11 }}
                      onClick={() =>
                        void uvdActions.promptRenameUvdGroup(
                          grp.node_id,
                          grp.name,
                          grp.index
                        )
                      }
                    >
                      Rename
                    </button>
                    <button
                      type="button"
                      className="pill-button"
                      style={{ height: 22, padding: "0 6px", fontSize: 11 }}
                      disabled={grp.index === 0}
                      onClick={() =>
                        void uvdActions.runUvdMutation(grp.node_id, "reorder_groups", {
                          group_indices: [grp.index],
                          direction: "up",
                        })
                      }
                      title="Move group up (UVD-side only)"
                    >
                      ↑
                    </button>
                    <button
                      type="button"
                      className="pill-button"
                      style={{ height: 22, padding: "0 6px", fontSize: 11 }}
                      disabled={grp.index + 1 >= uvd.groups.length}
                      onClick={() =>
                        void uvdActions.runUvdMutation(grp.node_id, "reorder_groups", {
                          group_indices: [grp.index],
                          direction: "down",
                        })
                      }
                      title="Move group down (UVD-side only)"
                    >
                      ↓
                    </button>
                    <button
                      type="button"
                      className="pill-button"
                      style={{ height: 22, padding: "0 7px", fontSize: 11 }}
                      onClick={() => {
                        if (
                          window.confirm(
                            `Delete UVD group '${grp.name}' (${grp.id_hex})?\n\nWTA texture slots will not be deleted or reordered.`
                          )
                        ) {
                          void uvdActions.runUvdMutation(grp.node_id, "delete_groups", {
                            group_indices: [grp.index],
                          });
                        }
                      }}
                    >
                      ✕
                    </button>
                  </div>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </div>
  );
};

export interface UvdInspectorSectionProps {
  uvd: UvdContextView;
  openCards: Record<string, boolean>;
  toggleCard: (key: string) => void;
  uvdActions: UvdActions;
}

export const UvdInspectorSection: React.FC<UvdInspectorSectionProps> = ({
  uvd,
  openCards,
  toggleCard,
  uvdActions,
}) => {
  const isOpen = openCards.uvd_panel !== false;
  const isSettingsOpen = openCards.uvd_settings !== false;
  const grp = uvd.group;
  const item = uvd.item;

  return (
    <>
      {uvd.warning_banner && (
        <div className="validation-banner warn" style={{ marginBottom: 10 }}>
          <span>⚠</span>
          <span>{uvd.warning_banner}</span>
        </div>
      )}

      {/* Mode 1: UVD Sprite Item Selected */}
      {uvd.mode === "item" && item && grp && (
        <div className="inspector-card">
          <button
            className="card-header-toggle"
            onClick={() => toggleCard("uvd_panel")}
          >
            <span className="card-header-left">
              <IconImage size={14} />
              <span>UVD Sprite Item</span>
            </span>
            <IconChevronDown collapsed={!isOpen} />
          </button>
          {isOpen && (
            <div className="card-body">
              <div className="inspector-row">
                <span className="inspector-row-label">Sprite Name</span>
                <span className="mono" style={{ fontWeight: 600, fontSize: 11.5 }}>
                  {item.name}
                </span>
              </div>
              <div className="inspector-row">
                <span className="inspector-row-label">Item ID (CRC32)</span>
                <span className="mono" style={{ fontSize: 11.5 }}>
                  {item.id_hex}
                </span>
              </div>
              <div className="inspector-row">
                <span className="inspector-row-label">Group</span>
                <span
                  className="mono"
                  style={{
                    fontSize: 11.5,
                    cursor: "pointer",
                    color: "var(--brush-accent)",
                  }}
                  onClick={() => void uvdActions.selectAndInspectNode(grp.node_id)}
                  title="Click to inspect parent UVD Group sheet"
                >
                  {grp.name} ({item.group_id_hex})
                </span>
              </div>
              <div className="inspector-row">
                <span className="inspector-row-label">Rect (x, y, w, h)</span>
                <span className="mono" style={{ fontSize: 11.5 }}>
                  ({item.x}, {item.y}) · {item.w} × {item.h} px
                </span>
              </div>
              <div className="inspector-row">
                <span className="inspector-row-label">invW / invH</span>
                <span className="mono" style={{ fontSize: 11 }}>
                  {item.inv_w.toExponential(5)} / {item.inv_h.toExponential(5)} ({item.tex_width}×
                  {item.tex_height})
                </span>
              </div>

              <div style={{ display: "flex", gap: 6, flexWrap: "wrap", marginTop: 8 }}>
                <button
                  type="button"
                  className="pill-button accent"
                  style={{ flex: 1 }}
                  onClick={() =>
                    void uvdActions.promptReplaceUvdSprite(
                      item.node_id,
                      grp.index,
                      item.index
                    )
                  }
                >
                  Replace Sprite...
                </button>
                <button
                  type="button"
                  className="pill-button"
                  onClick={() => void uvdActions.handleUvdExtract([item.node_id], false)}
                >
                  Extract PNG...
                </button>
              </div>

              <div style={{ display: "flex", gap: 6, flexWrap: "wrap", marginTop: 6 }}>
                <button
                  type="button"
                  className="pill-button"
                  onClick={() =>
                    void uvdActions.promptRenameUvdSprite(
                      item.node_id,
                      item.name,
                      grp.index,
                      item.index
                    )
                  }
                >
                  Rename...
                </button>
                <button
                  type="button"
                  className="pill-button"
                  disabled={item.index === 0}
                  onClick={() =>
                    void uvdActions.runUvdMutation(item.node_id, "reorder_items", {
                      group_index: grp.index,
                      item_indices: [item.index],
                      direction: "up",
                    })
                  }
                  title="Move sprite up in group and repack"
                >
                  ↑ Up
                </button>
                <button
                  type="button"
                  className="pill-button"
                  disabled={item.index + 1 >= uvd.items.length}
                  onClick={() =>
                    void uvdActions.runUvdMutation(item.node_id, "reorder_items", {
                      group_index: grp.index,
                      item_indices: [item.index],
                      direction: "down",
                    })
                  }
                  title="Move sprite down in group and repack"
                >
                  ↓ Down
                </button>
                <button
                  type="button"
                  className="pill-button"
                  onClick={() => {
                    if (
                      window.confirm(
                        `Delete sprite '${item.name}' (${item.id_hex}) from group '${grp.name}' and repack?`
                      )
                    ) {
                      void uvdActions.runUvdMutation(item.node_id, "delete_items", {
                        group_index: grp.index,
                        item_indices: [item.index],
                      });
                    }
                  }}
                >
                  Delete
                </button>
              </div>
            </div>
          )}
        </div>
      )}

      {/* Mode 2: UVD Group Selected */}
      {uvd.mode === "group" && grp && (
        <div className="inspector-card">
          <button
            className="card-header-toggle"
            onClick={() => toggleCard("uvd_panel")}
          >
            <span className="card-header-left">
              <IconImage size={14} />
              <span>UVD Group Atlas ({grp.name})</span>
            </span>
            <IconChevronDown collapsed={!isOpen} />
          </button>
          {isOpen && (
            <div className="card-body">
              <div className="inspector-row">
                <span className="inspector-row-label">Group Name</span>
                <span className="mono" style={{ fontWeight: 600, fontSize: 11.5 }}>
                  {grp.name}
                </span>
              </div>
              <div className="inspector-row">
                <span className="inspector-row-label">Group ID</span>
                <span className="mono" style={{ fontSize: 11.5 }}>
                  {grp.id_hex} (crc32 &amp; 0x7FFFFFFF)
                </span>
              </div>
              <div className="inspector-row">
                <span className="inspector-row-label">Matched WTA</span>
                <span className="mono" style={{ fontSize: 11.5 }}>
                  {grp.matched_wta_name
                    ? `${grp.matched_wta_name} #${grp.matched_tex_index}`
                    : "None (Unmatched ID)"}
                </span>
              </div>
              <div className="inspector-row">
                <span className="inspector-row-label">Sprites</span>
                <span className="mono" style={{ fontSize: 11.5 }}>
                  {grp.item_count} item(s) · {grp.width} × {grp.height} px
                </span>
              </div>

              <div style={{ display: "flex", gap: 6, flexWrap: "wrap", marginTop: 8 }}>
                <button
                  type="button"
                  className="pill-button accent"
                  onClick={() =>
                    void uvdActions.promptAddUvdSprite(grp.node_id, grp.index)
                  }
                >
                  + Add Sprite...
                </button>
                <button
                  type="button"
                  className="pill-button"
                  onClick={() =>
                    void uvdActions.handleUvdExtract([grp.node_id], false)
                  }
                >
                  Extract Group...
                </button>
                <button
                  type="button"
                  className="pill-button"
                  onClick={() =>
                    void uvdActions.runUvdMutation(grp.node_id, "repack_group", {
                      group_index: grp.index,
                    })
                  }
                  title="Re-run deterministic power-of-two rectangle packer on this group"
                >
                  Repack
                </button>
              </div>

              <div style={{ display: "flex", gap: 6, flexWrap: "wrap", marginTop: 6 }}>
                <button
                  type="button"
                  className="pill-button"
                  onClick={() =>
                    void uvdActions.promptRenameUvdGroup(
                      grp.node_id,
                      grp.name,
                      grp.index
                    )
                  }
                >
                  Rename Group...
                </button>
                <button
                  type="button"
                  className="pill-button"
                  onClick={() => {
                    if (
                      window.confirm(
                        `Delete UVD group '${grp.name}' (${grp.id_hex})?\n\nWTA texture slots will not be deleted or reordered.`
                      )
                    ) {
                      void uvdActions.runUvdMutation(grp.node_id, "delete_groups", {
                        group_indices: [grp.index],
                      });
                    }
                  }}
                >
                  Delete Group
                </button>
              </div>

              {uvd.items.length > 0 && (
                <div
                  style={{
                    marginTop: 8,
                    maxHeight: 165,
                    overflowY: "auto",
                    border: "1px solid var(--brush-border)",
                    borderRadius: 6,
                    padding: 4,
                  }}
                >
                  {uvd.items.map((it: UvdItemSummaryView) => (
                    <div
                      key={it.node_id}
                      style={{
                        display: "flex",
                        justifyContent: "space-between",
                        alignItems: "center",
                        padding: "3px 6px",
                        borderRadius: 4,
                        fontSize: 11,
                        cursor: "pointer",
                      }}
                      onClick={() => void uvdActions.selectAndInspectNode(it.node_id)}
                      title={`Click to inspect sprite '${it.name}' (${it.id_hex})`}
                    >
                      <span
                        className="mono"
                        style={{
                          overflow: "hidden",
                          textOverflow: "ellipsis",
                          whiteSpace: "nowrap",
                        }}
                      >
                        {it.name}
                      </span>
                      <span className="mono" style={{ color: "var(--brush-muted)" }}>
                        {it.w}×{it.h} @ ({it.x},{it.y})
                      </span>
                    </div>
                  ))}
                </div>
              )}
            </div>
          )}
        </div>
      )}

      {/* Packing Settings Card (Padding & Max Power-of-Two Dimension) */}
      <div className="inspector-card">
        <button
          className="card-header-toggle"
          onClick={() => toggleCard("uvd_settings")}
        >
          <span className="card-header-left">
            <span style={{ color: "var(--brush-muted)" }}>⚙</span>
            <span>UVD Packer Settings</span>
          </span>
          <IconChevronDown collapsed={!isSettingsOpen} />
        </button>
        {isSettingsOpen && (
          <div className="card-body">
            <div className="inspector-row">
              <span className="inspector-row-label">Padding (px)</span>
              <input
                type="number"
                min={0}
                max={64}
                className="field-box mono"
                style={{ width: 90, height: 25 }}
                value={uvdActions.uvdPadding}
                onChange={(e) => {
                  const v = Math.max(0, Math.min(64, parseInt(e.target.value || "0", 10) || 0));
                  uvdActions.setUvdPadding(v);
                }}
              />
            </div>
            <div className="inspector-row">
              <span className="inspector-row-label">Max Sheet Size</span>
              <select
                className="pill-select mono"
                style={{ width: 130, fontSize: 11.5 }}
                value={uvdActions.uvdMaxSize}
                onChange={(e) =>
                  uvdActions.setUvdMaxSize(parseInt(e.target.value || "4096", 10) || 4096)
                }
              >
                {[256, 512, 1024, 2048, 4096, 8192].map((sz) => (
                  <option key={sz} value={sz}>
                    {sz} × {sz}
                  </option>
                ))}
              </select>
            </div>
            <div style={{ color: "var(--brush-muted)", fontSize: 11, marginTop: 2 }}>
              Deterministic power-of-two rectangle packer (rectangular sheets allowed, no rotation, exact IEEE 1/W &amp; 1/H).
            </div>
          </div>
        )}
      </div>
    </>
  );
};

export interface UvdSaveEncodeModalProps {
  pendingSheets: PendingUvdSheetInfo[];
  onCancel: () => void;
  onConfirm: (opts: SaveEncodeOptions) => Promise<void> | void;
}

export const UvdSaveEncodeModal: React.FC<UvdSaveEncodeModalProps> = ({
  pendingSheets,
  onCancel,
  onConfirm,
}) => {
  const hasXt1Flags = pendingSheets.some((p) => p.has_confirmed_xt1_flags);

  const [encodeFormat, setEncodeFormat] = useState<string>("Auto");
  const [quality, setQuality] = useState<string>("Normal");
  const [generateMips, setGenerateMips] = useState<boolean>(false);
  const [solidify, setSolidify] = useState<boolean>(true);
  const [makeOpaque, setMakeOpaque] = useState<boolean>(false);
  const [syncFlags, setSyncFlags] = useState<boolean>(true);

  return (
    <div className="modal-backdrop" onClick={onCancel}>
      <div className="modal-card" onClick={(e) => e.stopPropagation()}>
        <div
          style={{
            display: "flex",
            justifyContent: "space-between",
            alignItems: "center",
          }}
        >
          <div>
            <div
              style={{
                fontSize: 15,
                fontWeight: 600,
                color: "var(--brush-heading)",
              }}
            >
              Encode Modified UVD Sprite Sheets on Save
            </div>
            <div
              className="mono"
              style={{
                color: "var(--brush-muted)",
                fontSize: 11.5,
                marginTop: 2,
              }}
            >
              {pendingSheets.length} repacked UVD group texture(s) ready for save
            </div>
          </div>
          <button
            type="button"
            className="pill-button"
            style={{ height: 26, padding: "0 8px" }}
            onClick={onCancel}
          >
            ✕
          </button>
        </div>

        <div
          className="mono"
          style={{
            padding: "7px 10px",
            borderRadius: 8,
            backgroundColor: "var(--brush-window)",
            border: "1px solid var(--brush-border)",
            fontSize: 11,
            color: "var(--brush-muted)",
            maxHeight: 110,
            overflowY: "auto",
          }}
        >
          {pendingSheets.map((p) => (
            <div key={`${p.wta_container_id}-${p.texture_id}`}>
              • Group <b style={{ color: "var(--brush-text)" }}>{p.group_name}</b> (
              {p.texture_id_hex} in {p.wta_name}) — {p.width}×{p.height} · Default:{" "}
              <span style={{ color: "var(--brush-text)", fontWeight: 600 }}>
                {p.original_format}
              </span>{" "}
              ({p.game_profile})
            </div>
          ))}
        </div>

        <div className="inspector-row">
          <span className="inspector-row-label">Target Format</span>
          <select
            className="pill-select mono"
            style={{ width: "100%", fontSize: 11.5 }}
            value={encodeFormat}
            onChange={(e) => setEncodeFormat(e.target.value)}
          >
            {ALL_TEXTURE_ENCODE_FORMATS.map((grp) => (
              <optgroup key={grp.group} label={grp.group}>
                {grp.options.map((opt) => (
                  <option key={opt.value} value={opt.value}>
                    {opt.label}
                  </option>
                ))}
              </optgroup>
            ))}
          </select>
        </div>

        <div className="inspector-row">
          <span className="inspector-row-label">Quality</span>
          <select
            className="pill-select"
            style={{ width: "100%", fontSize: 11.5 }}
            value={quality}
            onChange={(e) => setQuality(e.target.value)}
          >
            {ENCODE_QUALITY_OPTIONS.map((q) => (
              <option key={q.value} value={q.value}>
                {q.label}
              </option>
            ))}
          </select>
        </div>

        <div className="inspector-row">
          <span className="inspector-row-label">Mipmaps</span>
          <div
            className={`pill-checkbox ${generateMips ? "checked" : ""}`}
            onClick={() => setGenerateMips(!generateMips)}
          >
            <span className="pill-checkbox-box">
              {generateMips && <IconCheck />}
            </span>
            <span>Generate full box-filtered mipmap chain</span>
          </div>
        </div>

        <div className="inspector-row">
          <span className="inspector-row-label">Transparent RGB</span>
          <div
            className={`pill-checkbox ${solidify ? "checked" : ""}`}
            onClick={() => setSolidify(!solidify)}
          >
            <span className="pill-checkbox-box">{solidify && <IconCheck />}</span>
            <span>Solidify transparent edges (BFS alpha bleed)</span>
          </div>
        </div>

        <div className="inspector-row">
          <span className="inspector-row-label">Alpha</span>
          <div
            className={`pill-checkbox ${makeOpaque ? "checked" : ""}`}
            onClick={() => setMakeOpaque(!makeOpaque)}
          >
            <span className="pill-checkbox-box">
              {makeOpaque && <IconCheck />}
            </span>
            <span>Force opaque alpha channel (A = 255)</span>
          </div>
        </div>

        {hasXt1Flags && (
          <div className="inspector-row">
            <span className="inspector-row-label">WTB Flags</span>
            <div
              className={`pill-checkbox ${syncFlags ? "checked" : ""}`}
              onClick={() => setSyncFlags(!syncFlags)}
            >
              <span className="pill-checkbox-box">
                {syncFlags && <IconCheck />}
              </span>
              <span>Sync WTB sRGB / cubemap flags from format</span>
            </div>
          </div>
        )}

        <div
          style={{
            display: "flex",
            justifyContent: "flex-end",
            gap: 8,
            marginTop: 6,
          }}
        >
          <button type="button" className="pill-button" onClick={onCancel}>
            Cancel
          </button>
          <button
            type="button"
            className="pill-button accent"
            onClick={() =>
              void onConfirm({
                encode_format: encodeFormat,
                quality,
                generate_mips: generateMips,
                solidify,
                make_opaque: makeOpaque,
                sync_flags: syncFlags,
              })
            }
          >
            Encode &amp; Save
          </button>
        </div>
      </div>
    </div>
  );
};
