import { Dispatch, MutableRefObject, SetStateAction } from "react";
import {
  extractUvdAction,
  mutateUvdNode,
  pickFileDialog,
  pickFolderDialog,
  pickMultipleFilesDialog,
  saveFileDialog,
} from "../api/tauri";
import { studioAlert, studioConfirm, studioPrompt } from "../components/StudioDialog";
import { UvdActions } from "../editors/UvdEditor";
import { AssetTreeNode, ContextMenuState, DocumentOutline, NodeInspectPayload, TextureNodeView } from "../types";

interface UseUvdActionsDeps {
  outline: DocumentOutline | null;
  outlineRef: MutableRefObject<DocumentOutline | null>;
  nodePayload: NodeInspectPayload | null;
  activeTexture: TextureNodeView | null;
  currentDirRef: MutableRefObject<string>;
  uvdPadding: number;
  setUvdPadding: Dispatch<SetStateAction<number>>;
  uvdMaxSize: number;
  setUvdMaxSize: Dispatch<SetStateAction<number>>;
  setContextMenu: Dispatch<SetStateAction<ContextMenuState | null>>;
  setBusyLabel: Dispatch<SetStateAction<string | null>>;
  setOutline: (o: DocumentOutline | null) => void;
  setWmbDocGeneration: Dispatch<SetStateAction<number>>;
  setThumbResetGen: Dispatch<SetStateAction<number>>;
  setSelectedNodeIds: Dispatch<SetStateAction<string[]>>;
  setStatusLeft: Dispatch<SetStateAction<string>>;
  selectAndInspectNode: (nodeId: string) => Promise<void>;
  loadDirectory: (target?: string) => Promise<void>;
}

