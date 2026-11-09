import { useCallback, useState } from "react";
import { PkzSortCol } from "../types";

export interface DraggedBatchInfo {
  isWmb3: boolean;
  batchIdx: number;
  fromMeshIdx: number;
}

export interface MeshUiApi {
  expandedMeshGroups: Record<string, boolean>;
  setExpandedMeshGroups: React.Dispatch<
    React.SetStateAction<Record<string, boolean>>
  >;
  expandedBatchLods: Record<string, boolean>;
  setExpandedBatchLods: React.Dispatch<
    React.SetStateAction<Record<string, boolean>>
  >;
  draggedBatchInfo: DraggedBatchInfo | null;
  setDraggedBatchInfo: React.Dispatch<
    React.SetStateAction<DraggedBatchInfo | null>
  >;
  dragOverMeshIdx: number | null;
  setDragOverMeshIdx: React.Dispatch<React.SetStateAction<number | null>>;
  pkzBulkCodec: string;
  setPkzBulkCodec: React.Dispatch<React.SetStateAction<string>>;
  pkzBulkLevel: number;
  setPkzBulkLevel: React.Dispatch<React.SetStateAction<number>>;
  pkzSortCol: PkzSortCol;
  setPkzSortCol: React.Dispatch<React.SetStateAction<PkzSortCol>>;
  pkzSortAsc: boolean;
  setPkzSortAsc: React.Dispatch<React.SetStateAction<boolean>>;
  pkzSelectedIndices: number[];
  setPkzSelectedIndices: React.Dispatch<React.SetStateAction<number[]>>;
  pkzAnchorIndex: number | null;
  setPkzAnchorIndex: React.Dispatch<React.SetStateAction<number | null>>;
  togglePkzSort: (col: PkzSortCol) => void;
}

/**
 * WMB structured-editor mesh/batch tree UI state + PKZ table sort/selection state.
 */
export function useMeshUi(): MeshUiApi {
  const [expandedMeshGroups, setExpandedMeshGroups] = useState<
    Record<string, boolean>
  >({});
  const [expandedBatchLods, setExpandedBatchLods] = useState<
    Record<string, boolean>
  >({});
  const [draggedBatchInfo, setDraggedBatchInfo] =
    useState<DraggedBatchInfo | null>(null);
  const [dragOverMeshIdx, setDragOverMeshIdx] = useState<number | null>(null);
  const [pkzBulkCodec, setPkzBulkCodec] = useState("ZStandard");
  const [pkzBulkLevel, setPkzBulkLevel] = useState(3);
  const [pkzSortCol, setPkzSortCol] = useState<PkzSortCol>("pkz_order");
  const [pkzSortAsc, setPkzSortAsc] = useState<boolean>(true);
  const [pkzSelectedIndices, setPkzSelectedIndices] = useState<number[]>([]);
  const [pkzAnchorIndex, setPkzAnchorIndex] = useState<number | null>(null);

  const togglePkzSort = useCallback(
    (col: PkzSortCol) => {
      if (col === "pkz_order") {
        setPkzSortCol("pkz_order");
        setPkzSortAsc(true);
        return;
      }
      if (pkzSortCol === col) {
        if (!pkzSortAsc && col === "name") {
          // Clicking Filename 3 times returns to PKZ order (as stored), matching PKZ Explorer
          setPkzSortCol("pkz_order");
          setPkzSortAsc(true);
        } else {
          setPkzSortAsc((prev) => !prev);
        }
      } else {
        setPkzSortCol(col);
        setPkzSortAsc(true);
      }
    },
    [pkzSortCol, pkzSortAsc]
  );

  return {
    expandedMeshGroups,
    setExpandedMeshGroups,
    expandedBatchLods,
    setExpandedBatchLods,
    draggedBatchInfo,
    setDraggedBatchInfo,
    dragOverMeshIdx,
    setDragOverMeshIdx,
    pkzBulkCodec,
    setPkzBulkCodec,
    pkzBulkLevel,
    setPkzBulkLevel,
    pkzSortCol,
    setPkzSortCol,
    pkzSortAsc,
    setPkzSortAsc,
    pkzSelectedIndices,
    setPkzSelectedIndices,
    pkzAnchorIndex,
    setPkzAnchorIndex,
    togglePkzSort,
  };
}
