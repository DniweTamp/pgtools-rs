import { useState } from "react";

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
}

/**
 * WMB structured-editor mesh/batch tree UI state (expand, drag reorder, PKZ codec).
 * Lives next to editor drafts; only the WMB structured center cares about it.
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
  };
}
