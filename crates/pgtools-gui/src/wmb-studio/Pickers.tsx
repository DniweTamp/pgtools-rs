import React, { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { createPortal } from "react-dom";
import type { MaterialDesc, TextureRefDesc, WmbSceneManifest } from "./contract";
import type {
  CorePretextureInfo,
  MaterialBankTextureOption,
  MaterialPickerOption,
} from "../types";
import {
  IconChevronRight,
  IconDelete,
  IconDuplicate,
  IconEye,
  IconShadow,
} from "../components/Icons";
import { formatHexId, formatHexU8, parseStrictHexU32 } from "../utils/format";

export {
  IconChevronRight,
  IconDelete,
  IconDuplicate,
  IconEye,
  IconShadow,
  formatHexId,
  parseStrictHexU32,
};

function useOutsideDismiss(
  open: boolean,
  setOpen: (v: boolean) => void,
  triggerRef: React.RefObject<HTMLElement | null>,
  dropdownRef: React.RefObject<HTMLElement | null>
) {
  useEffect(() => {
    if (!open) return;
    const onPointerDown = (e: MouseEvent) => {
      const target = e.target as Node;
      if (
        triggerRef.current &&
        !triggerRef.current.contains(target) &&
        dropdownRef.current &&
        !dropdownRef.current.contains(target)
      ) {
        setOpen(false);
      }
    };
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        setOpen(false);
      }
    };
    document.addEventListener("mousedown", onPointerDown);
    document.addEventListener("keydown", onKeyDown);
    return () => {
      document.removeEventListener("mousedown", onPointerDown);
      document.removeEventListener("keydown", onKeyDown);
    };
  }, [open, setOpen, triggerRef, dropdownRef]);
}

export function bankOptionsToTextureRefs(
  bankTextures: MaterialBankTextureOption[]
): TextureRefDesc[] {
  return bankTextures.map((bt) => ({
    textureId: bt.texture_id >>> 0,
    idHex: bt.texture_id_hex,
    name: `tex${bt.index}`,
    width: bt.width,
    height: bt.height,
    formatName: bt.format_name,
    contentHash: "",
    usedByMaterialIds: [],
    thumbnailDataUrl: bt.thumbnail_data_url ?? null,
  }));
}

export function materialOptionsToMaterialDescs(
  options: MaterialPickerOption[]
): MaterialDesc[] {
  return options.map((opt) => {
    const firstTid = opt.first_tex_id_hex ? parseStrictHexU32(opt.first_tex_id_hex) : null;
    const materialType = parseInt(opt.badge.replace(/^0x/i, ""), 16) || null;
    return {
      id: opt.node_id,
      index: opt.index,
      name: opt.label,
      game: "b1",
      shaderName: opt.shader_name,
      materialType,
      technique: opt.badge ?? null,
      textureSlots:
        firstTid !== null && opt.first_tex_id_hex
          ? [
              {
                slot: 0,
                name: "Albedo",
                textureId: firstTid,
                textureIdHex: opt.first_tex_id_hex,
                role: "albedo",
                present: true,
                global: false,
              },
            ]
          : [],
      params: [],
      usedByBatchIds: [],
      thumbnailDataUrl: opt.thumbnail_data_url ?? null,
    };
  });
}

/**
 * In WTB/WTA (non-BNTX) archives, the binary format has no string texture names;
 * `wmb_scene.rs` synthesizes `"tex0"`, `"tex1"`, etc. when `meta_name` is `None`.
 * Returns true if the name is absent or one of those synthetic fallback strings.
 */
export function isSyntheticTexName(name: string | undefined | null): boolean {
  if (!name) return true;
  const trimmed = name.trim();
  return trimmed.length === 0 || /^tex\d+$/i.test(trimmed);
}

/**
 * Classify a material sampler slot label/role/index into a concise role badge
 * (e.g., "Albedo", "Normal", "Mask", "Spec", "Env", "LUT").
 */
