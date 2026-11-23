import React from "react";
import { saveFileDialog } from "../api/tauri";
import { IconChevronDown, IconCube } from "../components/Icons";
import { DecodedWmbMaterial, WmbSectionView } from "../types";

interface WmbInspectorViewProps {
  activeWmb: WmbSectionView;
  openCards: Record<string, boolean>;
  toggleCard: (key: string) => void;
  wmbB2Override: boolean | null;
  wmbMatDraft: DecodedWmbMaterial | null;
  setWmbMatDraft: React.Dispatch<React.SetStateAction<DecodedWmbMaterial | null>>;
  runWmbMutation: (nodeId: string, operation: string, payload?: unknown) => Promise<void>;
}

export const WmbInspectorView: React.FC<WmbInspectorViewProps> = ({
  activeWmb,
  openCards,
  toggleCard,
  wmbB2Override,
  wmbMatDraft,
  setWmbMatDraft,
  runWmbMutation,
}) => {
  return (
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
                                  <div className={`collapse-grid${!!openCards.wmb_props ? " expanded" : ""}`}>
  <div className="collapse-inner">
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
                          const dest = await saveFileDialog({
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
  </div>
</div>
                
              </div>
            </div>
  );
};