export function useUvdActions(deps: UseUvdActionsDeps): {
  runUvdMutation: (nodeId: string, action: string, payload?: Record<string, unknown>) => Promise<void>;
  handleUvdExtract: (nodeIds: string[], writeJson: boolean) => Promise<void>;
  resolveUvdNodeName: (targetId: string, fallback: string) => string;
  promptAddUvdGroup: (uvdNodeId: string) => Promise<void>;
  promptAddUvdSprite: (groupNodeId: string, groupIndex?: number) => Promise<void>;
  promptReplaceUvdSprite: (itemNodeId: string, groupIndex?: number, itemIndex?: number) => Promise<void>;
  promptRenameUvdGroup: (groupNodeId: string, currentName: string, groupIndex?: number) => Promise<void>;
  promptRenameUvdSprite: (itemNodeId: string, currentName: string, groupIndex?: number, itemIndex?: number) => Promise<void>;
  uvdActions: UvdActions;
} {
  const {
    outline,
    outlineRef,
    nodePayload,
    activeTexture,
    currentDirRef,
    uvdPadding,
    setUvdPadding,
    uvdMaxSize,
    setUvdMaxSize,
    setContextMenu,
    setBusyLabel,
    setOutline,
    setWmbDocGeneration,
    setThumbResetGen,
    setSelectedNodeIds,
    setStatusLeft,
    selectAndInspectNode,
    loadDirectory,
  } = deps;

async function runUvdMutation(nodeId: string, action: string, payload: Record<string, unknown> = {}) {
  setContextMenu(null);
  try {
    setBusyLabel(`Updating UVD (${action})...`);
    const updated = await mutateUvdNode(nodeId, action, payload);
    setOutline(updated);
    setWmbDocGeneration((g) => g + 1);
    setThumbResetGen((g) => g + 1);
    let nextInspectId = nodeId;
    if (action === "delete_groups" || action === "delete_items") {
      nextInspectId = nodeId.includes("/")
        ? nodeId.slice(0, nodeId.lastIndexOf("/"))
        : "root";
      setSelectedNodeIds([nextInspectId]);
    } else if (action === "reorder_groups") {
      const m = nodeId.match(/^(.*?)uvdgrp:(\d+)$/);
      if (m) {
        const curIdx = parseInt(m[2], 10);
        const nextIdx =
          typeof payload?.to_index === "number"
            ? payload.to_index
            : payload?.direction === "up"
            ? Math.max(0, curIdx - 1)
            : payload?.direction === "down"
            ? curIdx + 1
            : curIdx;
        nextInspectId = `${m[1]}uvdgrp:${nextIdx}`;
        setSelectedNodeIds([nextInspectId]);
      }
    } else if (action === "reorder_items") {
      const m = nodeId.match(/^(.*?)uvditem:(\d+)$/);
      if (m) {
        const curIdx = parseInt(m[2], 10);
        const nextIdx =
          typeof payload?.to_index === "number"
            ? payload.to_index
            : payload?.direction === "up"
            ? Math.max(0, curIdx - 1)
            : payload?.direction === "down"
            ? curIdx + 1
            : curIdx;
        nextInspectId = `${m[1]}uvditem:${nextIdx}`;
        setSelectedNodeIds([nextInspectId]);
      }
    }
    await selectAndInspectNode(nextInspectId);
    setStatusLeft(updated.undo_label ?? `Completed UVD ${action}`);
  } catch (err) {
    const msg = String(err);
    if (
      (action === "add_group" || action === "add_item") &&
      msg.includes("CRC32 collision") &&
      !payload?.auto_suffix
    ) {
      const useSuffix = await studioConfirm(
        `${msg}\n\nClick OK to automatically append a numeric suffix (_1, _2, ...) for a unique CRC32 ID, or Cancel to abort.`,
        { title: "CRC32 Collision", confirmLabel: "Append Suffix" }
      );
      if (useSuffix) {
        await runUvdMutation(nodeId, action, { ...payload, auto_suffix: true });
        return;
      }
    }
    setStatusLeft(`UVD error: ${msg}`);
  } finally {
    setBusyLabel(null);
  }
}

async function handleUvdExtract(nodeIds: string[], writeJson: boolean) {
  setContextMenu(null);
  if (nodeIds.length === 0) return;
  try {
    const firstId = nodeIds[0];
    const isSingleItem =
      nodeIds.length === 1 &&
      (firstId.includes("/uvditem:") || firstId.startsWith("uvditem:")) &&
      !writeJson;
    if (isSingleItem) {
      const defName = `${
        nodePayload?.uvd?.item?.name ||
        activeTexture?.embedded_name ||
        "sprite"
      }.png`;
      const destFile = await saveFileDialog({
        title: "Extract Single UVD Sprite (.PNG)",
        defaultName: defName,
      });
      if (!destFile) return;
      setBusyLabel("Extracting sprite PNG...");
      const msg = await extractUvdAction({
        nodeIds,
        destPath: destFile,
        writeJson: false,
      });
      setStatusLeft(msg);
      loadDirectory(currentDirRef.current);
    } else {
      const folder = await pickFolderDialog({
        title: writeJson
          ? "Select Output Folder for UVD Group Subfolders + .uvd.json"
          : "Select Output Folder for UVD Group Subfolders",
      });
      if (!folder) return;
      setBusyLabel("Extracting UVD sprites...");
      const msg = await extractUvdAction({
        nodeIds,
        destPath: folder,
        writeJson,
      });
      setStatusLeft(msg);
      loadDirectory(currentDirRef.current);
    }
  } catch (err) {
    const errMsg = String(err);
    setStatusLeft(`UVD extract error: ${errMsg}`);
    await studioAlert(errMsg, "UVD Extract Error");
  } finally {
    setBusyLabel(null);
  }
}

function resolveUvdNodeName(targetId: string, fallback: string): string {
  const tree = outlineRef.current?.tree;
  if (tree) {
    const walk = (nodes: AssetTreeNode[]): AssetTreeNode | null => {
      for (const n of nodes) {
        if (n.id === targetId) return n;
        if (n.children && n.children.length > 0) {
          const found = walk(n.children);
          if (found) return found;
        }
      }
      return null;
    };
    const found = walk(tree);
    if (found && found.label) {
      const idx = found.label.lastIndexOf(" (");
      return idx > 0 ? found.label.slice(0, idx) : found.label;
    }
  }
  return fallback;
}

async function promptAddUvdGroup(uvdNodeId: string) {
  setContextMenu(null);
  const rawName = await studioPrompt(
    "Enter new UVD Group name (max 31 ASCII characters):",
    "new_group",
    { title: "New UVD Group" }
  );
  if (!rawName || !rawName.trim()) return;
  const trimmed = rawName.trim();
  if (trimmed.length > 31) {
    const msg = `Group name '${trimmed}' is ${trimmed.length} bytes; maximum allowed is 31 characters.`;
    setStatusLeft(msg);
    await studioAlert(msg, "Invalid Group Name");
    return;
  }
  const paths = await pickMultipleFilesDialog({
    title: `Select Initial Sprite PNG(s) for Group '${trimmed}' (or Cancel for empty 256×256 sheet)`,
    extensions: ["png", "tga", "bmp", "jpg", "jpeg"],
  });
  await runUvdMutation(uvdNodeId, "add_group", {
    name: trimmed,
    paths: paths ?? [],
  });
}

async function promptAddUvdSprite(groupNodeId: string, groupIndex?: number) {
  setContextMenu(null);
  const rawName = await studioPrompt(
    "Enter new Sprite Item name (max 63 ASCII characters):",
    "new_sprite_00",
    { title: "New Sprite Item" }
  );
  if (!rawName || !rawName.trim()) return;
  const trimmed = rawName.trim();
  if (trimmed.length > 63) {
    const msg = `Item name '${trimmed}' is ${trimmed.length} bytes; maximum allowed is 63 characters.`;
    setStatusLeft(msg);
    await studioAlert(msg, "Invalid Sprite Name");
    return;
  }
  const path = await pickFileDialog({
    title: `Select Sprite Image (.PNG) for '${trimmed}'`,
    extensions: ["png", "tga", "bmp", "jpg", "jpeg"],
  });
  if (!path) return;
  await runUvdMutation(groupNodeId, "add_item", {
    group_index: groupIndex,
    name: trimmed,
    path,
  });
}

async function promptReplaceUvdSprite(
  itemNodeId: string,
  groupIndex?: number,
  itemIndex?: number
) {
  setContextMenu(null);
  const path = await pickFileDialog({
    title: "Select Replacement Sprite Image (.PNG)",
    extensions: ["png", "tga", "bmp", "jpg", "jpeg"],
  });
  if (!path) return;
  await runUvdMutation(itemNodeId, "replace_item", {
    group_index: groupIndex,
    item_index: itemIndex,
    path,
  });
}

async function promptRenameUvdGroup(
  groupNodeId: string,
  currentName: string,
  groupIndex?: number
) {
  setContextMenu(null);
  const rawName = await studioPrompt(
    "Rename UVD Group (max 31 ASCII characters):\n\nWarning: Renaming recomputes the Group ID (crc32 & 0x7FFFFFFF). Any .uid file referencing the old Group ID may break.",
    currentName,
    { title: "Rename UVD Group" }
  );
  if (!rawName || !rawName.trim() || rawName.trim() === currentName) return;
  const trimmed = rawName.trim();
  if (trimmed.length > 31) {
    const msg = `Group name '${trimmed}' is ${trimmed.length} bytes; maximum allowed is 31 characters.`;
    setStatusLeft(msg);
    await studioAlert(msg, "Invalid Group Name");
    return;
  }
  const ok = await studioConfirm(
    `Rename group '${currentName}' to '${trimmed}'?\n\nNote: The Group ID will be recomputed from the new name, and any .uid layout referencing the old ID may break.`,
    { title: "Confirm Group Rename", confirmLabel: "Rename" }
  );
  if (!ok) return;
  await runUvdMutation(groupNodeId, "rename_group", {
    group_index: groupIndex,
    new_name: trimmed,
  });
}

async function promptRenameUvdSprite(
  itemNodeId: string,
  currentName: string,
  groupIndex?: number,
  itemIndex?: number
) {
  setContextMenu(null);
  const rawName = await studioPrompt(
    "Rename UVD Sprite Item (max 63 ASCII characters):\n\nWarning: Renaming recomputes the Item ID (CRC32). Any .uid file referencing the old Item ID may break.",
    currentName,
    { title: "Rename Sprite Item" }
  );
  if (!rawName || !rawName.trim() || rawName.trim() === currentName) return;
  const trimmed = rawName.trim();
  if (trimmed.length > 63) {
    const msg = `Item name '${trimmed}' is ${trimmed.length} bytes; maximum allowed is 63 characters.`;
    setStatusLeft(msg);
    await studioAlert(msg, "Invalid Sprite Name");
    return;
  }
  const ok = await studioConfirm(
    `Rename sprite '${currentName}' to '${trimmed}'?\n\nNote: The Item ID (CRC32) will be recomputed from the new name, and any .uid layout referencing the old ID may break.`,
    { title: "Confirm Sprite Rename", confirmLabel: "Rename" }
  );
  if (!ok) return;
  await runUvdMutation(itemNodeId, "rename_item", {
    group_index: groupIndex,
    item_index: itemIndex,
    new_name: trimmed,
  });
}

const uvdActions: UvdActions = {
  runUvdMutation,
  handleUvdExtract,
  promptAddUvdGroup,
  promptAddUvdSprite,
  promptReplaceUvdSprite,
  promptRenameUvdGroup,
  promptRenameUvdSprite,
  selectAndInspectNode: (nid: string) => selectAndInspectNode(nid),
  resolveNodeName: resolveUvdNodeName,
  uvdPadding,
  setUvdPadding,
  uvdMaxSize,
  setUvdMaxSize,
};

  return {
    runUvdMutation,
    handleUvdExtract,
    resolveUvdNodeName,
    promptAddUvdGroup,
    promptAddUvdSprite,
    promptReplaceUvdSprite,
    promptRenameUvdGroup,
    promptRenameUvdSprite,
    uvdActions,
  };
}
