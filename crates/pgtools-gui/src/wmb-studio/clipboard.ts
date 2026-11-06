// [wmb-studio BEGIN phase2]
import {
  ClipboardStatus,
  wmbClipboardCopy,
  wmbClipboardStatus,
} from "./contract";
import { SceneSelectionItem } from "./store";

export async function refreshClipboardStatus(): Promise<ClipboardStatus | null> {
  try {
    return await wmbClipboardStatus();
  } catch {
    return null;
  }
}

export async function copySceneItemsToClipboard(
  nodeId: string,
  items: SceneSelectionItem[],
  onStatus: (msg: string) => void
): Promise<ClipboardStatus | null> {
  const batchIds = items.filter((i) => i.kind === "batch").map((i) => i.id);
  const matIds = items.filter((i) => i.kind === "material").map((i) => i.id);

  const idsToCopy = batchIds.length > 0 ? batchIds : matIds;
  if (idsToCopy.length === 0) {
    onStatus("Select one or more batches or materials to copy.");
    return null;
  }

  try {
    const status = await wmbClipboardCopy(nodeId, idsToCopy);
    onStatus(
      `Copied ${status.count} ${status.kind} (${status.game.toUpperCase()}) to clipboard.`
    );
    return status;
  } catch (err) {
    onStatus(`Clipboard copy failed: ${String(err)}`);
    return null;
  }
}

export function canPasteClipboard(
  clipboard: ClipboardStatus | null,
  currentGame: "b1" | "b2" | "b3",
  expectedKind?: "batches" | "materials"
): { allowed: boolean; reason?: string } {
  if (!clipboard || clipboard.count === 0) {
    return { allowed: false, reason: "Clipboard is empty." };
  }
  if (clipboard.game !== currentGame) {
    return {
      allowed: false,
      reason: `Clipboard holds ${clipboard.game.toUpperCase()} ${clipboard.kind} (cannot paste into ${currentGame.toUpperCase()} model).`,
    };
  }
  if (expectedKind && clipboard.kind !== expectedKind) {
    return {
      allowed: false,
      reason: `Clipboard holds ${clipboard.kind} (expected ${expectedKind}).`,
    };
  }
  return { allowed: true };
}
// [wmb-studio END]
