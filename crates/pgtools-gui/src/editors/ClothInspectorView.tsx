import React from "react";
import { IconChevronDown, IconCube } from "../components/Icons";
import { NodeInspectPayload } from "../types";

interface ClothInspectorViewProps {
  nodePayload: NodeInspectPayload;
  setNodePayload: React.Dispatch<React.SetStateAction<NodeInspectPayload | null>>;
  openCards: Record<string, boolean>;
  toggleCard: (key: string) => void;
  runClothMutation: (nodeId: string, operation: string, preset?: string, updatedJson?: unknown) => Promise<void>;
}

export const ClothInspectorView: React.FC<ClothInspectorViewProps> = ({
  nodePayload,
  setNodePayload,
  openCards,
  toggleCard,
  runClothMutation,
}) => {
  return (
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
                                      <div className={`collapse-grid${!!openCards.cloth_header ? " expanded" : ""}`}>
  <div className="collapse-inner">
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
                                data-tooltip={`${vecLabel}.${axis}`}
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
  </div>
</div>
                  
                </div>
              )}
            </div>
  );
};
