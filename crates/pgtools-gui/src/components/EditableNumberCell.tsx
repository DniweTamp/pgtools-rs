import React, { useEffect, useState } from "react";

export function EditableNumberCell({
  value,
  step,
  isFloat = false,
  decimals,
  style,
  className = "field-box mono",
  title,
  onClick,
  onChangeValue,
}: {
  value: number;
  step?: string | number;
  isFloat?: boolean;
  decimals?: number;
  style?: React.CSSProperties;
  className?: string;
  title?: string;
  onClick?: (e: React.MouseEvent<HTMLInputElement>) => void;
  onChangeValue: (next: number) => void;
}) {
  const formatVal = (v: number) => {
    const safe = Number.isFinite(v) ? v : 0;
    return decimals !== undefined ? String(Number(safe.toFixed(decimals))) : String(safe);
  };
  const [draft, setDraft] = useState<string>(() => formatVal(value));
  const [focused, setFocused] = useState<boolean>(false);

  useEffect(() => {
    if (!focused) {
      setDraft(formatVal(value));
    }
  }, [value, focused, decimals]);

  return (
    <input
      type="text"
      inputMode={isFloat ? "decimal" : "numeric"}
      step={step}
      size={1}
      className={className}
      style={style}
      data-tooltip={title}
      value={focused ? draft : formatVal(value)}
      onClick={onClick}
      onFocus={() => {
        setFocused(true);
        setDraft(formatVal(value));
      }}
      onChange={(e) => {
        const raw = e.target.value;
        setDraft(raw);
        const trimmed = raw.trim();
        if (
          trimmed === "" ||
          trimmed === "-" ||
          trimmed === "." ||
          trimmed === "-." ||
          (isFloat && /^-?\d*\.\d*0+$/.test(trimmed)) ||
          (isFloat && trimmed.endsWith("."))
        ) {
          return;
        }
        const parsed = isFloat ? parseFloat(trimmed) : parseInt(trimmed, 10);
        if (Number.isFinite(parsed)) {
          onChangeValue(parsed);
        }
      }}
      onBlur={() => {
        setFocused(false);
        const trimmed = draft.trim();
        const parsed = isFloat ? parseFloat(trimmed) : parseInt(trimmed, 10);
        if (Number.isFinite(parsed)) {
          onChangeValue(parsed);
          setDraft(formatVal(parsed));
        } else {
          setDraft(formatVal(value));
        }
      }}
      onKeyDown={(e) => {
        if (e.key === "Enter") {
          (e.target as HTMLInputElement).blur();
        }
      }}
    />
  );
}
