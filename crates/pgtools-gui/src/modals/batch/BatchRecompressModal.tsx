import React, { useState } from "react";
import { pickFolderDialog, runBatchRecompressAction } from "../../api/tauri";
import { IconCheck, IconFolder } from "../../components/Icons";
import { StudioRadio } from "../../components/StudioRadio";
import { StudioSelect } from "../../components/StudioSelect";
import { ENCODE_QUALITY_OPTIONS } from "../../constants/textureFormats";
import { DocumentOutline } from "../../types";

interface BatchRecompressModalProps {
  hasOpenDocument: boolean;
  defaultFolder: string;
  onClose: () => void;
  onOutlineUpdated: (outline: DocumentOutline, statusMsg: string) => void;
}

export const BatchRecompressModal: React.FC<BatchRecompressModalProps> = ({
  hasOpenDocument,
  defaultFolder,
  onClose,
  onOutlineUpdated,
}) => {
  const [sourceMode, setSourceMode] = useState<"current" | "folder">(
    hasOpenDocument ? "current" : "folder"
  );
  const [folderPath, setFolderPath] = useState(defaultFolder);
  const [recursive, setRecursive] = useState(true);
  const [opaqueFormat, setOpaqueFormat] = useState("BC1");
  const [alphaFormat, setAlphaFormat] = useState("BC3");
  const [opaqueQuality, setOpaqueQuality] = useState("Normal");
  const [alphaQuality, setAlphaQuality] = useState("Normal");
  const [downscale, setDownscale] = useState(false);
  const [skipNoMips, setSkipNoMips] = useState(true);
  const [syncFlags, setSyncFlags] = useState(true);
  const [busy, setBusy] = useState(false);
  const [logLines, setLogLines] = useState<string[]>([]);
  const [statusText, setStatusText] = useState("Ready to recompress ASTC textures to BC.");

  const handleBrowseFolder = async () => {
    const picked = await pickFolderDialog({
      title: "Select Folder of Texture Banks (.dat / .wta / .wtb / .xt1 / .bntx)",
    });
    if (picked) setFolderPath(picked);
  };

  const handleRun = async () => {
    setBusy(true);
    setStatusText("Recompressing ASTC textures to BC...");
    try {
      const report = await runBatchRecompressAction({
        sourceMode,
        folderPath: sourceMode === "folder" ? folderPath : null,
        recursive,
        opaqueFormat,
        alphaFormat,
        opaqueQuality,
        alphaQuality,
        downscale,
        skipNoMips,
        syncFlags,
      });
      setLogLines(report.log_lines);
      const summary = `Converted ${report.converted} texture(s), skipped ${report.skipped}.`;
      setStatusText(summary);
      if (report.updated_outline) {
        onOutlineUpdated(report.updated_outline, summary);
      }
    } catch (err) {
      setStatusText(`Batch recompress failed: ${String(err)}`);
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="modal-backdrop" onClick={onClose}>
      <div
        className="modal-card"
        style={{ width: 980, maxWidth: "96vw", maxHeight: "92vh", display: "flex", flexDirection: "column" }}
        onClick={(e) => e.stopPropagation()}
      >
        <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", flexShrink: 0 }}>
          <div style={{ fontSize: 15, fontWeight: 600, color: "var(--brush-heading)" }}>
            Batch recompress (ASTC to BC)
          </div>
          <button
            type="button"
            className="pill-button"
            style={{ height: 26, padding: "0 8px" }}
            onClick={onClose}
          >
            ✕
          </button>
        </div>

        <div style={{ flex: 1, minHeight: 0, overflowY: "auto", display: "flex", flexDirection: "column", gap: 10 }}>
        <div style={{ display: "grid", gridTemplateColumns: "repeat(3, minmax(0, 1fr))", gap: 10, flexShrink: 0 }}>
          {/* 1. Source Selection */}
          <div className="studio-card" style={{ padding: 12 }}>
            <div style={{ fontWeight: 600, marginBottom: 8, color: "var(--brush-heading)" }}>
              1. Source Selection
            </div>
            <div style={{ display: "flex", flexDirection: "column", gap: 6, marginBottom: 10 }}>
              <StudioRadio
                checked={sourceMode === "current"}
                disabled={!hasOpenDocument}
                onChange={() => setSourceMode("current")}
              >
                <span>Current document</span>
              </StudioRadio>
              <StudioRadio
                checked={sourceMode === "folder"}
                onChange={() => setSourceMode("folder")}
              >
                <span>Folder (.dat / .wta / .wtb / .xt1 / .bntx)</span>
              </StudioRadio>
            </div>

            <div style={{ fontSize: 11.5, color: "var(--brush-muted)", marginBottom: 4 }}>
              Folder Path
            </div>
            <div style={{ display: "flex", gap: 6, marginBottom: 8 }}>
              <input
                type="text"
                className="field-box mono"
                style={{ flex: 1, fontSize: 11.5 }}
                disabled={sourceMode !== "folder"}
                value={folderPath}
                onChange={(e) => setFolderPath(e.target.value)}
              />
              <button
                type="button"
                className="pill-button"
                disabled={sourceMode !== "folder"}
                onClick={handleBrowseFolder}
              >
                <IconFolder />
              </button>
            </div>

            <div
              className={`pill-checkbox ${recursive ? "checked" : ""}`}
              style={{ marginBottom: 8, opacity: sourceMode === "folder" ? 1 : 0.5 }}
              onClick={() => sourceMode === "folder" && setRecursive(!recursive)}
            >
              <span className="pill-checkbox-box">{recursive && <IconCheck />}</span>
              <span>Include subfolders (recursive)</span>
            </div>

            <div style={{ fontSize: 11, color: "var(--brush-warn)" }}>
              Folder mode rewrites the originals in place; there is no backup.
            </div>
          </div>

          {/* 2. Output Formats */}
          <div className="studio-card" style={{ padding: 12 }}>
            <div style={{ fontWeight: 600, marginBottom: 8, color: "var(--brush-heading)" }}>
              2. Output Formats
            </div>
            <div style={{ fontSize: 11.5, color: "var(--brush-muted)", marginBottom: 4 }}>
              Opaque Texture Format
            </div>
            <StudioSelect
              className="pill-select mono"
              style={{ width: "100%", marginBottom: 12 }}
              value={opaqueFormat}
              options={[
                { value: "BC1", label: "BC1 (DXT1 — 4 bpp, auto sRGB/UNORM)" },
                { value: "BC7", label: "BC7 (8 bpp, auto sRGB/UNORM)" },
              ]}
              onChange={(val) => setOpaqueFormat(val)}
            />

            <div style={{ fontSize: 11.5, color: "var(--brush-muted)", marginBottom: 4 }}>
              Alpha / Transparent Format
            </div>
            <StudioSelect
              className="pill-select mono"
              style={{ width: "100%", marginBottom: 10 }}
              value={alphaFormat}
              options={[
                { value: "BC3", label: "BC3 (DXT5 — 8 bpp, auto sRGB/UNORM)" },
                { value: "BC7", label: "BC7 (8 bpp, auto sRGB/UNORM)" },
              ]}
              onChange={(val) => setAlphaFormat(val)}
            />

            <div style={{ fontSize: 11, color: "var(--brush-muted)", lineHeight: 1.4 }}>
              Supports both <span className="mono">.xt1</span> banks and Switch{" "}
              <span className="mono">.bntx</span> containers (relocating{" "}
              <span className="mono">BRTD</span> &amp; <span className="mono">_RLT</span> automatically).
            </div>
          </div>

          {/* 3. Quality & Options */}
          <div className="studio-card" style={{ padding: 12 }}>
            <div style={{ fontWeight: 600, marginBottom: 8, color: "var(--brush-heading)" }}>
              3. Quality &amp; Options
            </div>
            <div style={{ fontSize: 11.5, color: "var(--brush-muted)", marginBottom: 4 }}>
              Opaque Quality
            </div>
            <StudioSelect
              className="pill-select"
              style={{ width: "100%", marginBottom: 8 }}
              value={opaqueQuality}
              options={ENCODE_QUALITY_OPTIONS}
              onChange={(val) => setOpaqueQuality(val)}
            />

            <div style={{ fontSize: 11.5, color: "var(--brush-muted)", marginBottom: 4 }}>
              Alpha Quality
            </div>
            <StudioSelect
              className="pill-select"
              style={{ width: "100%", marginBottom: 10 }}
              value={alphaQuality}
              options={ENCODE_QUALITY_OPTIONS}
              onChange={(val) => setAlphaQuality(val)}
            />

            <div
              className={`pill-checkbox ${downscale ? "checked" : ""}`}
              style={{ marginBottom: 6 }}
              onClick={() => setDownscale(!downscale)}
            >
              <span className="pill-checkbox-box">{downscale && <IconCheck />}</span>
              <span>Downscale by half (drop mip 0)</span>
            </div>

            <div
              className={`pill-checkbox ${skipNoMips ? "checked" : ""}`}
              style={{ marginBottom: 6, opacity: downscale ? 1 : 0.5 }}
              onClick={() => downscale && setSkipNoMips(!skipNoMips)}
            >
              <span className="pill-checkbox-box">{skipNoMips && <IconCheck />}</span>
              <span>Do not downscale single-mip images</span>
            </div>

            <div
              className={`pill-checkbox ${syncFlags ? "checked" : ""}`}
              onClick={() => setSyncFlags(!syncFlags)}
              data-tooltip="Only applied to Bayonetta 3 / Astral Chain (SwitchXt1) banks"
            >
              <span className="pill-checkbox-box">{syncFlags && <IconCheck />}</span>
              <span>Sync WTB sRGB / cubemap flags</span>
            </div>
          </div>
        </div>

        {/* File Processing List Log */}
        <div
          className="studio-card"
          style={{ padding: 12, flex: "1 0 140px", minHeight: 140, display: "flex", flexDirection: "column" }}
        >
          <div style={{ fontWeight: 600, marginBottom: 6, color: "var(--brush-heading)" }}>
            File Processing List
          </div>
          <pre
            className="mono"
            style={{
              flex: 1,
              margin: 0,
              padding: 10,
              borderRadius: 8,
              backgroundColor: "var(--brush-window)",
              border: "1px solid var(--brush-border)",
              overflow: "auto",
              fontSize: 11.5,
              whiteSpace: "pre-wrap",
            }}
          >
            {logLines.length > 0
              ? logLines.join("\n")
              : "Click Run to start batch recompression."}
          </pre>
        </div>
        </div>

        <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", flexShrink: 0, gap: 10 }}>
          <span style={{ fontSize: 11.5, color: "var(--brush-muted)" }}>{statusText}</span>
          <div style={{ display: "flex", gap: 8, flexShrink: 0 }}>
            <button
              type="button"
              className="pill-button accent"
              disabled={busy || (sourceMode === "folder" && !folderPath.trim())}
              onClick={handleRun}
            >
              {busy ? "Running..." : "Run"}
            </button>
            <button type="button" className="pill-button" onClick={onClose}>
              Close
            </button>
          </div>
        </div>
      </div>
    </div>
  );
};

// ============================================================================
// 3. EXPORT TEXTURES FROM FOLDER MODAL (Ported from ExportTexturesDialog.xaml)
// ============================================================================
