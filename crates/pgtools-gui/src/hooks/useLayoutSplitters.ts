import { RefObject, useEffect } from "react";

export type ActiveSplitter = "left" | "right" | "left-row" | null;

export function useLayoutSplitters(
  draggingSplit: ActiveSplitter,
  setDraggingSplit: (next: ActiveSplitter) => void,
  leftColRef: RefObject<HTMLDivElement | null>,
  setLeftWidth: (w: number) => void,
  setRightWidth: (w: number) => void,
  setAssetRatio: (r: number) => void
) {
  useEffect(() => {
    if (!draggingSplit) return;
    const onMove = (e: MouseEvent) => {
      if (draggingSplit === "left") {
        setLeftWidth(Math.max(220, Math.min(620, e.clientX - 4)));
      } else if (draggingSplit === "right") {
        setRightWidth(Math.max(290, Math.min(680, window.innerWidth - e.clientX - 4)));
      } else if (draggingSplit === "left-row" && leftColRef.current) {
        const rect = leftColRef.current.getBoundingClientRect();
        const rel = (e.clientY - rect.top) / Math.max(1, rect.height);
        setAssetRatio(Math.max(0.18, Math.min(0.82, rel)));
      }
    };
    const onUp = () => setDraggingSplit(null);
    window.addEventListener("mousemove", onMove);
    window.addEventListener("mouseup", onUp);
    return () => {
      window.removeEventListener("mousemove", onMove);
      window.removeEventListener("mouseup", onUp);
    };
  }, [draggingSplit, setDraggingSplit, leftColRef, setAssetRatio, setLeftWidth, setRightWidth]);
}
