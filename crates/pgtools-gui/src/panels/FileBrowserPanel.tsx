import React from "react";
import { FileSortCol, FsEntry } from "../types";
import { formatBytes } from "../utils/format";
import { IconFolder } from "../components/Icons";
import { FileBrowserApi } from "../hooks/useFileBrowser";
import { ContextMenuState } from "../types";

interface FileBrowserPanelProps {
  fb: FileBrowserApi;
  hideUnsupported: boolean;
  setHideUnsupported: (b: boolean) => void;
  fileSortCol: FileSortCol;
  fileSortAsc: boolean;
  onOpenFolder: () => void;
  onOpenFile: (path: string) => void;
  onContextMenu: (e: React.MouseEvent, entry: FsEntry) => void;
  setContextMenu: (v: ContextMenuState | null) => void;
}

export function FileBrowserPanel({
  fb,
  hideUnsupported,
  setHideUnsupported,
  fileSortCol,
  fileSortAsc,
  onOpenFolder,
  onOpenFile,
  onContextMenu,
}: FileBrowserPanelProps) {
  const {
    currentDir,
    dirInput,
    setDirInput,
    fileGlobFilter,
    setFileGlobFilter,
    selectedFilePath,
    selectedFilePaths,
    setSelectedFilePath,
    setSelectedFilePaths,
    filteredFiles,
    loadDirectory,
    toggleSort,
    goParent,
  } = fb;

  return (
    <div className="studio-card" style={{ flex: "1 1 0%", minHeight: 110, display: "flex", flexDirection: "column" }}>
      <div className="pane-header">
        <span className="pane-title">File Browser</span>
        <div style={{ display: "flex", gap: 4, alignItems: "center" }}>
          <button
            className={`pill-button ${hideUnsupported ? "accent" : ""}`}
            style={{ height: 24, padding: "0 8px", fontSize: 11 }}
            onClick={() => setHideUnsupported(!hideUnsupported)}
            data-tooltip="Hide unsupported file extensions (On by default)"
          >
            Supported Only
          </button>
          <button
            className="pill-button"
            style={{ height: 24, padding: "0 8px", fontSize: 11.5 }}
            onClick={goParent}
            data-tooltip="Parent Directory"
          >
            ↑ Up
          </button>
          <button
            className="pill-button"
            style={{ height: 24, padding: "0 8px", fontSize: 11.5 }}
            onClick={onOpenFolder}
            data-tooltip="Browse Folder..."
          >
            <IconFolder />
          </button>
        </div>
      </div>

      <div className="file-browser-controls">
        <div className="file-folder-bar">
          <input
            type="text"
            className="field-box mono"
            style={{ flex: 1, height: 25, fontSize: 11 }}
            value={dirInput}
            onChange={(e) => setDirInput(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") void loadDirectory(dirInput);
            }}
            placeholder="Folder path..."
          />
        </div>
        <input
          type="text"
          className="field-box mono"
          style={{ width: "100%", height: 25, fontSize: 11.5 }}
          value={fileGlobFilter}
          onChange={(e) => setFileGlobFilter(e.target.value)}
          placeholder="Glob filter (e.g. *.dat, pl00*.wmb, *.pkz)..."
        />
      </div>

      <div className="file-table-wrap" style={{ flex: 1, minHeight: 0 }}>
        <table className="file-table">
          <thead>
            <tr>
              <th style={{ width: "42%" }} onClick={() => toggleSort("name")}>
                Name {fileSortCol === "name" ? (fileSortAsc ? "▲" : "▼") : ""}
              </th>
              <th style={{ width: "16%" }} onClick={() => toggleSort("type")}>
                Type {fileSortCol === "type" ? (fileSortAsc ? "▲" : "▼") : ""}
              </th>
              <th style={{ width: "22%" }} onClick={() => toggleSort("modified")}>
                Modified {fileSortCol === "modified" ? (fileSortAsc ? "▲" : "▼") : ""}
              </th>
              <th
                style={{ width: "20%", textAlign: "right" }}
                onClick={() => toggleSort("size")}
              >
                Size {fileSortCol === "size" ? (fileSortAsc ? "▲" : "▼") : ""}
              </th>
            </tr>
          </thead>
          <tbody>
            {filteredFiles.map((entry, idx) => {
              const isSelected =
                selectedFilePath === entry.path || selectedFilePaths.includes(entry.path);
              return (
                <tr
                  key={entry.path}
                  className={`file-row ${isSelected ? "selected" : ""}`}
                  data-tooltip={entry.path}
                  onClick={(e) => {
                    if (e.ctrlKey || e.metaKey) {
                      setSelectedFilePaths((prev) =>
                        prev.includes(entry.path)
                          ? prev.filter((p) => p !== entry.path)
                          : [...prev, entry.path]
                      );
                      setSelectedFilePath(entry.path);
                      return;
                    }
                    if (e.shiftKey && selectedFilePath) {
                      const anchorIdx = filteredFiles.findIndex(
                        (f) => f.path === selectedFilePath
                      );
                      if (anchorIdx !== -1) {
                        const lo = Math.min(anchorIdx, idx);
                        const hi = Math.max(anchorIdx, idx);
                        setSelectedFilePaths(
                          filteredFiles.slice(lo, hi + 1).map((f) => f.path)
                        );
                        return;
                      }
                    }
                    setSelectedFilePaths([entry.path]);
                    setSelectedFilePath(entry.path);
                  }}
                  onDoubleClick={() => {
                    if (entry.is_dir) {
                      void loadDirectory(entry.path);
                    } else {
                      onOpenFile(entry.path);
                    }
                  }}
                  onContextMenu={(e) => {
                    e.preventDefault();
                    e.stopPropagation();
                    if (!selectedFilePaths.includes(entry.path)) {
                      setSelectedFilePaths([entry.path]);
                    }
                    setSelectedFilePath(entry.path);
                    onContextMenu(e, entry);
                  }}
                >
                  <td>
                    {entry.is_dir ? "📁 " : ""}
                    {entry.name}
                  </td>
                  <td
                    className="mono"
                    style={{
                      color: isSelected ? "var(--brush-text)" : "var(--brush-muted)",
                    }}
                  >
                    {entry.is_dir ? "DIR" : entry.extension ? `.${entry.extension}` : "—"}
                  </td>
                  <td
                    style={{
                      color: isSelected ? "var(--brush-text)" : "var(--brush-muted)",
                    }}
                  >
                    {entry.modified_str}
                  </td>
                  <td
                    style={{
                      textAlign: "right",
                      color: isSelected ? "var(--brush-text)" : "var(--brush-muted)",
                    }}
                  >
                    {entry.is_dir ? "" : formatBytes(entry.size)}
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
      </div>
    </div>
  );
}
