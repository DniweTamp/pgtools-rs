import React, { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  BxmXmlNode,
  ContextMenuState,
  MenuItemSpec,
  NodeInspectPayload,
  PointerDragState,
} from "../types";
import { getBxmNodeByPath, safeXmlName } from "./xml";
import { IconChevronDown, IconChevronRight, IconCode } from "../components/Icons";
import { AnimatedBranch } from "../components/AnimatedBranch";

export interface BxmCenterViewProps {
  nodePayload: NodeInspectPayload;
  bxmViewMode: "tree" | "table" | "xml";
  bxmTreeDraft: BxmXmlNode | null;
  bxmXmlDraft: string;
  setBxmXmlDraft: (val: string) => void;
  selectedBxmPath: string;
  setSelectedBxmPath: (path: string) => void;
  pointerDrag: PointerDragState | null;
  setPointerDrag: React.Dispatch<React.SetStateAction<PointerDragState | null>>;
  updateBxmDraftTree: (nextRoot: BxmXmlNode) => void;
  commitBxmDraft: (customXml?: string) => Promise<void>;
  runContainerMutation: (nodeId: string, action: string, targetPath?: string, payload?: any) => Promise<void>;
  setContextMenu: React.Dispatch<React.SetStateAction<ContextMenuState | null>>;
  setHoverSubmenuIdx: React.Dispatch<React.SetStateAction<number | null>>;
  setStatusLeft: (msg: string) => void;
}

interface BxmFocusRequest {
  path: string;
  field: string;
  seq: number;
}

interface BxmExpandChange {
  path: string;
  seq: number;
}

interface BxmTreeNodeProps {
  node: BxmXmlNode;
  pathStr: string;
  selectedBxmPath: string;
  bxmExpandedPaths: Record<string, boolean>;
  lastExpandChange: BxmExpandChange | null;
  pointerDrag: PointerDragState | null;
  focusRequest: BxmFocusRequest | null;
  onSelect: (pathStr: string, focusField?: string) => void;
  onToggleExpand: (pathStr: string, currentExpanded: boolean) => void;
  onStartDrag: (e: React.PointerEvent<HTMLDivElement>, pathStr: string, tagName: string) => void;
  onNodeContextMenu: (e: React.MouseEvent<HTMLDivElement>, pathStr: string) => void;
  onChangeTag: (pathStr: string, value: string) => void;
  onBlurTag: (pathStr: string, value: string) => void;
  onChangeAttrKey: (pathStr: string, idx: number, value: string) => void;
  onChangeAttrVal: (pathStr: string, idx: number, value: string) => void;
  onChangeValue: (pathStr: string, value: string) => void;
}

const isPathInSubtree = (target: string | null | undefined, rootPath: string): boolean => {
  if (!target) return false;
  return target === rootPath || target.startsWith(`${rootPath}.`);
};

