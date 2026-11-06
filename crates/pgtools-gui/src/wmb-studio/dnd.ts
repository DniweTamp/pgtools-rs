import { SelectionKind } from "./store";

export type DropPosition = "before" | "onto" | "after";

export interface DragPayload {
  kind: "texture" | "material" | "batch" | "meshGroup" | "bone";
  ids: string[];
  sourceParentId?: string;
}

export function computeDropPositionAt(
  clientY: number,
  element: { getBoundingClientRect: () => { top: number; height: number } },
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

export function isDropCompatible(
  payload: DragPayload,
  targetKind: SelectionKind,
  targetId: string,
  position: DropPosition,
  canReparentBone?: (draggedBoneId: string, targetBoneId: string) => boolean
): boolean {
  if (payload.ids.includes(targetId)) return false;

  if (
    payload.kind === "material" &&
    (targetKind === "batch" || targetKind === "meshGroup")
  ) {
    return true;
  }

  if (payload.kind === "texture" && targetKind === "material") {
    return true;
  }

  if (
    payload.kind === "batch" &&
    (targetKind === "meshGroup" || targetKind === "lod")
  ) {
    return true;
  }

  if (payload.kind === "bone" && targetKind === "bone") {
    const draggedId = payload.ids[0] ?? "";
    if (!draggedId || draggedId === targetId) return false;
    if (targetId !== "sec:bones" && canReparentBone) {
      return canReparentBone(draggedId, targetId);
    }
    return true;
  }

  if (
    payload.kind === targetKind &&
    payload.kind !== "bone" &&
    (position === "before" || position === "after")
  ) {
    return true;
  }

  return false;
}
