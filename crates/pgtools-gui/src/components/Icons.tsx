import React from "react";

export const IconUndo = () => (
  <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
    <path d="M3 7v6h6" />
    <path d="M21 17a9 9 0 0 0-9-9 9 9 0 0 0-6 2.3L3 13" />
  </svg>
);

export const IconRedo = () => (
  <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
    <path d="M21 7v6h-6" />
    <path d="M3 17a9 9 0 0 1 9-9 9 9 0 0 1 6 2.3l3 2.7" />
  </svg>
);

export const IconRefresh = () => (
  <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
    <path d="M21 2v6h-6" />
    <path d="M3 12a9 9 0 0 1 15-6.7L21 8" />
    <path d="M3 22v-6h6" />
    <path d="M21 12a9 9 0 0 1-15 6.7L3 16" />
  </svg>
);

export const IconSunMoon = () => (
  <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
    <circle cx="12" cy="12" r="4" />
    <path d="M12 2v2M12 20v2M4.93 4.93l1.41 1.41M17.66 17.66l1.41 1.41M2 12h2M20 12h2M6.34 17.66l-1.41 1.41M19.07 4.93l-1.41 1.41" />
  </svg>
);

export const IconFolder = () => (
  <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
    <path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z" />
  </svg>
);

export const IconImage = ({ size = 14 }: { size?: number }) => (
  <svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
    <rect x="3" y="3" width="18" height="18" rx="2" ry="2" />
    <circle cx="8.5" cy="8.5" r="1.5" />
    <polyline points="21 15 16 10 5 21" />
  </svg>
);

export const IconCube = ({ size = 14 }: { size?: number }) => (
  <svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
    <path d="M21 16V8a2 2 0 0 0-1-1.73l-7-4a2 2 0 0 0-2 0l-7 4A2 2 0 0 0 3 8v8a2 2 0 0 0 1 1.73l7 4a2 2 0 0 0 2 0l7-4A2 2 0 0 0 21 16z" />
    <polyline points="3.27 6.96 12 12.01 20.73 6.96" />
    <line x1="12" y1="22.08" x2="12" y2="12" />
  </svg>
);

export const IconCode = ({ size = 14 }: { size?: number }) => (
  <svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
    <polyline points="16 18 22 12 16 6" />
    <polyline points="8 6 2 12 8 18" />
  </svg>
);

export const IconChevronDown = ({ collapsed }: { collapsed?: boolean }) => (
  <svg
    width="12"
    height="12"
    viewBox="0 0 24 24"
    fill="none"
    stroke="currentColor"
    strokeWidth="2.2"
    style={{
      transform: collapsed ? "rotate(-90deg)" : "rotate(0deg)",
      transition: "transform 0.12s ease",
    }}
  >
    <polyline points="6 9 12 15 18 9" />
  </svg>
);

export const IconCheck = () => (
  <svg width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="3">
    <polyline points="20 6 9 17 4 12" />
  </svg>
);

export function IconChevronRight({ expanded, className = "" }: { expanded?: boolean; className?: string }) {
  return (
    <svg
      className={`chevron-icon${expanded ? " expanded" : ""}${className ? ` ${className}` : ""}`}
      viewBox="0 0 12 12"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.7"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      <path d="M4.25 2.5L7.75 6L4.25 9.5" />
    </svg>
  );
}

export function IconEye({ visible }: { visible: boolean }) {
  if (visible) {
    return (
      <svg viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
        <path d="M1.5 8C2.9 5.1 5.3 3.5 8 3.5C10.7 3.5 13.1 5.1 14.5 8C13.1 10.9 10.7 12.5 8 12.5C5.3 12.5 2.9 10.9 1.5 8Z" />
        <circle cx="8" cy="8" r="2.1" />
      </svg>
    );
  }
  return (
    <svg viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
      <path d="M2.2 2.2L13.8 13.8" />
      <path d="M6.5 3.7C7 3.57 7.5 3.5 8 3.5C10.7 3.5 13.1 5.1 14.5 8C13.88 9.28 13.02 10.33 12 11.08" />
      <path d="M4.2 5.3C3.07 6.02 2.15 6.94 1.5 8C2.9 10.9 5.3 12.5 8 12.5C9.1 12.5 10.15 12.23 11.1 11.74" />
    </svg>
  );
}

export function IconDuplicate() {
  return (
    <svg viewBox="0 0 14 14" fill="none" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
      <rect x="4.5" y="4.5" width="7.5" height="7.5" rx="1.3" />
      <path d="M2.5 9.5H2.2C1.8 9.5 1.5 9.2 1.5 8.8V2.2C1.5 1.8 1.8 1.5 2.2 1.5H8.8C9.2 1.5 9.5 1.8 9.5 2.2V2.5" />
    </svg>
  );
}

export function IconDelete() {
  return (
    <svg viewBox="0 0 14 14" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
      <path d="M3.25 3.25L10.75 10.75" />
      <path d="M10.75 3.25L3.25 10.75" />
    </svg>
  );
}

export function IconShadow() {
  return (
    <svg viewBox="0 0 14 14" fill="none" stroke="currentColor" strokeWidth="1.3" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
      <rect x="2" y="2" width="6.8" height="6.8" rx="1.1" />
      <path d="M5.5 8.8V10.9C5.5 11.5 6 12 6.6 12H10.9C11.5 12 12 11.5 12 10.9V6.6C12 6 11.5 5.5 10.9 5.5H8.8" strokeDasharray="1.6 1.4" />
    </svg>
  );
}
