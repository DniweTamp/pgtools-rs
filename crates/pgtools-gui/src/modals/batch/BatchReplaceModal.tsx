import React, { useEffect, useState } from "react";
import { pickFolderDialog, scanBatchReplaceFolder } from "../../api/tauri";
import { IconCheck, IconFolder } from "../../components/Icons";
import { StudioSelect } from "../../components/StudioSelect";
import { ENCODE_QUALITY_OPTIONS } from "../../constants/textureFormats";
import { BatchReplaceScanResult } from "../../types";

interface BatchReplaceModalProps {
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
      const res = await scanBatchReplaceFolder({
        nodeId,
        folder: targetFolder,
      });
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
    const picked = await pickFolderDialog({
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
        <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", flexShrink: 0 }}>
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

        <div style={{ flex: 1, minHeight: 0, overflowY: "auto", display: "flex", flexDirection: "column", gap: 10 }}>
        <div style={{ display: "grid", gridTemplateColumns: "repeat(2, minmax(0, 1fr))", gap: 10, flexShrink: 0 }}>
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
                data-tooltip="Browse for a folder"
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
            <StudioSelect
              className="pill-select"
              style={{ width: "100%", marginBottom: 10 }}
              value={quality}
              options={ENCODE_QUALITY_OPTIONS}
              onChange={(val) => setQuality(val)}
            />

            <div
              className={`pill-checkbox ${solidify ? "checked" : ""}`}
              style={{ marginBottom: 8 }}
              onClick={() => setSolidify(!solidify)}
              data-tooltip="Fill fully transparent pixels from their nearest opaque neighbors before encoding"
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
          style={{ padding: 12, flex: "1 0 140px", minHeight: 140, display: "flex", flexDirection: "column" }}
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
        </div>

        <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", flexShrink: 0, gap: 10 }}>
          <span style={{ fontSize: 11.5, color: "var(--brush-muted)" }}>{statusText}</span>
          <div style={{ display: "flex", gap: 8, flexShrink: 0 }}>
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
