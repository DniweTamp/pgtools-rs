import React, { useRef } from "react";

export interface StudioRangeProps {
  min: number;
  max: number;
  step?: number;
  value: number;
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
  const span = max - min;
  const clamped = Math.max(min, Math.min(max, Number.isFinite(value) ? value : min));
  const pct = span > 0 ? ((clamped - min) / span) * 100 : 0;

  const snap = (raw: number): number => {
    const bounded = Math.max(min, Math.min(max, raw));
    if (!step || step <= 0) return bounded;
    const steps = Math.round((bounded - min) / step);
    const snapped = min + steps * step;
    const precision = step < 1 ? Math.min(6, String(step).split(".")[1]?.length ?? 4) : 0;
    return Math.max(min, Math.min(max, Number(snapped.toFixed(precision))));
  };

  const updateFromClientX = (clientX: number) => {
    const el = trackRef.current;
    if (!el || disabled) return;
    const rect = el.getBoundingClientRect();
    const ratio = Math.max(0, Math.min(1, (clientX - rect.left) / Math.max(1, rect.width)));
    onChange(snap(min + ratio * span));
  };

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
      className={`studio-range${disabled ? " disabled" : ""}${className ? ` ${className}` : ""}`}
      style={style}
      onPointerDown={(e) => {
        if (disabled) return;
        e.preventDefault();
        (e.currentTarget as HTMLDivElement).setPointerCapture(e.pointerId);
        e.currentTarget.focus();
        updateFromClientX(e.clientX);
      }}
      onPointerMove={(e) => {
        if (disabled) return;
        if (e.buttons === 1) {
          updateFromClientX(e.clientX);
        }
      }}
      onPointerUp={() => {
        if (!disabled) onPointerUp?.();
      }}
      onKeyDown={(e) => {
        if (disabled) return;
        const delta = step > 0 ? step : span / 100;
        if (e.key === "ArrowRight" || e.key === "ArrowUp") {
          e.preventDefault();
          onChange(snap(clamped + delta));
        } else if (e.key === "ArrowLeft" || e.key === "ArrowDown") {
          e.preventDefault();
          onChange(snap(clamped - delta));
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
      <div className="studio-range-track">
        <div className="studio-range-fill" style={{ width: `${pct}%` }} />
      </div>
      <div className="studio-range-thumb" style={{ left: `${pct}%` }} />
    </div>
  );
};
