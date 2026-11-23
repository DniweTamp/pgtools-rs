import React, { useEffect, useRef, useState } from "react";
import { studioPrompt } from "../components/StudioDialog";
import { StudioRange } from "../components/StudioRange";
import { StudioSelect } from "../components/StudioSelect";
import { GpuTextureCanvas, GpuTextureCanvasHandle } from "../components/GpuTextureCanvas";
import {
  ALL_TEXTURE_ENCODE_FORMATS,
  ENCODE_QUALITY_OPTIONS,
} from "../constants/textureFormats";
import {
  ContextMenuState,
  MenuItemSpec,
  TextureNodeView,
  UvdContextView,
} from "../types";
import { CubemapSkyViewport } from "./CubemapSkyViewport";
import { UvdActions, UvdInspectorSection } from "./UvdEditor";
import {
  IconCheck,
  IconChevronDown,
  IconImage,
  IconRefresh,
} from "../components/Icons";

interface TextureCenterViewProps {
  activeTexture: TextureNodeView;
  mipIndex: number;
  layerIndex: number;
  channelMode: string;
  opaquePreview: boolean;
  onChangeLayer: (newLayer: number) => void;
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
  onStatus?: (msg: string) => void;
}

export const TextureCenterView: React.FC<TextureCenterViewProps> = ({
  activeTexture,
  mipIndex,
  layerIndex,
  channelMode,
  opaquePreview,
  onChangeLayer,
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
  onStatus,
}) => {
  const [animPlaying, setAnimPlaying] = useState(false);
  const [fpsDraft, setFpsDraft] = useState("15");
  const [fps, setFps] = useState(15);
  const gpuCanvasRef = useRef<GpuTextureCanvasHandle | null>(null);

  const isCubemap = activeTexture.dimension === "Cube" && activeTexture.uvd_mode !== "item";
  const layerCount = Math.max(1, activeTexture.layer_count);
  const isAnimated2D = !isCubemap && layerCount > 1;
  const clampedLayer = Math.max(0, Math.min(layerCount - 1, layerIndex));

  const layerIndexRef = useRef(clampedLayer);
  layerIndexRef.current = clampedLayer;
  const onChangeLayerRef = useRef(onChangeLayer);
  onChangeLayerRef.current = onChangeLayer;

  useEffect(() => {
    setAnimPlaying(false);
  }, [activeTexture.node_id]);

  useEffect(() => {
    if (!animPlaying || !isAnimated2D) return;
    const intervalMs = 1000 / Math.max(1, Math.min(240, fps));
    const timer = window.setInterval(() => {
      const next = (layerIndexRef.current + 1) % layerCount;
      layerIndexRef.current = next;
      onChangeLayerRef.current(next);
    }, intervalMs);
    return () => window.clearInterval(timer);
  }, [animPlaying, isAnimated2D, layerCount, fps]);

  const commitFps = () => {
    const parsed = parseFloat(fpsDraft.replace(",", "."));
    const clamped = Number.isFinite(parsed)
      ? Math.max(1, Math.min(240, parsed))
      : 15;
    setFps(clamped);
    setFpsDraft(String(Math.round(clamped * 100) / 100));
  };

  const copyPreviewImageToClipboard = async () => {
    setContextMenu(null);
    try {
      let blob: Blob | null = null;
      if (activeTexture.gpu_direct && gpuCanvasRef.current) {
        blob = await gpuCanvasRef.current.toPngBlob();
      } else if (activeTexture.preview_data_url) {
        const resp = await fetch(activeTexture.preview_data_url);
        blob = await resp.blob();
      }
      if (!blob) {
        onStatus?.("There is no decoded image to copy.");
        return;
      }
      if (navigator.clipboard && "write" in navigator.clipboard && typeof ClipboardItem !== "undefined") {
        await navigator.clipboard.write([new ClipboardItem({ "image/png": blob })]);
        onStatus?.(`Copied Texture #${activeTexture.index} image to clipboard.`);
      } else if (activeTexture.preview_data_url) {
        await navigator.clipboard?.writeText(activeTexture.preview_data_url);
        onStatus?.(`Copied Texture #${activeTexture.index} data URL to clipboard.`);
      }
    } catch (err) {
      onStatus?.(`Could not copy image: ${String(err)}`);
    }
  };

  const texRevision = `${activeTexture.size_bytes}:${activeTexture.raw_header_hex}:${activeTexture.spec_line}`;

  return (
    <>
      {activeTexture && !isCubemap ? (
        <div style={{ display: "flex", flexDirection: "column", flex: 1, minHeight: 0 }}>
          <div
            className="checkerboard-canvas"
            style={{ flex: 1, minHeight: 0 }}
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
              const menuKind =
                activeTexture.uvd_mode === "item"
                  ? "UVD_ITEM"
                  : activeTexture.uvd_mode === "group"
                  ? "UVD_GROUP"
                  : "TEXTURE";
              const baseItems = buildAssetNodeContextMenu(activeTexture.node_id, menuKind);
              setContextMenu({
                x: Math.min(e.clientX, window.innerWidth - 260),
                y: Math.min(e.clientY, window.innerHeight - 320),
                items: [
                  {
                    label: "Reset / Fit View (100%)",
                    action: () => {
                      setContextMenu(null);
                      handleResetView();
                    },
                  },
                  {
                    label: "Copy Image to Clipboard",
                    action: () => void copyPreviewImageToClipboard(),
                  },
                  { separator: true },
                  ...baseItems,
                ],
              });
            }}
          >
            {activeTexture.gpu_direct ? (
              <GpuTextureCanvas
                ref={gpuCanvasRef}
                nodeId={activeTexture.node_id}
                mipIndex={mipIndex}
                layerIndex={clampedLayer}
                revision={texRevision}
                channelMode={channelMode}
                opaquePreview={opaquePreview}
                alt={activeTexture.title}
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
            ) : activeTexture.preview_data_url ? (
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

          {/* 2D Animated Multilayer Bar (matches XT1 Studio AnimBar) */}
          {isAnimated2D && (
            <div
              style={{
                display: "flex",
                alignItems: "center",
                gap: 10,
                padding: "6px 12px",
                backgroundColor: "var(--brush-card)",
                borderTop: "1px solid var(--brush-border)",
              }}
            >
              <button
                type="button"
                className="pill-button"
                onClick={() => setAnimPlaying((p) => !p)}
                data-tooltip="Play or pause the layer animation"
              >
                <span>{animPlaying ? "⏸ Pause" : "▶ Play"}</span>
              </button>

              <span style={{ color: "var(--brush-muted)", fontSize: 12 }}>FPS</span>
              <input
                type="text"
                className="field-box mono"
                style={{ width: 48, height: 24 }}
                value={fpsDraft}
                onChange={(e) => setFpsDraft(e.target.value)}
                onBlur={commitFps}
                onKeyDown={(e) => {
                  if (e.key === "Enter") commitFps();
                }}
                data-tooltip="Playback speed in frames per second (1 - 240)"
              />

              <StudioRange
                min={0}
                max={Math.max(1, layerCount - 1)}
                step={1}
                value={clampedLayer}
                defaultValue={0}
                compact
                ticks={layerCount <= 24 ? layerCount : 0}
                style={{ flex: 1 }}
                onChange={(val) => {
                  setAnimPlaying(false);
                  onChangeLayer(val);
                }}
                data-tooltip="Scrub through the texture's animation frames / layers"
              />

              <span
                className="mono"
                style={{ color: "var(--brush-muted)", fontSize: 11.5, minWidth: 52, textAlign: "right" }}
              >
                {clampedLayer + 1} / {layerCount}
              </span>
            </div>
          )}
        </div>
      ) : activeTexture && isCubemap ? (
        <CubemapSkyViewport
          nodeId={activeTexture.node_id}
          mipIndex={mipIndex}
          channelMode={channelMode}
          opaquePreview={opaquePreview}
          revision={texRevision}
        />
      ) : null}
    </>
  );
};

