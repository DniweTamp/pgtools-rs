import React, { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { IconCheck } from "./Icons";

export interface StudioSelectOption {
  value: string;
  label: string;
  disabled?: boolean;
}

export interface StudioSelectGroup {
  label: string;
  options: StudioSelectOption[];
}

export interface StudioSelectProps {
  value: string;
  options?: StudioSelectOption[];
  groups?: StudioSelectGroup[];
  onChange: (value: string) => void;
  disabled?: boolean;
  className?: string;
  style?: React.CSSProperties;
  placeholder?: string;
  "data-tooltip"?: string;
  ariaLabel?: string;
}

export const StudioSelect: React.FC<StudioSelectProps> = ({
  value,
  options,
  groups,
  onChange,
  disabled = false,
  className = "pill-select",
  style,
  placeholder = "Select...",
  "data-tooltip": dataTooltip,
  ariaLabel,
}) => {
  const [open, setOpen] = useState(false);
  const [highlightIdx, setHighlightIdx] = useState(-1);
  const [menuPos, setMenuPos] = useState<{
    left: number;
    top: number;
    minWidth: number;
    maxHeight: number;
  }>({ left: 0, top: 0, minWidth: 120, maxHeight: 280 });

  const triggerRef = useRef<HTMLButtonElement | null>(null);
  const menuRef = useRef<HTMLDivElement | null>(null);

  const flatOptions = useMemo<StudioSelectOption[]>(() => {
    if (groups && groups.length > 0) {
      return groups.flatMap((g) => g.options);
    }
    return options ?? [];
  }, [options, groups]);

  const selectedOption = useMemo(
    () => flatOptions.find((o) => o.value === value) ?? null,
    [flatOptions, value]
  );

  const updatePosition = () => {
    const el = triggerRef.current;
    if (!el) return;
    const rect = el.getBoundingClientRect();
    const vw = window.innerWidth;
    const vh = window.innerHeight;
    const desiredMaxH = 280;
    const spaceBelow = vh - rect.bottom - 8;
    const spaceAbove = rect.top - 8;
    const openUp = spaceBelow < 160 && spaceAbove > spaceBelow;
    const maxHeight = Math.max(
      100,
      Math.min(desiredMaxH, openUp ? spaceAbove : spaceBelow)
    );
    const minWidth = Math.max(rect.width, 120);
    const left = Math.max(8, Math.min(rect.left, vw - minWidth - 8));
    const top = openUp
      ? Math.max(8, rect.top - Math.min(maxHeight, flatOptions.length * 28 + 36) - 4)
      : rect.bottom + 4;
    setMenuPos({ left, top, minWidth, maxHeight });
  };

  useLayoutEffect(() => {
    if (!open) return;
    updatePosition();
    const curIdx = flatOptions.findIndex((o) => o.value === value && !o.disabled);
    setHighlightIdx(curIdx >= 0 ? curIdx : flatOptions.findIndex((o) => !o.disabled));
  }, [open, value, flatOptions]);

  useEffect(() => {
    if (!open) return;
    const onPointerDown = (e: MouseEvent) => {
      const target = e.target as Node | null;
      if (
        triggerRef.current?.contains(target) ||
        menuRef.current?.contains(target)
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
    const onResize = () => updatePosition();
    document.addEventListener("mousedown", onPointerDown, true);
    document.addEventListener("keydown", onKeyDown, true);
    window.addEventListener("resize", onResize);
    return () => {
      document.removeEventListener("mousedown", onPointerDown, true);
      document.removeEventListener("keydown", onKeyDown, true);
      window.removeEventListener("resize", onResize);
    };
  }, [open]);

  useEffect(() => {
    if (!open || highlightIdx < 0 || !menuRef.current) return;
    const el = menuRef.current.querySelector<HTMLElement>(
      `[data-opt-idx="${highlightIdx}"]`
    );
    el?.scrollIntoView({ block: "nearest" });
  }, [open, highlightIdx]);

  const moveHighlight = (dir: 1 | -1) => {
    if (flatOptions.length === 0) return;
    let idx = highlightIdx;
    for (let step = 0; step < flatOptions.length; step++) {
      idx = (idx + dir + flatOptions.length) % flatOptions.length;
      if (!flatOptions[idx].disabled) {
        setHighlightIdx(idx);
        return;
      }
    }
  };

  const commitIndex = (idx: number) => {
    const opt = flatOptions[idx];
    if (!opt || opt.disabled) return;
    onChange(opt.value);
    setOpen(false);
    triggerRef.current?.focus();
  };

  const isMono = /\bmono\b/.test(className);

  const renderOption = (opt: StudioSelectOption, flatIdx: number) => {
    const isSelected = opt.value === value;
    const isHighlighted = flatIdx === highlightIdx;
    return (
      <button
        key={`${flatIdx}:${opt.value}`}
        type="button"
        role="option"
        aria-selected={isSelected}
        disabled={opt.disabled}
        data-opt-idx={flatIdx}
        className={`studio-select-option${isSelected ? " selected" : ""}${
          isHighlighted ? " highlighted" : ""
        }${isMono ? " mono" : ""}`}
        onMouseEnter={() => {
          if (!opt.disabled) setHighlightIdx(flatIdx);
        }}
        onClick={(e) => {
          e.stopPropagation();
          commitIndex(flatIdx);
        }}
      >
        <span className="studio-select-option-label">{opt.label}</span>
        <span className="studio-select-option-check">
          {isSelected && <IconCheck />}
        </span>
      </button>
    );
  };

  let runningIdx = 0;

  return (
    <>
      <button
        ref={triggerRef}
        type="button"
        role="combobox"
        aria-expanded={open}
        aria-haspopup="listbox"
        aria-label={ariaLabel}
        disabled={disabled}
        data-tooltip={dataTooltip}
        className={`studio-select-trigger ${className}${open ? " open" : ""}`}
        style={style}
        onClick={(e) => {
          e.stopPropagation();
          if (!disabled) setOpen((prev) => !prev);
        }}
        onKeyDown={(e) => {
          if (disabled) return;
          if (e.key === "ArrowDown") {
            e.preventDefault();
            if (!open) setOpen(true);
            else moveHighlight(1);
          } else if (e.key === "ArrowUp") {
            e.preventDefault();
            if (!open) setOpen(true);
            else moveHighlight(-1);
          } else if (e.key === "Enter" || e.key === " ") {
            e.preventDefault();
            if (!open) setOpen(true);
            else if (highlightIdx >= 0) commitIndex(highlightIdx);
          } else if (e.key === "Home" && open) {
            e.preventDefault();
            const first = flatOptions.findIndex((o) => !o.disabled);
            if (first >= 0) setHighlightIdx(first);
          } else if (e.key === "End" && open) {
            e.preventDefault();
            for (let i = flatOptions.length - 1; i >= 0; i--) {
              if (!flatOptions[i].disabled) {
                setHighlightIdx(i);
                break;
              }
            }
          }
        }}
      >
        <span className="studio-select-value">
          {selectedOption ? selectedOption.label : placeholder}
        </span>
        <svg
          className="studio-select-chevron"
          width="10"
          height="6"
          viewBox="0 0 10 6"
          aria-hidden="true"
        >
          <path
            d="M1 1l4 4 4-4"
            stroke="currentColor"
            strokeWidth="1.5"
            fill="none"
            strokeLinecap="round"
            strokeLinejoin="round"
          />
        </svg>
      </button>

      {open &&
        createPortal(
          <div
            ref={menuRef}
            role="listbox"
            className="studio-select-menu"
            style={{
              left: menuPos.left,
              top: menuPos.top,
              minWidth: menuPos.minWidth,
              maxHeight: menuPos.maxHeight,
            }}
            onClick={(e) => e.stopPropagation()}
          >
            {groups && groups.length > 0
              ? groups.map((grp) => (
                  <div key={grp.label} className="studio-select-group">
                    <div className="studio-select-group-label">{grp.label}</div>
                    {grp.options.map((opt) => {
                      const idx = runningIdx++;
                      return renderOption(opt, idx);
                    })}
                  </div>
                ))
              : flatOptions.map((opt, idx) => renderOption(opt, idx))}
          </div>,
          document.body
        )}
    </>
  );
};
