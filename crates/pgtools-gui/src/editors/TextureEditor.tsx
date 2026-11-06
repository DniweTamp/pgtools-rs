import React from "react";
import { ContextMenuState, MenuItemSpec, TexModalState, TextureNodeView } from "../types";
import { IconCheck, IconChevronDown, IconImage, IconRefresh } from "../components/Icons";

export const ALL_TEXTURE_ENCODE_FORMATS: { group: string; options: { value: string; label: string }[] }[] = [
  {
    group: "Automatic",
    options: [{ value: "Auto", label: "Auto (Match Existing Texture Format)" }],
  },
  {
    group: "BCn Block Compression (DX9 / DX10)",
    options: [
      { value: "BC1_UNORM", label: "BC1_UNORM (DXT1 — 4 bpp)" },
      { value: "BC1_SRGB", label: "BC1_SRGB (DXT1 sRGB)" },
      { value: "BC2_UNORM", label: "BC2_UNORM (DXT3 — Explicit Alpha)" },
      { value: "BC2_SRGB", label: "BC2_SRGB (DXT3 sRGB)" },
      { value: "BC3_UNORM", label: "BC3_UNORM (DXT5 — Interpolated Alpha)" },
      { value: "BC3_SRGB", label: "BC3_SRGB (DXT5 sRGB)" },
      { value: "BC4_UNORM", label: "BC4_UNORM (ATI1 — Single Channel)" },
      { value: "BC5_UNORM", label: "BC5_UNORM (ATI2 — Dual Channel Normal)" },
      { value: "BC6H_UF16", label: "BC6H_UF16 (DX10 HDR Unsigned Float)" },
      { value: "BC6H_SF16", label: "BC6H_SF16 (DX10 HDR Signed Float)" },
      { value: "BC7_UNORM", label: "BC7_UNORM (DX10 High-Quality RGBA)" },
      { value: "BC7_SRGB", label: "BC7_SRGB (DX10 High-Quality sRGB)" },
    ],
  },
  {
    group: "ASTC LDR (Switch XT1 / BNTX / DX10 — All 14 Footprints)",
    options: [
      "4x4", "5x4", "5x5", "6x5", "6x6", "8x5", "8x6",
      "8x8", "10x5", "10x6", "10x8", "10x10", "12x10", "12x12",
    ].flatMap((fp) => [
      { value: `ASTC_${fp}_UNORM`, label: `ASTC_${fp}_UNORM (Linear)` },
      { value: `ASTC_${fp}_SRGB`, label: `ASTC_${fp}_SRGB (sRGB)` },
    ]),
  },
  {
    group: "Uncompressed / Float Formats",
    options: [
      { value: "RGBA8_UNORM", label: "RGBA8_UNORM (32-bit)" },
      { value: "RGBA8_SRGB", label: "RGBA8_SRGB (32-bit sRGB)" },
      { value: "BGRA8_UNORM", label: "BGRA8_UNORM (32-bit)" },
      { value: "BGRA8_SRGB", label: "BGRA8_SRGB (32-bit sRGB)" },
      { value: "RG8_UNORM", label: "R8G8_UNORM (16-bit)" },
      { value: "R8_UNORM", label: "R8_UNORM / AF_L8_UNORM (8-bit)" },
      { value: "RGBA16_FLOAT", label: "RGBA16_FLOAT (64-bit Half Float)" },
    ],
  },
];

export interface TextureCenterViewProps {
  activeTexture: TextureNodeView;
  centerTab: string;
  zoom: number;
  setZoom: React.Dispatch<React.SetStateAction<number>>;
  pan: { x: number; y: number };
  setPan: React.Dispatch<React.SetStateAction<{ x: number; y: number }>>;
  isPanning: boolean;
  setIsPanning: React.Dispatch<React.SetStateAction<boolean>>;
  panStartRef: React.MutableRefObject<{ x: number; y: number }>;
  handleResetView: () => void;
  buildAssetNodeContextMenu: (nodeId: string, kind: string) => MenuItemSpec[];
  setContextMenu: React.Dispatch<React.SetStateAction<ContextMenuState | null>>;
  setHoverSubmenuIdx: React.Dispatch<React.SetStateAction<number | null>>;
}