export function classifySamplerRole(
  slotLabel: string | undefined,
  slotIndex: number,
  slotRole?: string
): string {
  if (slotRole && slotRole !== "unknown") {
    if (slotRole === "albedo") return "Albedo";
    if (slotRole === "normal") return "Normal";
    if (slotRole === "spec") return "Spec";
    if (slotRole === "mask") return "Mask";
    if (slotRole === "env") return "Env";
    if (slotRole === "lut") return "Grad";
  }
  const lbl = (slotLabel || "").toLowerCase();
  if (lbl.includes("albedo") || lbl.includes("color") || lbl.includes("diffuse") || lbl.includes("base")) {
    return "Albedo";
  }
  if (lbl.includes("normal") || lbl.includes("nrm") || lbl.includes("bump")) {
    return "Normal";
  }
  if (lbl.includes("spec") || lbl.includes("gloss")) {
    return "Spec";
  }
  if (lbl.includes("mask") || lbl.includes("rough") || lbl.includes("metal") || lbl.includes("rma") || lbl.includes("param")) {
    return "Mask";
  }
  if (lbl.includes("env") || lbl.includes("cube") || lbl.includes("refl")) {
    return "Env";
  }
  if (lbl.includes("toon") || lbl.includes("grad") || lbl.includes("ramp") || lbl.includes("lut")) {
    return "Grad";
  }
  if (lbl.includes("light") || lbl.includes("ao") || lbl.includes("occ") || lbl.includes("shadow")) {
    return "LightMap";
  }
  if (slotIndex === 0) return "Albedo";
  if (slotIndex === 1) return "Normal";
  if (slotIndex === 2) return "Mask";
  return `Slot ${slotIndex}`;
}

/**
 * Build a lookup from `textureId` (`u32`) to its primary inferred sampler role
 * across all materials in the manifest.
 */
export function inferTextureRoles(manifest: WmbSceneManifest | null): Map<number, string> {
  const map = new Map<number, string>();
  if (!manifest) return map;
  for (const mat of manifest.materials) {
    for (const slot of mat.textureSlots) {
      const tid = slot.textureId >>> 0;
      if (!tid) continue;
      if (!map.has(tid)) {
        map.set(tid, classifySamplerRole(slot.name, slot.slot, slot.role));
      }
    }
  }
  return map;
}

function usePortalDropdownPosition(
  open: boolean,
  triggerRef: React.RefObject<HTMLElement | null>,
  minWidth = 280,
  maxHeight = 320
) {
  const [pos, setPos] = useState<{ top: number; left: number; width: number }>({
    top: 0,
    left: 0,
    width: minWidth,
  });

  useLayoutEffect(() => {
    if (!open || !triggerRef.current) return;
    const updatePosition = () => {
      const el = triggerRef.current;
      if (!el) return;
      const rect = el.getBoundingClientRect();
      const vw = window.innerWidth;
      const vh = window.innerHeight;
      const width = Math.max(minWidth, rect.width);
      let left = rect.left;
      if (left + width > vw - 8) {
        left = Math.max(8, vw - width - 8);
      }
      let top = rect.bottom + 4;
      if (top + maxHeight > vh - 8 && rect.top > maxHeight + 12) {
        top = Math.max(8, rect.top - maxHeight - 4);
      }
      setPos({ top, left, width });
    };
    updatePosition();
    window.addEventListener("resize", updatePosition);
    window.addEventListener("scroll", updatePosition, true);
    return () => {
      window.removeEventListener("resize", updatePosition);
      window.removeEventListener("scroll", updatePosition, true);
    };
  }, [open, triggerRef, minWidth, maxHeight]);

  return pos;
}

export interface MaterialPickerPopoverProps {
  materials: MaterialDesc[];
  textures: TextureRefDesc[];
  currentMatIdx: number;
  onChange: (nextMatIdx: number) => void;
  onJumpToMaterial?: (matIdx: number) => void;
  compact?: boolean;
  allowReselect?: boolean;
  title?: string;
}

