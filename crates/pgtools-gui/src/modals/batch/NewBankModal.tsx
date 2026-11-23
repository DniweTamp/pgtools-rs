import React, { useState } from "react";
import { createEmptyBankDocument, saveFileDialog } from "../../api/tauri";
import { StudioRadio } from "../../components/StudioRadio";
import { StudioSelect } from "../../components/StudioSelect";
import { DocumentOutline } from "../../types";

interface NewBankModalProps {
  onClose: () => void;
  onCreated: (outline: DocumentOutline) => void;
  onStatus: (msg: string) => void;
}

export const NewBankModal: React.FC<NewBankModalProps> = ({
  onClose,
  onCreated,
  onStatus,
}) => {
  const [isSplit, setIsSplit] = useState(true);
  const [gameProfile, setGameProfile] = useState("switch_xt1");

  const handleCreate = async () => {
    const ext = isSplit ? "wta" : "wtb";
    const dest = await saveFileDialog({
      title: isSplit ? "Create Empty Split Bank (.wta + .wtp)" : "Create Empty Standalone Bank (.wtb)",
      defaultName: `new_bank.${ext}`,
    });
    if (!dest) return;
    try {
      const outline = await createEmptyBankDocument({
        outputPath: dest,
        isSplitWtaWtp: isSplit,
        gameProfile,
      });
      onCreated(outline);
      onStatus(`Created new empty bank: ${outline.file_name}`);
      onClose();
    } catch (err) {
      onStatus(`Failed to create bank: ${String(err)}`);
    }
  };

  return (
    <div className="modal-backdrop" onClick={onClose}>
      <div
        className="modal-card"
        style={{ width: 470, maxWidth: "94vw" }}
        onClick={(e) => e.stopPropagation()}
      >
        <div style={{ fontSize: 15, fontWeight: 600, color: "var(--brush-heading)" }}>
          New empty bank
        </div>

        <div style={{ display: "flex", flexDirection: "column", gap: 10, marginTop: 4 }}>
          <StudioRadio
            checked={isSplit}
            onChange={() => setIsSplit(true)}
            style={{ alignItems: "flex-start", gap: 8 }}
            circleStyle={{ marginTop: 2 }}
          >
            <div>
              <div style={{ fontWeight: 600 }}>Split bank (.wta + .wtp)</div>
              <div style={{ fontSize: 11.5, color: "var(--brush-muted)", marginTop: 2 }}>
                A header file plus a separate payload stream (used for models, UI, and effects).
              </div>
            </div>
          </StudioRadio>

          <StudioRadio
            checked={!isSplit}
            onChange={() => setIsSplit(false)}
            style={{ alignItems: "flex-start", gap: 8 }}
            circleStyle={{ marginTop: 2 }}
          >
            <div>
              <div style={{ fontWeight: 600 }}>Standalone bank (.wtb)</div>
              <div style={{ fontSize: 11.5, color: "var(--brush-muted)", marginTop: 2 }}>
                A single file with the payload inline.
              </div>
            </div>
          </StudioRadio>

          <div className="inspector-row" style={{ marginTop: 6 }}>
            <span className="inspector-row-label">Game Profile</span>
            <StudioSelect
              className="pill-select"
              style={{ width: "100%" }}
              value={gameProfile}
              options={[
                { value: "switch_xt1", label: "Bayonetta 3 / Astral Chain (Switch XT1)" },
                { value: "bayo1", label: "Bayonetta 1 / Vanquish / MGR:R (PC DDS)" },
                { value: "nier", label: "NieR:Automata (PC DDS)" },
                { value: "bayo2_wiiu", label: "Bayonetta 2 (Wii U GTX)" },
              ]}
              onChange={(val) => setGameProfile(val)}
            />
          </div>
        </div>

        <div style={{ display: "flex", justifyContent: "flex-end", gap: 8, marginTop: 10 }}>
          <button type="button" className="pill-button accent" onClick={handleCreate}>
            Create...
          </button>
          <button type="button" className="pill-button" onClick={onClose}>
            Cancel
          </button>
        </div>
      </div>
    </div>
  );
};


// ============================================================================
// 5. BATCH DAT CUSTOM ALIGN MODAL (Tools -> Batch DAT custom align...)
// ============================================================================
