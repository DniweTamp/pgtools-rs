import React, { useState } from "react";
import {
  pickFolderDialog,
  pickMultipleFilesDialog,
  runBatchDatAlignAction,
} from "../../api/tauri";
import { IconCheck, IconFolder } from "../../components/Icons";
import { StudioRadio } from "../../components/StudioRadio";
import { DAT_ALIGN_PRESETS, DEFAULT_CUSTOM_DAT_ALIGNMENT, parseCustomDatAlignmentInput } from "../../constants/datAlignment";
import { DocumentOutline } from "../../types";

interface BatchDatAlignModalProps {
  hasOpenDocument: boolean;
  defaultFolder: string;
  initialSelectedFiles?: string[];
  onClose: () => void;
  onOutlineUpdated: (outline: DocumentOutline, statusMsg: string) => void;
  onDirectoryMutated?: () => void;
}

export const BatchDatAlignModal: React.FC<BatchDatAlignModalProps> = ({
  hasOpenDocument,
  defaultFolder,
  initialSelectedFiles = [],
  onClose,
  onOutlineUpdated,
  onDirectoryMutated,
}) => {
  const [sourceMode, setSourceMode] = useState<"current" | "folder" | "files">(
    initialSelectedFiles.length > 0
      ? "files"
      : hasOpenDocument
      ? "current"
      : "folder"
  );
  const [folderPath, setFolderPath] = useState(defaultFolder);
  const [selectedFiles, setSelectedFiles] = useState<string[]>(initialSelectedFiles);
  const [recursive, setRecursive] = useState(true);
  const [pairDtt, setPairDtt] = useState(true);
  const [alignMode, setAlignMode] = useState<"custom" | "default">("custom");
  const [alignInputText, setAlignInputText] = useState<string>(
    String(DEFAULT_CUSTOM_DAT_ALIGNMENT)
  );
  const [alignNested, setAlignNested] = useState(false);
  const [destMode, setDestMode] = useState<"inplace" | "outfolder">("inplace");
  const [outputFolder, setOutputFolder] = useState("");
  const [busy, setBusy] = useState(false);
  const [logLines, setLogLines] = useState<string[]>([]);
  const [statusText, setStatusText] = useState("Ready to re-align DAT archives.");

  const parsedCustomAlign = parseCustomDatAlignmentInput(alignInputText);
  const isCustomValid = alignMode === "default" || parsedCustomAlign !== null;
  const effectiveAlignValue =
    parsedCustomAlign ?? DEFAULT_CUSTOM_DAT_ALIGNMENT;
  const hexPreview =
    parsedCustomAlign !== null
      ? `0x${parsedCustomAlign.toString(16).toUpperCase().padStart(4, "0")}`
      : "Invalid";

  const handleApplyPreset = (val: number) => {
    setAlignMode("custom");
    setAlignInputText(String(val));
  };

  const handleAlignInputChange = (raw: string) => {
    setAlignInputText(raw);
  };

  const handleBrowseFolder = async () => {
    const picked = await pickFolderDialog({
      title: "Select Folder of DAT Archives (.dat / .dtt / .evn)",
    });
    if (picked) setFolderPath(picked);
  };

  const handlePickFiles = async () => {
    const picked = await pickMultipleFilesDialog({
      title: "Select DAT Archives (.dat / .dtt / .evn) to Re-align",
      extensions: ["dat", "dtt", "evn"],
    });
    if (picked && picked.length > 0) {
      setSelectedFiles(picked);
      setSourceMode("files");
    }
  };

  const handleBrowseOutputFolder = async () => {
    const picked = await pickFolderDialog({
      title: "Select Output Folder for Re-aligned DAT Archives",
    });
    if (picked) setOutputFolder(picked);
  };

  const handleRun = async () => {
    if (!isCustomValid) return;
    setBusy(true);
    const modeDesc =
      alignMode === "default"
        ? "per-file-type default"
        : `${effectiveAlignValue} B (${hexPreview})`;
    setStatusText(`Re-aligning DAT archives (${modeDesc})...`);
    try {
      const report = await runBatchDatAlignAction({
        sourceMode,
        folderPath: sourceMode === "folder" ? folderPath : null,
        filePaths: sourceMode === "files" ? selectedFiles : null,
        recursive,
        pairDtt,
        customAlign: alignMode === "custom" ? effectiveAlignValue : null,
        perTypeDefault: alignMode === "default",
        alignNested,
        outputFolder:
          sourceMode !== "current" && destMode === "outfolder" ? outputFolder : null,
      });
      setLogLines(report.log_lines);
      const summary = `Re-aligned ${report.converted} archive(s), skipped ${report.skipped}.`;
      setStatusText(summary);
      if (report.updated_outline) {
        onOutlineUpdated(report.updated_outline, summary);
      }
      if (sourceMode !== "current" && onDirectoryMutated) {
        onDirectoryMutated();
      }
    } catch (err) {
      setStatusText(`Batch DAT align failed: ${String(err)}`);
    } finally {
      setBusy(false);
    }
  };

  const normFolder = folderPath.trim().replace(/[\\/]+$/, "").toLowerCase();
  const normOutFolder = outputFolder.trim().replace(/[\\/]+$/, "").toLowerCase();
  const isSameOutFolder =
    sourceMode === "folder" &&
    destMode === "outfolder" &&
    Boolean(normFolder) &&
    normFolder === normOutFolder;

  const canRun =
    !busy &&
    isCustomValid &&
    !isSameOutFolder &&
    (sourceMode === "current"
      ? hasOpenDocument
      : sourceMode === "folder"
      ? Boolean(folderPath.trim()) &&
        (destMode === "inplace" || Boolean(outputFolder.trim()))
      : selectedFiles.length > 0 &&
        (destMode === "inplace" || Boolean(outputFolder.trim())));

  return (
    <div className="modal-backdrop" onClick={onClose}>
      <div
        className="modal-card"
        style={{
          width: 980,
          maxWidth: "96vw",
          maxHeight: "92vh",
          display: "flex",
          flexDirection: "column",
        }}
        onClick={(e) => e.stopPropagation()}
      >
        <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", flexShrink: 0 }}>
          <div style={{ fontSize: 15, fontWeight: 600, color: "var(--brush-heading)" }}>
            Batch DAT custom align
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
        <div
          style={{
            display: "grid",
            gridTemplateColumns: "repeat(3, minmax(0, 1fr))",
            gap: 10,
            flexShrink: 0,
          }}
        >
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
                <span>Folder (.dat / .dtt / .evn)</span>
              </StudioRadio>
              <StudioRadio
                checked={sourceMode === "files"}
                onChange={() => setSourceMode("files")}
              >
                <span>Selected files ({selectedFiles.length})</span>
              </StudioRadio>
            </div>

            {sourceMode === "files" ? (
              <div style={{ marginBottom: 8 }}>
                <div style={{ fontSize: 11.5, color: "var(--brush-muted)", marginBottom: 4 }}>
                  Target Files
                </div>
                <div style={{ display: "flex", gap: 6 }}>
                  <input
                    type="text"
                    readOnly
                    className="field-box mono"
                    style={{ flex: 1, minWidth: 0, fontSize: 11.5 }}
                    value={
                      selectedFiles.length > 0
                        ? selectedFiles.map((f) => f.split(/[\\/]/).pop()).join(", ")
                        : ""
                    }
                    placeholder="Select .dat / .dtt / .evn files..."
                  />
                  <button
                    type="button"
                    className="pill-button"
                    onClick={handlePickFiles}
                    data-tooltip="Select .dat / .dtt / .evn files"
                  >
                    <IconFolder />
                  </button>
                </div>
              </div>
            ) : (
              <>
                <div style={{ fontSize: 11.5, color: "var(--brush-muted)", marginBottom: 4 }}>
                  Folder Path
                </div>
                <div style={{ display: "flex", gap: 6, marginBottom: 8 }}>
                  <input
                    type="text"
                    className="field-box mono"
                    style={{ flex: 1, minWidth: 0, fontSize: 11.5 }}
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
              </>
            )}

            <div
              className={`pill-checkbox ${recursive ? "checked" : ""}`}
              style={{ marginBottom: 6, opacity: sourceMode === "folder" ? 1 : 0.5 }}
              onClick={() => sourceMode === "folder" && setRecursive(!recursive)}
            >
              <span className="pill-checkbox-box">{recursive && <IconCheck />}</span>
              <span>Include subfolders (recursive)</span>
            </div>

            <div
              className={`pill-checkbox ${pairDtt ? "checked" : ""}`}
              onClick={() => setPairDtt(!pairDtt)}
              data-tooltip="Automatically re-align sibling .dtt archives alongside .dat archives"
            >
              <span className="pill-checkbox-box">{pairDtt && <IconCheck />}</span>
              <span>Include paired .dtt archives</span>
            </div>
          </div>

          {/* 2. Alignment */}
          <div className="studio-card" style={{ padding: 12 }}>
            <div style={{ fontWeight: 600, marginBottom: 8, color: "var(--brush-heading)" }}>
              2. Alignment
            </div>
            <div style={{ display: "flex", flexDirection: "column", gap: 6, marginBottom: 10 }}>
              <StudioRadio
                checked={alignMode === "custom"}
                onChange={() => setAlignMode("custom")}
              >
                <span>Custom alignment</span>
              </StudioRadio>
              <StudioRadio
                checked={alignMode === "default"}
                onChange={() => setAlignMode("default")}
              >
                <span>Default (per file type)</span>
              </StudioRadio>
            </div>

            <div
              style={{
                display: "flex",
                justifyContent: "space-between",
                alignItems: "center",
                marginBottom: 4,
              }}
            >
              <span style={{ fontSize: 11.5, color: "var(--brush-muted)" }}>
                Alignment (Bytes or 0x Hex)
              </span>
              <span
                className="mono"
                style={{
                  fontSize: 11.5,
                  fontWeight: 600,
                  color: isCustomValid ? "var(--brush-accent)" : "var(--brush-warning-text)",
                }}
              >
                {alignMode === "default" ? "Default" : hexPreview}
              </span>
            </div>

            <input
              type="text"
              className="field-box mono"
              style={{ width: "100%", minWidth: 0, marginBottom: 8 }}
              disabled={alignMode === "default"}
              value={alignInputText}
              onChange={(e) => handleAlignInputChange(e.target.value)}
              placeholder="2048"
              data-tooltip="Enter byte alignment in decimal (e.g. 2048) or hex (e.g. 0x800)"
            />

            <div style={{ display: "flex", flexWrap: "wrap", gap: 5 }}>
              {DAT_ALIGN_PRESETS.map((preset) => {
                const isSelected =
                  alignMode === "custom" && parsedCustomAlign === preset;
                return (
                  <button
                    key={preset}
                    type="button"
                    className={`pill-button ${isSelected ? "accent" : ""}`}
                    style={{ height: 24, padding: "0 8px", fontSize: 11 }}
                    onClick={() => handleApplyPreset(preset)}
                  >
                    {preset}
                  </button>
                );
              })}
            </div>
          </div>

          {/* 3. Destination & Options */}
          <div className="studio-card" style={{ padding: 12 }}>
            <div style={{ fontWeight: 600, marginBottom: 8, color: "var(--brush-heading)" }}>
              3. Destination &amp; Options
            </div>

            <div
              style={{
                display: "flex",
                flexDirection: "column",
                gap: 6,
                marginBottom: 10,
                opacity: sourceMode === "current" ? 0.5 : 1,
              }}
            >
              <StudioRadio
                disabled={sourceMode === "current"}
                checked={destMode === "inplace"}
                onChange={() => setDestMode("inplace")}
              >
                <span>Overwrite existing files</span>
              </StudioRadio>
              <StudioRadio
                disabled={sourceMode === "current"}
                checked={destMode === "outfolder"}
                onChange={() => setDestMode("outfolder")}
              >
                <span>Custom output folder</span>
              </StudioRadio>
            </div>

            <div style={{ fontSize: 11.5, color: "var(--brush-muted)", marginBottom: 4 }}>
              Output Folder
            </div>
            <div style={{ display: "flex", gap: 6, marginBottom: 8 }}>
              <input
                type="text"
                className="field-box mono"
                style={{ flex: 1, minWidth: 0, fontSize: 11.5 }}
                disabled={sourceMode === "current" || destMode !== "outfolder"}
                value={outputFolder}
                onChange={(e) => setOutputFolder(e.target.value)}
              />
              <button
                type="button"
                className="pill-button"
                disabled={sourceMode === "current" || destMode !== "outfolder"}
                onClick={handleBrowseOutputFolder}
              >
                <IconFolder />
              </button>
            </div>

            <div
              className={`pill-checkbox ${alignNested ? "checked" : ""}`}
              onClick={() => setAlignNested(!alignNested)}
              data-tooltip="Recursively re-align nested .dat / .dtt archives stored inside outer DAT containers"
            >
              <span className="pill-checkbox-box">{alignNested && <IconCheck />}</span>
              <span>Also re-align nested .dat / .dtt</span>
            </div>
          </div>
        </div>

        {/* File Processing List Log */}
        <div
          className="studio-card"
          style={{
            padding: 12,
            flex: "1 0 140px",
            minHeight: 140,
            display: "flex",
            flexDirection: "column",
          }}
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
              : "Click Run to start batch DAT alignment."}
          </pre>
        </div>
        </div>

        <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", flexShrink: 0, gap: 10 }}>
          <span style={{ fontSize: 11.5, color: "var(--brush-muted)" }}>{statusText}</span>
          <div style={{ display: "flex", gap: 8, flexShrink: 0 }}>
            <button
              type="button"
              className="pill-button accent"
              disabled={!canRun}
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
// 6. DAT SAVE ALIGNMENT SETTINGS MODAL (Tools -> Configure DAT save alignment...)
// ============================================================================
