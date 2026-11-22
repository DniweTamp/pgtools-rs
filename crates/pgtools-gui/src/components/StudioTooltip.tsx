import React, { useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";

interface TooltipState {
  text: string;
  x: number;
  y: number;
  placement: "top" | "bottom";
}

export const StudioTooltipHost: React.FC = () => {
  const [tip, setTip] = useState<TooltipState | null>(null);
  const timerRef = useRef<number | null>(null);
  const activeElRef = useRef<HTMLElement | null>(null);
  const lastHideRef = useRef<number>(0);

  useEffect(() => {
    const clearTimer = () => {
      if (timerRef.current !== null) {
        window.clearTimeout(timerRef.current);
        timerRef.current = null;
      }
    };

    const hide = () => {
      clearTimer();
      activeElRef.current = null;
      setTip((prev) => {
        if (prev) lastHideRef.current = Date.now();
        return null;
      });
    };

    const showFor = (el: HTMLElement) => {
      const text = el.getAttribute("data-tooltip");
      if (!text || !text.trim()) {
        hide();
        return;
      }
      const rect = el.getBoundingClientRect();
      if (rect.width === 0 && rect.height === 0) {
        hide();
        return;
      }
      const vw = window.innerWidth;
      const centerX = rect.left + rect.width / 2;
      const clampedX = Math.max(12, Math.min(vw - 12, centerX));
      const placeBottom = rect.top < 44;
      const y = placeBottom ? rect.bottom + 7 : rect.top - 7;
      setTip({
        text,
        x: clampedX,
        y,
        placement: placeBottom ? "bottom" : "top",
      });
    };

    const onPointerOver = (e: PointerEvent) => {
      const target = e.target as Element | null;
      const el = target?.closest<HTMLElement>("[data-tooltip]") ?? null;
      if (!el) {
        if (activeElRef.current) hide();
        return;
      }
      if (el === activeElRef.current) return;
      clearTimer();
      activeElRef.current = el;
      const warm = Date.now() - lastHideRef.current < 250;
      timerRef.current = window.setTimeout(() => {
        if (activeElRef.current === el && document.contains(el)) {
          showFor(el);
        }
      }, warm ? 60 : 380);
    };

    const onPointerOut = (e: PointerEvent) => {
      if (!activeElRef.current) return;
      const next = e.relatedTarget as Element | null;
      if (next && activeElRef.current.contains(next)) return;
      hide();
    };

    document.addEventListener("pointerover", onPointerOver, true);
    document.addEventListener("pointerout", onPointerOut, true);
    document.addEventListener("pointerdown", hide, true);
    document.addEventListener("keydown", hide, true);
    window.addEventListener("scroll", hide, true);
    window.addEventListener("blur", hide);

    return () => {
      clearTimer();
      document.removeEventListener("pointerover", onPointerOver, true);
      document.removeEventListener("pointerout", onPointerOut, true);
      document.removeEventListener("pointerdown", hide, true);
      document.removeEventListener("keydown", hide, true);
      window.removeEventListener("scroll", hide, true);
      window.removeEventListener("blur", hide);
    };
  }, []);

  if (!tip) return null;

  return createPortal(
    <div
      className={`studio-tooltip studio-tooltip-${tip.placement}`}
      style={{ left: tip.x, top: tip.y }}
      role="tooltip"
    >
      {tip.text}
    </div>,
    document.body
  );
};
