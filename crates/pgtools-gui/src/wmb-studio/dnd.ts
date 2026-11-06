// [wmb-studio BEGIN phase2]
import React from "react";
import { SelectionKind } from "./store";

export type DropPosition = "before" | "onto" | "after";

export interface DragPayload {
  kind: "texture" | "material" | "batch" | "meshGroup" | "bone";
  ids: string[];
  sourceParentId?: string;
}

const WMB_DND_MIME = "application/x-pgtools-wmb-dnd";
let activeDragPayload: DragPayload | null = null;

export function setDragPayload(
  e: React.DragEvent,
  payload: DragPayload
): void {
  activeDragPayload = payload;
  try {
    e.dataTransfer.effectAllowed = "copyMove";
    e.dataTransfer.setData(WMB_DND_MIME, JSON.stringify(payload));
    e.dataTransfer.setData("text/plain", JSON.stringify(payload));
  } catch {
    // Fallback uses activeDragPayload in-memory reference
  }
}

export function getActiveDragPayload(): DragPayload | null {
  return activeDragPayload;
}

export function clearDragPayload(): void {
  activeDragPayload = null;
}

export function parseDragPayload(e: React.DragEvent): DragPayload | null {
  try {
    const raw =
      e.dataTransfer.getData(WMB_DND_MIME) ||
      e.dataTransfer.getData("text/plain");
    if (raw) {
      const parsed = JSON.parse(raw) as DragPayload;
      if (parsed && Array.isArray(parsed.ids)) {
        activeDragPayload = null;
        return parsed;
      }
    }
  } catch {
    // Ignore parse error and fall back to activeDragPayload
  }
  const fallback = activeDragPayload;
  activeDragPayload = null;
  return fallback;
}

export function computeDropPositionAt(
  clientY: number,
  element: HTMLElement,
  allowOnto: boolean
): DropPosition {
  const rect = element.getBoundingClientRect();
  if (rect.height <= 0) return allowOnto ? "onto" : "after";
  const relY = (clientY - rect.top) / rect.height;
  if (allowOnto) {
    if (relY < 0.25) return "before";
    if (relY > 0.75) return "after";
    return "onto";
  }
  return relY < 0.5 ? "before" : "after";
}

export function computeDropPosition(
  e: React.DragEvent<HTMLElement>,
  element: HTMLElement,
  allowOnto: boolean
): DropPosition {
  return computeDropPositionAt(e.clientY, element, allowOnto);
}

export function isDropCompatible(
  payload: DragPayload,
  targetKind: SelectionKind,
  targetId: string,
  position: DropPosition,
  canReparentBone?: (draggedBoneId: string, targetBoneId: string) => boolean
): boolean {
  if (payload.ids.includes(targetId)) return false;

  // Material -> Batch or MeshGroup
  if (
    payload.kind === "material" &&
    (targetKind === "batch" || targetKind === "meshGroup")
  ) {
    return true;
  }

  // Texture -> Material
  if (payload.kind === "texture" && targetKind === "material") {
    return true;
  }

  // Batch -> MeshGroup or LOD
  if (
    payload.kind === "batch" &&
    (targetKind === "meshGroup" || targetKind === "lod")
  ) {
    return true;
  }

  // Bone -> Bone reparenting (onto another bone or onto Bones section header)
  if (payload.kind === "bone" && targetKind === "bone") {
    const draggedId = payload.ids[0] ?? "";
    if (!draggedId || draggedId === targetId) return false;
    if (targetId !== "sec:bones" && canReparentBone) {
      return canReparentBone(draggedId, targetId);
    }
    return true;
  }

  // Reorder within same kind (material, meshGroup, batch)
  if (
    payload.kind === targetKind &&
    payload.kind !== "bone" &&
    (position === "before" || position === "after")
  ) {
    return true;
  }

  return false;
}
// [wmb-studio END]
