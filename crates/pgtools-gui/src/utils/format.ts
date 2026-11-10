export function formatBytes(bytes: number): string {
  if (bytes === 0) return "0 B";
  if (bytes < 1024) return `${bytes} B`;
  const kb = bytes / 1024;
  if (kb < 1024) return `${kb.toFixed(1)} KiB`;
  return `${(kb / 1024).toFixed(2)} MiB`;
}

export function compileGlobFilter(pattern: string): RegExp | null {
  const trimmed = pattern.trim();
  if (!trimmed) return null;
  if (!trimmed.includes("*") && !trimmed.includes("?")) {
    const escaped = trimmed.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
    return new RegExp(escaped, "i");
  }
  const regexStr =
    "^" +
    trimmed
      .replace(/[.+^${}()|[\]\\]/g, "\\$&")
      .replace(/\*/g, ".*")
      .replace(/\?/g, ".") +
    "$";
  try {
    return new RegExp(regexStr, "i");
  } catch {
    return null;
  }
}

const HEX_U32_REGEX = /^(?:0x)?[0-9a-f]{1,8}$/i;

export function parseStrictHexU32(input: string): number | null {
  const trimmed = input.trim();
  if (!HEX_U32_REGEX.test(trimmed)) return null;
  const clean = trimmed.replace(/^0x/i, "");
  const parsed = parseInt(clean, 16);
  return Number.isNaN(parsed) ? null : parsed >>> 0;
}

export function formatHexId(id: number): string {
  return `0x${(id >>> 0).toString(16).toUpperCase().padStart(8, "0")}`;
}

export function formatHexU8(val: number): string {
  return `0x${(val & 0xff).toString(16).toUpperCase().padStart(2, "0")}`;
}

export function parseReorderableNode(nodeId: string): [string, number] | null {
  const m = nodeId.match(/^(.*?(?:dat|pkz|efffile|uvdgrp|uvditem)):(\d+)$/);
  if (!m) return null;
  return [m[1], parseInt(m[2], 10)];
}

export function isVirtualSubNode(nodeId: string): boolean {
  const leaf = nodeId.split("/").pop() || "";
  return (
    leaf.startsWith("wmb:") ||
    leaf.startsWith("wmb3:") ||
    leaf.startsWith("tex:") ||
    leaf.startsWith("uvdgrp:") ||
    leaf.startsWith("uvditem:")
  );
}