export const TextureCenterView: React.FC<TextureCenterViewProps> = ({
  activeTexture,
  centerTab,
  zoom,
  setZoom,
  pan,
  setPan,
  isPanning,
  setIsPanning,
  panStartRef,
  handleResetView,
  buildAssetNodeContextMenu,
  setContextMenu,
  setHoverSubmenuIdx,
}) => {
  return (
    <>
      {activeTexture && centerTab === "primary" ? (
              <div
                className="checkerboard-canvas"
                onWheel={(e) => {
                  e.preventDefault();
                  const factor = e.deltaY < 0 ? 1.12 : 1 / 1.12;
                  setZoom((z) => Math.max(0.05, Math.min(16.0, z * factor)));
                }}
                onMouseDown={(e) => {
                  if (e.button === 0) {
                    setIsPanning(true);
                    panStartRef.current = { x: e.clientX - pan.x, y: e.clientY - pan.y };
                  }
                }}
                onMouseMove={(e) => {
                  if (isPanning) {
                    setPan({
                      x: e.clientX - panStartRef.current.x,
                      y: e.clientY - panStartRef.current.y,
                    });
                  }
                }}
                onMouseUp={() => setIsPanning(false)}
                onMouseLeave={() => setIsPanning(false)}
                onDoubleClick={handleResetView}
                onContextMenu={(e) => {
                  e.preventDefault();
                  e.stopPropagation();
                  setHoverSubmenuIdx(null);
                  setContextMenu({
                    x: Math.min(e.clientX, window.innerWidth - 250),
                    y: Math.min(e.clientY, window.innerHeight - 280),
                    items: buildAssetNodeContextMenu(activeTexture.node_id, "TEXTURE"),
                  });
                }}
              >
                {activeTexture.preview_data_url ? (
                  <img
                    src={activeTexture.preview_data_url}
                    alt={activeTexture.title}
                    draggable={false}
                    style={{
                      transform: `translate(${pan.x}px, ${pan.y}px) scale(${zoom})`,
                      transformOrigin: "center center",
                      imageRendering: zoom >= 2.0 ? "pixelated" : "auto",
                      maxWidth: "90%",
                      maxHeight: "90%",
                      objectFit: "contain",
                      pointerEvents: "none",
                    }}
                  />
                ) : (
                  <div className="viewport-empty">
                    <IconImage size={44} />
                    <div className="viewport-empty-title">No preview</div>
                    <div className="viewport-empty-note">{activeTexture.validation_msg}</div>
                  </div>
                )}
              </div>
            ) : activeTexture && centerTab === "secondary" ? (
              <div className="struct-table-container">
                <div style={{ fontWeight: 600, color: "var(--brush-heading)", marginBottom: 8 }}>
                  Raw Header Bytes (First 128 Bytes)
                </div>
                <pre
                  className="mono"
                  style={{
                    backgroundColor: "var(--brush-card)",
                    border: "1px solid var(--brush-border)",
                    borderRadius: 8,
                    padding: 12,
                    color: "var(--brush-text)",
                    overflowX: "auto",
                  }}
                >
                  {activeTexture.raw_header_hex}
                </pre>
              </div>
            ) : null}
    </>
  );
};

export interface TextureInspectorViewProps {
  activeTexture: TextureNodeView;
  openCards: Record<string, boolean>;
  toggleCard: (key: string) => void;
  hashDraft: string;
  setHashDraft: (val: string) => void;
  flagsDraft: string;
  setFlagsDraft: (val: string) => void;
  texEncodeFormat: string;
  setTexEncodeFormat: (val: string) => void;
  texGenerateMips: boolean;
  setTexGenerateMips: (val: boolean) => void;
  texSolidify: boolean;
  setTexSolidify: (val: boolean) => void;
  texMakeOpaque: boolean;
  setTexMakeOpaque: (val: boolean) => void;
  runTextureMutation: (nodeId: string, action: string, payload?: any) => Promise<void>;
  promptAndRunTextureImport: (nodeId: string, action: "replace_file" | "add_texture") => Promise<void>;
  handleTextureExport: (nodeId: string, scope: "single" | "bank", format: "png" | "dds" | "raw") => Promise<void>;
}

