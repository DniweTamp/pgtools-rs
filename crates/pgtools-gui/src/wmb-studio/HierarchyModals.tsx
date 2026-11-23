import React from "react";
import { StudioSelect } from "../components/StudioSelect";
import { MaterialDesc } from "./contract";

export interface TextureSlotDropModal {
  x: number;
  y: number;
  textureId: number;
  textureName: string;
  material: MaterialDesc;
}

export interface MaterialDeleteModal {
  material: MaterialDesc;
  reassignToIndex: number;
}

interface HierarchyModalsProps {
  activeSlotDrop: TextureSlotDropModal | null;
  isSlotDropClosing: boolean;
  matDeleteModal: MaterialDeleteModal | null;
  materials: MaterialDesc[];
  onCloseSlotDrop: () => void;
  onMutateOp: (op: Record<string, unknown>) => Promise<void>;
  onChangeReassignIndex: (nextIndex: number) => void;
  onCloseMatDelete: () => void;
}

export const HierarchyModals: React.FC<HierarchyModalsProps> = ({
  activeSlotDrop,
  isSlotDropClosing,
  matDeleteModal,
  materials,
  onCloseSlotDrop,
  onMutateOp,
  onChangeReassignIndex,
  onCloseMatDelete,
}) => (
  <>
    {activeSlotDrop && (
      <div
        className={`wmb-context-menu ${isSlotDropClosing ? "closing" : ""}`}
        style={{
          left: Math.min(window.innerWidth - 260, activeSlotDrop.x),
          top: Math.min(window.innerHeight - 240, activeSlotDrop.y),
        }}
        onClick={(e) => e.stopPropagation()}
      >
        <div className="wmb-ctx-header">
          Assign {activeSlotDrop.textureName} to {activeSlotDrop.material.name}
        </div>
        {activeSlotDrop.material.textureSlots.map((slot) => (
          <button
            key={slot.slot}
            type="button"
            className="wmb-ctx-item"
            onClick={() => {
              const modal = activeSlotDrop;
              onCloseSlotDrop();
              void onMutateOp({
                op: "set_material_texture",
                materialId: modal.material.id,
                slot: slot.slot,
                textureId: modal.textureId,
              });
            }}
          >
            <span>
              #{slot.slot} {slot.name}
            </span>
            <span className={`wmb-role-badge role-${slot.role}`}>
              {slot.role}
            </span>
          </button>
        ))}
      </div>
    )}

    {matDeleteModal && (
      <div
        className="wmb-modal-backdrop"
        onClick={onCloseMatDelete}
      >
        <div
          className="wmb-modal-card"
          onClick={(e) => e.stopPropagation()}
        >
          <div className="wmb-modal-title">
            Delete {matDeleteModal.material.name}?
          </div>
          <p className="wmb-modal-desc">
            This material is currently used by{" "}
            <strong>{matDeleteModal.material.usedByBatchIds.length}</strong>{" "}
            batch(es). Choose a material to reassign those batches to before
            deleting:
          </p>
          <div className="wmb-field-row">
            <span>Reassign batches to:</span>
            <StudioSelect
              className="wmb-prop-select mono"
              value={String(matDeleteModal.reassignToIndex)}
              options={materials
                .filter((m) => m.index !== matDeleteModal.material.index)
                .map((m) => ({
                  value: String(m.index),
                  label: `mat${m.index} — ${m.shaderName ?? m.name}`,
                }))}
              onChange={(val) => onChangeReassignIndex(parseInt(val, 10))}
            />
          </div>
          <div className="wmb-modal-actions">
            <button
              type="button"
              className="wmb-vp-btn"
              onClick={onCloseMatDelete}
            >
              Cancel
            </button>
            <button
              type="button"
              className="wmb-vp-btn danger"
              onClick={() => {
                const m = matDeleteModal;
                onCloseMatDelete();
                void onMutateOp({
                  op: "delete_items",
                  items: [m.material.id],
                  reassignMaterialTo: m.reassignToIndex,
                });
              }}
            >
              Reassign &amp; Delete
            </button>
          </div>
        </div>
      </div>
    )}
  </>
);