const BxmTreeNode: React.FC<BxmTreeNodeProps> = React.memo(
  function BxmTreeNode({
    node,
    pathStr,
    selectedBxmPath,
    bxmExpandedPaths,
    lastExpandChange,
    pointerDrag,
    focusRequest,
    onSelect,
    onToggleExpand,
    onStartDrag,
    onNodeContextMenu,
    onChangeTag,
    onBlurTag,
    onChangeAttrKey,
    onChangeAttrVal,
    onChangeValue,
  }) {
    const rowRef = useRef<HTMLDivElement | null>(null);
    const isSel = selectedBxmPath === pathStr;
    const hasChildren = node.children && node.children.length > 0;
    const isExpanded = bxmExpandedPaths[pathStr] ?? true;
    const isDragBefore =
      pointerDrag?.active &&
      pointerDrag.kind === "bxm" &&
      pointerDrag.overId === pathStr &&
      pointerDrag.insertPos === "before";
    const isDragAfter =
      pointerDrag?.active &&
      pointerDrag.kind === "bxm" &&
      pointerDrag.overId === pathStr &&
      pointerDrag.insertPos === "after";

    useEffect(() => {
      if (!isSel || !focusRequest || focusRequest.path !== pathStr) return;
      const input = rowRef.current?.querySelector<HTMLInputElement>(
        `[data-bxm-field="${focusRequest.field}"]`
      );
      if (input) {
        input.focus();
        input.select();
      }
    }, [isSel, focusRequest, pathStr]);

    return (
      <div style={{ marginLeft: pathStr === "0" ? 0 : 16 }}>
        <div
          ref={rowRef}
          data-bxm-path={pathStr}
          className={`tree-row bxm-tree-row ${isSel ? "selected" : ""} ${
            isDragBefore ? "drag-insert-before" : ""
          } ${isDragAfter ? "drag-insert-after" : ""}`}
          onPointerDown={(e) => onStartDrag(e, pathStr, node.name)}
          onClick={() => onSelect(pathStr)}
          onContextMenu={(e) => onNodeContextMenu(e, pathStr)}
        >
          <span
            className={`wmb-tree-caret ${hasChildren ? "clickable" : "leaf"}`}
            onClick={(e) => {
              if (!hasChildren) return;
              e.stopPropagation();
              onToggleExpand(pathStr, isExpanded);
            }}
          >
            {hasChildren ? <IconChevronRight expanded={isExpanded} /> : "·"}
          </span>

          {/* Inline Tag Name */}
          <span className="mono" style={{ color: "var(--brush-muted)" }}>&lt;</span>
          {isSel ? (
            <input
              type="text"
              data-bxm-field="tag"
              className="field-box mono"
              style={{
                width: Math.max(64, (node.name?.length || 4) * 8 + 18),
                height: 22,
                padding: "1px 5px",
                fontWeight: 600,
                fontSize: 11.5,
                boxSizing: "border-box",
              }}
              value={node.name}
              onClick={(e) => e.stopPropagation()}
              onChange={(e) => onChangeTag(pathStr, e.target.value)}
              onBlur={(e) => onBlurTag(pathStr, e.target.value)}
            />
          ) : (
            <span
              className="bxm-inline-chip tag mono"
              onClick={(e) => {
                e.stopPropagation();
                onSelect(pathStr, "tag");
              }}
            >
              {node.name}
            </span>
          )}
          <span className="mono" style={{ color: "var(--brush-muted)" }}>&gt;</span>

          {/* Inline Attributes */}
          {node.attributes.map(([k, v], idx) => (
            <span
              key={idx}
              style={{ display: "inline-flex", alignItems: "center", gap: 3 }}
              onClick={(e) => e.stopPropagation()}
            >
              {isSel ? (
                <input
                  type="text"
                  data-bxm-field={`ak-${idx}`}
                  className="field-box mono"
                  style={{
                    width: Math.max(48, k.length * 7.5 + 14),
                    height: 21,
                    padding: "1px 4px",
                    fontSize: 11,
                    color: "var(--brush-muted)",
                    boxSizing: "border-box",
                  }}
                  value={k}
                  onChange={(e) => onChangeAttrKey(pathStr, idx, e.target.value)}
                />
              ) : (
                <span
                  className="bxm-inline-chip attr-key mono"
                  onClick={() => onSelect(pathStr, `ak-${idx}`)}
                >
                  {k}
                </span>
              )}
              <span className="mono" style={{ color: "var(--brush-muted)" }}>=</span>
              {isSel ? (
                <input
                  type="text"
                  data-bxm-field={`av-${idx}`}
                  className="field-box mono"
                  style={{
                    width: Math.max(62, (v?.length || 2) * 7.5 + 16),
                    height: 21,
                    padding: "1px 5px",
                    fontSize: 11,
                    boxSizing: "border-box",
                  }}
                  value={v}
                  onChange={(e) => onChangeAttrVal(pathStr, idx, e.target.value)}
                />
              ) : (
                <span
                  className="bxm-inline-chip attr-val mono"
                  onClick={() => onSelect(pathStr, `av-${idx}`)}
                >
                  {v}
                </span>
              )}
            </span>
          ))}

          {/* Inline Element Text Value */}
          {isSel ? (
            <input
              type="text"
              data-bxm-field="val"
              className="field-box mono"
              placeholder="(value)"
              style={{
                width: Math.max(75, ((node.value ?? "").length || 5) * 7.5 + 18),
                height: 21,
                padding: "1px 6px",
                fontSize: 11.5,
                color: "var(--brush-accent)",
                boxSizing: "border-box",
              }}
              value={node.value ?? ""}
              onClick={(e) => e.stopPropagation()}
              onChange={(e) => onChangeValue(pathStr, e.target.value)}
            />
          ) : (
            <span
              className={`bxm-inline-chip val mono${node.value ? "" : " empty"}`}
              onClick={(e) => {
                e.stopPropagation();
                onSelect(pathStr, "val");
              }}
            >
              {node.value || "(value)"}
            </span>
          )}
        </div>
        {hasChildren && (
          <AnimatedBranch expanded={isExpanded} scrollSelector="[data-bxm-scroll]">
            {node.children.map((c, i) => (
              <BxmTreeNode
                key={`${pathStr}.${i}`}
                node={c}
                pathStr={`${pathStr}.${i}`}
                selectedBxmPath={selectedBxmPath}
                bxmExpandedPaths={bxmExpandedPaths}
                lastExpandChange={lastExpandChange}
                pointerDrag={pointerDrag}
                focusRequest={focusRequest}
                onSelect={onSelect}
                onToggleExpand={onToggleExpand}
                onStartDrag={onStartDrag}
                onNodeContextMenu={onNodeContextMenu}
                onChangeTag={onChangeTag}
                onBlurTag={onBlurTag}
                onChangeAttrKey={onChangeAttrKey}
                onChangeAttrVal={onChangeAttrVal}
                onChangeValue={onChangeValue}
              />
            ))}
          </AnimatedBranch>
        )}
      </div>
    );
  },
  (prev, next) => {
    if (prev.node !== next.node || prev.pathStr !== next.pathStr) return false;

    const p = next.pathStr;
    const prevExp = prev.bxmExpandedPaths[p] ?? true;
    const nextExp = next.bxmExpandedPaths[p] ?? true;
    if (prevExp !== nextExp) return false;

    if (prev.bxmExpandedPaths !== next.bxmExpandedPaths) {
      if (
        !next.lastExpandChange ||
        isPathInSubtree(next.lastExpandChange.path, p)
      ) {
        return false;
      }
    }

    if (prev.selectedBxmPath !== next.selectedBxmPath) {
      if (
        isPathInSubtree(prev.selectedBxmPath, p) ||
        isPathInSubtree(next.selectedBxmPath, p)
      ) {
        return false;
      }
    }

    if (prev.focusRequest !== next.focusRequest) {
      if (
        isPathInSubtree(prev.focusRequest?.path, p) ||
        isPathInSubtree(next.focusRequest?.path, p)
      ) {
        return false;
      }
    }

    const prevDragOver =
      prev.pointerDrag?.active && prev.pointerDrag.kind === "bxm"
        ? prev.pointerDrag.overId
        : null;
    const nextDragOver =
      next.pointerDrag?.active && next.pointerDrag.kind === "bxm"
        ? next.pointerDrag.overId
        : null;
    if (
      prevDragOver !== nextDragOver ||
      (nextDragOver && prev.pointerDrag?.insertPos !== next.pointerDrag?.insertPos)
    ) {
      if (isPathInSubtree(prevDragOver, p) || isPathInSubtree(nextDragOver, p)) {
        return false;
      }
    }

    return true;
  }
);

