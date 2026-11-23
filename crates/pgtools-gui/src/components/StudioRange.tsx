import React, { useRef, useState } from "react";

interface StudioRangeProps {
  min: number;
  max: number;
  step?: number;
  value: number;
  defaultValue?: number;
  compact?: boolean;
  ticks?: number;
  onChange: (value: number) => void;
  onPointerUp?: () => void;
  onKeyUp?: () => void;
  disabled?: boolean;
  className?: string;
  style?: React.CSSProperties;
  "data-tooltip"?: string;
  ariaLabel?: string;
}

export const StudioRange: React.FC<StudioRangeProps> = ({
  min,
  max,
  step = 1,
  value,
  defaultValue,
  compact = false,
  ticks = 0,
  onChange,
  onPointerUp,
  onKeyUp,
  disabled = false,
  className = "",
  style,
  "data-tooltip": dataTooltip,
  ariaLabel,
}) => {
  const trackRef = useRef<HTMLDivElement | null>(null);
  const dragStartRef = useRef<{ x: number; val: number } | null>(null);
  const [dragging, setDragging] = useState(false);

  const span = max - min;
  const clamped = Math.max(min, Math.min(max, Number.isFinite(value) ? value : min));
  const pct = span > 0 ? ((clamped - min) / span) * 100 : 0;

  const snap = (raw: number, fine: boolean): number => {
    const bounded = Math.max(min, Math.min(max, raw));
    const effStep = fine && step > 0 && step < 1 ? step * 0.1 : step;
    if (!effStep || effStep <= 0) return bounded;
    const steps = Math.round((bounded - min) / effStep);
    const snapped = min + steps * effStep;
    const precision = effStep < 1 ? Math.min(6, String(effStep).split(".")[1]?.length ?? 4) : 0;
    return Math.max(min, Math.min(max, Number(snapped.toFixed(precision))));
  };

  const updateFromClientX = (clientX: number, shiftKey: boolean) => {
    const el = trackRef.current;
    if (!el || disabled) return;
    const rect = el.getBoundingClientRect();
    let raw: number;
    if (shiftKey && dragStartRef.current) {
      const dx = clientX - dragStartRef.current.x;
      raw = dragStartRef.current.val + (dx / Math.max(1, rect.width)) * span * 0.15;
    } else {
      const ratio = Math.max(0, Math.min(1, (clientX - rect.left) / Math.max(1, rect.width)));
      raw = min + ratio * span;
    }
    const next = snap(raw, shiftKey);
    if (next !== clamped) {
      onChange(next);
    }
  };

  const tickCount = ticks > 2 ? Math.min(ticks, 24) : 0;

  return (
    <div
      ref={trackRef}
      role="slider"
      tabIndex={disabled ? -1 : 0}
      aria-label={ariaLabel}
      aria-valuemin={min}
      aria-valuemax={max}
      aria-valuenow={clamped}
      aria-disabled={disabled}
      data-tooltip={dataTooltip}
      className={`studio-range${compact ? " compact" : ""}${dragging ? " dragging" : ""}${
        disabled ? " disabled" : ""
      }${className ? ` ${className}` : ""}`}
      style={style}
      onPointerDown={(e) => {
        if (disabled) return;
        e.preventDefault();
        (e.currentTarget as HTMLDivElement).setPointerCapture(e.pointerId);
        e.currentTarget.focus();
        dragStartRef.current = { x: e.clientX, val: clamped };
        setDragging(true);
        updateFromClientX(e.clientX, e.shiftKey);
      }}
      onPointerMove={(e) => {
        if (disabled || !dragging) return;
        updateFromClientX(e.clientX, e.shiftKey);
      }}
      onPointerUp={() => {
        if (dragging) {
          setDragging(false);
          dragStartRef.current = null;
          if (!disabled) onPointerUp?.();
        }
      }}
      onPointerCancel={() => {
        setDragging(false);
        dragStartRef.current = null;
      }}
      onDoubleClick={(e) => {
        if (disabled || defaultValue === undefined) return;
        e.preventDefault();
        const resetVal = snap(defaultValue, false);
        onChange(resetVal);
        onPointerUp?.();
      }}
      onKeyDown={(e) => {
        if (disabled) return;
        const delta = step > 0 ? step : span / 100;
        if (e.key === "ArrowRight" || e.key === "ArrowUp") {
          e.preventDefault();
          onChange(snap(clamped + delta, e.shiftKey));
        } else if (e.key === "ArrowLeft" || e.key === "ArrowDown") {
          e.preventDefault();
          onChange(snap(clamped - delta, e.shiftKey));
        } else if (e.key === "Home") {
          e.preventDefault();
          onChange(min);
        } else if (e.key === "End") {
          e.preventDefault();
          onChange(max);
        }
      }}
      onKeyUp={(e) => {
        if (
          !disabled &&
          (e.key === "ArrowRight" ||
            e.key === "ArrowLeft" ||
            e.key === "ArrowUp" ||
            e.key === "ArrowDown" ||
            e.key === "Home" ||
            e.key === "End")
        ) {
          onKeyUp?.();
        }
      }}
    >
      <div className="studio-range-fill" style={{ width: `${pct}%` }} />
      <div
        className="studio-range-edge"
        style={{ left: `${pct}%`, opacity: pct <= 0.5 ? 0 : 1 }}
      />
      {tickCount > 0 && (
        <div className="studio-range-ticks" aria-hidden="true">
          {Array.from({ length: tickCount - 2 }).map((_, idx) => (
            <div
              key={idx}
              className="studio-range-tick"
              style={{ left: `${((idx + 1) / (tickCount - 1)) * 100}%` }}
            />
          ))}
        </div>
      )}
    </div>
  );
};