export const MaterialPickerPopover: React.FC<MaterialPickerPopoverProps> = ({
  materials,
  textures,
  currentMatIdx,
  onChange,
  onJumpToMaterial,
  compact = false,
  allowReselect = false,
  title,
}) => {
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState("");
  const triggerRef = useRef<HTMLButtonElement | null>(null);
  const dropdownRef = useRef<HTMLDivElement | null>(null);
  const searchInputRef = useRef<HTMLInputElement | null>(null);

  const pos = usePortalDropdownPosition(open, triggerRef, compact ? 290 : 300, 310);

  useOutsideDismiss(open, setOpen, triggerRef, dropdownRef);

  useEffect(() => {
    if (open) {
      setQuery("");
      requestAnimationFrame(() => {
        searchInputRef.current?.focus({ preventScroll: true });
      });
    }
  }, [open]);

  const texById = useMemo(() => {
    const m = new Map<number, { tex: TextureRefDesc; idx: number }>();
    textures.forEach((t, idx) => {
      m.set(t.textureId >>> 0, { tex: t, idx });
    });
    return m;
  }, [textures]);

  const resolveMatThumbAndHex = (mat: MaterialDesc | undefined): { thumbUrl?: string | null; texSummary: string; shaderLabel: string } => {
    const shaderLabel = mat?.shaderName || (mat?.materialType !== null && mat?.materialType !== undefined ? `type ${mat.materialType}` : mat?.name || "standard");
    if (!mat || mat.textureSlots.length === 0) {
      return { thumbUrl: mat?.thumbnailDataUrl, texSummary: "No samplers bound", shaderLabel };
    }
    const primary = mat.textureSlots[0];
    const tid = primary.textureId >>> 0;
    const hit = texById.get(tid);
    const thumbUrl = mat.thumbnailDataUrl || hit?.tex.thumbnailDataUrl || null;
    const hex = hit ? hit.tex.idHex : primary.textureIdHex || formatHexId(tid);
    const slot0Role = classifySamplerRole(primary.name, primary.slot, primary.role);
    return {
      thumbUrl,
      texSummary: `${slot0Role}/Tex0: ${hex}${hit ? ` (#${hit.idx})` : ""}`,
      shaderLabel,
    };
  };

  const currentMat = materials[currentMatIdx];
  const currentInfo = resolveMatThumbAndHex(currentMat);
  const currentHexBadge = `0x${Math.max(0, currentMatIdx).toString(16).toUpperCase().padStart(2, "0")}`;

  const filteredMaterials = useMemo(() => {
    const q = query.trim().toLowerCase();
    if (!q) return materials.map((m, idx) => ({ mat: m, idx }));
    return materials
      .map((mat, idx) => ({ mat, idx }))
      .filter(({ mat, idx }) => {
        const hex = `0x${idx.toString(16).padStart(2, "0")}`;
        const info = resolveMatThumbAndHex(mat);
        return (
          `#${idx}`.includes(q) ||
          hex.toLowerCase().includes(q) ||
          mat.name.toLowerCase().includes(q) ||
          info.shaderLabel.toLowerCase().includes(q) ||
          info.texSummary.toLowerCase().includes(q)
        );
      });
  }, [materials, query, texById]);

  return (
    <div className={compact ? "mat-picker-inline-wrap" : "mat-picker-row-wrap"} onClick={(e) => e.stopPropagation()}>
      <button
        ref={triggerRef}
        type="button"
        className={compact ? "batch-mat-pill" : "mat-picker-trigger"}
        data-tooltip={title ?? (currentMat ? `Material #${currentMatIdx} (${currentInfo.shaderLabel}) — Click to switch material` : `Material #${currentMatIdx}`)}
        onClick={(e) => {
          e.stopPropagation();
          setOpen((v) => !v);
        }}
      >
        {compact ? (
          <>
            <span className="batch-mat-pill-thumb">
              {currentInfo.thumbUrl ? <img src={currentInfo.thumbUrl} alt="" /> : null}
            </span>
            <span>Mat #{currentMatIdx}</span>
            <span className="batch-mat-pill-caret">▾</span>
          </>
        ) : (
          <>
            <span className="mat-picker-thumb">
              {currentInfo.thumbUrl ? (
                <img src={currentInfo.thumbUrl} alt="" />
              ) : (
                <span className="mat-picker-thumb-empty">◆</span>
              )}
            </span>
            <span className="tree-badge">{currentHexBadge}</span>
            <span className="mat-picker-trigger-text">
              <span className="mat-picker-trigger-title">
                {currentMat ? `Material #${currentMatIdx} (${currentInfo.shaderLabel})` : `Material #${currentMatIdx}`}
              </span>
              <span className="mat-picker-trigger-sub">{currentInfo.texSummary}</span>
            </span>
            <span className="mat-picker-caret">▾</span>
          </>
        )}
      </button>

      {!compact && onJumpToMaterial && currentMat && (
        <button
          type="button"
          className="secondary-btn compact-btn"
          data-tooltip={`Select Material #${currentMatIdx} in Inspector`}
          onClick={(e) => {
            e.stopPropagation();
            onJumpToMaterial(currentMatIdx);
          }}
        >
          ↗ Edit Mat
        </button>
      )}

      {open &&
        createPortal(
          <div
            ref={dropdownRef}
            className="mat-picker-dropdown wmb-portal-popover"
            style={{
              position: "fixed",
              top: pos.top,
              left: pos.left,
              width: pos.width,
              maxHeight: 310,
              zIndex: 9999,
            }}
            onClick={(e) => e.stopPropagation()}
          >
            <div className="mat-picker-search-wrap">
              <input
                ref={searchInputRef}
                type="text"
                className="text-input mat-picker-search-input"
                placeholder="Filter materials by #id, shader, or 0xHEX..."
                value={query}
                onChange={(e) => setQuery(e.target.value)}
              />
            </div>
            <div className="mat-picker-list">
              {filteredMaterials.map(({ mat, idx }) => {
                const info = resolveMatThumbAndHex(mat);
                const hexBadge = `0x${idx.toString(16).toUpperCase().padStart(2, "0")}`;
                const active = idx === currentMatIdx;
                return (
                  <button
                    key={mat.id || idx}
                    type="button"
                    className={`mat-picker-option${active ? " active" : ""}`}
                    onClick={(e) => {
                      e.stopPropagation();
                      setOpen(false);
                      if (allowReselect || idx !== currentMatIdx) {
                        onChange(idx);
                      }
                    }}
                  >
                    <span className="mat-picker-thumb">
                      {info.thumbUrl ? (
                        <img src={info.thumbUrl} alt="" />
                      ) : (
                        <span className="mat-picker-thumb-empty">◆</span>
                      )}
                    </span>
                    <span className="tree-badge">{hexBadge}</span>
                    <span className="mat-picker-option-text">
                      <span className="mat-picker-option-title">
                        Material #{idx} ({info.shaderLabel})
                      </span>
                      <span className="mat-picker-option-sub">
                        {info.texSummary} · {mat.textureSlots.length} slots
                      </span>
                    </span>
                  </button>
                );
              })}
              {filteredMaterials.length === 0 && (
                <div className="mat-picker-empty">No matching materials</div>
              )}
            </div>
          </div>,
          document.body
        )}
    </div>
  );
};

export interface TextureSlotPickerPopoverProps {
  slotIndex: number;
  slotLabel: string;
  slotRole?: string;
  currentTexId: number;
  textures: TextureRefDesc[];
  materials: MaterialDesc[];
  corePretextures?: CorePretextureInfo[];
  textureRoles: Map<number, string>;
  useSlotIndexValue?: boolean;
  onSelectTextureId: (nextTexId: number) => void;
  onJumpToTexture?: (texId: number) => void;
}

export const TextureSlotPickerPopover: React.FC<TextureSlotPickerPopoverProps> = ({
  slotIndex,
  slotLabel,
  slotRole,
  currentTexId,
  textures,
  materials,
  corePretextures = [],
  textureRoles,
  useSlotIndexValue = false,
  onSelectTextureId,
  onJumpToTexture,
}) => {
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState("");
  const [customHex, setCustomHex] = useState("");
  const triggerRef = useRef<HTMLButtonElement | null>(null);
  const dropdownRef = useRef<HTMLDivElement | null>(null);
  const searchInputRef = useRef<HTMLInputElement | null>(null);

  const pos = usePortalDropdownPosition(open, triggerRef, 305, 340);

  useOutsideDismiss(open, setOpen, triggerRef, dropdownRef);

  useEffect(() => {
    if (open) {
      setQuery("");
      setCustomHex(formatHexId(currentTexId));
      requestAnimationFrame(() => {
        searchInputRef.current?.focus({ preventScroll: true });
      });
    }
  }, [open, currentTexId]);

  const uCurrentId = currentTexId >>> 0;
  const bankIdx = useSlotIndexValue
    ? (uCurrentId < textures.length ? uCurrentId : textures.findIndex((t) => (t.textureId >>> 0) === uCurrentId))
    : textures.findIndex((t) => (t.textureId >>> 0) === uCurrentId);
  const currentBankTex = bankIdx >= 0 ? textures[bankIdx] : undefined;
  const currentThumb = currentBankTex?.thumbnailDataUrl || null;
  const currentHex = currentBankTex ? currentBankTex.idHex : formatHexId(uCurrentId);
  const currentRole =
    textureRoles.get(uCurrentId) || classifySamplerRole(slotLabel, slotIndex, slotRole);
  const hasRealBntxName =
    currentBankTex && !isSyntheticTexName(currentBankTex.name);

  // Collect external/pretexture IDs (referenced across materials or known engine globals) not in local bank
  const externalPretextures = useMemo(() => {
    const bankSet = new Set(textures.map((t) => t.textureId >>> 0));
    const list: Array<{ id: number; idHex: string; role: string; desc: string }> = [];
    const seen = new Set<number>();

    for (const pt of corePretextures) {
      const tid = (pt.id ?? pt.tex_id ?? 0) >>> 0;
      if (!tid || bankSet.has(tid) || seen.has(tid)) continue;
      seen.add(tid);
      list.push({
        id: tid,
        idHex: pt.id_hex || pt.tex_id_hex || formatHexId(tid),
        role: pt.role || pt.name || "Pretexture",
        desc: pt.description
          ? `${pt.name} · ${pt.description}`
          : `Global Engine Pre-Texture (${pt.name})`,
      });
    }

    for (const mat of materials) {
      for (const s of mat.textureSlots) {
        const tid = s.textureId >>> 0;
        if (!tid || bankSet.has(tid) || seen.has(tid)) continue;
        seen.add(tid);
        list.push({
          id: tid,
          idHex: s.textureIdHex || formatHexId(tid),
          role: classifySamplerRole(s.name, s.slot, s.role),
          desc: `External / Shared Pretexture · Used by ${mat.name}`,
        });
      }
    }
    return list;
  }, [textures, materials, corePretextures]);

  const qLower = query.trim().toLowerCase();

  const filteredBank = useMemo(() => {
    return textures
      .map((tex, idx) => ({ tex, idx }))
      .filter(({ tex, idx }) => {
        if (!qLower) return true;
        const role = textureRoles.get(tex.textureId >>> 0) || "";
        const realName = !isSyntheticTexName(tex.name) ? tex.name.toLowerCase() : "";
        return (
          `#${idx}`.includes(qLower) ||
          tex.idHex.toLowerCase().includes(qLower) ||
          role.toLowerCase().includes(qLower) ||
          realName.includes(qLower) ||
          tex.formatName.toLowerCase().includes(qLower)
        );
      });
  }, [textures, textureRoles, qLower]);

  const filteredExternals = useMemo(() => {
    return externalPretextures.filter((ext) => {
      if (!qLower) return true;
      return (
        ext.idHex.toLowerCase().includes(qLower) ||
        ext.role.toLowerCase().includes(qLower) ||
        ext.desc.toLowerCase().includes(qLower)
      );
    });
  }, [externalPretextures, qLower]);

  return (
    <div className="mat-picker-row-wrap" onClick={(e) => e.stopPropagation()}>
      <button
        ref={triggerRef}
        type="button"
        className="mat-picker-trigger"
        data-tooltip={`Change sampler slot s${slotIndex} (${slotLabel})`}
        onClick={(e) => {
          e.stopPropagation();
          setOpen((v) => !v);
        }}
      >
        <span className="mat-picker-thumb">
          {currentThumb ? (
            <img src={currentThumb} alt="" />
          ) : (
            <span className="mat-picker-thumb-empty">▨</span>
          )}
        </span>
        <span className="tree-badge">{bankIdx >= 0 ? `#${bankIdx}` : "EXT"}</span>
        <span className="mat-picker-trigger-text">
          <span className="mat-picker-trigger-title">
            <span className="mono">{currentHex}</span>
            {hasRealBntxName ? ` · ${currentBankTex!.name}` : ""}
            <span className="tex-role-pill">{currentRole}</span>
          </span>
          <span className="mat-picker-trigger-sub">
            {currentBankTex
              ? `${currentBankTex.width}×${currentBankTex.height} · ${currentBankTex.formatName}`
              : "External / Engine Pretexture"}
          </span>
        </span>
        <span className="mat-picker-caret">▾</span>
      </button>

      {onJumpToTexture && currentBankTex && (
        <button
          type="button"
          className="secondary-btn compact-btn"
          data-tooltip={`Inspect Texture #${bankIdx} (${currentHex})`}
          onClick={(e) => {
            e.stopPropagation();
            onJumpToTexture(uCurrentId);
          }}
        >
          ↗ Open
        </button>
      )}

      {open &&
        createPortal(
          <div
            ref={dropdownRef}
            className="mat-picker-dropdown wmb-portal-popover"
            style={{
              position: "fixed",
              top: pos.top,
              left: pos.left,
              width: pos.width,
              maxHeight: 340,
              zIndex: 9999,
            }}
            onClick={(e) => e.stopPropagation()}
          >
            <div className="mat-picker-search-wrap">
              <input
                ref={searchInputRef}
                type="text"
                className="text-input mat-picker-search-input"
                placeholder="Filter by 0xHEX, #idx, role, format..."
                value={query}
                onChange={(e) => setQuery(e.target.value)}
              />
            </div>
            <div className="mat-picker-list">
              <div className="picker-group-label">
                Model Texture Bank ({filteredBank.length})
              </div>
              {filteredBank.map(({ tex, idx }) => {
                const tid = tex.textureId >>> 0;
                const optionValue = useSlotIndexValue ? idx : tid;
                const active = useSlotIndexValue ? idx === bankIdx : tid === uCurrentId;
                const thumb = tex.thumbnailDataUrl;
                const role = textureRoles.get(tid) || "Bank";
                const bntxName = !isSyntheticTexName(tex.name) ? tex.name : null;
                return (
                  <button
                    key={`${idx}_${tid}`}
                    type="button"
                    className={`mat-picker-option${active ? " active" : ""}`}
                    onClick={(e) => {
                      e.stopPropagation();
                      setOpen(false);
                      onSelectTextureId(optionValue);
                    }}
                  >
                    <span className="mat-picker-thumb">
                      {thumb ? (
                        <img src={thumb} alt="" />
                      ) : (
                        <span className="mat-picker-thumb-empty">▨</span>
                      )}
                    </span>
                    <span className="tree-badge">#{idx}</span>
                    <span className="mat-picker-option-text">
                      <span className="mat-picker-option-title">
                        <span className="mono">{tex.idHex}</span>
                        {bntxName ? ` · ${bntxName}` : ""}
                        <span className="tex-role-pill">{role}</span>
                      </span>
                      <span className="mat-picker-option-sub">
                        {tex.width}×{tex.height} · {tex.formatName}
                      </span>
                    </span>
                  </button>
                );
              })}

              {filteredExternals.length > 0 && (
                <>
                  <div className="picker-group-label">
                    Core Engine Pretextures ({filteredExternals.length})
                  </div>
                  {filteredExternals.map((ext) => {
                    const active = (ext.id >>> 0) === uCurrentId;
                    return (
                      <button
                        key={ext.idHex}
                        type="button"
                        className={`mat-picker-option${active ? " active" : ""}`}
                        onClick={(e) => {
                          e.stopPropagation();
                          setOpen(false);
                          onSelectTextureId(ext.id >>> 0);
                        }}
                      >
                        <span className="mat-picker-thumb">
                          <span className="mat-picker-thumb-empty">PRE</span>
                        </span>
                        <span className="tree-badge">EXT</span>
                        <span className="mat-picker-option-text">
                          <span className="mat-picker-option-title">
                            <span className="mono">{ext.idHex}</span>
                            <span className="tex-role-pill">{ext.role}</span>
                          </span>
                          <span className="mat-picker-option-sub">{ext.desc}</span>
                        </span>
                      </button>
                    );
                  })}
                </>
              )}

              <div className="picker-custom-hex-row">
                <input
                  type="text"
                  className="text-input mono picker-custom-hex-input"
                  value={customHex}
                  placeholder="0x00000000"
                  onChange={(e) => setCustomHex(e.target.value)}
                  onKeyDown={(e) => {
                    if (e.key === "Enter") {
                      const parsed = parseStrictHexU32(customHex);
                      if (parsed !== null) {
                        setOpen(false);
                        onSelectTextureId(parsed);
                      }
                    }
                  }}
                />
                <button
                  type="button"
                  className="secondary-btn compact-btn"
                  disabled={parseStrictHexU32(customHex) === null}
                  onClick={(e) => {
                    e.stopPropagation();
                    const parsed = parseStrictHexU32(customHex);
                    if (parsed !== null) {
                      setOpen(false);
                      onSelectTextureId(parsed);
                    }
                  }}
                >
                  Apply ID
                </button>
              </div>
            </div>
          </div>,
          document.body
        )}
    </div>
  );
};
