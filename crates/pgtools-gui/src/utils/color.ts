export function rgbFloatsToHex(r: number, g: number, b: number): string {
  const clampByte = (v: number) =>
    Math.max(0, Math.min(255, Math.round((v || 0) * 255)));
  const hr = clampByte(r).toString(16).padStart(2, "0");
  const hg = clampByte(g).toString(16).padStart(2, "0");
  const hb = clampByte(b).toString(16).padStart(2, "0");
  return `#${hr}${hg}${hb}`;
}

export function rgbFloatToHex(rgb: number[]): string {
  return rgbFloatsToHex(rgb[0] ?? 0, rgb[1] ?? 0, rgb[2] ?? 0);
}

export function hexToRgbFloat(hex: string): [number, number, number] {
  const clean = hex.replace(/^#/, "");
  if (clean.length !== 6) return [1, 1, 1];
  const r = parseInt(clean.slice(0, 2), 16) / 255;
  const g = parseInt(clean.slice(2, 4), 16) / 255;
  const b = parseInt(clean.slice(4, 6), 16) / 255;
  return [
    Number.isFinite(r) ? Number(r.toFixed(4)) : 1,
    Number.isFinite(g) ? Number(g.toFixed(4)) : 1,
    Number.isFinite(b) ? Number(b.toFixed(4)) : 1,
  ];
}

export const hexToRgbFloats = hexToRgbFloat;

export function dwordToFloat(dword: number): number {
  const buf = new ArrayBuffer(4);
  const view = new DataView(buf);
  view.setUint32(0, dword >>> 0, true);
  const f = view.getFloat32(0, true);
  return Number.isFinite(f) ? Number(f.toFixed(4)) : 0;
}

export function isUnitRangeScalarName(name: string): boolean {
  const low = (name || "").toLowerCase();
  return (
    low.includes("metal") ||
    low.includes("rough") ||
    low.includes("smooth") ||
    low.includes("alpha") ||
    low.includes("opacity") ||
    low.includes("blend") ||
    low.includes("weight") ||
    low.includes("ratio") ||
    low.includes("factor") ||
    low.includes("occlusion") ||
    low.includes("reflect") ||
    low.includes("spec") ||
    low.includes("gloss") ||
    low.includes("color") ||
    low.includes("albedo") ||
    low.includes("diffuse")
  );
}

export function isColorLikeParamName(name: string, flagged?: boolean): boolean {
  if (flagged) return true;
  const low = (name || "").toLowerCase();
  return (
    low.includes("color") ||
    low.includes("albedo") ||
    low.includes("tint") ||
    low.includes("emissive") ||
    low.includes("diffuse") ||
    low.includes("ambient") ||
    low.includes("specular") ||
    low.includes("light_col") ||
    low.includes("sss_color") ||
    low.includes("dot_color")
  );
}