interface TextureInspectorViewProps {
  activeTexture: TextureNodeView;
  openCards: Record<string, boolean>;
  toggleCard: (key: string) => void;
  hashDraft: string;
  setHashDraft: (val: string) => void;
  flagsDraft: string;
  setFlagsDraft: (val: string) => void;
  texEncodeFormat: string;
  setTexEncodeFormat: (val: string) => void;
  texQuality: string;
  setTexQuality: (val: string) => void;
  texGenerateMips: boolean;
  setTexGenerateMips: (val: boolean) => void;
  texSolidify: boolean;
  setTexSolidify: (val: boolean) => void;
  texMakeOpaque: boolean;
  setTexMakeOpaque: (val: boolean) => void;
  texSyncFlags: boolean;
  setTexSyncFlags: (val: boolean) => void;
  runTextureMutation: (nodeId: string, action: string, payload?: Record<string, unknown>) => Promise<void>;
  promptAndRunTextureImport: (
    nodeId: string,
    action: "replace_file" | "add_texture"
  ) => Promise<void>;
  handleTextureExport: (
    nodeId: string,
    scope: "single" | "bank" | "container",
    format: "png" | "dds" | "raw"
  ) => Promise<void>;
  uvdContext?: UvdContextView | null;
  uvdActions?: UvdActions;
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
  texQuality,
  setTexQuality,
  texGenerateMips,
  setTexGenerateMips,
  texSolidify,
  setTexSolidify,
  texMakeOpaque,
  setTexMakeOpaque,
  texSyncFlags,
  setTexSyncFlags,
  runTextureMutation,
  promptAndRunTextureImport,
  handleTextureExport,
  uvdContext,
  uvdActions,
}) => {
  const hashDisabled =
    activeTexture.has_confirmed_xt1_flags && !activeTexture.has_crc32_flag;
  const isUvdItem = activeTexture.uvd_mode === "item";

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

      {uvdContext && uvdActions && (
        <UvdInspectorSection
          uvd={uvdContext}
          openCards={openCards}
          toggleCard={toggleCard}
          uvdActions={uvdActions}
        />
      )}

      {!isUvdItem && (
        <>
      {/* Card 1: # Hash & Built-in BNTX Name */}
      <div className="inspector-card">
        <button className="card-header-toggle" onClick={() => toggleCard("hash")}>
          <span className="card-header-left">
            <span style={{ color: "var(--brush-muted)", fontWeight: 700 }}>#</span>
            <span>Hash &amp; Game Profile</span>
          </span>
          <IconChevronDown collapsed={!openCards.hash} />
        </button>
                  <div className={`collapse-grid${!!openCards.hash ? " expanded" : ""}`}>
  <div className="collapse-inner">
    <div className="card-body">
            <div className="inspector-row" style={{ marginBottom: 6 }}>
              <span className="inspector-row-label">Profile</span>
              <span className="mono" style={{ fontSize: 11.5 }}>
                {activeTexture.game_profile}
              </span>
            </div>

            {activeTexture.embedded_name && (
              <div className="inspector-row" style={{ marginBottom: 6 }}>
                <span className="inspector-row-label">BNTX Name</span>
                <div style={{ display: "flex", alignItems: "center", gap: 6, flex: 1 }}>
                  <span
                    className="mono"
                    style={{
                      fontSize: 11.5,
                      padding: "3px 7px",
                      borderRadius: 6,
                      backgroundColor: "var(--brush-window)",
                      border: "1px solid var(--brush-border)",
                      flex: 1,
                      overflow: "hidden",
                      textOverflow: "ellipsis",
                    }}
                    data-tooltip={`Built-in BNTX BRTI texture name: ${activeTexture.embedded_name}`}
                  >
                    {activeTexture.embedded_name}
                  </span>
                  <button
                    type="button"
                    className="pill-button"
                    onClick={() =>
                      runTextureMutation(activeTexture.node_id, "calc_crc32", {
                        name: activeTexture.embedded_name,
                      })
                    }
                    data-tooltip={`Recalculate CRC32 hash from embedded BNTX name '${activeTexture.embedded_name}'`}
                  >
                    <IconRefresh />
                    <span>Recalc CRC32</span>
                  </button>
                </div>
              </div>
            )}

            <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
              <span style={{ color: "var(--brush-muted)", whiteSpace: "nowrap" }}>
                CRC32 Hash
              </span>
              <input
                type="text"
                className="field-box mono"
                style={{ flex: 1, minWidth: 0, opacity: hashDisabled ? 0.5 : 1 }}
                disabled={hashDisabled}
                value={hashDraft}
                onChange={(e) => setHashDraft(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === "Enter" && !hashDisabled) {
                    runTextureMutation(activeTexture.node_id, "set_hash", {
                      hash: hashDraft,
                    });
                  }
                }}
                onBlur={() => {
                  if (!hashDisabled && hashDraft !== activeTexture.texture_id_hex) {
                    runTextureMutation(activeTexture.node_id, "set_hash", {
                      hash: hashDraft,
                    });
                  }
                }}
              />
              <button
                className="pill-button"
                disabled={hashDisabled}
                onClick={async () => {
                  const defaultName =
                    activeTexture.embedded_name || `tex${activeTexture.index}`;
                  const inputName = await studioPrompt(
                    "Enter texture name to compute crc32(name) & 0x7FFFFFFF:",
                    defaultName,
                    { title: "Compute Texture CRC32" }
                  );
                  if (inputName) {
                    runTextureMutation(activeTexture.node_id, "calc_crc32", {
                      name: inputName,
                    });
                  }
                }}
                data-tooltip={
                  hashDisabled
                    ? "Enable 'Has CRC32 hash' below to edit or compute the table3 hash"
                    : "Compute crc32(name) & 0x7FFFFFFF from a texture name"
                }
              >
                <IconRefresh />
                <span>Calculate CRC32</span>
              </button>
            </div>
          </div>
  </div>
</div>
        
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
                  <div className={`collapse-grid${!!openCards.flags ? " expanded" : ""}`}>
  <div className="collapse-inner">
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
                      onClick={() =>
                        runTextureMutation(activeTexture.node_id, "toggle_xt1_flag", {
                          flag: "crc32",
                        })
                      }
                      data-tooltip="Toggle table2 byte 0 bit 0x20 (LE u32 0x00000020): whether this bank entry carries a CRC32 hash in table3"
                    >
                      <span className="pill-checkbox-box">
                        {activeTexture.has_crc32_flag && <IconCheck />}
                      </span>
                      <span>Has CRC32 hash (b0: 0x20)</span>
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
                      onClick={() =>
                        runTextureMutation(activeTexture.node_id, "toggle_xt1_flag", {
                          flag: "srgb",
                        })
                      }
                      data-tooltip="Toggle table2 byte 3 bit 0x04 (LE u32 0x04000000): sRGB color space"
                    >
                      <span className="pill-checkbox-box">
                        {activeTexture.srgb_flag && <IconCheck />}
                      </span>
                      <span>sRGB color space (b3: 0x04)</span>
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
                      onClick={() =>
                        runTextureMutation(activeTexture.node_id, "toggle_xt1_flag", {
                          flag: "cubemap",
                        })
                      }
                      data-tooltip="Toggle table2 byte 3 bit 0x80 (LE u32 0x80000000): cubemap / environment map usage"
                    >
                      <span className="pill-checkbox-box">
                        {activeTexture.cubemap_flag && <IconCheck />}
                      </span>
                      <span>Cube map usage (b3: 0x80)</span>
                    </div>
                  </div>
                </div>
              </>
            ) : (
              <div style={{ color: "var(--brush-muted)", fontSize: 11.5, marginBottom: 6 }}>
                Raw WTB flags for {activeTexture.game_profile} (Bayonetta 3 XT1 bitmasks are isolated from non-XT1 games):
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
              table2 = {activeTexture.table2_bytes_str}
            </div>
          </div>
  </div>
</div>
        
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
                  <div className={`collapse-grid${!!openCards.encoding ? " expanded" : ""}`}>
  <div className="collapse-inner">
    <div className="card-body">
            <div className="inspector-row">
              <span className="inspector-row-label">Format</span>
              <StudioSelect
                className="pill-select mono"
                style={{ width: "100%", fontSize: 11.5 }}
                value={texEncodeFormat}
                groups={ALL_TEXTURE_ENCODE_FORMATS.map((grp) => ({
                  label: grp.group,
                  options: grp.options,
                }))}
                onChange={(val) => setTexEncodeFormat(val)}
              />
            </div>

            <div className="inspector-row">
              <span className="inspector-row-label">Quality</span>
              <StudioSelect
                className="pill-select"
                style={{ width: "100%", fontSize: 11.5 }}
                value={texQuality}
                options={ENCODE_QUALITY_OPTIONS}
                onChange={(val) => setTexQuality(val)}
              />
            </div>

            <div className="inspector-row">
              <span className="inspector-row-label">Mipmaps</span>
              <div
                className={`pill-checkbox ${texGenerateMips ? "checked" : ""}`}
                onClick={() => setTexGenerateMips(!texGenerateMips)}
              >
                <span className="pill-checkbox-box">
                  {texGenerateMips && <IconCheck />}
                </span>
                <span>Generate full mipmap chain</span>
              </div>
            </div>

            <div className="inspector-row">
              <span className="inspector-row-label">Solidify</span>
              <div
                className={`pill-checkbox ${texSolidify ? "checked" : ""}`}
                onClick={() => setTexSolidify(!texSolidify)}
                data-tooltip="Multi-source BFS alpha edge bleed (prevents dark fringes on transparent cutouts)"
              >
                <span className="pill-checkbox-box">
                  {texSolidify && <IconCheck />}
                </span>
                <span>Solidify alpha edges (BFS bleed)</span>
              </div>
            </div>

            <div className="inspector-row">
              <span className="inspector-row-label">Alpha</span>
              <div
                className={`pill-checkbox ${texMakeOpaque ? "checked" : ""}`}
                onClick={() => setTexMakeOpaque(!texMakeOpaque)}
              >
                <span className="pill-checkbox-box">
                  {texMakeOpaque && <IconCheck />}
                </span>
                <span>Force opaque alpha (A = 255)</span>
              </div>
            </div>

            {activeTexture.has_confirmed_xt1_flags && (
              <div className="inspector-row">
                <span className="inspector-row-label">Flags</span>
                <div
                  className={`pill-checkbox ${texSyncFlags ? "checked" : ""}`}
                  onClick={() => setTexSyncFlags(!texSyncFlags)}
                  data-tooltip="On add or replace, sync the sRGB (b3: 0x04) and cubemap (b3: 0x80) flags from the texture format"
                >
                  <span className="pill-checkbox-box">
                    {texSyncFlags && <IconCheck />}
                  </span>
                  <span>Sync WTB sRGB / cubemap flags</span>
                </div>
              </div>
            )}

            <div
              className="mono"
              style={{
                fontSize: 10.5,
                color: "var(--brush-muted)",
                marginTop: 2,
                marginBottom: 4,
              }}
            >
              Encoder: {activeTexture.nvtt_status}
            </div>

            <div style={{ display: "flex", gap: 6, marginTop: 6 }}>
              <button
                className="pill-button accent"
                style={{ flex: 1 }}
                onClick={() =>
                  promptAndRunTextureImport(activeTexture.node_id, "replace_file")
                }
              >
                Replace Texture...
              </button>
              <button
                className="pill-button"
                onClick={() =>
                  handleTextureExport(activeTexture.node_id, "single", "png")
                }
              >
                Export PNG
              </button>
            </div>
          </div>
  </div>
</div>
        
      </div>
        </>
      )}
    </div>
  );
};

