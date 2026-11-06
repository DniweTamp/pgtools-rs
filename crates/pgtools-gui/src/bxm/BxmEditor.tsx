import React from "react";
import { invoke } from "@tauri-apps/api/core";
import { BxmXmlNode, ContextMenuState, MenuItemSpec, PointerDragState } from "../types";
import { getBxmNodeByPath } from "./xml";
import { IconChevronDown, IconCode } from "../components/Icons";

export interface BxmCenterViewProps {
  nodePayload: any;
  bxmViewMode: "tree" | "table" | "xml";
  bxmTreeDraft: BxmXmlNode | null;
  bxmXmlDraft: string;
  setBxmXmlDraft: (val: string) => void;
  selectedBxmPath: string;
  setSelectedBxmPath: (path: string) => void;
  bxmExpandedPaths: Record<string, boolean>;
  setBxmExpandedPaths: React.Dispatch<React.SetStateAction<Record<string, boolean>>>;
  pointerDrag: PointerDragState | null;
  setPointerDrag: React.Dispatch<React.SetStateAction<PointerDragState | null>>;
  updateBxmDraftTree: (nextRoot: BxmXmlNode) => void;
  commitBxmDraft: (customXml?: string) => Promise<void>;
  runContainerMutation: (nodeId: string, action: string, targetPath?: string, payload?: any) => Promise<void>;
  setContextMenu: React.Dispatch<React.SetStateAction<ContextMenuState | null>>;
  setHoverSubmenuIdx: React.Dispatch<React.SetStateAction<number | null>>;
  setStatusLeft: (msg: string) => void;
}

