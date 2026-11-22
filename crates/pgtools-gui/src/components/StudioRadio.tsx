import React from "react";

export interface StudioRadioProps {
  checked: boolean;
  onChange: () => void;
  disabled?: boolean;
  className?: string;
  style?: React.CSSProperties;
  circleStyle?: React.CSSProperties;
  children?: React.ReactNode;
}

export const StudioRadio: React.FC<StudioRadioProps> = ({
  checked,
  onChange,
  disabled = false,
  className = "",
  style,
  circleStyle,
  children,
}) => {
  return (
    <div
      role="radio"
      aria-checked={checked}
      aria-disabled={disabled}
      tabIndex={disabled ? -1 : 0}
      className={`studio-radio${checked ? " checked" : ""}${disabled ? " disabled" : ""}${
        className ? ` ${className}` : ""
      }`}
      style={style}
      onClick={() => {
        if (!disabled && !checked) onChange();
      }}
      onKeyDown={(e) => {
        if (disabled) return;
        if (e.key === " " || e.key === "Enter") {
          e.preventDefault();
          if (!checked) onChange();
        }
      }}
    >
      <span className="studio-radio-circle" style={circleStyle}>
        {checked && <span className="studio-radio-dot" />}
      </span>
      {children}
    </div>
  );
};
