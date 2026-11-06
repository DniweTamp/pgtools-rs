import { useCallback, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { FileSortCol, FsEntry } from "../types";
import { compileGlobFilter } from "../utils/format";

export interface FileBrowserState {
  currentDir: string;
  dirInput: string;
  fsEntries: FsEntry[];
  fileGlobFilter: string;
  selectedFilePath: string | null;
  selectedFilePaths: string[];
  fsClipboard: { mode: "copy" | "cut"; paths: string[] } | null;
  filteredFiles: FsEntry[];
  currentDirRef: React.MutableRefObject<string>;
}

export interface FileBrowserApi extends FileBrowserState {
  setDirInput: (s: string) => void;
  setFileGlobFilter: (s: string) => void;
  setSelectedFilePath: (p: string | null) => void;
  setSelectedFilePaths: (p: string[] | ((prev: string[]) => string[])) => void;
  setFsClipboard: (
    v: { mode: "copy" | "cut"; paths: string[] } | null
  ) => void;
  loadDirectory: (target?: string) => Promise<void>;
  toggleSort: (col: FileSortCol) => void;
  goParent: () => void;
}

export interface UseFileBrowserOptions {
  hideUnsupported: boolean;
  fileSortCol: FileSortCol;
  fileSortAsc: boolean;
  setFileSortCol: (c: FileSortCol) => void;
  setFileSortAsc: (b: boolean) => void;
  onStatus: (msg: string) => void;
}

/**
 * Owns the bottom-left file browser state that previously lived in App.
 * Sort preferences remain in StudioSettings; this hook only consumes them.
 */
export function useFileBrowser(opts: UseFileBrowserOptions): FileBrowserApi {
  const {
    hideUnsupported,
    fileSortCol,
    fileSortAsc,
    setFileSortCol,
    setFileSortAsc,
    onStatus,
  } = opts;

  const [currentDir, setCurrentDir] = useState("");
  const currentDirRef = useRef("");
  currentDirRef.current = currentDir;
  const [dirInput, setDirInput] = useState("");
  const [fsEntries, setFsEntries] = useState<FsEntry[]>([]);
  const [fileGlobFilter, setFileGlobFilter] = useState("");
  const [selectedFilePath, setSelectedFilePath] = useState<string | null>(null);
  const [selectedFilePaths, setSelectedFilePaths] = useState<string[]>([]);
  const [fsClipboard, setFsClipboard] = useState<{
    mode: "copy" | "cut";
    paths: string[];
  } | null>(null);

  const loadDirectory = useCallback(
    async (target?: string) => {
      try {
        const [resolved, list] = await invoke<[string, FsEntry[]]>("list_directory", {
          dirPath: target ?? null,
        });
        setCurrentDir(resolved);
        setDirInput(resolved);
        setFsEntries(list);
      } catch (err) {
        onStatus(`Directory error: ${String(err)}`);
      }
    },
    [onStatus]
  );

  const filteredFiles = useMemo(() => {
    const regex = compileGlobFilter(fileGlobFilter);
    const list = fsEntries.filter((e) => {
      if (hideUnsupported && !e.is_dir && !e.is_supported) return false;
      if (!regex) return true;
      return regex.test(e.name);
    });
    list.sort((a, b) => {
      if (a.is_dir !== b.is_dir) return a.is_dir ? -1 : 1;
      let cmp = 0;
      if (fileSortCol === "name") {
        cmp = a.name.localeCompare(b.name, undefined, {
          numeric: true,
          sensitivity: "base",
        });
      } else if (fileSortCol === "type") {
        cmp =
          a.extension.localeCompare(b.extension, undefined, {
            sensitivity: "base",
          }) ||
          a.name.localeCompare(b.name, undefined, {
            numeric: true,
            sensitivity: "base",
          });
      } else if (fileSortCol === "modified") {
        cmp = a.modified_epoch_ms - b.modified_epoch_ms;
      } else {
        cmp = a.size - b.size;
      }
      return fileSortAsc ? cmp : -cmp;
    });
    return list;
  }, [fsEntries, fileGlobFilter, hideUnsupported, fileSortCol, fileSortAsc]);

  const toggleSort = useCallback(
    (col: FileSortCol) => {
      if (fileSortCol === col) {
        setFileSortAsc(!fileSortAsc);
      } else {
        setFileSortCol(col);
        setFileSortAsc(true);
      }
    },
    [fileSortCol, fileSortAsc, setFileSortCol, setFileSortAsc]
  );

  const goParent = useCallback(() => {
    if (!currentDir) return;
    const up = currentDir.replace(/[\\/][^\\/]+$/, "");
    if (up && up !== currentDir) void loadDirectory(up);
  }, [currentDir, loadDirectory]);

  return {
    currentDir,
    dirInput,
    fsEntries,
    fileGlobFilter,
    selectedFilePath,
    selectedFilePaths,
    fsClipboard,
    filteredFiles,
    currentDirRef,
    setDirInput,
    setFileGlobFilter,
    setSelectedFilePath,
    setSelectedFilePaths,
    setFsClipboard,
    loadDirectory,
    toggleSort,
    goParent,
  };
}