export const BxmCenterView: React.FC<BxmCenterViewProps> = ({
  nodePayload,
  bxmViewMode,
  bxmTreeDraft,
  bxmXmlDraft,
  setBxmXmlDraft,
  selectedBxmPath,
  setSelectedBxmPath,
  bxmExpandedPaths,
  setBxmExpandedPaths,
  pointerDrag,
  setPointerDrag,
  updateBxmDraftTree,
  commitBxmDraft,
  runContainerMutation,
  setContextMenu,
  setHoverSubmenuIdx,
  setStatusLeft,
}) => {
  function buildBxmNodeContextMenu(pathStr: string): MenuItemSpec[] {
    if (!bxmTreeDraft) return [];
    const isRoot = pathStr === "0";
    return [
      {
        label: "+ Add Child Element",
        action: () => {
          setContextMenu(null);
          const copy = structuredClone(bxmTreeDraft);
          const target = getBxmNodeByPath(copy, pathStr);
          if (target) {
            target.children.push({
              name: "Item",
              attributes: [["id", String(target.children.length)]],
              value: null,
              children: [],
            });
            setBxmExpandedPaths((p) => ({ ...p, [pathStr]: true }));
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
          if (isRoot) return;
          const parentPath = pathStr.slice(0, pathStr.lastIndexOf("."));
          const idx = parseInt(pathStr.split(".").pop() || "0", 10);
          const copy = structuredClone(bxmTreeDraft);
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
          if (isRoot) return;
          const parentPath = pathStr.slice(0, pathStr.lastIndexOf("."));
          const idx = parseInt(pathStr.split(".").pop() || "0", 10);
          const copy = structuredClone(bxmTreeDraft);
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
          const copy = structuredClone(bxmTreeDraft);
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
          if (isRoot) return;
          const parentPath = pathStr.slice(0, pathStr.lastIndexOf("."));
          const idx = parseInt(pathStr.split(".").pop() || "0", 10);
          const copy = structuredClone(bxmTreeDraft);
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

  // Interactive BXM Tree Renderer with Inline Editing + Pointer Drag-and-Drop Reordering
  function renderBxmInteractiveTree(node: BxmXmlNode, pathStr: string) {
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

    return (
      <div key={pathStr} style={{ marginLeft: pathStr === "0" ? 0 : 16 }}>
        <div
          data-bxm-path={pathStr}
          className={`tree-row ${isSel ? "selected" : ""} ${
            isDragBefore ? "drag-insert-before" : ""
          } ${isDragAfter ? "drag-insert-after" : ""}`}
          style={{ flexWrap: "wrap", gap: 6, padding: "4px 6px" }}
          onPointerDown={(e) => {
            if (e.button !== 0 || pathStr === "0") return;
            if ((e.target as HTMLElement).tagName === "INPUT") return;
            setPointerDrag({
              kind: "bxm",
              sourceId: pathStr,
              label: `<${node.name}>`,
              startX: e.clientX,
              startY: e.clientY,
              curX: e.clientX,
              curY: e.clientY,
              active: false,
              overId: null,
              insertPos: null,
              dragOutTriggered: false,
            });
          }}
          onClick={() => setSelectedBxmPath(pathStr)}
          onContextMenu={(e) => {
            e.preventDefault();
            e.stopPropagation();
            setSelectedBxmPath(pathStr);
            setHoverSubmenuIdx(null);
            setContextMenu({
              x: Math.min(e.clientX, window.innerWidth - 240),
              y: Math.min(e.clientY, window.innerHeight - 240),
              items: buildBxmNodeContextMenu(pathStr),
            });
          }}
        >
          <span
            className={`tree-arrow ${isExpanded ? "expanded" : ""}`}
            style={{ visibility: hasChildren ? "visible" : "hidden" }}
            onClick={(e) => {
              e.stopPropagation();
              setBxmExpandedPaths((p) => ({ ...p, [pathStr]: !isExpanded }));
            }}
          >
            <svg width="6" height="9" viewBox="0 0 6 10" fill="none" stroke="currentColor" strokeWidth="1.5">
              <path d="M1 1L5 5L1 9" strokeLinecap="round" strokeLinejoin="round" />
            </svg>
          </span>

          {/* Inline Tag Name */}
          <span className="mono" style={{ color: "var(--brush-muted)" }}>&lt;</span>
          <input
            type="text"
            className="field-box mono"
            style={{
              width: Math.max(64, (node.name?.length || 4) * 8 + 18),
              height: 22,
              padding: "1px 5px",
              fontWeight: 600,
              fontSize: 11.5,
            }}
            value={node.name}
            onClick={(e) => e.stopPropagation()}
            onChange={(e) => {
              if (!bxmTreeDraft) return;
              const copy = structuredClone(bxmTreeDraft);
              const target = getBxmNodeByPath(copy, pathStr);
              if (target) {
                target.name = e.target.value;
                updateBxmDraftTree(copy);
              }
            }}
          />
          <span className="mono" style={{ color: "var(--brush-muted)" }}>&gt;</span>

          {/* Inline Attributes */}
          {node.attributes.map(([k, v], idx) => (
            <span
              key={idx}
              style={{ display: "inline-flex", alignItems: "center", gap: 3 }}
              onClick={(e) => e.stopPropagation()}
            >
              <input
                type="text"
                className="field-box mono"
                style={{
                  width: Math.max(48, k.length * 7.5 + 14),
                  height: 21,
                  padding: "1px 4px",
                  fontSize: 11,
                  color: "var(--brush-muted)",
                }}
                value={k}
                onChange={(e) => {
                  if (!bxmTreeDraft) return;
                  const copy = structuredClone(bxmTreeDraft);
                  const target = getBxmNodeByPath(copy, pathStr);
                  if (target && target.attributes[idx]) {
                    target.attributes[idx][0] = e.target.value;
                    updateBxmDraftTree(copy);
                  }
                }}
              />
              <span className="mono" style={{ color: "var(--brush-muted)" }}>=</span>
              <input
                type="text"
                className="field-box mono"
                style={{
                  width: Math.max(62, (v?.length || 2) * 7.5 + 16),
                  height: 21,
                  padding: "1px 5px",
                  fontSize: 11,
                }}
                value={v}
                onChange={(e) => {
                  if (!bxmTreeDraft) return;
                  const copy = structuredClone(bxmTreeDraft);
                  const target = getBxmNodeByPath(copy, pathStr);
                  if (target && target.attributes[idx]) {
                    target.attributes[idx][1] = e.target.value;
                    updateBxmDraftTree(copy);
                  }
                }}
              />
            </span>
          ))}

          {/* Inline Element Text Value */}
          <input
            type="text"
            className="field-box mono"
            placeholder="(value)"
            style={{
              width: Math.max(75, ((node.value ?? "").length || 5) * 7.5 + 18),
              height: 21,
              padding: "1px 6px",
              fontSize: 11.5,
              color: "var(--brush-accent)",
            }}
            value={node.value ?? ""}
            onClick={(e) => e.stopPropagation()}
            onChange={(e) => {
              if (!bxmTreeDraft) return;
              const copy = structuredClone(bxmTreeDraft);
              const target = getBxmNodeByPath(copy, pathStr);
              if (target) {
                target.value = e.target.value.length > 0 ? e.target.value : null;
                updateBxmDraftTree(copy);
              }
            }}
          />
        </div>
        {hasChildren && isExpanded && (
          <div>{node.children.map((c, i) => renderBxmInteractiveTree(c, `${pathStr}.${i}`))}</div>
        )}
      </div>
    );
  }

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
                      title="Duplicate Row"
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
                      title="Delete Row"
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
                <div className="struct-table-container">
                  <div
                    style={{
                      display: "flex",
                      justifyContent: "space-between",
                      alignItems: "center",
                      marginBottom: 10,
                    }}
                  >
                    <span style={{ color: "var(--brush-muted)", fontSize: 12 }}>
                      Edit tags, @attributes, and values inline. Right-click any element to add/duplicate/delete, or drag to reorder.
                    </span>
                    <div style={{ display: "flex", gap: 6 }}>
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
                  {bxmTreeDraft ? (
                    renderBxmInteractiveTree(bxmTreeDraft, "0")
                  ) : (
                    <div className="viewport-empty">Empty BXM Document</div>
                  )}
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
  nodePayload: any;
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
                    {openCards.bxm_node && (
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
                              title="Remove attribute"
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
                    )}
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
