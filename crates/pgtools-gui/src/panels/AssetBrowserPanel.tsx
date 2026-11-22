import React, { useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { AssetTreeNode, PointerDragState } from "../types";
import { isVirtualSubNode } from "../utils/format";
import { StudioRange } from "../components/StudioRange";
import { decodeWtb2Batch, renderGpuSurfaceToDataUrl } from "../utils/gpuTexture";

export interface AssetBrowserPanelProps {
  outline: { tree: AssetTreeNode[]; file_name?: string; dirty?: boolean } | null;
  expandedNodes: Record<string, boolean>;
  selectedNodeId: string;
  selectedNodeIds?: string[];
  assetSearch: string;
  pointerDrag: PointerDragState | null;
  assetRatio: number;
  zoomLevel?: number;
  docGeneration?: number;
  invalidatedThumbs?: { nodeIds: string[]; seq: number } | null;
  onChangeZoomLevel?: (level: number) => void;
  searchInputRef?: React.RefObject<HTMLInputElement | null>;
  onSearchChange: (s: string) => void;
  onToggleExpand: (nodeId: string) => void;
  onExpandAll: () => void;
  onCollapseAll: () => void;
  onSelectNode: (nodeId: string, e?: React.MouseEvent) => void;
  onContextMenu: (e: React.MouseEvent, nodeId: string, kind: string) => void;
  onPointerDownNode: (e: React.PointerEvent, node: AssetTreeNode) => void;
  cardRef?: React.RefObject<HTMLDivElement | null>;
}

const ZOOM_TILE_SIZES = [0, 64, 96, 128, 160];

interface BankGridGroup {
  bankId: string;
  bankLabel: string;
  textures: AssetTreeNode[];
}

function collectTextureGroups(nodes: AssetTreeNode[], filterLow: string): BankGridGroup[] {
  const groups: BankGridGroup[] = [];

  const matches = (n: AssetTreeNode) =>
    !filterLow ||
    n.label.toLowerCase().includes(filterLow) ||
    n.format_badge.toLowerCase().includes(filterLow);

  const walk = (list: AssetTreeNode[], parentLabel: string) => {
    for (const n of list) {
      const texChildren = (n.children || []).filter(
        (c) => c.kind === "TEXTURE" && matches(c)
      );
      if (texChildren.length > 0) {
        groups.push({
          bankId: n.id,
          bankLabel: `${n.label} (${texChildren.length})`,
          textures: texChildren,
        });
      } else if (
        matches(n) &&
        ["DDS", "GTX", "XT1", "BNTX"].includes(n.kind) &&
        (!n.children || n.children.length === 0)
      ) {
        groups.push({
          bankId: n.id,
          bankLabel: parentLabel || n.label,
          textures: [n],
        });
      }
      if (n.children && n.children.length > 0) {
        walk(n.children, n.label);
      }
    }
  };

  walk(nodes, "");
  return groups;
}

export function AssetBrowserPanel({
  outline,
  expandedNodes,
  selectedNodeId,
  selectedNodeIds,
  assetSearch,
  pointerDrag,
  assetRatio,
  zoomLevel = 0,
  docGeneration = 0,
  invalidatedThumbs = null,
  onChangeZoomLevel,
  searchInputRef,
  onSearchChange,
  onToggleExpand,
  onExpandAll,
  onCollapseAll,
  onSelectNode,
  onContextMenu,
  onPointerDownNode,
  cardRef,
}: AssetBrowserPanelProps) {
  const [thumbMap, setThumbMap] = useState<Record<string, string | null>>({});
  const tileSize = ZOOM_TILE_SIZES[Math.max(0, Math.min(4, zoomLevel))] || 0;
  const filterLow = assetSearch.trim().toLowerCase();

  const gridGroups = useMemo(() => {
    if (!outline || tileSize === 0) return [];
    return collectTextureGroups(outline.tree, filterLow);
  }, [outline, tileSize, filterLow]);

  useEffect(() => {
    setThumbMap({});
  }, [outline?.file_name, docGeneration]);

  useEffect(() => {
    if (!invalidatedThumbs || invalidatedThumbs.nodeIds.length === 0) return;
    setThumbMap((prev) => {
      let changed = false;
      const next = { ...prev };
      for (const id of invalidatedThumbs.nodeIds) {
        if (id in next) {
          delete next[id];
          changed = true;
        }
      }
      return changed ? next : prev;
    });
  }, [invalidatedThumbs]);

  useEffect(() => {
    if (tileSize === 0 || gridGroups.length === 0) return;
    const missingIds: string[] = [];
    for (const g of gridGroups) {
      for (const t of g.textures) {
        if (!(t.id in thumbMap)) {
          missingIds.push(t.id);
        }
      }
    }
    if (missingIds.length === 0) return;

    let cancelled = false;
    const batch = missingIds.slice(0, 32);
    (async () => {
      try {
        const rawBatch = await invoke<ArrayBuffer>(
          "get_texture_gpu_thumbnails_batch",
          { nodeIds: batch }
        );
        if (cancelled) return;
        const decoded = decodeWtb2Batch(rawBatch);
        const gpuUrls: Record<string, string | null> = {};
        const fallbackIds: string[] = [];
        for (const item of decoded) {
          if (item.surface) {
            const url = renderGpuSurfaceToDataUrl(item.surface, 128);
            if (url) {
              gpuUrls[item.id] = url;
              continue;
            }
          }
          fallbackIds.push(item.id);
        }
        if (fallbackIds.length > 0 && !cancelled) {
          try {
            const pairs = await invoke<[string, string | null][]>(
              "get_texture_thumbnails_batch",
              { nodeIds: fallbackIds }
            );
            for (const [id, url] of pairs) {
              gpuUrls[id] = url;
            }
          } catch {
            for (const id of fallbackIds) {
              gpuUrls[id] = null;
            }
          }
        }
        if (cancelled) return;
        setThumbMap((prev) => ({ ...prev, ...gpuUrls }));
      } catch {
        // Fallback to legacy CPU PNG thumbnails if binary batch fails
        try {
          const pairs = await invoke<[string, string | null][]>(
            "get_texture_thumbnails_batch",
            { nodeIds: batch }
          );
          if (cancelled) return;
          setThumbMap((prev) => {
            const next = { ...prev };
            for (const [id, url] of pairs) {
              next[id] = url;
            }
            return next;
          });
        } catch {
          // ignore thumbnail errors
        }
      }
    })();

    return () => {
      cancelled = true;
    };
  }, [tileSize, gridGroups, thumbMap]);

  function renderTreeNodes(nodes: AssetTreeNode[]): React.ReactNode {
    return nodes.map((node) => {
      const hasChildren = node.children && node.children.length > 0;
      const isExpanded = !!expandedNodes[node.id];
      const isSelected =
        selectedNodeId === node.id ||
        Boolean(selectedNodeIds && selectedNodeIds.includes(node.id));
      const canDragNode =
        node.id !== "root" &&
        (!isVirtualSubNode(node.id) ||
          node.kind === "UVD_GROUP" ||
          node.kind === "UVD_ITEM");
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
          n.format_badge.toLowerCase().includes(filterLow) ||
          n.id.toLowerCase().includes(filterLow)
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
              if (e.button !== 0 || !canDragNode || e.ctrlKey || e.metaKey || e.shiftKey) return;
              onPointerDownNode(e, node);
            }}
            onClick={(e) => {
              const isMultiClick = e.ctrlKey || e.metaKey || e.shiftKey;
              if (
                !isMultiClick &&
                hasChildren &&
                !expandedNodes[node.id] &&
                node.kind !== "UVD" &&
                node.kind !== "UVD_GROUP"
              ) {
                onToggleExpand(node.id);
              }
              onSelectNode(node.id, e);
            }}
            onDoubleClick={() => {
              if (hasChildren) {
                onToggleExpand(node.id);
              }
            }}
            onContextMenu={(e) => {
              e.preventDefault();
              e.stopPropagation();
              if (!(selectedNodeIds && selectedNodeIds.includes(node.id))) {
                onSelectNode(node.id);
              }
              onContextMenu(e, node.id, node.kind);
            }}
            data-tooltip={
              node.compression
                ? `${node.label} (${node.compression}${node.modified ? " · staged*" : ""})`
                : node.modified
                ? `${node.label} (staged*)`
                : node.label
            }
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
            <span
              style={{
                overflow: "hidden",
                textOverflow: "ellipsis",
                fontWeight: node.modified ? 600 : undefined,
                color: node.modified && !isSelected ? "var(--brush-accent)" : undefined,
              }}
            >
              {node.label}
            </span>
            {node.modified && (
              <span
                style={{
                  marginLeft: 5,
                  color: "var(--brush-accent)",
                  fontWeight: 700,
                }}
                data-tooltip="Staged unsaved change"
              >
                *
              </span>
            )}
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
      onWheel={(e) => {
        if ((e.ctrlKey || e.metaKey) && onChangeZoomLevel) {
          e.preventDefault();
          const step = e.deltaY < 0 ? 1 : -1;
          onChangeZoomLevel(Math.max(0, Math.min(4, zoomLevel + step)));
        }
      }}
    >
      <div className="pane-header" style={{ gap: 6, flexWrap: "wrap" }}>
        <span className="pane-title">Asset Browser</span>
        <div style={{ display: "flex", alignItems: "center", gap: 6 }}>
          {onChangeZoomLevel && (
            <div
              style={{ display: "flex", alignItems: "center", gap: 4 }}
              data-tooltip="Asset Browser zoom (List vs. Thumbnail Grid, or Ctrl+Wheel)"
            >
              <span
                className="mono"
                style={{ fontSize: 10.5, color: "var(--brush-muted)", minWidth: 34, textAlign: "right" }}
              >
                {tileSize === 0 ? "List" : `${tileSize}px`}
              </span>
              <StudioRange
                min={0}
                max={4}
                step={1}
                value={zoomLevel}
                defaultValue={2}
                compact
                ticks={5}
                style={{ width: 72 }}
                ariaLabel="Thumbnail tile zoom"
                onChange={(v) => onChangeZoomLevel(v)}
              />
            </div>
          )}
          {outline && (
            <>
              <button
                type="button"
                className="pill-button"
                style={{ height: 22, padding: "0 6px", fontSize: 10.5 }}
                onClick={onExpandAll}
                data-tooltip="Expand all folders in tree"
              >
                Expand
              </button>
              <button
                type="button"
                className="pill-button"
                style={{ height: 22, padding: "0 6px", fontSize: 10.5 }}
                onClick={onCollapseAll}
                data-tooltip="Collapse all folders in tree"
              >
                Collapse
              </button>
            </>
          )}
          <div style={{ position: "relative", display: "flex", alignItems: "center" }}>
            <input
              ref={searchInputRef as React.RefObject<HTMLInputElement>}
              type="text"
              className="field-box"
              style={{ width: 112, height: 24, fontSize: 11.5, paddingRight: assetSearch ? 18 : 6 }}
              placeholder="Filter (Ctrl+F)..."
              value={assetSearch}
              onChange={(e) => onSearchChange(e.target.value)}
            />
            {assetSearch && (
              <button
                type="button"
                onClick={() => onSearchChange("")}
                data-tooltip="Clear filter"
                style={{
                  position: "absolute",
                  right: 4,
                  background: "none",
                  border: "none",
                  color: "var(--brush-muted)",
                  cursor: "pointer",
                  fontSize: 11,
                  padding: 0,
                }}
              >
                ✕
              </button>
            )}
          </div>
        </div>
      </div>

      <div className="tree-scroll">
        {!outline ? (
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
        ) : tileSize > 0 && gridGroups.length > 0 ? (
          <div style={{ padding: "4px 6px" }}>
            {gridGroups.map((group) => (
              <div key={group.bankId} style={{ marginBottom: 12 }}>
                <div
                  style={{
                    fontSize: 11.5,
                    fontWeight: 600,
                    color: "var(--brush-heading)",
                    marginBottom: 6,
                    padding: "2px 4px",
                  }}
                >
                  {group.bankLabel}
                </div>
                <div
                  style={{
                    display: "flex",
                    flexWrap: "wrap",
                    gap: 6,
                  }}
                >
                  {group.textures.map((texNode) => {
                    const isSelected = selectedNodeId === texNode.id;
                    const thumbUrl = thumbMap[texNode.id];
                    return (
                      <div
                        key={texNode.id}
                        onClick={() => onSelectNode(texNode.id)}
                        onContextMenu={(e) => {
                          e.preventDefault();
                          e.stopPropagation();
                          onSelectNode(texNode.id);
                          onContextMenu(e, texNode.id, texNode.kind);
                        }}
                        style={{
                          width: tileSize + 12,
                          padding: 5,
                          borderRadius: 8,
                          cursor: "pointer",
                          backgroundColor: isSelected
                            ? "var(--brush-selection)"
                            : "var(--brush-window)",
                          border: isSelected
                            ? "1px solid var(--brush-accent)"
                            : "1px solid var(--brush-border)",
                          display: "flex",
                          flexDirection: "column",
                          alignItems: "center",
                          gap: 4,
                        }}
                        data-tooltip={`${texNode.label} (${texNode.format_badge})`}
                      >
                        <div
                          className="checkerboard-canvas"
                          style={{
                            width: tileSize,
                            height: tileSize,
                            borderRadius: 5,
                            overflow: "hidden",
                            display: "flex",
                            alignItems: "center",
                            justifyContent: "center",
                          }}
                        >
                          {thumbUrl ? (
                            <img
                              src={thumbUrl}
                              alt={texNode.label}
                              draggable={false}
                              style={{
                                maxWidth: "100%",
                                maxHeight: "100%",
                                objectFit: "contain",
                              }}
                            />
                          ) : (
                            <span
                              className="mono"
                              style={{ fontSize: 10, color: "var(--brush-muted)" }}
                            >
                              {texNode.format_badge}
                            </span>
                          )}
                        </div>
                        <div
                          className="mono"
                          style={{
                            fontSize: 10,
                            width: "100%",
                            overflow: "hidden",
                            textOverflow: "ellipsis",
                            whiteSpace: "nowrap",
                            textAlign: "center",
                          }}
                        >
                          {texNode.label}
                        </div>
                      </div>
                    );
                  })}
                </div>
              </div>
            ))}
          </div>
        ) : (
          renderTreeNodes(outline.tree)
        )}
      </div>
    </div>
  );
}
