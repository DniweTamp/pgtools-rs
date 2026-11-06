import React from "react";
import { AssetTreeNode, PointerDragState } from "../types";
import { isVirtualSubNode } from "../utils/format";

export interface AssetBrowserPanelProps {
  outline: { tree: AssetTreeNode[]; file_name?: string } | null;
  expandedNodes: Record<string, boolean>;
  selectedNodeId: string;
  assetSearch: string;
  pointerDrag: PointerDragState | null;
  assetRatio: number;
  onSearchChange: (s: string) => void;
  onToggleExpand: (nodeId: string) => void;
  onSelectNode: (nodeId: string) => void;
  onContextMenu: (e: React.MouseEvent, nodeId: string, kind: string) => void;
  onPointerDownNode: (e: React.PointerEvent, node: AssetTreeNode) => void;
  cardRef?: React.RefObject<HTMLDivElement | null>;
}

export function AssetBrowserPanel({
  outline,
  expandedNodes,
  selectedNodeId,
  assetSearch,
  pointerDrag,
  assetRatio,
  onSearchChange,
  onToggleExpand,
  onSelectNode,
  onContextMenu,
  onPointerDownNode,
  cardRef,
}: AssetBrowserPanelProps) {
  function renderTreeNodes(nodes: AssetTreeNode[]): React.ReactNode {
    const filterLow = assetSearch.trim().toLowerCase();
    return nodes.map((node) => {
      const hasChildren = node.children && node.children.length > 0;
      const isExpanded = !!expandedNodes[node.id];
      const isSelected = selectedNodeId === node.id;
      const canDragNode = node.id !== "root" && !isVirtualSubNode(node.id);
      const isDragOverBefore =
        pointerDrag?.active &&
        pointerDrag.kind === "asset" &&
        pointerDrag.overId === node.id &&
        pointerDrag.insertPos === "before";
      const isDragOverAfter =
        pointerDrag?.active &&
        pointerDrag.kind === "asset" &&
        pointerDrag.overId === node.id &&
        pointerDrag.insertPos === "after";

      const matchesFilter = (n: AssetTreeNode): boolean => {
        if (!filterLow) return true;
        if (
          n.label.toLowerCase().includes(filterLow) ||
          n.format_badge.toLowerCase().includes(filterLow)
        ) {
          return true;
        }
        return n.children?.some(matchesFilter) ?? false;
      };
      if (!matchesFilter(node)) return null;

      return (
        <div key={node.id}>
          <div
            data-asset-node-id={node.id}
            className={`tree-row ${isSelected ? "selected" : ""} ${
              isDragOverBefore ? "drag-insert-before" : ""
            } ${isDragOverAfter ? "drag-insert-after" : ""}`}
            onPointerDown={(e) => {
              if (e.button !== 0 || !canDragNode) return;
              onPointerDownNode(e, node);
            }}
            onClick={() => {
              if (hasChildren && !expandedNodes[node.id]) {
                onToggleExpand(node.id);
              }
              onSelectNode(node.id);
            }}
            onContextMenu={(e) => {
              e.preventDefault();
              e.stopPropagation();
              onSelectNode(node.id);
              onContextMenu(e, node.id, node.kind);
            }}
          >
            <span
              className={`tree-arrow ${isExpanded ? "expanded" : ""}`}
              style={{ visibility: hasChildren ? "visible" : "hidden" }}
              onClick={(e) => {
                e.stopPropagation();
                onToggleExpand(node.id);
              }}
            >
              <svg
                width="6"
                height="9"
                viewBox="0 0 6 10"
                fill="none"
                stroke="currentColor"
                strokeWidth="1.5"
              >
                <path d="M1 1L5 5L1 9" strokeLinecap="round" strokeLinejoin="round" />
              </svg>
            </span>
            <span className="tree-badge">{node.format_badge}</span>
            <span style={{ overflow: "hidden", textOverflow: "ellipsis" }}>{node.label}</span>
          </div>
          {hasChildren && (isExpanded || !!filterLow) && (
            <div className="tree-children">{renderTreeNodes(node.children)}</div>
          )}
        </div>
      );
    });
  }

  return (
    <div
      ref={cardRef as React.RefObject<HTMLDivElement>}
      className="studio-card"
      style={{ flex: `${assetRatio} 1 0%`, minHeight: 110 }}
    >
      <div className="pane-header">
        <span className="pane-title">Asset Browser</span>
        <input
          type="text"
          className="field-box"
          style={{ width: 135, height: 24, fontSize: 11.5 }}
          placeholder="Filter nodes..."
          value={assetSearch}
          onChange={(e) => onSearchChange(e.target.value)}
        />
      </div>
      <div className="tree-scroll">
        {outline ? (
          renderTreeNodes(outline.tree)
        ) : (
          <div
            style={{
              padding: "18px 10px",
              color: "var(--brush-muted)",
              textAlign: "center",
            }}
          >
            Select a file in the File Browser below, drag &amp; drop a file onto the
            window, or press Ctrl+O.
          </div>
        )}
      </div>
    </div>
  );
}