export const BxmCenterView: React.FC<BxmCenterViewProps> = ({
  nodePayload,
  bxmViewMode,
  bxmTreeDraft,
  bxmXmlDraft,
  setBxmXmlDraft,
  selectedBxmPath,
  setSelectedBxmPath,
  pointerDrag,
  setPointerDrag,
  updateBxmDraftTree,
  commitBxmDraft,
  runContainerMutation,
  setContextMenu,
  setHoverSubmenuIdx,
  setStatusLeft,
}) => {
  const [localExpandedPaths, setLocalExpandedPaths] = useState<Record<string, boolean>>(() => ({
    "0": true,
    "0.0": true,
    "0.1": true,
  }));
  const [prevPayload, setPrevPayload] = useState(nodePayload);
  const [treeResetKey, setTreeResetKey] = useState(0);
  const [focusRequest, setFocusRequest] = useState<BxmFocusRequest | null>(null);
  const [lastExpandChange, setLastExpandChange] = useState<BxmExpandChange | null>(null);
  const seqRef = useRef(0);
  const draftRef = useRef(bxmTreeDraft);
  draftRef.current = bxmTreeDraft;

  if (nodePayload !== prevPayload) {
    setPrevPayload(nodePayload);
    setTreeResetKey((k) => k + 1);
    setLocalExpandedPaths({ "0": true, "0.0": true, "0.1": true });
    setLastExpandChange(null);
    setFocusRequest(null);
  }

  function buildBxmNodeContextMenu(pathStr: string): MenuItemSpec[] {
    const root = draftRef.current;
    if (!root) return [];
    const isRoot = pathStr === "0";
    return [
      {
        label: "+ Add Child Element",
        action: () => {
          setContextMenu(null);
          const cur = draftRef.current;
          if (!cur) return;
          const copy = structuredClone(cur);
          const target = getBxmNodeByPath(copy, pathStr);
          if (target) {
            target.children.push({
              name: "Item",
              attributes: [["id", String(target.children.length)]],
              value: null,
              children: [],
            });
            setLastExpandChange({ path: pathStr, seq: ++seqRef.current });
            setLocalExpandedPaths((p) => ({ ...p, [pathStr]: true }));
            updateBxmDraftTree(copy);
            setSelectedBxmPath(`${pathStr}.${target.children.length - 1}`);
          }
        },
      },
      {
        label: "+ Add Sibling Element",
        disabled: isRoot,
        action: () => {
          setContextMenu(null);
          const cur = draftRef.current;
          if (isRoot || !cur) return;
          const parentPath = pathStr.slice(0, pathStr.lastIndexOf("."));
          const idx = parseInt(pathStr.split(".").pop() || "0", 10);
          const copy = structuredClone(cur);
          const parent = getBxmNodeByPath(copy, parentPath);
          if (parent) {
            parent.children.splice(idx + 1, 0, {
              name: parent.children[idx]?.name || "Item",
              attributes: [],
              value: null,
              children: [],
            });
            updateBxmDraftTree(copy);
            setSelectedBxmPath(`${parentPath}.${idx + 1}`);
          }
        },
      },
      {
        label: "Duplicate Element",
        disabled: isRoot,
        action: () => {
          setContextMenu(null);
          const cur = draftRef.current;
          if (isRoot || !cur) return;
          const parentPath = pathStr.slice(0, pathStr.lastIndexOf("."));
          const idx = parseInt(pathStr.split(".").pop() || "0", 10);
          const copy = structuredClone(cur);
          const parent = getBxmNodeByPath(copy, parentPath);
          if (parent && parent.children[idx]) {
            parent.children.splice(idx + 1, 0, structuredClone(parent.children[idx]));
            updateBxmDraftTree(copy);
            setSelectedBxmPath(`${parentPath}.${idx + 1}`);
          }
        },
      },
      {
        label: "+ Add @Attribute",
        action: () => {
          setContextMenu(null);
          const cur = draftRef.current;
          if (!cur) return;
          const copy = structuredClone(cur);
          const target = getBxmNodeByPath(copy, pathStr);
          if (target) {
            target.attributes.push([`attr${target.attributes.length}`, "0"]);
            updateBxmDraftTree(copy);
          }
        },
      },
      { separator: true },
      {
        label: "Delete Element",
        disabled: isRoot,
        action: () => {
          setContextMenu(null);
          const cur = draftRef.current;
          if (isRoot || !cur) return;
          const parentPath = pathStr.slice(0, pathStr.lastIndexOf("."));
          const idx = parseInt(pathStr.split(".").pop() || "0", 10);
          const copy = structuredClone(cur);
          const parent = getBxmNodeByPath(copy, parentPath);
          if (parent && parent.children[idx]) {
            parent.children.splice(idx, 1);
            updateBxmDraftTree(copy);
            setSelectedBxmPath(parentPath);
          }
        },
      },
    ];
  }

  const handleSelectNode = useCallback(
    (pathStr: string, focusField?: string) => {
      setSelectedBxmPath(pathStr);
      setFocusRequest(
        focusField ? { path: pathStr, field: focusField, seq: ++seqRef.current } : null
      );
    },
    [setSelectedBxmPath]
  );

  const handleToggleExpand = useCallback(
    (pathStr: string, currentExpanded: boolean) => {
      setLastExpandChange({ path: pathStr, seq: ++seqRef.current });
      setLocalExpandedPaths((p) => ({ ...p, [pathStr]: !currentExpanded }));
    },
    []
  );

  const handleStartDrag = useCallback(
    (e: React.PointerEvent<HTMLDivElement>, pathStr: string, tagName: string) => {
      if (e.button !== 0 || pathStr === "0") return;
      const target = e.target as HTMLElement;
      if (
        target.tagName === "INPUT" ||
        target.closest(".wmb-tree-caret") ||
        target.closest(".bxm-inline-chip")
      ) {
        return;
      }
      setPointerDrag({
        kind: "bxm",
        sourceId: pathStr,
        label: `<${tagName}>`,
        startX: e.clientX,
        startY: e.clientY,
        curX: e.clientX,
        curY: e.clientY,
        active: false,
        overId: null,
        insertPos: null,
        dragOutTriggered: false,
      });
    },
    [setPointerDrag]
  );

  const handleNodeContextMenu = useCallback(
    (e: React.MouseEvent<HTMLDivElement>, pathStr: string) => {
      e.preventDefault();
      e.stopPropagation();
      setSelectedBxmPath(pathStr);
      setFocusRequest(null);
      setHoverSubmenuIdx(null);
      setContextMenu({
        x: Math.min(e.clientX, window.innerWidth - 240),
        y: Math.min(e.clientY, window.innerHeight - 240),
        items: buildBxmNodeContextMenu(pathStr),
      });
    },
    [setContextMenu, setHoverSubmenuIdx, setSelectedBxmPath]
  );

  const handleChangeTag = useCallback(
    (pathStr: string, value: string) => {
      const cur = draftRef.current;
      if (!cur) return;
      const copy = structuredClone(cur);
      const target = getBxmNodeByPath(copy, pathStr);
      if (target) {
        target.name = value;
        updateBxmDraftTree(copy);
      }
    },
    [updateBxmDraftTree]
  );

  const handleBlurTag = useCallback(
    (pathStr: string, value: string) => {
      const cur = draftRef.current;
      if (!cur) return;
      const sanitized = safeXmlName(value, "Node");
      if (sanitized === value) return;
      const copy = structuredClone(cur);
      const target = getBxmNodeByPath(copy, pathStr);
      if (target) {
        target.name = sanitized;
        updateBxmDraftTree(copy);
      }
    },
    [updateBxmDraftTree]
  );

  const handleChangeAttrKey = useCallback(
    (pathStr: string, idx: number, value: string) => {
      const cur = draftRef.current;
      if (!cur) return;
      const copy = structuredClone(cur);
      const target = getBxmNodeByPath(copy, pathStr);
      if (target && target.attributes[idx]) {
        target.attributes[idx][0] = value;
        updateBxmDraftTree(copy);
      }
    },
    [updateBxmDraftTree]
  );

  const handleChangeAttrVal = useCallback(
    (pathStr: string, idx: number, value: string) => {
      const cur = draftRef.current;
      if (!cur) return;
      const copy = structuredClone(cur);
      const target = getBxmNodeByPath(copy, pathStr);
      if (target && target.attributes[idx]) {
        target.attributes[idx][1] = value;
        updateBxmDraftTree(copy);
      }
    },
    [updateBxmDraftTree]
  );

  const handleChangeValue = useCallback(
    (pathStr: string, value: string) => {
      const cur = draftRef.current;
      if (!cur) return;
      const copy = structuredClone(cur);
      const target = getBxmNodeByPath(copy, pathStr);
      if (target) {
        target.value = value.length > 0 ? value : null;
        updateBxmDraftTree(copy);
      }
    },
    [updateBxmDraftTree]
  );

  // Automatic / Optional Table View for BXM / XML / GAD
  function renderBxmTableView() {
    if (!bxmTreeDraft) return null;
    // Choose the selected node if it has children, otherwise root
    const currentContainerPath = (() => {
      const sel = getBxmNodeByPath(bxmTreeDraft, selectedBxmPath);
      if (sel && sel.children && sel.children.length > 0) return selectedBxmPath;
      if (selectedBxmPath.includes(".")) {
        return selectedBxmPath.slice(0, selectedBxmPath.lastIndexOf("."));
      }
      return "0";
    })();
    const tableParent = getBxmNodeByPath(bxmTreeDraft, currentContainerPath) || bxmTreeDraft;
    const rows = tableParent.children || [];

    // Collect union of all @attribute names and leaf child tag names across rows
    const attrCols: string[] = [];
    const childCols: string[] = [];
    for (const r of rows) {
      for (const [ak] of r.attributes || []) {
        if (ak && !attrCols.includes(ak)) attrCols.push(ak);
      }
      for (const ch of r.children || []) {
        if (ch.name && (!ch.children || ch.children.length === 0) && !childCols.includes(ch.name)) {
          childCols.push(ch.name);
        }
      }
    }

    return (
      <div style={{ display: "flex", flexDirection: "column", gap: 10 }}>
        <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center" }}>
          <span style={{ fontWeight: 600, color: "var(--brush-heading)" }}>
            Table View — &lt;{tableParent.name}&gt; ({rows.length} child rows)
          </span>
          <div style={{ display: "flex", gap: 8 }}>
            <button
              className="pill-button"
              onClick={() => {
                const copy = structuredClone(bxmTreeDraft);
                const parent = getBxmNodeByPath(copy, currentContainerPath);
                if (!parent) return;
                const template = parent.children[parent.children.length - 1];
                const newRow: BxmXmlNode = template
                  ? structuredClone(template)
                  : {
                      name: "Entry",
                      attributes: attrCols.map((k) => [k, ""]),
                      value: null,
                      children: childCols.map((cn) => ({
                        name: cn,
                        attributes: [],
                        value: "",
                        children: [],
                      })),
                    };
                parent.children.push(newRow);
                updateBxmDraftTree(copy);
              }}
            >
              + Add Row
            </button>
            <button className="pill-button accent" onClick={() => commitBxmDraft()}>
              Commit BXM Changes
            </button>
          </div>
        </div>

        {(() => {
          const showTextValueCol = rows.some((r) => (r.value ?? "").trim().length > 0);
          return (
        <table className="struct-table">
          <thead>
            <tr>
              <th>#</th>
              <th>Tag</th>
              {showTextValueCol && <th>Text Value</th>}
              {attrCols.map((ak) => (
                <th key={`attr:${ak}`} className="mono">
                  @{ak}
                </th>
              ))}
              {childCols.map((ck) => (
                <th key={`child:${ck}`} className="mono">
                  &lt;{ck}&gt;
                </th>
              ))}
              <th>Actions</th>
            </tr>
          </thead>
          <tbody>
            {rows.map((r, ri) => (
              <tr key={ri}>
                <td className="mono">#{ri}</td>
                <td>
                  <input
                    type="text"
                    size={1}
                    className="field-box mono"
                    style={{ height: 24, fontWeight: 600 }}
                    value={r.name}
                    onChange={(e) => {
                      const copy = structuredClone(bxmTreeDraft);
                      const parent = getBxmNodeByPath(copy, currentContainerPath);
                      if (parent && parent.children[ri]) {
                        parent.children[ri].name = e.target.value;
                        updateBxmDraftTree(copy);
                      }
                    }}
                    onBlur={(e) => {
                      const sanitized = safeXmlName(e.target.value, "Node");
                      if (sanitized === e.target.value) return;
                      const copy = structuredClone(bxmTreeDraft);
                      const parent = getBxmNodeByPath(copy, currentContainerPath);
                      if (parent && parent.children[ri]) {
                        parent.children[ri].name = sanitized;
                        updateBxmDraftTree(copy);
                      }
                    }}
                  />
                </td>
                {showTextValueCol && (
                  <td>
                    <input
                      type="text"
                      size={1}
                      className="field-box mono"
                      style={{ height: 24 }}
                      value={r.value ?? ""}
                      onChange={(e) => {
                        const copy = structuredClone(bxmTreeDraft);
                        const parent = getBxmNodeByPath(copy, currentContainerPath);
                        if (parent && parent.children[ri]) {
                          parent.children[ri].value = e.target.value.length > 0 ? e.target.value : null;
                          updateBxmDraftTree(copy);
                        }
                      }}
                    />
                  </td>
                )}
                {attrCols.map((ak) => {
                  const pair = r.attributes.find(([k]) => k === ak);
                  return (
                    <td key={`attr:${ak}`}>
                      <input
                        type="text"
                        size={1}
                        className="field-box mono"
                        style={{ height: 24 }}
                        value={pair ? pair[1] : ""}
                        onChange={(e) => {
                          const copy = structuredClone(bxmTreeDraft);
                          const parent = getBxmNodeByPath(copy, currentContainerPath);
                          if (parent && parent.children[ri]) {
                            const rowNode = parent.children[ri];
                            const existing = rowNode.attributes.find(([k]) => k === ak);
                            if (existing) {
                              existing[1] = e.target.value;
                            } else {
                              rowNode.attributes.push([ak, e.target.value]);
                            }
                            updateBxmDraftTree(copy);
                          }
                        }}
                      />
                    </td>
                  );
                })}
                {childCols.map((ck) => {
                  const childNode = r.children.find((c) => c.name === ck);
                  return (
                    <td key={`child:${ck}`}>
                      <input
                        type="text"
                        size={1}
                        className="field-box mono"
                        style={{ height: 24 }}
                        value={childNode?.value ?? ""}
                        onChange={(e) => {
                          const copy = structuredClone(bxmTreeDraft);
                          const parent = getBxmNodeByPath(copy, currentContainerPath);
                          if (parent && parent.children[ri]) {
                            const rowNode = parent.children[ri];
                            const existing = rowNode.children.find((c) => c.name === ck);
                            if (existing) {
                              existing.value = e.target.value;
                            } else {
                              rowNode.children.push({
                                name: ck,
                                attributes: [],
                                value: e.target.value,
                                children: [],
                              });
                            }
                            updateBxmDraftTree(copy);
                          }
                        }}
                      />
                    </td>
                  );
                })}
                <td>
                  <div style={{ display: "inline-flex", gap: 4 }}>
                    <button
                      className="pill-button"
                      style={{ height: 23, padding: "0 7px", fontSize: 11 }}
                      data-tooltip="Duplicate Row"
                      onClick={() => {
                        const copy = structuredClone(bxmTreeDraft);
                        const parent = getBxmNodeByPath(copy, currentContainerPath);
                        if (parent && parent.children[ri]) {
                          parent.children.splice(ri + 1, 0, structuredClone(parent.children[ri]));
                          updateBxmDraftTree(copy);
                        }
                      }}
                    >
                      ⧉
                    </button>
                    <button
                      className="pill-button"
                      style={{ height: 23, padding: "0 7px", fontSize: 11 }}
                      data-tooltip="Delete Row"
                      onClick={() => {
                        const copy = structuredClone(bxmTreeDraft);
                        const parent = getBxmNodeByPath(copy, currentContainerPath);
                        if (parent && parent.children[ri]) {
                          parent.children.splice(ri, 1);
                          updateBxmDraftTree(copy);
                        }
                      }}
                    >
                      ✕
                    </button>
                  </div>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
          );
        })()}
      </div>
    );
  }

  return (
    <>
      {              bxmViewMode === "tree" ? (
                <div style={{ flex: 1, minHeight: 0, display: "flex", flexDirection: "column" }}>
                  <div
                    style={{
                      display: "flex",
                      justifyContent: "space-between",
                      alignItems: "center",
                      padding: "10px 10px 8px 10px",
                      flexShrink: 0,
                      gap: 10,
                    }}
                  >
                    <span style={{ color: "var(--brush-muted)", fontSize: 12 }}>
                      Edit tags, @attributes, and values inline. Right-click any element to add/duplicate/delete, or drag to reorder.
                    </span>
                    <div style={{ display: "flex", gap: 6, flexShrink: 0 }}>
                      <button
                        className="pill-button"
                        onClick={async () => {
                          const srcXml = await invoke<string | null>("pick_file_dialog", {
                            title: "Import & Encode XML into Binary BXM",
                            extensions: ["xml"],
                          });
                          if (srcXml) {
                            await runContainerMutation(nodePayload.node_id, "import_bxm_xml", srcXml);
                          }
                        }}
                      >
                        Import .XML...
                      </button>
                      <button
                        className="pill-button"
                        onClick={async () => {
                          const dest = await invoke<string | null>("save_file_dialog", {
                            title: "Export Decoded XML (.xml)",
                            defaultName: `${(nodePayload.title || "decoded").replace(/\.[^.]+$/, "")}.xml`,
                          });
                          if (dest) {
                            await runContainerMutation(nodePayload.node_id, "export_bxm_xml", dest);
                            setStatusLeft(`Exported decoded XML to ${dest}`);
                          }
                        }}
                      >
                        Export .XML...
                      </button>
                      <button className="pill-button accent" onClick={() => commitBxmDraft()}>
                        Commit BXM Changes
                      </button>
                    </div>
                  </div>
                  <div
                    data-bxm-scroll="true"
                    style={{
                      flex: 1,
                      minHeight: 0,
                      overflowY: "auto",
                      overflowX: "auto",
                      scrollbarGutter: "stable",
                      padding: "0 10px 10px 10px",
                    }}
                  >
                    {bxmTreeDraft ? (
                      <BxmTreeNode
                        key={treeResetKey}
                        node={bxmTreeDraft}
                        pathStr="0"
                        selectedBxmPath={selectedBxmPath}
                        bxmExpandedPaths={localExpandedPaths}
                        lastExpandChange={lastExpandChange}
                        pointerDrag={pointerDrag}
                        focusRequest={focusRequest}
                        onSelect={handleSelectNode}
                        onToggleExpand={handleToggleExpand}
                        onStartDrag={handleStartDrag}
                        onNodeContextMenu={handleNodeContextMenu}
                        onChangeTag={handleChangeTag}
                        onBlurTag={handleBlurTag}
                        onChangeAttrKey={handleChangeAttrKey}
                        onChangeAttrVal={handleChangeAttrVal}
                        onChangeValue={handleChangeValue}
                      />
                    ) : (
                      <div className="viewport-empty">Empty BXM Document</div>
                    )}
                  </div>
                </div>
              ) : bxmViewMode === "table" ? (
                <div className="struct-table-container">{renderBxmTableView()}</div>
              ) : (
                <div style={{ flex: 1, display: "flex", flexDirection: "column", padding: 10, gap: 8 }}>
                  <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center" }}>
                    <span style={{ fontWeight: 600, color: "var(--brush-heading)" }}>
                      Raw UTF-8 XML Source
                    </span>
                    <button
                      className="pill-button accent"
                      onClick={() => commitBxmDraft(bxmXmlDraft)}
                    >
                      Commit Raw XML to Binary BXM
                    </button>
                  </div>
                  <textarea
                    className="field-box mono"
                    style={{
                      flex: 1,
                      width: "100%",
                      height: "100%",
                      resize: "none",
                      padding: 12,
                      lineHeight: 1.5,
                    }}
                    value={bxmXmlDraft}
                    onChange={(e) => setBxmXmlDraft(e.target.value)}
                  />
                </div>
              )}
    </>
  );
};

export interface BxmInspectorViewProps {
  nodePayload: NodeInspectPayload;
  bxmTreeDraft: BxmXmlNode | null;
  selectedBxmPath: string;
  openCards: Record<string, boolean>;
  toggleCard: (key: string) => void;
  updateBxmDraftTree: (nextRoot: BxmXmlNode) => void;
  commitBxmDraft: (customXml?: string) => Promise<void>;
}

export const BxmInspectorView: React.FC<BxmInspectorViewProps> = ({
  nodePayload,
  bxmTreeDraft,
  selectedBxmPath,
  openCards,
  toggleCard,
  updateBxmDraftTree,
  commitBxmDraft,
}) => {
  return (
            <div>
              <div className="inspector-header">
                <div className="inspector-icon-box">
                  <IconCode size={20} />
                </div>
                <div>
                  <div className="inspector-title">{nodePayload.title}</div>
                  <div className="inspector-subtitle">
                    Binary XML ({nodePayload.tag}) · {nodePayload.size_bytes} bytes
                  </div>
                </div>
              </div>

              <div className="validation-banner">
                <span>✓</span>
                <span>Decoded cleanly — string table and node offsets are recalculated on commit.</span>
              </div>

              {(() => {
                const selNode = getBxmNodeByPath(bxmTreeDraft, selectedBxmPath);
                if (!selNode) return null;
                return (
                  <div className="inspector-card">
                    <button className="card-header-toggle" onClick={() => toggleCard("bxm_node")}>
                      <span className="card-header-left">
                        <span style={{ color: "var(--brush-muted)" }}>&lt;/&gt;</span>
                        <span>Selected Element: &lt;{selNode.name}&gt;</span>
                      </span>
                      <IconChevronDown collapsed={!openCards.bxm_node} />
                    </button>
                                          <div className={`collapse-grid${!!openCards.bxm_node ? " expanded" : ""}`}>
  <div className="collapse-inner">
    <div className="card-body">
                        <div className="inspector-row">
                          <span className="inspector-row-label">Tag Name</span>
                          <input
                            type="text"
                            className="field-box mono"
                            value={selNode.name}
                            onChange={(e) => {
                              if (!bxmTreeDraft) return;
                              const copy = structuredClone(bxmTreeDraft);
                              const t = getBxmNodeByPath(copy, selectedBxmPath);
                              if (t) {
                                t.name = e.target.value;
                                updateBxmDraftTree(copy);
                              }
                            }}
                            onBlur={(e) => {
                              if (!bxmTreeDraft) return;
                              const sanitized = safeXmlName(e.target.value, "Node");
                              if (sanitized === e.target.value) return;
                              const copy = structuredClone(bxmTreeDraft);
                              const t = getBxmNodeByPath(copy, selectedBxmPath);
                              if (t) {
                                t.name = sanitized;
                                updateBxmDraftTree(copy);
                              }
                            }}
                          />
                        </div>
                        <div className="inspector-row">
                          <span className="inspector-row-label">Text Value</span>
                          <input
                            type="text"
                            className="field-box mono"
                            placeholder="(empty)"
                            value={selNode.value ?? ""}
                            onChange={(e) => {
                              if (!bxmTreeDraft) return;
                              const copy = structuredClone(bxmTreeDraft);
                              const t = getBxmNodeByPath(copy, selectedBxmPath);
                              if (t) {
                                t.value = e.target.value.length > 0 ? e.target.value : null;
                                updateBxmDraftTree(copy);
                              }
                            }}
                          />
                        </div>

                        <div
                          style={{
                            display: "flex",
                            justifyContent: "space-between",
                            alignItems: "center",
                            marginTop: 6,
                          }}
                        >
                          <span style={{ color: "var(--brush-muted)", fontWeight: 600 }}>
                            @Attributes ({selNode.attributes.length})
                          </span>
                          <button
                            className="pill-button"
                            style={{ height: 23, padding: "0 8px", fontSize: 11 }}
                            onClick={() => {
                              if (!bxmTreeDraft) return;
                              const copy = structuredClone(bxmTreeDraft);
                              const t = getBxmNodeByPath(copy, selectedBxmPath);
                              if (t) {
                                t.attributes.push([`attr${t.attributes.length}`, "0"]);
                                updateBxmDraftTree(copy);
                              }
                            }}
                          >
                            + Add @Attr
                          </button>
                        </div>

                        {selNode.attributes.map(([k, v], i) => (
                          <div
                            key={i}
                            style={{ display: "grid", gridTemplateColumns: "95px 1fr 28px", gap: 6 }}
                          >
                            <input
                              type="text"
                              className="field-box mono"
                              style={{ height: 25 }}
                              value={k}
                              onChange={(e) => {
                                if (!bxmTreeDraft) return;
                                const copy = structuredClone(bxmTreeDraft);
                                const t = getBxmNodeByPath(copy, selectedBxmPath);
                                if (t && t.attributes[i]) {
                                  t.attributes[i][0] = e.target.value;
                                  updateBxmDraftTree(copy);
                                }
                              }}
                            />
                            <input
                              type="text"
                              className="field-box mono"
                              style={{ height: 25 }}
                              value={v}
                              onChange={(e) => {
                                if (!bxmTreeDraft) return;
                                const copy = structuredClone(bxmTreeDraft);
                                const t = getBxmNodeByPath(copy, selectedBxmPath);
                                if (t && t.attributes[i]) {
                                  t.attributes[i][1] = e.target.value;
                                  updateBxmDraftTree(copy);
                                }
                              }}
                            />
                            <button
                              className="pill-button"
                              style={{ height: 25, padding: 0 }}
                              data-tooltip="Remove attribute"
                              onClick={() => {
                                if (!bxmTreeDraft) return;
                                const copy = structuredClone(bxmTreeDraft);
                                const t = getBxmNodeByPath(copy, selectedBxmPath);
                                if (t && t.attributes[i]) {
                                  t.attributes.splice(i, 1);
                                  updateBxmDraftTree(copy);
                                }
                              }}
                            >
                              ✕
                            </button>
                          </div>
                        ))}
                      </div>
  </div>
</div>
                    
                  </div>
                );
              })()}

              <div className="inspector-card">
                <div className="card-body" style={{ paddingTop: 14 }}>
                  <button className="pill-button accent" onClick={() => commitBxmDraft()}>
                    <span>Commit XML Changes to Binary BXM</span>
                  </button>
                </div>
              </div>
            </div>
  );
};
