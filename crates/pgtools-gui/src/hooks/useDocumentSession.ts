import { useCallback, useRef, useState } from "react";
import { DocumentOutline } from "../types";
import { isVirtualSubNode } from "../utils/format";

export interface DocumentSessionState {
  outline: DocumentOutline | null;
  expandedNodes: Record<string, boolean>;
  selectedNodeId: string;
  assetSearch: string;
  outlineRef: React.MutableRefObject<DocumentOutline | null>;
}

export interface DocumentSessionApi extends DocumentSessionState {
  setOutline: (o: DocumentOutline | null) => void;
  setExpandedNodes: (
    v: Record<string, boolean> | ((prev: Record<string, boolean>) => Record<string, boolean>)
  ) => void;
  setSelectedNodeId: (id: string) => void;
  setAssetSearch: (s: string) => void;
  confirmDiscardIfDirty: (actionLabel?: string) => boolean;
  expandForOpen: (doc: DocumentOutline) => {
    nextExpanded: Record<string, boolean>;
    defaultTargetId: string;
  };
}

/**
 * Document outline + asset-tree selection/expansion state that previously
 * lived in App. Open/inspect still call into App (they touch many drafts),
 * but the pure document session surface is isolated here.
 */
export function useDocumentSession(): DocumentSessionApi {
  const [outline, setOutline] = useState<DocumentOutline | null>(null);
  const outlineRef = useRef<DocumentOutline | null>(null);
  outlineRef.current = outline;

  const [expandedNodes, setExpandedNodes] = useState<Record<string, boolean>>({
    root: true,
    "root:dat": true,
    "root:dtt": true,
  });
  const [selectedNodeId, setSelectedNodeId] = useState("root");
  const [assetSearch, setAssetSearch] = useState("");

  const confirmDiscardIfDirty = useCallback((actionLabel = "open another document"): boolean => {
    if (!outlineRef.current?.dirty) return true;
    return window.confirm(
      `You have unsaved changes in "${outlineRef.current.file_name}".\n\nDiscard unsaved changes and ${actionLabel}?`
    );
  }, []);

  const expandForOpen = useCallback((doc: DocumentOutline) => {
    const nextExpanded: Record<string, boolean> = {
      root: true,
      "root:dat": true,
      "root:dtt": true,
    };
    let defaultTargetId = doc.tree[0]?.id ?? "root";
    for (const rootNode of doc.tree) {
      nextExpanded[rootNode.id] = true;
      if (rootNode.format_badge === "WMB" || rootNode.format_badge === "WMB3") {
        defaultTargetId = rootNode.id;
        break;
      }
      const wmbChild = rootNode.children.find(
        (c) =>
          (c.format_badge === "WMB" || c.format_badge === "WMB3") &&
          !isVirtualSubNode(c.id)
      );
      if (wmbChild) {
        defaultTargetId = wmbChild.id;
        break;
      }
      if (rootNode.children.length > 0) {
        const firstChild = rootNode.children[0];
        nextExpanded[firstChild.id] = true;
        if (firstChild.children.length > 0) {
          defaultTargetId = firstChild.children[0].id;
          break;
        } else {
          defaultTargetId = firstChild.id;
          break;
        }
      }
    }
    return { nextExpanded, defaultTargetId };
  }, []);

  return {
    outline,
    expandedNodes,
    selectedNodeId,
    assetSearch,
    outlineRef,
    setOutline,
    setExpandedNodes,
    setSelectedNodeId,
    setAssetSearch,
    confirmDiscardIfDirty,
    expandForOpen,
  };
}
