import React, { useEffect, useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";

interface StudioColorPickerProps {
  value: string;
  onChange: (hex: string) => void;
  onCommit?: (hex: string) => void;
  disabled?: boolean;
  className?: string;
  style?: React.CSSProperties;
  "data-tooltip"?: string;
}

const PRESET_SWATCHES = [
  "#FFFFFF",
  "#BFBFBF",
  "#808080",
  "#262626",
  "#000000",
  "#E5484D",
  "#F59E0B",
  "#34D399",
  "#4F8DF7",
  "#A78BFA",
];

function clamp01(v: number): number {
  return Math.max(0, Math.min(1, v));
}

function normalizeHex(raw: string): string {
  const m = raw.trim().match(/^#?([0-9a-fA-F]{6})$/);
  if (m) return `#${m[1].toUpperCase()}`;
  const m3 = raw.trim().match(/^#?([0-9a-fA-F]{3})$/);
  if (m3) {
    const s = m3[1];
    return `#${s[0]}${s[0]}${s[1]}${s[1]}${s[2]}${s[2]}`.toUpperCase();
  }
  return "#FFFFFF";
}

function hexToRgb(hex: string): [number, number, number] {
  const n = normalizeHex(hex).slice(1);
  return [
    parseInt(n.slice(0, 2), 16),
    parseInt(n.slice(2, 4), 16),
    parseInt(n.slice(4, 6), 16),
  ];
}

function rgbToHex(r: number, g: number, b: number): string {
  const c = (x: number) =>
    Math.max(0, Math.min(255, Math.round(x)))
      .toString(16)
      .padStart(2, "0")
      .toUpperCase();
  return `#${c(r)}${c(g)}${c(b)}`;
}

function rgbToHsv(r255: number, g255: number, b255: number): [number, number, number] {
  const r = r255 / 255;
  const g = g255 / 255;
  const b = b255 / 255;
  const max = Math.max(r, g, b);
  const min = Math.min(r, g, b);
  const d = max - min;
  let h = 0;
  if (d > 1e-6) {
    if (max === r) h = ((g - b) / d + (g < b ? 6 : 0)) / 6;
    else if (max === g) h = ((b - r) / d + 2) / 6;
    else h = ((r - g) / d + 4) / 6;
  }
  const s = max > 1e-6 ? d / max : 0;
  return [h * 360, s, max];
}

function hsvToRgb(hDeg: number, s: number, v: number): [number, number, number] {
  const h = (((hDeg % 360) + 360) % 360) / 60;
  const c = v * s;
  const x = c * (1 - Math.abs((h % 2) - 1));
  const m = v - c;
  let r = 0, g = 0, b = 0;
  if (h < 1) [r, g, b] = [c, x, 0];
  else if (h < 2) [r, g, b] = [x, c, 0];
  else if (h < 3) [r, g, b] = [0, c, x];
  else if (h < 4) [r, g, b] = [0, x, c];
  else if (h < 5) [r, g, b] = [x, 0, c];
  else [r, g, b] = [c, 0, x];
  return [Math.round((r + m) * 255), Math.round((g + m) * 255), Math.round((b + m) * 255)];
}

export const StudioColorPicker: React.FC<StudioColorPickerProps> = ({
  value,
  onChange,
  onCommit,
  disabled = false,
  className = "",
  style,
  "data-tooltip": dataTooltip,
}) => {
  const normalized = normalizeHex(value);
  const [open, setOpen] = useState(false);
  const [hsv, setHsv] = useState<[number, number, number]>(() =>
    rgbToHsv(...hexToRgb(normalized))
  );
  const [hexDraft, setHexDraft] = useState(normalized);
  const [pos, setPos] = useState<{ left: number; top: number }>({ left: 0, top: 0 });

  const triggerRef = useRef<HTMLButtonElement | null>(null);
  const popRef = useRef<HTMLDivElement | null>(null);
  const svPadRef = useRef<HTMLDivElement | null>(null);
  const hueBarRef = useRef<HTMLDivElement | null>(null);
  const latestHexRef = useRef(normalized);

  useEffect(() => {
    if (!open) {
      setHsv(rgbToHsv(...hexToRgb(normalized)));
      setHexDraft(normalized);
      latestHexRef.current = normalized;
    }
  }, [normalized, open]);

  const updatePosition = () => {
    const el = triggerRef.current;
    if (!el) return;
    const rect = el.getBoundingClientRect();
    const popW = 228;
    const popH = 258;
    const vw = window.innerWidth;
    const vh = window.innerHeight;
    const left = Math.max(8, Math.min(rect.left, vw - popW - 8));
    const top =
      rect.bottom + popH + 8 <= vh
        ? rect.bottom + 6
        : Math.max(8, rect.top - popH - 6);
    setPos({ left, top });
  };

  useLayoutEffect(() => {
    if (!open) return;
    updatePosition();
  }, [open]);

  useEffect(() => {
    if (!open) return;
    const onMouseDown = (e: MouseEvent) => {
      const target = e.target as Node | null;
      if (
        triggerRef.current?.contains(target) ||
        popRef.current?.contains(target)
      ) {
        return;
      }
      setOpen(false);
    };
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        e.stopPropagation();
        setOpen(false);
        triggerRef.current?.focus();
      }
    };
    document.addEventListener("mousedown", onMouseDown, true);
    document.addEventListener("keydown", onKeyDown, true);
    window.addEventListener("resize", updatePosition);
    return () => {
      document.removeEventListener("mousedown", onMouseDown, true);
      document.removeEventListener("keydown", onKeyDown, true);
      window.removeEventListener("resize", updatePosition);
    };
  }, [open]);

  const emitHsv = (nextH: number, nextS: number, nextV: number) => {
    setHsv([nextH, nextS, nextV]);
    const [r, g, b] = hsvToRgb(nextH, nextS, nextV);
    const hex = rgbToHex(r, g, b);
    setHexDraft(hex);
    latestHexRef.current = hex;
    onChange(hex);
  };

  const handleSvPointer = (clientX: number, clientY: number) => {
    const pad = svPadRef.current;
    if (!pad) return;
    const rect = pad.getBoundingClientRect();
    const s = clamp01((clientX - rect.left) / Math.max(1, rect.width));
    const v = 1 - clamp01((clientY - rect.top) / Math.max(1, rect.height));
    emitHsv(hsv[0], s, v);
  };

  const handleHuePointer = (clientX: number) => {
    const bar = hueBarRef.current;
    if (!bar) return;
    const rect = bar.getBoundingClientRect();
    const h = clamp01((clientX - rect.left) / Math.max(1, rect.width)) * 360;
    emitHsv(h, hsv[1], hsv[2]);
  };

  const [curR, curG, curB] = hsvToRgb(hsv[0], hsv[1], hsv[2]);
  const currentHex = rgbToHex(curR, curG, curB);

  return (
    <>
      <button
        ref={triggerRef}
        type="button"
        disabled={disabled}
        data-tooltip={dataTooltip ?? `Pick color (${normalized})`}
        className={`studio-color-swatch ${className}`}
        style={{ backgroundColor: normalized, ...style }}
        onClick={(e) => {
          e.stopPropagation();
          if (!disabled) setOpen((prev) => !prev);
        }}
      />

      {open &&
        createPortal(
          <div
            ref={popRef}
            className="studio-color-popover"
            style={{ left: pos.left, top: pos.top }}
            onClick={(e) => e.stopPropagation()}
          >
            <div
              ref={svPadRef}
              className="studio-color-sv"
              style={{ backgroundColor: `hsl(${hsv[0]}, 100%, 50%)` }}
              onPointerDown={(e) => {
                e.preventDefault();
                (e.currentTarget as HTMLDivElement).setPointerCapture(e.pointerId);
                handleSvPointer(e.clientX, e.clientY);
              }}
              onPointerMove={(e) => {
                if (e.buttons === 1) {
                  handleSvPointer(e.clientX, e.clientY);
                }
              }}
              onPointerUp={() => {
                onCommit?.(latestHexRef.current);
              }}
            >
              <div className="studio-color-sv-white" />
              <div className="studio-color-sv-black" />
              <div
                className="studio-color-sv-thumb"
                style={{
                  left: `${hsv[1] * 100}%`,
                  top: `${(1 - hsv[2]) * 100}%`,
                  backgroundColor: currentHex,
                }}
              />
            </div>

            <div
              ref={hueBarRef}
              className="studio-color-hue"
              onPointerDown={(e) => {
                e.preventDefault();
                (e.currentTarget as HTMLDivElement).setPointerCapture(e.pointerId);
                handleHuePointer(e.clientX);
              }}
              onPointerMove={(e) => {
                if (e.buttons === 1) {
                  handleHuePointer(e.clientX);
                }
              }}
              onPointerUp={() => {
                onCommit?.(latestHexRef.current);
              }}
            >
              <div
                className="studio-color-hue-thumb"
                style={{ left: `${(hsv[0] / 360) * 100}%` }}
              />
            </div>

            <div className="studio-color-controls">
              <div
                className="studio-color-preview"
                style={{ backgroundColor: currentHex }}
              />
              <input
                type="text"
                className="field-box mono studio-color-hex"
                value={hexDraft}
                onChange={(e) => {
                  const raw = e.target.value;
                  setHexDraft(raw);
                  if (/^#?[0-9a-fA-F]{6}$/.test(raw.trim())) {
                    const norm = normalizeHex(raw);
                    setHsv(rgbToHsv(...hexToRgb(norm)));
                    latestHexRef.current = norm;
                    onChange(norm);
                  }
                }}
                onBlur={() => {
                  const norm = normalizeHex(hexDraft);
                  setHexDraft(norm);
                  setHsv(rgbToHsv(...hexToRgb(norm)));
                  latestHexRef.current = norm;
                  onChange(norm);
                  onCommit?.(norm);
                }}
                onKeyDown={(e) => {
                  if (e.key === "Enter") {
                    (e.target as HTMLInputElement).blur();
                  }
                }}
              />
              <span className="studio-color-rgb mono">
                {(curR / 255).toFixed(2)}, {(curG / 255).toFixed(2)}, {(curB / 255).toFixed(2)}
              </span>
            </div>

            <div className="studio-color-presets">
              {PRESET_SWATCHES.map((sw) => (
                <button
                  key={sw}
                  type="button"
                  className="studio-color-preset-btn"
                  style={{ backgroundColor: sw }}
                  data-tooltip={sw}
                  onClick={() => {
                    setHsv(rgbToHsv(...hexToRgb(sw)));
                    setHexDraft(sw);
                    latestHexRef.current = sw;
                    onChange(sw);
                    onCommit?.(sw);
                  }}
                />
              ))}
            </div>
          </div>,
          document.body
        )}
    </>
  );
};
