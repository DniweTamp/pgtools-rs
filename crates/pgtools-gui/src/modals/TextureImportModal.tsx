import React from "react";
import { IconCheck } from "../components/Icons";
import { StudioSelect } from "../components/StudioSelect";
import {
  ALL_TEXTURE_ENCODE_FORMATS,
  ENCODE_QUALITY_OPTIONS,
} from "../constants/textureFormats";
import { TexModalState, TextureNodeView } from "../types";

interface TextureImportModalProps {
  texModalState: TexModalState;
  setTexModalState: React.Dispatch<React.SetStateAction<TexModalState | null>>;
  activeTexture: TextureNodeView | null;
  setTexEncodeFormat: (val: string) => void;
  setTexQuality: (val: string) => void;
  setTexGenerateMips: (val: boolean) => void;
  setTexSolidify: (val: boolean) => void;
  setTexMakeOpaque: (val: boolean) => void;
  setTexSyncFlags: (val: boolean) => void;
  runTextureMutation: (nodeId: string, action: string, payload?: Record<string, unknown>) => Promise<void>;
}

export const TextureImportModal: React.FC<TextureImportModalProps> = ({
  texModalState,
  setTexModalState,
  activeTexture,
  setTexEncodeFormat,
  setTexQuality,
  setTexGenerateMips,
  setTexSolidify,
  setTexMakeOpaque,
  setTexSyncFlags,
  runTextureMutation,
}) => {
  return (
    <div className="modal-backdrop" onClick={() => setTexModalState(null)}>
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
              {texModalState.action === "replace_file"
                ? "Replace & Encode Texture"
                : "Add & Encode Texture to Bank"}
            </div>
            <div
              className="mono"
              style={{
                color: "var(--brush-muted)",
                fontSize: 11.5,
                marginTop: 2,
              }}
            >
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
            ({activeTexture.width}×{activeTexture.height}, {activeTexture.mip_count}{" "}
            mips)
          </div>
        )}

        {texModalState.action === "add_texture" && (
          <div className="inspector-row">
            <span className="inspector-row-label">Texture Name</span>
            <input
              type="text"
              className="field-box mono"
              style={{ width: "100%", fontSize: 11.5 }}
              placeholder="Optional name for crc32(name) & 0x7FFFFFFF..."
              value={texModalState.textureName ?? ""}
              onChange={(e) =>
                setTexModalState({
                  ...texModalState,
                  textureName: e.target.value,
                })
              }
            />
          </div>
        )}

        <div className="inspector-row">
          <span className="inspector-row-label">Target Format</span>
          <StudioSelect
            className="pill-select mono"
            style={{ width: "100%", fontSize: 11.5 }}
            value={texModalState.encodeFormat}
            groups={ALL_TEXTURE_ENCODE_FORMATS.map((grp) => ({
              label: grp.group,
              options: grp.options,
            }))}
            onChange={(val) =>
              setTexModalState({ ...texModalState, encodeFormat: val })
            }
          />
        </div>

        <div className="inspector-row">
          <span className="inspector-row-label">Quality</span>
          <StudioSelect
            className="pill-select"
            style={{ width: "100%", fontSize: 11.5 }}
            value={texModalState.quality}
            options={ENCODE_QUALITY_OPTIONS}
            onChange={(val) =>
              setTexModalState({ ...texModalState, quality: val })
            }
          />
        </div>

        <div className="inspector-row">
          <span className="inspector-row-label">Mipmaps</span>
          <div
            className={`pill-checkbox ${
              texModalState.generateMips ? "checked" : ""
            }`}
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
          <span className="inspector-row-label">Transparent RGB</span>
          <div
            className={`pill-checkbox ${texModalState.solidify ? "checked" : ""}`}
            onClick={() =>
              setTexModalState({
                ...texModalState,
                solidify: !texModalState.solidify,
              })
            }
          >
            <span className="pill-checkbox-box">
              {texModalState.solidify && <IconCheck />}
            </span>
            <span>Solidify transparent edges (BFS alpha bleed)</span>
          </div>
        </div>

        <div className="inspector-row">
          <span className="inspector-row-label">Alpha</span>
          <div
            className={`pill-checkbox ${
              texModalState.makeOpaque ? "checked" : ""
            }`}
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

        {activeTexture?.has_confirmed_xt1_flags && (
          <div className="inspector-row">
            <span className="inspector-row-label">WTB Flags</span>
            <div
              className={`pill-checkbox ${
                texModalState.syncFlags ? "checked" : ""
              }`}
              onClick={() =>
                setTexModalState({
                  ...texModalState,
                  syncFlags: !texModalState.syncFlags,
                })
              }
            >
              <span className="pill-checkbox-box">
                {texModalState.syncFlags && <IconCheck />}
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
              setTexQuality(modal.quality);
              setTexGenerateMips(modal.generateMips);
              setTexSolidify(modal.solidify);
              setTexMakeOpaque(modal.makeOpaque);
              setTexSyncFlags(modal.syncFlags);
              await runTextureMutation(modal.nodeId, modal.action, {
                path: modal.path,
                encode_format: modal.encodeFormat,
                quality: modal.quality,
                generate_mips: modal.generateMips,
                solidify: modal.solidify,
                make_opaque: modal.makeOpaque,
                sync_flags: modal.syncFlags,
                name: modal.textureName,
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
