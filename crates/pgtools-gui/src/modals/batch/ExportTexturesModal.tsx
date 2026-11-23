import React, { useState } from "react";
import { pickFolderDialog, runFolderTextureExport } from "../../api/tauri";
import { IconCheck, IconFolder } from "../../components/Icons";
import { StudioRadio } from "../../components/StudioRadio";
import { StudioSelect } from "../../components/StudioSelect";

interface ExportTexturesModalProps {
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
    const picked = await pickFolderDialog({
      title: "Select Source Folder of Texture Banks",
    });
    if (picked) setSourceFolder(picked);
  };

  const handleBrowseOutput = async () => {
    const picked = await pickFolderDialog({
      title: "Select Destination Folder for Exported Textures",
    });
    if (picked) setOutputFolder(picked);
  };

  const handleExport = async () => {
    if (!sourceFolder.trim() || !outputFolder.trim()) return;
    setBusy(true);
    setStatusText("Exporting textures from folder...");
    try {
      const report = await runFolderTextureExport({
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
        <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", flexShrink: 0 }}>
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

        <div style={{ flex: 1, minHeight: 0, overflowY: "auto", display: "flex", flexDirection: "column", gap: 10 }}>
        <div style={{ display: "grid", gridTemplateColumns: "repeat(3, minmax(0, 1fr))", gap: 10, flexShrink: 0 }}>
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
              <StudioRadio
                checked={!subfolderPerBank}
                onChange={() => setSubfolderPerBank(false)}
              >
                <span>One folder for everything</span>
              </StudioRadio>
              <StudioRadio
                checked={subfolderPerBank}
                onChange={() => setSubfolderPerBank(true)}
              >
                <span>Subfolder per bank</span>
              </StudioRadio>
            </div>

            <div
              className={`pill-checkbox ${includeBntxName ? "checked" : ""}`}
              onClick={() => setIncludeBntxName(!includeBntxName)}
              data-tooltip="Include embedded BRTI texture name in exported filenames for Switch .bntx banks"
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
            <StudioSelect
              className="pill-select mono"
              style={{ width: "100%", marginBottom: 10 }}
              value={format}
              options={[
                { value: "png", label: "PNG (.png — decoded RGBA8)" },
                { value: "dds", label: "DDS (.dds — lossless multi-mip deswizzle)" },
                { value: "raw", label: "Raw (.xt1 / .dds / .gtx / .bntx)" },
              ]}
              onChange={(val) => setFormat(val as "png" | "dds" | "raw")}
            />
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
          style={{ padding: 12, flex: "1 0 140px", minHeight: 140, display: "flex", flexDirection: "column" }}
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
        </div>

        <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", flexShrink: 0, gap: 10 }}>
          <span style={{ fontSize: 11.5, color: "var(--brush-muted)" }}>{statusText}</span>
          <div style={{ display: "flex", gap: 8, flexShrink: 0 }}>
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
