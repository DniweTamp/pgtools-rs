import React, { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import {
  AssetTreeNode,
  BxmXmlNode,
  CorePretextureInfo,
  DecodedWmbMaterial,
  DocumentOutline,
  FsEntry,
  MaterialBankTextureOption,
  MaterialPickerOption,
  ShaderOptionDesc,
  StudioSettings,
  TextureNodeView,
  Wmb3Material,
  WmbSectionView,
} from "./types";
// [wmb-studio BEGIN phase2]
import { Workspace } from "./wmb-studio/Workspace";
import { createDemoModel } from "./wmb-studio/contract";
import { LegacyActionKind } from "./wmb-studio/Hierarchy";
// [wmb-studio END]

type ThemeMode = "dark" | "light";
type FileSortCol = "name" | "type" | "modified" | "size";

interface MenuItemSpec {
  label?: string;
  shortcut?: string;
  disabled?: boolean;
  separator?: boolean;
  action?: () => void;
  submenu?: MenuItemSpec[];
}

interface ContextMenuState {
  x: number;
  y: number;
  items: MenuItemSpec[];
}

interface PointerDragState {
  kind: "asset" | "bxm";
  sourceId: string;
  label: string;
  startX: number;
  startY: number;
  curX: number;
  curY: number;
  active: boolean;
  overId: string | null;
  insertPos: "before" | "after" | null;
  dragOutTriggered: boolean;
}

function formatBytes(bytes: number): string {
  if (bytes === 0) return "0 B";
  if (bytes < 1024) return `${bytes} B`;
  const kb = bytes / 1024;
  if (kb < 1024) return `${kb.toFixed(1)} KiB`;
  return `${(kb / 1024).toFixed(2)} MiB`;
}

function compileGlobFilter(pattern: string): RegExp | null {
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

function rgbFloatToHex(rgb: number[]): string {
  const clampByte = (v: number) => Math.max(0, Math.min(255, Math.round((v || 0) * 255)));
  const r = clampByte(rgb[0]).toString(16).padStart(2, "0");
  const g = clampByte(rgb[1]).toString(16).padStart(2, "0");
  const b = clampByte(rgb[2]).toString(16).padStart(2, "0");
  return `#${r}${g}${b}`;
}

function hexToRgbFloat(hex: string): [number, number, number] {
  const clean = hex.replace(/^#/, "");
  if (clean.length !== 6) return [1, 1, 1];
  const r = parseInt(clean.slice(0, 2), 16) / 255;
  const g = parseInt(clean.slice(2, 4), 16) / 255;
  const b = parseInt(clean.slice(4, 6), 16) / 255;
  return [Number(r.toFixed(4)), Number(g.toFixed(4)), Number(b.toFixed(4))];
}

function stripInvalidXmlChars(s: string): string {
  return (s ?? "").replace(/[\x00-\x08\x0B\x0C\x0E-\x1F]/g, "");
}

function escapeXmlAttr(s: string): string {
  return stripInvalidXmlChars(s)
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&apos;");
}

function escapeXmlText(s: string): string {
  return stripInvalidXmlChars(s)
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;");
}

function parseXmlToBxmNode(xmlStr: string): BxmXmlNode | null {
  try {
    const parser = new DOMParser();
    const doc = parser.parseFromString(xmlStr, "application/xml");
    if (doc.querySelector("parsererror")) return null;
    const rootEl = doc.documentElement;
    if (!rootEl) return null;
    const convertEl = (el: Element): BxmXmlNode => {
      const attributes: [string, string][] = [];
      for (let i = 0; i < el.attributes.length; i++) {
        const attr = el.attributes[i];
        attributes.push([attr.name, attr.value]);
      }
      const children: BxmXmlNode[] = [];
      let directText = "";
      for (let i = 0; i < el.childNodes.length; i++) {
        const cn = el.childNodes[i];
        if (cn.nodeType === Node.ELEMENT_NODE) {
          children.push(convertEl(cn as Element));
        } else if (cn.nodeType === Node.TEXT_NODE || cn.nodeType === Node.CDATA_SECTION_NODE) {
          directText += cn.nodeValue ?? "";
        }
      }
      const trimmedText = directText.trim();
      return {
        name: el.tagName,
        attributes,
        value: trimmedText.length > 0 ? trimmedText : null,
        children,
      };
    };
    return convertEl(rootEl);
  } catch {
    return null;
  }
}

function serializeBxmNodeToXml(node: BxmXmlNode, indent = 0): string {
  const pad = "  ".repeat(indent);
  const tag = (node.name || "Node").trim() || "Node";
  const attrs = [...(node.attributes || [])]
    .filter(([k]) => k && k.trim().length > 0)
    .map(([k, v]) => ` ${k.trim()}="${escapeXmlAttr(v ?? "")}"`)
    .join("");
  const children = node.children || [];
  const val = node.value !== null && node.value !== undefined ? String(node.value) : "";

  if (children.length === 0 && val.length === 0) {
    return `${pad}<${tag}${attrs}/>`;
  }
  if (children.length === 0) {
    return `${pad}<${tag}${attrs}>${escapeXmlText(val)}</${tag}>`;
  }
  const inner: string[] = [];
  if (val.length > 0) {
    inner.push(`${pad}  ${escapeXmlText(val)}`);
  }
  for (const child of children) {
    inner.push(serializeBxmNodeToXml(child, indent + 1));
  }
  return `${pad}<${tag}${attrs}>\n${inner.join("\n")}\n${pad}</${tag}>`;
}

function bxmTreeToXmlDocument(root: BxmXmlNode | null): string {
  if (!root) return '<?xml version="1.0" encoding="utf-8"?>\n<Root/>\n';
  return `<?xml version="1.0" encoding="utf-8"?>\n${serializeBxmNodeToXml(root, 0)}\n`;
}

function isUnitRangeScalarName(name: string): boolean {
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

const ALL_TEXTURE_ENCODE_FORMATS: { group: string; options: { value: string; label: string }[] }[] = [
  {
    group: "Automatic",
    options: [{ value: "Auto", label: "Auto (Match Existing Texture Format)" }],
  },
  {
    group: "BCn Block Compression (DX9 / DX10)",
    options: [
      { value: "BC1_UNORM", label: "BC1_UNORM (DXT1 — 4 bpp)" },
      { value: "BC1_SRGB", label: "BC1_SRGB (DXT1 sRGB)" },
      { value: "BC2_UNORM", label: "BC2_UNORM (DXT3 — Explicit Alpha)" },
      { value: "BC2_SRGB", label: "BC2_SRGB (DXT3 sRGB)" },
      { value: "BC3_UNORM", label: "BC3_UNORM (DXT5 — Interpolated Alpha)" },
      { value: "BC3_SRGB", label: "BC3_SRGB (DXT5 sRGB)" },
      { value: "BC4_UNORM", label: "BC4_UNORM (ATI1 — Single Channel)" },
      { value: "BC5_UNORM", label: "BC5_UNORM (ATI2 — Dual Channel Normal)" },
      { value: "BC6H_UF16", label: "BC6H_UF16 (DX10 HDR Unsigned Float)" },
      { value: "BC6H_SF16", label: "BC6H_SF16 (DX10 HDR Signed Float)" },
      { value: "BC7_UNORM", label: "BC7_UNORM (DX10 High-Quality RGBA)" },
      { value: "BC7_SRGB", label: "BC7_SRGB (DX10 High-Quality sRGB)" },
    ],
  },
  {
    group: "ASTC LDR (Switch XT1 / BNTX / DX10 — All 14 Footprints)",
    options: [
      "4x4", "5x4", "5x5", "6x5", "6x6", "8x5", "8x6",
      "8x8", "10x5", "10x6", "10x8", "10x10", "12x10", "12x12",
    ].flatMap((fp) => [
      { value: `ASTC_${fp}_UNORM`, label: `ASTC_${fp}_UNORM (Linear)` },
      { value: `ASTC_${fp}_SRGB`, label: `ASTC_${fp}_SRGB (sRGB)` },
    ]),
  },
  {
    group: "Uncompressed / Float Formats",
    options: [
      { value: "RGBA8_UNORM", label: "RGBA8_UNORM (32-bit)" },
      { value: "RGBA8_SRGB", label: "RGBA8_SRGB (32-bit sRGB)" },
      { value: "BGRA8_UNORM", label: "BGRA8_UNORM (32-bit)" },
      { value: "BGRA8_SRGB", label: "BGRA8_SRGB (32-bit sRGB)" },
      { value: "RG8_UNORM", label: "R8G8_UNORM (16-bit)" },
      { value: "R8_UNORM", label: "R8_UNORM / AF_L8_UNORM (8-bit)" },
      { value: "RGBA16_FLOAT", label: "RGBA16_FLOAT (64-bit Half Float)" },
    ],
  },
];

function TextureSlotPickerPopover({
  value,
  isB1SlotMode,
  bankTextures,
  corePretextures,
  onChange,
}: {
  value: number;
  isB1SlotMode: boolean;
  bankTextures: MaterialBankTextureOption[];
  corePretextures: CorePretextureInfo[];
  onChange: (nextVal: number) => void;
}) {
  const [open, setOpen] = useState(false);
  const [search, setSearch] = useState("");
  const [coords, setCoords] = useState<{ left: number; top: number }>({ left: 0, top: 0 });
  const btnRef = useRef<HTMLButtonElement | null>(null);

  const unsignedVal = value >>> 0;
  const bankMatch = isB1SlotMode
    ? bankTextures.find((b) => b.index === unsignedVal) ||
      bankTextures.find((b) => (b.texture_id >>> 0) === unsignedVal)
    : bankTextures.find((b) => (b.texture_id >>> 0) === unsignedVal) ||
      bankTextures.find((b) => b.index === unsignedVal);
  const pretexMatch = corePretextures.find((p) => ((p.id ?? p.tex_id ?? 0) >>> 0) === unsignedVal);

  const q = search.trim().toLowerCase();
  const filteredBank = bankTextures.filter(
    (bt) =>
      !q ||
      bt.label.toLowerCase().includes(q) ||
      bt.texture_id_hex.toLowerCase().includes(q) ||
      bt.format_name.toLowerCase().includes(q)
  );
  const filteredPre = corePretextures.filter(
    (pt) =>
      !q ||
      pt.name.toLowerCase().includes(q) ||
      (pt.id_hex ?? pt.tex_id_hex ?? "").toLowerCase().includes(q) ||
      pt.description.toLowerCase().includes(q)
  );

  const displayLabel = bankMatch
    ? `tex${bankMatch.index} (${bankMatch.texture_id_hex}) · ${bankMatch.width}×${bankMatch.height}`
    : pretexMatch
    ? `${pretexMatch.name} (${pretexMatch.id_hex ?? pretexMatch.tex_id_hex})`
    : isB1SlotMode && unsignedVal < 256
    ? `Slot #${unsignedVal} (0x${unsignedVal.toString(16).toUpperCase()})`
    : `0x${unsignedVal.toString(16).toUpperCase().padStart(8, "0")}`;

  return (
    <div className="custom-picker-wrap" style={{ width: "100%" }}>
      <button
        ref={btnRef}
        type="button"
        className="custom-picker-trigger"
        style={{ width: "100%", justifyContent: "space-between" }}
        onClick={(e) => {
          e.stopPropagation();
          if (!open && btnRef.current) {
            const r = btnRef.current.getBoundingClientRect();
            setCoords({
              left: Math.min(r.left, window.innerWidth - 395),
              top: r.bottom + 4 + 340 > window.innerHeight ? Math.max(12, r.top - 344) : r.bottom + 4,
            });
          }
          setOpen(!open);
        }}
      >
        <span style={{ display: "inline-flex", alignItems: "center", gap: 8, minWidth: 0, overflow: "hidden" }}>
          <span className="custom-picker-thumb">
            {bankMatch?.thumbnail_data_url ? (
              <img src={bankMatch.thumbnail_data_url} alt="" />
            ) : (
              <span className="mono" style={{ fontSize: 9, color: "var(--brush-muted)" }}>
                {pretexMatch ? "PRE" : "TEX"}
              </span>
            )}
          </span>
          <span className="mono" style={{ overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
            {displayLabel}
          </span>
        </span>
        <span style={{ color: "var(--brush-muted)", fontSize: 10 }}>▾</span>
      </button>

      {open && (
        <>
          <div
            className="context-menu-backdrop"
            onClick={(e) => {
              e.stopPropagation();
              setOpen(false);
            }}
          />
          <div
            className="custom-picker-popover"
            style={{ left: coords.left, top: coords.top }}
            onClick={(e) => e.stopPropagation()}
          >
            <div className="custom-picker-search">
              <input
                type="text"
                className="field-box mono"
                style={{ width: "100%", height: 26, fontSize: 11.5 }}
                placeholder="Filter bank textures or engine pretextures..."
                value={search}
                autoFocus
                onChange={(e) => setSearch(e.target.value)}
              />
            </div>
            <div className="custom-picker-list">
              {filteredBank.length > 0 && (
                <>
                  <div className="custom-picker-group-label">
                    Model Texture Bank ({filteredBank.length})
                  </div>
                  {filteredBank.map((bt) => {
                    const optVal = isB1SlotMode ? bt.index >>> 0 : bt.texture_id >>> 0;
                    const isSel = optVal === unsignedVal || (bt.texture_id >>> 0) === unsignedVal;
                    return (
                      <button
                        key={`bank:${bt.index}`}
                        type="button"
                        className={`custom-picker-item ${isSel ? "selected" : ""}`}
                        onClick={() => {
                          onChange(optVal);
                          setOpen(false);
                        }}
                      >
                        <span className="custom-picker-thumb" style={{ width: 32, height: 32 }}>
                          {bt.thumbnail_data_url ? (
                            <img src={bt.thumbnail_data_url} alt="" />
                          ) : (
                            <span className="mono" style={{ fontSize: 9 }}>TEX</span>
                          )}
                        </span>
                        <div style={{ flex: 1, minWidth: 0 }}>
                          <div className="mono" style={{ fontWeight: 600, fontSize: 11.5 }}>
                            tex{bt.index} ({bt.texture_id_hex})
                          </div>
                          <div style={{ color: "var(--brush-muted)", fontSize: 11 }}>
                            {bt.format_name} · {bt.width}×{bt.height}
                          </div>
                        </div>
                      </button>
                    );
                  })}
                </>
              )}
              {filteredPre.length > 0 && (
                <>
                  <div className="custom-picker-group-label">
                    Core Engine Pretextures ({filteredPre.length})
                  </div>
                  {filteredPre.map((pt) => {
                    const optVal = (pt.id ?? pt.tex_id ?? 0) >>> 0;
                    const hexStr = pt.id_hex ?? pt.tex_id_hex ?? `0x${optVal.toString(16).toUpperCase()}`;
                    const isSel = optVal === unsignedVal;
                    return (
                      <button
                        key={`pre:${hexStr}`}
                        type="button"
                        className={`custom-picker-item ${isSel ? "selected" : ""}`}
                        onClick={() => {
                          onChange(optVal);
                          setOpen(false);
                        }}
                      >
                        <span className="custom-picker-thumb" style={{ width: 32, height: 32 }}>
                          <span className="mono" style={{ fontSize: 9, color: "var(--brush-accent)" }}>
                            PRE
                          </span>
                        </span>
                        <div style={{ flex: 1, minWidth: 0 }}>
                          <div className="mono" style={{ fontWeight: 600, fontSize: 11.5 }}>
                            {pt.name} ({hexStr})
                          </div>
                          <div style={{ color: "var(--brush-muted)", fontSize: 11 }}>
                            {pt.description}
                          </div>
                        </div>
                      </button>
                    );
                  })}
                </>
              )}
            </div>
          </div>
        </>
      )}
    </div>
  );
}

function MaterialPickerPopover({
  value,
  options,
  onChange,
  onNavigateMaterial,
}: {
  value: number;
  options: MaterialPickerOption[];
  onChange: (nextMatIdx: number) => void;
  onNavigateMaterial: (nodeId: string) => void;
}) {
  const [open, setOpen] = useState(false);
  const [search, setSearch] = useState("");
  const [coords, setCoords] = useState<{ left: number; top: number }>({ left: 0, top: 0 });
  const btnRef = useRef<HTMLButtonElement | null>(null);

  const currentOpt = options.find((o) => o.index === value);
  const q = search.trim().toLowerCase();
  const filtered = options.filter(
    (o) =>
      !q ||
      o.label.toLowerCase().includes(q) ||
      o.shader_name.toLowerCase().includes(q) ||
      o.badge.toLowerCase().includes(q)
  );

  return (
    <div className="custom-picker-wrap">
      <button
        ref={btnRef}
        type="button"
        className="custom-picker-trigger"
        style={{ minWidth: 250 }}
        onClick={(e) => {
          e.stopPropagation();
          if (!open && btnRef.current) {
            const r = btnRef.current.getBoundingClientRect();
            setCoords({
              left: Math.min(r.left, window.innerWidth - 395),
              top: r.bottom + 4 + 340 > window.innerHeight ? Math.max(12, r.top - 344) : r.bottom + 4,
            });
          }
          setOpen(!open);
        }}
      >
        <span style={{ display: "inline-flex", alignItems: "center", gap: 8, minWidth: 0, overflow: "hidden" }}>
          <span className="custom-picker-thumb">
            {currentOpt?.thumbnail_data_url ? (
              <img src={currentOpt.thumbnail_data_url} alt="" />
            ) : (
              <span className="mono" style={{ fontSize: 9, color: "var(--brush-muted)" }}>
                MAT
              </span>
            )}
          </span>
          <span className="tree-badge">{currentOpt?.badge ?? `#${value}`}</span>
          <span style={{ overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap", fontWeight: 500 }}>
            {currentOpt ? currentOpt.label : `Material #${value}`}
          </span>
        </span>
        <span style={{ color: "var(--brush-muted)", fontSize: 10 }}>▾</span>
      </button>

      {currentOpt && (
        <button
          type="button"
          className="pill-button"
          style={{ height: 30, padding: "0 8px", fontSize: 11.5 }}
          title={`Go to ${currentOpt.label} in Material Editor`}
          onClick={(e) => {
            e.stopPropagation();
            onNavigateMaterial(currentOpt.node_id);
          }}
        >
          ↗ Edit Mat
        </button>
      )}

      {open && (
        <>
          <div
            className="context-menu-backdrop"
            onClick={(e) => {
              e.stopPropagation();
              setOpen(false);
            }}
          />
          <div
            className="custom-picker-popover"
            style={{ left: coords.left, top: coords.top }}
            onClick={(e) => e.stopPropagation()}
          >
            <div className="custom-picker-search">
              <input
                type="text"
                className="field-box mono"
                style={{ width: "100%", height: 26, fontSize: 11.5 }}
                placeholder="Filter materials by # or shader name..."
                value={search}
                autoFocus
                onChange={(e) => setSearch(e.target.value)}
              />
            </div>
            <div className="custom-picker-list">
              {filtered.map((opt) => (
                <button
                  key={opt.index}
                  type="button"
                  className={`custom-picker-item ${opt.index === value ? "selected" : ""}`}
                  onClick={() => {
                    onChange(opt.index);
                    setOpen(false);
                  }}
                >
                  <span className="custom-picker-thumb" style={{ width: 32, height: 32 }}>
                    {opt.thumbnail_data_url ? (
                      <img src={opt.thumbnail_data_url} alt="" />
                    ) : (
                      <span className="mono" style={{ fontSize: 9 }}>MAT</span>
                    )}
                  </span>
                  <div style={{ flex: 1, minWidth: 0 }}>
                    <div style={{ display: "flex", alignItems: "center", gap: 6 }}>
                      <span className="tree-badge">{opt.badge}</span>
                      <span style={{ fontWeight: 600, fontSize: 12, overflow: "hidden", textOverflow: "ellipsis" }}>
                        {opt.label}
                      </span>
                    </div>
                    <div className="mono" style={{ color: "var(--brush-muted)", fontSize: 10.5, marginTop: 1 }}>
                      {opt.first_tex_id_hex ? `Albedo/Tex0: ${opt.first_tex_id_hex}` : "No texture sampler"}
                    </div>
                  </div>
                </button>
              ))}
            </div>
          </div>
        </>
      )}
    </div>
  );
}

function isColorLikeParamName(name: string, flagged?: boolean): boolean {
  if (flagged) return true;
  const low = (name || "").toLowerCase();
  return (
    low.includes("color") ||
    low.includes("diffuse") ||
    low.includes("ambient") ||
    low.includes("specular") ||
    low.includes("albedo") ||
    low.includes("tint") ||
    low.includes("emission") ||
    low.includes("fog")
  );
}

function EditableNumberCell({
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
      title={title}
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

const IconUndo = () => (
  <svg width="13" height="13" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.5">
    <path d="M4 6H10.5C12.433 6 14 7.567 14 9.5C14 11.433 12.433 13 10.5 13H6" strokeLinecap="round" />
    <path d="M6.5 3.5L3.5 6L6.5 8.5" strokeLinecap="round" strokeLinejoin="round" />
  </svg>
);

const IconRedo = () => (
  <svg width="13" height="13" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.5">
    <path d="M12 6H5.5C3.567 6 2 7.567 2 9.5C2 11.433 3.567 13 5.5 13H10" strokeLinecap="round" />
    <path d="M9.5 3.5L12.5 6L9.5 8.5" strokeLinecap="round" strokeLinejoin="round" />
  </svg>
);

const IconRefresh = () => (
  <svg width="13" height="13" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.5">
    <path d="M13.5 8A5.5 5.5 0 1 1 11.8 4.05" strokeLinecap="round" />
    <path d="M12 1.8V4.4H9.4" strokeLinecap="round" strokeLinejoin="round" />
  </svg>
);

const IconSunMoon = () => (
  <svg width="13" height="13" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.4">
    <circle cx="8" cy="8" r="3.2" />
    <path d="M8 1.5V3M8 13V14.5M1.5 8H3M13 8H14.5M3.4 3.4L4.5 4.5M11.5 11.5L12.6 12.6M12.6 3.4L11.5 4.5M4.5 11.5L3.4 12.6" strokeLinecap="round" />
  </svg>
);

const IconFolder = () => (
  <svg width="13" height="13" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.4">
    <path d="M2 4.5C2 3.95 2.45 3.5 3 3.5H6.2L7.7 5H13C13.55 5 14 5.45 14 6V12C14 12.55 13.55 13 13 13H3C2.45 13 2 12.55 2 12V4.5Z" />
  </svg>
);

const IconImage = ({ size = 14 }: { size?: number }) => (
  <svg width={size} height={size} viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.4">
    <rect x="2" y="2.5" width="12" height="11" rx="2" />
    <circle cx="5.5" cy="6" r="1.1" />
    <path d="M2.5 11.5L6 8.2L9 11L11 9.2L13.5 11.5" strokeLinecap="round" strokeLinejoin="round" />
  </svg>
);

const IconCube = ({ size = 14 }: { size?: number }) => (
  <svg width={size} height={size} viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.4">
    <path d="M8 1.8L13.5 4.8V11.2L8 14.2L2.5 11.2V4.8L8 1.8Z" strokeLinejoin="round" />
    <path d="M2.5 4.8L8 7.8L13.5 4.8M8 7.8V14.2" strokeLinejoin="round" />
  </svg>
);

const IconCode = ({ size = 14 }: { size?: number }) => (
  <svg width={size} height={size} viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.5">
    <path d="M5.5 4.5L2 8L5.5 11.5M10.5 4.5L14 8L10.5 11.5" strokeLinecap="round" strokeLinejoin="round" />
  </svg>
);

const IconChevronDown = ({ collapsed }: { collapsed?: boolean }) => (
  <svg
    width="10"
    height="10"
    viewBox="0 0 10 6"
    fill="none"
    stroke="currentColor"
    strokeWidth="1.6"
    className={`card-chevron ${collapsed ? "collapsed" : ""}`}
  >
    <path d="M1 1L5 5L9 1" strokeLinecap="round" strokeLinejoin="round" />
  </svg>
);

const IconCheck = () => (
  <svg width="10" height="10" viewBox="0 0 12 10" fill="none" stroke="#FFFFFF" strokeWidth="2">
    <path d="M1.5 5.2L4.5 8.2L10.5 1.8" strokeLinecap="round" strokeLinejoin="round" />
  </svg>
);

export default function App() {
  const [settingsLoaded, setSettingsLoaded] = useState<boolean>(false);
  const [theme, setTheme] = useState<ThemeMode>("dark");

  // Resizable pane dimensions (persisted in StudioSettings)
  const [leftWidth, setLeftWidth] = useState<number>(310);
  const [rightWidth, setRightWidth] = useState<number>(380);
  const [assetRatio, setAssetRatio] = useState<number>(0.5);
  const [draggingSplit, setDraggingSplit] = useState<"left" | "right" | "left-row" | null>(null);
  const leftColRef = useRef<HTMLDivElement | null>(null);
  const assetBrowserCardRef = useRef<HTMLDivElement | null>(null);

  // External DLL paths (persisted in StudioSettings)
  const [oodleDllPath, setOodleDllPath] = useState<string>("");
  const [nvttDllPath, setNvttDllPath] = useState<string>("");
  // [wmb-studio BEGIN phase1]
  const [coreDatB1, setCoreDatB1] = useState<string>("");
  const [coreDatB2, setCoreDatB2] = useState<string>("");
  const [coreDatB3, setCoreDatB3] = useState<string>("");
  // [wmb-studio END]
  // [wmb-studio BEGIN phase2]
  const [wmbHierarchyRatio, setWmbHierarchyRatio] = useState<number>(0.48);
  const [wmbWireframe, setWmbWireframe] = useState<boolean>(false);
  const [wmbNormalsView, setWmbNormalsView] = useState<boolean>(false);
  const [wmbSkeletonOverlay, setWmbSkeletonOverlay] = useState<boolean>(false);
  const [wmbGridVisible, setWmbGridVisible] = useState<boolean>(true);
  const [wmbDocGeneration, setWmbDocGeneration] = useState<number>(0);
  const [coreDatModalOpen, setCoreDatModalOpen] = useState<boolean>(false);
  // [wmb-studio END]

  // Bottom-Left File Browser state
  const [currentDir, setCurrentDir] = useState<string>("");
  const currentDirRef = useRef<string>("");
  currentDirRef.current = currentDir;
  const inspectSeqRef = useRef<number>(0);
  const [dirInput, setDirInput] = useState<string>("");
  const [fsEntries, setFsEntries] = useState<FsEntry[]>([]);
  const [fileGlobFilter, setFileGlobFilter] = useState<string>("");
  const [hideUnsupported, setHideUnsupported] = useState<boolean>(true);
  const [fileSortCol, setFileSortCol] = useState<FileSortCol>("name");
  const [fileSortAsc, setFileSortAsc] = useState<boolean>(true);
  const [selectedFilePath, setSelectedFilePath] = useState<string | null>(null);
  const [selectedFilePaths, setSelectedFilePaths] = useState<string[]>([]);
  const [fsClipboard, setFsClipboard] = useState<{ mode: "copy" | "cut"; paths: string[] } | null>(
    null
  );
  const [recentFiles, setRecentFiles] = useState<string[]>([]);

  // Texture Replacement & Compression Modal Dialog state
  const [texModalState, setTexModalState] = useState<{
    nodeId: string;
    action: "replace_file" | "add_texture";
    path: string;
    fileName: string;
    encodeFormat: string;
    generateMips: boolean;
    solidify: boolean;
    makeOpaque: boolean;
  } | null>(null);

  // PKZ Bulk Compression & WMB Mesh->Batch->LOD Tree state
  const [pkzBulkCodec, setPkzBulkCodec] = useState<string>("ZStandard");
  const [expandedMeshGroups, setExpandedMeshGroups] = useState<Record<string, boolean>>({});
  const [expandedBatchLods, setExpandedBatchLods] = useState<Record<string, boolean>>({});
  const [draggedBatchInfo, setDraggedBatchInfo] = useState<{
    isWmb3: boolean;
    batchIdx: number;
    fromMeshIdx: number;
  } | null>(null);
  const [dragOverMeshIdx, setDragOverMeshIdx] = useState<number | null>(null);

  // Top-Left Asset Browser & Document state
  const [outline, setOutline] = useState<DocumentOutline | null>(null);
  const outlineRef = useRef<DocumentOutline | null>(null);
  outlineRef.current = outline;

  function confirmDiscardIfDirty(actionLabel = "open another document"): boolean {
    if (!outlineRef.current?.dirty) return true;
    return window.confirm(
      `You have unsaved changes in "${outlineRef.current.file_name}".\n\nDiscard unsaved changes and ${actionLabel}?`
    );
  }

  useEffect(() => {
    const onBeforeUnload = (e: BeforeUnloadEvent) => {
      if (outlineRef.current?.dirty) {
        e.preventDefault();
        e.returnValue = "";
      }
    };
    window.addEventListener("beforeunload", onBeforeUnload);
    return () => window.removeEventListener("beforeunload", onBeforeUnload);
  }, []);
  const [expandedNodes, setExpandedNodes] = useState<Record<string, boolean>>({
    root: true,
    "root:dat": true,
    "root:dtt": true,
  });
  const [selectedNodeId, setSelectedNodeId] = useState<string>("root");
  const [assetSearch, setAssetSearch] = useState<string>("");

  // Pointer-based Drag-and-Drop (reordering + drag-out to Explorer) & External File Drop
  const [pointerDrag, setPointerDrag] = useState<PointerDragState | null>(null);
  const [externalDragHover, setExternalDragHover] = useState<boolean>(false);

  // Active Node View & Toolbar Controls
  const [nodePayload, setNodePayload] = useState<any>(null);
  const [mipIndex, setMipIndex] = useState<number>(0);
  const [channelMode, setChannelMode] = useState<string>("RGBA");
  const [opaquePreview, setOpaquePreview] = useState<boolean>(false);

  // Custom Texture Encoding Options (XT1 Studio parity)
  const [texEncodeFormat, setTexEncodeFormat] = useState<string>("Auto");
  const [texGenerateMips, setTexGenerateMips] = useState<boolean>(true);
  const [texSolidify, setTexSolidify] = useState<boolean>(true);
  const [texMakeOpaque, setTexMakeOpaque] = useState<boolean>(false);

  // 2D Viewport Pan & Zoom
  const [zoom, setZoom] = useState<number>(1.0);
  const [pan, setPan] = useState<{ x: number; y: number }>({ x: 0, y: 0 });
  const [isPanning, setIsPanning] = useState<boolean>(false);
  const panStartRef = useRef<{ x: number; y: number }>({ x: 0, y: 0 });
  const [centerTab, setCenterTab] = useState<string>("primary");

  // Inspector collapsible cards state
  const [openCards, setOpenCards] = useState<Record<string, boolean>>({
    hash: true,
    flags: true,
    encoding: true,
    advanced: false,
    wmb_props: true,
    wmb_params: true,
    bxm_node: true,
    cloth_header: true,
    dll_config: true,
  });

  // Editable Inspector & Center Structure Drafts
  const [hashDraft, setHashDraft] = useState<string>("");
  const [flagsDraft, setFlagsDraft] = useState<string>("");
  const [wmbMatDraft, setWmbMatDraft] = useState<DecodedWmbMaterial | null>(null);
  const [wmb3MatDraft, setWmb3MatDraft] = useState<Wmb3Material | null>(null);
  const [wmbRawMode, setWmbRawMode] = useState<boolean>(false);
  const [wmbB2Override, setWmbB2Override] = useState<boolean | null>(null);
  const [materialClipboard, setMaterialClipboard] = useState<{
    game: "b1" | "b2" | "b3";
    payload: any;
  } | null>(null);

  const [wmbHeaderFlagsDraft, setWmbHeaderFlagsDraft] = useState<string>("");
  const [wmbBboxDraft, setWmbBboxDraft] = useState<number[]>([0, 0, 0, 0, 0, 0]);

  // BXM Interactive Tree + Table + Raw XML state
  const [bxmTreeDraft, setBxmTreeDraft] = useState<BxmXmlNode | null>(null);
  const [bxmXmlDraft, setBxmXmlDraft] = useState<string>("");
  const [selectedBxmPath, setSelectedBxmPath] = useState<string>("0");
  const [bxmViewMode, setBxmViewMode] = useState<"tree" | "table" | "xml">("tree");
  const [bxmExpandedPaths, setBxmExpandedPaths] = useState<Record<string, boolean>>({ "0": true });

  const [estDraft, setEstDraft] = useState<any>(null);
  const [csvHeadersDraft, setCsvHeadersDraft] = useState<string[]>([]);
  const [csvRowsDraft, setCsvRowsDraft] = useState<string[][]>([]);
  const [structDraft, setStructDraft] = useState<any>(null);
  const [selectedMotTrackIdx, setSelectedMotTrackIdx] = useState<number>(0);

  // Menus & Status
  const [activeTopMenu, setActiveTopMenu] = useState<"file" | "tools" | "help" | null>(null);
  const [contextMenu, setContextMenu] = useState<ContextMenuState | null>(null);
  const [hoverSubmenuIdx, setHoverSubmenuIdx] = useState<number | null>(null);
  const [statusLeft, setStatusLeft] = useState<string>("Ready");

  // 1. Suppress generic browser right-click context menu globally
  useEffect(() => {
    const suppressNativeContextMenu = (e: MouseEvent) => {
      e.preventDefault();
    };
    window.addEventListener("contextmenu", suppressNativeContextMenu);
    return () => window.removeEventListener("contextmenu", suppressNativeContextMenu);
  }, []);

  // Helper to measure and auto-fit a single column or all columns of a table
  const autoFitColumn = (table: HTMLTableElement, th: HTMLTableCellElement, colIdx: number) => {
    const thInp = th.querySelector("input") as HTMLInputElement | null;
    const headerText = thInp ? thInp.value || "" : (th.textContent || "").trim();
    let maxW = Math.max(34, headerText.length * 7.2 + 16);
    const rows = table.querySelectorAll("tbody tr");
    const limit = Math.min(rows.length, 250);
    for (let i = 0; i < limit; i++) {
      const cell = rows[i].children[colIdx] as HTMLElement | undefined;
      if (!cell) continue;
      const inputs = cell.querySelectorAll("input, select");
      if (inputs.length > 1) {
        let multiW = 12;
        inputs.forEach((el) => {
          const inpEl = el as HTMLInputElement;
          if (inpEl.type === "color" || inpEl.type === "checkbox") {
            multiW += 30;
          } else if (inpEl.type === "range") {
            multiW += 80;
          } else {
            const vLen = (inpEl.value || "").length;
            multiW += Math.max(46, vLen * 7.2 + 16);
          }
        });
        if (multiW > maxW) maxW = Math.min(440, multiW);
      } else if (inputs.length === 1) {
        const inp = inputs[0] as HTMLInputElement;
        const valStr = inp.value || inp.placeholder || "";
        const w = Math.max(36, valStr.length * 7.3 + 20);
        if (w > maxW) maxW = Math.min(380, w);
      } else {
        const txt = (cell.textContent || "").trim();
        const w = Math.max(34, txt.length * 7.0 + 16);
        if (w > maxW) maxW = Math.min(380, w);
      }
    }
    const finalW = Math.round(maxW);
    th.style.width = `${finalW}px`;
    th.style.minWidth = `${finalW}px`;
    th.style.maxWidth = "";
  };

  // Auto-fit all visible struct-tables whenever a file/section/table view is first opened or switched
  useEffect(() => {
    const timer = window.requestAnimationFrame(() => {
      const tables = document.querySelectorAll("table.struct-table");
      tables.forEach((tbl) => {
        const tableEl = tbl as HTMLTableElement;
        const ths = tableEl.querySelectorAll("thead tr th");
        ths.forEach((thEl, idx) => {
          autoFitColumn(tableEl, thEl as HTMLTableCellElement, idx);
        });
      });
    });
    return () => window.cancelAnimationFrame(timer);
  }, [
    nodePayload?.node_id,
    nodePayload?.view_type,
    nodePayload?.wmb?.section,
    bxmViewMode,
    selectedBxmPath,
    centerTab,
  ]);

  // 1B. Universal Resizable Table Columns (drag right 8px edge of any <th> or double-click to auto-fit)
  useEffect(() => {
    let activeTh: HTMLTableCellElement | null = null;
    let startX = 0;
    let startWidth = 0;

    const onMouseMove = (e: MouseEvent) => {
      if (activeTh) {
        const nextW = Math.max(28, startWidth + (e.clientX - startX));
        activeTh.style.width = `${nextW}px`;
        activeTh.style.minWidth = `${nextW}px`;
        activeTh.style.maxWidth = `${nextW}px`;
        document.body.style.cursor = "col-resize";
        return;
      }
      const target = e.target as HTMLElement | null;
      const th = target?.closest("table.struct-table th, table.file-table th") as HTMLTableCellElement | null;
      if (th) {
        const r = th.getBoundingClientRect();
        if (r.right - e.clientX <= 8) {
          th.style.cursor = "col-resize";
        } else {
          th.style.cursor = "";
        }
      }
    };

    const onMouseDown = (e: MouseEvent) => {
      if (e.button !== 0) return;
      const target = e.target as HTMLElement | null;
      const th = target?.closest("table.struct-table th, table.file-table th") as HTMLTableCellElement | null;
      if (!th) return;
      const r = th.getBoundingClientRect();
      if (r.right - e.clientX <= 8) {
        e.preventDefault();
        e.stopPropagation();
        activeTh = th;
        startX = e.clientX;
        startWidth = r.width;
      }
    };

    const onDoubleClick = (e: MouseEvent) => {
      const target = e.target as HTMLElement | null;
      const th = target?.closest("table.struct-table th, table.file-table th") as HTMLTableCellElement | null;
      if (!th) return;
      const r = th.getBoundingClientRect();
      if (r.right - e.clientX <= 10) {
        e.preventDefault();
        e.stopPropagation();
        const tr = th.parentElement;
        const table = th.closest("table") as HTMLTableElement | null;
        if (!tr || !table) return;
        const colIdx = Array.from(tr.children).indexOf(th);
        autoFitColumn(table, th, colIdx);
      }
    };

    const onMouseUp = () => {
      if (activeTh) {
        activeTh = null;
        document.body.style.cursor = "";
      }
    };

    window.addEventListener("mousemove", onMouseMove, true);
    window.addEventListener("mousedown", onMouseDown, true);
    window.addEventListener("dblclick", onDoubleClick, true);
    window.addEventListener("mouseup", onMouseUp, true);
    return () => {
      window.removeEventListener("mousemove", onMouseMove, true);
      window.removeEventListener("mousedown", onMouseDown, true);
      window.removeEventListener("dblclick", onDoubleClick, true);
      window.removeEventListener("mouseup", onMouseUp, true);
    };
  }, []);

  // 2. Listen for external OS file drops (`tauri://drag-drop`) anywhere on the window
  const openDocRef = useRef<(p: string) => Promise<void>>(async () => {});
  useEffect(() => {
    let unlistenDrop: (() => void) | undefined;
    let unlistenEnter: (() => void) | undefined;
    let unlistenLeave: (() => void) | undefined;

    (async () => {
      try {
        unlistenDrop = await listen<{ paths?: string[] }>("tauri://drag-drop", (event) => {
          setExternalDragHover(false);
          const paths = event.payload?.paths;
          if (Array.isArray(paths) && paths.length > 0 && paths[0]) {
            openDocRef.current(paths[0]);
          }
        });
        unlistenEnter = await listen("tauri://drag-enter", () => {
          setExternalDragHover(true);
        });
        unlistenLeave = await listen("tauri://drag-leave", () => {
          setExternalDragHover(false);
        });
      } catch {
        // Fallback handled via DOM onDrop
      }
    })();

    return () => {
      if (unlistenDrop) unlistenDrop();
      if (unlistenEnter) unlistenEnter();
      if (unlistenLeave) unlistenLeave();
    };
  }, []);

  // Load persisted settings on mount
  useEffect(() => {
    (async () => {
      try {
        const s = await invoke<StudioSettings>("get_studio_settings");
        const loadedTheme: ThemeMode = s.theme?.toLowerCase() === "light" ? "light" : "dark";
        setTheme(loadedTheme);
        document.documentElement.setAttribute("data-theme", loadedTheme);
        setRecentFiles(s.recent_files ?? []);
        setHideUnsupported(s.hide_unsupported ?? true);
        if (["name", "type", "modified", "size"].includes(s.file_sort_col)) {
          setFileSortCol(s.file_sort_col as FileSortCol);
        }
        setFileSortAsc(s.file_sort_asc ?? true);
        if (s.left_width >= 200 && s.left_width <= 650) setLeftWidth(s.left_width);
        if (s.right_width >= 260 && s.right_width <= 700) setRightWidth(s.right_width);
        if (s.asset_ratio >= 0.18 && s.asset_ratio <= 0.82) setAssetRatio(s.asset_ratio);
        setOodleDllPath(s.oodle_dll_path ?? "");
        setNvttDllPath(s.nvtt_dll_path ?? "");
        // [wmb-studio BEGIN phase1]
        setCoreDatB1(s.core_dat_b1 ?? "");
        setCoreDatB2(s.core_dat_b2 ?? "");
        setCoreDatB3(s.core_dat_b3 ?? "");
        // [wmb-studio END]
        // [wmb-studio BEGIN phase2]
        if (typeof s.wmb_hierarchy_ratio === "number" && s.wmb_hierarchy_ratio >= 0.15 && s.wmb_hierarchy_ratio <= 0.85) {
          setWmbHierarchyRatio(s.wmb_hierarchy_ratio);
        }
        if (typeof s.wmb_wireframe === "boolean") setWmbWireframe(s.wmb_wireframe);
        if (typeof s.wmb_normals_view === "boolean") setWmbNormalsView(s.wmb_normals_view);
        if (typeof s.wmb_skeleton_overlay === "boolean") setWmbSkeletonOverlay(s.wmb_skeleton_overlay);
        if (typeof s.wmb_grid_visible === "boolean") setWmbGridVisible(s.wmb_grid_visible);
        // [wmb-studio END]
        setSettingsLoaded(true);
        await loadDirectory(s.last_dir || undefined);
      } catch {
        setSettingsLoaded(true);
        await loadDirectory();
      }
    })();
  }, []);

  // Sync theme attribute on document root
  useEffect(() => {
    document.documentElement.setAttribute("data-theme", theme);
  }, [theme]);

  const buildStudioSettingsPayload = useCallback(
    (overrides?: Partial<StudioSettings>): StudioSettings => ({
      theme: theme === "light" ? "Light" : "Dark",
      recent_files: recentFiles,
      last_dir: currentDir,
      hide_unsupported: hideUnsupported,
      file_sort_col: fileSortCol,
      file_sort_asc: fileSortAsc,
      left_width: Math.round(leftWidth),
      right_width: Math.round(rightWidth),
      asset_ratio: assetRatio,
      oodle_dll_path: oodleDllPath,
      nvtt_dll_path: nvttDllPath,
      // [wmb-studio BEGIN phase1]
      core_dat_b1: coreDatB1,
      core_dat_b2: coreDatB2,
      core_dat_b3: coreDatB3,
      // [wmb-studio END]
      // [wmb-studio BEGIN phase2]
      wmb_hierarchy_ratio: wmbHierarchyRatio,
      wmb_wireframe: wmbWireframe,
      wmb_normals_view: wmbNormalsView,
      wmb_skeleton_overlay: wmbSkeletonOverlay,
      wmb_grid_visible: wmbGridVisible,
      // [wmb-studio END]
      ...overrides,
    }),
    [
      theme,
      recentFiles,
      currentDir,
      hideUnsupported,
      fileSortCol,
      fileSortAsc,
      leftWidth,
      rightWidth,
      assetRatio,
      oodleDllPath,
      nvttDllPath,
      coreDatB1,
      coreDatB2,
      coreDatB3,
      wmbHierarchyRatio,
      wmbWireframe,
      wmbNormalsView,
      wmbSkeletonOverlay,
      wmbGridVisible,
    ]
  );

  // Persist StudioSettings whenever preferences change (debounced to avoid IPC spam during splitter drag — B-15)
  useEffect(() => {
    if (!settingsLoaded) return;
    const payload = buildStudioSettingsPayload();
    const timer = window.setTimeout(() => {
      invoke("save_studio_settings", { settings: payload }).catch(() => {});
    }, 250);
    return () => window.clearTimeout(timer);
  }, [
    buildStudioSettingsPayload,
    settingsLoaded,
    theme,
    recentFiles,
    currentDir,
    hideUnsupported,
    fileSortCol,
    fileSortAsc,
    leftWidth,
    rightWidth,
    assetRatio,
    oodleDllPath,
    nvttDllPath,
    // [wmb-studio BEGIN phase1]
    coreDatB1,
    coreDatB2,
    coreDatB3,
    // [wmb-studio END]
    // [wmb-studio BEGIN phase2]
    wmbHierarchyRatio,
    wmbWireframe,
    wmbNormalsView,
    wmbSkeletonOverlay,
    wmbGridVisible,
    // [wmb-studio END]
  ]);

  const shortcutHandlersRef = useRef<{
    openFile: () => void;
    save: () => void;
    saveAs: () => void;
    undo: () => void;
    redo: () => void;
  }>({
    openFile: () => {},
    save: () => {},
    saveAs: () => {},
    undo: () => {},
    redo: () => {},
  });

  // Global keyboard shortcuts (Ctrl+O, Ctrl+S, Ctrl+Z, Ctrl+Y) with editable input guard (B-12)
  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      const target = e.target as HTMLElement | null;
      const inEditableField = Boolean(
        target &&
          (target.tagName === "INPUT" ||
            target.tagName === "TEXTAREA" ||
            target.tagName === "SELECT" ||
            target.isContentEditable)
      );
      if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "o") {
        e.preventDefault();
        shortcutHandlersRef.current.openFile();
      } else if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "s") {
        e.preventDefault();
        if (e.shiftKey) {
          shortcutHandlersRef.current.saveAs();
        } else {
          shortcutHandlersRef.current.save();
        }
      } else if (!inEditableField && (e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "z") {
        if (e.shiftKey) {
          e.preventDefault();
          shortcutHandlersRef.current.redo();
        } else {
          e.preventDefault();
          shortcutHandlersRef.current.undo();
        }
      } else if (!inEditableField && (e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "y") {
        e.preventDefault();
        shortcutHandlersRef.current.redo();
      } else if (e.key === "Escape") {
        setContextMenu(null);
        setActiveTopMenu(null);
        setPointerDrag(null);
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, []);

  // Splitter drag handler
  useEffect(() => {
    if (!draggingSplit) return;
    const onMove = (e: MouseEvent) => {
      if (draggingSplit === "left") {
        setLeftWidth(Math.max(220, Math.min(620, e.clientX - 10)));
      } else if (draggingSplit === "right") {
        setRightWidth(Math.max(290, Math.min(680, window.innerWidth - e.clientX - 10)));
      } else if (draggingSplit === "left-row" && leftColRef.current) {
        const rect = leftColRef.current.getBoundingClientRect();
        const rel = (e.clientY - rect.top) / Math.max(1, rect.height);
        setAssetRatio(Math.max(0.18, Math.min(0.82, rel)));
      }
    };
    const onUp = () => setDraggingSplit(null);
    window.addEventListener("mousemove", onMove);
    window.addEventListener("mouseup", onUp);
    return () => {
      window.removeEventListener("mousemove", onMove);
      window.removeEventListener("mouseup", onUp);
    };
  }, [draggingSplit]);

  // Parse trailing index from a sibling container node ID (e.g. `root:dat/dat:3` -> `["root:dat/dat", 3]`)
  function parseReorderableNode(nodeId: string): [string, number] | null {
    const m = nodeId.match(/^(.*?(?:dat|pkz|efffile)):(\d+)$/);
    if (!m) return null;
    return [m[1], parseInt(m[2], 10)];
  }

  function isVirtualSubNode(nodeId: string): boolean {
    const leaf = nodeId.split("/").pop() || "";
    return leaf.startsWith("wmb:") || leaf.startsWith("wmb3:") || leaf.startsWith("tex:");
  }

  // Pointer-based drag-and-drop for Asset Browser reordering, Drag-Out to Explorer, and BXM XML reordering
  useEffect(() => {
    if (!pointerDrag) return;
    const onPointerMove = (e: PointerEvent) => {
      const dx = e.clientX - pointerDrag.startX;
      const dy = e.clientY - pointerDrag.startY;
      const dist = Math.hypot(dx, dy);
      const isNowActive = pointerDrag.active || dist >= 6;
      if (!isNowActive) return;

      // Check if dragging an Asset Browser file node outside the Asset Browser card to Windows Explorer / Desktop
      if (
        pointerDrag.kind === "asset" &&
        !pointerDrag.dragOutTriggered &&
        assetBrowserCardRef.current &&
        !isVirtualSubNode(pointerDrag.sourceId) &&
        pointerDrag.sourceId !== "root"
      ) {
        const rect = assetBrowserCardRef.current.getBoundingClientRect();
        const outsideCard =
          e.clientX < rect.left - 28 ||
          e.clientX > rect.right + 28 ||
          e.clientY < rect.top - 28 ||
          e.clientY > rect.bottom + 28;
        if (outsideCard) {
          setPointerDrag(null);
          setStatusLeft(`Dragging ${pointerDrag.label} out to Windows Explorer...`);
          invoke<string>("drag_out_asset_node", { nodeId: pointerDrag.sourceId })
            .then((msg) => setStatusLeft(msg))
            .catch((err) => setStatusLeft(`Drag-out note: ${String(err)}`));
          return;
        }
      }

      // Hit-test row under cursor for sibling reordering
      const el = document.elementFromPoint(e.clientX, e.clientY) as HTMLElement | null;
      const rowEl = el?.closest(
        pointerDrag.kind === "asset" ? "[data-asset-node-id]" : "[data-bxm-path]"
      ) as HTMLElement | null;

      let overId: string | null = null;
      let insertPos: "before" | "after" | null = null;

      if (rowEl) {
        const candidateId =
          pointerDrag.kind === "asset"
            ? rowEl.getAttribute("data-asset-node-id")
            : rowEl.getAttribute("data-bxm-path");

        if (candidateId && candidateId !== pointerDrag.sourceId) {
          if (pointerDrag.kind === "asset") {
            const srcInfo = parseReorderableNode(pointerDrag.sourceId);
            const dstInfo = parseReorderableNode(candidateId);
            if (srcInfo && dstInfo && srcInfo[0] === dstInfo[0]) {
              overId = candidateId;
              const r = rowEl.getBoundingClientRect();
              insertPos = e.clientY < r.top + r.height / 2 ? "before" : "after";
            } else if (
              pointerDrag.sourceId.includes("pkz:") &&
              candidateId.includes("pkzfolder:")
            ) {
              overId = candidateId;
              insertPos = "after";
            }
          } else if (pointerDrag.kind === "bxm") {
            const srcParent = pointerDrag.sourceId.slice(0, pointerDrag.sourceId.lastIndexOf("."));
            const dstParent = candidateId.slice(0, candidateId.lastIndexOf("."));
            if (srcParent && srcParent === dstParent) {
              overId = candidateId;
              const r = rowEl.getBoundingClientRect();
              insertPos = e.clientY < r.top + r.height / 2 ? "before" : "after";
            }
          }
        }
      }

      setPointerDrag((prev) =>
        prev
          ? {
              ...prev,
              active: true,
              curX: e.clientX,
              curY: e.clientY,
              overId,
              insertPos,
            }
          : null
      );
    };

    const onPointerUp = () => {
      if (pointerDrag.active && pointerDrag.overId && pointerDrag.insertPos) {
        if (pointerDrag.kind === "asset") {
          if (
            pointerDrag.sourceId.includes("pkz:") &&
            pointerDrag.overId.includes("pkzfolder:")
          ) {
            const folderPrefix = pointerDrag.overId.split("pkzfolder:").pop() || "";
            runContainerMutation(pointerDrag.sourceId, "move_pkz_entry_to_folder", folderPrefix, {
              target_folder: folderPrefix,
            });
            setPointerDrag(null);
            return;
          }
          const srcInfo = parseReorderableNode(pointerDrag.sourceId);
          const dstInfo = parseReorderableNode(pointerDrag.overId);
          if (srcInfo && dstInfo && srcInfo[0] === dstInfo[0]) {
            const fromIdx = srcInfo[1];
            let toIdx = dstInfo[1];
            if (pointerDrag.insertPos === "after" && fromIdx < toIdx) {
              // removing fromIdx shifts subsequent items left by 1
            } else if (pointerDrag.insertPos === "before" && fromIdx > toIdx) {
              // target index stays toIdx
            } else if (pointerDrag.insertPos === "after" && fromIdx > toIdx) {
              toIdx += 1;
            } else if (pointerDrag.insertPos === "before" && fromIdx < toIdx) {
              toIdx = Math.max(0, toIdx - 1);
            }
            if (fromIdx !== toIdx) {
              runContainerMutation(pointerDrag.sourceId, "reorder_entry", "", {
                to_index: toIdx,
              });
            }
          }
        } else if (pointerDrag.kind === "bxm" && bxmTreeDraft) {
          const parentPath = pointerDrag.sourceId.slice(0, pointerDrag.sourceId.lastIndexOf("."));
          const fromIdx = parseInt(pointerDrag.sourceId.split(".").pop() || "0", 10);
          let toIdx = parseInt(pointerDrag.overId.split(".").pop() || "0", 10);
          if (pointerDrag.insertPos === "after" && fromIdx > toIdx) toIdx += 1;
          if (pointerDrag.insertPos === "before" && fromIdx < toIdx) toIdx = Math.max(0, toIdx - 1);
          if (fromIdx !== toIdx) {
            const nextTree = structuredClone(bxmTreeDraft);
            const parentNode = getBxmNodeByPath(nextTree, parentPath);
            if (parentNode && Array.isArray(parentNode.children)) {
              const [moved] = parentNode.children.splice(fromIdx, 1);
              parentNode.children.splice(
                Math.min(parentNode.children.length, Math.max(0, toIdx)),
                0,
                moved
              );
              updateBxmDraftTree(nextTree);
              setSelectedBxmPath(`${parentPath}.${toIdx}`);
            }
          }
        }
      }
      setPointerDrag(null);
    };

    window.addEventListener("pointermove", onPointerMove);
    window.addEventListener("pointerup", onPointerUp);
    return () => {
      window.removeEventListener("pointermove", onPointerMove);
      window.removeEventListener("pointerup", onPointerUp);
    };
  }, [pointerDrag, bxmTreeDraft]);

  async function refreshRecents() {
    try {
      const rec = await invoke<string[]>("get_recent_files");
      setRecentFiles(rec);
    } catch {
      // ignore
    }
  }

  async function loadDirectory(target?: string) {
    try {
      const [resolved, list] = await invoke<[string, FsEntry[]]>("list_directory", {
        dirPath: target ?? null,
      });
      setCurrentDir(resolved);
      setDirInput(resolved);
      setFsEntries(list);
    } catch (err) {
      setStatusLeft(`Directory error: ${String(err)}`);
    }
  }

  async function openDocumentByPath(path: string, skipDirtyPrompt = false) {
    if (!skipDirtyPrompt && !confirmDiscardIfDirty(`open "${path.split(/[\\/]/).pop() || path}"`)) {
      return;
    }
    try {
      setStatusLeft(`Loading ${path}...`);
      const doc = await invoke<DocumentOutline>("open_document", { path });
      setOutline(doc);
      setSelectedFilePath(path);
      setWmbB2Override(null);
      refreshRecents();

      const nextExpanded: Record<string, boolean> = {
        root: true,
        "root:dat": true,
        "root:dtt": true,
      };
      let defaultTargetId = doc.tree[0]?.id ?? "root";
      // [wmb-studio BEGIN phase2]
      for (const rootNode of doc.tree) {
        nextExpanded[rootNode.id] = true;
        if (rootNode.format_badge === "WMB" || rootNode.format_badge === "WMB3") {
          defaultTargetId = rootNode.id;
          break;
        }
        const wmbChild = rootNode.children.find(
          (c) => (c.format_badge === "WMB" || c.format_badge === "WMB3") && !isVirtualSubNode(c.id)
        );
        if (wmbChild) {
          defaultTargetId = wmbChild.id;
          break;
        }
        if (rootNode.children.length > 0) {
          const firstChild = rootNode.children[0];
          nextExpanded[firstChild.id] = true;
          if (firstChild.children.length > 0) {
            defaultTargetId = firstChild.children[0].id;
            break;
          } else {
            defaultTargetId = firstChild.id;
            break;
          }
        }
      }
      setWmbDocGeneration((g) => g + 1);
      // [wmb-studio END]
      setExpandedNodes(nextExpanded);
      setMipIndex(0);
      await selectAndInspectNode(defaultTargetId, 0, channelMode, opaquePreview, null, doc.file_name);
      setStatusLeft(`Loaded ${doc.file_name}`);
    } catch (err) {
      setStatusLeft(`Failed to open: ${String(err)}`);
    }
  }
  openDocRef.current = openDocumentByPath;

  function detectBxmTabular(root: BxmXmlNode | null): boolean {
    if (!root || !Array.isArray(root.children)) return false;
    if (root.children.length >= 2) {
      const firstTag = root.children[0].name;
      const sameTagCount = root.children.filter((c) => c.name === firstTag).length;
      if (sameTagCount >= 2 && (root.children[0].attributes.length > 0 || root.children[0].children.length > 0)) {
        return true;
      }
    }
    return false;
  }

  async function selectAndInspectNode(
    nodeId: string,
    mip = mipIndex,
    channel = channelMode,
    opaque = opaquePreview,
    b2Override: boolean | null = wmbB2Override,
    docLabel?: string
  ) {
    const reqSeq = ++inspectSeqRef.current;
    setSelectedNodeId(nodeId);
    try {
      const res = await invoke<any>("inspect_asset_node", {
        nodeId,
        mipIndex: mip,
        channel,
        opaque,
        b2Override,
      });
      if (reqSeq !== inspectSeqRef.current) return;
      setNodePayload(res);
      if (res.view_type === "TEXTURE" && res.texture) {
        const t: TextureNodeView = res.texture;
        setHashDraft(t.texture_id_hex);
        setFlagsDraft(t.flags_hex);
      } else if (res.view_type === "WMB" && res.wmb) {
        const w: WmbSectionView = res.wmb;
        if (w.material_decoded) {
          const norm: DecodedWmbMaterial = {
            ...w.material_decoded,
            fields: Array.isArray(w.material_decoded.fields) ? w.material_decoded.fields : [],
            raw_dwords: Array.isArray(w.material_decoded.raw_dwords)
              ? w.material_decoded.raw_dwords
              : [],
          };
          setWmbMatDraft(norm);
        } else {
          setWmbMatDraft(null);
        }
        if (w.wmb3_material) {
          setWmb3MatDraft(structuredClone(w.wmb3_material));
        } else {
          setWmb3MatDraft(null);
        }
        if (w.section === "header" && w.data) {
          const vf = w.data.vertex_flags;
          if (typeof vf === "number") {
            setWmbHeaderFlagsDraft(`0x${(vf >>> 0).toString(16).toUpperCase().padStart(8, "0")}`);
          }
          if (Array.isArray(w.data.bounding_box) && w.data.bounding_box.length === 6) {
            setWmbBboxDraft([...w.data.bounding_box]);
          }
        }
      } else if (res.view_type === "BXM") {
        const clonedRoot: BxmXmlNode | null = res.root ? structuredClone(res.root) : null;
        setBxmTreeDraft(clonedRoot);
        setBxmXmlDraft(res.xml ?? bxmTreeToXmlDocument(clonedRoot));
        setSelectedBxmPath("0");
        setBxmExpandedPaths({ "0": true, "0.0": true, "0.1": true });
        if (detectBxmTabular(clonedRoot)) {
          setBxmViewMode("table");
        } else {
          setBxmViewMode("tree");
        }
      } else if (res.view_type === "EST" && res.est) {
        setEstDraft(structuredClone(res.est));
      } else if (res.view_type === "CSV") {
        setCsvHeadersDraft(Array.isArray(res.headers) ? [...res.headers] : []);
        setCsvRowsDraft(Array.isArray(res.rows) ? structuredClone(res.rows) : []);
      } else if (res.view_type === "RAW_STRUCT") {
        setStructDraft(structuredClone(res.data));
        setSelectedMotTrackIdx(0);
      }
      if (docLabel) {
        setStatusLeft(`Loaded ${docLabel}`);
      }
    } catch (err) {
      if (reqSeq === inspectSeqRef.current) {
        setStatusLeft(`Inspect error: ${String(err)}`);
      }
    }
  }

  function updateBxmDraftTree(nextRoot: BxmXmlNode) {
    setBxmTreeDraft(nextRoot);
    setBxmXmlDraft(bxmTreeToXmlDocument(nextRoot));
  }

  function switchBxmViewMode(nextMode: "tree" | "table" | "xml") {
    if (bxmViewMode === "xml" && nextMode !== "xml") {
      const parsed = parseXmlToBxmNode(bxmXmlDraft);
      if (!parsed) {
        setStatusLeft("XML parse error: fix malformed XML syntax before switching to Tree/Table view");
        return;
      }
      setBxmTreeDraft(parsed);
    } else if (bxmViewMode !== "xml" && nextMode === "xml" && bxmTreeDraft) {
      setBxmXmlDraft(bxmTreeToXmlDocument(bxmTreeDraft));
    }
    setBxmViewMode(nextMode);
  }

  async function commitBxmDraft(customXml?: string) {
    if (!nodePayload || nodePayload.view_type !== "BXM") return;
    try {
      const xmlToSave =
        customXml ??
        (bxmViewMode === "xml" || !bxmTreeDraft
          ? bxmXmlDraft
          : bxmTreeToXmlDocument(bxmTreeDraft));
      const updated = await invoke<DocumentOutline>("mutate_bxm_node", {
        nodeId: nodePayload.node_id,
        xmlContent: xmlToSave,
        tag: nodePayload.tag || "BXM",
      });
      setOutline(updated);
      await selectAndInspectNode(nodePayload.node_id);
      setStatusLeft(`Committed ${nodePayload.title} changes to binary ${nodePayload.tag || "BXM"}`);
    } catch (err) {
      setStatusLeft(`BXM encode error: ${String(err)}`);
    }
  }

  async function handleTexturePreviewParamChange(newMip: number, newChannel: string, newOpaque: boolean) {
    setMipIndex(newMip);
    setChannelMode(newChannel);
    setOpaquePreview(newOpaque);
    if (nodePayload?.view_type === "TEXTURE") {
      await selectAndInspectNode(nodePayload.texture.node_id, newMip, newChannel, newOpaque);
    }
  }

  async function handleMenuOpenFile() {
    setActiveTopMenu(null);
    const picked = await invoke<string | null>("pick_file_dialog", {
      title: "Open PlatinumGames Archive or Asset",
      extensions: [
        "dat", "dtt", "evn", "pkz", "eff", "idd", "scr",
        "wtb", "wta", "wtp", "dds", "gtx", "xt1", "bntx",
        "wmb", "mdb", "bxm", "xml", "gad", "clp", "clh", "clw",
        "mot", "exp", "est", "cos", "att", "obi", "msq", "col",
        "fls", "fla", "cta", "lit", "csv",
      ],
    });
    if (picked) {
      const slash = Math.max(picked.lastIndexOf("\\"), picked.lastIndexOf("/"));
      if (slash > 0) {
        loadDirectory(picked.slice(0, slash));
      }
      await openDocumentByPath(picked);
    }
  }

  async function handleMenuOpenFolder() {
    setActiveTopMenu(null);
    const folder = await invoke<string | null>("pick_folder_dialog", {
      title: "Select Workspace Folder",
    });
    if (folder) {
      await loadDirectory(folder);
      setStatusLeft(`Opened folder ${folder}`);
    }
  }

  async function handleMenuSave() {
    setActiveTopMenu(null);
    if (!outline) return;
    try {
      const updated = await invoke<DocumentOutline>("save_document", { outputPath: null });
      setOutline(updated);
      setStatusLeft(`Saved ${updated.file_name}`);
      loadDirectory(currentDirRef.current);
    } catch (err) {
      setStatusLeft(`Save error: ${String(err)}`);
    }
  }

  async function handleMenuSaveAs() {
    setActiveTopMenu(null);
    if (!outline) return;
    const dest = await invoke<string | null>("save_file_dialog", {
      title: "Save Document As...",
      defaultName: outline.file_name.split(" + ")[0],
    });
    if (!dest) return;
    try {
      const updated = await invoke<DocumentOutline>("save_document", { outputPath: dest });
      setOutline(updated);
      setStatusLeft(`Saved to ${dest}`);
      loadDirectory(currentDirRef.current);
    } catch (err) {
      setStatusLeft(`Save As error: ${String(err)}`);
    }
  }

  async function handleUndo() {
    if (!outline?.can_undo) return;
    try {
      const label = outline.undo_label ?? "edit";
      const updated = await invoke<DocumentOutline>("undo_document");
      setOutline(updated);
      // [wmb-studio BEGIN phase2]
      setWmbDocGeneration((g) => g + 1);
      // [wmb-studio END]
      await selectAndInspectNode(selectedNodeId);
      setStatusLeft(`Undid: ${label}`);
    } catch (err) {
      setStatusLeft(`Undo error: ${String(err)}`);
    }
  }

  async function handleRedo() {
    if (!outline?.can_redo) return;
    try {
      const label = outline.redo_label ?? "edit";
      const updated = await invoke<DocumentOutline>("redo_document");
      setOutline(updated);
      // [wmb-studio BEGIN phase2]
      setWmbDocGeneration((g) => g + 1);
      // [wmb-studio END]
      await selectAndInspectNode(selectedNodeId);
      setStatusLeft(`Redid: ${label}`);
    } catch (err) {
      setStatusLeft(`Redo error: ${String(err)}`);
    }
  }

  shortcutHandlersRef.current = {
    openFile: () => {
      void handleMenuOpenFile();
    },
    save: () => {
      void handleMenuSave();
    },
    saveAs: () => {
      void handleMenuSaveAs();
    },
    undo: () => {
      void handleUndo();
    },
    redo: () => {
      void handleRedo();
    },
  };

  function handleResetView() {
    setZoom(1.0);
    setPan({ x: 0, y: 0 });
  }

  async function handlePickOodleDll() {
    const picked = await invoke<string | null>("pick_file_dialog", {
      title: "Locate Oodle Kraken Library (oo2core_*_win64.dll)",
      extensions: ["dll", "so", "dylib"],
    });
    if (picked) {
      setOodleDllPath(picked);
      await invoke("save_studio_settings", {
        settings: buildStudioSettingsPayload({ oodle_dll_path: picked }),
      }).catch(() => {});
      setStatusLeft(`Configured Oodle DLL: ${picked}`);
      if (outline) {
        await openDocumentByPath(outline.path);
      }
    }
  }

  async function handlePickNvttDll() {
    const picked = await invoke<string | null>("pick_file_dialog", {
      title: "Locate NVIDIA Texture Tools Library (nvtt30205.dll / nvtt.dll)",
      extensions: ["dll", "so", "dylib"],
    });
    if (picked) {
      setNvttDllPath(picked);
      await invoke("save_studio_settings", {
        settings: buildStudioSettingsPayload({ nvtt_dll_path: picked }),
      }).catch(() => {});
      setStatusLeft(`Configured NVTT DLL: ${picked}`);
    }
  }

  async function runTextureMutation(nodeId: string, action: string, payload: any = {}) {
    try {
      const fullPayload = {
        encode_format: texEncodeFormat,
        generate_mips: texGenerateMips,
        solidify: texSolidify,
        make_opaque: texMakeOpaque,
        ...payload,
      };
      const updated = await invoke<DocumentOutline>("mutate_texture_node", {
        nodeId,
        action,
        payload: fullPayload,
      });
      setOutline(updated);
      // [wmb-studio BEGIN phase2]
      setWmbDocGeneration((g) => g + 1);
      // [wmb-studio END]
      await selectAndInspectNode(nodeId);
      setStatusLeft(updated.undo_label ?? `Updated texture`);
    } catch (err) {
      setStatusLeft(`Texture error: ${String(err)}`);
    }
  }

  async function promptAndRunTextureImport(
    nodeId: string,
    action: "replace_file" | "add_texture"
  ) {
    setContextMenu(null);
    const path = await invoke<string | null>("pick_file_dialog", {
      title:
        action === "replace_file"
          ? "Select Replacement Texture (PNG / TGA / BMP / JPG or Raw DDS / GTX / XT1 / BNTX)"
          : "Select Texture to Add to Bank",
      extensions: ["png", "dds", "gtx", "xt1", "bntx", "tga", "bmp", "jpg", "jpeg"],
    });
    if (!path) return;
    const low = path.toLowerCase();
    const isRawContainer =
      low.endsWith(".dds") || low.endsWith(".gtx") || low.endsWith(".xt1") || low.endsWith(".bntx");
    if (isRawContainer) {
      await runTextureMutation(nodeId, action, { path });
      return;
    }
    const fileName = path.split(/[\\/]/).pop() || path;
    setTexModalState({
      nodeId,
      action,
      path,
      fileName,
      encodeFormat: action === "add_texture" && texEncodeFormat === "Auto" ? "BC3_UNORM" : texEncodeFormat,
      generateMips: texGenerateMips,
      solidify: texSolidify,
      makeOpaque: texMakeOpaque,
    });
  }

  async function handleTextureExport(
    nodeId: string,
    scope: "single" | "bank",
    format: "png" | "dds" | "raw"
  ) {
    setContextMenu(null);
    try {
      if (scope === "single") {
        const ext = format === "png" ? "png" : format === "dds" ? "dds" : "bin";
        const dest = await invoke<string | null>("save_file_dialog", {
          title: `Export Texture (.${ext})`,
          defaultName: `texture.${ext}`,
        });
        if (!dest) return;
        const msg = await invoke<string>("export_texture_action", {
          nodeId,
          scope,
          format,
          destPath: dest,
        });
        setStatusLeft(msg);
      } else {
        const folder = await invoke<string | null>("pick_folder_dialog", {
          title: `Export All Textures as ${format.toUpperCase()} Folder`,
        });
        if (!folder) return;
        const msg = await invoke<string>("export_texture_action", {
          nodeId,
          scope,
          format,
          destPath: folder,
        });
        setStatusLeft(msg);
        loadDirectory(currentDirRef.current);
      }
    } catch (err) {
      setStatusLeft(`Export failed: ${String(err)}`);
    }
  }

  async function runWmbMutation(nodeId: string, operation: string, payload: any = {}) {
    setContextMenu(null);
    try {
      const updated = await invoke<DocumentOutline>("mutate_wmb_node", {
        nodeId,
        operation,
        payload,
      });
      setOutline(updated);
      // [wmb-studio BEGIN phase2]
      setWmbDocGeneration((g) => g + 1);
      // [wmb-studio END]
      await selectAndInspectNode(nodeId);
      setStatusLeft(updated.undo_label ?? `Completed WMB ${operation}`);
    } catch (err) {
      setStatusLeft(`WMB error: ${String(err)}`);
    }
  }

  async function handleCrossGameMaterialConvert(
    wmb: WmbSectionView,
    dstGame: "b1" | "b2" | "b3"
  ) {
    setContextMenu(null);
    try {
      const srcGame: "b1" | "b2" | "b3" = wmb.is_wmb3
        ? "b3"
        : (wmbB2Override ?? wmb.is_bayo2)
        ? "b2"
        : "b1";
      const srcPayload = wmb.is_wmb3 ? wmb3MatDraft : wmbMatDraft;
      if (!srcPayload) return;

      const res = await invoke<{ converted: any; report: any }>("convert_material_action", {
        srcGame,
        dstGame,
        materialPayload: srcPayload,
        targetShader: null,
      });

      // Always store in clipboard so user can paste into another open model
      setMaterialClipboard({
        game: dstGame,
        payload: res.converted,
      });

      // If destination matches current model format (WMB0 for b1/b2, WMB3 for b3), apply in-place!
      if (!wmb.is_wmb3 && (dstGame === "b1" || dstGame === "b2") && res.converted?.fields) {
        const nextMat: DecodedWmbMaterial = {
          ...res.converted,
          index: wmbMatDraft?.index ?? 0,
        };
        setWmbB2Override(dstGame === "b2");
        setWmbMatDraft(nextMat);
        await runWmbMutation(wmb.node_id, "update_material", nextMat);
        setStatusLeft(
          `Converted material (${srcGame.toUpperCase()} → ${dstGame.toUpperCase()}) and applied in-place`
        );
      } else if (wmb.is_wmb3 && dstGame === "b3" && res.converted?.shader_name) {
        const matIdx = parseInt(wmb.section.replace("mat:", ""), 10) || 0;
        setWmb3MatDraft(res.converted);
        await runWmbMutation(wmb.node_id, "update_wmb3_material", {
          index: matIdx,
          material: res.converted,
        });
        setStatusLeft(`Converted material to Bayonetta 3 (${res.converted.shader_name})`);
      } else {
        setStatusLeft(
          `Converted material to ${dstGame.toUpperCase()} and copied to Material Clipboard (ready to paste)`
        );
      }
    } catch (err) {
      setStatusLeft(`Material conversion error: ${String(err)}`);
    }
  }

  async function runStructMutation(nodeId: string, structKind: string, updatedData: any) {
    try {
      const updated = await invoke<DocumentOutline>("mutate_struct_node", {
        nodeId,
        structKind,
        updatedData,
      });
      setOutline(updated);
      await selectAndInspectNode(nodeId);
      setStatusLeft(updated.undo_label ?? `Updated ${structKind}`);
    } catch (err) {
      setStatusLeft(`${structKind} update error: ${String(err)}`);
    }
  }

  async function runClothMutation(
    nodeId: string,
    operation: string,
    preset?: string,
    updatedJson?: any
  ) {
    setContextMenu(null);
    try {
      const updated = await invoke<DocumentOutline>("mutate_cloth_node", {
        nodeId,
        operation,
        preset: preset ?? null,
        updatedJson: updatedJson ?? null,
      });
      setOutline(updated);
      await selectAndInspectNode(nodeId);
      setStatusLeft(updated.undo_label ?? `Completed Cloth ${operation}`);
    } catch (err) {
      setStatusLeft(`Cloth error: ${String(err)}`);
    }
  }

  async function runContainerMutation(
    nodeId: string,
    action: string,
    targetPath = "",
    payload: any = null
  ) {
    setContextMenu(null);
    try {
      const updated = await invoke<DocumentOutline>("mutate_container_node", {
        nodeId,
        action,
        targetPath,
        payload,
      });
      setOutline(updated);
      if (action === "remove_entry") {
        const parent = nodeId.includes("/") ? nodeId.slice(0, nodeId.lastIndexOf("/")) : "root";
        await selectAndInspectNode(parent);
      } else {
        await selectAndInspectNode(nodeId);
      }
      setStatusLeft(updated.undo_label ?? `Completed ${action}`);
    } catch (err) {
      const msg = String(err);
      if (msg.includes("OODLE_DLL_MISSING")) {
        const useZstd = window.confirm(
          "OodleKraken compression requires oo2core_*_win64.dll.\n\nClick OK to compress using ZStandard instead, or Cancel to locate oo2core_*_win64.dll."
        );
        if (useZstd) {
          await runContainerMutation(nodeId, action, targetPath, {
            ...(payload || {}),
            compression: "ZStandard",
            allow_zstd_fallback: true,
          });
        } else {
          await handlePickOodleDll();
        }
        return;
      }
      setStatusLeft(`Container operation failed: ${msg}`);
    }
  }

  // Strictly Context-Sensitive Right-Click Menu Builder for Asset Browser Tree & Viewport
  function buildAssetNodeContextMenu(nodeId: string, kind: string): MenuItemSpec[] {
    const virtualSub = isVirtualSubNode(nodeId);
    const leafSeg = nodeId.split("/").pop() || "";
    const isContainerEntry =
      !virtualSub &&
      nodeId !== "root" &&
      nodeId !== "root:dat" &&
      nodeId !== "root:dtt" &&
      (leafSeg.startsWith("dat:") || leafSeg.startsWith("pkz:") || leafSeg.startsWith("efffile:"));
    const isContainer =
      kind === "DAT" ||
      kind === "DTT" ||
      kind === "PKZ" ||
      kind === "EFF" ||
      kind === "EFF_DIR" ||
      kind === "SCR";

    // 1. Virtual Material Sub-Node (`wmb:mat:X` or `wmb3:mat:X`)
    if (virtualSub && (leafSeg.startsWith("wmb:mat:") || leafSeg.startsWith("wmb3:mat:"))) {
      const isWmb3Mat = leafSeg.startsWith("wmb3:mat:");
      return [
        {
          label: "Copy Material to Clipboard",
          action: () => {
            setContextMenu(null);
            if (isWmb3Mat && wmb3MatDraft) {
              setMaterialClipboard({ game: "b3", payload: structuredClone(wmb3MatDraft) });
              setStatusLeft(`Copied WMB3 material '${wmb3MatDraft.name}' to clipboard`);
            } else if (wmbMatDraft) {
              const g = (wmbB2Override ?? activeWmb?.is_bayo2) ? "b2" : "b1";
              setMaterialClipboard({ game: g, payload: structuredClone(wmbMatDraft) });
              setStatusLeft(`Copied Material #${wmbMatDraft.index} to clipboard`);
            }
          },
        },
        {
          label: "Paste Material from Clipboard",
          disabled: !materialClipboard,
          action: async () => {
            setContextMenu(null);
            if (!materialClipboard || !activeWmb) return;
            const dstGame = activeWmb.is_wmb3
              ? "b3"
              : (wmbB2Override ?? activeWmb.is_bayo2)
              ? "b2"
              : "b1";
            if (materialClipboard.game === dstGame) {
              if (activeWmb.is_wmb3) {
                const matIdx = parseInt(activeWmb.section.replace("mat:", ""), 10) || 0;
                await runWmbMutation(activeWmb.node_id, "update_wmb3_material", {
                  index: matIdx,
                  material: materialClipboard.payload,
                });
              } else {
                const nextMat = {
                  ...materialClipboard.payload,
                  index: wmbMatDraft?.index ?? 0,
                };
                await runWmbMutation(activeWmb.node_id, "update_material", nextMat);
              }
            } else {
              const res = await invoke<{ converted: any }>("convert_material_action", {
                srcGame: materialClipboard.game,
                dstGame,
                materialPayload: materialClipboard.payload,
                targetShader: null,
              });
              if (activeWmb.is_wmb3) {
                const matIdx = parseInt(activeWmb.section.replace("mat:", ""), 10) || 0;
                await runWmbMutation(activeWmb.node_id, "update_wmb3_material", {
                  index: matIdx,
                  material: res.converted,
                });
              } else {
                const nextMat = {
                  ...res.converted,
                  index: wmbMatDraft?.index ?? 0,
                };
                await runWmbMutation(activeWmb.node_id, "update_material", nextMat);
              }
            }
          },
        },
        { separator: true },
        {
          label: "Cross-Game Convert Material",
          submenu: [
            {
              label: "Convert → Bayonetta 1 (0x24..0xBC)",
              action: () => activeWmb && handleCrossGameMaterialConvert(activeWmb, "b1"),
            },
            {
              label: "Convert → Bayonetta 2 (843 Shaders)",
              action: () => activeWmb && handleCrossGameMaterialConvert(activeWmb, "b2"),
            },
            {
              label: "Convert → Bayonetta 3 (WMB3 PBS00)",
              action: () => activeWmb && handleCrossGameMaterialConvert(activeWmb, "b3"),
            },
          ],
        },
      ];
    }

    // 2. Other virtual WMB sub-nodes (`wmb:header`, `wmb:bones`, `wmb:meshes`, `wmb:vertexes`)
    if (virtualSub && (leafSeg.startsWith("wmb:") || leafSeg.startsWith("wmb3:"))) {
      const isWmb3Node = leafSeg.startsWith("wmb3:");
      return [
        {
          label: "Export Model to Wavefront .OBJ...",
          action: async () => {
            setContextMenu(null);
            const dest = await invoke<string | null>("save_file_dialog", {
              title: "Export WMB Model to .OBJ",
              defaultName: "model.obj",
            });
            if (dest) await runWmbMutation(nodeId, "export_obj", { path: dest });
          },
        },
        ...(isWmb3Node
          ? [
              {
                label: "Bake Skeleton & Vertices to T-Pose",
                action: () => runWmbMutation(nodeId, "set_tpose"),
              },
            ]
          : [
              {
                label: "Cleanup Unreferenced Vertexes",
                action: () => runWmbMutation(nodeId, "cleanup_vertexes"),
              },
              {
                label: "Cleanup Unused Materials",
                action: () => runWmbMutation(nodeId, "cleanup_materials"),
              },
              {
                label: "Recompute Relative Bone Positions",
                action: () => runWmbMutation(nodeId, "recompute_relative_positions"),
              },
              {
                label: "Swap Endianness (Wii U ↔ PC)",
                action: () => runWmbMutation(nodeId, "swap_endian"),
              },
            ]),
      ];
    }

    // Standard leaf entry actions inside a parent container (NO Move Up/Down — drag & drop handles reordering!)
    const leafEntryMgmtItems: MenuItemSpec[] = isContainerEntry
      ? [
          {
            label: "Replace Sub-File from Disk...",
            action: async () => {
              setContextMenu(null);
              const src = await invoke<string | null>("pick_file_dialog", {
                title: "Select Replacement Sub-File",
                extensions: null,
              });
              if (src) {
                await runContainerMutation(nodeId, "replace_subfile", src);
              }
            },
          },
          {
            label: "Export Raw Sub-File...",
            action: async () => {
              setContextMenu(null);
              const dest = await invoke<string | null>("save_file_dialog", {
                title: "Export Raw Sub-File",
                defaultName: "extracted.bin",
              });
              if (dest) {
                await runContainerMutation(nodeId, "export_subfile", dest);
                setStatusLeft(`Exported raw sub-file to ${dest}`);
              }
            },
          },
          {
            label: "Rename Entry...",
            action: async () => {
              setContextMenu(null);
              const nextName = window.prompt("Enter new entry filename:");
              if (nextName && nextName.trim()) {
                await runContainerMutation(nodeId, "rename_entry", nextName.trim(), {
                  new_name: nextName.trim(),
                });
              }
            },
          },
          {
            label: "Delete from Container",
            action: () => runContainerMutation(nodeId, "remove_entry"),
          },
        ]
      : [];

    // 2B. PKZ Virtual Folder Node (`PKZ_FOLDER` / `pkzfolder:<prefix>`)
    if (kind === "PKZ_FOLDER" || leafSeg.startsWith("pkzfolder:")) {
      const folderPrefix = leafSeg.replace(/^pkzfolder:/, "");
      return [
        {
          label: `Add File(s) into '${folderPrefix}/'...`,
          action: async () => {
            setContextMenu(null);
            const paths = await invoke<string[]>("pick_multiple_files_dialog", {
              title: `Add File(s) into PKZ Folder '${folderPrefix}/'`,
              extensions: null,
            });
            if (paths && paths.length > 0) {
              await runContainerMutation(nodeId, "add_files", "", {
                paths,
                folder_prefix: folderPrefix,
              });
            }
          },
        },
        {
          label: `Extract Folder '${folderPrefix}/' to Disk...`,
          action: async () => {
            setContextMenu(null);
            const folder = await invoke<string | null>("pick_folder_dialog", {
              title: `Extract PKZ Folder '${folderPrefix}'`,
            });
            if (folder) {
              await runContainerMutation(nodeId, "extract_pkz_folder", folder, {
                folder_prefix: folderPrefix,
              });
              setStatusLeft(`Extracted PKZ folder '${folderPrefix}' to ${folder}`);
              loadDirectory(currentDirRef.current);
            }
          },
        },
        {
          label: "Rename Folder Prefix...",
          action: async () => {
            setContextMenu(null);
            const nextPrefix = window.prompt("Enter new PKZ folder prefix:", folderPrefix);
            if (nextPrefix !== null && nextPrefix.trim() !== folderPrefix) {
              await runContainerMutation(nodeId, "rename_pkz_folder", nextPrefix.trim(), {
                folder_prefix: folderPrefix,
                new_prefix: nextPrefix.trim(),
              });
            }
          },
        },
        { separator: true },
        {
          label: `Delete Folder '${folderPrefix}/' from PKZ`,
          action: () =>
            runContainerMutation(nodeId, "remove_pkz_folder", "", {
              folder_prefix: folderPrefix,
            }),
        },
      ];
    }

    // 3. Container Node (`DAT`, `DTT`, `PKZ`, `EFF`, `EFF_DIR`, `SCR`)
    if (isContainer) {
      const items: MenuItemSpec[] = [];
      if (kind !== "SCR") {
        items.push({
          label: "Add File(s) to Container...",
          action: async () => {
            setContextMenu(null);
            const paths = await invoke<string[]>("pick_multiple_files_dialog", {
              title: "Select File(s) to Add to Container",
              extensions: null,
            });
            if (paths && paths.length > 0) {
              await runContainerMutation(nodeId, "add_files", "", { paths });
            }
          },
        });
      }
      items.push({
        label: "Extract All to Folder...",
        action: async () => {
          setContextMenu(null);
          const folder = await invoke<string | null>("pick_folder_dialog", {
            title: "Extract Container Entries to Folder",
          });
          if (folder) {
            await runContainerMutation(nodeId, "extract_all", folder);
            setStatusLeft(`Extracted all entries to ${folder}`);
            loadDirectory(currentDirRef.current);
          }
        },
      });
      if (nodeId !== "root") {
        items.push({
          label: "Export Raw Container...",
          action: async () => {
            setContextMenu(null);
            const dest = await invoke<string | null>("save_file_dialog", {
              title: "Export Raw Container",
              defaultName: `archive.${kind.toLowerCase()}`,
            });
            if (dest) {
              await runContainerMutation(nodeId, "export_subfile", dest);
              setStatusLeft(`Exported container to ${dest}`);
            }
          },
        });
      }
      if (leafEntryMgmtItems.length > 0) {
        items.push({ separator: true }, ...leafEntryMgmtItems);
      }
      return items;
    }

    // 4. Texture / WTB / BNTX / DDS / GTX / XT1
    if (
      kind === "TEXTURE" ||
      kind === "WTB" ||
      kind === "DDS" ||
      kind === "GTX" ||
      kind === "XT1" ||
      kind === "BNTX"
    ) {
      const targetId =
        kind === "WTB" || kind === "BNTX"
          ? nodeId === "root"
            ? "tex:0"
            : `${nodeId}/tex:0`
          : nodeId;
      return [
        {
          label: "Replace Texture (PNG / DDS / GTX / XT1 / BNTX)...",
          action: () => promptAndRunTextureImport(targetId, "replace_file"),
        },
        {
          label: "Dummy out",
          submenu: [
            {
              label: "White (8x8)",
              action: () => {
                setContextMenu(null);
                runTextureMutation(targetId, "dummy_out", { kind: "white" });
              },
            },
            {
              label: "Black (8x8)",
              action: () => {
                setContextMenu(null);
                runTextureMutation(targetId, "dummy_out", { kind: "black" });
              },
            },
            {
              label: "Normal (128, 128, 255)",
              action: () => {
                setContextMenu(null);
                runTextureMutation(targetId, "dummy_out", { kind: "normal" });
              },
            },
          ],
        },
        { separator: true },
        {
          label: "Export",
          submenu: [
            {
              label: "Export .png...",
              action: () => handleTextureExport(targetId, "single", "png"),
            },
            {
              label: "Export .dds...",
              action: () => handleTextureExport(targetId, "single", "dds"),
            },
            {
              label: "Export raw payload...",
              action: () => handleTextureExport(targetId, "single", "raw"),
            },
          ],
        },
        { separator: true },
        {
          label: "Add texture to bank...",
          action: () => promptAndRunTextureImport(targetId, "add_texture"),
        },
        {
          label: "Delete texture from bank",
          action: () => {
            setContextMenu(null);
            runTextureMutation(targetId, "delete_texture");
          },
        },
        { separator: true },
        {
          label: "Export every texture in this bank",
          submenu: [
            {
              label: "As PNG folder...",
              action: () => handleTextureExport(targetId, "bank", "png"),
            },
            {
              label: "As DDS folder...",
              action: () => handleTextureExport(targetId, "bank", "dds"),
            },
            {
              label: "As raw folder...",
              action: () => handleTextureExport(targetId, "bank", "raw"),
            },
          ],
        },
        {
          label: "Convert Wii U (.gtx) bank to PC (.dds)",
          action: () => {
            setContextMenu(null);
            runTextureMutation(targetId, "convert_wiiu_to_pc");
          },
        },
        {
          label: "Batch replace from folder...",
          action: async () => {
            setContextMenu(null);
            const folder = await invoke<string | null>("pick_folder_dialog", {
              title: "Select Folder of Replacement Textures (matched by tex<idx> or hash)",
            });
            if (folder) await runTextureMutation(targetId, "batch_replace_folder", { folder });
          },
        },
        ...(leafEntryMgmtItems.length > 0 ? [{ separator: true }, ...leafEntryMgmtItems] : []),
      ];
    }

    // 5. WMB / WMB3 Model File
    if (kind.startsWith("WMB")) {
      const isWmb3 = kind === "WMB3";
      return [
        {
          label: "Export Model to Wavefront .OBJ...",
          action: async () => {
            setContextMenu(null);
            const dest = await invoke<string | null>("save_file_dialog", {
              title: "Export WMB Model to .OBJ",
              defaultName: "model.obj",
            });
            if (dest) await runWmbMutation(nodeId, "export_obj", { path: dest });
          },
        },
        ...(isWmb3
          ? [
              {
                label: "Bake WMB3 Skeleton & Vertices to T-Pose",
                action: () => runWmbMutation(nodeId, "set_tpose"),
              },
            ]
          : [
              { separator: true },
              {
                label: "Cleanup & Optimization",
                submenu: [
                  {
                    label: "Cleanup Unreferenced Vertexes",
                    action: () => runWmbMutation(nodeId, "cleanup_vertexes"),
                  },
                  {
                    label: "Cleanup Unused Materials",
                    action: () => runWmbMutation(nodeId, "cleanup_materials"),
                  },
                  {
                    label: "Clamp Material Sizes to Shader Spec",
                    action: () => runWmbMutation(nodeId, "cleanup_material_sizes"),
                  },
                  {
                    label: "Maximize Material Sizes",
                    action: () => runWmbMutation(nodeId, "maximize_material_sizes"),
                  },
                  {
                    label: "Convert Triangle Strips to Triangles",
                    action: () => runWmbMutation(nodeId, "remove_triangle_strips"),
                  },
                ],
              },
              {
                label: "Normals & Skeleton",
                submenu: [
                  {
                    label: "Convert Normals to 10-10-10-2 Wide",
                    action: () => runWmbMutation(nodeId, "normals_to_wide"),
                  },
                  {
                    label: "Check & Normalize Vertex Normals",
                    action: () => runWmbMutation(nodeId, "check_normals"),
                  },
                  {
                    label: "Recompute Relative Bone Positions",
                    action: () => runWmbMutation(nodeId, "recompute_relative_positions"),
                  },
                ],
              },
              {
                label: "Swap Endianness (Wii U ↔ PC)",
                action: () => runWmbMutation(nodeId, "swap_endian"),
              },
            ]),
        ...(leafEntryMgmtItems.length > 0 ? [{ separator: true }, ...leafEntryMgmtItems] : []),
      ];
    }

    // 6. Cloth Physics (`CLP`, `CLH`, `CLW`)
    if (kind === "CLP" || kind === "CLH" || kind === "CLW") {
      return [
        {
          label: "Remap Bone IDs (Cross-Game Presets)",
          submenu: [
            {
              label: "Bayonetta 2 (pl0010) → Bayonetta 1 (pl0010)",
              action: () => runClothMutation(nodeId, "remap_bones", "bayo2_pl0010_to_bayo1"),
            },
            {
              label: "Bayonetta 2 (pl0000) → Bayonetta 1 (pl0010)",
              action: () => runClothMutation(nodeId, "remap_bones", "bayo2_pl0000_to_bayo1"),
            },
            {
              label: "Bayonetta 2 Combat Head → Bayonetta 1",
              action: () => runClothMutation(nodeId, "remap_bones", "bayo2_combat_head"),
            },
            {
              label: "Bayonetta 2 Cutscene Head → Bayonetta 1",
              action: () => runClothMutation(nodeId, "remap_bones", "bayo2_cutscene_head"),
            },
            {
              label: "Bayonetta 2 Common Bones → Bayonetta 1",
              action: () => runClothMutation(nodeId, "remap_bones", "bayo2_common"),
            },
            {
              label: "NieR:Automata → Bayonetta 1",
              action: () => runClothMutation(nodeId, "remap_bones", "nier_to_bayo1"),
            },
          ],
        },
        {
          label: "Swap Endianness (Wii U Big ↔ PC Little)",
          action: () => runClothMutation(nodeId, "swap_endian"),
        },
        ...(leafEntryMgmtItems.length > 0 ? [{ separator: true }, ...leafEntryMgmtItems] : []),
      ];
    }

    // 7. BXM / XML / GAD
    if (kind === "BXM") {
      return [
        {
          label: "Export Decoded XML (.xml)...",
          action: async () => {
            setContextMenu(null);
            const dest = await invoke<string | null>("save_file_dialog", {
              title: "Export Decoded XML (.xml)",
              defaultName: "decoded.xml",
            });
            if (dest) {
              await runContainerMutation(nodeId, "export_bxm_xml", dest);
              setStatusLeft(`Exported decoded UTF-8 XML to ${dest}`);
              loadDirectory(currentDirRef.current);
            }
          },
        },
        {
          label: "Import & Encode from .XML...",
          action: async () => {
            setContextMenu(null);
            const srcXml = await invoke<string | null>("pick_file_dialog", {
              title: "Import & Encode XML into Binary BXM",
              extensions: ["xml"],
            });
            if (srcXml) {
              await runContainerMutation(nodeId, "import_bxm_xml", srcXml);
              setStatusLeft(`Imported and encoded ${srcXml} into BXM`);
            }
          },
        },
        ...(leafEntryMgmtItems.length > 0 ? [{ separator: true }, ...leafEntryMgmtItems] : []),
      ];
    }

    if (leafEntryMgmtItems.length > 0) {
      return leafEntryMgmtItems;
    }

    return [
      {
        label: "Export Raw File...",
        action: async () => {
          setContextMenu(null);
          const dest = await invoke<string | null>("save_file_dialog", {
            title: "Export Raw File",
            defaultName: "extracted.bin",
          });
          if (dest) await runContainerMutation(nodeId, "export_subfile", dest);
        },
      },
      {
        label: "Replace from Disk...",
        action: async () => {
          setContextMenu(null);
          const src = await invoke<string | null>("pick_file_dialog", {
            title: "Select Replacement File",
            extensions: null,
          });
          if (src) await runContainerMutation(nodeId, "replace_subfile", src);
        },
      },
    ];
  }

  function buildFileBrowserContextMenu(entry: FsEntry): MenuItemSpec[] {
    const ext = entry.extension.toLowerCase();
    const runCliOp = async (op: string, dest?: string) => {
      setContextMenu(null);
      try {
        setStatusLeft(`Running ${op} on ${entry.name}...`);
        const msg = await invoke<string>("run_file_context_operation", {
          filePath: entry.path,
          operation: op,
          destPath: dest ?? null,
        });
        setStatusLeft(msg);
        loadDirectory(currentDirRef.current);
      } catch (err) {
        setStatusLeft(`Operation failed: ${String(err)}`);
      }
    };

    const items: MenuItemSpec[] = [
      {
        label: entry.is_dir ? "Open Folder" : "Open in Studio",
        action: () => {
          setContextMenu(null);
          if (entry.is_dir) {
            loadDirectory(entry.path);
          } else {
            openDocumentByPath(entry.path);
          }
        },
      },
      {
        label: "Reload Directory",
        action: () => {
          setContextMenu(null);
          loadDirectory(currentDirRef.current);
        },
      },
      { separator: true },
    ];

    if (entry.is_dir) {
      items.push(
        {
          label: "Repack Folder into .DAT / .DTT Archive",
          action: () => runCliOp("repack_folder_dat"),
        },
        {
          label: "Pack Folder into .PKZ (ZStandard) Archive",
          action: () => runCliOp("repack_folder_pkz"),
        },
        { separator: true }
      );
    } else {
      if (["dat", "dtt", "evn", "pkz", "eff", "idd", "scr", "wtb", "wta", "bntx"].includes(ext)) {
        items.push({
          label: `Extract .${ext.toUpperCase()} to Folder...`,
          action: () => runCliOp("extract_archive"),
        });
      }
      if (["wtb", "wta"].includes(ext)) {
        items.push({
          label: "Convert Wii U (.gtx) → PC (.wtb)",
          action: () => runCliOp("convert_wtb_wiiu_pc"),
        });
      }
      if (ext === "wmb" || ext === "mdb") {
        items.push({
          label: "Export Model to Wavefront .OBJ",
          action: () => runCliOp("wmb_export_obj"),
        });
        if (ext === "wmb") {
          items.push({
            label: "Swap Endianness (Wii U ↔ PC)",
            action: () => runCliOp("swap_endian_file"),
          });
        }
      }
      if (ext === "bxm" || ext === "gad") {
        items.push({
          label: "Decode Binary .BXM/.GAD → .XML",
          action: () => runCliOp("bxm_decode_xml"),
        });
      }
      if (ext === "xml") {
        items.push({
          label: "Encode .XML → Binary .BXM",
          action: () => runCliOp("bxm_encode_xml"),
        });
      }
      if (["clp", "clh", "clw", "mot", "exp"].includes(ext)) {
        items.push({
          label: `Swap .${ext.toUpperCase()} Endianness (Wii U ↔ PC)`,
          action: () => runCliOp("swap_endian_file"),
        });
      }
      items.push({ separator: true });
    }

    const targetPaths =
      selectedFilePaths.includes(entry.path) && selectedFilePaths.length > 1
        ? selectedFilePaths
        : [entry.path];

    const runFsBatch = async (
      op: string,
      paths: string[],
      destDir?: string,
      newName?: string
    ) => {
      setContextMenu(null);
      try {
        const msg = await invoke<string>("fs_batch_operation", {
          operation: op,
          sourcePaths: paths,
          destDir: destDir ?? null,
          newName: newName ?? null,
        });
        setStatusLeft(msg);
        await loadDirectory(currentDirRef.current);
      } catch (err) {
        setStatusLeft(`File operation failed: ${String(err)}`);
      }
    };

    items.push(
      {
        label: targetPaths.length > 1 ? `Copy (${targetPaths.length} items)` : "Copy",
        shortcut: "Ctrl+C",
        action: () => {
          setContextMenu(null);
          setFsClipboard({ mode: "copy", paths: targetPaths });
          setStatusLeft(`Copied ${targetPaths.length} item(s) to clipboard`);
        },
      },
      {
        label: targetPaths.length > 1 ? `Cut (${targetPaths.length} items)` : "Cut",
        shortcut: "Ctrl+X",
        action: () => {
          setContextMenu(null);
          setFsClipboard({ mode: "cut", paths: targetPaths });
          setStatusLeft(`Cut ${targetPaths.length} item(s) to clipboard`);
        },
      },
      {
        label: fsClipboard
          ? `Paste into Current Folder (${fsClipboard.paths.length})`
          : "Paste",
        shortcut: "Ctrl+V",
        disabled: !fsClipboard || fsClipboard.paths.length === 0,
        action: async () => {
          if (!fsClipboard || !currentDir) return;
          const op = fsClipboard.mode === "cut" ? "paste_cut" : "paste_copy";
          const paths = fsClipboard.paths;
          if (fsClipboard.mode === "cut") setFsClipboard(null);
          await runFsBatch(op, paths, currentDir);
        },
      },
      {
        label: "Duplicate",
        action: () => runFsBatch("duplicate", targetPaths),
      },
      {
        label: "Rename...",
        shortcut: "F2",
        action: async () => {
          setContextMenu(null);
          const nextName = window.prompt("Rename to:", entry.name);
          if (nextName && nextName.trim() && nextName.trim() !== entry.name) {
            await runFsBatch("rename", [entry.path], undefined, nextName.trim());
          }
        },
      },
      {
        label: targetPaths.length > 1 ? `Delete (${targetPaths.length} items)` : "Delete",
        shortcut: "Del",
        action: async () => {
          setContextMenu(null);
          const ok = window.confirm(
            targetPaths.length > 1
              ? `Delete ${targetPaths.length} selected items from disk?`
              : `Delete '${entry.name}' from disk?`
          );
          if (ok) {
            await runFsBatch("delete", targetPaths);
            setSelectedFilePaths([]);
          }
        },
      },
      { separator: true },
      {
        label: "New Folder...",
        action: async () => {
          setContextMenu(null);
          const folderName = window.prompt("New folder name:", "New Folder");
          if (folderName && folderName.trim()) {
            await runFsBatch("new_folder", [], currentDir, folderName.trim());
          }
        },
      },
      {
        label: "Select All",
        shortcut: "Ctrl+A",
        action: () => {
          setContextMenu(null);
          setSelectedFilePaths(filteredFiles.map((f) => f.path));
        },
      },
      { separator: true },
      {
        label: "Copy full path",
        action: () => {
          setContextMenu(null);
          navigator.clipboard?.writeText(entry.path);
          setStatusLeft(`Copied ${entry.path}`);
        },
      },
      {
        label: "Show in Explorer",
        action: () => {
          setContextMenu(null);
          invoke("reveal_in_explorer", { path: entry.path });
        },
      }
    );

    return items;
  }

  // Filtered and sorted File Browser entries
  const filteredFiles = useMemo(() => {
    const regex = compileGlobFilter(fileGlobFilter);
    const list = fsEntries.filter((e) => {
      if (hideUnsupported && !e.is_dir && !e.is_supported) return false;
      if (!regex) return true;
      return regex.test(e.name);
    });
    list.sort((a, b) => {
      if (a.is_dir !== b.is_dir) return a.is_dir ? -1 : 1;
      let cmp = 0;
      if (fileSortCol === "name") {
        cmp = a.name.localeCompare(b.name, undefined, { numeric: true, sensitivity: "base" });
      } else if (fileSortCol === "type") {
        cmp =
          a.extension.localeCompare(b.extension, undefined, { sensitivity: "base" }) ||
          a.name.localeCompare(b.name, undefined, { numeric: true, sensitivity: "base" });
      } else if (fileSortCol === "modified") {
        cmp = a.modified_epoch_ms - b.modified_epoch_ms;
      } else {
        cmp = a.size - b.size;
      }
      return fileSortAsc ? cmp : -cmp;
    });
    return list;
  }, [fsEntries, fileGlobFilter, hideUnsupported, fileSortCol, fileSortAsc]);

  function toggleCard(key: string) {
    setOpenCards((prev) => ({ ...prev, [key]: !prev[key] }));
  }

  function toggleSort(col: FileSortCol) {
    if (fileSortCol === col) {
      setFileSortAsc(!fileSortAsc);
    } else {
      setFileSortCol(col);
      setFileSortAsc(true);
    }
  }

  // Recursive Asset Tree Renderer with Pointer-Based Drag Reordering + Drag-Out to Explorer
  function renderTreeNodes(nodes: AssetTreeNode[]) {
    const filterLow = assetSearch.trim().toLowerCase();
    return nodes.map((node) => {
      const hasChildren = node.children && node.children.length > 0;
      const isExpanded = !!expandedNodes[node.id];
      const isSelected = selectedNodeId === node.id;
      const canDragNode = node.id !== "root" && !isVirtualSubNode(node.id);
      const isDragOverBefore =
        pointerDrag?.active &&
        pointerDrag.kind === "asset" &&
        pointerDrag.overId === node.id &&
        pointerDrag.insertPos === "before";
      const isDragOverAfter =
        pointerDrag?.active &&
        pointerDrag.kind === "asset" &&
        pointerDrag.overId === node.id &&
        pointerDrag.insertPos === "after";

      const matchesFilter = (n: AssetTreeNode): boolean => {
        if (!filterLow) return true;
        if (
          n.label.toLowerCase().includes(filterLow) ||
          n.format_badge.toLowerCase().includes(filterLow)
        ) {
          return true;
        }
        return n.children?.some(matchesFilter) ?? false;
      };
      if (!matchesFilter(node)) return null;

      return (
        <div key={node.id}>
          <div
            data-asset-node-id={node.id}
            className={`tree-row ${isSelected ? "selected" : ""} ${
              isDragOverBefore ? "drag-insert-before" : ""
            } ${isDragOverAfter ? "drag-insert-after" : ""}`}
            onPointerDown={(e) => {
              if (e.button !== 0 || !canDragNode) return;
              setPointerDrag({
                kind: "asset",
                sourceId: node.id,
                label: node.label,
                startX: e.clientX,
                startY: e.clientY,
                curX: e.clientX,
                curY: e.clientY,
                active: false,
                overId: null,
                insertPos: null,
                dragOutTriggered: false,
              });
            }}
            onClick={() => {
              if (hasChildren && !expandedNodes[node.id]) {
                setExpandedNodes((p) => ({ ...p, [node.id]: true }));
              }
              selectAndInspectNode(node.id);
            }}
            onContextMenu={(e) => {
              e.preventDefault();
              e.stopPropagation();
              selectAndInspectNode(node.id);
              setHoverSubmenuIdx(null);
              const items = buildAssetNodeContextMenu(node.id, node.kind);
              if (items.length > 0) {
                setContextMenu({
                  x: Math.min(e.clientX, window.innerWidth - 260),
                  y: Math.min(e.clientY, window.innerHeight - 340),
                  items,
                });
              }
            }}
          >
            <span
              className={`tree-arrow ${isExpanded ? "expanded" : ""}`}
              style={{ visibility: hasChildren ? "visible" : "hidden" }}
              onClick={(e) => {
                e.stopPropagation();
                setExpandedNodes((p) => ({ ...p, [node.id]: !p[node.id] }));
              }}
            >
              <svg width="6" height="9" viewBox="0 0 6 10" fill="none" stroke="currentColor" strokeWidth="1.5">
                <path d="M1 1L5 5L1 9" strokeLinecap="round" strokeLinejoin="round" />
              </svg>
            </span>
            <span className="tree-badge">{node.format_badge}</span>
            <span style={{ overflow: "hidden", textOverflow: "ellipsis" }}>{node.label}</span>
          </div>
          {hasChildren && (isExpanded || !!filterLow) && (
            <div className="tree-children">{renderTreeNodes(node.children)}</div>
          )}
        </div>
      );
    });
  }

  function getBxmNodeByPath(root: BxmXmlNode | null, pathStr: string): BxmXmlNode | null {
    if (!root) return null;
    if (pathStr === "0") return root;
    const indices = pathStr
      .split(".")
      .slice(1)
      .map((x) => parseInt(x, 10));
    let cur: BxmXmlNode = root;
    for (const idx of indices) {
      if (!cur.children || !cur.children[idx]) return cur;
      cur = cur.children[idx];
    }
    return cur;
  }

  function buildBxmNodeContextMenu(pathStr: string): MenuItemSpec[] {
    if (!bxmTreeDraft) return [];
    const isRoot = pathStr === "0";
    return [
      {
        label: "+ Add Child Element",
        action: () => {
          setContextMenu(null);
          const copy = structuredClone(bxmTreeDraft);
          const target = getBxmNodeByPath(copy, pathStr);
          if (target) {
            target.children.push({
              name: "Item",
              attributes: [["id", String(target.children.length)]],
              value: null,
              children: [],
            });
            setBxmExpandedPaths((p) => ({ ...p, [pathStr]: true }));
            updateBxmDraftTree(copy);
            setSelectedBxmPath(`${pathStr}.${target.children.length - 1}`);
          }
        },
      },
      {
        label: "+ Add Sibling Element",
        disabled: isRoot,
        action: () => {
          setContextMenu(null);
          if (isRoot) return;
          const parentPath = pathStr.slice(0, pathStr.lastIndexOf("."));
          const idx = parseInt(pathStr.split(".").pop() || "0", 10);
          const copy = structuredClone(bxmTreeDraft);
          const parent = getBxmNodeByPath(copy, parentPath);
          if (parent) {
            parent.children.splice(idx + 1, 0, {
              name: parent.children[idx]?.name || "Item",
              attributes: [],
              value: null,
              children: [],
            });
            updateBxmDraftTree(copy);
            setSelectedBxmPath(`${parentPath}.${idx + 1}`);
          }
        },
      },
      {
        label: "Duplicate Element",
        disabled: isRoot,
        action: () => {
          setContextMenu(null);
          if (isRoot) return;
          const parentPath = pathStr.slice(0, pathStr.lastIndexOf("."));
          const idx = parseInt(pathStr.split(".").pop() || "0", 10);
          const copy = structuredClone(bxmTreeDraft);
          const parent = getBxmNodeByPath(copy, parentPath);
          if (parent && parent.children[idx]) {
            parent.children.splice(idx + 1, 0, structuredClone(parent.children[idx]));
            updateBxmDraftTree(copy);
            setSelectedBxmPath(`${parentPath}.${idx + 1}`);
          }
        },
      },
      {
        label: "+ Add @Attribute",
        action: () => {
          setContextMenu(null);
          const copy = structuredClone(bxmTreeDraft);
          const target = getBxmNodeByPath(copy, pathStr);
          if (target) {
            target.attributes.push([`attr${target.attributes.length}`, "0"]);
            updateBxmDraftTree(copy);
          }
        },
      },
      { separator: true },
      {
        label: "Delete Element",
        disabled: isRoot,
        action: () => {
          setContextMenu(null);
          if (isRoot) return;
          const parentPath = pathStr.slice(0, pathStr.lastIndexOf("."));
          const idx = parseInt(pathStr.split(".").pop() || "0", 10);
          const copy = structuredClone(bxmTreeDraft);
          const parent = getBxmNodeByPath(copy, parentPath);
          if (parent && parent.children[idx]) {
            parent.children.splice(idx, 1);
            updateBxmDraftTree(copy);
            setSelectedBxmPath(parentPath);
          }
        },
      },
    ];
  }

  // Interactive BXM Tree Renderer with Inline Editing + Pointer Drag-and-Drop Reordering
  function renderBxmInteractiveTree(node: BxmXmlNode, pathStr: string) {
    const isSel = selectedBxmPath === pathStr;
    const hasChildren = node.children && node.children.length > 0;
    const isExpanded = bxmExpandedPaths[pathStr] ?? true;
    const isDragBefore =
      pointerDrag?.active &&
      pointerDrag.kind === "bxm" &&
      pointerDrag.overId === pathStr &&
      pointerDrag.insertPos === "before";
    const isDragAfter =
      pointerDrag?.active &&
      pointerDrag.kind === "bxm" &&
      pointerDrag.overId === pathStr &&
      pointerDrag.insertPos === "after";

    return (
      <div key={pathStr} style={{ marginLeft: pathStr === "0" ? 0 : 16 }}>
        <div
          data-bxm-path={pathStr}
          className={`tree-row ${isSel ? "selected" : ""} ${
            isDragBefore ? "drag-insert-before" : ""
          } ${isDragAfter ? "drag-insert-after" : ""}`}
          style={{ flexWrap: "wrap", gap: 6, padding: "4px 6px" }}
          onPointerDown={(e) => {
            if (e.button !== 0 || pathStr === "0") return;
            if ((e.target as HTMLElement).tagName === "INPUT") return;
            setPointerDrag({
              kind: "bxm",
              sourceId: pathStr,
              label: `<${node.name}>`,
              startX: e.clientX,
              startY: e.clientY,
              curX: e.clientX,
              curY: e.clientY,
              active: false,
              overId: null,
              insertPos: null,
              dragOutTriggered: false,
            });
          }}
          onClick={() => setSelectedBxmPath(pathStr)}
          onContextMenu={(e) => {
            e.preventDefault();
            e.stopPropagation();
            setSelectedBxmPath(pathStr);
            setHoverSubmenuIdx(null);
            setContextMenu({
              x: Math.min(e.clientX, window.innerWidth - 240),
              y: Math.min(e.clientY, window.innerHeight - 240),
              items: buildBxmNodeContextMenu(pathStr),
            });
          }}
        >
          <span
            className={`tree-arrow ${isExpanded ? "expanded" : ""}`}
            style={{ visibility: hasChildren ? "visible" : "hidden" }}
            onClick={(e) => {
              e.stopPropagation();
              setBxmExpandedPaths((p) => ({ ...p, [pathStr]: !isExpanded }));
            }}
          >
            <svg width="6" height="9" viewBox="0 0 6 10" fill="none" stroke="currentColor" strokeWidth="1.5">
              <path d="M1 1L5 5L1 9" strokeLinecap="round" strokeLinejoin="round" />
            </svg>
          </span>

          {/* Inline Tag Name */}
          <span className="mono" style={{ color: "var(--brush-muted)" }}>&lt;</span>
          <input
            type="text"
            className="field-box mono"
            style={{
              width: Math.max(64, (node.name?.length || 4) * 8 + 18),
              height: 22,
              padding: "1px 5px",
              fontWeight: 600,
              fontSize: 11.5,
            }}
            value={node.name}
            onClick={(e) => e.stopPropagation()}
            onChange={(e) => {
              if (!bxmTreeDraft) return;
              const copy = structuredClone(bxmTreeDraft);
              const target = getBxmNodeByPath(copy, pathStr);
              if (target) {
                target.name = e.target.value;
                updateBxmDraftTree(copy);
              }
            }}
          />
          <span className="mono" style={{ color: "var(--brush-muted)" }}>&gt;</span>

          {/* Inline Attributes */}
          {node.attributes.map(([k, v], idx) => (
            <span
              key={idx}
              style={{ display: "inline-flex", alignItems: "center", gap: 3 }}
              onClick={(e) => e.stopPropagation()}
            >
              <input
                type="text"
                className="field-box mono"
                style={{
                  width: Math.max(48, k.length * 7.5 + 14),
                  height: 21,
                  padding: "1px 4px",
                  fontSize: 11,
                  color: "var(--brush-muted)",
                }}
                value={k}
                onChange={(e) => {
                  if (!bxmTreeDraft) return;
                  const copy = structuredClone(bxmTreeDraft);
                  const target = getBxmNodeByPath(copy, pathStr);
                  if (target && target.attributes[idx]) {
                    target.attributes[idx][0] = e.target.value;
                    updateBxmDraftTree(copy);
                  }
                }}
              />
              <span className="mono" style={{ color: "var(--brush-muted)" }}>=</span>
              <input
                type="text"
                className="field-box mono"
                style={{
                  width: Math.max(62, (v?.length || 2) * 7.5 + 16),
                  height: 21,
                  padding: "1px 5px",
                  fontSize: 11,
                }}
                value={v}
                onChange={(e) => {
                  if (!bxmTreeDraft) return;
                  const copy = structuredClone(bxmTreeDraft);
                  const target = getBxmNodeByPath(copy, pathStr);
                  if (target && target.attributes[idx]) {
                    target.attributes[idx][1] = e.target.value;
                    updateBxmDraftTree(copy);
                  }
                }}
              />
            </span>
          ))}

          {/* Inline Element Text Value */}
          <input
            type="text"
            className="field-box mono"
            placeholder="(value)"
            style={{
              width: Math.max(75, ((node.value ?? "").length || 5) * 7.5 + 18),
              height: 21,
              padding: "1px 6px",
              fontSize: 11.5,
              color: "var(--brush-accent)",
            }}
            value={node.value ?? ""}
            onClick={(e) => e.stopPropagation()}
            onChange={(e) => {
              if (!bxmTreeDraft) return;
              const copy = structuredClone(bxmTreeDraft);
              const target = getBxmNodeByPath(copy, pathStr);
              if (target) {
                target.value = e.target.value.length > 0 ? e.target.value : null;
                updateBxmDraftTree(copy);
              }
            }}
          />
        </div>
        {hasChildren && isExpanded && (
          <div>{node.children.map((c, i) => renderBxmInteractiveTree(c, `${pathStr}.${i}`))}</div>
        )}
      </div>
    );
  }

  // Automatic / Optional Table View for BXM / XML / GAD
  function renderBxmTableView() {
    if (!bxmTreeDraft) return null;
    // Choose the selected node if it has children, otherwise root
    const currentContainerPath = (() => {
      const sel = getBxmNodeByPath(bxmTreeDraft, selectedBxmPath);
      if (sel && sel.children && sel.children.length > 0) return selectedBxmPath;
      if (selectedBxmPath.includes(".")) {
        return selectedBxmPath.slice(0, selectedBxmPath.lastIndexOf("."));
      }
      return "0";
    })();
    const tableParent = getBxmNodeByPath(bxmTreeDraft, currentContainerPath) || bxmTreeDraft;
    const rows = tableParent.children || [];

    // Collect union of all @attribute names and leaf child tag names across rows
    const attrCols: string[] = [];
    const childCols: string[] = [];
    for (const r of rows) {
      for (const [ak] of r.attributes || []) {
        if (ak && !attrCols.includes(ak)) attrCols.push(ak);
      }
      for (const ch of r.children || []) {
        if (ch.name && (!ch.children || ch.children.length === 0) && !childCols.includes(ch.name)) {
          childCols.push(ch.name);
        }
      }
    }

    return (
      <div style={{ display: "flex", flexDirection: "column", gap: 10 }}>
        <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center" }}>
          <span style={{ fontWeight: 600, color: "var(--brush-heading)" }}>
            Table View — &lt;{tableParent.name}&gt; ({rows.length} child rows)
          </span>
          <div style={{ display: "flex", gap: 8 }}>
            <button
              className="pill-button"
              onClick={() => {
                const copy = structuredClone(bxmTreeDraft);
                const parent = getBxmNodeByPath(copy, currentContainerPath);
                if (!parent) return;
                const template = parent.children[parent.children.length - 1];
                const newRow: BxmXmlNode = template
                  ? structuredClone(template)
                  : {
                      name: "Entry",
                      attributes: attrCols.map((k) => [k, ""]),
                      value: null,
                      children: childCols.map((cn) => ({
                        name: cn,
                        attributes: [],
                        value: "",
                        children: [],
                      })),
                    };
                parent.children.push(newRow);
                updateBxmDraftTree(copy);
              }}
            >
              + Add Row
            </button>
            <button className="pill-button accent" onClick={() => commitBxmDraft()}>
              Commit BXM Changes
            </button>
          </div>
        </div>

        {(() => {
          const showTextValueCol = rows.some((r) => (r.value ?? "").trim().length > 0);
          return (
        <table className="struct-table">
          <thead>
            <tr>
              <th>#</th>
              <th>Tag</th>
              {showTextValueCol && <th>Text Value</th>}
              {attrCols.map((ak) => (
                <th key={`attr:${ak}`} className="mono">
                  @{ak}
                </th>
              ))}
              {childCols.map((ck) => (
                <th key={`child:${ck}`} className="mono">
                  &lt;{ck}&gt;
                </th>
              ))}
              <th>Actions</th>
            </tr>
          </thead>
          <tbody>
            {rows.map((r, ri) => (
              <tr key={ri}>
                <td className="mono">#{ri}</td>
                <td>
                  <input
                    type="text"
                    size={1}
                    className="field-box mono"
                    style={{ height: 24, fontWeight: 600 }}
                    value={r.name}
                    onChange={(e) => {
                      const copy = structuredClone(bxmTreeDraft);
                      const parent = getBxmNodeByPath(copy, currentContainerPath);
                      if (parent && parent.children[ri]) {
                        parent.children[ri].name = e.target.value;
                        updateBxmDraftTree(copy);
                      }
                    }}
                  />
                </td>
                {showTextValueCol && (
                  <td>
                    <input
                      type="text"
                      size={1}
                      className="field-box mono"
                      style={{ height: 24 }}
                      value={r.value ?? ""}
                      onChange={(e) => {
                        const copy = structuredClone(bxmTreeDraft);
                        const parent = getBxmNodeByPath(copy, currentContainerPath);
                        if (parent && parent.children[ri]) {
                          parent.children[ri].value = e.target.value.length > 0 ? e.target.value : null;
                          updateBxmDraftTree(copy);
                        }
                      }}
                    />
                  </td>
                )}
                {attrCols.map((ak) => {
                  const pair = r.attributes.find(([k]) => k === ak);
                  return (
                    <td key={`attr:${ak}`}>
                      <input
                        type="text"
                        size={1}
                        className="field-box mono"
                        style={{ height: 24 }}
                        value={pair ? pair[1] : ""}
                        onChange={(e) => {
                          const copy = structuredClone(bxmTreeDraft);
                          const parent = getBxmNodeByPath(copy, currentContainerPath);
                          if (parent && parent.children[ri]) {
                            const rowNode = parent.children[ri];
                            const existing = rowNode.attributes.find(([k]) => k === ak);
                            if (existing) {
                              existing[1] = e.target.value;
                            } else {
                              rowNode.attributes.push([ak, e.target.value]);
                            }
                            updateBxmDraftTree(copy);
                          }
                        }}
                      />
                    </td>
                  );
                })}
                {childCols.map((ck) => {
                  const childNode = r.children.find((c) => c.name === ck);
                  return (
                    <td key={`child:${ck}`}>
                      <input
                        type="text"
                        size={1}
                        className="field-box mono"
                        style={{ height: 24 }}
                        value={childNode?.value ?? ""}
                        onChange={(e) => {
                          const copy = structuredClone(bxmTreeDraft);
                          const parent = getBxmNodeByPath(copy, currentContainerPath);
                          if (parent && parent.children[ri]) {
                            const rowNode = parent.children[ri];
                            const existing = rowNode.children.find((c) => c.name === ck);
                            if (existing) {
                              existing.value = e.target.value;
                            } else {
                              rowNode.children.push({
                                name: ck,
                                attributes: [],
                                value: e.target.value,
                                children: [],
                              });
                            }
                            updateBxmDraftTree(copy);
                          }
                        }}
                      />
                    </td>
                  );
                })}
                <td>
                  <div style={{ display: "inline-flex", gap: 4 }}>
                    <button
                      className="pill-button"
                      style={{ height: 23, padding: "0 7px", fontSize: 11 }}
                      title="Duplicate Row"
                      onClick={() => {
                        const copy = structuredClone(bxmTreeDraft);
                        const parent = getBxmNodeByPath(copy, currentContainerPath);
                        if (parent && parent.children[ri]) {
                          parent.children.splice(ri + 1, 0, structuredClone(parent.children[ri]));
                          updateBxmDraftTree(copy);
                        }
                      }}
                    >
                      ⧉
                    </button>
                    <button
                      className="pill-button"
                      style={{ height: 23, padding: "0 7px", fontSize: 11 }}
                      title="Delete Row"
                      onClick={() => {
                        const copy = structuredClone(bxmTreeDraft);
                        const parent = getBxmNodeByPath(copy, currentContainerPath);
                        if (parent && parent.children[ri]) {
                          parent.children.splice(ri, 1);
                          updateBxmDraftTree(copy);
                        }
                      }}
                    >
                      ✕
                    </button>
                  </div>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
          );
        })()}
      </div>
    );
  }

  const activeTexture: TextureNodeView | null =
    nodePayload?.view_type === "TEXTURE" ? nodePayload.texture : null;
  const activeWmb: WmbSectionView | null =
    nodePayload?.view_type === "WMB" ? nodePayload.wmb : null;

  // Helper to match a texture ID / slot against `wmb.bank_textures` and `wmb.core_pretextures`
  function resolveSamplerMatches(
    texId: number,
    matchBySlot: boolean,
    bankTextures: MaterialBankTextureOption[],
    corePretextures: CorePretextureInfo[]
  ): {
    bankMatch: MaterialBankTextureOption | undefined;
    pretexMatch: CorePretextureInfo | undefined;
  } {
    const unsigned = texId >>> 0;
    const bankMatch = matchBySlot
      ? bankTextures.find((b) => b.index === unsigned) ||
        bankTextures.find((b) => (b.texture_id >>> 0) === unsigned)
      : bankTextures.find((b) => (b.texture_id >>> 0) === unsigned);
    const pretexMatch = corePretextures.find((p) => ((p.id ?? p.tex_id ?? 0) >>> 0) === unsigned);
    return { bankMatch, pretexMatch };
  }

  // Render clean structured tables & Material Editor for WMB1 / WMB2 / WMB3
  function renderWmbStructuredCenter(wmb: WmbSectionView) {
    const bankTextures: MaterialBankTextureOption[] = Array.isArray(wmb.bank_textures)
      ? wmb.bank_textures
      : [];
    const corePretextures: CorePretextureInfo[] = Array.isArray(wmb.core_pretextures)
      ? wmb.core_pretextures
      : [];
    const availableShaders: ShaderOptionDesc[] = Array.isArray(wmb.available_shaders)
      ? wmb.available_shaders
      : [];

    // 1. Single Active Material Editor (WMB1 / WMB2 or WMB3)
    if (wmb.section.startsWith("mat:")) {
      // 1A. WMB3 Material Editor (Bayonetta 3 / NieR:Automata / Astral Chain)
      if (wmb.is_wmb3 && wmb3MatDraft) {
        const matIdx = parseInt(wmb.section.replace("mat:", ""), 10) || 0;
        const texViews = Array.isArray(wmb.wmb3_texture_views) ? wmb.wmb3_texture_views : [];
        const namedParams = Array.isArray(wmb.wmb3_named_params) ? wmb.wmb3_named_params : [];

        return (
          <div style={{ display: "flex", flexDirection: "column", gap: 16 }}>
            {/* Top Material Action Bar */}
            <div
              style={{
                display: "flex",
                flexWrap: "wrap",
                justifyContent: "space-between",
                alignItems: "center",
                gap: 10,
                padding: "10px 12px",
                borderRadius: 8,
                backgroundColor: "var(--brush-card)",
                border: "1px solid var(--brush-border)",
              }}
            >
              <div style={{ display: "flex", alignItems: "center", gap: 10, flexWrap: "wrap" }}>
                <span style={{ fontWeight: 600, color: "var(--brush-heading)" }}>Shader:</span>
                <select
                  className="pill-select mono"
                  style={{ minWidth: 230 }}
                  value={wmb3MatDraft.shader_name}
                  onChange={(e) => {
                    const nextShader = e.target.value;
                    runWmbMutation(wmb.node_id, "switch_shader", {
                      index: matIdx,
                      shader: nextShader,
                    });
                  }}
                >
                  <option value={wmb3MatDraft.shader_name}>{wmb3MatDraft.shader_name}</option>
                  {availableShaders
                    .filter((s) => (s.key ?? s.id) !== wmb3MatDraft.shader_name)
                    .map((s) => {
                      const shKey = s.key ?? s.id ?? s.label;
                      return (
                        <option key={shKey} value={shKey}>
                          {s.label}
                        </option>
                      );
                    })}
                </select>

                <span style={{ color: "var(--brush-muted)" }}>Technique:</span>
                <input
                  type="text"
                  className="field-box mono"
                  style={{ width: 145, height: 28 }}
                  value={wmb3MatDraft.technique_name}
                  onChange={(e) => {
                    const copy = structuredClone(wmb3MatDraft);
                    copy.technique_name = e.target.value;
                    setWmb3MatDraft(copy);
                  }}
                />
              </div>

              <div style={{ display: "flex", alignItems: "center", gap: 6, flexWrap: "wrap" }}>
                <button
                  className="pill-button"
                  onClick={() => {
                    setMaterialClipboard({ game: "b3", payload: structuredClone(wmb3MatDraft) });
                    setStatusLeft(`Copied WMB3 material '${wmb3MatDraft.name}' to clipboard`);
                  }}
                >
                  Copy
                </button>
                <button
                  className="pill-button"
                  disabled={!materialClipboard}
                  onClick={async () => {
                    if (!materialClipboard) return;
                    try {
                      if (materialClipboard.game === "b3") {
                        const nextMat = {
                          ...structuredClone(materialClipboard.payload),
                          name: wmb3MatDraft.name,
                        };
                        setWmb3MatDraft(nextMat);
                        await runWmbMutation(wmb.node_id, "update_wmb3_material", {
                          index: matIdx,
                          material: nextMat,
                        });
                      } else {
                        const res = await invoke<{ converted: any }>("convert_material_action", {
                          srcGame: materialClipboard.game,
                          dstGame: "b3",
                          materialPayload: materialClipboard.payload,
                          targetShader: null,
                        });
                        const nextMat = {
                          ...res.converted,
                          name: wmb3MatDraft.name || res.converted?.name,
                        };
                        setWmb3MatDraft(nextMat);
                        await runWmbMutation(wmb.node_id, "update_wmb3_material", {
                          index: matIdx,
                          material: nextMat,
                        });
                      }
                    } catch (err) {
                      setStatusLeft(`Material paste error: ${String(err)}`);
                    }
                  }}
                  title="Paste material from clipboard"
                >
                  Paste
                </button>
                <button
                  className="pill-button"
                  onClick={() => handleCrossGameMaterialConvert(wmb, "b2")}
                  title="Convert this WMB3 material to Bayonetta 2 format and copy to clipboard"
                >
                  → B2
                </button>
                <button
                  className="pill-button"
                  onClick={() => handleCrossGameMaterialConvert(wmb, "b1")}
                  title="Convert this WMB3 material to Bayonetta 1 format and copy to clipboard"
                >
                  → B1
                </button>
                <button
                  className="pill-button accent"
                  onClick={() =>
                    runWmbMutation(wmb.node_id, "update_wmb3_material", {
                      index: matIdx,
                      material: wmb3MatDraft,
                    })
                  }
                >
                  Apply WMB3 Material
                </button>
              </div>
            </div>

            {/* WMB3 Texture Samplers with Live Thumbnails, Bank Picker & Core Pretextures */}
            <div>
              <div style={{ fontWeight: 600, color: "var(--brush-heading)", marginBottom: 8 }}>
                Texture Samplers ({wmb3MatDraft.textures.length})
              </div>
              <div className="mat-sampler-grid">
                {wmb3MatDraft.textures.map(([samplerName, texId], ti) => {
                  const viewSlot = texViews[ti];
                  const { bankMatch, pretexMatch } = resolveSamplerMatches(
                    texId,
                    false,
                    bankTextures,
                    corePretextures
                  );
                  return (
                    <div key={ti} className="mat-sampler-card">
                      <div className="mat-sampler-thumb">
                        {bankMatch?.thumbnail_data_url ? (
                          <img src={bankMatch.thumbnail_data_url} alt={samplerName} />
                        ) : (
                          <span className="mono" style={{ fontSize: 10, color: "var(--brush-muted)" }}>
                            {pretexMatch ? "PRE" : "TEX"}
                          </span>
                        )}
                      </div>
                      <div style={{ flex: 1, minWidth: 0, display: "flex", flexDirection: "column", gap: 4 }}>
                        <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", gap: 6 }}>
                          <span className="mono" style={{ fontWeight: 600, fontSize: 12 }}>
                            {samplerName}
                          </span>
                          <span style={{ color: "var(--brush-muted)", fontSize: 11 }}>
                            {viewSlot?.pretexture_name || viewSlot?.role || `Slot #${ti}`}
                          </span>
                        </div>

                        {pretexMatch && (
                          <div
                            className="mono"
                            style={{
                              fontSize: 10.5,
                              color: "var(--brush-accent)",
                            }}
                          >
                            Core Pretexture: {pretexMatch.name}
                          </div>
                        )}

                        <TextureSlotPickerPopover
                          value={texId >>> 0}
                          isB1SlotMode={false}
                          bankTextures={bankTextures}
                          corePretextures={corePretextures}
                          onChange={(nextVal) => {
                            const copy = structuredClone(wmb3MatDraft);
                            copy.textures[ti][1] = nextVal >>> 0;
                            setWmb3MatDraft(copy);
                          }}
                        />

                        <div style={{ display: "flex", alignItems: "center", gap: 6 }}>
                          <input
                            type="text"
                            className="field-box mono"
                            style={{ flex: 1, height: 24, fontSize: 11.5 }}
                            value={`0x${(texId >>> 0).toString(16).toUpperCase().padStart(8, "0")}`}
                            onChange={(e) => {
                              const clean = e.target.value.replace(/^0x/i, "");
                              const parsed = parseInt(clean || "0", 16);
                              if (!Number.isNaN(parsed)) {
                                const copy = structuredClone(wmb3MatDraft);
                                copy.textures[ti][1] = parsed >>> 0;
                                setWmb3MatDraft(copy);
                              }
                            }}
                          />
                          {bankMatch && (
                            <button
                              className="pill-button"
                              style={{ height: 24, padding: "0 8px", fontSize: 11 }}
                              title={`Jump to ${bankMatch.label}`}
                              onClick={() => selectAndInspectNode(bankMatch.node_id)}
                            >
                              ↗ Open
                            </button>
                          )}
                        </div>
                      </div>
                    </div>
                  );
                })}
              </div>
            </div>

            {/* WMB3 Shader Parameters (Auto-grouped _X/_Y/_Z/_W vectors + Color Swatches + 0.0..1.0 Sliders) */}
            <div>
              <div style={{ fontWeight: 600, color: "var(--brush-heading)", marginBottom: 8 }}>
                Shader Parameters ({namedParams.length} floats across {(wmb3MatDraft.parameter_groups || []).length} groups)
              </div>
              <table className="struct-table">
                <thead>
                  <tr>
                    <th style={{ width: 95 }}>Group / Idx</th>
                    <th style={{ width: 230 }}>Parameter Name</th>
                    <th>Value(s), Slider &amp; Color Swatch</th>
                  </tr>
                </thead>
                <tbody>
                  {(() => {
                    const paramGroups = Array.isArray(wmb3MatDraft.parameter_groups)
                      ? wmb3MatDraft.parameter_groups
                      : [];
                    const rows: React.ReactNode[] = [];

                    paramGroups.forEach((grpTuple, gi) => {
                      const floats: number[] = Array.isArray(grpTuple?.[1]) ? grpTuple[1] : [];
                      const getName = (pi: number) =>
                        namedParams.find((np) => np.group_index === gi && np.param_index === pi)
                          ?.name || `Group${gi}_${String(pi).padStart(2, "0")}`;

                      let pi = 0;
                      while (pi < floats.length) {
                        const rawName = getName(pi);
                        const matchVec = rawName.match(/^(.*?)_([XYZW])$/i);
                        if (matchVec && matchVec[2].toUpperCase() === "X") {
                          const baseName = matchVec[1];
                          const compIndices: number[] = [pi];
                          const compLabels: string[] = ["X"];
                          const expectedSuffixes = ["Y", "Z", "W"];
                          for (const suf of expectedSuffixes) {
                            const nextIdx = pi + compIndices.length;
                            if (
                              nextIdx < floats.length &&
                              getName(nextIdx).toLowerCase() === `${baseName}_${suf}`.toLowerCase()
                            ) {
                              compIndices.push(nextIdx);
                              compLabels.push(suf);
                            } else {
                              break;
                            }
                          }
                          if (compIndices.length >= 2) {
                            const startPi = pi;
                            const endPi = pi + compIndices.length - 1;
                            const vecVals = compIndices.map((idx) => floats[idx] ?? 0);
                            const isColor =
                              compIndices.length >= 3 && isColorLikeParamName(baseName);
                            const showUnitSlider = isUnitRangeScalarName(baseName);
                            rows.push(
                              <tr key={`g${gi}:v${startPi}`}>
                                <td className="mono" style={{ color: "var(--brush-muted)" }}>
                                  G{gi} [{startPi}..{endPi}]
                                </td>
                                <td className="mono" style={{ fontWeight: 600 }} title={baseName}>
                                  {baseName}
                                </td>
                                <td>
                                  <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
                                    {isColor && (
                                      <input
                                        type="color"
                                        value={rgbFloatToHex(vecVals)}
                                        onChange={(e) => {
                                          const [r, g, b] = hexToRgbFloat(e.target.value);
                                          const copy = structuredClone(wmb3MatDraft);
                                          copy.parameter_groups[gi][1][compIndices[0]] = r;
                                          copy.parameter_groups[gi][1][compIndices[1]] = g;
                                          copy.parameter_groups[gi][1][compIndices[2]] = b;
                                          setWmb3MatDraft(copy);
                                        }}
                                        style={{
                                          width: 26,
                                          height: 26,
                                          padding: 0,
                                          border: "1px solid var(--brush-border)",
                                          borderRadius: 4,
                                          background: "transparent",
                                          cursor: "pointer",
                                          flexShrink: 0,
                                        }}
                                        title="Pick RGB color"
                                      />
                                    )}
                                    <div
                                      style={{
                                        flex: 1,
                                        display: "grid",
                                        gridTemplateColumns: `repeat(${compIndices.length}, minmax(115px, 1fr))`,
                                        gap: 6,
                                      }}
                                    >
                                      {compIndices.map((cIdx, ci) => {
                                        const fv = floats[cIdx] ?? 0;
                                        return (
                                          <div
                                            key={cIdx}
                                            style={{ display: "flex", flexDirection: "column", gap: 2 }}
                                          >
                                            <span
                                              className="mono"
                                              style={{ fontSize: 10, color: "var(--brush-muted)" }}
                                            >
                                              {compLabels[ci]}
                                            </span>
                                            <div className="param-slider-group">
                                              {showUnitSlider && fv >= 0.0 && fv <= 1.0 && (
                                                <input
                                                  type="range"
                                                  min="0"
                                                  max="1"
                                                  step="0.01"
                                                  className="param-range"
                                                  value={fv}
                                                  onChange={(e) => {
                                                    const v = parseFloat(e.target.value);
                                                    const copy = structuredClone(wmb3MatDraft);
                                                    copy.parameter_groups[gi][1][cIdx] = v;
                                                    setWmb3MatDraft(copy);
                                                  }}
                                                />
                                              )}
                                              <input
                                                type="number"
                                                step="0.01"
                                                className="field-box mono"
                                                style={{ width: "100%", height: 25 }}
                                                value={Number(fv.toFixed(4))}
                                                onChange={(e) => {
                                                  const v = parseFloat(e.target.value);
                                                  if (!Number.isNaN(v)) {
                                                    const copy = structuredClone(wmb3MatDraft);
                                                    copy.parameter_groups[gi][1][cIdx] = v;
                                                    setWmb3MatDraft(copy);
                                                  }
                                                }}
                                              />
                                            </div>
                                          </div>
                                        );
                                      })}
                                    </div>
                                  </div>
                                </td>
                              </tr>
                            );
                            pi += compIndices.length;
                            continue;
                          }
                        }

                        // Standalone scalar parameter row
                        const curPi = pi;
                        const scalarVal = floats[curPi] ?? 0;
                        const useSlider =
                          isUnitRangeScalarName(rawName) &&
                          scalarVal >= 0.0 &&
                          scalarVal <= 1.0;
                        rows.push(
                          <tr key={`g${gi}:s${curPi}`}>
                            <td className="mono" style={{ color: "var(--brush-muted)" }}>
                              G{gi} [{curPi}]
                            </td>
                            <td className="mono" style={{ fontWeight: 600 }} title={rawName}>
                              {rawName}
                            </td>
                            <td>
                              <div className="param-slider-group" style={{ maxWidth: 280 }}>
                                {useSlider && (
                                  <input
                                    type="range"
                                    min="0"
                                    max="1"
                                    step="0.01"
                                    className="param-range"
                                    style={{ maxWidth: 120 }}
                                    value={scalarVal}
                                    onChange={(e) => {
                                      const v = parseFloat(e.target.value);
                                      const copy = structuredClone(wmb3MatDraft);
                                      copy.parameter_groups[gi][1][curPi] = v;
                                      setWmb3MatDraft(copy);
                                    }}
                                  />
                                )}
                                <input
                                  type="number"
                                  step="0.01"
                                  className="field-box mono"
                                  style={{ width: 130, height: 25 }}
                                  value={Number(scalarVal.toFixed(4))}
                                  onChange={(e) => {
                                    const v = parseFloat(e.target.value);
                                    if (!Number.isNaN(v)) {
                                      const copy = structuredClone(wmb3MatDraft);
                                      copy.parameter_groups[gi][1][curPi] = v;
                                      setWmb3MatDraft(copy);
                                    }
                                  }}
                                />
                              </div>
                            </td>
                          </tr>
                        );
                        pi += 1;
                      }
                    });
                    return rows;
                  })()}
                </tbody>
              </table>
            </div>

            {/* WMB3 Scalar Variables */}
            {wmb3MatDraft.variables.length > 0 && (
              <div>
                <div style={{ fontWeight: 600, color: "var(--brush-heading)", marginBottom: 8 }}>
                  Scalar Material Variables ({wmb3MatDraft.variables.length})
                </div>
                <table className="struct-table">
                  <thead>
                    <tr>
                      <th>Variable Name</th>
                      <th>Float Value</th>
                    </tr>
                  </thead>
                  <tbody>
                    {wmb3MatDraft.variables.map(([vName, vVal], vi) => (
                      <tr key={vi}>
                        <td className="mono" style={{ fontWeight: 600 }}>
                          {vName}
                        </td>
                        <td>
                          <input
                            type="number"
                            step="0.01"
                            className="field-box mono"
                            style={{ width: 150, height: 25 }}
                            value={Number((vVal ?? 0).toFixed(4))}
                            onChange={(e) => {
                              const v = parseFloat(e.target.value);
                              if (!Number.isNaN(v)) {
                                const copy = structuredClone(wmb3MatDraft);
                                copy.variables[vi][1] = v;
                                setWmb3MatDraft(copy);
                              }
                            }}
                          />
                        </td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            )}
          </div>
        );
      }

      // 1B. WMB1 / WMB2 Material Editor (Bayonetta 1 & Bayonetta 2)
      if (wmbMatDraft) {
        const fields = Array.isArray(wmbMatDraft.fields) ? wmbMatDraft.fields : [];
        const rawDwords = Array.isArray(wmbMatDraft.raw_dwords) ? wmbMatDraft.raw_dwords : [];
        const effectiveB2 = wmbB2Override ?? Boolean(wmbMatDraft.is_bayo2 ?? wmb.is_bayo2);
        const samplerFields = fields
          .map((f, idx) => ({ f, idx }))
          .filter((item) => item.f.sampler_tex_id !== null && item.f.sampler_tex_id !== undefined);
        const vectorFields = fields
          .map((f, idx) => ({ f, idx }))
          .filter((item) => item.f.float_values !== null && item.f.float_values !== undefined);

        const currentShaderVal = effectiveB2
          ? wmbMatDraft.shader_name || ""
          : String(wmbMatDraft.mat_type ?? 0);

        return (
          <div style={{ display: "flex", flexDirection: "column", gap: 16 }}>
            {/* Top Material Action Bar: B1/B2 Mode Toggle, Typed/Raw Toggle, Shader Dropdown, Copy/Paste/Convert */}
            <div
              style={{
                display: "flex",
                flexWrap: "wrap",
                justifyContent: "space-between",
                alignItems: "center",
                gap: 10,
                padding: "10px 12px",
                borderRadius: 8,
                backgroundColor: "var(--brush-card)",
                border: "1px solid var(--brush-border)",
              }}
            >
              <div style={{ display: "flex", alignItems: "center", gap: 8, flexWrap: "wrap" }}>
                {/* B1 vs B2 Game Mode Toggle (`WMB0`) */}
                <button
                  className={`pill-button ${!effectiveB2 ? "accent" : ""}`}
                  style={{ height: 27, padding: "0 9px", fontSize: 11.5 }}
                  onClick={() => {
                    setWmbB2Override(false);
                    selectAndInspectNode(wmb.node_id, mipIndex, channelMode, opaquePreview, false);
                  }}
                  title="Interpret material using Bayonetta 1 layout database (0x24..0xBC)"
                >
                  Bayonetta 1
                </button>
                <button
                  className={`pill-button ${effectiveB2 ? "accent" : ""}`}
                  style={{ height: 27, padding: "0 9px", fontSize: 11.5 }}
                  onClick={() => {
                    setWmbB2Override(true);
                    selectAndInspectNode(wmb.node_id, mipIndex, channelMode, opaquePreview, true);
                  }}
                  title="Interpret material using Bayonetta 2 shader database (843 shaders)"
                >
                  Bayonetta 2
                </button>

                <div className="toolbar-spacer" />

                {/* Typed View vs Raw Vec4 Words Toggle */}
                <button
                  className={`pill-button ${!wmbRawMode ? "accent" : ""}`}
                  style={{ height: 27, padding: "0 9px", fontSize: 11.5 }}
                  onClick={() => setWmbRawMode(false)}
                >
                  Typed View
                </button>
                <button
                  className={`pill-button ${wmbRawMode ? "accent" : ""}`}
                  style={{ height: 27, padding: "0 9px", fontSize: 11.5 }}
                  onClick={() => setWmbRawMode(true)}
                >
                  Raw Words ({rawDwords.length})
                </button>

                <div className="toolbar-spacer" />

                {/* Shader / Material Type Dropdown */}
                <span style={{ fontWeight: 600, color: "var(--brush-heading)" }}>
                  {effectiveB2 ? "Shader:" : "Mat Type:"}
                </span>
                <select
                  className="pill-select mono"
                  style={{ minWidth: 230, height: 28 }}
                  value={currentShaderVal}
                  onChange={(e) => {
                    const targetShader = e.target.value;
                    runWmbMutation(wmb.node_id, "switch_shader", {
                      index: wmbMatDraft.index,
                      shader: targetShader,
                      is_bayo2: effectiveB2,
                    });
                  }}
                >
                  <option value={currentShaderVal}>
                    {wmbMatDraft.shader_name
                      ? `${wmbMatDraft.shader_name} (0x${wmbMatDraft.mat_type.toString(16).toUpperCase()})`
                      : `Type ${wmbMatDraft.mat_type ?? 0} (0x${(wmbMatDraft.mat_type ?? 0).toString(16).toUpperCase().padStart(2, "0")})`}
                  </option>
                  {availableShaders
                    .filter((s) => (s.key ?? s.id) !== currentShaderVal)
                    .map((s) => {
                      const shKey = s.key ?? s.id ?? s.label;
                      return (
                        <option key={shKey} value={shKey}>
                          {s.label}
                        </option>
                      );
                    })}
                </select>
              </div>

              <div style={{ display: "flex", alignItems: "center", gap: 6, flexWrap: "wrap" }}>
                <button
                  className="pill-button"
                  style={{ height: 27 }}
                  onClick={() => {
                    setMaterialClipboard({
                      game: effectiveB2 ? "b2" : "b1",
                      payload: structuredClone(wmbMatDraft),
                    });
                    setStatusLeft(`Copied Material #${wmbMatDraft.index} to clipboard`);
                  }}
                >
                  Copy
                </button>
                <button
                  className="pill-button"
                  style={{ height: 27 }}
                  disabled={!materialClipboard}
                  onClick={async () => {
                    if (!materialClipboard) return;
                    const dstGame = effectiveB2 ? "b2" : "b1";
                    if (materialClipboard.game === dstGame) {
                      const nextMat = {
                        ...materialClipboard.payload,
                        index: wmbMatDraft.index,
                      };
                      setWmbMatDraft(nextMat);
                      await runWmbMutation(wmb.node_id, "update_material", nextMat);
                    } else {
                      const res = await invoke<{ converted: any }>("convert_material_action", {
                        srcGame: materialClipboard.game,
                        dstGame,
                        materialPayload: materialClipboard.payload,
                        targetShader: null,
                      });
                      const nextMat = {
                        ...res.converted,
                        index: wmbMatDraft.index,
                      };
                      setWmbMatDraft(nextMat);
                      await runWmbMutation(wmb.node_id, "update_material", nextMat);
                    }
                  }}
                >
                  Paste
                </button>
                <button
                  className="pill-button"
                  style={{ height: 27 }}
                  onClick={() =>
                    handleCrossGameMaterialConvert(wmb, effectiveB2 ? "b1" : "b2")
                  }
                  title={`Convert Material to ${effectiveB2 ? "Bayonetta 1" : "Bayonetta 2"}`}
                >
                  {effectiveB2 ? "→ B1" : "→ B2"}
                </button>
                <button
                  className="pill-button"
                  style={{ height: 27 }}
                  onClick={() => handleCrossGameMaterialConvert(wmb, "b3")}
                  title="Convert Material to Bayonetta 3 (WMB3) and copy to clipboard"
                >
                  → B3
                </button>
                <button
                  className="pill-button accent"
                  style={{ height: 27 }}
                  onClick={() => runWmbMutation(wmb.node_id, "update_material", wmbMatDraft)}
                >
                  Apply Material Changes
                </button>
              </div>
            </div>

            {/* Shader Family & Decoded Flags Banner */}
            {(wmbMatDraft.shader_family ||
              (wmbMatDraft.shader_flags_notes && wmbMatDraft.shader_flags_notes.length > 0)) && (
              <div
                style={{
                  display: "flex",
                  flexWrap: "wrap",
                  gap: 8,
                  alignItems: "center",
                  padding: "6px 10px",
                  borderRadius: 7,
                  backgroundColor: "var(--brush-control)",
                  border: "1px solid var(--brush-border)",
                  fontSize: 11.5,
                }}
              >
                {wmbMatDraft.shader_family && (
                  <span className="mono" style={{ fontWeight: 600, color: "var(--brush-accent)" }}>
                    Family: {wmbMatDraft.shader_family}
                  </span>
                )}
                {wmbMatDraft.shader_flags_notes?.map((note, ni) => (
                  <span key={ni} className="tree-badge">
                    {note}
                  </span>
                ))}
              </div>
            )}

            {/* Typed View: Texture Sampler Cards + Vector Parameters Table */}
            {!wmbRawMode && fields.length > 0 && (
              <>
                {samplerFields.length > 0 && (
                  <div>
                    <div style={{ fontWeight: 600, color: "var(--brush-heading)", marginBottom: 8 }}>
                      Texture Samplers ({samplerFields.length})
                    </div>
                    <div className="mat-sampler-grid">
                      {samplerFields.map(({ f: param, idx: pIdx }) => {
                        const texVal = (param.sampler_tex_id ?? 0) >>> 0;
                        const { bankMatch, pretexMatch } = resolveSamplerMatches(
                          texVal,
                          !effectiveB2,
                          bankTextures,
                          corePretextures
                        );
                        return (
                          <div key={pIdx} className="mat-sampler-card">
                            <div className="mat-sampler-thumb">
                              {bankMatch?.thumbnail_data_url ? (
                                <img src={bankMatch.thumbnail_data_url} alt={param.name} />
                              ) : (
                                <span
                                  className="mono"
                                  style={{ fontSize: 10, color: "var(--brush-muted)" }}
                                >
                                  {pretexMatch || param.pretexture_name ? "PRE" : "TEX"}
                                </span>
                              )}
                            </div>

                            <div
                              style={{
                                flex: 1,
                                minWidth: 0,
                                display: "flex",
                                flexDirection: "column",
                                gap: 4,
                              }}
                            >
                              <div
                                style={{
                                  display: "flex",
                                  justifyContent: "space-between",
                                  alignItems: "center",
                                  gap: 6,
                                }}
                              >
                                <span className="mono" style={{ fontWeight: 600, fontSize: 12 }}>
                                  {param.name}
                                </span>
                                <span style={{ color: "var(--brush-muted)", fontSize: 11 }}>
                                  {param.role || param.param_type}
                                </span>
                              </div>

                              {(pretexMatch || param.pretexture_name) && (
                                <div
                                  className="mono"
                                  style={{ fontSize: 10.5, color: "var(--brush-accent)" }}
                                >
                                  Core Pretexture: {pretexMatch?.name || param.pretexture_name}
                                </div>
                              )}

                              {/* Bank & Core Pretexture Custom Popover with Live Thumbnails */}
                              <TextureSlotPickerPopover
                                value={texVal}
                                isB1SlotMode={!effectiveB2}
                                bankTextures={bankTextures}
                                corePretextures={corePretextures}
                                onChange={(nextVal) => {
                                  const copy = structuredClone(wmbMatDraft);
                                  copy.fields[pIdx].sampler_tex_id = nextVal >>> 0;
                                  setWmbMatDraft(copy);
                                }}
                              />

                              <div style={{ display: "flex", alignItems: "center", gap: 6 }}>
                                <input
                                  type="text"
                                  className="field-box mono"
                                  style={{ flex: 1, height: 24, fontSize: 11.5 }}
                                  value={
                                    effectiveB2 || texVal > 255
                                      ? `0x${texVal.toString(16).toUpperCase().padStart(8, "0")}`
                                      : String(texVal)
                                  }
                                  onChange={(e) => {
                                    const rawStr = e.target.value.trim();
                                    const parsed = rawStr.toLowerCase().startsWith("0x")
                                      ? parseInt(rawStr.slice(2) || "0", 16)
                                      : parseInt(rawStr || "0", 10);
                                    if (!Number.isNaN(parsed)) {
                                      const copy = structuredClone(wmbMatDraft);
                                      copy.fields[pIdx].sampler_tex_id = parsed >>> 0;
                                      setWmbMatDraft(copy);
                                    }
                                  }}
                                />
                                {bankMatch && (
                                  <button
                                    className="pill-button"
                                    style={{ height: 24, padding: "0 8px", fontSize: 11 }}
                                    title={`Open ${bankMatch.label} in Texture Viewer`}
                                    onClick={() => selectAndInspectNode(bankMatch.node_id)}
                                  >
                                    ↗ Open
                                  </button>
                                )}
                              </div>
                            </div>
                          </div>
                        );
                      })}
                    </div>
                  </div>
                )}

                {vectorFields.length > 0 && (
                  <div>
                    <div style={{ fontWeight: 600, color: "var(--brush-heading)", marginBottom: 8 }}>
                      Shader Vector &amp; Color Parameters ({vectorFields.length})
                    </div>
                    <table className="struct-table">
                      <thead>
                        <tr>
                          <th style={{ width: 230 }}>Parameter Name</th>
                          <th>Components (X, Y, Z, W), Sliders &amp; Color Swatch</th>
                        </tr>
                      </thead>
                      <tbody>
                        {vectorFields.map(({ f: param, idx: pIdx }) => {
                          const flts = param.float_values || [0, 0, 0, 0];
                          const isColor = isColorLikeParamName(param.name, param.is_color);
                          const showSlider = isColor || isUnitRangeScalarName(param.name);
                          const labels = param.component_labels || ["X", "Y", "Z", "W"];
                          return (
                            <tr key={pIdx}>
                              <td
                                className="mono"
                                style={{ fontWeight: 600 }}
                                title={param.description || param.role || param.param_type}
                              >
                                {param.name}
                              </td>
                              <td>
                                <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
                                  {isColor && (
                                    <input
                                      type="color"
                                      value={rgbFloatToHex(flts)}
                                      onChange={(e) => {
                                        const [r, g, b] = hexToRgbFloat(e.target.value);
                                        const copy = structuredClone(wmbMatDraft);
                                        if (copy.fields[pIdx].float_values) {
                                          copy.fields[pIdx].float_values![0] = r;
                                          copy.fields[pIdx].float_values![1] = g;
                                          copy.fields[pIdx].float_values![2] = b;
                                          setWmbMatDraft(copy);
                                        }
                                      }}
                                      style={{
                                        width: 26,
                                        height: 26,
                                        padding: 0,
                                        border: "1px solid var(--brush-border)",
                                        borderRadius: 4,
                                        background: "transparent",
                                        cursor: "pointer",
                                        flexShrink: 0,
                                      }}
                                      title="Pick RGB color"
                                    />
                                  )}
                                  <div
                                    style={{
                                      flex: 1,
                                      display: "grid",
                                      gridTemplateColumns: "repeat(4, minmax(125px, 1fr))",
                                      gap: 6,
                                    }}
                                  >
                                    {flts.map((fv, fi) => (
                                      <div
                                        key={fi}
                                        style={{ display: "flex", flexDirection: "column", gap: 2 }}
                                      >
                                        <span
                                          className="mono"
                                          style={{ fontSize: 10, color: "var(--brush-muted)" }}
                                        >
                                          {labels[fi] || `C${fi}`}
                                        </span>
                                        <div className="param-slider-group">
                                          {showSlider && (fv ?? 0) >= 0.0 && (fv ?? 0) <= 1.0 && (
                                            <input
                                              type="range"
                                              min="0"
                                              max="1"
                                              step="0.01"
                                              className="param-range"
                                              value={fv ?? 0}
                                              onChange={(e) => {
                                                const val = parseFloat(e.target.value);
                                                const copy = structuredClone(wmbMatDraft);
                                                if (copy.fields[pIdx].float_values) {
                                                  copy.fields[pIdx].float_values![fi] = val;
                                                  setWmbMatDraft(copy);
                                                }
                                              }}
                                            />
                                          )}
                                          <input
                                            type="number"
                                            step="0.01"
                                            className="field-box mono"
                                            style={{ width: "100%", height: 25 }}
                                            value={Number((fv ?? 0).toFixed(4))}
                                            onChange={(e) => {
                                              const val = parseFloat(e.target.value);
                                              if (!Number.isNaN(val)) {
                                                const copy = structuredClone(wmbMatDraft);
                                                if (copy.fields[pIdx].float_values) {
                                                  copy.fields[pIdx].float_values![fi] = val;
                                                  setWmbMatDraft(copy);
                                                }
                                              }
                                            }}
                                          />
                                        </div>
                                      </div>
                                    ))}
                                  </div>
                                </div>
                              </td>
                            </tr>
                          );
                        })}
                      </tbody>
                    </table>
                  </div>
                )}
              </>
            )}

            {/* Raw Parameter Words View (shown when Raw Words toggle is active OR when shader has no typed fields) */}
            {(wmbRawMode || fields.length === 0) && (
              <div>
                <div
                  style={{
                    display: "flex",
                    justifyContent: "space-between",
                    alignItems: "center",
                    marginBottom: 8,
                  }}
                >
                  <span style={{ fontWeight: 600, color: "var(--brush-heading)" }}>
                    Raw Material Parameter Words ({rawDwords.length} × u32 / f32)
                  </span>
                </div>
                <table className="struct-table">
                  <thead>
                    <tr>
                      <th style={{ width: 90 }}>Word #</th>
                      <th style={{ width: 110 }}>Byte Offset</th>
                      <th>Hex Value (u32)</th>
                      <th>Float Value (f32)</th>
                    </tr>
                  </thead>
                  <tbody>
                    {rawDwords.map((dw, di) => {
                      const buf = new ArrayBuffer(4);
                      const view = new DataView(buf);
                      view.setUint32(0, dw >>> 0, true);
                      const fVal = view.getFloat32(0, true);
                      return (
                        <tr key={di}>
                          <td className="mono">#{di}</td>
                          <td className="mono" style={{ color: "var(--brush-muted)" }}>
                            +0x{(4 + di * 4).toString(16).toUpperCase().padStart(2, "0")}
                          </td>
                          <td>
                            <input
                              type="text"
                              className="field-box mono"
                              style={{ width: 140, height: 25 }}
                              value={`0x${(dw >>> 0).toString(16).toUpperCase().padStart(8, "0")}`}
                              onChange={(e) => {
                                const clean = e.target.value.replace(/^0x/i, "");
                                const parsed = parseInt(clean || "0", 16);
                                if (!Number.isNaN(parsed)) {
                                  const copy = structuredClone(wmbMatDraft);
                                  copy.raw_dwords[di] = parsed >>> 0;
                                  copy.fields = [];
                                  setWmbMatDraft(copy);
                                }
                              }}
                            />
                          </td>
                          <td>
                            <input
                              type="number"
                              step="0.01"
                              className="field-box mono"
                              style={{ width: 140, height: 25 }}
                              value={
                                Number.isFinite(fVal) && Math.abs(fVal) < 1e7
                                  ? Number(fVal.toFixed(5))
                                  : 0
                              }
                              onChange={(e) => {
                                const fv = parseFloat(e.target.value);
                                if (!Number.isNaN(fv)) {
                                  const b2 = new ArrayBuffer(4);
                                  const v2 = new DataView(b2);
                                  v2.setFloat32(0, fv, true);
                                  const copy = structuredClone(wmbMatDraft);
                                  copy.raw_dwords[di] = v2.getUint32(0, true);
                                  copy.fields = [];
                                  setWmbMatDraft(copy);
                                }
                              }}
                            />
                          </td>
                        </tr>
                      );
                    })}
                  </tbody>
                </table>
              </div>
            )}
          </div>
        );
      }
    }

    // 2. Materials Table Summary
    if (wmb.section === "materials" && Array.isArray(wmb.data)) {
      const parentBase = wmb.node_id.replace(/:materials$/, "");
      return (
        <table className="struct-table">
          <thead>
            <tr>
              <th style={{ width: 70 }}>Mat #</th>
              <th>Shader / Material Name</th>
              <th>Type / Technique</th>
              <th>Parameters</th>
              <th style={{ width: 110 }}>Action</th>
            </tr>
          </thead>
          <tbody>
            {(wmb.data as any[]).map((m, mi) => (
              <tr key={mi}>
                <td className="mono">#{mi}</td>
                <td className="mono" style={{ fontWeight: 600 }}>
                  {m.name || m.shader_name || `Material_${mi}`}
                </td>
                <td className="mono" style={{ color: "var(--brush-muted)" }}>
                  {m.technique_name
                    ? `${m.shader_name} (${m.technique_name})`
                    : m.mat_type !== undefined
                    ? `0x${Number(m.mat_type).toString(16).toUpperCase().padStart(2, "0")}${
                        m.shader_family ? ` · ${m.shader_family}` : ""
                      }`
                    : "—"}
                </td>
                <td className="mono">
                  {Array.isArray(m.fields)
                    ? `${m.fields.length} params (${(m.raw_dwords?.length ?? 0) * 4 + 4} B)`
                    : Array.isArray(m.textures)
                    ? `${m.textures.length} tex, ${
                        Array.isArray(m.parameter_groups)
                          ? m.parameter_groups.reduce((acc: number, g: any[]) => acc + (Array.isArray(g?.[1]) ? g[1].length : 0), 0)
                          : Array.isArray(m.parameters)
                          ? m.parameters.reduce((acc: number, g: any[]) => acc + (g?.length ?? 0), 0)
                          : 0
                      } floats`
                    : "—"}
                </td>
                <td>
                  <button
                    className="pill-button"
                    style={{ height: 24, padding: "0 10px", fontSize: 11.5 }}
                    onClick={() => selectAndInspectNode(`${parentBase}:mat:${mi}`)}
                  >
                    Edit Material
                  </button>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      );
    }

    // 3. Bones Table (Editable for both WMB1/2 and WMB3!)
    if (wmb.section === "bones") {
      const boneList: any[] = Array.isArray(wmb.data?.bones)
        ? wmb.data.bones
        : Array.isArray(wmb.data)
        ? wmb.data
        : [];
      return (
        <div style={{ display: "flex", flexDirection: "column", gap: 10 }}>
          {wmb.is_wmb3 && (
            <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center" }}>
              <span style={{ fontWeight: 600, color: "var(--brush-heading)" }}>
                WMB3 Skeleton ({boneList.length} bones)
              </span>
              <button
                className="pill-button accent"
                onClick={() => runWmbMutation(wmb.node_id, "set_tpose")}
              >
                Bake Skeleton &amp; Vertices to T-Pose
              </button>
            </div>
          )}
          <table className="struct-table">
            <thead>
              <tr>
                <th style={{ width: 65 }}>Bone #</th>
                <th style={{ width: 95 }}>Global ID</th>
                <th style={{ width: 95 }}>Parent #</th>
                <th>World Position (X, Y, Z)</th>
                <th>Relative / Local Offset (X, Y, Z)</th>
              </tr>
            </thead>
            <tbody>
              {boneList.map((b, bi) => {
                const pos = Array.isArray(b.position)
                  ? b.position
                  : b.position
                  ? [b.position.x, b.position.y, b.position.z]
                  : b.world_position
                  ? [b.world_position.x, b.world_position.y, b.world_position.z]
                  : [0, 0, 0];
                const rel = Array.isArray(b.relative_position)
                  ? b.relative_position
                  : b.relative_position
                  ? [b.relative_position.x, b.relative_position.y, b.relative_position.z]
                  : b.local_position
                  ? [b.local_position.x, b.local_position.y, b.local_position.z]
                  : [0, 0, 0];
                return (
                  <tr key={bi}>
                    <td className="mono">#{b.index ?? bi}</td>
                    <td className="mono" style={{ fontWeight: 600 }}>
                      {b.global_id ?? b.id ?? bi}
                    </td>
                    <td>
                      <input
                        type="number"
                        className="field-box mono"
                        style={{ width: 85, height: 24 }}
                        defaultValue={b.parent_index}
                        onBlur={(e) => {
                          const pVal = parseInt(e.target.value, 10);
                          if (!Number.isNaN(pVal) && pVal !== b.parent_index) {
                            if (pVal < -1 || pVal >= boneList.length || pVal === bi) {
                              setStatusLeft(
                                `Invalid parent bone #${pVal} for bone #${bi} (must be -1..${boneList.length - 1} and not self)`
                              );
                              e.target.value = String(b.parent_index);
                              return;
                            }
                            let cur = pVal;
                            const seen = new Set<number>([bi]);
                            let hasCycle = false;
                            while (cur >= 0 && cur < boneList.length) {
                              if (seen.has(cur)) {
                                hasCycle = true;
                                break;
                              }
                              seen.add(cur);
                              cur = Number(boneList[cur]?.parent_index ?? -1);
                            }
                            if (hasCycle) {
                              setStatusLeft(
                                `Cannot reparent bone #${bi} to descendant #${pVal} (cycle detected)`
                              );
                              e.target.value = String(b.parent_index);
                              return;
                            }
                            runWmbMutation(wmb.node_id, "update_bone", {
                              index: bi,
                              parent_index: pVal,
                            });
                          }
                        }}
                      />
                    </td>
                    <td className="mono">
                      {Number(pos[0] ?? 0).toFixed(3)}, {Number(pos[1] ?? 0).toFixed(3)},{" "}
                      {Number(pos[2] ?? 0).toFixed(3)}
                    </td>
                    <td className="mono">
                      {Number(rel[0] ?? 0).toFixed(3)}, {Number(rel[1] ?? 0).toFixed(3)},{" "}
                      {Number(rel[2] ?? 0).toFixed(3)}
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        </div>
      );
    }

    // 4. Interactive Meshes -> Batches (-> LODs) Tree with Drag-and-Drop & Custom Material Picker
    if (wmb.section === "meshes" || wmb.section.startsWith("mesh:")) {
      const matOptions: MaterialPickerOption[] = Array.isArray(wmb.material_options)
        ? wmb.material_options
        : [];

      // 4A. WMB3 Meshes -> Batches -> LODs Tree
      if (wmb.is_wmb3 && wmb.data) {
        const meshes3: any[] = Array.isArray(wmb.data.meshes) ? wmb.data.meshes : [];
        const batches3: any[] = Array.isArray(wmb.data.batches) ? wmb.data.batches : [];
        const lods3: any[] = Array.isArray(wmb.data.lods) ? wmb.data.lods : [];

        // Build per-batch metadata & LOD sub-rows using `inf.mesh_index` (NOT `mesh_group_index`!)
        interface Wmb3BatchResolved {
          batchIdx: number;
          batch: any;
          meshIndex: number;
          materialIndex: number;
          lodRows: {
            lodIndex: number;
            lodName: string;
            lodLevel: number;
            meshMatIndex: number;
            materialIndex: number;
            meshIndex: number;
            vertexGroupIndex: number;
            callTreeNodeIndex: number;
            uB: number;
          }[];
        }

        const resolvedBatches: Wmb3BatchResolved[] = batches3.map((b, bi) => {
          let primaryMeshIdx = 0;
          let primaryMatIdx = 0;
          let foundPrimary = false;
          const lodRows: Wmb3BatchResolved["lodRows"] = [];

          lods3.forEach((lod, li) => {
            const start = lod.batch_start ?? 0;
            const infos: any[] = Array.isArray(lod.batch_infos) ? lod.batch_infos : [];
            if (bi >= start && bi < start + infos.length) {
              const inf = infos[bi - start];
              const mIdx = inf?.mesh_index ?? inf?.mesh_group_index ?? 0;
              const matIdx = inf?.material_index ?? 0;
              if (!foundPrimary) {
                primaryMeshIdx = mIdx;
                primaryMatIdx = matIdx;
                foundPrimary = true;
              }
              lodRows.push({
                lodIndex: li,
                lodName: lod.name || `lod${li}`,
                lodLevel: lod.lod_level ?? li,
                meshMatIndex: inf?.mesh_material_index ?? 0,
                materialIndex: matIdx,
                meshIndex: mIdx,
                vertexGroupIndex: inf?.vertex_group_index ?? b.vertex_group_index ?? 0,
                callTreeNodeIndex: inf?.call_tree_node_index ?? -1,
                uB: inf?.u_b ?? 0,
              });
            }
          });

          return {
            batchIdx: bi,
            batch: b,
            meshIndex: primaryMeshIdx,
            materialIndex: primaryMatIdx,
            lodRows,
          };
        });

        const meshGroupCount = Math.max(
          meshes3.length,
          resolvedBatches.reduce((acc, r) => Math.max(acc, r.meshIndex + 1), 1)
        );

        return (
          <div style={{ display: "flex", flexDirection: "column", gap: 10 }}>
            <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", marginBottom: 4 }}>
              <div>
                <div style={{ fontWeight: 600, color: "var(--brush-heading)" }}>
                  WMB3 Mesh Groups ({meshes3.length}) → Batches ({batches3.length}) → LODs ({lods3.length})
                </div>
                <div style={{ color: "var(--brush-muted)", fontSize: 11.5 }}>
                  Drag any Batch row onto another Mesh Group to reassign it, or click ⚙ Header / LODs to edit all batch header fields.
                </div>
              </div>
            </div>

            {Array.from({ length: meshGroupCount }).map((_, mi) => {
              const mg = meshes3[mi] || { name: `MeshGroup_${mi}`, materials: [], bone_indices: [] };
              const groupBatches = resolvedBatches.filter((rb) => rb.meshIndex === mi);
              const mgKey = `wmb3_mg_${mi}`;
              const isExpanded = expandedMeshGroups[mgKey] ?? true;
              const isDropTarget = dragOverMeshIdx === mi && draggedBatchInfo?.isWmb3;

              return (
                <div
                  key={mi}
                  className={`mesh-tree-card ${isDropTarget ? "drag-over-target" : ""}`}
                  onDragOver={(e) => {
                    if (draggedBatchInfo?.isWmb3) {
                      e.preventDefault();
                      e.dataTransfer.dropEffect = "move";
                      if (dragOverMeshIdx !== mi) setDragOverMeshIdx(mi);
                    }
                  }}
                  onDragLeave={() => {
                    if (dragOverMeshIdx === mi) setDragOverMeshIdx(null);
                  }}
                  onDrop={(e) => {
                    e.preventDefault();
                    setDragOverMeshIdx(null);
                    if (draggedBatchInfo?.isWmb3 && draggedBatchInfo.fromMeshIdx !== mi) {
                      runWmbMutation(wmb.node_id, "update_wmb3_batch", {
                        batch_index: draggedBatchInfo.batchIdx,
                        mesh_index: mi,
                      });
                    }
                    setDraggedBatchInfo(null);
                  }}
                >
                  <div
                    className="mesh-tree-header"
                    onClick={() =>
                      setExpandedMeshGroups((p) => ({ ...p, [mgKey]: !isExpanded }))
                    }
                  >
                    <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
                      <span className={`tree-arrow ${isExpanded ? "expanded" : ""}`}>
                        <svg width="6" height="9" viewBox="0 0 6 10" fill="none" stroke="currentColor" strokeWidth="1.5">
                          <path d="M1 1L5 5L1 9" strokeLinecap="round" strokeLinejoin="round" />
                        </svg>
                      </span>
                      <span className="tree-badge">MESH #{mi}</span>
                      <span style={{ fontWeight: 600, color: "var(--brush-heading)" }}>
                        {mg.name || `MeshGroup_${mi}`}
                      </span>
                      <span className="mono" style={{ color: "var(--brush-muted)", fontSize: 11.5 }}>
                        ({groupBatches.length} batches · mats: [{(mg.materials || []).join(", ")}])
                      </span>
                    </div>
                    <span className="mono" style={{ color: "var(--brush-muted)", fontSize: 11 }}>
                      {Array.isArray(mg.bone_indices) ? `${mg.bone_indices.length} bones` : ""}
                    </span>
                  </div>

                  {isExpanded && (
                    <div>
                      {groupBatches.length === 0 ? (
                        <div style={{ padding: "10px 24px", color: "var(--brush-muted)", fontSize: 12 }}>
                          (No batches assigned to Mesh Group #{mi} — drag a batch here to assign)
                        </div>
                      ) : (
                        groupBatches.map((rb) => {
                          const lodKey = `wmb3_b_${rb.batchIdx}`;
                          const hdrKey = `wmb3_hdr_${rb.batchIdx}`;
                          const lodExpanded = expandedBatchLods[lodKey] ?? false;
                          const hdrExpanded = expandedBatchLods[hdrKey] ?? false;
                          const hasShadowLod = rb.lodRows.some(
                            (lr) => lr.lodLevel === -1 || String(lr.lodName).toUpperCase().includes("SHADOW")
                          );
                          return (
                            <div key={rb.batchIdx}>
                              <div
                                className={`mesh-batch-row ${
                                  draggedBatchInfo?.batchIdx === rb.batchIdx ? "dragging-batch" : ""
                                }`}
                                draggable
                                onDragStart={(e) => {
                                  e.dataTransfer.effectAllowed = "move";
                                  e.dataTransfer.setData("text/plain", String(rb.batchIdx));
                                  setDraggedBatchInfo({
                                    isWmb3: true,
                                    batchIdx: rb.batchIdx,
                                    fromMeshIdx: mi,
                                  });
                                }}
                                onDragEnd={() => {
                                  setDraggedBatchInfo(null);
                                  setDragOverMeshIdx(null);
                                }}
                              >
                                <div style={{ display: "flex", alignItems: "center", gap: 8, flexWrap: "wrap" }}>
                                  <span
                                    style={{ cursor: "grab", color: "var(--brush-muted)", fontWeight: 700 }}
                                    title="Drag to another Mesh Group"
                                  >
                                    ⋮⋮
                                  </span>
                                  <button
                                    type="button"
                                    className="pill-button"
                                    style={{ height: 24, padding: "0 7px", fontSize: 11 }}
                                    onClick={() =>
                                      setExpandedBatchLods((p) => ({ ...p, [lodKey]: !lodExpanded }))
                                    }
                                    title="Toggle LOD sub-rows"
                                  >
                                    <span>{lodExpanded ? "▾" : "▸"}</span>
                                    <span className="mono">Batch #{rb.batchIdx}</span>
                                    <span style={{ color: "var(--brush-muted)" }}>
                                      ({rb.lodRows.length} LOD{rb.lodRows.length === 1 ? "" : "s"})
                                    </span>
                                  </button>

                                  {hasShadowLod && (
                                    <span
                                      className="tree-badge"
                                      style={{ background: "rgba(180, 130, 255, 0.16)", color: "#c49bff" }}
                                      title="Includes -1 SHADOW LOD in WMB3"
                                    >
                                      SHADOW LOD
                                    </span>
                                  )}

                                  <MaterialPickerPopover
                                    value={rb.materialIndex}
                                    options={matOptions}
                                    onChange={(nextMat) =>
                                      runWmbMutation(wmb.node_id, "update_wmb3_batch", {
                                        batch_index: rb.batchIdx,
                                        material_index: nextMat,
                                      })
                                    }
                                    onNavigateMaterial={(matNodeId) => selectAndInspectNode(matNodeId)}
                                  />
                                </div>

                                <div style={{ display: "flex", alignItems: "center", gap: 8, flexWrap: "wrap" }}>
                                  <label style={{ display: "inline-flex", alignItems: "center", gap: 4, fontSize: 11.5 }}>
                                    <span style={{ color: "var(--brush-muted)" }}>Group:</span>
                                    <select
                                      className="pill-select mono"
                                      style={{ height: 26, fontSize: 11.5, paddingRight: 24 }}
                                      value={rb.meshIndex}
                                      onChange={(e) => {
                                        const nextMg = parseInt(e.target.value, 10);
                                        if (!Number.isNaN(nextMg) && nextMg !== rb.meshIndex) {
                                          runWmbMutation(wmb.node_id, "update_wmb3_batch", {
                                            batch_index: rb.batchIdx,
                                            mesh_index: nextMg,
                                          });
                                        }
                                      }}
                                    >
                                      {Array.from({ length: meshGroupCount }).map((__, mgi) => (
                                        <option key={mgi} value={mgi}>
                                          #{mgi} {meshes3[mgi]?.name || `Group_${mgi}`}
                                        </option>
                                      ))}
                                    </select>
                                  </label>

                                  <label style={{ display: "inline-flex", alignItems: "center", gap: 4, fontSize: 11.5 }}>
                                    <span style={{ color: "var(--brush-muted)" }}>VtxGrp:</span>
                                    <input
                                      type="number"
                                      className="field-box mono"
                                      style={{ width: 54, height: 24 }}
                                      defaultValue={rb.batch.vertex_group_index ?? 0}
                                      onBlur={(e) => {
                                        const vgi = parseInt(e.target.value, 10);
                                        if (!Number.isNaN(vgi) && vgi !== rb.batch.vertex_group_index) {
                                          runWmbMutation(wmb.node_id, "update_wmb3_batch", {
                                            batch_index: rb.batchIdx,
                                            vertex_group_index: vgi,
                                          });
                                        }
                                      }}
                                    />
                                  </label>

                                  <label style={{ display: "inline-flex", alignItems: "center", gap: 4, fontSize: 11.5 }}>
                                    <span style={{ color: "var(--brush-muted)" }}>BoneSet:</span>
                                    <input
                                      type="number"
                                      className="field-box mono"
                                      style={{ width: 54, height: 24 }}
                                      defaultValue={rb.batch.bone_set_index ?? -1}
                                      onBlur={(e) => {
                                        const bsi = parseInt(e.target.value, 10);
                                        if (!Number.isNaN(bsi) && bsi !== rb.batch.bone_set_index) {
                                          runWmbMutation(wmb.node_id, "update_wmb3_batch", {
                                            batch_index: rb.batchIdx,
                                            bone_set_index: bsi,
                                          });
                                        }
                                      }}
                                    />
                                  </label>

                                  <button
                                    type="button"
                                    className={`pill-button ${hdrExpanded ? "accent" : ""}`}
                                    style={{ height: 24, padding: "0 8px", fontSize: 11 }}
                                    onClick={() =>
                                      setExpandedBatchLods((p) => ({ ...p, [hdrKey]: !hdrExpanded }))
                                    }
                                    title="Edit all WMB3 Batch Header fields"
                                  >
                                    ⚙ Header
                                  </button>

                                  <span className="mono" style={{ color: "var(--brush-muted)", fontSize: 11 }}>
                                    {rb.batch.num_primitives} tris ({rb.batch.num_indices} idx)
                                  </span>
                                </div>
                              </div>

                              {/* Expandable WMB3 Batch Header Editor */}
                              {hdrExpanded && (
                                <div
                                  className="mesh-lod-subrow"
                                  style={{
                                    display: "grid",
                                    gridTemplateColumns: "repeat(auto-fill, minmax(150px, 1fr))",
                                    gap: 8,
                                    padding: "8px 16px 8px 36px",
                                    background: "var(--brush-panel)",
                                  }}
                                >
                                  {(
                                    [
                                      ["vertex_group_index", "vertex_group_index (u32)"],
                                      ["bone_set_index", "bone_set_index (i32)"],
                                      ["vertex_start", "vertex_start (u32)"],
                                      ["index_start", "index_start (u32)"],
                                      ["num_vertexes", "num_vertexes (u32)"],
                                      ["num_indices", "num_indices (u32)"],
                                      ["num_primitives", "num_primitives (u32)"],
                                    ] as [string, string][]
                                  ).map(([fKey, fLabel]) => (
                                    <label
                                      key={fKey}
                                      style={{ display: "flex", flexDirection: "column", gap: 3, fontSize: 11 }}
                                    >
                                      <span className="mono" style={{ color: "var(--brush-muted)" }}>
                                        {fLabel}
                                      </span>
                                      <input
                                        type="number"
                                        className="field-box mono"
                                        style={{ height: 24 }}
                                        defaultValue={rb.batch[fKey] ?? 0}
                                        onBlur={(e) => {
                                          const val = parseInt(e.target.value, 10);
                                          if (!Number.isNaN(val) && val !== rb.batch[fKey]) {
                                            runWmbMutation(wmb.node_id, "update_wmb3_batch", {
                                              batch_index: rb.batchIdx,
                                              [fKey]: val,
                                            });
                                          }
                                        }}
                                      />
                                    </label>
                                  ))}
                                </div>
                              )}

                              {/* Non-draggable LOD sub-rows under this Batch (with editable LOD BatchInfo fields) */}
                              {lodExpanded &&
                                rb.lodRows.map((lr) => (
                                  <div
                                    key={lr.lodIndex}
                                    className="mesh-lod-subrow"
                                    style={{ display: "flex", alignItems: "center", gap: 10, flexWrap: "wrap" }}
                                  >
                                    <span className="tree-badge">LOD #{lr.lodLevel}</span>
                                    <span className="mono" style={{ fontWeight: 600, color: "var(--brush-text)" }}>
                                      {lr.lodName}
                                    </span>
                                    <label style={{ display: "inline-flex", alignItems: "center", gap: 4, fontSize: 11 }}>
                                      <span className="mono" style={{ color: "var(--brush-muted)" }}>mesh_idx:</span>
                                      <input
                                        type="number"
                                        className="field-box mono"
                                        style={{ width: 52, height: 22 }}
                                        defaultValue={lr.meshIndex}
                                        onBlur={(e) => {
                                          const v = parseInt(e.target.value, 10);
                                          if (!Number.isNaN(v) && v !== lr.meshIndex) {
                                            runWmbMutation(wmb.node_id, "update_wmb3_batch", {
                                              batch_index: rb.batchIdx,
                                              lod_index: lr.lodIndex,
                                              mesh_index: v,
                                            });
                                          }
                                        }}
                                      />
                                    </label>
                                    <label style={{ display: "inline-flex", alignItems: "center", gap: 4, fontSize: 11 }}>
                                      <span className="mono" style={{ color: "var(--brush-muted)" }}>mat_idx:</span>
                                      <input
                                        type="number"
                                        className="field-box mono"
                                        style={{ width: 52, height: 22 }}
                                        defaultValue={lr.materialIndex}
                                        onBlur={(e) => {
                                          const v = parseInt(e.target.value, 10);
                                          if (!Number.isNaN(v) && v !== lr.materialIndex) {
                                            runWmbMutation(wmb.node_id, "update_wmb3_batch", {
                                              batch_index: rb.batchIdx,
                                              lod_index: lr.lodIndex,
                                              material_index: v,
                                            });
                                          }
                                        }}
                                      />
                                    </label>
                                    <label style={{ display: "inline-flex", alignItems: "center", gap: 4, fontSize: 11 }}>
                                      <span className="mono" style={{ color: "var(--brush-muted)" }}>mesh_mat_idx:</span>
                                      <input
                                        type="number"
                                        className="field-box mono"
                                        style={{ width: 52, height: 22 }}
                                        defaultValue={lr.meshMatIndex}
                                        onBlur={(e) => {
                                          const v = parseInt(e.target.value, 10);
                                          if (!Number.isNaN(v) && v !== lr.meshMatIndex) {
                                            runWmbMutation(wmb.node_id, "update_wmb3_batch", {
                                              batch_index: rb.batchIdx,
                                              lod_index: lr.lodIndex,
                                              mesh_material_index: v,
                                            });
                                          }
                                        }}
                                      />
                                    </label>
                                    <label style={{ display: "inline-flex", alignItems: "center", gap: 4, fontSize: 11 }}>
                                      <span className="mono" style={{ color: "var(--brush-muted)" }}>col_tree_idx:</span>
                                      <input
                                        type="number"
                                        className="field-box mono"
                                        style={{ width: 52, height: 22 }}
                                        defaultValue={lr.callTreeNodeIndex}
                                        onBlur={(e) => {
                                          const v = parseInt(e.target.value, 10);
                                          if (!Number.isNaN(v) && v !== lr.callTreeNodeIndex) {
                                            runWmbMutation(wmb.node_id, "update_wmb3_batch", {
                                              batch_index: rb.batchIdx,
                                              lod_index: lr.lodIndex,
                                              call_tree_node_index: v,
                                            });
                                          }
                                        }}
                                      />
                                    </label>
                                    <label style={{ display: "inline-flex", alignItems: "center", gap: 4, fontSize: 11 }}>
                                      <span className="mono" style={{ color: "var(--brush-muted)" }}>u_b:</span>
                                      <input
                                        type="number"
                                        className="field-box mono"
                                        style={{ width: 52, height: 22 }}
                                        defaultValue={lr.uB}
                                        onBlur={(e) => {
                                          const v = parseInt(e.target.value, 10);
                                          if (!Number.isNaN(v) && v !== lr.uB) {
                                            runWmbMutation(wmb.node_id, "update_wmb3_batch", {
                                              batch_index: rb.batchIdx,
                                              lod_index: lr.lodIndex,
                                              u_b: v,
                                            });
                                          }
                                        }}
                                      />
                                    </label>
                                  </div>
                                ))}
                            </div>
                          );
                        })
                      )}
                    </div>
                  )}
                </div>
              );
            })}
          </div>
        );
      }

      // 4B. WMB0 (Bayonetta 1 / Bayonetta 2) Meshes -> Batches Tree
      const isB2BatchMode = wmbB2Override !== null ? wmbB2Override : wmb.is_bayo2;
      const getWmb0EffectiveMatId = (hdr: any): number => {
        if (!hdr) return 0;
        if (isB2BatchMode) {
          return hdr.ex_mat_id !== undefined && hdr.ex_mat_id >= 0
            ? hdr.ex_mat_id
            : hdr.material_id ?? 0;
        }
        return hdr.material_id ?? 0;
      };
      const isWmb0BatchShadowProxy = (hdr: any): boolean => {
        if (!hdr) return false;
        return isB2BatchMode ? hdr.u_e1 === 48 : hdr.u_e1 === 32 && hdr.u_e2 === 15;
      };

      if (wmb.section.includes(":batch:") && wmb.data?.batch) {
        const b = wmb.data.batch;
        const hdr = b.header || {};
        const mIdx = wmb.data.mesh_index ?? 0;
        const bIdx = wmb.data.batch_index ?? 0;
        const curMatId = getWmb0EffectiveMatId(hdr);
        const isShadow = isWmb0BatchShadowProxy(hdr);
        return (
          <div style={{ display: "flex", flexDirection: "column", gap: 14 }}>
            <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", flexWrap: "wrap", gap: 8 }}>
              <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
                <span style={{ fontWeight: 600, color: "var(--brush-heading)" }}>
                  Mesh #{mIdx} ({wmb.data.mesh_name}) → Batch #{bIdx}
                </span>
                {isShadow && (
                  <span
                    className="tree-badge"
                    style={{ background: "rgba(180, 130, 255, 0.18)", color: "#c49bff" }}
                  >
                    SHADOW PROXY
                  </span>
                )}
              </div>
              <div style={{ display: "flex", gap: 6 }}>
                <button
                  type="button"
                  className={`pill-button ${isShadow ? "accent" : ""}`}
                  onClick={() =>
                    runWmbMutation(wmb.node_id, "update_batch", {
                      mesh_index: mIdx,
                      batch_index: bIdx,
                      shadow_proxy: !isShadow,
                    })
                  }
                  title={
                    isB2BatchMode
                      ? "Toggle Bayonetta 2 Shadow Proxy (u_e1 = 48 / 0x30)"
                      : "Toggle Bayonetta 1 Shadow Proxy (u_e1 = 32, u_e2 = 15)"
                  }
                >
                  ☁ {isShadow ? "Shadow Proxy: ON" : "Shadow Proxy: OFF"}
                </button>
                <button
                  type="button"
                  className="pill-button"
                  onClick={() =>
                    runWmbMutation(wmb.node_id, "duplicate_wmb_batch", {
                      mesh_index: mIdx,
                      batch_index: bIdx,
                    })
                  }
                >
                  ⧉ Duplicate Batch
                </button>
              </div>
            </div>
            <table className="struct-table">
              <thead>
                <tr>
                  <th>Batch Header Field</th>
                  <th>Value (Editable)</th>
                </tr>
              </thead>
              <tbody>
                <tr>
                  <td className="mono">Assigned Material ({isB2BatchMode ? "Bayo 2 ex_mat_id" : "Bayo 1 material_id"})</td>
                  <td>
                    <MaterialPickerPopover
                      value={curMatId}
                      options={matOptions}
                      onChange={(nextMat) =>
                        runWmbMutation(wmb.node_id, "update_batch", {
                          mesh_index: mIdx,
                          batch_index: bIdx,
                          material_id: nextMat,
                        })
                      }
                      onNavigateMaterial={(matNodeId) => selectAndInspectNode(matNodeId)}
                    />
                  </td>
                </tr>
                <tr>
                  <td className="mono">Shadow Caster Proxy (u_e1 / u_e2)</td>
                  <td>
                    <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
                      <button
                        type="button"
                        className={`pill-button ${isShadow ? "accent" : ""}`}
                        style={{ height: 24, padding: "0 10px", fontSize: 11.5 }}
                        onClick={() =>
                          runWmbMutation(wmb.node_id, "update_batch", {
                            mesh_index: mIdx,
                            batch_index: bIdx,
                            shadow_proxy: !isShadow,
                          })
                        }
                      >
                        {isShadow ? "✓ Enabled (Shadow Proxy)" : "Disabled (Visible Mesh)"}
                      </button>
                      <span className="mono" style={{ fontSize: 11, color: "var(--brush-muted)" }}>
                        u_e1={hdr.u_e1 ?? 0}, u_e2={hdr.u_e2 ?? 0}
                      </span>
                    </div>
                  </td>
                </tr>
                {(
                  [
                    ["batch_id", "batch_id (i16)"],
                    ["mesh_id", "mesh_id (i16)"],
                    ["ex_mat_id", "ex_mat_id (i16 — Bayo 2 Material ID)"],
                    ["raw_material_id", "material_id (u8 — Bayo 1 Material ID)", hdr.material_id ?? 0],
                    ["has_bone_refs", "has_bone_refs (u8)"],
                    ["u_e1", "u_e1 (u8 — Shadow LOD Flag 1)"],
                    ["u_e2", "u_e2 (u8 — Shadow LOD Flag 2)"],
                    ["vertex_start", "vertex_start (u32)"],
                    ["vertex_end", "vertex_end (u32)"],
                    ["primitive_type", "primitive_type (4=Triangle List, 5=Strip)"],
                    ["offset_indices", "offset_indices (u32)"],
                    ["num_indices", "num_indices (i32)"],
                    ["vertex_offset", "vertex_offset (i32)"],
                  ] as [string, string, number?][]
                ).map(([fKey, fLabel, overrideVal]) => {
                  const curVal = overrideVal !== undefined ? overrideVal : hdr[fKey] ?? 0;
                  return (
                    <tr key={`${mIdx}_${bIdx}_${fKey}_${curVal}`}>
                      <td className="mono">{fLabel}</td>
                      <td>
                        <input
                          type="number"
                          size={1}
                          className="field-box mono"
                          style={{ height: 24, width: 120 }}
                          defaultValue={curVal}
                          onBlur={(e) => {
                            const parsed = parseInt(e.target.value, 10);
                            if (!Number.isNaN(parsed) && parsed !== curVal) {
                              runWmbMutation(wmb.node_id, "update_batch", {
                                mesh_index: mIdx,
                                batch_index: bIdx,
                                [fKey]: parsed,
                              });
                            }
                          }}
                        />
                      </td>
                    </tr>
                  );
                })}
                <tr key={`${mIdx}_${bIdx}_flags_${hdr.flags}`}>
                  <td className="mono">flags (u16 hex)</td>
                  <td>
                    <input
                      type="text"
                      size={1}
                      className="field-box mono"
                      style={{ height: 24, width: 120 }}
                      defaultValue={`0x${(hdr.flags ?? 0).toString(16).toUpperCase().padStart(4, "0")}`}
                      onBlur={(e) => {
                        runWmbMutation(wmb.node_id, "update_batch", {
                          mesh_index: mIdx,
                          batch_index: bIdx,
                          flags_hex: e.target.value,
                        });
                      }}
                    />
                  </td>
                </tr>
                <tr>
                  <td className="mono">u_f (7 × i32)</td>
                  <td>
                    <div style={{ display: "flex", gap: 4, flexWrap: "wrap" }}>
                      {(Array.isArray(hdr.u_f) ? hdr.u_f : [0, 0, 0, 0, 0, 0, 0]).map(
                        (uv: number, ui: number) => (
                          <input
                            key={`${ui}_${uv}`}
                            type="number"
                            size={1}
                            className="field-box mono"
                            style={{ width: 64, height: 24 }}
                            defaultValue={uv}
                            onBlur={(e) => {
                              const parsed = parseInt(e.target.value, 10);
                              if (!Number.isNaN(parsed) && parsed !== uv) {
                                const nextUf = [...(hdr.u_f || [0, 0, 0, 0, 0, 0, 0])];
                                nextUf[ui] = parsed;
                                runWmbMutation(wmb.node_id, "update_batch", {
                                  mesh_index: mIdx,
                                  batch_index: bIdx,
                                  u_f: nextUf,
                                });
                              }
                            }}
                          />
                        )
                      )}
                    </div>
                  </td>
                </tr>
                <tr key={`${mIdx}_${bIdx}_bonerefs_${(b.bone_refs || []).join(",")}`}>
                  <td className="mono">Bone Palette Refs ({b.bone_refs?.length ?? 0})</td>
                  <td>
                    <input
                      type="text"
                      size={1}
                      className="field-box mono"
                      style={{ height: 24, width: "100%", minWidth: 220 }}
                      defaultValue={Array.isArray(b.bone_refs) ? b.bone_refs.join(", ") : ""}
                      onBlur={(e) => {
                        const nums = e.target.value
                          .split(",")
                          .map((s) => parseInt(s.trim(), 10))
                          .filter((n) => !Number.isNaN(n) && n >= 0 && n <= 255);
                        runWmbMutation(wmb.node_id, "update_batch", {
                          mesh_index: mIdx,
                          batch_index: bIdx,
                          bone_refs: nums,
                        });
                      }}
                    />
                  </td>
                </tr>
              </tbody>
            </table>
          </div>
        );
      }

      const meshes: any[] = Array.isArray(wmb.data)
        ? wmb.data
        : Array.isArray(wmb.data?.meshes)
        ? wmb.data.meshes
        : wmb.data?.batches
        ? [wmb.data]
        : [];

      return (
        <div style={{ display: "flex", flexDirection: "column", gap: 10 }}>
          <div>
            <div style={{ fontWeight: 600, color: "var(--brush-heading)" }}>
              WMB0 Meshes ({meshes.length}) → Batches Tree ({isB2BatchMode ? "Bayonetta 2" : "Bayonetta 1"})
            </div>
            <div style={{ color: "var(--brush-muted)", fontSize: 11.5, marginBottom: 4 }}>
              Drag any Batch row onto another Mesh to move it, toggle ☁ Shadow Proxy on a Mesh or Batch, or click ⚙ Header to edit batch header fields.
            </div>
          </div>

          {meshes.map((mesh, mi) => {
            const mKey = `wmb0_m_${mi}`;
            const isExpanded = expandedMeshGroups[mKey] ?? true;
            const batches: any[] = Array.isArray(mesh.batches) ? mesh.batches : [];
            const isDropTarget = dragOverMeshIdx === mi && draggedBatchInfo && !draggedBatchInfo.isWmb3;
            const allShadowInMesh =
              batches.length > 0 && batches.every((b) => isWmb0BatchShadowProxy(b.header));
            const someShadowInMesh = batches.some((b) => isWmb0BatchShadowProxy(b.header));

            return (
              <div
                key={mi}
                className={`mesh-tree-card ${isDropTarget ? "drag-over-target" : ""}`}
                onDragOver={(e) => {
                  if (draggedBatchInfo && !draggedBatchInfo.isWmb3) {
                    e.preventDefault();
                    e.dataTransfer.dropEffect = "move";
                    if (dragOverMeshIdx !== mi) setDragOverMeshIdx(mi);
                  }
                }}
                onDragLeave={() => {
                  if (dragOverMeshIdx === mi) setDragOverMeshIdx(null);
                }}
                onDrop={(e) => {
                  e.preventDefault();
                  setDragOverMeshIdx(null);
                  if (
                    draggedBatchInfo &&
                    !draggedBatchInfo.isWmb3 &&
                    draggedBatchInfo.fromMeshIdx !== mi
                  ) {
                    runWmbMutation(wmb.node_id, "move_wmb_batch", {
                      from_mesh_index: draggedBatchInfo.fromMeshIdx,
                      from_batch_index: draggedBatchInfo.batchIdx,
                      to_mesh_index: mi,
                    });
                  }
                  setDraggedBatchInfo(null);
                }}
              >
                <div
                  className="mesh-tree-header"
                  onClick={() =>
                    setExpandedMeshGroups((p) => ({ ...p, [mKey]: !isExpanded }))
                  }
                >
                  <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
                    <span className={`tree-arrow ${isExpanded ? "expanded" : ""}`}>
                      <svg width="6" height="9" viewBox="0 0 6 10" fill="none" stroke="currentColor" strokeWidth="1.5">
                        <path d="M1 1L5 5L1 9" strokeLinecap="round" strokeLinejoin="round" />
                      </svg>
                    </span>
                    <span className="tree-badge">MESH #{mesh.id ?? mesh.header?.id ?? mi}</span>
                    <span style={{ fontWeight: 600, color: "var(--brush-heading)" }}>
                      {mesh.name || `Mesh_${mi}`}
                    </span>
                    <span className="mono" style={{ color: "var(--brush-muted)", fontSize: 11.5 }}>
                      ({batches.length} batches)
                    </span>
                    {someShadowInMesh && (
                      <span
                        className="tree-badge"
                        style={{ background: "rgba(180, 130, 255, 0.18)", color: "#c49bff" }}
                      >
                        {allShadowInMesh ? "SHADOW PROXY MESH" : "PARTIAL SHADOW"}
                      </span>
                    )}
                  </div>
                  <div
                    style={{ display: "flex", alignItems: "center", gap: 6 }}
                    onClick={(e) => e.stopPropagation()}
                  >
                    <button
                      type="button"
                      className={`pill-button ${allShadowInMesh ? "accent" : ""}`}
                      style={{ height: 24, padding: "0 8px", fontSize: 11 }}
                      title={
                        isB2BatchMode
                          ? "Toggle Shadow Proxy for all batches in this Mesh (Bayo 2: u_e1=48)"
                          : "Toggle Shadow Proxy for all batches in this Mesh (Bayo 1: u_e1=32, u_e2=15)"
                      }
                      onClick={() =>
                        runWmbMutation(wmb.node_id, "toggle_wmb_mesh_shadow_proxy", {
                          mesh_index: mi,
                          enabled: !allShadowInMesh,
                        })
                      }
                    >
                      ☁ {allShadowInMesh ? "Shadow Mesh: ON" : "Shadow Mesh"}
                    </button>
                  </div>
                </div>

                {isExpanded && (
                  <div>
                    {batches.map((b: any, bi: number) => {
                      const hdr = b.header || {};
                      const curMatId = getWmb0EffectiveMatId(hdr);
                      const isShadow = isWmb0BatchShadowProxy(hdr);
                      const hdrKey = `wmb0_hdr_${mi}_${bi}`;
                      const hdrExpanded = expandedBatchLods[hdrKey] ?? false;

                      return (
                        <div key={bi}>
                          <div
                            className={`mesh-batch-row ${
                              draggedBatchInfo &&
                              !draggedBatchInfo.isWmb3 &&
                              draggedBatchInfo.fromMeshIdx === mi &&
                              draggedBatchInfo.batchIdx === bi
                                ? "dragging-batch"
                                : ""
                            }`}
                            draggable
                            onDragStart={(e) => {
                              e.dataTransfer.effectAllowed = "move";
                              e.dataTransfer.setData("text/plain", `${mi}:${bi}`);
                              setDraggedBatchInfo({
                                isWmb3: false,
                                batchIdx: bi,
                                fromMeshIdx: mi,
                              });
                            }}
                            onDragEnd={() => {
                              setDraggedBatchInfo(null);
                              setDragOverMeshIdx(null);
                            }}
                          >
                            <div style={{ display: "flex", alignItems: "center", gap: 8, flexWrap: "wrap" }}>
                              <span
                                style={{ cursor: "grab", color: "var(--brush-muted)", fontWeight: 700 }}
                                title="Drag to move batch to another Mesh"
                              >
                                ⋮⋮
                              </span>
                              <span className="mono" style={{ fontWeight: 600 }}>
                                Batch #{bi}
                              </span>

                              {isShadow && (
                                <span
                                  className="tree-badge"
                                  style={{ background: "rgba(180, 130, 255, 0.18)", color: "#c49bff" }}
                                  title={`Shadow Proxy (u_e1=${hdr.u_e1}, u_e2=${hdr.u_e2})`}
                                >
                                  SHADOW
                                </span>
                              )}

                              <MaterialPickerPopover
                                value={curMatId}
                                options={matOptions}
                                onChange={(nextMat) =>
                                  runWmbMutation(wmb.node_id, "update_batch", {
                                    mesh_index: mi,
                                    batch_index: bi,
                                    material_id: nextMat,
                                  })
                                }
                                onNavigateMaterial={(matNodeId) => selectAndInspectNode(matNodeId)}
                              />
                            </div>

                            <div style={{ display: "flex", alignItems: "center", gap: 6, flexWrap: "wrap" }}>
                              {meshes.length > 1 && (
                                <label style={{ display: "inline-flex", alignItems: "center", gap: 4, fontSize: 11.5 }}>
                                  <span style={{ color: "var(--brush-muted)" }}>Mesh:</span>
                                  <select
                                    className="pill-select mono"
                                    style={{ height: 26, fontSize: 11.5, paddingRight: 24 }}
                                    value={mi}
                                    onChange={(e) => {
                                      const dstMesh = parseInt(e.target.value, 10);
                                      if (!Number.isNaN(dstMesh) && dstMesh !== mi) {
                                        runWmbMutation(wmb.node_id, "move_wmb_batch", {
                                          from_mesh_index: mi,
                                          from_batch_index: bi,
                                          to_mesh_index: dstMesh,
                                        });
                                      }
                                    }}
                                  >
                                    {meshes.map((mOpt, mOptIdx) => (
                                      <option key={mOptIdx} value={mOptIdx}>
                                        #{mOptIdx} {mOpt.name || `Mesh_${mOptIdx}`}
                                      </option>
                                    ))}
                                  </select>
                                </label>
                              )}

                              <button
                                type="button"
                                className={`pill-button ${isShadow ? "accent" : ""}`}
                                style={{ height: 24, padding: "0 7px", fontSize: 11 }}
                                title={
                                  isB2BatchMode
                                    ? `Shadow Proxy toggle (Bayo 2: u_e1=48, currently u_e1=${hdr.u_e1 ?? 0}, u_e2=${hdr.u_e2 ?? 0})`
                                    : `Shadow Proxy toggle (Bayo 1: u_e1=32, u_e2=15, currently u_e1=${hdr.u_e1 ?? 0}, u_e2=${hdr.u_e2 ?? 0})`
                                }
                                onClick={() =>
                                  runWmbMutation(wmb.node_id, "update_batch", {
                                    mesh_index: mi,
                                    batch_index: bi,
                                    shadow_proxy: !isShadow,
                                  })
                                }
                              >
                                ☁ Shadow
                              </button>

                              <button
                                type="button"
                                className={`pill-button ${hdrExpanded ? "accent" : ""}`}
                                style={{ height: 24, padding: "0 7px", fontSize: 11 }}
                                title="Edit WMB0 Batch Header fields (batch_id, flags, u_e1, u_e2, vertex range, bone_refs...)"
                                onClick={() =>
                                  setExpandedBatchLods((p) => ({ ...p, [hdrKey]: !hdrExpanded }))
                                }
                              >
                                ⚙ Header
                              </button>

                              <span className="mono" style={{ fontSize: 11, color: "var(--brush-muted)" }}>
                                vtx {hdr.vertex_start}..{hdr.vertex_end} ·{" "}
                                {b.indices?.length ?? hdr.num_indices ?? 0} idx ·{" "}
                                {hdr.primitive_type === 4 ? "Tris" : "Strip"}
                              </span>

                              <button
                                type="button"
                                className="pill-button"
                                style={{ height: 24, padding: "0 7px", fontSize: 11 }}
                                title="Duplicate Batch"
                                onClick={() =>
                                  runWmbMutation(wmb.node_id, "duplicate_wmb_batch", {
                                    mesh_index: mi,
                                    batch_index: bi,
                                  })
                                }
                              >
                                ⧉
                              </button>
                              <button
                                type="button"
                                className="pill-button"
                                style={{ height: 24, padding: "0 7px", fontSize: 11 }}
                                disabled={batches.length <= 1}
                                title="Delete Batch"
                                onClick={() =>
                                  runWmbMutation(wmb.node_id, "delete_wmb_batch", {
                                    mesh_index: mi,
                                    batch_index: bi,
                                  })
                                }
                              >
                                ✕
                              </button>
                            </div>
                          </div>

                          {/* Expandable WMB0 Batch Header Editor */}
                          {hdrExpanded && (
                            <div
                              className="mesh-lod-subrow"
                              style={{
                                display: "grid",
                                gridTemplateColumns: "repeat(auto-fill, minmax(145px, 1fr))",
                                gap: 8,
                                padding: "8px 16px 8px 36px",
                                background: "var(--brush-panel)",
                              }}
                            >
                              {(
                                [
                                  ["batch_id", "batch_id (i16)", hdr.batch_id ?? bi],
                                  ["mesh_id", "mesh_id (i16)", hdr.mesh_id ?? mi],
                                  ["ex_mat_id", "ex_mat_id (i16 B2)", hdr.ex_mat_id ?? 0],
                                  ["raw_material_id", "material_id (u8 B1)", hdr.material_id ?? 0],
                                  ["has_bone_refs", "has_bone_refs (u8)", hdr.has_bone_refs ?? 1],
                                  ["u_e1", "u_e1 (u8 Shadow 1)", hdr.u_e1 ?? 0],
                                  ["u_e2", "u_e2 (u8 Shadow 2)", hdr.u_e2 ?? 0],
                                  ["vertex_start", "vertex_start (u32)", hdr.vertex_start ?? 0],
                                  ["vertex_end", "vertex_end (u32)", hdr.vertex_end ?? 0],
                                  ["primitive_type", "primitive_type (4/5)", hdr.primitive_type ?? 4],
                                  ["offset_indices", "offset_indices (u32)", hdr.offset_indices ?? 0],
                                  ["num_indices", "num_indices (i32)", hdr.num_indices ?? 0],
                                  ["vertex_offset", "vertex_offset (i32)", hdr.vertex_offset ?? 0],
                                ] as [string, string, number][]
                              ).map(([fKey, fLabel, curVal]) => (
                                <label
                                  key={`${mi}_${bi}_${fKey}_${curVal}`}
                                  style={{ display: "flex", flexDirection: "column", gap: 3, fontSize: 11 }}
                                >
                                  <span className="mono" style={{ color: "var(--brush-muted)" }}>
                                    {fLabel}
                                  </span>
                                  <input
                                    type="number"
                                    size={1}
                                    className="field-box mono"
                                    style={{ height: 24 }}
                                    defaultValue={curVal}
                                    onBlur={(e) => {
                                      const parsed = parseInt(e.target.value, 10);
                                      if (!Number.isNaN(parsed) && parsed !== curVal) {
                                        runWmbMutation(wmb.node_id, "update_batch", {
                                          mesh_index: mi,
                                          batch_index: bi,
                                          [fKey]: parsed,
                                        });
                                      }
                                    }}
                                  />
                                </label>
                              ))}
                              <label
                                key={`${mi}_${bi}_flags_${hdr.flags}`}
                                style={{ display: "flex", flexDirection: "column", gap: 3, fontSize: 11 }}
                              >
                                <span className="mono" style={{ color: "var(--brush-muted)" }}>
                                  flags (u16 hex)
                                </span>
                                <input
                                  type="text"
                                  size={1}
                                  className="field-box mono"
                                  style={{ height: 24 }}
                                  defaultValue={`0x${(hdr.flags ?? 0).toString(16).toUpperCase().padStart(4, "0")}`}
                                  onBlur={(e) => {
                                    runWmbMutation(wmb.node_id, "update_batch", {
                                      mesh_index: mi,
                                      batch_index: bi,
                                      flags_hex: e.target.value,
                                    });
                                  }}
                                />
                              </label>
                            </div>
                          )}
                        </div>
                      );
                    })}
                  </div>
                )}
              </div>
            );
          })}
        </div>
      );
    }

    // 5. Vertexes Table
    if (wmb.section === "vertexes" && Array.isArray(wmb.data)) {
      return (
        <table className="struct-table">
          <thead>
            <tr>
              <th>#</th>
              <th>Position (X, Y, Z)</th>
              <th>UV (U, V)</th>
              <th>Normal (Nx, Ny, Nz)</th>
            </tr>
          </thead>
          <tbody>
            {(wmb.data as any[]).map((v, vi) => (
              <tr key={vi}>
                <td className="mono">{vi}</td>
                <td className="mono">
                  {v.position
                    ? `${v.position.x.toFixed(3)}, ${v.position.y.toFixed(3)}, ${v.position.z.toFixed(3)}`
                    : "—"}
                </td>
                <td className="mono">
                  {v.mapping ? `${v.mapping[0].toFixed(4)}, ${v.mapping[1].toFixed(4)}` : "—"}
                </td>
                <td className="mono">
                  {v.normal?.normal
                    ? `${v.normal.normal[0].toFixed(3)}, ${v.normal.normal[1].toFixed(3)}, ${v.normal.normal[2].toFixed(3)}`
                    : "—"}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      );
    }

    // 6. WMB / WMB3 Header View
    const hdr = wmb.data || {};
    return (
      <div style={{ display: "flex", flexDirection: "column", gap: 16 }}>
        <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center" }}>
          <span style={{ fontWeight: 600, color: "var(--brush-heading)" }}>
            Model Header &amp; Bounding Box
          </span>
          {!wmb.is_wmb3 && (
            <button
              className="pill-button accent"
              onClick={() =>
                runWmbMutation(wmb.node_id, "update_header", {
                  vertex_flags_hex: wmbHeaderFlagsDraft,
                  bounding_box: wmbBboxDraft,
                })
              }
            >
              <span>Apply Header Changes</span>
            </button>
          )}
        </div>
        <table className="struct-table">
          <thead>
            <tr>
              <th style={{ width: 220 }}>Header Property</th>
              <th>Value</th>
            </tr>
          </thead>
          <tbody>
            {!wmb.is_wmb3 && (
              <tr>
                <td className="mono" style={{ fontWeight: 600 }}>
                  Vertex Format Flags
                </td>
                <td>
                  <input
                    type="text"
                    className="field-box mono"
                    style={{ width: 160, height: 26 }}
                    value={wmbHeaderFlagsDraft}
                    onChange={(e) => setWmbHeaderFlagsDraft(e.target.value)}
                  />
                </td>
              </tr>
            )}
            <tr>
              <td className="mono" style={{ fontWeight: 600 }}>
                Bounding Box (Min / Max XYZ)
              </td>
              <td>
                <div style={{ display: "grid", gridTemplateColumns: "repeat(3, 1fr)", gap: 6 }}>
                  {wmbBboxDraft.map((bv, bi) => (
                    <input
                      key={bi}
                      type="number"
                      step="0.05"
                      className="field-box mono"
                      style={{ height: 26 }}
                      value={Number((bv ?? 0).toFixed(4))}
                      disabled={wmb.is_wmb3}
                      onChange={(e) => {
                        const v = parseFloat(e.target.value);
                        if (!Number.isNaN(v)) {
                          const next = [...wmbBboxDraft];
                          next[bi] = v;
                          setWmbBboxDraft(next);
                        }
                      }}
                    />
                  ))}
                </div>
              </td>
            </tr>
            {Object.entries(hdr)
              .filter(([k]) => k !== "bounding_box" && k !== "vertex_flags" && k !== "id")
              .map(([k, v]) => (
                <tr key={k}>
                  <td className="mono" style={{ color: "var(--brush-muted)" }}>
                    {k}
                  </td>
                  <td className="mono">
                    {typeof v === "number" && k.startsWith("offset_")
                      ? `0x${v.toString(16).toUpperCase()} (${v.toLocaleString()} B)`
                      : typeof v === "object"
                      ? JSON.stringify(v)
                      : String(v)}
                  </td>
                </tr>
              ))}
          </tbody>
        </table>
      </div>
    );
  }

  // Render clean structured tables for MOT, EXP, COS, ATT, OBI, MSQ, COL, FLS, XAD, LIT, MDB
  function renderRawStructCenter() {
    const kind = nodePayload?.struct_kind;
    if (!structDraft) return null;

    if (kind === "COS" && Array.isArray(structDraft.entries)) {
      return (
        <div style={{ display: "flex", flexDirection: "column", gap: 12 }}>
          <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center" }}>
            <span style={{ fontWeight: 600, color: "var(--brush-heading)" }}>
              Costume Color Palettes ({structDraft.entries.length} entries)
            </span>
            <button
              className="pill-button accent"
              onClick={() => runStructMutation(nodePayload.node_id, "COS", structDraft)}
            >
              <span>Commit COS Changes</span>
            </button>
          </div>
          <table className="struct-table">
            <thead>
              <tr>
                <th style={{ width: 55 }}>#</th>
                <th style={{ width: 180 }}>Costume Name</th>
                <th>7 RGB Swatches (Click to Edit)</th>
              </tr>
            </thead>
            <tbody>
              {structDraft.entries.map((e: any, ei: number) => (
                <tr key={ei}>
                  <td className="mono">#{ei}</td>
                  <td>
                    <input
                      type="text"
                      className="field-box mono"
                      style={{ width: "100%", height: 25 }}
                      value={e.name}
                      onChange={(ev) => {
                        const copy = structuredClone(structDraft);
                        copy.entries[ei].name = ev.target.value;
                        setStructDraft(copy);
                      }}
                    />
                  </td>
                  <td>
                    <div style={{ display: "flex", gap: 8, flexWrap: "wrap" }}>
                      {(e.colors as number[][]).map((rgb, ci) => (
                        <label
                          key={ci}
                          style={{ display: "flex", alignItems: "center", gap: 4, fontSize: 11 }}
                          className="mono"
                        >
                          <input
                            type="color"
                            value={rgbFloatToHex(rgb)}
                            onChange={(ev) => {
                              const [r, g, b] = hexToRgbFloat(ev.target.value);
                              const copy = structuredClone(structDraft);
                              copy.entries[ei].colors[ci] = [r, g, b];
                              setStructDraft(copy);
                            }}
                            style={{
                              width: 24,
                              height: 24,
                              padding: 0,
                              border: "1px solid var(--brush-border)",
                              borderRadius: 4,
                              cursor: "pointer",
                            }}
                          />
                          <span>C{ci}</span>
                        </label>
                      ))}
                    </div>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      );
    }

    if (kind === "ATT" && Array.isArray(structDraft.points)) {
      return (
        <div style={{ display: "flex", flexDirection: "column", gap: 12 }}>
          <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center" }}>
            <span style={{ fontWeight: 600, color: "var(--brush-heading)" }}>
              Bone Attachment Points ({structDraft.points.length})
            </span>
            <div style={{ display: "flex", gap: 8 }}>
              <button
                className="pill-button"
                onClick={() => {
                  const copy = structuredClone(structDraft);
                  copy.points.push({ source_bone: 0, target_bone: 0, option: 0 });
                  setStructDraft(copy);
                }}
              >
                + Add Point
              </button>
              <button
                className="pill-button accent"
                onClick={() => runStructMutation(nodePayload.node_id, "ATT", structDraft)}
              >
                Commit ATT Changes
              </button>
            </div>
          </div>
          <table className="struct-table">
            <thead>
              <tr>
                <th style={{ width: 65 }}>#</th>
                <th>Source Bone</th>
                <th>Target Bone</th>
                <th>Option Flag</th>
                <th style={{ width: 80 }}>Remove</th>
              </tr>
            </thead>
            <tbody>
              {structDraft.points.map((pt: any, pi: number) => (
                <tr key={pi}>
                  <td className="mono">#{pi}</td>
                  <td>
                    <EditableNumberCell
                      style={{ width: 110, height: 25 }}
                      value={pt.source_bone ?? 0}
                      onChangeValue={(v) => {
                        const copy = structuredClone(structDraft);
                        copy.points[pi].source_bone = v;
                        setStructDraft(copy);
                      }}
                    />
                  </td>
                  <td>
                    <EditableNumberCell
                      style={{ width: 110, height: 25 }}
                      value={pt.target_bone ?? 0}
                      onChangeValue={(v) => {
                        const copy = structuredClone(structDraft);
                        copy.points[pi].target_bone = v;
                        setStructDraft(copy);
                      }}
                    />
                  </td>
                  <td>
                    <EditableNumberCell
                      style={{ width: 110, height: 25 }}
                      value={pt.option ?? 0}
                      onChangeValue={(v) => {
                        const copy = structuredClone(structDraft);
                        copy.points[pi].option = v;
                        setStructDraft(copy);
                      }}
                    />
                  </td>
                  <td>
                    <button
                      className="pill-button"
                      style={{ height: 24, padding: "0 8px" }}
                      onClick={() => {
                        const copy = structuredClone(structDraft);
                        copy.points.splice(pi, 1);
                        setStructDraft(copy);
                      }}
                    >
                      ✕
                    </button>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      );
    }

    if (kind === "OBI" && Array.isArray(structDraft.entries)) {
      return (
        <div style={{ display: "flex", flexDirection: "column", gap: 12 }}>
          <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center" }}>
            <span style={{ fontWeight: 600, color: "var(--brush-heading)" }}>
              Object Placement Instances ({structDraft.entries.length})
            </span>
            <button
              className="pill-button accent"
              onClick={() => runStructMutation(nodePayload.node_id, "OBI", structDraft)}
            >
              Commit OBI Changes
            </button>
          </div>
          <table className="struct-table">
            <thead>
              <tr>
                <th>#</th>
                <th>Object ID</th>
                <th>Position (X, Y, Z)</th>
                <th>Rotation (X, Y, Z)</th>
                <th>Scale (X, Y, Z)</th>
                <th>Difficulty</th>
              </tr>
            </thead>
            <tbody>
              {structDraft.entries.map((e: any, ei: number) => (
                <tr key={ei}>
                  <td className="mono">#{ei}</td>
                  <td className="mono" style={{ fontWeight: 600 }}>
                    {e.object_name} (0x{Number(e.object_id).toString(16).toUpperCase()})
                  </td>
                  <td>
                    <div style={{ display: "grid", gridTemplateColumns: "repeat(3, 1fr)", gap: 4 }}>
                      {(["x", "y", "z"] as const).map((axis) => (
                        <EditableNumberCell
                          key={axis}
                          isFloat
                          decimals={3}
                          step="0.1"
                          style={{ height: 24 }}
                          value={e.position?.[axis] ?? 0}
                          onChangeValue={(v) => {
                            const copy = structuredClone(structDraft);
                            copy.entries[ei].position[axis] = v;
                            setStructDraft(copy);
                          }}
                        />
                      ))}
                    </div>
                  </td>
                  <td>
                    <div style={{ display: "grid", gridTemplateColumns: "repeat(3, 1fr)", gap: 4 }}>
                      {(["x", "y", "z"] as const).map((axis) => (
                        <EditableNumberCell
                          key={axis}
                          isFloat
                          decimals={3}
                          step="0.05"
                          style={{ height: 24 }}
                          value={e.orientation?.[axis] ?? 0}
                          onChangeValue={(v) => {
                            const copy = structuredClone(structDraft);
                            copy.entries[ei].orientation[axis] = v;
                            setStructDraft(copy);
                          }}
                        />
                      ))}
                    </div>
                  </td>
                  <td>
                    <div style={{ display: "grid", gridTemplateColumns: "repeat(3, 1fr)", gap: 4 }}>
                      {(["x", "y", "z"] as const).map((axis) => (
                        <EditableNumberCell
                          key={axis}
                          isFloat
                          decimals={3}
                          step="0.05"
                          style={{ height: 24 }}
                          value={e.scale?.[axis] ?? 1}
                          onChangeValue={(v) => {
                            const copy = structuredClone(structDraft);
                            copy.entries[ei].scale[axis] = v;
                            setStructDraft(copy);
                          }}
                        />
                      ))}
                    </div>
                  </td>
                  <td>
                    <input
                      type="number"
                      className="field-box mono"
                      style={{ width: 85, height: 24 }}
                      value={e.difficulty_selector}
                      onChange={(ev) => {
                        const copy = structuredClone(structDraft);
                        copy.entries[ei].difficulty_selector = parseInt(ev.target.value || "0", 10);
                        setStructDraft(copy);
                      }}
                    />
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      );
    }

    if (kind === "MSQ" && Array.isArray(structDraft.events)) {
      return (
        <div style={{ display: "flex", flexDirection: "column", gap: 12 }}>
          <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center" }}>
            <span style={{ fontWeight: 600, color: "var(--brush-heading)" }}>
              Subtitle / Sequence Events ({structDraft.events.length})
            </span>
            <button
              className="pill-button accent"
              onClick={() => runStructMutation(nodePayload.node_id, "MSQ", structDraft)}
            >
              Commit MSQ Changes
            </button>
          </div>
          <table className="struct-table">
            <thead>
              <tr>
                <th>#</th>
                <th>Event Name</th>
                <th>ID</th>
                <th>Event Type</th>
                <th>Line / Group</th>
                <th>Duration (Frames)</th>
              </tr>
            </thead>
            <tbody>
              {structDraft.events.map((ev: any, ei: number) => (
                <tr key={ei}>
                  <td className="mono">#{ei}</td>
                  <td>
                    <input
                      type="text"
                      className="field-box mono"
                      style={{ width: "100%", height: 25 }}
                      value={ev.name}
                      onChange={(e) => {
                        const copy = structuredClone(structDraft);
                        copy.events[ei].name = e.target.value;
                        setStructDraft(copy);
                      }}
                    />
                  </td>
                  <td>
                    <input
                      type="number"
                      className="field-box mono"
                      style={{ width: 95, height: 25 }}
                      value={ev.id}
                      onChange={(e) => {
                        const copy = structuredClone(structDraft);
                        copy.events[ei].id = parseInt(e.target.value || "0", 10);
                        setStructDraft(copy);
                      }}
                    />
                  </td>
                  <td>
                    <input
                      type="number"
                      className="field-box mono"
                      style={{ width: 95, height: 25 }}
                      value={ev.event_type}
                      onChange={(e) => {
                        const copy = structuredClone(structDraft);
                        copy.events[ei].event_type = parseInt(e.target.value || "0", 10);
                        setStructDraft(copy);
                      }}
                    />
                  </td>
                  <td>
                    <input
                      type="number"
                      className="field-box mono"
                      style={{ width: 95, height: 25 }}
                      value={ev.line_number_per_group}
                      onChange={(e) => {
                        const copy = structuredClone(structDraft);
                        copy.events[ei].line_number_per_group = parseInt(e.target.value || "0", 10);
                        setStructDraft(copy);
                      }}
                    />
                  </td>
                  <td>
                    <input
                      type="number"
                      className="field-box mono"
                      style={{ width: 95, height: 25 }}
                      value={ev.event_duration}
                      onChange={(e) => {
                        const copy = structuredClone(structDraft);
                        copy.events[ei].event_duration = parseInt(e.target.value || "0", 10);
                        setStructDraft(copy);
                      }}
                    />
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      );
    }

    if (kind === "MOT" && Array.isArray(structDraft.records)) {
      const records: any[] = structDraft.records;
      const selTrack = records[selectedMotTrackIdx] ?? null;
      return (
        <div style={{ display: "flex", flexDirection: "column", gap: 14 }}>
          <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center" }}>
            <div style={{ display: "flex", alignItems: "center", gap: 12 }}>
              <span style={{ fontWeight: 600, color: "var(--brush-heading)" }}>
                Motion Header — Frame Count:
              </span>
              <input
                type="number"
                className="field-box mono"
                style={{ width: 95, height: 26 }}
                value={structDraft.frame_count ?? 0}
                onChange={(e) => {
                  const copy = structuredClone(structDraft);
                  copy.frame_count = parseInt(e.target.value || "0", 10);
                  setStructDraft(copy);
                }}
              />
            </div>
            <button
              className="pill-button accent"
              onClick={() => runStructMutation(nodePayload.node_id, "MOT", structDraft)}
            >
              Commit MOT Changes
            </button>
          </div>
          <div style={{ display: "grid", gridTemplateColumns: "1.2fr 1fr", gap: 14 }}>
            <table className="struct-table">
              <thead>
                <tr>
                  <th>#</th>
                  <th>Bone Index</th>
                  <th>Channel</th>
                  <th>Mode</th>
                  <th>Value / Keys</th>
                </tr>
              </thead>
              <tbody>
                {records.map((r, ri) => (
                  <tr
                    key={ri}
                    className={`file-row ${selectedMotTrackIdx === ri ? "selected" : ""}`}
                    onClick={() => setSelectedMotTrackIdx(ri)}
                  >
                    <td className="mono">#{ri}</td>
                    <td>
                      <input
                        type="number"
                        className="field-box mono"
                        style={{ width: 85, height: 23 }}
                        value={r.bone_index}
                        onClick={(e) => e.stopPropagation()}
                        onChange={(e) => {
                          const copy = structuredClone(structDraft);
                          copy.records[ri].bone_index = parseInt(e.target.value || "0", 10);
                          setStructDraft(copy);
                        }}
                      />
                    </td>
                    <td className="mono">{r.value_index}</td>
                    <td className="mono">{r.flag}</td>
                    <td>
                      {r.flag === 0 || !r.interpolation ? (
                        <input
                          type="number"
                          step="0.01"
                          className="field-box mono"
                          style={{ width: 105, height: 23 }}
                          value={Number((r.value ?? 0).toFixed(5))}
                          onClick={(e) => e.stopPropagation()}
                          onChange={(e) => {
                            const copy = structuredClone(structDraft);
                            copy.records[ri].value = parseFloat(e.target.value || "0");
                            setStructDraft(copy);
                          }}
                        />
                      ) : (
                        <span className="mono" style={{ color: "var(--brush-accent)" }}>
                          {r.elem_number} keyframes →
                        </span>
                      )}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>

            <div>
              <div style={{ fontWeight: 600, color: "var(--brush-heading)", marginBottom: 6 }}>
                {selTrack
                  ? `Track #${selectedMotTrackIdx} (Bone ${selTrack.bone_index}, Ch ${selTrack.value_index})`
                  : "Select a track"}
              </div>
              {selTrack?.interpolation ? (
                <table className="struct-table">
                  <thead>
                    <tr>
                      <th>Key #</th>
                      <th>Value / Quantized</th>
                    </tr>
                  </thead>
                  <tbody>
                    {(() => {
                      const interp = selTrack.interpolation;
                      const variantKey = Object.keys(interp)[0];
                      const payload = interp[variantKey];
                      const keys: any[] = payload?.keys ?? (Array.isArray(payload) ? payload : []);
                      return keys.map((k: any, ki: number) => (
                        <tr key={ki}>
                          <td className="mono">
                            {k.index !== undefined ? `Frame ${k.index}` : `#${ki}`}
                          </td>
                          <td className="mono">
                            {typeof k === "number"
                              ? k.toFixed(5)
                              : k.p !== undefined
                              ? `p=${Number(k.p).toFixed(4)} (m0=${k.m0}, m1=${k.m1})`
                              : JSON.stringify(k)}
                          </td>
                        </tr>
                      ));
                    })()}
                  </tbody>
                </table>
              ) : (
                <div style={{ color: "var(--brush-muted)", padding: 12 }}>
                  Constant value track ({selTrack?.value ?? 0})
                </div>
              )}
            </div>
          </div>
        </div>
      );
    }

    // COL (Editable Collision Groups)
    if (kind === "COL" && Array.isArray(structDraft.groups)) {
      return (
        <div style={{ display: "flex", flexDirection: "column", gap: 12 }}>
          <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center" }}>
            <span style={{ fontWeight: 600, color: "var(--brush-heading)" }}>
              Collision Groups ({structDraft.groups.length}) · {structDraft.num_vertices} vertices
            </span>
            <button
              className="pill-button accent"
              onClick={() => runStructMutation(nodePayload.node_id, "COL", structDraft)}
            >
              Commit COL Changes
            </button>
          </div>
          <table className="struct-table">
            <thead>
              <tr>
                <th>Group #</th>
                <th>Collision Group Name</th>
                <th>Flags 1 (Hex)</th>
                <th>Flags 2 (Hex)</th>
              </tr>
            </thead>
            <tbody>
              {structDraft.groups.map((g: any, gi: number) => (
                <tr key={gi}>
                  <td className="mono">#{g.index ?? gi}</td>
                  <td>
                    <input
                      type="text"
                      className="field-box mono"
                      style={{ height: 24, fontWeight: 600 }}
                      value={g.name}
                      onChange={(e) => {
                        const copy = structuredClone(structDraft);
                        copy.groups[gi].name = e.target.value;
                        setStructDraft(copy);
                      }}
                    />
                  </td>
                  <td>
                    <input
                      type="text"
                      className="field-box mono"
                      style={{ height: 24 }}
                      value={`0x${Number(g.flags >>> 0).toString(16).toUpperCase().padStart(8, "0")}`}
                      onChange={(e) => {
                        const clean = e.target.value.replace(/^0x/i, "");
                        const parsed = parseInt(clean || "0", 16);
                        if (!Number.isNaN(parsed)) {
                          const copy = structuredClone(structDraft);
                          copy.groups[gi].flags = parsed >>> 0;
                          setStructDraft(copy);
                        }
                      }}
                    />
                  </td>
                  <td>
                    <input
                      type="text"
                      className="field-box mono"
                      style={{ height: 24 }}
                      value={`0x${Number(g.flags2 >>> 0).toString(16).toUpperCase().padStart(8, "0")}`}
                      onChange={(e) => {
                        const clean = e.target.value.replace(/^0x/i, "");
                        const parsed = parseInt(clean || "0", 16);
                        if (!Number.isNaN(parsed)) {
                          const copy = structuredClone(structDraft);
                          copy.groups[gi].flags2 = parsed >>> 0;
                          setStructDraft(copy);
                        }
                      }}
                    />
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      );
    }

    // FLS (Stage Lighting Filters & 16 Fog Banks)
    if (kind === "FLS" && Array.isArray(structDraft.filters)) {
      return (
        <div style={{ display: "flex", flexDirection: "column", gap: 16 }}>
          <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center" }}>
            <span style={{ fontWeight: 600, color: "var(--brush-heading)" }}>
              FLS Stage Filters ({structDraft.filters.length}) &amp; Fog Banks ({structDraft.fogs?.length ?? 0})
            </span>
            <button
              className="pill-button accent"
              onClick={() => runStructMutation(nodePayload.node_id, "FLS", structDraft)}
            >
              Commit FLS Changes
            </button>
          </div>
          <table className="struct-table">
            <thead>
              <tr>
                <th>Filter #</th>
                <th>Position / Direction</th>
                <th>Color 1 (RGB)</th>
                <th>Color 2 (RGB)</th>
              </tr>
            </thead>
            <tbody>
              {structDraft.filters.map((f: any, fi: number) => (
                <tr key={fi}>
                  <td className="mono">#{fi}</td>
                  <td className="mono">
                    {Array.isArray(f.pos) ? f.pos.map((v: number) => Number(v).toFixed(2)).join(", ") : "—"}
                  </td>
                  <td>
                    <div style={{ display: "flex", alignItems: "center", gap: 6 }}>
                      <input
                        type="color"
                        value={rgbFloatToHex(f.color1 || [1, 1, 1])}
                        onChange={(e) => {
                          const [r, g, b] = hexToRgbFloat(e.target.value);
                          const copy = structuredClone(structDraft);
                          copy.filters[fi].color1 = [r, g, b, f.color1?.[3] ?? 1];
                          setStructDraft(copy);
                        }}
                        style={{ width: 24, height: 24, padding: 0, borderRadius: 4, cursor: "pointer" }}
                      />
                      <span className="mono" style={{ fontSize: 11 }}>
                        {(f.color1 || []).slice(0, 3).map((v: number) => Number(v).toFixed(2)).join(", ")}
                      </span>
                    </div>
                  </td>
                  <td>
                    <div style={{ display: "flex", alignItems: "center", gap: 6 }}>
                      <input
                        type="color"
                        value={rgbFloatToHex(f.color2 || [1, 1, 1])}
                        onChange={(e) => {
                          const [r, g, b] = hexToRgbFloat(e.target.value);
                          const copy = structuredClone(structDraft);
                          copy.filters[fi].color2 = [r, g, b, f.color2?.[3] ?? 1];
                          setStructDraft(copy);
                        }}
                        style={{ width: 24, height: 24, padding: 0, borderRadius: 4, cursor: "pointer" }}
                      />
                      <span className="mono" style={{ fontSize: 11 }}>
                        {(f.color2 || []).slice(0, 3).map((v: number) => Number(v).toFixed(2)).join(", ")}
                      </span>
                    </div>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>

          {Array.isArray(structDraft.fogs) && structDraft.fogs.length > 0 && (
            <table className="struct-table">
              <thead>
                <tr>
                  <th>Fog #</th>
                  <th>Start Distance</th>
                  <th>End Distance</th>
                  <th>Fog Color (RGB)</th>
                </tr>
              </thead>
              <tbody>
                {structDraft.fogs.map((fg: any, fi: number) => (
                  <tr key={fi}>
                    <td className="mono">#{fi}</td>
                    <td>
                      <input
                        type="number"
                        step="0.5"
                        className="field-box mono"
                        style={{ height: 24 }}
                        value={Number((fg.start ?? 0).toFixed(2))}
                        onChange={(e) => {
                          const copy = structuredClone(structDraft);
                          copy.fogs[fi].start = parseFloat(e.target.value || "0");
                          setStructDraft(copy);
                        }}
                      />
                    </td>
                    <td>
                      <input
                        type="number"
                        step="0.5"
                        className="field-box mono"
                        style={{ height: 24 }}
                        value={Number((fg.end ?? 0).toFixed(2))}
                        onChange={(e) => {
                          const copy = structuredClone(structDraft);
                          copy.fogs[fi].end = parseFloat(e.target.value || "0");
                          setStructDraft(copy);
                        }}
                      />
                    </td>
                    <td>
                      <div style={{ display: "flex", alignItems: "center", gap: 6 }}>
                        <input
                          type="color"
                          value={rgbFloatToHex(fg.color || [1, 1, 1])}
                          onChange={(e) => {
                            const [r, g, b] = hexToRgbFloat(e.target.value);
                            const copy = structuredClone(structDraft);
                            copy.fogs[fi].color = [r, g, b];
                            setStructDraft(copy);
                          }}
                          style={{ width: 24, height: 24, padding: 0, borderRadius: 4, cursor: "pointer" }}
                        />
                        <span className="mono" style={{ fontSize: 11 }}>
                          {(fg.color || []).map((v: number) => Number(v).toFixed(2)).join(", ")}
                        </span>
                      </div>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
        </div>
      );
    }

    // XAD (`.fla` / `.cta` Trigger Areas)
    if (kind === "XAD" && Array.isArray(structDraft.entries)) {
      return (
        <div style={{ display: "flex", flexDirection: "column", gap: 12 }}>
          <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center" }}>
            <span style={{ fontWeight: 600, color: "var(--brush-heading)" }}>
              XAD (.fla / .cta) Trigger Areas ({structDraft.entries.length})
            </span>
            <button
              className="pill-button accent"
              onClick={() => runStructMutation(nodePayload.node_id, "XAD", structDraft)}
            >
              Commit XAD Changes
            </button>
          </div>
          <table className="struct-table">
            <thead>
              <tr>
                <th>#</th>
                <th>Area ID</th>
                <th>Priority</th>
                <th>Target Index</th>
                <th>Center (X, Y, Z)</th>
                <th>Radius / Extent</th>
                <th>Interp Time</th>
              </tr>
            </thead>
            <tbody>
              {structDraft.entries.map((en: any, ei: number) => (
                <tr key={ei}>
                  <td className="mono">#{ei}</td>
                  <td className="mono">{en.id}</td>
                  <td>
                    <input
                      type="number"
                      className="field-box mono"
                      style={{ height: 24 }}
                      value={en.priority ?? 0}
                      onChange={(e) => {
                        const copy = structuredClone(structDraft);
                        copy.entries[ei].priority = parseInt(e.target.value || "0", 10);
                        setStructDraft(copy);
                      }}
                    />
                  </td>
                  <td>
                    <input
                      type="number"
                      className="field-box mono"
                      style={{ height: 24 }}
                      value={en.target_index ?? 0}
                      onChange={(e) => {
                        const copy = structuredClone(structDraft);
                        copy.entries[ei].target_index = parseInt(e.target.value || "0", 10);
                        setStructDraft(copy);
                      }}
                    />
                  </td>
                  <td>
                    <div style={{ display: "grid", gridTemplateColumns: "repeat(3, 1fr)", gap: 4 }}>
                      {[0, 1, 2].map((ai) => (
                        <input
                          key={ai}
                          type="number"
                          step="0.5"
                          className="field-box mono"
                          style={{ height: 24 }}
                          value={Number((en.position?.[ai] ?? 0).toFixed(2))}
                          onChange={(e) => {
                            const copy = structuredClone(structDraft);
                            copy.entries[ei].position[ai] = parseFloat(e.target.value || "0");
                            setStructDraft(copy);
                          }}
                        />
                      ))}
                    </div>
                  </td>
                  <td>
                    <input
                      type="number"
                      step="0.5"
                      className="field-box mono"
                      style={{ height: 24 }}
                      value={Number((en.radius ?? 0).toFixed(2))}
                      onChange={(e) => {
                        const copy = structuredClone(structDraft);
                        copy.entries[ei].radius = parseFloat(e.target.value || "0");
                        setStructDraft(copy);
                      }}
                    />
                  </td>
                  <td>
                    <input
                      type="number"
                      step="0.1"
                      className="field-box mono"
                      style={{ height: 24 }}
                      value={Number((en.interp_time ?? 0).toFixed(2))}
                      onChange={(e) => {
                        const copy = structuredClone(structDraft);
                        copy.entries[ei].interp_time = parseFloat(e.target.value || "0");
                        setStructDraft(copy);
                      }}
                    />
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      );
    }

    // LIT (Stage Lights & Shadow/Sun Lights)
    if (kind === "LIT" && Array.isArray(structDraft.lights)) {
      return (
        <div style={{ display: "flex", flexDirection: "column", gap: 12 }}>
          <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center" }}>
            <span style={{ fontWeight: 600, color: "var(--brush-heading)" }}>
              LIT Stage Lights ({structDraft.lights.length})
            </span>
            <button
              className="pill-button accent"
              onClick={() => runStructMutation(nodePayload.node_id, "LIT", structDraft)}
            >
              Commit LIT Changes
            </button>
          </div>
          <table className="struct-table">
            <thead>
              <tr>
                <th>Light #</th>
                <th>Type</th>
                <th>Position / Direction (X, Y, Z)</th>
                <th>Color (RGBA)</th>
                <th>Intensity</th>
              </tr>
            </thead>
            <tbody>
              {structDraft.lights.map((lt: any, li: number) => (
                <tr key={li}>
                  <td className="mono">#{li}</td>
                  <td className="mono">{lt.light_type}</td>
                  <td>
                    <div style={{ display: "grid", gridTemplateColumns: "repeat(3, 1fr)", gap: 4 }}>
                      {[0, 1, 2].map((ai) => (
                        <input
                          key={ai}
                          type="number"
                          step="0.25"
                          className="field-box mono"
                          style={{ height: 24 }}
                          value={Number((lt.position?.[ai] ?? 0).toFixed(3))}
                          onChange={(e) => {
                            const copy = structuredClone(structDraft);
                            copy.lights[li].position[ai] = parseFloat(e.target.value || "0");
                            setStructDraft(copy);
                          }}
                        />
                      ))}
                    </div>
                  </td>
                  <td>
                    <div style={{ display: "flex", alignItems: "center", gap: 6 }}>
                      <input
                        type="color"
                        value={rgbFloatToHex(lt.color || [1, 1, 1])}
                        onChange={(e) => {
                          const [r, g, b] = hexToRgbFloat(e.target.value);
                          const copy = structuredClone(structDraft);
                          copy.lights[li].color[0] = r;
                          copy.lights[li].color[1] = g;
                          copy.lights[li].color[2] = b;
                          setStructDraft(copy);
                        }}
                        style={{ width: 24, height: 24, padding: 0, borderRadius: 4, cursor: "pointer" }}
                      />
                      <span className="mono" style={{ fontSize: 11 }}>
                        {(lt.color || []).slice(0, 3).map((v: number) => Number(v).toFixed(2)).join(", ")}
                      </span>
                    </div>
                  </td>
                  <td>
                    <input
                      type="number"
                      step="0.05"
                      className="field-box mono"
                      style={{ height: 24 }}
                      value={Number((lt.intensity ?? 1).toFixed(3))}
                      onChange={(e) => {
                        const copy = structuredClone(structDraft);
                        copy.lights[li].intensity = parseFloat(e.target.value || "1");
                        setStructDraft(copy);
                      }}
                    />
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      );
    }

    // MDB (MadWorld `.mdb` Models)
    if (kind === "MDB" && Array.isArray(structDraft.meshes)) {
      return (
        <div style={{ display: "flex", flexDirection: "column", gap: 12 }}>
          <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center" }}>
            <span style={{ fontWeight: 600, color: "var(--brush-heading)" }}>
              MDB Model ({structDraft.num_positions} vertices · {structDraft.num_bones} bones ·{" "}
              {structDraft.meshes.length} meshes)
            </span>
            <button
              className="pill-button accent"
              onClick={() => runStructMutation(nodePayload.node_id, "MDB", structDraft)}
            >
              Commit MDB Changes
            </button>
          </div>
          <table className="struct-table">
            <thead>
              <tr>
                <th>Mesh #</th>
                <th>Material ID</th>
                <th>Batch Count</th>
                <th>Total Triangles</th>
              </tr>
            </thead>
            <tbody>
              {structDraft.meshes.map((m: any, mi: number) => (
                <tr key={mi}>
                  <td className="mono">#{mi}</td>
                  <td>
                    <input
                      type="number"
                      className="field-box mono"
                      style={{ height: 24 }}
                      value={m.material_id ?? 0}
                      onChange={(e) => {
                        const copy = structuredClone(structDraft);
                        copy.meshes[mi].material_id = parseInt(e.target.value || "0", 10);
                        setStructDraft(copy);
                      }}
                    />
                  </td>
                  <td className="mono">{m.num_batches ?? 0}</td>
                  <td className="mono">{m.num_triangles ?? 0}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      );
    }

    // EXP (Bayo1 / Bayo2 Expression Constraints) or generic structured record list
    const expVariantKey = structDraft.Bayo1
      ? "Bayo1"
      : structDraft.Bayo2
      ? "Bayo2"
      : structDraft.V1
      ? "V1"
      : structDraft.V2
      ? "V2"
      : null;
    const expData = expVariantKey ? structDraft[expVariantKey] : structDraft;
    if (expData && Array.isArray(expData.records)) {
      return (
        <div style={{ display: "flex", flexDirection: "column", gap: 12 }}>
          <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center" }}>
            <span style={{ fontWeight: 600, color: "var(--brush-heading)" }}>
              Expression Constraint Tracks ({expData.records.length})
            </span>
            <button
              className="pill-button accent"
              onClick={() => runStructMutation(nodePayload.node_id, "EXP", structDraft)}
            >
              Commit EXP Changes
            </button>
          </div>
          <table className="struct-table">
            <thead>
              <tr>
                <th>#</th>
                <th>Bone Index</th>
                <th>Animation Track</th>
                <th>Flags / Type</th>
                <th>Operations / Points</th>
              </tr>
            </thead>
            <tbody>
              {expData.records.map((r: any, ri: number) => (
                <tr key={ri}>
                  <td className="mono">#{ri}</td>
                  <td>
                    <input
                      type="number"
                      className="field-box mono"
                      style={{ width: 85, height: 24 }}
                      value={r.bone_index ?? 0}
                      onChange={(e) => {
                        const copy = structuredClone(structDraft);
                        const target = expVariantKey ? copy[expVariantKey] : copy;
                        target.records[ri].bone_index = parseInt(e.target.value || "0", 10);
                        setStructDraft(copy);
                      }}
                    />
                  </td>
                  <td>
                    <input
                      type="number"
                      className="field-box mono"
                      style={{ width: 85, height: 24 }}
                      value={r.animation_track ?? 0}
                      onChange={(e) => {
                        const copy = structuredClone(structDraft);
                        const target = expVariantKey ? copy[expVariantKey] : copy;
                        target.records[ri].animation_track = parseInt(e.target.value || "0", 10);
                        setStructDraft(copy);
                      }}
                    />
                  </td>
                  <td className="mono">{r.flags ?? r.entry_type ?? 0}</td>
                  <td className="mono">
                    {Array.isArray(r.operations)
                      ? `${r.operations.length} bytecode ops`
                      : Array.isArray(r.points)
                      ? `${r.points.length} spline points`
                      : "—"}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      );
    }

    return null;
  }

  return (
    <div
      className="studio-window"
      onClick={() => {
        setActiveTopMenu(null);
        setContextMenu(null);
      }}
      onContextMenu={(e) => {
        // Clicking empty space shows nothing and closes any open custom menu
        e.preventDefault();
        setContextMenu(null);
        setActiveTopMenu(null);
      }}
      onDragOver={(e) => {
        // External OS file drops are handled via Tauri's native tauri://drag-drop event listener (B-43)
        if (e.dataTransfer?.types?.includes("Files")) {
          e.preventDefault();
          e.dataTransfer.dropEffect = "copy";
        }
      }}
    >
      {/* Floating Pointer Drag Ghost Badge */}
      {pointerDrag?.active && (
        <div
          className="pointer-drag-ghost"
          style={{
            left: pointerDrag.curX + 14,
            top: pointerDrag.curY + 12,
          }}
        >
          <span className="mono">{pointerDrag.label}</span>
          <span style={{ color: "var(--brush-muted)", fontWeight: 400, fontSize: 11 }}>
            {pointerDrag.overId
              ? `Drop ${pointerDrag.insertPos} target`
              : pointerDrag.kind === "asset"
              ? "Drag outside to extract to Explorer"
              : "Drag to reorder"}
          </span>
        </div>
      )}

      {/* External OS File Drop Visual Overlay */}
      {externalDragHover && (
        <div className="external-drop-overlay">Drop PlatinumGames file to open in Studio</div>
      )}

      {/* ==================== ROW 0: MENU BAR ==================== */}
      <header className="studio-menubar" onClick={(e) => e.stopPropagation()}>
        <div style={{ position: "relative" }}>
          <button
            className={`menu-top-button ${activeTopMenu === "file" ? "active" : ""}`}
            onClick={() => setActiveTopMenu(activeTopMenu === "file" ? null : "file")}
          >
            File
          </button>
          {activeTopMenu === "file" && (
            <div className="context-menu" style={{ top: 28, left: 0 }}>
              <button className="context-menu-item" onClick={handleMenuOpenFile}>
                <span>Open File...</span>
                <span className="context-menu-shortcut">Ctrl+O</span>
              </button>
              <button className="context-menu-item" onClick={handleMenuOpenFolder}>
                <span>Open Folder...</span>
              </button>
              <div
                className="context-menu-item"
                onMouseEnter={() => setHoverSubmenuIdx(99)}
                onMouseLeave={() => setHoverSubmenuIdx(null)}
              >
                <span>Open Recent</span>
                <span className="context-menu-shortcut">›</span>
                {hoverSubmenuIdx === 99 && (
                  <div className="context-submenu">
                    {recentFiles.length === 0 ? (
                      <button className="context-menu-item" disabled>
                        <span>(No recent files)</span>
                      </button>
                    ) : (
                      recentFiles.map((rf, idx) => (
                        <button
                          key={idx}
                          className="context-menu-item"
                          onClick={() => {
                            setActiveTopMenu(null);
                            openDocumentByPath(rf);
                          }}
                          title={rf}
                        >
                          <span style={{ overflow: "hidden", textOverflow: "ellipsis", maxWidth: 260 }}>
                            {rf.split(/[\\/]/).pop()}
                          </span>
                        </button>
                      ))
                    )}
                  </div>
                )}
              </div>
              <button
                className="context-menu-item"
                disabled={!outline}
                onClick={() => {
                  setActiveTopMenu(null);
                  if (outline) openDocumentByPath(outline.path);
                }}
              >
                <span>Reload</span>
              </button>
              <div className="context-menu-sep" />
              <button className="context-menu-item" disabled={!outline} onClick={handleMenuSave}>
                <span>Save</span>
                <span className="context-menu-shortcut">Ctrl+S</span>
              </button>
              <button className="context-menu-item" disabled={!outline} onClick={handleMenuSaveAs}>
                <span>Save As...</span>
                <span className="context-menu-shortcut">Ctrl+Shift+S</span>
              </button>
            </div>
          )}
        </div>

        <div style={{ position: "relative" }}>
          <button
            className={`menu-top-button ${activeTopMenu === "tools" ? "active" : ""}`}
            onClick={() => setActiveTopMenu(activeTopMenu === "tools" ? null : "tools")}
          >
            Tools
          </button>
          {activeTopMenu === "tools" && (
            <div className="context-menu" style={{ top: 28, left: 0, minWidth: 265 }}>
              <button
                className="context-menu-item"
                disabled={!activeTexture}
                onClick={() => {
                  setActiveTopMenu(null);
                  if (activeTexture) handleTextureExport(activeTexture.node_id, "bank", "png");
                }}
              >
                <span>Export all textures in bank to PNG...</span>
              </button>
              <button
                className="context-menu-item"
                disabled={!activeTexture}
                onClick={() => {
                  setActiveTopMenu(null);
                  if (activeTexture) handleTextureExport(activeTexture.node_id, "bank", "dds");
                }}
              >
                <span>Export all textures in bank to DDS...</span>
              </button>
              <div className="context-menu-sep" />
              <button
                className="context-menu-item"
                disabled={!activeWmb}
                onClick={() => {
                  setActiveTopMenu(null);
                  if (activeWmb) runWmbMutation(activeWmb.node_id, "cleanup_vertexes");
                }}
              >
                <span>WMB: Cleanup unreferenced vertexes</span>
              </button>
              <button
                className="context-menu-item"
                disabled={!activeWmb}
                onClick={() => {
                  setActiveTopMenu(null);
                  if (activeWmb) runWmbMutation(activeWmb.node_id, "cleanup_materials");
                }}
              >
                <span>WMB: Cleanup unused materials</span>
              </button>
              <div className="context-menu-sep" />
              <button
                className="context-menu-item"
                onClick={() => {
                  setActiveTopMenu(null);
                  handlePickOodleDll();
                }}
              >
                <span>Configure Oodle Kraken DLL (oo2core)...</span>
              </button>
              <button
                className="context-menu-item"
                onClick={() => {
                  setActiveTopMenu(null);
                  handlePickNvttDll();
                }}
              >
                <span>Configure NVIDIA Texture Tools DLL (nvtt30205)...</span>
              </button>
              {/* [wmb-studio BEGIN phase2] */}
              <div className="context-menu-sep" />
              <button
                className="context-menu-item"
                onClick={() => {
                  setActiveTopMenu(null);
                  setCoreDatModalOpen(true);
                }}
              >
                <span>Configure core.dat (per game)...</span>
              </button>
              <div className="context-menu-sep" />
              <div style={{ padding: "4px 10px", fontSize: 11, color: "var(--brush-muted)", fontWeight: 600 }}>
                Open demo model
              </div>
              {(
                [
                  ["b1", "Bayonetta 1 (WMB0)"],
                  ["b2", "Bayonetta 2 (WMB0)"],
                  ["b3", "Bayonetta 3 (WMB3)"],
                ] as const
              ).map(([gameKey, gameLabel]) => (
                <button
                  key={gameKey}
                  className="context-menu-item"
                  onClick={async () => {
                    setActiveTopMenu(null);
                    try {
                      setStatusLeft(`Generating ${gameLabel} demo model...`);
                      const demoPath = await createDemoModel(gameKey);
                      await openDocumentByPath(demoPath);
                    } catch (err) {
                      setStatusLeft(`Demo model error: ${String(err)}`);
                    }
                  }}
                >
                  <span>• {gameLabel}</span>
                </button>
              ))}
              {/* [wmb-studio END] */}
            </div>
          )}
        </div>

        <div style={{ position: "relative" }}>
          <button
            className={`menu-top-button ${activeTopMenu === "help" ? "active" : ""}`}
            onClick={() => setActiveTopMenu(activeTopMenu === "help" ? null : "help")}
          >
            Help
          </button>
          {activeTopMenu === "help" && (
            <div className="context-menu" style={{ top: 28, left: 0, minWidth: 280 }}>
              <div style={{ padding: "8px 10px" }}>
                <div style={{ fontWeight: 600, color: "var(--brush-heading)" }}>PGTools Studio</div>
                <div style={{ color: "var(--brush-muted)", fontSize: 11.5, marginTop: 3 }}>
                  Pure Rust &amp; Tauri v2 suite for Bayonetta 1/2/3, NieR:Automata, MGR:R, Vanquish &amp; Astral Chain.
                </div>
              </div>
            </div>
          )}
        </div>
      </header>

      {/* ==================== ROW 1: SEGMENTED TOOLBAR (Texture Controls Auto-Hide!) ==================== */}
      <div className="studio-toolbar">
        <div className="toolbar-group">
          {activeTexture && (
            <>
              <span className="toolbar-label">Mip</span>
              <select
                className="pill-select"
                style={{ width: 156 }}
                value={mipIndex}
                onChange={(e) =>
                  handleTexturePreviewParamChange(Number(e.target.value), channelMode, opaquePreview)
                }
              >
                {activeTexture.mips.map((m) => (
                  <option key={m.index} value={m.index}>
                    {m.label}
                  </option>
                ))}
              </select>

              <div className="toolbar-spacer" />

              <span className="toolbar-label">Channel</span>
              <select
                className="pill-select"
                style={{ width: 110 }}
                value={channelMode}
                onChange={(e) =>
                  handleTexturePreviewParamChange(mipIndex, e.target.value, opaquePreview)
                }
              >
                {["RGBA", "RGB", "R", "G", "B", "A"].map((ch) => (
                  <option key={ch} value={ch}>
                    {ch}
                  </option>
                ))}
              </select>

              <div className="toolbar-spacer" />

              <div
                className={`pill-checkbox ${opaquePreview ? "checked" : ""}`}
                onClick={() =>
                  handleTexturePreviewParamChange(mipIndex, channelMode, !opaquePreview)
                }
                title="Ignore the alpha channel in the 2D preview"
              >
                <span className="pill-checkbox-box">{opaquePreview && <IconCheck />}</span>
                <span>Opaque Preview</span>
              </div>

              <div className="toolbar-spacer" />

              <button
                className="pill-button"
                onClick={handleResetView}
                title="Fit preview to viewport (100%)"
              >
                <IconRefresh />
                <span>Reset View</span>
              </button>

              <div className="toolbar-spacer" />
            </>
          )}

          <button
            className="pill-button"
            disabled={!outline?.can_undo}
            onClick={handleUndo}
            title={outline?.undo_label ? `Undo ${outline.undo_label} (Ctrl+Z)` : "Undo (Ctrl+Z)"}
          >
            <IconUndo />
            <span>Undo</span>
          </button>

          <button
            className="pill-button"
            disabled={!outline?.can_redo}
            onClick={handleRedo}
            title={outline?.redo_label ? `Redo ${outline.redo_label} (Ctrl+Y)` : "Redo (Ctrl+Y)"}
          >
            <IconRedo />
            <span>Redo</span>
          </button>
        </div>

        <div className="toolbar-group">
          {outline?.dirty && (
            <button
              className="pill-button accent"
              onClick={handleMenuSave}
              title="Save modified container in place (Ctrl+S)"
            >
              <span>● Save {outline.file_name}</span>
            </button>
          )}
          <button
            className="pill-button"
            onClick={() => setTheme(theme === "dark" ? "light" : "dark")}
            title="Toggle light / dark theme"
          >
            <IconSunMoon />
            <span>Theme</span>
          </button>
        </div>
      </div>

      {/* ==================== ROW 2: 3-PANE STUDIO WORKSPACE ==================== */}
      <div className="studio-main">
        {/* LEFT COLUMN: Stacked Asset Browser + Resizable File Browser */}
        <div className="left-column" ref={leftColRef} style={{ width: leftWidth }}>
          {/* Top-Left: Asset Browser */}
          <div
            ref={assetBrowserCardRef}
            className="studio-card"
            style={{ flex: `${assetRatio} 1 0%`, minHeight: 110 }}
          >
            <div className="pane-header">
              <span className="pane-title">Asset Browser</span>
              <input
                type="text"
                className="field-box"
                style={{ width: 135, height: 24, fontSize: 11.5 }}
                placeholder="Filter nodes..."
                value={assetSearch}
                onChange={(e) => setAssetSearch(e.target.value)}
              />
            </div>
            <div className="tree-scroll">
              {outline ? (
                renderTreeNodes(outline.tree)
              ) : (
                <div style={{ padding: "18px 10px", color: "var(--brush-muted)", textAlign: "center" }}>
                  Select a file in the File Browser below, drag &amp; drop a file onto the window, or press Ctrl+O.
                </div>
              )}
            </div>
          </div>

          {/* Horizontal Splitter between Asset Browser and File Browser */}
          <div
            className={`splitter-h ${draggingSplit === "left-row" ? "dragging" : ""}`}
            onMouseDown={(e) => {
              e.preventDefault();
              setDraggingSplit("left-row");
            }}
            onDoubleClick={() => setAssetRatio(0.5)}
            title="Drag to resize, double-click to reset"
          >
            <div className="splitter-h-grip" />
          </div>

          {/* Bottom-Left: File Browser with Sort by Type & Hide Unsupported Toggle */}
          <div className="studio-card" style={{ flex: `${1 - assetRatio} 1 0%`, minHeight: 110 }}>
            <div className="pane-header">
              <span className="pane-title">File Browser</span>
              <div style={{ display: "flex", gap: 4, alignItems: "center" }}>
                <button
                  className={`pill-button ${hideUnsupported ? "accent" : ""}`}
                  style={{ height: 24, padding: "0 8px", fontSize: 11 }}
                  onClick={() => setHideUnsupported(!hideUnsupported)}
                  title="Hide unsupported file extensions (On by default)"
                >
                  Supported Only
                </button>
                <button
                  className="pill-button"
                  style={{ height: 24, padding: "0 8px", fontSize: 11.5 }}
                  onClick={() => {
                    if (!currentDir) return;
                    const up = currentDir.replace(/[\\/][^\\/]+$/, "");
                    if (up && up !== currentDir) loadDirectory(up);
                  }}
                  title="Parent Directory"
                >
                  ↑ Up
                </button>
                <button
                  className="pill-button"
                  style={{ height: 24, padding: "0 8px", fontSize: 11.5 }}
                  onClick={handleMenuOpenFolder}
                  title="Browse Folder..."
                >
                  <IconFolder />
                </button>
              </div>
            </div>

            <div className="file-browser-controls">
              <div className="file-folder-bar">
                <input
                  type="text"
                  className="field-box mono"
                  style={{ flex: 1, height: 25, fontSize: 11 }}
                  value={dirInput}
                  onChange={(e) => setDirInput(e.target.value)}
                  onKeyDown={(e) => {
                    if (e.key === "Enter") loadDirectory(dirInput);
                  }}
                  placeholder="Folder path..."
                />
              </div>
              <input
                type="text"
                className="field-box mono"
                style={{ width: "100%", height: 25, fontSize: 11.5 }}
                value={fileGlobFilter}
                onChange={(e) => setFileGlobFilter(e.target.value)}
                placeholder="Glob filter (e.g. *.dat, pl00*.wmb, *.pkz)..."
              />
            </div>

            <div className="file-table-wrap">
              <table className="file-table">
                <thead>
                  <tr>
                    <th style={{ width: "42%" }} onClick={() => toggleSort("name")}>
                      Name {fileSortCol === "name" ? (fileSortAsc ? "▲" : "▼") : ""}
                    </th>
                    <th style={{ width: "16%" }} onClick={() => toggleSort("type")}>
                      Type {fileSortCol === "type" ? (fileSortAsc ? "▲" : "▼") : ""}
                    </th>
                    <th style={{ width: "22%" }} onClick={() => toggleSort("modified")}>
                      Modified {fileSortCol === "modified" ? (fileSortAsc ? "▲" : "▼") : ""}
                    </th>
                    <th style={{ width: "20%", textAlign: "right" }} onClick={() => toggleSort("size")}>
                      Size {fileSortCol === "size" ? (fileSortAsc ? "▲" : "▼") : ""}
                    </th>
                  </tr>
                </thead>
                <tbody>
                  {filteredFiles.map((entry, idx) => {
                    const isSelected =
                      selectedFilePath === entry.path || selectedFilePaths.includes(entry.path);
                    return (
                      <tr
                        key={entry.path}
                        className={`file-row ${isSelected ? "selected" : ""}`}
                        title={entry.path}
                        onClick={(e) => {
                          if (e.ctrlKey || e.metaKey) {
                            setSelectedFilePaths((prev) =>
                              prev.includes(entry.path)
                                ? prev.filter((p) => p !== entry.path)
                                : [...prev, entry.path]
                            );
                            setSelectedFilePath(entry.path);
                            return;
                          }
                          if (e.shiftKey && selectedFilePath) {
                            const anchorIdx = filteredFiles.findIndex(
                              (f) => f.path === selectedFilePath
                            );
                            if (anchorIdx !== -1) {
                              const lo = Math.min(anchorIdx, idx);
                              const hi = Math.max(anchorIdx, idx);
                              setSelectedFilePaths(filteredFiles.slice(lo, hi + 1).map((f) => f.path));
                              return;
                            }
                          }
                          setSelectedFilePaths([entry.path]);
                          setSelectedFilePath(entry.path);
                        }}
                        onDoubleClick={() => {
                          if (entry.is_dir) {
                            void loadDirectory(entry.path);
                          } else {
                            void openDocumentByPath(entry.path);
                          }
                        }}
                        onContextMenu={(e) => {
                          e.preventDefault();
                          e.stopPropagation();
                          if (!selectedFilePaths.includes(entry.path)) {
                            setSelectedFilePaths([entry.path]);
                          }
                          setSelectedFilePath(entry.path);
                          setHoverSubmenuIdx(null);
                          setContextMenu({
                            x: Math.min(e.clientX, window.innerWidth - 250),
                            y: Math.min(e.clientY, window.innerHeight - 360),
                            items: buildFileBrowserContextMenu(entry),
                          });
                        }}
                      >
                        <td>
                          {entry.is_dir ? "📁 " : ""}
                          {entry.name}
                        </td>
                        <td
                          className="mono"
                          style={{ color: isSelected ? "var(--brush-text)" : "var(--brush-muted)" }}
                        >
                          {entry.is_dir ? "DIR" : entry.extension ? `.${entry.extension}` : "—"}
                        </td>
                        <td style={{ color: isSelected ? "var(--brush-text)" : "var(--brush-muted)" }}>
                          {entry.modified_str}
                        </td>
                        <td
                          style={{
                            textAlign: "right",
                            color: isSelected ? "var(--brush-text)" : "var(--brush-muted)",
                          }}
                        >
                          {entry.is_dir ? "" : formatBytes(entry.size)}
                        </td>
                      </tr>
                    );
                  })}
                </tbody>
              </table>
            </div>
          </div>
        </div>

        {/* Left Vertical Splitter */}
        <div
          className={`splitter-v ${draggingSplit === "left" ? "dragging" : ""}`}
          onMouseDown={(e) => {
            e.preventDefault();
            setDraggingSplit("left");
          }}
          onDoubleClick={() => setLeftWidth(310)}
          title="Drag to resize, double-click to reset"
        >
          <div className="splitter-v-grip" />
        </div>

        {/* [wmb-studio BEGIN phase2] */}
        {Boolean(activeWmb && !isVirtualSubNode(selectedNodeId)) ? (
          <Workspace
            nodeId={selectedNodeId}
            docGeneration={wmbDocGeneration}
            theme={theme}
            rightPanelWidth={rightWidth}
            onStartResizeRightPanel={(e) => {
              e.preventDefault();
              setDraggingSplit("right");
            }}
            hierarchyRatio={wmbHierarchyRatio}
            onChangeHierarchyRatio={setWmbHierarchyRatio}
            wireframe={wmbWireframe}
            onToggleWireframe={() => setWmbWireframe((v) => !v)}
            normalsView={wmbNormalsView}
            onToggleNormalsView={() => setWmbNormalsView((v) => !v)}
            skeletonOverlay={wmbSkeletonOverlay}
            onToggleSkeletonOverlay={() => setWmbSkeletonOverlay((v) => !v)}
            gridVisible={wmbGridVisible}
            onToggleGridVisible={() => setWmbGridVisible((v) => !v)}
            onOutlineUpdated={(updated, statusMsg) => {
              setOutline(updated);
              setWmbDocGeneration((g) => g + 1);
              if (statusMsg) setStatusLeft(updated.undo_label ?? statusMsg);
            }}
            onStatusError={(msg) => setStatusLeft(msg)}
            onTriggerLegacyAction={async (legacyNodeId: string, action: LegacyActionKind) => {
              if (action === "export_obj") {
                const dest = await invoke<string | null>("save_file_dialog", {
                  title: "Export WMB Model to Wavefront .OBJ",
                  defaultName: "model.obj",
                });
                if (dest) await runWmbMutation(selectedNodeId, "export_obj", { path: dest });
              } else if (action === "export_texture") {
                await handleTextureExport(legacyNodeId, "single", "png");
              } else if (action === "replace_texture") {
                await promptAndRunTextureImport(legacyNodeId, "replace_file");
              } else if (action === "import_texture") {
                await promptAndRunTextureImport(legacyNodeId, "add_texture");
              } else {
                await selectAndInspectNode(legacyNodeId);
              }
            }}
          />
        ) : (
        <>
        {/* [wmb-studio END] */}
        {/* CENTER COLUMN: Contextual Viewport / Structured Table Editor */}
        <div className="center-column">
          <div className="studio-card viewer-card">
            <div className="underline-tabbar">
              {activeTexture ? (
                <>
                  <button
                    className={`underline-tab ${centerTab === "primary" ? "active" : ""}`}
                    onClick={() => setCenterTab("primary")}
                  >
                    <IconImage size={12} />
                    <span>2D</span>
                  </button>
                  <button
                    className={`underline-tab ${centerTab === "secondary" ? "active" : ""}`}
                    onClick={() => setCenterTab("secondary")}
                  >
                    <IconCube size={12} />
                    <span>Header &amp; Surface Layout</span>
                  </button>
                </>
              ) : nodePayload?.view_type === "BXM" ? (
                <>
                  <button
                    className={`underline-tab ${bxmViewMode === "tree" ? "active" : ""}`}
                    onClick={() => switchBxmViewMode("tree")}
                  >
                    <IconCode size={12} />
                    <span>Interactive Tree</span>
                  </button>
                  <button
                    className={`underline-tab ${bxmViewMode === "table" ? "active" : ""}`}
                    onClick={() => switchBxmViewMode("table")}
                  >
                    <IconCube size={12} />
                    <span>Table View</span>
                  </button>
                  <button
                    className={`underline-tab ${bxmViewMode === "xml" ? "active" : ""}`}
                    onClick={() => switchBxmViewMode("xml")}
                  >
                    <IconCode size={12} />
                    <span>Raw XML</span>
                  </button>
                </>
              ) : (
                <>
                  <button
                    className={`underline-tab ${centerTab === "primary" ? "active" : ""}`}
                    onClick={() => setCenterTab("primary")}
                  >
                    <IconCube size={12} />
                    <span>Structured Editor</span>
                  </button>
                  <button
                    className={`underline-tab ${centerTab === "secondary" ? "active" : ""}`}
                    onClick={() => setCenterTab("secondary")}
                  >
                    <IconCode size={12} />
                    <span>Hex / Raw Inspector</span>
                  </button>
                </>
              )}
            </div>

            {/* Center Body */}
            {activeTexture && centerTab === "primary" ? (
              <div
                className="checkerboard-canvas"
                onWheel={(e) => {
                  e.preventDefault();
                  const factor = e.deltaY < 0 ? 1.12 : 1 / 1.12;
                  setZoom((z) => Math.max(0.05, Math.min(16.0, z * factor)));
                }}
                onMouseDown={(e) => {
                  if (e.button === 0) {
                    setIsPanning(true);
                    panStartRef.current = { x: e.clientX - pan.x, y: e.clientY - pan.y };
                  }
                }}
                onMouseMove={(e) => {
                  if (isPanning) {
                    setPan({
                      x: e.clientX - panStartRef.current.x,
                      y: e.clientY - panStartRef.current.y,
                    });
                  }
                }}
                onMouseUp={() => setIsPanning(false)}
                onMouseLeave={() => setIsPanning(false)}
                onDoubleClick={handleResetView}
                onContextMenu={(e) => {
                  e.preventDefault();
                  e.stopPropagation();
                  setHoverSubmenuIdx(null);
                  setContextMenu({
                    x: Math.min(e.clientX, window.innerWidth - 250),
                    y: Math.min(e.clientY, window.innerHeight - 280),
                    items: buildAssetNodeContextMenu(activeTexture.node_id, "TEXTURE"),
                  });
                }}
              >
                {activeTexture.preview_data_url ? (
                  <img
                    src={activeTexture.preview_data_url}
                    alt={activeTexture.title}
                    draggable={false}
                    style={{
                      transform: `translate(${pan.x}px, ${pan.y}px) scale(${zoom})`,
                      transformOrigin: "center center",
                      imageRendering: zoom >= 2.0 ? "pixelated" : "auto",
                      maxWidth: "90%",
                      maxHeight: "90%",
                      objectFit: "contain",
                      pointerEvents: "none",
                    }}
                  />
                ) : (
                  <div className="viewport-empty">
                    <IconImage size={44} />
                    <div className="viewport-empty-title">No preview</div>
                    <div className="viewport-empty-note">{activeTexture.validation_msg}</div>
                  </div>
                )}
              </div>
            ) : activeTexture && centerTab === "secondary" ? (
              <div className="struct-table-container">
                <div style={{ fontWeight: 600, color: "var(--brush-heading)", marginBottom: 8 }}>
                  Raw Header Bytes (First 128 Bytes)
                </div>
                <pre
                  className="mono"
                  style={{
                    backgroundColor: "var(--brush-card)",
                    border: "1px solid var(--brush-border)",
                    borderRadius: 8,
                    padding: 12,
                    color: "var(--brush-text)",
                    overflowX: "auto",
                  }}
                >
                  {activeTexture.raw_header_hex}
                </pre>
              </div>
            ) : activeWmb && centerTab === "primary" ? (
              <div
                className="struct-table-container"
                onContextMenu={(e) => {
                  e.preventDefault();
                  e.stopPropagation();
                  setHoverSubmenuIdx(null);
                  const items = buildAssetNodeContextMenu(
                    activeWmb.node_id,
                    activeWmb.is_wmb3 ? "WMB3" : "WMB"
                  );
                  if (items.length > 0) {
                    setContextMenu({
                      x: Math.min(e.clientX, window.innerWidth - 250),
                      y: Math.min(e.clientY, window.innerHeight - 260),
                      items,
                    });
                  }
                }}
              >
                {renderWmbStructuredCenter(activeWmb)}
              </div>
            ) : nodePayload?.view_type === "BXM" ? (
              bxmViewMode === "tree" ? (
                <div className="struct-table-container">
                  <div
                    style={{
                      display: "flex",
                      justifyContent: "space-between",
                      alignItems: "center",
                      marginBottom: 10,
                    }}
                  >
                    <span style={{ color: "var(--brush-muted)", fontSize: 12 }}>
                      Edit tags, @attributes, and values inline. Right-click any element to add/duplicate/delete, or drag to reorder.
                    </span>
                    <div style={{ display: "flex", gap: 6 }}>
                      <button
                        className="pill-button"
                        onClick={async () => {
                          const srcXml = await invoke<string | null>("pick_file_dialog", {
                            title: "Import & Encode XML into Binary BXM",
                            extensions: ["xml"],
                          });
                          if (srcXml) {
                            await runContainerMutation(nodePayload.node_id, "import_bxm_xml", srcXml);
                          }
                        }}
                      >
                        Import .XML...
                      </button>
                      <button
                        className="pill-button"
                        onClick={async () => {
                          const dest = await invoke<string | null>("save_file_dialog", {
                            title: "Export Decoded XML (.xml)",
                            defaultName: `${(nodePayload.title || "decoded").replace(/\.[^.]+$/, "")}.xml`,
                          });
                          if (dest) {
                            await runContainerMutation(nodePayload.node_id, "export_bxm_xml", dest);
                            setStatusLeft(`Exported decoded XML to ${dest}`);
                          }
                        }}
                      >
                        Export .XML...
                      </button>
                      <button className="pill-button accent" onClick={() => commitBxmDraft()}>
                        Commit BXM Changes
                      </button>
                    </div>
                  </div>
                  {bxmTreeDraft ? (
                    renderBxmInteractiveTree(bxmTreeDraft, "0")
                  ) : (
                    <div className="viewport-empty">Empty BXM Document</div>
                  )}
                </div>
              ) : bxmViewMode === "table" ? (
                <div className="struct-table-container">{renderBxmTableView()}</div>
              ) : (
                <div style={{ flex: 1, display: "flex", flexDirection: "column", padding: 10, gap: 8 }}>
                  <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center" }}>
                    <span style={{ fontWeight: 600, color: "var(--brush-heading)" }}>
                      Raw UTF-8 XML Source
                    </span>
                    <button
                      className="pill-button accent"
                      onClick={() => commitBxmDraft(bxmXmlDraft)}
                    >
                      Commit Raw XML to Binary BXM
                    </button>
                  </div>
                  <textarea
                    className="field-box mono"
                    style={{
                      flex: 1,
                      width: "100%",
                      height: "100%",
                      resize: "none",
                      padding: 12,
                      lineHeight: 1.5,
                    }}
                    value={bxmXmlDraft}
                    onChange={(e) => setBxmXmlDraft(e.target.value)}
                  />
                </div>
              )
            ) : nodePayload?.view_type === "CSV" && centerTab === "primary" ? (
              <div className="struct-table-container">
                <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", marginBottom: 10 }}>
                  <span style={{ fontWeight: 600, color: "var(--brush-heading)" }}>
                    CSV Table ({csvRowsDraft.length} rows × {csvHeadersDraft.length} columns)
                  </span>
                  <div style={{ display: "flex", gap: 8 }}>
                    <button
                      className="pill-button"
                      onClick={() => {
                        setCsvRowsDraft([
                          ...csvRowsDraft,
                          new Array(Math.max(1, csvHeadersDraft.length)).fill(""),
                        ]);
                      }}
                    >
                      + Add Row
                    </button>
                    <button
                      className="pill-button accent"
                      onClick={() =>
                        runStructMutation(nodePayload.node_id, "CSV", {
                          headers: csvHeadersDraft,
                          rows: csvRowsDraft,
                        })
                      }
                    >
                      Commit CSV Changes
                    </button>
                  </div>
                </div>
                <table className="struct-table">
                  <thead>
                    <tr>
                      <th>#</th>
                      {csvHeadersDraft.map((h, hi) => (
                        <th key={hi}>
                          <input
                            type="text"
                            size={1}
                            className="field-box mono"
                            style={{ width: "100%", height: 24, fontWeight: 600 }}
                            value={h}
                            onChange={(e) => {
                              const next = [...csvHeadersDraft];
                              next[hi] = e.target.value;
                              setCsvHeadersDraft(next);
                            }}
                          />
                        </th>
                      ))}
                      <th style={{ width: 65 }}>Del</th>
                    </tr>
                  </thead>
                  <tbody>
                    {csvRowsDraft.map((row, ri) => (
                      <tr key={ri}>
                        <td className="mono">#{ri + 1}</td>
                        {csvHeadersDraft.map((_, ci) => (
                          <td key={ci}>
                            <input
                              type="text"
                              size={1}
                              className="field-box mono"
                              style={{ width: "100%", height: 24 }}
                              value={row[ci] ?? ""}
                              onChange={(e) => {
                                const next = csvRowsDraft.map((r, idx) => {
                                  if (idx !== ri) return r;
                                  const rowCopy = [...r];
                                  while (rowCopy.length <= ci) rowCopy.push("");
                                  rowCopy[ci] = e.target.value;
                                  return rowCopy;
                                });
                                setCsvRowsDraft(next);
                              }}
                            />
                          </td>
                        ))}
                        <td>
                          <button
                            className="pill-button"
                            style={{ height: 23, padding: "0 8px" }}
                            onClick={() => {
                              const next = [...csvRowsDraft];
                              next.splice(ri, 1);
                              setCsvRowsDraft(next);
                            }}
                          >
                            ✕
                          </button>
                        </td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            ) : nodePayload?.view_type === "EST" && centerTab === "primary" && estDraft ? (
              <div className="struct-table-container">
                <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", marginBottom: 10 }}>
                  <span style={{ fontWeight: 600, color: "var(--brush-heading)" }}>
                    {nodePayload.spec_line}
                  </span>
                  <button
                    className="pill-button accent"
                    onClick={() => runStructMutation(nodePayload.node_id, "EST", estDraft)}
                  >
                    Commit EST Particle Changes
                  </button>
                </div>
                <table className="struct-table">
                  <thead>
                    <tr>
                      <th style={{ width: 55 }}>#</th>
                      <th style={{ width: 85 }}>Tag</th>
                      <th style={{ width: 85 }}>Flag</th>
                      <th style={{ width: 85 }}>Bone</th>
                      <th>Offset (X, Y, Z)</th>
                      <th style={{ width: 95 }}>Scale</th>
                      <th>RGBA Color / Intensity</th>
                    </tr>
                  </thead>
                  <tbody>
                    {(estDraft.records as any[]).map((rec, ri) => (
                      <tr key={ri}>
                        <td className="mono">#{rec.index}</td>
                        <td className="mono" style={{ fontWeight: 600 }}>
                          {rec.group_tag}
                        </td>
                        <td>
                          <input
                            type="number"
                            className="field-box mono"
                            style={{ width: 70, height: 24 }}
                            value={rec.enabled_flag}
                            onChange={(e) => {
                              const copy = structuredClone(estDraft);
                              copy.records[ri].enabled_flag = parseInt(e.target.value || "0", 10);
                              setEstDraft(copy);
                            }}
                          />
                        </td>
                        <td>
                          <input
                            type="number"
                            className="field-box mono"
                            style={{ width: 70, height: 24 }}
                            value={rec.attach_bone}
                            onChange={(e) => {
                              const copy = structuredClone(estDraft);
                              copy.records[ri].attach_bone = parseInt(e.target.value || "0", 10);
                              setEstDraft(copy);
                            }}
                          />
                        </td>
                        <td>
                          <div style={{ display: "grid", gridTemplateColumns: "repeat(3, 1fr)", gap: 4 }}>
                            {(["x", "y", "z"] as const).map((axis) => (
                              <input
                                key={axis}
                                type="number"
                                step="0.05"
                                className="field-box mono"
                                style={{ height: 24 }}
                                value={Number((rec.offset?.[axis] ?? 0).toFixed(3))}
                                onChange={(e) => {
                                  const copy = structuredClone(estDraft);
                                  copy.records[ri].offset[axis] = parseFloat(e.target.value || "0");
                                  setEstDraft(copy);
                                }}
                              />
                            ))}
                          </div>
                        </td>
                        <td>
                          <input
                            type="number"
                            step="0.05"
                            className="field-box mono"
                            style={{ width: 85, height: 24 }}
                            value={Number((rec.scale ?? 1).toFixed(3))}
                            onChange={(e) => {
                              const copy = structuredClone(estDraft);
                              copy.records[ri].scale = parseFloat(e.target.value || "1");
                              setEstDraft(copy);
                            }}
                          />
                        </td>
                        <td>
                          <div style={{ display: "flex", alignItems: "center", gap: 6 }}>
                            <input
                              type="color"
                              value={rgbFloatToHex(rec.color_rgba ?? [1, 1, 1, 1])}
                              onChange={(e) => {
                                const [r, g, b] = hexToRgbFloat(e.target.value);
                                const copy = structuredClone(estDraft);
                                copy.records[ri].color_rgba[0] = r;
                                copy.records[ri].color_rgba[1] = g;
                                copy.records[ri].color_rgba[2] = b;
                                setEstDraft(copy);
                              }}
                              style={{
                                width: 24,
                                height: 24,
                                padding: 0,
                                border: "1px solid var(--brush-border)",
                                borderRadius: 4,
                                cursor: "pointer",
                              }}
                            />
                            <div style={{ display: "grid", gridTemplateColumns: "repeat(4, 1fr)", gap: 4, flex: 1 }}>
                              {[0, 1, 2, 3].map((ci) => (
                                <input
                                  key={ci}
                                  type="number"
                                  step="0.05"
                                  className="field-box mono"
                                  style={{ height: 24 }}
                                  value={Number((rec.color_rgba?.[ci] ?? 1).toFixed(3))}
                                  onChange={(e) => {
                                    const copy = structuredClone(estDraft);
                                    copy.records[ri].color_rgba[ci] = parseFloat(e.target.value || "0");
                                    setEstDraft(copy);
                                  }}
                                />
                              ))}
                            </div>
                          </div>
                        </td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            ) : nodePayload?.view_type === "CLOTH" && centerTab === "primary" ? (
              <div
                className="struct-table-container"
                onContextMenu={(e) => {
                  e.preventDefault();
                  e.stopPropagation();
                  setHoverSubmenuIdx(null);
                  setContextMenu({
                    x: Math.min(e.clientX, window.innerWidth - 250),
                    y: Math.min(e.clientY, window.innerHeight - 240),
                    items: buildAssetNodeContextMenu(nodePayload.node_id, nodePayload.cloth_kind),
                  });
                }}
              >
                <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", marginBottom: 10 }}>
                  <span style={{ fontWeight: 600, color: "var(--brush-heading)" }}>
                    {nodePayload.cloth_kind} Cloth Physics Nodes
                  </span>
                  <button
                    className="pill-button accent"
                    onClick={() =>
                      runClothMutation(
                        nodePayload.node_id,
                        "update_values",
                        undefined,
                        nodePayload.data
                      )
                    }
                  >
                    Commit {nodePayload.cloth_kind} Changes
                  </button>
                </div>
                {nodePayload.cloth_kind === "CLP" && Array.isArray(nodePayload.data?.cloth) ? (
                  <table className="struct-table">
                    <thead>
                      <tr>
                        <th>#</th>
                        <th>no</th>
                        <th>no_up</th>
                        <th>no_down</th>
                        <th>no_side</th>
                        <th>no_poly</th>
                        <th>no_fix</th>
                        <th>rot_limit</th>
                        <th>offset.x</th>
                        <th>offset.y</th>
                        <th>offset.z</th>
                      </tr>
                    </thead>
                    <tbody>
                      {(nodePayload.data.cloth as any[]).map((row, ri) => (
                        <tr key={ri}>
                          <td className="mono">#{ri}</td>
                          {(["no", "no_up", "no_down", "no_side", "no_poly", "no_fix"] as const).map((fk) => (
                            <td key={fk}>
                              <input
                                type="number"
                                size={1}
                                className="field-box mono"
                                style={{ height: 24 }}
                                value={row[fk] ?? 0}
                                onChange={(e) => {
                                  const copy = structuredClone(nodePayload);
                                  copy.data.cloth[ri][fk] = parseInt(e.target.value || "0", 10);
                                  setNodePayload(copy);
                                }}
                              />
                            </td>
                          ))}
                          <td>
                            <input
                              type="number"
                              step="0.01"
                              size={1}
                              className="field-box mono"
                              style={{ height: 24 }}
                              value={Number((row.rot_limit ?? 0).toFixed(4))}
                              onChange={(e) => {
                                const copy = structuredClone(nodePayload);
                                copy.data.cloth[ri].rot_limit = parseFloat(e.target.value || "0");
                                setNodePayload(copy);
                              }}
                            />
                          </td>
                          {(["x", "y", "z"] as const).map((axis) => (
                            <td key={`off_${axis}`}>
                              <input
                                type="number"
                                step="0.001"
                                size={1}
                                className="field-box mono"
                                style={{ height: 24 }}
                                value={Number((row.offset?.[axis] ?? 0).toFixed(4))}
                                onChange={(e) => {
                                  const copy = structuredClone(nodePayload);
                                  if (!copy.data.cloth[ri].offset) {
                                    copy.data.cloth[ri].offset = { x: 0, y: 0, z: 0 };
                                  }
                                  copy.data.cloth[ri].offset[axis] = parseFloat(e.target.value || "0");
                                  setNodePayload(copy);
                                }}
                              />
                            </td>
                          ))}
                        </tr>
                      ))}
                    </tbody>
                  </table>
                ) : nodePayload.cloth_kind === "CLH" &&
                  Array.isArray(nodePayload.data?.entries ?? nodePayload.data?.cloth_wk) ? (
                  <table className="struct-table">
                    <thead>
                      <tr>
                        <th>#</th>
                        <th>Bone 1 (p1)</th>
                        <th>Bone 2 (p2)</th>
                        <th>Weight</th>
                        <th>Radius</th>
                        <th>P1.x</th>
                        <th>P1.y</th>
                        <th>P1.z</th>
                        <th>P2.x</th>
                        <th>P2.y</th>
                        <th>P2.z</th>
                      </tr>
                    </thead>
                    <tbody>
                      {((nodePayload.data.entries ?? nodePayload.data.cloth_wk) as any[]).map(
                        (row, ri) => {
                          const listKey = Array.isArray(nodePayload.data.entries)
                            ? "entries"
                            : "cloth_wk";
                          return (
                            <tr key={ri}>
                              <td className="mono">#{ri}</td>
                              <td>
                                <input
                                  type="number"
                                  size={1}
                                  className="field-box mono"
                                  style={{ height: 24 }}
                                  value={row.p1 ?? row.no1 ?? 0}
                                  onChange={(e) => {
                                    const copy = structuredClone(nodePayload);
                                    const v = parseInt(e.target.value || "0", 10);
                                    copy.data[listKey][ri].p1 = v;
                                    setNodePayload(copy);
                                  }}
                                />
                              </td>
                              <td>
                                <input
                                  type="number"
                                  size={1}
                                  className="field-box mono"
                                  style={{ height: 24 }}
                                  value={row.p2 ?? row.no2 ?? 0}
                                  onChange={(e) => {
                                    const copy = structuredClone(nodePayload);
                                    const v = parseInt(e.target.value || "0", 10);
                                    copy.data[listKey][ri].p2 = v;
                                    setNodePayload(copy);
                                  }}
                                />
                              </td>
                              <td>
                                <input
                                  type="number"
                                  step="0.01"
                                  size={1}
                                  className="field-box mono"
                                  style={{ height: 24 }}
                                  value={Number((row.weight ?? 0).toFixed(4))}
                                  onChange={(e) => {
                                    const copy = structuredClone(nodePayload);
                                    copy.data[listKey][ri].weight = parseFloat(e.target.value || "0");
                                    setNodePayload(copy);
                                  }}
                                />
                              </td>
                              <td>
                                <input
                                  type="number"
                                  step="0.01"
                                  size={1}
                                  className="field-box mono"
                                  style={{ height: 24 }}
                                  value={Number((row.radius ?? row.r ?? 0).toFixed(4))}
                                  onChange={(e) => {
                                    const copy = structuredClone(nodePayload);
                                    copy.data[listKey][ri].radius = parseFloat(e.target.value || "0");
                                    setNodePayload(copy);
                                  }}
                                />
                              </td>
                              {(["x", "y", "z"] as const).map((axis) => (
                                <td key={`o1_${axis}`}>
                                  <input
                                    type="number"
                                    step="0.001"
                                    size={1}
                                    className="field-box mono"
                                    style={{ height: 24 }}
                                    value={Number((row.offset1?.[axis] ?? 0).toFixed(4))}
                                    onChange={(e) => {
                                      const copy = structuredClone(nodePayload);
                                      if (!copy.data[listKey][ri].offset1) {
                                        copy.data[listKey][ri].offset1 = { x: 0, y: 0, z: 0 };
                                      }
                                      copy.data[listKey][ri].offset1[axis] = parseFloat(e.target.value || "0");
                                      setNodePayload(copy);
                                    }}
                                  />
                                </td>
                              ))}
                              {(["x", "y", "z"] as const).map((axis) => (
                                <td key={`o2_${axis}`}>
                                  <input
                                    type="number"
                                    step="0.001"
                                    size={1}
                                    className="field-box mono"
                                    style={{ height: 24 }}
                                    value={Number((row.offset2?.[axis] ?? 0).toFixed(4))}
                                    onChange={(e) => {
                                      const copy = structuredClone(nodePayload);
                                      if (!copy.data[listKey][ri].offset2) {
                                        copy.data[listKey][ri].offset2 = { x: 0, y: 0, z: 0 };
                                      }
                                      copy.data[listKey][ri].offset2[axis] = parseFloat(e.target.value || "0");
                                      setNodePayload(copy);
                                    }}
                                  />
                                </td>
                              ))}
                            </tr>
                          );
                        }
                      )}
                    </tbody>
                  </table>
                ) : Array.isArray(nodePayload.data?.entries ?? nodePayload.data?.cloth_wind_wk) ? (
                  <table className="struct-table">
                    <thead>
                      <tr>
                        <th>#</th>
                        <th>Wind Type</th>
                        <th>Parts No</th>
                        <th>Offset1.x</th>
                        <th>Offset1.y</th>
                        <th>Offset1.z</th>
                        <th>Offset2.x</th>
                        <th>Offset2.y</th>
                        <th>Offset2.z</th>
                        <th>Radius</th>
                        <th>Power</th>
                        <th>Timer</th>
                        <th>Swing Rate</th>
                        <th>Swing Spd</th>
                        <th>Delay Max</th>
                      </tr>
                    </thead>
                    <tbody>
                      {((nodePayload.data.entries ?? nodePayload.data.cloth_wind_wk) as any[]).map(
                        (row, ri) => {
                          const listKey = Array.isArray(nodePayload.data.entries)
                            ? "entries"
                            : "cloth_wind_wk";
                          return (
                            <tr key={ri}>
                              <td className="mono">#{ri}</td>
                              <td>
                                <input
                                  type="number"
                                  size={1}
                                  className="field-box mono"
                                  style={{ height: 24 }}
                                  value={row.wind_type ?? row.ent_type ?? 0}
                                  onChange={(e) => {
                                    const copy = structuredClone(nodePayload);
                                    copy.data[listKey][ri].wind_type = parseInt(e.target.value || "0", 10);
                                    setNodePayload(copy);
                                  }}
                                />
                              </td>
                              <td>
                                <input
                                  type="number"
                                  size={1}
                                  className="field-box mono"
                                  style={{ height: 24 }}
                                  value={row.parts_no ?? row.no ?? 0}
                                  onChange={(e) => {
                                    const copy = structuredClone(nodePayload);
                                    copy.data[listKey][ri].parts_no = parseInt(e.target.value || "0", 10);
                                    setNodePayload(copy);
                                  }}
                                />
                              </td>
                              {(["x", "y", "z"] as const).map((axis) => (
                                <td key={`w1_${axis}`}>
                                  <input
                                    type="number"
                                    step="0.001"
                                    size={1}
                                    className="field-box mono"
                                    style={{ height: 24 }}
                                    value={Number((row.offset?.[axis] ?? 0).toFixed(4))}
                                    onChange={(e) => {
                                      const copy = structuredClone(nodePayload);
                                      if (!copy.data[listKey][ri].offset) {
                                        copy.data[listKey][ri].offset = { x: 0, y: 0, z: 0 };
                                      }
                                      copy.data[listKey][ri].offset[axis] = parseFloat(e.target.value || "0");
                                      setNodePayload(copy);
                                    }}
                                  />
                                </td>
                              ))}
                              {(["x", "y", "z"] as const).map((axis) => (
                                <td key={`w2_${axis}`}>
                                  <input
                                    type="number"
                                    step="0.001"
                                    size={1}
                                    className="field-box mono"
                                    style={{ height: 24 }}
                                    value={Number((row.offset2?.[axis] ?? 0).toFixed(4))}
                                    onChange={(e) => {
                                      const copy = structuredClone(nodePayload);
                                      if (!copy.data[listKey][ri].offset2) {
                                        copy.data[listKey][ri].offset2 = { x: 0, y: 0, z: 0 };
                                      }
                                      copy.data[listKey][ri].offset2[axis] = parseFloat(e.target.value || "0");
                                      setNodePayload(copy);
                                    }}
                                  />
                                </td>
                              ))}
                              {(
                                [
                                  "radius",
                                  "power",
                                  "timer",
                                  "swing_rate",
                                  "swing_spd",
                                  "delay_max",
                                ] as const
                              ).map((fKey) => (
                                <td key={fKey}>
                                  <input
                                    type="number"
                                    step="0.01"
                                    size={1}
                                    className="field-box mono"
                                    style={{ height: 24 }}
                                    value={Number((row[fKey] ?? 0).toFixed(4))}
                                    onChange={(e) => {
                                      const copy = structuredClone(nodePayload);
                                      copy.data[listKey][ri][fKey] = parseFloat(e.target.value || "0");
                                      setNodePayload(copy);
                                    }}
                                  />
                                </td>
                              ))}
                            </tr>
                          );
                        }
                      )}
                    </tbody>
                  </table>
                ) : null}
              </div>
            ) : nodePayload?.view_type === "RAW_STRUCT" && centerTab === "primary" ? (
              <div className="struct-table-container">{renderRawStructCenter()}</div>
            ) : nodePayload?.view_type === "CONTAINER" && centerTab === "primary" ? (
              <div className="struct-table-container">
                {nodePayload.pkz_info?.uses_oodle && nodePayload.pkz_info?.oodle_fallback_active && (
                  <div className="validation-banner" style={{ marginBottom: 12 }}>
                    <span>✓</span>
                    <div style={{ flex: 1 }}>
                      Using built-in pure-Rust Oodle Kraken (<span className="mono">ooz</span>) fallback
                      decompressor. You can also optionally configure external{" "}
                      <span className="mono">oo2core_*_win64.dll</span>.
                    </div>
                    <button className="pill-button" onClick={handlePickOodleDll}>
                      Locate Oodle DLL...
                    </button>
                  </div>
                )}

                <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", flexWrap: "wrap", gap: 10, marginBottom: 10 }}>
                  <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
                    <span style={{ fontWeight: 600, color: "var(--brush-heading)" }}>
                      {nodePayload.title} ({nodePayload.format} · {formatBytes(nodePayload.size_bytes)})
                    </span>
                    {nodePayload.folder_prefix && (
                      <button
                        className="pill-button"
                        style={{ height: 24, padding: "0 8px", fontSize: 11 }}
                        onClick={() => {
                          const rootId = nodePayload.node_id.includes("/pkzfolder:")
                            ? nodePayload.node_id.split("/pkzfolder:")[0]
                            : "root";
                          selectAndInspectNode(rootId);
                        }}
                      >
                        ✕ Clear Folder Filter ({nodePayload.folder_prefix}/)
                      </button>
                    )}
                  </div>
                  <div style={{ display: "flex", alignItems: "center", gap: 8, flexWrap: "wrap" }}>
                    {nodePayload.format === "PKZ" && (
                      <div style={{ display: "inline-flex", alignItems: "center", gap: 6 }}>
                        <span style={{ color: "var(--brush-muted)", fontSize: 11.5 }}>Set All Codec:</span>
                        <select
                          className="pill-select mono"
                          style={{ height: 28, fontSize: 11.5 }}
                          value={pkzBulkCodec}
                          onChange={(e) => setPkzBulkCodec(e.target.value)}
                        >
                          <option value="ZStandard">ZStandard</option>
                          <option value="OodleKraken">OodleKraken</option>
                          <option value="None">None (Uncompressed)</option>
                        </select>
                        <button
                          className="pill-button"
                          style={{ height: 28 }}
                          onClick={() =>
                            runContainerMutation(
                              nodePayload.node_id,
                              "set_pkz_compression",
                              pkzBulkCodec,
                              { compression: pkzBulkCodec }
                            )
                          }
                        >
                          Apply All
                        </button>
                      </div>
                    )}
                    <button
                      className="pill-button"
                      onClick={async () => {
                        const paths = await invoke<string[]>("pick_multiple_files_dialog", {
                          title: "Add File(s) to Container",
                          extensions: null,
                        });
                        if (paths && paths.length > 0) {
                          await runContainerMutation(nodePayload.node_id, "add_files", "", {
                            paths,
                            folder_prefix: nodePayload.folder_prefix ?? null,
                          });
                        }
                      }}
                    >
                      + Add File(s)...
                    </button>
                    <button
                      className="pill-button accent"
                      onClick={async () => {
                        const folder = await invoke<string | null>("pick_folder_dialog", {
                          title: "Extract All Entries to Folder",
                        });
                        if (folder) {
                          await runContainerMutation(nodePayload.node_id, "extract_all", folder);
                          setStatusLeft(`Extracted all entries to ${folder}`);
                          loadDirectory(currentDirRef.current);
                        }
                      }}
                    >
                      Extract All...
                    </button>
                  </div>
                </div>

                {Array.isArray(nodePayload.entries) && nodePayload.entries.length > 0 ? (
                  <table className="struct-table">
                    <thead>
                      <tr>
                        <th style={{ width: 55 }}>#</th>
                        <th>Entry Name</th>
                        <th>Type / Compression Codec</th>
                        <th style={{ textAlign: "right" }}>Size</th>
                        <th style={{ width: 170, textAlign: "right" }}>Actions</th>
                      </tr>
                    </thead>
                    <tbody>
                      {nodePayload.entries
                        .filter((e: any) => {
                          if (!nodePayload.folder_prefix) return true;
                          const norm = String(e.name || "").replace(/\\/g, "/").replace(/^\/+/, "");
                          const fp = String(nodePayload.folder_prefix).replace(/^\/+|\/+$/g, "");
                          return norm === fp || norm.startsWith(`${fp}/`);
                        })
                        .map((e: any, ei: number) => {
                        const baseContainerId = nodePayload.node_id.includes("/pkzfolder:")
                          ? nodePayload.node_id.split("/pkzfolder:")[0]
                          : nodePayload.node_id.startsWith("pkzfolder:")
                          ? "root"
                          : nodePayload.node_id;
                        const prefix =
                          baseContainerId === "root" || !baseContainerId
                            ? ""
                            : `${baseContainerId}/`;
                        const childNodeId =
                          nodePayload.format === "PKZ"
                            ? `${prefix}pkz:${e.index}`
                            : nodePayload.format === "EFF"
                            ? `${prefix}effdir:${e.dir_index}/efffile:${e.index}`
                            : `${prefix}dat:${e.index}`;
                        return (
                          <tr key={ei}>
                            <td className="mono">#{e.index}</td>
                            <td className="mono" style={{ fontWeight: 600 }}>
                              {e.dir_name ? `${e.dir_name} / ${e.name}` : e.name}
                            </td>
                            <td className="mono" style={{ color: "var(--brush-muted)" }}>
                              {nodePayload.format === "PKZ" ? (
                                <select
                                  className="pill-select mono"
                                  style={{ height: 25, fontSize: 11.5 }}
                                  value={
                                    String(e.compression || "").toLowerCase().includes("oodle")
                                      ? "OodleKraken"
                                      : String(e.compression || "").toLowerCase().includes("zst")
                                      ? "ZStandard"
                                      : "None"
                                  }
                                  onChange={(ev) => {
                                    const nextCodec = ev.target.value;
                                    runContainerMutation(
                                      baseContainerId,
                                      "set_pkz_compression",
                                      nextCodec,
                                      {
                                        compression: nextCodec,
                                        entry_indices: [e.index],
                                      }
                                    );
                                  }}
                                >
                                  <option value="ZStandard">ZStandard</option>
                                  <option value="OodleKraken">OodleKraken</option>
                                  <option value="None">None</option>
                                </select>
                              ) : (
                                e.compression || e.extension || "BIN"
                              )}
                            </td>
                            <td className="mono" style={{ textAlign: "right" }}>
                              {formatBytes(e.size ?? 0)}
                              {e.compressed_size !== undefined && e.compressed_size !== e.size
                                ? ` (${formatBytes(e.compressed_size)} comp)`
                                : ""}
                            </td>
                            <td style={{ textAlign: "right" }}>
                              <div style={{ display: "inline-flex", gap: 4 }}>
                                <button
                                  className="pill-button"
                                  style={{ height: 23, padding: "0 8px", fontSize: 11 }}
                                  onClick={() => selectAndInspectNode(childNodeId)}
                                >
                                  Open
                                </button>
                                <button
                                  className="pill-button"
                                  style={{ height: 23, padding: "0 7px", fontSize: 11 }}
                                  onClick={() => runContainerMutation(childNodeId, "remove_entry")}
                                  title="Delete Entry from Container"
                                >
                                  ✕
                                </button>
                              </div>
                            </td>
                          </tr>
                        );
                      })}
                    </tbody>
                  </table>
                ) : (
                  <pre className="mono" style={{ margin: 0, lineHeight: 1.55 }}>
                    {nodePayload.hex_preview}
                  </pre>
                )}
              </div>
            ) : nodePayload ? (
              <div className="struct-table-container">
                {nodePayload.hex_preview ? (
                  <pre className="mono" style={{ margin: 0, lineHeight: 1.55 }}>
                    {nodePayload.hex_preview}
                  </pre>
                ) : (
                  <pre className="mono" style={{ margin: 0, lineHeight: 1.45 }}>
                    {JSON.stringify(nodePayload.data ?? nodePayload, null, 2)}
                  </pre>
                )}
              </div>
            ) : (
              <div className="checkerboard-canvas">
                <div className="viewport-empty">
                  <IconImage size={44} />
                  <div className="viewport-empty-title">No preview</div>
                  <div className="viewport-empty-note">
                    Open a container or select an item from the Asset Browser.
                  </div>
                </div>
              </div>
            )}
          </div>
        </div>

        {/* Right Vertical Splitter */}
        <div
          className={`splitter-v ${draggingSplit === "right" ? "dragging" : ""}`}
          onMouseDown={(e) => {
            e.preventDefault();
            setDraggingSplit("right");
          }}
          onDoubleClick={() => setRightWidth(380)}
          title="Drag to resize, double-click to reset"
        >
          <div className="splitter-v-grip" />
        </div>

        {/* RIGHT COLUMN: Contextual Inspector */}
        <div className="right-column" style={{ width: rightWidth }}>
          {activeTexture ? (
            <div>
              <div className="inspector-header">
                <div className="inspector-icon-box">
                  <IconImage size={20} />
                </div>
                <div>
                  <div className="inspector-title">{activeTexture.title}</div>
                  <div className="inspector-subtitle">{activeTexture.breadcrumb}</div>
                </div>
              </div>

              <div className="inspector-spec">{activeTexture.spec_line}</div>

              <div className={`validation-banner ${activeTexture.validation_ok ? "" : "warn"}`}>
                <span>{activeTexture.validation_ok ? "✓" : "⚠"}</span>
                <span>{activeTexture.validation_msg}</span>
              </div>

              {/* Card 1: # Hash */}
              <div className="inspector-card">
                <button className="card-header-toggle" onClick={() => toggleCard("hash")}>
                  <span className="card-header-left">
                    <span style={{ color: "var(--brush-muted)", fontWeight: 700 }}>#</span>
                    <span>Hash &amp; Game Profile</span>
                  </span>
                  <IconChevronDown collapsed={!openCards.hash} />
                </button>
                {openCards.hash && (
                  <div className="card-body">
                    <div className="inspector-row" style={{ marginBottom: 6 }}>
                      <span className="inspector-row-label">Profile</span>
                      <span className="mono" style={{ fontSize: 11.5 }}>
                        {activeTexture.game_profile}
                      </span>
                    </div>
                    <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
                      <span style={{ color: "var(--brush-muted)", whiteSpace: "nowrap" }}>
                        CRC32 Hash
                      </span>
                      <input
                        type="text"
                        className="field-box mono"
                        style={{ flex: 1, minWidth: 0 }}
                        value={hashDraft}
                        onChange={(e) => setHashDraft(e.target.value)}
                        onKeyDown={(e) => {
                          if (e.key === "Enter") {
                            runTextureMutation(activeTexture.node_id, "set_hash", { hash: hashDraft });
                          }
                        }}
                        onBlur={() => {
                          if (hashDraft !== activeTexture.texture_id_hex) {
                            runTextureMutation(activeTexture.node_id, "set_hash", { hash: hashDraft });
                          }
                        }}
                      />
                      <button
                        className="pill-button"
                        onClick={() => {
                          const inputName = window.prompt(
                            "Enter texture name to compute crc32(name) & 0x7FFFFFFF:",
                            `tex${activeTexture.index}`
                          );
                          if (inputName) {
                            runTextureMutation(activeTexture.node_id, "calc_crc32", {
                              name: inputName,
                            });
                          }
                        }}
                      >
                        <IconRefresh />
                        <span>Calc</span>
                      </button>
                    </div>
                  </div>
                )}
              </div>

              {/* Card 2: Game-Aware Flags */}
              <div className="inspector-card">
                <button className="card-header-toggle" onClick={() => toggleCard("flags")}>
                  <span className="card-header-left">
                    <span style={{ color: "var(--brush-muted)" }}>⚑</span>
                    <span>
                      {activeTexture.has_confirmed_xt1_flags
                        ? "XT1 Flags (table2)"
                        : "WTB Entry Flags"}
                    </span>
                  </span>
                  <IconChevronDown collapsed={!openCards.flags} />
                </button>
                {openCards.flags && (
                  <div className="card-body">
                    {activeTexture.has_confirmed_xt1_flags ? (
                      <>
                        <div className="inspector-row">
                          <span className="inspector-row-label">Hash</span>
                          <div>
                            <div
                              className={`pill-checkbox ${
                                activeTexture.has_crc32_flag ? "checked" : ""
                              }`}
                            >
                              <span className="pill-checkbox-box">
                                {activeTexture.has_crc32_flag && <IconCheck />}
                              </span>
                              <span>Has CRC32 hash (0x80000000)</span>
                            </div>
                          </div>
                        </div>

                        <div className="inspector-row">
                          <span className="inspector-row-label">Color</span>
                          <div>
                            <div
                              className={`pill-checkbox ${
                                activeTexture.srgb_flag ? "checked" : ""
                              }`}
                            >
                              <span className="pill-checkbox-box">
                                {activeTexture.srgb_flag && <IconCheck />}
                              </span>
                              <span>sRGB color space (0x02000000)</span>
                            </div>
                          </div>
                        </div>

                        <div className="inspector-row">
                          <span className="inspector-row-label">Usage</span>
                          <div>
                            <div
                              className={`pill-checkbox ${
                                activeTexture.cubemap_flag ? "checked" : ""
                              }`}
                            >
                              <span className="pill-checkbox-box">
                                {activeTexture.cubemap_flag && <IconCheck />}
                              </span>
                              <span>Cube map usage (0x00000200)</span>
                            </div>
                          </div>
                        </div>
                      </>
                    ) : (
                      <div style={{ color: "var(--brush-muted)", fontSize: 11.5, marginBottom: 6 }}>
                        Raw WTB flags for {activeTexture.game_profile} (XT1 bitmasks hidden for non-XT1 banks):
                      </div>
                    )}

                    <div className="inspector-row">
                      <span className="inspector-row-label">Flags Hex</span>
                      <input
                        type="text"
                        className="field-box mono"
                        value={flagsDraft}
                        onChange={(e) => setFlagsDraft(e.target.value)}
                        onKeyDown={(e) => {
                          if (e.key === "Enter") {
                            runTextureMutation(activeTexture.node_id, "set_flags", {
                              flags_hex: flagsDraft,
                            });
                          }
                        }}
                        onBlur={() => {
                          if (flagsDraft !== activeTexture.flags_hex) {
                            runTextureMutation(activeTexture.node_id, "set_flags", {
                              flags_hex: flagsDraft,
                            });
                          }
                        }}
                      />
                    </div>
                    <div className="mono" style={{ color: "var(--brush-muted)", marginTop: 4 }}>
                      LE bytes = {activeTexture.table2_bytes_str}
                    </div>
                  </div>
                )}
              </div>

              {/* Card 3: Custom Texture Import & Encoding */}
              <div className="inspector-card">
                <button className="card-header-toggle" onClick={() => toggleCard("encoding")}>
                  <span className="card-header-left">
                    <span style={{ color: "var(--brush-muted)" }}>⇄</span>
                    <span>Import &amp; Encoding</span>
                  </span>
                  <IconChevronDown collapsed={!openCards.encoding} />
                </button>
                {openCards.encoding && (
                  <div className="card-body">
                    <div className="inspector-row">
                      <span className="inspector-row-label">Format</span>
                      <select
                        className="pill-select mono"
                        style={{ width: "100%", fontSize: 11.5 }}
                        value={texEncodeFormat}
                        onChange={(e) => setTexEncodeFormat(e.target.value)}
                      >
                        {ALL_TEXTURE_ENCODE_FORMATS.map((grp) => (
                          <optgroup key={grp.group} label={grp.group}>
                            {grp.options.map((opt) => (
                              <option key={opt.value} value={opt.value}>
                                {opt.label}
                              </option>
                            ))}
                          </optgroup>
                        ))}
                      </select>
                    </div>

                    <div className="inspector-row">
                      <span className="inspector-row-label">Mipmaps</span>
                      <div
                        className={`pill-checkbox ${texGenerateMips ? "checked" : ""}`}
                        onClick={() => setTexGenerateMips(!texGenerateMips)}
                      >
                        <span className="pill-checkbox-box">{texGenerateMips && <IconCheck />}</span>
                        <span>Generate full mipmap chain</span>
                      </div>
                    </div>

                    <div className="inspector-row">
                      <span className="inspector-row-label">Solidify</span>
                      <div
                        className={`pill-checkbox ${texSolidify ? "checked" : ""}`}
                        onClick={() => setTexSolidify(!texSolidify)}
                        title="Multi-source BFS alpha edge bleed (prevents dark fringes on transparent cutouts)"
                      >
                        <span className="pill-checkbox-box">{texSolidify && <IconCheck />}</span>
                        <span>Solidify alpha edges (BFS bleed)</span>
                      </div>
                    </div>

                    <div className="inspector-row">
                      <span className="inspector-row-label">Alpha</span>
                      <div
                        className={`pill-checkbox ${texMakeOpaque ? "checked" : ""}`}
                        onClick={() => setTexMakeOpaque(!texMakeOpaque)}
                      >
                        <span className="pill-checkbox-box">{texMakeOpaque && <IconCheck />}</span>
                        <span>Force opaque alpha (A = 255)</span>
                      </div>
                    </div>

                    <div style={{ display: "flex", gap: 6, marginTop: 6 }}>
                      <button
                        className="pill-button accent"
                        style={{ flex: 1 }}
                        onClick={() => promptAndRunTextureImport(activeTexture.node_id, "replace_file")}
                      >
                        Replace Texture...
                      </button>
                      <button
                        className="pill-button"
                        onClick={() => handleTextureExport(activeTexture.node_id, "single", "png")}
                      >
                        Export PNG
                      </button>
                    </div>
                  </div>
                )}
              </div>
            </div>
          ) : activeWmb ? (
            <div>
              <div className="inspector-header">
                <div className="inspector-icon-box">
                  <IconCube size={20} />
                </div>
                <div>
                  <div className="inspector-title">{activeWmb.title}</div>
                  <div className="inspector-subtitle">{activeWmb.breadcrumb}</div>
                </div>
              </div>

              <div className="inspector-spec">{activeWmb.spec_line}</div>

              <div className="validation-banner">
                <span>✓</span>
                <span>{activeWmb.validation_msg}</span>
              </div>

              <div className="inspector-card">
                <button className="card-header-toggle" onClick={() => toggleCard("wmb_props")}>
                  <span className="card-header-left">
                    <span style={{ color: "var(--brush-muted)" }}>#</span>
                    <span>WMB Model Operations</span>
                  </span>
                  <IconChevronDown collapsed={!openCards.wmb_props} />
                </button>
                {openCards.wmb_props && (
                  <div className="card-body">
                    <div className="inspector-row">
                      <span className="inspector-row-label">Format</span>
                      <span className="mono">
                        {activeWmb.is_wmb3
                          ? "WMB3 (Bayonetta 3 / NieR)"
                          : (wmbB2Override ?? activeWmb.is_bayo2)
                          ? "WMB0 (Bayonetta 2)"
                          : "WMB0 (Bayonetta 1)"}
                      </span>
                    </div>
                    <div className="inspector-row">
                      <span className="inspector-row-label">Endianness</span>
                      <span className="mono">{activeWmb.endian}</span>
                    </div>
                    {wmbMatDraft && (
                      <>
                        <div className="inspector-row">
                          <span className="inspector-row-label">Shader</span>
                          <span className="mono">{wmbMatDraft.shader_name ?? "Custom"}</span>
                        </div>
                        <div className="inspector-row">
                          <span className="inspector-row-label">Material Type</span>
                          <input
                            type="text"
                            className="field-box mono"
                            style={{ height: 25 }}
                            value={`0x${wmbMatDraft.mat_type.toString(16).toUpperCase()}`}
                            onChange={(e) => {
                              const clean = e.target.value.replace(/^0x/i, "");
                              const parsed = parseInt(clean || "0", 16);
                              if (!Number.isNaN(parsed)) {
                                setWmbMatDraft({
                                  ...wmbMatDraft,
                                  mat_type: parsed,
                                });
                              }
                            }}
                          />
                        </div>
                        <div className="inspector-row">
                          <span className="inspector-row-label">Flags (u16)</span>
                          <input
                            type="text"
                            className="field-box mono"
                            style={{ height: 25 }}
                            value={`0x${wmbMatDraft.flag.toString(16).toUpperCase()}`}
                            onChange={(e) => {
                              const clean = e.target.value.replace(/^0x/i, "");
                              const parsed = parseInt(clean || "0", 16);
                              if (!Number.isNaN(parsed)) {
                                setWmbMatDraft({
                                  ...wmbMatDraft,
                                  flag: parsed,
                                });
                              }
                            }}
                          />
                        </div>
                        <button
                          className="pill-button accent"
                          style={{ marginTop: 6 }}
                          onClick={() =>
                            runWmbMutation(activeWmb.node_id, "update_material", wmbMatDraft)
                          }
                        >
                          <span>Apply Material Changes</span>
                        </button>
                      </>
                    )}
                    <div style={{ display: "flex", flexWrap: "wrap", gap: 6, marginTop: 8 }}>
                      <button
                        className="pill-button"
                        onClick={async () => {
                          const dest = await invoke<string | null>("save_file_dialog", {
                            title: "Export WMB Model to Wavefront .OBJ",
                            defaultName: "model.obj",
                          });
                          if (dest) await runWmbMutation(activeWmb.node_id, "export_obj", { path: dest });
                        }}
                      >
                        Export .OBJ...
                      </button>
                      {activeWmb.is_wmb3 ? (
                        <button
                          className="pill-button"
                          onClick={() => runWmbMutation(activeWmb.node_id, "set_tpose")}
                        >
                          Bake T-Pose
                        </button>
                      ) : (
                        <>
                          <button
                            className="pill-button"
                            onClick={() => runWmbMutation(activeWmb.node_id, "cleanup_vertexes")}
                          >
                            Cleanup Verts
                          </button>
                          <button
                            className="pill-button"
                            onClick={() => runWmbMutation(activeWmb.node_id, "cleanup_materials")}
                          >
                            Cleanup Mats
                          </button>
                          <button
                            className="pill-button"
                            onClick={() => runWmbMutation(activeWmb.node_id, "swap_endian")}
                          >
                            Swap Endian
                          </button>
                        </>
                      )}
                    </div>
                  </div>
                )}
              </div>
            </div>
          ) : nodePayload?.view_type === "BXM" ? (
            <div>
              <div className="inspector-header">
                <div className="inspector-icon-box">
                  <IconCode size={20} />
                </div>
                <div>
                  <div className="inspector-title">{nodePayload.title}</div>
                  <div className="inspector-subtitle">
                    Binary XML ({nodePayload.tag}) · {nodePayload.size_bytes} bytes
                  </div>
                </div>
              </div>

              <div className="validation-banner">
                <span>✓</span>
                <span>Decoded cleanly — string table and node offsets are recalculated on commit.</span>
              </div>

              {(() => {
                const selNode = getBxmNodeByPath(bxmTreeDraft, selectedBxmPath);
                if (!selNode) return null;
                return (
                  <div className="inspector-card">
                    <button className="card-header-toggle" onClick={() => toggleCard("bxm_node")}>
                      <span className="card-header-left">
                        <span style={{ color: "var(--brush-muted)" }}>&lt;/&gt;</span>
                        <span>Selected Element: &lt;{selNode.name}&gt;</span>
                      </span>
                      <IconChevronDown collapsed={!openCards.bxm_node} />
                    </button>
                    {openCards.bxm_node && (
                      <div className="card-body">
                        <div className="inspector-row">
                          <span className="inspector-row-label">Tag Name</span>
                          <input
                            type="text"
                            className="field-box mono"
                            value={selNode.name}
                            onChange={(e) => {
                              if (!bxmTreeDraft) return;
                              const copy = structuredClone(bxmTreeDraft);
                              const t = getBxmNodeByPath(copy, selectedBxmPath);
                              if (t) {
                                t.name = e.target.value;
                                updateBxmDraftTree(copy);
                              }
                            }}
                          />
                        </div>
                        <div className="inspector-row">
                          <span className="inspector-row-label">Text Value</span>
                          <input
                            type="text"
                            className="field-box mono"
                            placeholder="(empty)"
                            value={selNode.value ?? ""}
                            onChange={(e) => {
                              if (!bxmTreeDraft) return;
                              const copy = structuredClone(bxmTreeDraft);
                              const t = getBxmNodeByPath(copy, selectedBxmPath);
                              if (t) {
                                t.value = e.target.value.length > 0 ? e.target.value : null;
                                updateBxmDraftTree(copy);
                              }
                            }}
                          />
                        </div>

                        <div
                          style={{
                            display: "flex",
                            justifyContent: "space-between",
                            alignItems: "center",
                            marginTop: 6,
                          }}
                        >
                          <span style={{ color: "var(--brush-muted)", fontWeight: 600 }}>
                            @Attributes ({selNode.attributes.length})
                          </span>
                          <button
                            className="pill-button"
                            style={{ height: 23, padding: "0 8px", fontSize: 11 }}
                            onClick={() => {
                              if (!bxmTreeDraft) return;
                              const copy = structuredClone(bxmTreeDraft);
                              const t = getBxmNodeByPath(copy, selectedBxmPath);
                              if (t) {
                                t.attributes.push([`attr${t.attributes.length}`, "0"]);
                                updateBxmDraftTree(copy);
                              }
                            }}
                          >
                            + Add @Attr
                          </button>
                        </div>

                        {selNode.attributes.map(([k, v], i) => (
                          <div
                            key={i}
                            style={{ display: "grid", gridTemplateColumns: "95px 1fr 28px", gap: 6 }}
                          >
                            <input
                              type="text"
                              className="field-box mono"
                              style={{ height: 25 }}
                              value={k}
                              onChange={(e) => {
                                if (!bxmTreeDraft) return;
                                const copy = structuredClone(bxmTreeDraft);
                                const t = getBxmNodeByPath(copy, selectedBxmPath);
                                if (t && t.attributes[i]) {
                                  t.attributes[i][0] = e.target.value;
                                  updateBxmDraftTree(copy);
                                }
                              }}
                            />
                            <input
                              type="text"
                              className="field-box mono"
                              style={{ height: 25 }}
                              value={v}
                              onChange={(e) => {
                                if (!bxmTreeDraft) return;
                                const copy = structuredClone(bxmTreeDraft);
                                const t = getBxmNodeByPath(copy, selectedBxmPath);
                                if (t && t.attributes[i]) {
                                  t.attributes[i][1] = e.target.value;
                                  updateBxmDraftTree(copy);
                                }
                              }}
                            />
                            <button
                              className="pill-button"
                              style={{ height: 25, padding: 0 }}
                              title="Remove attribute"
                              onClick={() => {
                                if (!bxmTreeDraft) return;
                                const copy = structuredClone(bxmTreeDraft);
                                const t = getBxmNodeByPath(copy, selectedBxmPath);
                                if (t && t.attributes[i]) {
                                  t.attributes.splice(i, 1);
                                  updateBxmDraftTree(copy);
                                }
                              }}
                            >
                              ✕
                            </button>
                          </div>
                        ))}
                      </div>
                    )}
                  </div>
                );
              })()}

              <div className="inspector-card">
                <div className="card-body" style={{ paddingTop: 14 }}>
                  <button className="pill-button accent" onClick={() => commitBxmDraft()}>
                    <span>Commit XML Changes to Binary BXM</span>
                  </button>
                </div>
              </div>
            </div>
          ) : nodePayload?.view_type === "CLOTH" ? (
            <div>
              <div className="inspector-header">
                <div className="inspector-icon-box">
                  <IconCube size={20} />
                </div>
                <div>
                  <div className="inspector-title">{nodePayload.title}</div>
                  <div className="inspector-subtitle">
                    {nodePayload.cloth_kind} Physics · {nodePayload.endian} Endian
                  </div>
                </div>
              </div>

              <div className="validation-banner">
                <span>✓</span>
                <span>Right-click the cloth node or table to remap bones or swap Wii U ↔ PC endianness.</span>
              </div>

              {nodePayload.cloth_kind === "CLP" && nodePayload.data?.header && (
                <div className="inspector-card">
                  <button className="card-header-toggle" onClick={() => toggleCard("cloth_header")}>
                    <span className="card-header-left">
                      <span style={{ color: "var(--brush-muted)" }}>⚙</span>
                      <span>Spring, Gravity &amp; Wind Header</span>
                    </span>
                    <IconChevronDown collapsed={!openCards.cloth_header} />
                  </button>
                  {openCards.cloth_header && (
                    <div className="card-body">
                      {(
                        [
                          ["limit_spring_rate", "Spring Rate"],
                          ["spd_rate", "Speed Rate"],
                          ["stretchy", "Stretchy"],
                          ["thick", "Thickness"],
                          ["first_bundle_rate", "First Bundle"],
                          ["wind_sin", "Wind Sine"],
                          ["hit_adjust_rate", "Hit Adjust"],
                        ] as [string, string][]
                      ).map(([fieldKey, label]) => (
                        <div key={fieldKey} className="inspector-row">
                          <span className="inspector-row-label">{label}</span>
                          <input
                            type="number"
                            step="0.01"
                            className="field-box mono"
                            value={nodePayload.data.header[fieldKey]}
                            onChange={(e) => {
                              const v = parseFloat(e.target.value);
                              if (!Number.isNaN(v)) {
                                const copy = structuredClone(nodePayload);
                                copy.data.header[fieldKey] = v;
                                setNodePayload(copy);
                              }
                            }}
                          />
                        </div>
                      ))}
                      {(
                        [
                          ["gravity_vec", "Gravity Vec"],
                          ["wind_vec", "Wind Vec"],
                          ["wind_offset", "Wind Offset"],
                        ] as [string, string][]
                      ).map(([vecKey, vecLabel]) => (
                        <div key={vecKey} className="inspector-row">
                          <span className="inspector-row-label">{vecLabel}</span>
                          <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr 1fr", gap: 4 }}>
                            {(["x", "y", "z"] as const).map((axis) => (
                              <input
                                key={axis}
                                type="number"
                                step="0.01"
                                size={1}
                                className="field-box mono"
                                title={`${vecLabel}.${axis}`}
                                value={Number((nodePayload.data.header[vecKey]?.[axis] ?? 0).toFixed(4))}
                                onChange={(e) => {
                                  const v = parseFloat(e.target.value || "0");
                                  if (!Number.isNaN(v)) {
                                    const copy = structuredClone(nodePayload);
                                    if (!copy.data.header[vecKey]) {
                                      copy.data.header[vecKey] = { x: 0, y: 0, z: 0 };
                                    }
                                    copy.data.header[vecKey][axis] = v;
                                    setNodePayload(copy);
                                  }
                                }}
                              />
                            ))}
                          </div>
                        </div>
                      ))}
                      <button
                        className="pill-button accent"
                        style={{ marginTop: 6 }}
                        onClick={() =>
                          runClothMutation(
                            nodePayload.node_id,
                            "update_values",
                            undefined,
                            nodePayload.data
                          )
                        }
                      >
                        <span>Apply Header Parameters</span>
                      </button>
                    </div>
                  )}
                </div>
              )}
            </div>
          ) : (
            <div>
              <div className="inspector-header">
                <div className="inspector-icon-box">
                  <IconCube size={20} />
                </div>
                <div>
                  <div className="inspector-title">
                    {nodePayload?.title ?? "No asset selected"}
                  </div>
                  <div className="inspector-subtitle">
                    {nodePayload?.spec_line ??
                      (outline
                        ? `${outline.root_format} · ${outline.endian}`
                        : "Open a container from the File Browser and pick an item from the Asset Browser.")}
                  </div>
                </div>
              </div>

              <div className="inspector-card">
                <button className="card-header-toggle" onClick={() => toggleCard("dll_config")}>
                  <span className="card-header-left">
                    <span style={{ color: "var(--brush-muted)" }}>⚙</span>
                    <span>External Libraries (Oodle &amp; NVTT)</span>
                  </span>
                  <IconChevronDown collapsed={!openCards.dll_config} />
                </button>
                {openCards.dll_config && (
                  <div className="card-body">
                    <div className="inspector-row">
                      <span className="inspector-row-label">Oodle DLL</span>
                      <div style={{ display: "flex", gap: 6, flex: 1 }}>
                        <input
                          type="text"
                          className="field-box mono"
                          style={{ flex: 1, minWidth: 0, fontSize: 11 }}
                          value={oodleDllPath}
                          placeholder="oo2core_*_win64.dll (or pure-Rust ooz fallback)"
                          onChange={(e) => setOodleDllPath(e.target.value)}
                        />
                        <button className="pill-button" onClick={handlePickOodleDll}>
                          Browse
                        </button>
                      </div>
                    </div>
                    <div className="inspector-row">
                      <span className="inspector-row-label">NVTT DLL</span>
                      <div style={{ display: "flex", gap: 6, flex: 1 }}>
                        <input
                          type="text"
                          className="field-box mono"
                          style={{ flex: 1, minWidth: 0, fontSize: 11 }}
                          value={nvttDllPath}
                          placeholder="nvtt30205.dll (or pure-Rust BCn/ASTC fallback)"
                          onChange={(e) => setNvttDllPath(e.target.value)}
                        />
                        <button className="pill-button" onClick={handlePickNvttDll}>
                          Browse
                        </button>
                      </div>
                    </div>
                    <div style={{ color: "var(--brush-muted)", fontSize: 11, marginTop: 4 }}>
                      Includes built-in pure-Rust Oodle Kraken (<span className="mono">ooz</span>) fallback
                      decompressor when <span className="mono">oo2core_*_win64.dll</span> is not present.
                    </div>
                  </div>
                )}
              </div>
            </div>
          )}
        </div>
        {/* [wmb-studio BEGIN phase2] */}
        </>
        )}
        {/* [wmb-studio END] */}
      </div>

      {/* ==================== ROW 3: STATUS BAR ==================== */}
      <footer className="studio-statusbar">
        <div className="status-left">
          {outline?.dirty ? "● " : ""}
          {statusLeft}
        </div>
        <div className="status-right">
          {activeTexture && <span title="Preview zoom">{Math.round(zoom * 100)}%</span>}
          <span>{outline ? outline.summary_right : "PGTools Studio"}</span>
        </div>
      </footer>

      {/* ==================== FLOATING CONTEXT MENU & SUBMENUS ==================== */}
      {contextMenu && (
        <div
          className="context-menu-backdrop"
          onClick={() => setContextMenu(null)}
          onContextMenu={(e) => {
            e.preventDefault();
            setContextMenu(null);
          }}
        >
          <div
            className="context-menu"
            style={{ left: contextMenu.x, top: contextMenu.y }}
            onClick={(e) => e.stopPropagation()}
          >
            {contextMenu.items.map((item, idx) =>
              item.separator ? (
                <div key={idx} className="context-menu-sep" />
              ) : (
                <div
                  key={idx}
                  className="context-menu-item"
                  onMouseEnter={() => setHoverSubmenuIdx(idx)}
                  onClick={() => {
                    if (!item.disabled && item.action) {
                      item.action();
                    }
                  }}
                >
                  <span>{item.label}</span>
                  {item.shortcut && (
                    <span className="context-menu-shortcut">{item.shortcut}</span>
                  )}
                  {item.submenu && <span className="context-menu-shortcut">›</span>}
                  {item.submenu && hoverSubmenuIdx === idx && (
                    <div className="context-submenu">
                      {item.submenu.map((sub, sIdx) =>
                        sub.separator ? (
                          <div key={sIdx} className="context-menu-sep" />
                        ) : (
                          <button
                            key={sIdx}
                            className="context-menu-item"
                            disabled={sub.disabled}
                            onClick={() => {
                              if (sub.action) sub.action();
                            }}
                          >
                            <span>{sub.label}</span>
                          </button>
                        )
                      )}
                    </div>
                  )}
                </div>
              )
            )}
          </div>
        </div>
      )}

      {/* ==================== TEXTURE IMPORT & COMPRESSION MODAL DIALOG ==================== */}
      {texModalState && (
        <div
          className="modal-backdrop"
          onClick={() => setTexModalState(null)}
        >
          <div
            className="modal-card"
            onClick={(e) => e.stopPropagation()}
          >
            <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center" }}>
              <div>
                <div style={{ fontSize: 15, fontWeight: 600, color: "var(--brush-heading)" }}>
                  {texModalState.action === "replace_file"
                    ? "Replace & Encode Texture"
                    : "Add & Encode Texture to Bank"}
                </div>
                <div className="mono" style={{ color: "var(--brush-muted)", fontSize: 11.5, marginTop: 2 }}>
                  Source: {texModalState.fileName}
                </div>
              </div>
              <button
                type="button"
                className="pill-button"
                style={{ height: 26, padding: "0 8px" }}
                onClick={() => setTexModalState(null)}
              >
                ✕
              </button>
            </div>

            {activeTexture && (
              <div
                className="mono"
                style={{
                  padding: "7px 10px",
                  borderRadius: 8,
                  backgroundColor: "var(--brush-window)",
                  border: "1px solid var(--brush-border)",
                  fontSize: 11.5,
                  color: "var(--brush-muted)",
                }}
              >
                Target: {activeTexture.container_ext} · Current Format:{" "}
                <span style={{ color: "var(--brush-text)", fontWeight: 600 }}>
                  {activeTexture.format_name}
                </span>{" "}
                ({activeTexture.width}×{activeTexture.height}, {activeTexture.mip_count} mips)
              </div>
            )}

            <div className="inspector-row">
              <span className="inspector-row-label">Encode Format</span>
              <select
                className="pill-select mono"
                style={{ width: "100%", fontSize: 11.5 }}
                value={texModalState.encodeFormat}
                onChange={(e) =>
                  setTexModalState({ ...texModalState, encodeFormat: e.target.value })
                }
              >
                {ALL_TEXTURE_ENCODE_FORMATS.map((grp) => (
                  <optgroup key={grp.group} label={grp.group}>
                    {grp.options.map((opt) => (
                      <option key={opt.value} value={opt.value}>
                        {opt.label}
                      </option>
                    ))}
                  </optgroup>
                ))}
              </select>
            </div>

            <div className="inspector-row">
              <span className="inspector-row-label">Mipmaps</span>
              <div
                className={`pill-checkbox ${texModalState.generateMips ? "checked" : ""}`}
                onClick={() =>
                  setTexModalState({
                    ...texModalState,
                    generateMips: !texModalState.generateMips,
                  })
                }
              >
                <span className="pill-checkbox-box">
                  {texModalState.generateMips && <IconCheck />}
                </span>
                <span>Generate full box-filtered mipmap chain</span>
              </div>
            </div>

            <div className="inspector-row">
              <span className="inspector-row-label">Solidify</span>
              <div
                className={`pill-checkbox ${texModalState.solidify ? "checked" : ""}`}
                onClick={() =>
                  setTexModalState({ ...texModalState, solidify: !texModalState.solidify })
                }
              >
                <span className="pill-checkbox-box">
                  {texModalState.solidify && <IconCheck />}
                </span>
                <span>Solidify transparent edges (BFS alpha bleed)</span>
              </div>
            </div>

            <div className="inspector-row">
              <span className="inspector-row-label">Opaque</span>
              <div
                className={`pill-checkbox ${texModalState.makeOpaque ? "checked" : ""}`}
                onClick={() =>
                  setTexModalState({
                    ...texModalState,
                    makeOpaque: !texModalState.makeOpaque,
                  })
                }
              >
                <span className="pill-checkbox-box">
                  {texModalState.makeOpaque && <IconCheck />}
                </span>
                <span>Force opaque alpha channel (A = 255)</span>
              </div>
            </div>

            <div style={{ display: "flex", justifyContent: "flex-end", gap: 8, marginTop: 6 }}>
              <button
                type="button"
                className="pill-button"
                onClick={() => setTexModalState(null)}
              >
                Cancel
              </button>
              <button
                type="button"
                className="pill-button accent"
                onClick={async () => {
                  const modal = texModalState;
                  setTexModalState(null);
                  setTexEncodeFormat(modal.encodeFormat);
                  setTexGenerateMips(modal.generateMips);
                  setTexSolidify(modal.solidify);
                  setTexMakeOpaque(modal.makeOpaque);
                  await runTextureMutation(modal.nodeId, modal.action, {
                    path: modal.path,
                    encode_format: modal.encodeFormat,
                    generate_mips: modal.generateMips,
                    solidify: modal.solidify,
                    make_opaque: modal.makeOpaque,
                  });
                }}
              >
                Encode &amp; Import Texture
              </button>
            </div>
          </div>
        </div>
      )}

      {/* [wmb-studio BEGIN phase2] */}
      {coreDatModalOpen && (
        <div
          className="context-menu-backdrop"
          style={{
            display: "flex",
            alignItems: "center",
            justifyContent: "center",
            backgroundColor: "rgba(0, 0, 0, 0.55)",
          }}
          onClick={() => setCoreDatModalOpen(false)}
        >
          <div
            className="studio-card"
            style={{
              width: 540,
              maxWidth: "94vw",
              padding: 16,
              display: "flex",
              flexDirection: "column",
              gap: 12,
            }}
            onClick={(e) => e.stopPropagation()}
          >
            <div style={{ fontWeight: 700, fontSize: 14, color: "var(--brush-heading)" }}>
              Configure Global Engine Textures (core.dat / pre_texture.wtb)
            </div>
            <div style={{ fontSize: 12, color: "var(--brush-muted)", lineHeight: 1.45 }}>
              Assign each game&apos;s <span className="mono">core.dat</span> (or extracted{" "}
              <span className="mono">pre_texture.wtb</span>) so global cubemaps and specular LUTs{" "}
              (<span className="mono">0x8000000N</span>) load with authentic stage reflections.
            </div>
            {(
              [
                ["Bayonetta 1 (core.dat / pre_texture.wtb)", coreDatB1, setCoreDatB1],
                ["Bayonetta 2 (core.dat / pre_texture.wta)", coreDatB2, setCoreDatB2],
                ["Bayonetta 3 (core.dat / pre_texture.wta)", coreDatB3, setCoreDatB3],
              ] as const
            ).map(([label, val, setter]) => (
              <div key={label} style={{ display: "flex", flexDirection: "column", gap: 4 }}>
                <span style={{ fontSize: 11.5, fontWeight: 600 }}>{label}</span>
                <div style={{ display: "flex", gap: 6 }}>
                  <input
                    type="text"
                    className="field-box mono"
                    style={{ flex: 1, fontSize: 11.5 }}
                    placeholder="Not configured (using studio procedural fallback)"
                    value={val}
                    onChange={(e) => setter(e.target.value)}
                  />
                  <button
                    type="button"
                    className="pill-button"
                    onClick={async () => {
                      const picked = await invoke<string | null>("pick_file_dialog", {
                        title: `Select ${label}`,
                        extensions: ["dat", "wtb", "wta"],
                      });
                      if (picked) {
                        setter(picked);
                        setWmbDocGeneration((g) => g + 1);
                      }
                    }}
                  >
                    Browse…
                  </button>
                  {val && (
                    <button
                      type="button"
                      className="pill-button"
                      onClick={() => {
                        setter("");
                        setWmbDocGeneration((g) => g + 1);
                      }}
                    >
                      Clear
                    </button>
                  )}
                </div>
              </div>
            ))}
            <div style={{ display: "flex", justifyContent: "flex-end", gap: 8, marginTop: 4 }}>
              <button
                type="button"
                className="pill-button accent"
                onClick={() => {
                  setCoreDatModalOpen(false);
                  setWmbDocGeneration((g) => g + 1);
                }}
              >
                Done
              </button>
            </div>
          </div>
        </div>
      )}
      {/* [wmb-studio END] */}
    </div>
  );
}
