import React, { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  BatchOperationReport,
  BatchReplaceScanResult,
  DocumentOutline,
} from "../types";
import { IconCheck, IconFolder } from "../components/Icons";
import { ENCODE_QUALITY_OPTIONS } from "../editors/TextureEditor";

// ============================================================================
// 1. BATCH REPLACE FROM FOLDER MODAL (Ported from BatchReplaceDialog.xaml)
// ============================================================================

export interface BatchReplaceModalProps {
  nodeId: string;
  initialFolder?: string;
  onClose: () => void;
  onRunReplace: (
    nodeId: string,
    folder: string,
    quality: string,
    solidify: boolean,
    syncFlags: boolean
  ) => Promise<void>;
}

export const BatchReplaceModal: React.FC<BatchReplaceModalProps> = ({
  nodeId,
  initialFolder = "",
  onClose,
  onRunReplace,
}) => {
  const [folder, setFolder] = useState(initialFolder);
  const [quality, setQuality] = useState("Normal");
  const [solidify, setSolidify] = useState(true);
  const [syncFlags, setSyncFlags] = useState(true);
  const [scanResult, setScanResult] = useState<BatchReplaceScanResult | null>(
    null
  );
  const [busy, setBusy] = useState(false);
  const [statusText, setStatusText] = useState(
    "Select a folder of replacement textures to scan."
  );

  const runScan = async (targetFolder: string) => {
    if (!targetFolder.trim()) return;
    setBusy(true);
    setStatusText("Scanning folder...");
    try {
      const res = await invoke<BatchReplaceScanResult>(
        "scan_batch_replace_folder",
        {
          nodeId,
          folder: targetFolder,
        }
      );
      setScanResult(res);
      setStatusText(
        `Matched: ${res.matched} · Missing: ${res.missing} · Ambiguous: ${res.ambiguous} · Unsupported: ${res.unsupported} · Unused: ${res.unused}`
      );
    } catch (err) {
      setStatusText(`Scan failed: ${String(err)}`);
    } finally {
      setBusy(false);
    }
  };

  useEffect(() => {
    if (initialFolder) {
      void runScan(initialFolder);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [initialFolder]);

  const handleBrowse = async () => {
    const picked = await invoke<string | null>("pick_folder_dialog", {
      title: "Select Folder of Replacement Textures",
    });
    if (picked) {
      setFolder(picked);
      await runScan(picked);
    }
  };

  const handleRun = async () => {
    if (!folder || !scanResult || scanResult.matched === 0) return;
    setBusy(true);
    setStatusText(`Replacing ${scanResult.matched} matched texture(s)...`);
    try {
      await onRunReplace(nodeId, folder, quality, solidify, syncFlags);
      onClose();
    } catch (err) {
      setStatusText(`Replace failed: ${String(err)}`);
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="modal-backdrop" onClick={onClose}>
      <div
        className="modal-card"
        style={{ width: 880, maxWidth: "95vw", maxHeight: "90vh", display: "flex", flexDirection: "column" }}
        onClick={(e) => e.stopPropagation()}
      >
        <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center" }}>
          <div style={{ fontSize: 15, fontWeight: 600, color: "var(--brush-heading)" }}>
            Batch replace textures
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

        <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr", gap: 10 }}>
          {/* Source Card */}
          <div className="studio-card" style={{ padding: 12 }}>
            <div style={{ fontWeight: 600, marginBottom: 8, color: "var(--brush-heading)" }}>
              Source
            </div>
            {scanResult && (
              <div className="mono" style={{ fontSize: 11.5, marginBottom: 8, color: "var(--brush-muted)" }}>
                {scanResult.bank_label}
              </div>
            )}
            <div style={{ fontSize: 11.5, color: "var(--brush-muted)", marginBottom: 4 }}>
              Folder of replacement files
            </div>
            <div style={{ display: "flex", gap: 6, marginBottom: 8 }}>
              <input
                type="text"
                className="field-box mono"
                style={{ flex: 1, fontSize: 11.5 }}
                value={folder}
                onChange={(e) => setFolder(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === "Enter") void runScan(folder);
                }}
                placeholder="Select folder..."
              />
              <button
                type="button"
                className="pill-button"
                onClick={handleBrowse}
                title="Browse for a folder"
              >
                <IconFolder />
              </button>
            </div>
            <div style={{ fontSize: 11, color: "var(--brush-muted)", lineHeight: 1.4 }}>
              Matched by <span className="mono">tex&lt;index&gt;_0x&lt;hash&gt;</span>,{" "}
              <span className="mono">tex&lt;index&gt;</span>,{" "}
              <span className="mono">0x&lt;hash&gt;</span>, or embedded BNTX name. Priority:{" "}
              <span className="mono">.xt1 &gt; .dds &gt; image</span>. Existing CRC32 hashes and mip counts are preserved.
            </div>
          </div>

          {/* Options Card */}
          <div className="studio-card" style={{ padding: 12 }}>
            <div style={{ fontWeight: 600, marginBottom: 8, color: "var(--brush-heading)" }}>
              Options
            </div>
            <div style={{ fontSize: 11.5, color: "var(--brush-muted)", marginBottom: 4 }}>
              Texture compression quality
            </div>
            <select
              className="pill-select"
              style={{ width: "100%", marginBottom: 10 }}
              value={quality}
              onChange={(e) => setQuality(e.target.value)}
            >
              {ENCODE_QUALITY_OPTIONS.map((q) => (
                <option key={q.value} value={q.value}>
                  {q.label}
                </option>
              ))}
            </select>

            <div
              className={`pill-checkbox ${solidify ? "checked" : ""}`}
              style={{ marginBottom: 8 }}
              onClick={() => setSolidify(!solidify)}
              title="Fill fully transparent pixels from their nearest opaque neighbors before encoding"
            >
              <span className="pill-checkbox-box">{solidify && <IconCheck />}</span>
              <span>Solidify (edge-pad transparent RGB)</span>
            </div>

            {scanResult?.has_confirmed_xt1_flags && (
              <div
                className={`pill-checkbox ${syncFlags ? "checked" : ""}`}
                style={{ marginBottom: 8 }}
                onClick={() => setSyncFlags(!syncFlags)}
              >
                <span className="pill-checkbox-box">{syncFlags && <IconCheck />}</span>
                <span>Sync WTB sRGB / cubemap flags</span>
              </div>
            )}

            <div style={{ fontSize: 11, color: "var(--brush-muted)", marginTop: 4 }}>
              Applied to every re-encoded image. Each texture keeps its existing format and CRC32 hash.
            </div>
          </div>
        </div>

        {/* Scan / Log Console */}
        <div
          className="studio-card"
          style={{ padding: 12, flex: 1, minHeight: 220, display: "flex", flexDirection: "column" }}
        >
          <div style={{ fontWeight: 600, marginBottom: 6, color: "var(--brush-heading)" }}>
            Scan / Log
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
            {scanResult
              ? scanResult.log_lines.length > 0
                ? scanResult.log_lines.join("\n")
                : "(No matching files found in folder)"
              : "Browse for a folder above to preview matches."}
          </pre>
        </div>

        <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center" }}>
          <span style={{ fontSize: 11.5, color: "var(--brush-muted)" }}>{statusText}</span>
          <div style={{ display: "flex", gap: 8 }}>
            <button
              type="button"
              className="pill-button accent"
              disabled={busy || !scanResult || scanResult.matched === 0}
              onClick={handleRun}
            >
              Replace ({scanResult?.matched ?? 0})
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
// 2. BATCH RECOMPRESS (ASTC TO BC) MODAL (Ported from BatchRecompressDialog.xaml)
// ============================================================================

export interface BatchRecompressModalProps {
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
    const picked = await invoke<string | null>("pick_folder_dialog", {
      title: "Select Folder of Texture Banks (.dat / .wta / .wtb / .xt1 / .bntx)",
    });
    if (picked) setFolderPath(picked);
  };

  const handleRun = async () => {
    setBusy(true);
    setStatusText("Recompressing ASTC textures to BC...");
    try {
      const report = await invoke<BatchOperationReport>("run_batch_recompress_action", {
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
        <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center" }}>
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

        <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr 1fr", gap: 10 }}>
          {/* 1. Source Selection */}
          <div className="studio-card" style={{ padding: 12 }}>
            <div style={{ fontWeight: 600, marginBottom: 8, color: "var(--brush-heading)" }}>
              1. Source Selection
            </div>
            <div style={{ display: "flex", flexDirection: "column", gap: 6, marginBottom: 10 }}>
              <label style={{ display: "flex", alignItems: "center", gap: 6, cursor: "pointer" }}>
                <input
                  type="radio"
                  checked={sourceMode === "current"}
                  disabled={!hasOpenDocument}
                  onChange={() => setSourceMode("current")}
                />
                <span>Current document</span>
              </label>
              <label style={{ display: "flex", alignItems: "center", gap: 6, cursor: "pointer" }}>
                <input
                  type="radio"
                  checked={sourceMode === "folder"}
                  onChange={() => setSourceMode("folder")}
                />
                <span>Folder (.dat / .wta / .wtb / .xt1 / .bntx)</span>
              </label>
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
            <select
              className="pill-select mono"
              style={{ width: "100%", marginBottom: 12 }}
              value={opaqueFormat}
              onChange={(e) => setOpaqueFormat(e.target.value)}
            >
              <option value="BC1">BC1 (DXT1 — 4 bpp, auto sRGB/UNORM)</option>
              <option value="BC7">BC7 (8 bpp, auto sRGB/UNORM)</option>
            </select>

            <div style={{ fontSize: 11.5, color: "var(--brush-muted)", marginBottom: 4 }}>
              Alpha / Transparent Format
            </div>
            <select
              className="pill-select mono"
              style={{ width: "100%", marginBottom: 10 }}
              value={alphaFormat}
              onChange={(e) => setAlphaFormat(e.target.value)}
            >
              <option value="BC3">BC3 (DXT5 — 8 bpp, auto sRGB/UNORM)</option>
              <option value="BC7">BC7 (8 bpp, auto sRGB/UNORM)</option>
            </select>

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
            <select
              className="pill-select"
              style={{ width: "100%", marginBottom: 8 }}
              value={opaqueQuality}
              onChange={(e) => setOpaqueQuality(e.target.value)}
            >
              {ENCODE_QUALITY_OPTIONS.map((q) => (
                <option key={q.value} value={q.value}>
                  {q.label}
                </option>
              ))}
            </select>

            <div style={{ fontSize: 11.5, color: "var(--brush-muted)", marginBottom: 4 }}>
              Alpha Quality
            </div>
            <select
              className="pill-select"
              style={{ width: "100%", marginBottom: 10 }}
              value={alphaQuality}
              onChange={(e) => setAlphaQuality(e.target.value)}
            >
              {ENCODE_QUALITY_OPTIONS.map((q) => (
                <option key={q.value} value={q.value}>
                  {q.label}
                </option>
              ))}
            </select>

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
              title="Only applied to Bayonetta 3 / Astral Chain (SwitchXt1) banks"
            >
              <span className="pill-checkbox-box">{syncFlags && <IconCheck />}</span>
              <span>Sync WTB sRGB / cubemap flags</span>
            </div>
          </div>
        </div>

        {/* File Processing List Log */}
        <div
          className="studio-card"
          style={{ padding: 12, flex: 1, minHeight: 220, display: "flex", flexDirection: "column" }}
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

        <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center" }}>
          <span style={{ fontSize: 11.5, color: "var(--brush-muted)" }}>{statusText}</span>
          <div style={{ display: "flex", gap: 8 }}>
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

export interface ExportTexturesModalProps {
  defaultSourceFolder: string;
  onClose: () => void;
}

export const ExportTexturesModal: React.FC<ExportTexturesModalProps> = ({
  defaultSourceFolder,
  onClose,
}) => {
  const [sourceFolder, setSourceFolder] = useState(defaultSourceFolder);
  const [recursive, setRecursive] = useState(true);
  const [outputFolder, setOutputFolder] = useState(defaultSourceFolder);
  const [subfolderPerBank, setSubfolderPerBank] = useState(true);
  const [format, setFormat] = useState<"png" | "dds" | "raw">("png");
  const [includeBntxName, setIncludeBntxName] = useState(false);
  const [busy, setBusy] = useState(false);
  const [logLines, setLogLines] = useState<string[]>([]);
  const [statusText, setStatusText] = useState("Select source and destination folders to export.");

  const handleBrowseSource = async () => {
    const picked = await invoke<string | null>("pick_folder_dialog", {
      title: "Select Source Folder of Texture Banks",
    });
    if (picked) setSourceFolder(picked);
  };

  const handleBrowseOutput = async () => {
    const picked = await invoke<string | null>("pick_folder_dialog", {
      title: "Select Destination Folder for Exported Textures",
    });
    if (picked) setOutputFolder(picked);
  };

  const handleExport = async () => {
    if (!sourceFolder.trim() || !outputFolder.trim()) return;
    setBusy(true);
    setStatusText("Exporting textures from folder...");
    try {
      const report = await invoke<BatchOperationReport>("run_folder_texture_export", {
        sourceFolder,
        recursive,
        outputFolder,
        subfolderPerBank,
        format,
        includeBntxName,
      });
      setLogLines(report.log_lines);
      setStatusText(
        `Scanned ${report.files_processed} file(s), exported ${report.converted} texture(s), skipped ${report.skipped}.`
      );
    } catch (err) {
      setStatusText(`Export failed: ${String(err)}`);
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
        <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center" }}>
          <div style={{ fontSize: 15, fontWeight: 600, color: "var(--brush-heading)" }}>
            Export textures from folder
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

        <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr 1fr", gap: 10 }}>
          {/* 1. Source */}
          <div className="studio-card" style={{ padding: 12 }}>
            <div style={{ fontWeight: 600, marginBottom: 8, color: "var(--brush-heading)" }}>
              1. Source
            </div>
            <div style={{ fontSize: 11.5, color: "var(--brush-muted)", marginBottom: 4 }}>
              Source Folder
            </div>
            <div style={{ display: "flex", gap: 6, marginBottom: 8 }}>
              <input
                type="text"
                className="field-box mono"
                style={{ flex: 1, fontSize: 11.5 }}
                value={sourceFolder}
                onChange={(e) => setSourceFolder(e.target.value)}
              />
              <button type="button" className="pill-button" onClick={handleBrowseSource}>
                <IconFolder />
              </button>
            </div>
            <div
              className={`pill-checkbox ${recursive ? "checked" : ""}`}
              style={{ marginBottom: 8 }}
              onClick={() => setRecursive(!recursive)}
            >
              <span className="pill-checkbox-box">{recursive && <IconCheck />}</span>
              <span>Include subfolders (recursive)</span>
            </div>
            <div style={{ fontSize: 11, color: "var(--brush-muted)", lineHeight: 1.4 }}>
              Scans <span className="mono">.dat/.wta/.wtb/.xt1/.bntx/.gtx/.dds</span>, pairing each bank&apos;s{" "}
              <span className="mono">.dtt/.wtp</span> payload automatically.
            </div>
          </div>

          {/* 2. Destination */}
          <div className="studio-card" style={{ padding: 12 }}>
            <div style={{ fontWeight: 600, marginBottom: 8, color: "var(--brush-heading)" }}>
              2. Destination
            </div>
            <div style={{ fontSize: 11.5, color: "var(--brush-muted)", marginBottom: 4 }}>
              Output Folder
            </div>
            <div style={{ display: "flex", gap: 6, marginBottom: 10 }}>
              <input
                type="text"
                className="field-box mono"
                style={{ flex: 1, fontSize: 11.5 }}
                value={outputFolder}
                onChange={(e) => setOutputFolder(e.target.value)}
              />
              <button type="button" className="pill-button" onClick={handleBrowseOutput}>
                <IconFolder />
              </button>
            </div>

            <div style={{ display: "flex", flexDirection: "column", gap: 6, marginBottom: 8 }}>
              <label style={{ display: "flex", alignItems: "center", gap: 6, cursor: "pointer" }}>
                <input
                  type="radio"
                  checked={!subfolderPerBank}
                  onChange={() => setSubfolderPerBank(false)}
                />
                <span>One folder for everything</span>
              </label>
              <label style={{ display: "flex", alignItems: "center", gap: 6, cursor: "pointer" }}>
                <input
                  type="radio"
                  checked={subfolderPerBank}
                  onChange={() => setSubfolderPerBank(true)}
                />
                <span>Subfolder per bank</span>
              </label>
            </div>

            <div
              className={`pill-checkbox ${includeBntxName ? "checked" : ""}`}
              onClick={() => setIncludeBntxName(!includeBntxName)}
              title="Include embedded BRTI texture name in exported filenames for Switch .bntx banks"
            >
              <span className="pill-checkbox-box">{includeBntxName && <IconCheck />}</span>
              <span>Include BNTX name in filename</span>
            </div>
          </div>

          {/* 3. Format */}
          <div className="studio-card" style={{ padding: 12 }}>
            <div style={{ fontWeight: 600, marginBottom: 8, color: "var(--brush-heading)" }}>
              3. Format
            </div>
            <div style={{ fontSize: 11.5, color: "var(--brush-muted)", marginBottom: 4 }}>
              File Format
            </div>
            <select
              className="pill-select mono"
              style={{ width: "100%", marginBottom: 10 }}
              value={format}
              onChange={(e) => setFormat(e.target.value as "png" | "dds" | "raw")}
            >
              <option value="png">PNG (.png — decoded RGBA8)</option>
              <option value="dds">DDS (.dds — lossless multi-mip deswizzle)</option>
              <option value="raw">Raw (.xt1 / .dds / .gtx / .bntx)</option>
            </select>
            <div style={{ fontSize: 11, color: "var(--brush-muted)", lineHeight: 1.4 }}>
              {format === "png"
                ? "Decodes mip 0 (base level) of each texture to a lossless 32-bit PNG."
                : format === "dds"
                ? "Deswizzles all stored mip levels and cubemap faces directly into a DX9/DX10 .dds without re-compressing."
                : "Writes each texture in its native stored container format (.xt1, .dds, .gtx, or .bntx)."}
            </div>
          </div>
        </div>

        {/* Export Log */}
        <div
          className="studio-card"
          style={{ padding: 12, flex: 1, minHeight: 220, display: "flex", flexDirection: "column" }}
        >
          <div style={{ fontWeight: 600, marginBottom: 6, color: "var(--brush-heading)" }}>
            Export Log
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
              : "Click Export to scan and extract textures."}
          </pre>
        </div>

        <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center" }}>
          <span style={{ fontSize: 11.5, color: "var(--brush-muted)" }}>{statusText}</span>
          <div style={{ display: "flex", gap: 8 }}>
            <button
              type="button"
              className="pill-button accent"
              disabled={busy || !sourceFolder.trim() || !outputFolder.trim()}
              onClick={handleExport}
            >
              {busy ? "Exporting..." : "Export"}
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
// 4. NEW EMPTY BANK MODAL (Ported from NewBankDialog.xaml)
// ============================================================================

export interface NewBankModalProps {
  onClose: () => void;
  onCreated: (outline: DocumentOutline) => void;
  onStatus: (msg: string) => void;
}

export const NewBankModal: React.FC<NewBankModalProps> = ({
  onClose,
  onCreated,
  onStatus,
}) => {
  const [isSplit, setIsSplit] = useState(true);
  const [gameProfile, setGameProfile] = useState("switch_xt1");

  const handleCreate = async () => {
    const ext = isSplit ? "wta" : "wtb";
    const dest = await invoke<string | null>("save_file_dialog", {
      title: isSplit ? "Create Empty Split Bank (.wta + .wtp)" : "Create Empty Standalone Bank (.wtb)",
      defaultName: `new_bank.${ext}`,
    });
    if (!dest) return;
    try {
      const outline = await invoke<DocumentOutline>("create_empty_bank_document", {
        outputPath: dest,
        isSplitWtaWtp: isSplit,
        gameProfile,
      });
      onCreated(outline);
      onStatus(`Created new empty bank: ${outline.file_name}`);
      onClose();
    } catch (err) {
      onStatus(`Failed to create bank: ${String(err)}`);
    }
  };

  return (
    <div className="modal-backdrop" onClick={onClose}>
      <div
        className="modal-card"
        style={{ width: 470, maxWidth: "94vw" }}
        onClick={(e) => e.stopPropagation()}
      >
        <div style={{ fontSize: 15, fontWeight: 600, color: "var(--brush-heading)" }}>
          New empty bank
        </div>

        <div style={{ display: "flex", flexDirection: "column", gap: 10, marginTop: 4 }}>
          <label style={{ display: "flex", alignItems: "flex-start", gap: 8, cursor: "pointer" }}>
            <input
              type="radio"
              checked={isSplit}
              onChange={() => setIsSplit(true)}
              style={{ marginTop: 3 }}
            />
            <div>
              <div style={{ fontWeight: 600 }}>Split bank (.wta + .wtp)</div>
              <div style={{ fontSize: 11.5, color: "var(--brush-muted)", marginTop: 2 }}>
                A header file plus a separate payload stream (used for models, UI, and effects).
              </div>
            </div>
          </label>

          <label style={{ display: "flex", alignItems: "flex-start", gap: 8, cursor: "pointer" }}>
            <input
              type="radio"
              checked={!isSplit}
              onChange={() => setIsSplit(false)}
              style={{ marginTop: 3 }}
            />
            <div>
              <div style={{ fontWeight: 600 }}>Standalone bank (.wtb)</div>
              <div style={{ fontSize: 11.5, color: "var(--brush-muted)", marginTop: 2 }}>
                A single file with the payload inline.
              </div>
            </div>
          </label>

          <div className="inspector-row" style={{ marginTop: 6 }}>
            <span className="inspector-row-label">Game Profile</span>
            <select
              className="pill-select"
              style={{ width: "100%" }}
              value={gameProfile}
              onChange={(e) => setGameProfile(e.target.value)}
            >
              <option value="switch_xt1">Bayonetta 3 / Astral Chain (Switch XT1)</option>
              <option value="bayo1">Bayonetta 1 / Vanquish / MGR:R (PC DDS)</option>
              <option value="nier">NieR:Automata (PC DDS)</option>
              <option value="bayo2_wiiu">Bayonetta 2 (Wii U GTX)</option>
            </select>
          </div>
        </div>

        <div style={{ display: "flex", justifyContent: "flex-end", gap: 8, marginTop: 10 }}>
          <button type="button" className="pill-button accent" onClick={handleCreate}>
            Create...
          </button>
          <button type="button" className="pill-button" onClick={onClose}>
            Cancel
          </button>
        </div>
      </div>
    </div>
  );
};