export const TextureInspectorView: React.FC<TextureInspectorViewProps> = ({
  activeTexture,
  openCards,
  toggleCard,
  hashDraft,
  setHashDraft,
  flagsDraft,
  setFlagsDraft,
  texEncodeFormat,
  setTexEncodeFormat,
  texGenerateMips,
  setTexGenerateMips,
  texSolidify,
  setTexSolidify,
  texMakeOpaque,
  setTexMakeOpaque,
  runTextureMutation,
  promptAndRunTextureImport,
  handleTextureExport,
}) => {
  return (
            <div>
              <div className="inspector-header">
                <div className="inspector-icon-box">
                  <IconImage size={20} />
                </div>
                <div>
                  <div className="inspector-title">{activeTexture.title}</div>
                  <div className="inspector-subtitle">{activeTexture.breadcrumb}</div>
                </div>
              </div>

              <div className="inspector-spec">{activeTexture.spec_line}</div>

              <div className={`validation-banner ${activeTexture.validation_ok ? "" : "warn"}`}>
                <span>{activeTexture.validation_ok ? "✓" : "⚠"}</span>
                <span>{activeTexture.validation_msg}</span>
              </div>

              {/* Card 1: # Hash */}
              <div className="inspector-card">
                <button className="card-header-toggle" onClick={() => toggleCard("hash")}>
                  <span className="card-header-left">
                    <span style={{ color: "var(--brush-muted)", fontWeight: 700 }}>#</span>
                    <span>Hash &amp; Game Profile</span>
                  </span>
                  <IconChevronDown collapsed={!openCards.hash} />
                </button>
                {openCards.hash && (
                  <div className="card-body">
                    <div className="inspector-row" style={{ marginBottom: 6 }}>
                      <span className="inspector-row-label">Profile</span>
                      <span className="mono" style={{ fontSize: 11.5 }}>
                        {activeTexture.game_profile}
                      </span>
                    </div>
                    <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
                      <span style={{ color: "var(--brush-muted)", whiteSpace: "nowrap" }}>
                        CRC32 Hash
                      </span>
                      <input
                        type="text"
                        className="field-box mono"
                        style={{ flex: 1, minWidth: 0 }}
                        value={hashDraft}
                        onChange={(e) => setHashDraft(e.target.value)}
                        onKeyDown={(e) => {
                          if (e.key === "Enter") {
                            runTextureMutation(activeTexture.node_id, "set_hash", { hash: hashDraft });
                          }
                        }}
                        onBlur={() => {
                          if (hashDraft !== activeTexture.texture_id_hex) {
                            runTextureMutation(activeTexture.node_id, "set_hash", { hash: hashDraft });
                          }
                        }}
                      />
                      <button
                        className="pill-button"
                        onClick={() => {
                          const inputName = window.prompt(
                            "Enter texture name to compute crc32(name) & 0x7FFFFFFF:",
                            `tex${activeTexture.index}`
                          );
                          if (inputName) {
                            runTextureMutation(activeTexture.node_id, "calc_crc32", {
                              name: inputName,
                            });
                          }
                        }}
                      >
                        <IconRefresh />
                        <span>Calc</span>
                      </button>
                    </div>
                  </div>
                )}
              </div>

              {/* Card 2: Game-Aware Flags */}
              <div className="inspector-card">
                <button className="card-header-toggle" onClick={() => toggleCard("flags")}>
                  <span className="card-header-left">
                    <span style={{ color: "var(--brush-muted)" }}>⚑</span>
                    <span>
                      {activeTexture.has_confirmed_xt1_flags
                        ? "XT1 Flags (table2)"
                        : "WTB Entry Flags"}
                    </span>
                  </span>
                  <IconChevronDown collapsed={!openCards.flags} />
                </button>
                {openCards.flags && (
                  <div className="card-body">
                    {activeTexture.has_confirmed_xt1_flags ? (
                      <>
                        <div className="inspector-row">
                          <span className="inspector-row-label">Hash</span>
                          <div>
                            <div
                              className={`pill-checkbox ${
                                activeTexture.has_crc32_flag ? "checked" : ""
                              }`}
                            >
                              <span className="pill-checkbox-box">
                                {activeTexture.has_crc32_flag && <IconCheck />}
                              </span>
                              <span>Has CRC32 hash (0x80000000)</span>
                            </div>
                          </div>
                        </div>

                        <div className="inspector-row">
                          <span className="inspector-row-label">Color</span>
                          <div>
                            <div
                              className={`pill-checkbox ${
                                activeTexture.srgb_flag ? "checked" : ""
                              }`}
                            >
                              <span className="pill-checkbox-box">
                                {activeTexture.srgb_flag && <IconCheck />}
                              </span>
                              <span>sRGB color space (0x02000000)</span>
                            </div>
                          </div>
                        </div>

                        <div className="inspector-row">
                          <span className="inspector-row-label">Usage</span>
                          <div>
                            <div
                              className={`pill-checkbox ${
                                activeTexture.cubemap_flag ? "checked" : ""
                              }`}
                            >
                              <span className="pill-checkbox-box">
                                {activeTexture.cubemap_flag && <IconCheck />}
                              </span>
                              <span>Cube map usage (0x00000200)</span>
                            </div>
                          </div>
                        </div>
                      </>
                    ) : (
                      <div style={{ color: "var(--brush-muted)", fontSize: 11.5, marginBottom: 6 }}>
                        Raw WTB flags for {activeTexture.game_profile} (XT1 bitmasks hidden for non-XT1 banks):
                      </div>
                    )}

                    <div className="inspector-row">
                      <span className="inspector-row-label">Flags Hex</span>
                      <input
                        type="text"
                        className="field-box mono"
                        value={flagsDraft}
                        onChange={(e) => setFlagsDraft(e.target.value)}
                        onKeyDown={(e) => {
                          if (e.key === "Enter") {
                            runTextureMutation(activeTexture.node_id, "set_flags", {
                              flags_hex: flagsDraft,
                            });
                          }
                        }}
                        onBlur={() => {
                          if (flagsDraft !== activeTexture.flags_hex) {
                            runTextureMutation(activeTexture.node_id, "set_flags", {
                              flags_hex: flagsDraft,
                            });
                          }
                        }}
                      />
                    </div>
                    <div className="mono" style={{ color: "var(--brush-muted)", marginTop: 4 }}>
                      LE bytes = {activeTexture.table2_bytes_str}
                    </div>
                  </div>
                )}
              </div>

              {/* Card 3: Custom Texture Import & Encoding */}
              <div className="inspector-card">
                <button className="card-header-toggle" onClick={() => toggleCard("encoding")}>
                  <span className="card-header-left">
                    <span style={{ color: "var(--brush-muted)" }}>⇄</span>
                    <span>Import &amp; Encoding</span>
                  </span>
                  <IconChevronDown collapsed={!openCards.encoding} />
                </button>
                {openCards.encoding && (
                  <div className="card-body">
                    <div className="inspector-row">
                      <span className="inspector-row-label">Format</span>
                      <select
                        className="pill-select mono"
                        style={{ width: "100%", fontSize: 11.5 }}
                        value={texEncodeFormat}
                        onChange={(e) => setTexEncodeFormat(e.target.value)}
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
                      <span className="inspector-row-label">Mipmaps</span>
                      <div
                        className={`pill-checkbox ${texGenerateMips ? "checked" : ""}`}
                        onClick={() => setTexGenerateMips(!texGenerateMips)}
                      >
                        <span className="pill-checkbox-box">{texGenerateMips && <IconCheck />}</span>
                        <span>Generate full mipmap chain</span>
                      </div>
                    </div>

                    <div className="inspector-row">
                      <span className="inspector-row-label">Solidify</span>
                      <div
                        className={`pill-checkbox ${texSolidify ? "checked" : ""}`}
                        onClick={() => setTexSolidify(!texSolidify)}
                        title="Multi-source BFS alpha edge bleed (prevents dark fringes on transparent cutouts)"
                      >
                        <span className="pill-checkbox-box">{texSolidify && <IconCheck />}</span>
                        <span>Solidify alpha edges (BFS bleed)</span>
                      </div>
                    </div>

                    <div className="inspector-row">
                      <span className="inspector-row-label">Alpha</span>
                      <div
                        className={`pill-checkbox ${texMakeOpaque ? "checked" : ""}`}
                        onClick={() => setTexMakeOpaque(!texMakeOpaque)}
                      >
                        <span className="pill-checkbox-box">{texMakeOpaque && <IconCheck />}</span>
                        <span>Force opaque alpha (A = 255)</span>
                      </div>
                    </div>

                    <div style={{ display: "flex", gap: 6, marginTop: 6 }}>
                      <button
                        className="pill-button accent"
                        style={{ flex: 1 }}
                        onClick={() => promptAndRunTextureImport(activeTexture.node_id, "replace_file")}
                      >
                        Replace Texture...
                      </button>
                      <button
                        className="pill-button"
                        onClick={() => handleTextureExport(activeTexture.node_id, "single", "png")}
                      >
                        Export PNG
                      </button>
                    </div>
                  </div>
                )}
              </div>
            </div>
  );
};

