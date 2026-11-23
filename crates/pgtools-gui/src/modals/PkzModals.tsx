import React from "react";
import { IconCheck } from "../components/Icons";
import { StudioRadio } from "../components/StudioRadio";
import { StudioRange } from "../components/StudioRange";
import { StudioSelect } from "../components/StudioSelect";
import { clampPkzLevel } from "../hooks/useStudioSettings";
import { OodleValidationReport, PkzCompressionModalState, PkzConflictModalState } from "../types";

interface PkzModalsProps {
  pkzCompModal: PkzCompressionModalState | null;
  setPkzCompModal: React.Dispatch<React.SetStateAction<PkzCompressionModalState | null>>;
  pkzConflictModal: PkzConflictModalState | null;
  setPkzConflictModal: React.Dispatch<React.SetStateAction<PkzConflictModalState | null>>;
  oodleConfigured: boolean;
  oodleValidation: OodleValidationReport | null;
  setPkzDefaultCompression: React.Dispatch<React.SetStateAction<"None" | "ZStandard" | "OodleKraken">>;
  setPkzDefaultLevel: React.Dispatch<React.SetStateAction<number>>;
  handlePickOodleDll: () => Promise<void>;
  setStatusLeft: (msg: string) => void;
}

export const PkzModals: React.FC<PkzModalsProps> = ({
  pkzCompModal,
  setPkzCompModal,
  pkzConflictModal,
  setPkzConflictModal,
  oodleConfigured,
  oodleValidation,
  setPkzDefaultCompression,
  setPkzDefaultLevel,
  handlePickOodleDll,
  setStatusLeft,
}) => (
  <>
      {pkzCompModal && (
        <div
          className="context-menu-backdrop"
          style={{
            display: "flex",
            alignItems: "center",
            justifyContent: "center",
            backgroundColor: "rgba(0, 0, 0, 0.55)",
          }}
          onClick={() => setPkzCompModal(null)}
        >
          <div
            className="studio-card"
            style={{
              width: 470,
              maxWidth: "94vw",
              padding: 16,
              display: "flex",
              flexDirection: "column",
              gap: 12,
            }}
            onClick={(e) => e.stopPropagation()}
          >
            <div>
              <div style={{ fontWeight: 700, fontSize: 14, color: "var(--brush-heading)" }}>
                {pkzCompModal.title}
              </div>
              {pkzCompModal.subtitle && (
                <div style={{ fontSize: 11.5, color: "var(--brush-muted)", marginTop: 3 }}>
                  {pkzCompModal.subtitle}
                </div>
              )}
            </div>

            <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
              {(
                [
                  [
                    "None",
                    "None (Uncompressed)",
                    "Store every file as it is. Fastest loading, largest archives.",
                  ],
                  [
                    "ZStandard",
                    "ZStandard",
                    "Built in, no extra DLL needed. Fast and widely supported.",
                  ],
                  [
                    "OodleKraken",
                    "Oodle Kraken",
                    "What Bayonetta 3 and Astral Chain use natively. Requires oo2core_*_win64.dll for encoding.",
                  ],
                ] as const
              ).map(([codecKey, codecLabel, codecDesc]) => {
                const checked = pkzCompModal.compression === codecKey;
                return (
                  <StudioRadio
                    key={codecKey}
                    checked={checked}
                    onChange={() => {
                      const nextLvl = clampPkzLevel(
                        codecKey,
                        codecKey === "OodleKraken"
                          ? 4
                          : codecKey === "ZStandard"
                          ? 3
                          : 0
                      );
                      setPkzCompModal((prev) =>
                        prev
                          ? { ...prev, compression: codecKey, level: nextLvl }
                          : null
                      );
                    }}
                    style={{
                      display: "flex",
                      alignItems: "flex-start",
                      gap: 9,
                      padding: "8px 10px",
                      borderRadius: 6,
                      border: `1px solid ${
                        checked ? "var(--brush-accent)" : "var(--brush-border)"
                      }`,
                      width: "100%",
                    }}
                    circleStyle={{ marginTop: 2 }}
                  >
                    <div>
                      <div style={{ fontWeight: 600, fontSize: 12.5 }}>{codecLabel}</div>
                      <div style={{ fontSize: 11.5, color: "var(--brush-muted)", marginTop: 2 }}>
                        {codecDesc}
                      </div>
                    </div>
                  </StudioRadio>
                );
              })}
            </div>

            {pkzCompModal.compression !== "None" && (
              <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
                <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center" }}>
                  <span style={{ fontSize: 12, fontWeight: 600 }}>
                    Compression Level ({pkzCompModal.compression === "OodleKraken" ? "1–9" : "1–22"})
                  </span>
                  <span className="mono" style={{ fontWeight: 700, fontSize: 12.5 }}>
                    {pkzCompModal.level}
                  </span>
                </div>
                <StudioRange
                  min={1}
                  max={pkzCompModal.compression === "OodleKraken" ? 9 : 22}
                  step={1}
                  value={pkzCompModal.level}
                  ticks={pkzCompModal.compression === "OodleKraken" ? 9 : 8}
                  ariaLabel="Compression level"
                  onChange={(next) => {
                    setPkzCompModal((prev) =>
                      prev ? { ...prev, level: next } : null
                    );
                  }}
                />
                <button
                  type="button"
                  role="checkbox"
                  aria-checked={pkzCompModal.forceCompression}
                  className={`studio-inline-checkbox${pkzCompModal.forceCompression ? " checked" : ""}`}
                  style={{
                    fontSize: 11.5,
                    color: "var(--brush-muted)",
                    marginTop: 2,
                  }}
                  onClick={() =>
                    setPkzCompModal((prev) =>
                      prev ? { ...prev, forceCompression: !prev.forceCompression } : null
                    )
                  }
                >
                  <span className="pill-checkbox-box">
                    {pkzCompModal.forceCompression && <IconCheck />}
                  </span>
                  <span>
                    Force compression even if file stays at ≥ 90% of original size
                  </span>
                </button>
                <div style={{ fontSize: 11, color: "var(--brush-muted)", lineHeight: 1.4 }}>
                  {pkzCompModal.forceCompression
                    ? "Every file will be stored compressed even if compression does not reduce size by 10%."
                    : "Files that stay at 90% or more of their original size are automatically stored uncompressed (None) for faster in-game loading."}
                </div>
              </div>
            )}

            {pkzCompModal.compression === "OodleKraken" && (
              <div
                style={{
                  padding: "8px 10px",
                  borderRadius: 6,
                  border: "1px solid var(--brush-border)",
                  fontSize: 11.5,
                  display: "flex",
                  justifyContent: "space-between",
                  alignItems: "center",
                  gap: 8,
                }}
              >
                <div style={{ color: "var(--brush-muted)", flex: 1 }}>
                  {oodleValidation?.can_compress
                    ? `✓ ${oodleValidation.message}`
                    : oodleValidation?.message ??
                      "Oodle Kraken compression requires oo2core_*_win64.dll."}
                </div>
                <button
                  type="button"
                  className="pill-button"
                  onClick={() => void handlePickOodleDll()}
                >
                  Browse DLL...
                </button>
              </div>
            )}

            <div style={{ display: "flex", justifyContent: "flex-end", gap: 8, marginTop: 4 }}>
              <button
                type="button"
                className="pill-button"
                onClick={() => setPkzCompModal(null)}
              >
                Cancel
              </button>
              <button
                type="button"
                className="pill-button accent"
                onClick={() => {
                  const { compression, level, forceCompression, onConfirm } = pkzCompModal;
                  setPkzDefaultCompression(compression);
                  if (compression !== "None") {
                    setPkzDefaultLevel(level);
                  }
                  setPkzCompModal(null);
                  void onConfirm(compression, level, forceCompression);
                }}
              >
                OK
              </button>
            </div>
          </div>
        </div>
      )}

      {/* PKZ Collision & Extract Overwrite Modal (matching PKZ Explorer OverwriteDialog) */}
      {pkzConflictModal && (
        <div
          className="context-menu-backdrop"
          style={{
            display: "flex",
            alignItems: "center",
            justifyContent: "center",
            backgroundColor: "rgba(0, 0, 0, 0.55)",
          }}
          onClick={() => setPkzConflictModal(null)}
        >
          <div
            className="studio-card"
            style={{
              width: 440,
              maxWidth: "94vw",
              padding: 16,
              display: "flex",
              flexDirection: "column",
              gap: 12,
            }}
            onClick={(e) => e.stopPropagation()}
          >
            <div style={{ fontWeight: 700, fontSize: 14, color: "var(--brush-heading)" }}>
              {pkzConflictModal.title}
            </div>
            <div
              style={{
                fontSize: 12,
                color: "var(--brush-muted)",
                whiteSpace: "pre-line",
                lineHeight: 1.45,
              }}
            >
              {pkzConflictModal.message}
            </div>
            {pkzConflictModal.paths && pkzConflictModal.paths.length > 0 && (
              <div
                className="mono"
                style={{
                  fontSize: 11,
                  padding: "6px 8px",
                  borderRadius: 4,
                  border: "1px solid var(--brush-border)",
                  maxHeight: 110,
                  overflowY: "auto",
                }}
              >
                {pkzConflictModal.paths.map((p, i) => (
                  <div key={i}>{p}</div>
                ))}
              </div>
            )}
            <div style={{ display: "flex", justifyContent: "flex-end", gap: 8, marginTop: 4 }}>
              <button
                type="button"
                className="pill-button"
                onClick={() => setPkzConflictModal(null)}
              >
                Cancel
              </button>
              <button
                type="button"
                className="pill-button"
                onClick={() => void pkzConflictModal.onChoice("skip")}
              >
                Skip Existing
              </button>
              <button
                type="button"
                className="pill-button accent"
                onClick={() => void pkzConflictModal.onChoice("overwrite")}
              >
                Overwrite
              </button>
            </div>
          </div>
        </div>
      )}
  </>
);
