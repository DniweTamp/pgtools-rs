import {
  ClipboardStatus,
  wmbClipboardStatus,
} from "./contract";

export async function refreshClipboardStatus(): Promise<ClipboardStatus | null> {
  try {
    return await wmbClipboardStatus();
  } catch {
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
