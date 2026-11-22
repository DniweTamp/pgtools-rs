import React, { useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { IconCheck } from "../components/Icons";
import { StudioRadio } from "../components/StudioRadio";
import { StudioRange } from "../components/StudioRange";
import { StudioSelect } from "../components/StudioSelect";
import { ALL_TEXTURE_ENCODE_FORMATS, ENCODE_QUALITY_OPTIONS } from "../editors/TextureEditor";
import {
  clampPkzLevel,
  DEFAULT_CUSTOM_DAT_ALIGNMENT,
  parseCustomDatAlignmentInput,
  sanitizeCustomDatAlignment,
} from "../hooks/useStudioSettings";
import { OodleValidationReport, ThemeMode } from "../types";

export interface SettingsDraftValues {
  theme: ThemeMode;
  hideUnsupported: boolean;
  assetBrowserZoom: number;
  oodleDllPath: string;
  nvttDllPath: string;
  coreDatB1: string;
  coreDatB2: string;
  coreDatB3: string;
  pkzDefaultCompression: "None" | "ZStandard" | "OodleKraken";
  pkzDefaultLevel: number;
  forceCustomDatAlignment: boolean;
  customDatAlignment: number;
  defaultImportFormat: string;
  defaultEncodeQuality: string;
  syncXt1FlagsOnImport: boolean;
  uvdPadding: number;
  uvdMaxSize: number;
}

export interface SettingsModalProps {
  initialValues: SettingsDraftValues;
  oodleConfigured: boolean;
  nvttConfigured: boolean;
  recentFilesCount: number;
  onSave: (next: SettingsDraftValues) => void;
  onClose: () => void;
  onClearRecentFiles: () => void;
  onResetPaneLayout: () => void;
}

type SettingsTabId = "general" | "libraries" | "game_assets" | "archives" | "textures_uvd";

const ZOOM_LABELS = ["List", "48px", "64px", "96px", "128px"];
const UVD_SIZE_OPTIONS = [256, 512, 1024, 2048, 4096, 8192].map((sz) => ({
  value: String(sz),
  label: `${sz} × ${sz}`,
}));

const TABS: { id: SettingsTabId; label: string; keywords: string }[] = [
  {
    id: "general",
    label: "General",
    keywords: "theme dark light file browser unsupported zoom recent layout reset appearance",
  },
  {
    id: "libraries",
    label: "External Libraries",
    keywords: "oodle kraken oo2core dll nvtt nvidia texture tools decompress compress fallback",
  },
  {
    id: "game_assets",
    label: "Game Assets",
    keywords: "core.dat pre_texture wtb wta bayonetta 1 2 3 cubemap lut reflection",
  },
  {
    id: "archives",
    label: "Archives",
    keywords: "pkz compression zstandard oodle kraken level dat alignment bytes hex 0x800",
  },
  {
    id: "textures_uvd",
    label: "Textures & UVD",
    keywords: "texture import format bc1 bc3 bc7 encode quality xt1 flags sync uvd atlas padding max sheet size",
  },
];

function TabIcon({ id }: { id: SettingsTabId }) {
  if (id === "general") {
    return (
      <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8">
        <rect x="3" y="3" width="7" height="7" rx="1.5" />
        <rect x="14" y="3" width="7" height="7" rx="1.5" />
        <rect x="3" y="14" width="7" height="7" rx="1.5" />
        <rect x="14" y="14" width="7" height="7" rx="1.5" />
      </svg>
    );
  }
  if (id === "libraries") {
    return (
      <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8">
        <path d="M12 2 2 7l10 5 10-5-10-5Z" />
        <path d="m2 17 10 5 10-5" />
        <path d="m2 12 10 5 10-5" />
      </svg>
    );
  }
  if (id === "game_assets") {
    return (
      <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8">
        <path d="M21 16V8a2 2 0 0 0-1-1.73l-7-4a2 2 0 0 0-2 0l-7 4A2 2 0 0 0 3 8v8a2 2 0 0 0 1 1.73l7 4a2 2 0 0 0 2 0l7-4A2 2 0 0 0 21 16z" />
        <path d="M3.27 6.96 12 12.01l8.73-5.05" />
        <path d="M12 22.08V12" />
      </svg>
    );
  }
  if (id === "archives") {
    return (
      <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8">
        <rect x="3" y="4" width="18" height="4" rx="1" />
        <path d="M5 8v10a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V8" />
        <path d="M10 12h4" />
      </svg>
    );
  }
  return (
    <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8">
      <rect x="3" y="3" width="18" height="18" rx="2" />
      <circle cx="8.5" cy="8.5" r="1.5" />
      <path d="m21 15-5-5L5 21" />
    </svg>
  );
}

export const SettingsModal: React.FC<SettingsModalProps> = ({
  initialValues,
  oodleConfigured,
  nvttConfigured,
  recentFilesCount,
  onSave,
  onClose,
  onClearRecentFiles,
  onResetPaneLayout,
}) => {
  const [draft, setDraft] = useState<SettingsDraftValues>(() => ({ ...initialValues }));
  const [activeTab, setActiveTab] = useState<SettingsTabId>("general");
  const [searchQuery, setSearchQuery] = useState("");
  const [alignInputText, setAlignInputText] = useState(
    () => `0x${sanitizeCustomDatAlignment(initialValues.customDatAlignment).toString(16).toUpperCase()}`
  );
  const [oodleReport, setOodleReport] = useState<OodleValidationReport | null>(null);
  const [testingOodle, setTestingOodle] = useState(false);

  // Live theme preview while dialog is open; revert on unmount if canceled
  useEffect(() => {
    document.documentElement.setAttribute("data-theme", draft.theme);
  }, [draft.theme]);

  useEffect(() => {
    return () => {
      document.documentElement.setAttribute("data-theme", initialValues.theme);
    };
  }, [initialValues.theme]);

  const runOodleCheck = async (pathOverride?: string) => {
    setTestingOodle(true);
    try {
      const target = pathOverride !== undefined ? pathOverride : draft.oodleDllPath;
      const report = await invoke<OodleValidationReport>("validate_oodle_dll", {
        path: target.trim() ? target : null,
      });
      setOodleReport(report);
    } catch {
      setOodleReport(null);
    } finally {
      setTestingOodle(false);
    }
  };

  useEffect(() => {
    if (activeTab === "libraries" && !oodleReport) {
      void runOodleCheck(draft.oodleDllPath);
    }
  }, [activeTab]);

  const visibleTabs = useMemo(() => {
    const q = searchQuery.trim().toLowerCase();
    if (!q) return TABS;
    return TABS.filter(
      (t) => t.label.toLowerCase().includes(q) || t.keywords.includes(q)
    );
  }, [searchQuery]);

  useEffect(() => {
    if (visibleTabs.length > 0 && !visibleTabs.some((t) => t.id === activeTab)) {
      setActiveTab(visibleTabs[0].id);
    }
  }, [visibleTabs, activeTab]);

  const parsedAlign = useMemo(
    () => parseCustomDatAlignmentInput(alignInputText),
    [alignInputText]
  );

  const alignInvalid = draft.forceCustomDatAlignment && parsedAlign === null;

  const isDirty = useMemo(() => {
    const effectiveAlign = parsedAlign ?? draft.customDatAlignment;
    return (
      draft.theme !== initialValues.theme ||
      draft.hideUnsupported !== initialValues.hideUnsupported ||
      draft.assetBrowserZoom !== initialValues.assetBrowserZoom ||
      draft.oodleDllPath !== initialValues.oodleDllPath ||
      draft.nvttDllPath !== initialValues.nvttDllPath ||
      draft.coreDatB1 !== initialValues.coreDatB1 ||
      draft.coreDatB2 !== initialValues.coreDatB2 ||
      draft.coreDatB3 !== initialValues.coreDatB3 ||
      draft.pkzDefaultCompression !== initialValues.pkzDefaultCompression ||
      draft.pkzDefaultLevel !== initialValues.pkzDefaultLevel ||
      draft.forceCustomDatAlignment !== initialValues.forceCustomDatAlignment ||
      effectiveAlign !== initialValues.customDatAlignment ||
      draft.defaultImportFormat !== initialValues.defaultImportFormat ||
      draft.defaultEncodeQuality !== initialValues.defaultEncodeQuality ||
      draft.syncXt1FlagsOnImport !== initialValues.syncXt1FlagsOnImport ||
      draft.uvdPadding !== initialValues.uvdPadding ||
      draft.uvdMaxSize !== initialValues.uvdMaxSize
    );
  }, [draft, initialValues, parsedAlign]);

  const handleSave = () => {
    if (alignInvalid) return;
    const finalAlign = sanitizeCustomDatAlignment(parsedAlign ?? draft.customDatAlignment);
    const finalLevel = clampPkzLevel(draft.pkzDefaultCompression, draft.pkzDefaultLevel);
    onSave({
      ...draft,
      customDatAlignment: finalAlign,
      pkzDefaultLevel: finalLevel,
    });
  };

  const handleResetTab = () => {
    if (activeTab === "general") {
      setDraft((prev) => ({
        ...prev,
        theme: "dark",
        hideUnsupported: true,
        assetBrowserZoom: 0,
      }));
    } else if (activeTab === "libraries") {
      setDraft((prev) => ({
        ...prev,
        oodleDllPath: "",
        nvttDllPath: "",
      }));
      void runOodleCheck("");
    } else if (activeTab === "game_assets") {
      setDraft((prev) => ({
        ...prev,
        coreDatB1: "",
        coreDatB2: "",
        coreDatB3: "",
      }));
    } else if (activeTab === "archives") {
      setDraft((prev) => ({
        ...prev,
        pkzDefaultCompression: "ZStandard",
        pkzDefaultLevel: 3,
        forceCustomDatAlignment: false,
        customDatAlignment: DEFAULT_CUSTOM_DAT_ALIGNMENT,
      }));
      setAlignInputText(`0x${DEFAULT_CUSTOM_DAT_ALIGNMENT.toString(16).toUpperCase()}`);
    } else if (activeTab === "textures_uvd") {
      setDraft((prev) => ({
        ...prev,
        defaultImportFormat: "Auto",
        defaultEncodeQuality: "Normal",
        syncXt1FlagsOnImport: true,
        uvdPadding: 4,
        uvdMaxSize: 4096,
      }));
    }
  };

  const oodleFull = oodleReport
    ? oodleReport.found && oodleReport.can_compress && oodleReport.can_decompress
    : oodleConfigured;
  const oodleDecompOnly = Boolean(
    oodleReport && oodleReport.found && oodleReport.can_decompress && !oodleReport.can_compress
  );
  const nvttActive = draft.nvttDllPath.trim().length > 0 || nvttConfigured;

  return (
    <div
      className="modal-backdrop"
      onMouseDown={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
      onKeyDown={(e) => {
        if (e.key === "Escape") {
          e.preventDefault();
          e.stopPropagation();
          onClose();
        }
      }}
    >
      <div
        className="settings-dialog"
        role="dialog"
        aria-modal="true"
        aria-label="PGTools Studio Settings"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="settings-header">
          <div className="settings-title-wrap">
            <span className="settings-title">Settings</span>
            <span className="settings-shortcut-pill mono">Ctrl+,</span>
          </div>
          <button
            type="button"
            className="settings-close-btn"
            aria-label="Close settings"
            onClick={onClose}
          >
            <svg width="14" height="14" viewBox="0 0 14 14" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round">
              <path d="M3 3l8 8M11 3l-8 8" />
            </svg>
          </button>
        </div>

        <div className="settings-body">
          <aside className="settings-sidebar">
            <div className="settings-search-wrap">
              <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
                <circle cx="11" cy="11" r="8" />
                <path d="m21 21-4.3-4.3" />
              </svg>
              <input
                type="text"
                className="field-box settings-search-input"
                placeholder="Filter settings..."
                value={searchQuery}
                onChange={(e) => setSearchQuery(e.target.value)}
              />
            </div>

            <nav className="settings-nav-list" aria-label="Settings categories">
              {visibleTabs.map((t) => {
                let dotClass = "";
                if (t.id === "libraries") {
                  dotClass = oodleFull || nvttActive ? "ok" : "muted";
                } else if (t.id === "game_assets") {
                  dotClass =
                    draft.coreDatB1.trim() || draft.coreDatB2.trim() || draft.coreDatB3.trim()
                      ? "ok"
                      : "muted";
                } else if (t.id === "archives" && draft.forceCustomDatAlignment) {
                  dotClass = "accent";
                }
                return (
                  <button
                    key={t.id}
                    type="button"
                    className={`settings-nav-item${activeTab === t.id ? " active" : ""}`}
                    onClick={() => setActiveTab(t.id)}
                  >
                    <span className="settings-nav-left">
                      <TabIcon id={t.id} />
                      <span>{t.label}</span>
                    </span>
                    {dotClass && <span className={`settings-nav-dot ${dotClass}`} />}
                  </button>
                );
              })}
              {visibleTabs.length === 0 && (
                <div className="settings-nav-empty">
                  No settings matching &ldquo;{searchQuery}&rdquo;
                </div>
              )}
            </nav>

            <div className="settings-sidebar-footer mono">pgtools_settings.json</div>
          </aside>

          <section className="settings-content">
            {activeTab === "general" && (
              <>
                <div className="settings-pane-head">
                  <div className="settings-pane-title">General</div>
                </div>

                <div className="settings-section">
                  <div className="settings-section-label">Appearance</div>
                  <div className="settings-row">
                    <span className="settings-row-title">Theme</span>
                    <div className="settings-segmented">
                      <button
                        type="button"
                        className={`settings-seg-btn${draft.theme === "dark" ? " active" : ""}`}
                        onClick={() => setDraft((p) => ({ ...p, theme: "dark" }))}
                      >
                        Dark
                      </button>
                      <button
                        type="button"
                        className={`settings-seg-btn${draft.theme === "light" ? " active" : ""}`}
                        onClick={() => setDraft((p) => ({ ...p, theme: "light" }))}
                      >
                        Light
                      </button>
                    </div>
                  </div>
                </div>

                <div className="settings-section">
                  <div className="settings-section-label">Browsers</div>
                  <div className="settings-row">
                    <span className="settings-row-title">Hide unsupported files</span>
                    <button
                      type="button"
                      role="checkbox"
                      aria-checked={draft.hideUnsupported}
                      className={`studio-inline-checkbox${draft.hideUnsupported ? " checked" : ""}`}
                      onClick={() =>
                        setDraft((p) => ({ ...p, hideUnsupported: !p.hideUnsupported }))
                      }
                    >
                      <span className="pill-checkbox-box">
                        {draft.hideUnsupported && <IconCheck />}
                      </span>
                      <span>Enabled</span>
                    </button>
                  </div>
                  <div className="settings-row">
                    <span className="settings-row-title">Default Asset Browser zoom</span>
                    <div className="settings-control">
                      <span
                        className="mono"
                        style={{
                          fontSize: 11,
                          color: "var(--brush-muted)",
                          minWidth: 36,
                          textAlign: "right",
                        }}
                      >
                        {ZOOM_LABELS[draft.assetBrowserZoom] ?? "List"}
                      </span>
                      <div style={{ width: 128 }}>
                        <StudioRange
                          min={0}
                          max={4}
                          step={1}
                          value={draft.assetBrowserZoom}
                          defaultValue={0}
                          ticks={5}
                          ariaLabel="Default Asset Browser zoom"
                          onChange={(val) =>
                            setDraft((p) => ({ ...p, assetBrowserZoom: val }))
                          }
                        />
                      </div>
                    </div>
                  </div>
                </div>

                <div className="settings-section">
                  <div className="settings-section-label">Workspace</div>
                  <div className="settings-row">
                    <span className="settings-row-title">Recent files</span>
                    <button
                      type="button"
                      className="pill-button"
                      style={{ height: 25, fontSize: 11.5 }}
                      disabled={recentFilesCount === 0}
                      onClick={onClearRecentFiles}
                    >
                      Clear Recent ({recentFilesCount})
                    </button>
                  </div>
                  <div className="settings-row">
                    <span className="settings-row-title">Pane layout</span>
                    <button
                      type="button"
                      className="pill-button"
                      style={{ height: 25, fontSize: 11.5 }}
                      onClick={onResetPaneLayout}
                    >
                      Reset Layout
                    </button>
                  </div>
                </div>
              </>
            )}

            {activeTab === "libraries" && (
              <>
                <div className="settings-pane-head">
                  <div className="settings-pane-title">External Libraries</div>
                </div>

                <div className="settings-section">
                  <div className="settings-path-block">
                    <div className="settings-path-top">
                      <span className="settings-row-title">
                        Oodle Kraken DLL (<span className="mono">oo2core_*_win64.dll</span>)
                      </span>
                      <span
                        className={`settings-status-pill ${
                          oodleFull ? "ok" : oodleDecompOnly ? "warn" : "neutral"
                        }`}
                      >
                        {oodleFull
                          ? "Compress + Decompress"
                          : oodleDecompOnly
                          ? "Decompress Only"
                          : "Pure-Rust ooz Fallback"}
                      </span>
                    </div>
                    <div className="settings-path-row">
                      <input
                        type="text"
                        className="field-box mono"
                        style={{ flex: 1, fontSize: 11.5 }}
                        placeholder="Auto-detect in app folder (or pure-Rust ooz fallback)"
                        value={draft.oodleDllPath}
                        onChange={(e) =>
                          setDraft((p) => ({ ...p, oodleDllPath: e.target.value }))
                        }
                      />
                      <button
                        type="button"
                        className="pill-button"
                        onClick={async () => {
                          const picked = await invoke<string | null>("pick_file_dialog", {
                            title: "Locate Oodle Kraken Library (oo2core_*_win64.dll)",
                            extensions: ["dll", "so", "dylib"],
                          });
                          if (picked) {
                            setDraft((p) => ({ ...p, oodleDllPath: picked }));
                            void runOodleCheck(picked);
                          }
                        }}
                      >
                        Browse…
                      </button>
                      <button
                        type="button"
                        className="pill-button"
                        disabled={testingOodle}
                        onClick={() => void runOodleCheck(draft.oodleDllPath)}
                      >
                        {testingOodle ? "Testing…" : "Test"}
                      </button>
                      {draft.oodleDllPath && (
                        <button
                          type="button"
                          className="pill-button"
                          onClick={() => {
                            setDraft((p) => ({ ...p, oodleDllPath: "" }));
                            void runOodleCheck("");
                          }}
                        >
                          Clear
                        </button>
                      )}
                    </div>
                    {oodleReport && (
                      <div
                        className={`settings-status-banner ${
                          oodleFull ? "ok" : oodleDecompOnly ? "warn" : ""
                        }`}
                      >
                        {oodleReport.message}
                      </div>
                    )}
                  </div>
                </div>

                <div className="settings-section">
                  <div className="settings-path-block">
                    <div className="settings-path-top">
                      <span className="settings-row-title">
                        NVIDIA Texture Tools DLL (<span className="mono">nvtt30205.dll</span>)
                      </span>
                      <span
                        className={`settings-status-pill ${nvttActive ? "ok" : "neutral"}`}
                      >
                        {nvttActive ? "NVTT Active" : "Pure-Rust BCn/ASTC"}
                      </span>
                    </div>
                    <div className="settings-path-row">
                      <input
                        type="text"
                        className="field-box mono"
                        style={{ flex: 1, fontSize: 11.5 }}
                        placeholder="Auto-detect in app folder (or pure-Rust BCn/ASTC fallback)"
                        value={draft.nvttDllPath}
                        onChange={(e) =>
                          setDraft((p) => ({ ...p, nvttDllPath: e.target.value }))
                        }
                      />
                      <button
                        type="button"
                        className="pill-button"
                        onClick={async () => {
                          const picked = await invoke<string | null>("pick_file_dialog", {
                            title: "Locate NVIDIA Texture Tools Library (nvtt30205.dll / nvtt.dll)",
                            extensions: ["dll", "so", "dylib"],
                          });
                          if (picked) {
                            setDraft((p) => ({ ...p, nvttDllPath: picked }));
                          }
                        }}
                      >
                        Browse…
                      </button>
                      {draft.nvttDllPath && (
                        <button
                          type="button"
                          className="pill-button"
                          onClick={() => setDraft((p) => ({ ...p, nvttDllPath: "" }))}
                        >
                          Clear
                        </button>
                      )}
                    </div>
                  </div>
                </div>
              </>
            )}

            {activeTab === "game_assets" && (
              <>
                <div className="settings-pane-head">
                  <div className="settings-pane-title">Game Assets</div>
                </div>

                <div className="settings-section">
                  {(
                    [
                      ["coreDatB1", "Bayonetta 1", "core.dat / pre_texture.wtb"],
                      ["coreDatB2", "Bayonetta 2", "core.dat / pre_texture.wta"],
                      ["coreDatB3", "Bayonetta 3", "core.dat / pre_texture.wta"],
                    ] as const
                  ).map(([key, label, ext]) => {
                    const val = draft[key];
                    return (
                      <div key={key} className="settings-path-block">
                        <div className="settings-path-top">
                          <span className="settings-row-title">
                            {label}{" "}
                            <span
                              className="mono"
                              style={{ color: "var(--brush-muted)", fontSize: 11 }}
                            >
                              ({ext})
                            </span>
                          </span>
                          <span
                            className={`settings-status-pill ${
                              val.trim() ? "ok" : "neutral"
                            }`}
                          >
                            {val.trim() ? "Configured" : "Procedural Fallback"}
                          </span>
                        </div>
                        <div className="settings-path-row">
                          <input
                            type="text"
                            className="field-box mono"
                            style={{ flex: 1, fontSize: 11.5 }}
                            placeholder="Not configured (using studio procedural fallback)"
                            value={val}
                            onChange={(e) =>
                              setDraft((p) => ({ ...p, [key]: e.target.value }))
                            }
                          />
                          <button
                            type="button"
                            className="pill-button"
                            onClick={async () => {
                              const picked = await invoke<string | null>("pick_file_dialog", {
                                title: `Select ${label} (${ext})`,
                                extensions: ["dat", "wtb", "wta"],
                              });
                              if (picked) {
                                setDraft((p) => ({ ...p, [key]: picked }));
                              }
                            }}
                          >
                            Browse…
                          </button>
                          {val && (
                            <button
                              type="button"
                              className="pill-button"
                              onClick={() => setDraft((p) => ({ ...p, [key]: "" }))}
                            >
                              Clear
                            </button>
                          )}
                        </div>
                      </div>
                    );
                  })}
                </div>
              </>
            )}

            {activeTab === "archives" && (
              <>
                <div className="settings-pane-head">
                  <div className="settings-pane-title">Archives</div>
                </div>

                <div className="settings-section">
                  <div className="settings-section-label">Default PKZ Compression</div>
                  <div className="settings-radio-stack">
                    {(
                      [
                        ["ZStandard", "ZStandard"],
                        ["OodleKraken", "Oodle Kraken"],
                        ["None", "Uncompressed (None)"],
                      ] as const
                    ).map(([codecKey, codecLabel]) => {
                      const checked = draft.pkzDefaultCompression === codecKey;
                      return (
                        <StudioRadio
                          key={codecKey}
                          checked={checked}
                          onChange={() => {
                            const nextLvl = clampPkzLevel(
                              codecKey,
                              codecKey === "OodleKraken"
                                ? Math.min(9, draft.pkzDefaultLevel || 4)
                                : codecKey === "ZStandard"
                                ? Math.max(1, draft.pkzDefaultLevel || 3)
                                : 0
                            );
                            setDraft((p) => ({
                              ...p,
                              pkzDefaultCompression: codecKey,
                              pkzDefaultLevel: nextLvl,
                            }));
                          }}
                          className={`settings-radio-card${checked ? " checked" : ""}`}
                        >
                          <span style={{ fontWeight: 500, fontSize: 12 }}>{codecLabel}</span>
                        </StudioRadio>
                      );
                    })}
                  </div>

                  {draft.pkzDefaultCompression !== "None" && (
                    <div style={{ display: "flex", flexDirection: "column", gap: 6, marginTop: 4 }}>
                      <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center" }}>
                        <span style={{ fontSize: 12, fontWeight: 500 }}>
                          Default Compression Level (
                          {draft.pkzDefaultCompression === "OodleKraken" ? "1–9" : "1–22"})
                        </span>
                        <span className="mono" style={{ fontSize: 12, fontWeight: 600 }}>
                          {draft.pkzDefaultLevel}
                        </span>
                      </div>
                      <StudioRange
                        min={1}
                        max={draft.pkzDefaultCompression === "OodleKraken" ? 9 : 22}
                        step={1}
                        value={draft.pkzDefaultLevel}
                        defaultValue={draft.pkzDefaultCompression === "OodleKraken" ? 4 : 3}
                        ticks={draft.pkzDefaultCompression === "OodleKraken" ? 9 : 8}
                        ariaLabel="Default PKZ compression level"
                        onChange={(val) =>
                          setDraft((p) => ({ ...p, pkzDefaultLevel: val }))
                        }
                      />
                    </div>
                  )}
                </div>

                <div className="settings-section">
                  <div className="settings-section-label">DAT Save Alignment</div>
                  <div className="settings-radio-stack">
                    <StudioRadio
                      checked={!draft.forceCustomDatAlignment}
                      onChange={() =>
                        setDraft((p) => ({ ...p, forceCustomDatAlignment: false }))
                      }
                      className={`settings-radio-card${
                        !draft.forceCustomDatAlignment ? " checked" : ""
                      }`}
                    >
                      <span style={{ fontWeight: 500, fontSize: 12 }}>
                        Default (per file type)
                      </span>
                    </StudioRadio>
                    <StudioRadio
                      checked={draft.forceCustomDatAlignment}
                      onChange={() =>
                        setDraft((p) => ({ ...p, forceCustomDatAlignment: true }))
                      }
                      className={`settings-radio-card${
                        draft.forceCustomDatAlignment ? " checked" : ""
                      }`}
                    >
                      <span style={{ fontWeight: 500, fontSize: 12 }}>
                        Force custom alignment on DAT save
                      </span>
                    </StudioRadio>
                  </div>

                  <div
                    className="settings-path-block"
                    style={{
                      marginTop: 4,
                      opacity: draft.forceCustomDatAlignment ? 1 : 0.5,
                    }}
                  >
                    <div className="settings-path-top">
                      <span style={{ fontSize: 11.5, color: "var(--brush-muted)" }}>
                        Custom Alignment (Bytes or 0x Hex)
                      </span>
                      <span
                        className="mono"
                        style={{
                          fontSize: 11,
                          color: parsedAlign
                            ? "var(--brush-accent)"
                            : "var(--brush-warning-text)",
                        }}
                      >
                        {parsedAlign
                          ? `${parsedAlign} B (0x${parsedAlign.toString(16).toUpperCase()})`
                          : "Invalid (1..1048576)"}
                      </span>
                    </div>
                    <input
                      type="text"
                      className={`field-box mono${alignInvalid ? " invalid" : ""}`}
                      disabled={!draft.forceCustomDatAlignment}
                      value={alignInputText}
                      onChange={(e) => {
                        const txt = e.target.value;
                        setAlignInputText(txt);
                        const p = parseCustomDatAlignmentInput(txt);
                        if (p !== null) {
                          setDraft((prev) => ({ ...prev, customDatAlignment: p }));
                        }
                      }}
                    />
                    <div className="settings-preset-row">
                      <span style={{ fontSize: 10.5, color: "var(--brush-muted)" }}>
                        Presets:
                      </span>
                      {["0x10", "0x100", "0x800", "0x1000"].map((preset) => (
                        <button
                          key={preset}
                          type="button"
                          className="pill-button mono"
                          style={{ height: 23, padding: "0 8px", fontSize: 11 }}
                          disabled={!draft.forceCustomDatAlignment}
                          onClick={() => {
                            setAlignInputText(preset);
                            const p = parseCustomDatAlignmentInput(preset);
                            if (p !== null) {
                              setDraft((prev) => ({ ...prev, customDatAlignment: p }));
                            }
                          }}
                        >
                          {preset}
                        </button>
                      ))}
                    </div>
                  </div>
                </div>
              </>
            )}

            {activeTab === "textures_uvd" && (
              <>
                <div className="settings-pane-head">
                  <div className="settings-pane-title">Textures &amp; UVD</div>
                </div>

                <div className="settings-section">
                  <div className="settings-section-label">Texture Import Defaults</div>
                  <div className="settings-row">
                    <span className="settings-row-title">Default import format</span>
                    <StudioSelect
                      className="pill-select mono"
                      style={{ width: 220, fontSize: 11.5 }}
                      value={draft.defaultImportFormat}
                      groups={ALL_TEXTURE_ENCODE_FORMATS.map((grp) => ({
                        label: grp.group,
                        options: grp.options,
                      }))}
                      onChange={(val) =>
                        setDraft((p) => ({ ...p, defaultImportFormat: val }))
                      }
                    />
                  </div>
                  <div className="settings-row">
                    <span className="settings-row-title">Default encode quality</span>
                    <StudioSelect
                      className="pill-select"
                      style={{ width: 220, fontSize: 11.5 }}
                      value={draft.defaultEncodeQuality}
                      options={ENCODE_QUALITY_OPTIONS}
                      onChange={(val) =>
                        setDraft((p) => ({ ...p, defaultEncodeQuality: val }))
                      }
                    />
                  </div>
                  <div className="settings-row">
                    <span className="settings-row-title">Sync XT1 flags on import</span>
                    <button
                      type="button"
                      role="checkbox"
                      aria-checked={draft.syncXt1FlagsOnImport}
                      className={`studio-inline-checkbox${
                        draft.syncXt1FlagsOnImport ? " checked" : ""
                      }`}
                      onClick={() =>
                        setDraft((p) => ({
                          ...p,
                          syncXt1FlagsOnImport: !p.syncXt1FlagsOnImport,
                        }))
                      }
                    >
                      <span className="pill-checkbox-box">
                        {draft.syncXt1FlagsOnImport && <IconCheck />}
                      </span>
                      <span>Enabled</span>
                    </button>
                  </div>
                </div>

                <div className="settings-section">
                  <div className="settings-section-label">UVD Atlas Packer Defaults</div>
                  <div className="settings-row">
                    <span className="settings-row-title">Default sprite padding</span>
                    <div className="settings-control">
                      <span
                        className="mono"
                        style={{
                          fontSize: 11,
                          color: "var(--brush-muted)",
                          minWidth: 36,
                          textAlign: "right",
                        }}
                      >
                        {draft.uvdPadding} px
                      </span>
                      <div style={{ width: 136 }}>
                        <StudioRange
                          min={0}
                          max={64}
                          step={1}
                          value={draft.uvdPadding}
                          defaultValue={4}
                          ticks={9}
                          ariaLabel="Default UVD sprite padding"
                          onChange={(val) =>
                            setDraft((p) => ({ ...p, uvdPadding: val }))
                          }
                        />
                      </div>
                    </div>
                  </div>
                  <div className="settings-row">
                    <span className="settings-row-title">Default max sheet size</span>
                    <StudioSelect
                      className="pill-select mono"
                      style={{ width: 140, fontSize: 11.5 }}
                      value={String(draft.uvdMaxSize)}
                      options={UVD_SIZE_OPTIONS}
                      onChange={(val) =>
                        setDraft((p) => ({
                          ...p,
                          uvdMaxSize: parseInt(val || "4096", 10) || 4096,
                        }))
                      }
                    />
                  </div>
                </div>
              </>
            )}
          </section>
        </div>

        <footer className="settings-footer">
          <div className="settings-footer-left">
            <button
              type="button"
              className="pill-button"
              style={{ height: 26, fontSize: 11.5 }}
              onClick={handleResetTab}
            >
              Reset Tab Defaults
            </button>
            {alignInvalid ? (
              <span className="settings-dirty-badge error">Invalid DAT alignment value</span>
            ) : isDirty ? (
              <span className="settings-dirty-badge">Unsaved changes</span>
            ) : (
              <span className="settings-clean-badge">All changes saved</span>
            )}
          </div>
          <div className="settings-footer-right">
            <button type="button" className="pill-button" onClick={onClose}>
              Cancel
            </button>
            <button
              type="button"
              className="pill-button accent"
              disabled={!isDirty || alignInvalid}
              onClick={handleSave}
            >
              Save Settings
            </button>
          </div>
        </footer>
      </div>
    </div>
  );
};