export interface TextureImportModalProps {
  texModalState: TexModalState;
  setTexModalState: React.Dispatch<React.SetStateAction<TexModalState | null>>;
  activeTexture: TextureNodeView | null;
  setTexEncodeFormat: (val: string) => void;
  setTexGenerateMips: (val: boolean) => void;
  setTexSolidify: (val: boolean) => void;
  setTexMakeOpaque: (val: boolean) => void;
  runTextureMutation: (nodeId: string, action: string, payload?: any) => Promise<void>;
}

export const TextureImportModal: React.FC<TextureImportModalProps> = ({
  texModalState,
  setTexModalState,
  activeTexture,
  setTexEncodeFormat,
  setTexGenerateMips,
  setTexSolidify,
  setTexMakeOpaque,
  runTextureMutation,
}) => {
  return (
        <div
          className="modal-backdrop"
          onClick={() => setTexModalState(null)}
        >
          <div
            className="modal-card"
            onClick={(e) => e.stopPropagation()}
          >
            <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center" }}>
              <div>
                <div style={{ fontSize: 15, fontWeight: 600, color: "var(--brush-heading)" }}>
                  {texModalState.action === "replace_file"
                    ? "Replace & Encode Texture"
                    : "Add & Encode Texture to Bank"}
                </div>
                <div className="mono" style={{ color: "var(--brush-muted)", fontSize: 11.5, marginTop: 2 }}>
                  Source: {texModalState.fileName}
                </div>
              </div>
              <button
                type="button"
                className="pill-button"
                style={{ height: 26, padding: "0 8px" }}
                onClick={() => setTexModalState(null)}
              >
                ✕
              </button>
            </div>

            {activeTexture && (
              <div
                className="mono"
                style={{
                  padding: "7px 10px",
                  borderRadius: 8,
                  backgroundColor: "var(--brush-window)",
                  border: "1px solid var(--brush-border)",
                  fontSize: 11.5,
                  color: "var(--brush-muted)",
                }}
              >
                Target: {activeTexture.container_ext} · Current Format:{" "}
                <span style={{ color: "var(--brush-text)", fontWeight: 600 }}>
                  {activeTexture.format_name}
                </span>{" "}
                ({activeTexture.width}×{activeTexture.height}, {activeTexture.mip_count} mips)
              </div>
            )}

            <div className="inspector-row">
              <span className="inspector-row-label">Encode Format</span>
              <select
                className="pill-select mono"
                style={{ width: "100%", fontSize: 11.5 }}
                value={texModalState.encodeFormat}
                onChange={(e) =>
                  setTexModalState({ ...texModalState, encodeFormat: e.target.value })
                }
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
              <span className="inspector-row-label">Mipmaps</span>
              <div
                className={`pill-checkbox ${texModalState.generateMips ? "checked" : ""}`}
                onClick={() =>
                  setTexModalState({
                    ...texModalState,
                    generateMips: !texModalState.generateMips,
                  })
                }
              >
                <span className="pill-checkbox-box">
                  {texModalState.generateMips && <IconCheck />}
                </span>
                <span>Generate full box-filtered mipmap chain</span>
              </div>
            </div>

            <div className="inspector-row">
              <span className="inspector-row-label">Solidify</span>
              <div
                className={`pill-checkbox ${texModalState.solidify ? "checked" : ""}`}
                onClick={() =>
                  setTexModalState({ ...texModalState, solidify: !texModalState.solidify })
                }
              >
                <span className="pill-checkbox-box">
                  {texModalState.solidify && <IconCheck />}
                </span>
                <span>Solidify transparent edges (BFS alpha bleed)</span>
              </div>
            </div>

            <div className="inspector-row">
              <span className="inspector-row-label">Opaque</span>
              <div
                className={`pill-checkbox ${texModalState.makeOpaque ? "checked" : ""}`}
                onClick={() =>
                  setTexModalState({
                    ...texModalState,
                    makeOpaque: !texModalState.makeOpaque,
                  })
                }
              >
                <span className="pill-checkbox-box">
                  {texModalState.makeOpaque && <IconCheck />}
                </span>
                <span>Force opaque alpha channel (A = 255)</span>
              </div>
            </div>

            <div style={{ display: "flex", justifyContent: "flex-end", gap: 8, marginTop: 6 }}>
              <button
                type="button"
                className="pill-button"
                onClick={() => setTexModalState(null)}
              >
                Cancel
              </button>
              <button
                type="button"
                className="pill-button accent"
                onClick={async () => {
                  const modal = texModalState;
                  setTexModalState(null);
                  setTexEncodeFormat(modal.encodeFormat);
                  setTexGenerateMips(modal.generateMips);
                  setTexSolidify(modal.solidify);
                  setTexMakeOpaque(modal.makeOpaque);
                  await runTextureMutation(modal.nodeId, modal.action, {
                    path: modal.path,
                    encode_format: modal.encodeFormat,
                    generate_mips: modal.generateMips,
                    solidify: modal.solidify,
                    make_opaque: modal.makeOpaque,
                  });
                }}
              >
                Encode &amp; Import Texture
              </button>
            </div>
          </div>
        </div>
  );
};
