export const DAT_ALIGN_PRESETS = [16, 64, 512, 1024, 2048, 4096, 8192] as const;
export const DEFAULT_CUSTOM_DAT_ALIGNMENT = 2048;
export const MAX_CUSTOM_DAT_ALIGNMENT = 1048576;

export function sanitizeCustomDatAlignment(val?: number | null): number {
  if (typeof val !== "number" || !Number.isFinite(val) || val <= 0) {
    return DEFAULT_CUSTOM_DAT_ALIGNMENT;
  }
  return Math.max(1, Math.min(MAX_CUSTOM_DAT_ALIGNMENT, Math.round(val)));
}

export function parseCustomDatAlignmentInput(raw: string | number): number | null {
  if (typeof raw === "number") {
    if (!Number.isFinite(raw) || raw <= 0 || raw > MAX_CUSTOM_DAT_ALIGNMENT) return null;
    return Math.round(raw);
  }
  const trimmed = (raw ?? "").trim().toLowerCase().replace(/_/g, "");
  if (!trimmed) return null;
  let num = NaN;
  if (trimmed.startsWith("0x")) {
    if (/^0x[0-9a-f]+$/.test(trimmed)) {
      num = parseInt(trimmed.slice(2), 16);
    }
  } else if (/^\d+$/.test(trimmed)) {
    num = parseInt(trimmed, 10);
  }
  if (!Number.isFinite(num) || num <= 0 || num > MAX_CUSTOM_DAT_ALIGNMENT) {
    return null;
  }
  return num;
}
