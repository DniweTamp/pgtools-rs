import React, { useEffect, useRef, useState } from "react";
import { EditableNumberCell } from "../components/EditableNumberCell";
import { StudioColorPicker } from "../components/StudioColorPicker";
import { hexToRgbFloat, rgbFloatToHex } from "../utils/color";
import { clampUtf8Bytes, utf8ByteLength } from "../utils/text";
import type { RecordListConfig } from "./RecordListEditor";

/* ------------------------------------------------------------------ */
/* Shared cells                                                        */
/* ------------------------------------------------------------------ */

const NameInput: React.FC<{
  value: string;
  maxBytes: number;
  onChange: (next: string) => void;
  tooltip?: string;
}> = ({ value, maxBytes, onChange, tooltip }) => {
  const bytes = utf8ByteLength(value);
  const nearLimit = bytes > maxBytes - 16;
  return (
    <div className="record-name-wrap">
      <input
        type="text"
        className="field-box mono record-name-input"
        style={nearLimit ? { paddingRight: 48 } : undefined}
        value={value}
        spellCheck={false}
        autoComplete="off"
        data-tooltip={tooltip}
        onChange={(e) => onChange(clampUtf8Bytes(e.target.value, maxBytes))}
      />
      {nearLimit && (
        <span className="record-name-count mono">
          {bytes}/{maxBytes}
        </span>
      )}
    </div>
  );
};

const Vec3Cell: React.FC<{
  value: any;
  step: string;
  decimals?: number;
  fallback: number;
  onChange: (axis: "x" | "y" | "z", next: number) => void;
}> = ({ value, step, decimals, fallback, onChange }) => (
  <div className="record-vec3">
    {(["x", "y", "z"] as const).map((axis) => (
      <EditableNumberCell
        key={axis}
        isFloat
        decimals={decimals}
        step={step}
        style={{ height: 24 }}
        title={axis.toUpperCase()}
        value={value?.[axis] ?? fallback}
        onChangeValue={(next) => onChange(axis, next)}
      />
    ))}
  </div>
);

const IntCell: React.FC<{
  value: number;
  width?: number;
  onChange: (next: number) => void;
}> = ({ value, width = 95, onChange }) => (
  <EditableNumberCell
    style={{ width, height: 24 }}
    value={value ?? 0}
    onChangeValue={onChange}
  />
);

const readVec3 = (value: any, fallback: number) => ({
  x: Number(value?.x ?? fallback),
  y: Number(value?.y ?? fallback),
  z: Number(value?.z ?? fallback),
});

/* ------------------------------------------------------------------ */
/* COS — Bayonetta 3 costume colour palettes                           */
/* ------------------------------------------------------------------ */

const COS_HEADER_BYTES = 16;
const COS_RECORD_BYTES = 7 * 12 + 0x40; // 7 RGB f32 + 64-byte name

export const COS_LIST_CONFIG: RecordListConfig = {
  title: "Costume Palettes",
  listKey: "entries",
  entryNoun: "costume",
  sizeBytes: (count) => COS_HEADER_BYTES + count * COS_RECORD_BYTES,
  makeEntry: (_index, count) => ({
    name: `Costume ${count + 1}`,
    // 7 RGB floats, neutral white so a new row is visible against any theme
    colors: Array.from({ length: 7 }, () => [1, 1, 1]),
    // No counterpart record in the original file: the backend writes a fresh name field.
    src_index: -1,
  }),
  describe: (entry, index) =>
    `#${index} ${String(entry?.name ?? "").trim() || "(unnamed)"}`,
  copyValues: (entry) => ({
    colors: (Array.isArray(entry?.colors) ? entry.colors : []).map(
      (rgb: number[]) => [Number(rgb?.[0] ?? 0), Number(rgb?.[1] ?? 0), Number(rgb?.[2] ?? 0)]
    ),
  }),
  pasteValues: (entry, values) => {
    if (!Array.isArray(values?.colors)) return;
    entry.colors = values.colors.map((rgb: number[]) => [
      Number(rgb?.[0] ?? 0),
      Number(rgb?.[1] ?? 0),
      Number(rgb?.[2] ?? 0),
    ]);
  },
  columns: [
    {
      id: "name",
      header: "Costume Name",
      render: ({ entry, patch }) => (
        <NameInput
          value={String(entry?.name ?? "")}
          maxBytes={0x3f}
          tooltip="Stored in a 64-byte fixed field (63 usable bytes)"
          onChange={(next) => patch((draft) => { draft.name = next; })}
        />
      ),
    },
    ...Array.from({ length: 7 }, (_, ci) => ({
      id: `color-${ci}`,
      header: <span className="mono">C{ci}</span>,
      width: 38,
      align: "center" as const,
      render: ({ entry, patch }: { entry: any; patch: (m: (e: any) => void) => void }) => {
        const rgb = entry?.colors?.[ci] ?? [0, 0, 0];
        return (
          <StudioColorPicker
            value={rgbFloatToHex([Number(rgb[0] ?? 0), Number(rgb[1] ?? 0), Number(rgb[2] ?? 0)])}
            data-tooltip={`Palette slot C${ci}`}
            onChange={(hex) => {
              const [r, g, b] = hexToRgbFloat(hex);
              patch((draft) => {
                if (!Array.isArray(draft.colors)) draft.colors = [];
                while (draft.colors.length < 7) draft.colors.push([1, 1, 1]);
                draft.colors[ci] = [r, g, b];
              });
            }}
          />
        );
      },
    })),
  ],
};

/* ------------------------------------------------------------------ */
/* ATT — bone attachment points                                        */
/* ------------------------------------------------------------------ */

export const ATT_LIST_CONFIG: RecordListConfig = {
  title: "Bone Attachment Points",
  listKey: "points",
  entryNoun: "point",
  sizeBytes: (count) => 4 + count * 12,
  makeEntry: () => ({ source_bone: 0, target_bone: 0, option: 0 }),
  describe: (entry, index) =>
    `#${index} (bone ${entry?.source_bone ?? 0} -> ${entry?.target_bone ?? 0})`,
  copyValues: (entry) => ({
    source_bone: Number(entry?.source_bone ?? 0),
    target_bone: Number(entry?.target_bone ?? 0),
    option: Number(entry?.option ?? 0),
  }),
  pasteValues: (entry, values) => {
    entry.source_bone = Number(values?.source_bone ?? 0);
    entry.target_bone = Number(values?.target_bone ?? 0);
    entry.option = Number(values?.option ?? 0);
  },
  columns: [
    {
      id: "source_bone",
      header: "Source Bone",
      width: 110,
      render: ({ entry, patch }) => (
        <IntCell
          value={entry?.source_bone ?? 0}
          width={110}
          onChange={(next) => patch((draft) => { draft.source_bone = next; })}
        />
      ),
    },
    {
      id: "target_bone",
      header: "Target Bone",
      width: 110,
      render: ({ entry, patch }) => (
        <IntCell
          value={entry?.target_bone ?? 0}
          width={110}
          onChange={(next) => patch((draft) => { draft.target_bone = next; })}
        />
      ),
    },
    {
      id: "option",
      header: "Option Flag",
      width: 110,
      render: ({ entry, patch }) => (
        <IntCell
          value={entry?.option ?? 0}
          width={110}
          onChange={(next) => patch((draft) => { draft.option = next; })}
        />
      ),
    },
  ],
};

/* ------------------------------------------------------------------ */
/* OBI — object placement instances                                    */
/* ------------------------------------------------------------------ */

const OBI_PREFIXES: Record<number, string> = {
  0: "sc", 1: "pl", 2: "em", 3: "wp", 4: "bg", 5: "gm", 6: "et", 7: "ef",
  9: "it", 10: "id", 11: "es", 12: "bm", 13: "ic", 16: "ct",
};

const OBI_PREFIX_TO_HI: Record<string, number> = Object.fromEntries(
  Object.entries(OBI_PREFIXES).map(([hi, prefix]) => [prefix, Number(hi)])
);

function obiObjectName(id: number): string {
  const prefix = OBI_PREFIXES[(id >>> 16) & 0xffff] ?? "uk";
  return `${prefix}${(id & 0xffff).toString(16).padStart(4, "0")}`;
}

function parseObiObjectId(raw: string): number | null {
  const trimmed = raw.trim().toLowerCase();
  if (!trimmed) return null;
  const named = trimmed.match(/^([a-z]{2})([0-9a-f]{1,4})$/);
  if (named) {
    const hi = OBI_PREFIX_TO_HI[named[1]];
    if (hi !== undefined) {
      return (((hi & 0xffff) << 16) | parseInt(named[2], 16)) >>> 0;
    }
  }
  const hex = trimmed.match(/^0x([0-9a-f]{1,8})$/);
  if (hex) {
    const val = parseInt(hex[1], 16);
    return Number.isFinite(val) && val <= 0xffffffff ? val : null;
  }
  if (/^\d+$/.test(trimmed)) {
    const val = Number(trimmed);
    return Number.isSafeInteger(val) && val >= 0 && val <= 0xffffffff ? val : null;
  }
  return null;
}

const ObiObjectCell: React.FC<{
  objectId: number;
  objectName?: string;
  onChange: (nextId: number) => void;
}> = ({ objectId, objectName, onChange }) => {
  const formatted = objectName || obiObjectName(objectId);
  const [focused, setFocused] = useState(false);
  const [text, setText] = useState(formatted);
  const cancelledRef = useRef(false);

  useEffect(() => {
    if (!focused) setText(formatted);
  }, [formatted, focused]);

  const commit = () => {
    setFocused(false);
    if (cancelledRef.current) {
      cancelledRef.current = false;
      setText(formatted);
      return;
    }
    const parsed = parseObiObjectId(text);
    if (parsed !== null) {
      onChange(parsed);
      setText(obiObjectName(parsed));
    } else {
      setText(formatted);
    }
  };

  return (
    <div style={{ display: "flex", alignItems: "center", gap: 6 }}>
      <input
        type="text"
        className="field-box mono record-name-input"
        style={{ width: 92, height: 24 }}
        value={focused ? text : formatted}
        spellCheck={false}
        autoComplete="off"
        data-tooltip="Object ID (e.g. pl0010, sc0000, or 0x10010)"
        onFocus={() => {
          cancelledRef.current = false;
          setFocused(true);
          setText(formatted);
        }}
        onChange={(e) => setText(e.target.value)}
        onBlur={commit}
        onKeyDown={(e) => {
          if (e.key === "Enter") (e.target as HTMLInputElement).blur();
          if (e.key === "Escape") {
            cancelledRef.current = true;
            setText(formatted);
            (e.target as HTMLInputElement).blur();
          }
        }}
      />
      <span className="mono" style={{ fontSize: 11, color: "var(--brush-muted)" }}>
        0x{(objectId >>> 0).toString(16).toUpperCase()}
      </span>
    </div>
  );
};

export const OBI_LIST_CONFIG: RecordListConfig = {
  title: "Object Placement Instances",
  listKey: "entries",
  entryNoun: "instance",
  // The rebuild path preserves bytes after the record block, so the file size is not
  // a pure function of the entry count.
  makeEntry: () => ({
    object_id: 0,
    object_name: obiObjectName(0),
    flags: 0,
    position: { x: 0, y: 0, z: 0 },
    orientation: { x: 0, y: 0, z: 0 },
    scale: { x: 1, y: 1, z: 1 },
    difficulty_selector: 0,
    // No counterpart record in the original file: the backend zero-fills reserved bytes.
    src_index: -1,
  }),
  describe: (entry, index) =>
    `#${index} ${entry?.object_name || obiObjectName(Number(entry?.object_id ?? 0))}`,
  // Identity (object_id) and the unmodelled flags stay with the row.
  copyValues: (entry) => ({
    position: readVec3(entry?.position, 0),
    orientation: readVec3(entry?.orientation, 0),
    scale: readVec3(entry?.scale, 1),
    difficulty_selector: Number(entry?.difficulty_selector ?? 0),
  }),
  pasteValues: (entry, values) => {
    entry.position = readVec3(values?.position, 0);
    entry.orientation = readVec3(values?.orientation, 0);
    entry.scale = readVec3(values?.scale, 1);
    entry.difficulty_selector = Number(values?.difficulty_selector ?? 0);
  },
  columns: [
    {
      id: "object",
      header: "Object",
      width: 190,
      render: ({ entry, patch }) => (
        <ObiObjectCell
          objectId={Number(entry?.object_id ?? 0)}
          objectName={entry?.object_name}
          onChange={(nextId) =>
            patch((draft) => {
              draft.object_id = nextId;
              draft.object_name = obiObjectName(nextId);
            })
          }
        />
      ),
    },
    {
      id: "position",
      header: "Position (X, Y, Z)",
      width: 230,
      render: ({ entry, patch }) => (
        <Vec3Cell
          value={entry?.position}
          step="0.1"
          decimals={3}
          fallback={0}
          onChange={(axis, next) =>
            patch((draft) => {
              draft.position = { ...readVec3(draft.position, 0), [axis]: next };
            })
          }
        />
      ),
    },
    {
      id: "orientation",
      header: "Rotation (X, Y, Z)",
      width: 230,
      render: ({ entry, patch }) => (
        <Vec3Cell
          value={entry?.orientation}
          step="0.05"
          decimals={3}
          fallback={0}
          onChange={(axis, next) =>
            patch((draft) => {
              draft.orientation = { ...readVec3(draft.orientation, 0), [axis]: next };
            })
          }
        />
      ),
    },
    {
      id: "scale",
      header: "Scale (X, Y, Z)",
      width: 230,
      render: ({ entry, patch }) => (
        <Vec3Cell
          value={entry?.scale}
          step="0.05"
          decimals={3}
          fallback={1}
          onChange={(axis, next) =>
            patch((draft) => {
              draft.scale = { ...readVec3(draft.scale, 1), [axis]: next };
            })
          }
        />
      ),
    },
    {
      id: "difficulty",
      header: "Difficulty",
      width: 100,
      render: ({ entry, patch }) => (
        <IntCell
          value={entry?.difficulty_selector ?? 0}
          onChange={(next) =>
            patch((draft) => {
              draft.difficulty_selector = Math.max(0, Math.min(255, Math.round(next)));
            })
          }
        />
      ),
    },
  ],
};

/* ------------------------------------------------------------------ */
/* MSQ — subtitle / speech sequence events                             */
/* ------------------------------------------------------------------ */

const clampU32 = (n: number) => Math.max(0, Math.min(0xffffffff, Math.trunc(n)));

export const MSQ_LIST_CONFIG: RecordListConfig = {
  title: "Subtitle / Sequence Events",
  listKey: "events",
  entryNoun: "event",
  makeEntry: () => ({
    name: "new_event",
    id: 0,
    event_type: 0,
    line_number_per_group: 0,
    event_duration: 0,
    src_index: -1,
  }),
  describe: (entry, index) =>
    `#${index} ${String(entry?.name ?? "").trim() || "(unnamed event)"}`,
  // Name and id identify the event and are left alone by paste.
  copyValues: (entry) => ({
    event_type: Number(entry?.event_type ?? 0),
    line_number_per_group: Number(entry?.line_number_per_group ?? 0),
    event_duration: Number(entry?.event_duration ?? 0),
  }),
  pasteValues: (entry, values) => {
    entry.event_type = Number(values?.event_type ?? 0);
    entry.line_number_per_group = Number(values?.line_number_per_group ?? 0);
    entry.event_duration = Number(values?.event_duration ?? 0);
  },
  columns: [
    {
      id: "name",
      header: "Event Name",
      render: ({ entry, patch }) => (
        <NameInput
          value={String(entry?.name ?? "")}
          maxBytes={0x1f}
          tooltip="Stored in a 32-byte fixed field (31 usable bytes)"
          onChange={(next) => patch((draft) => { draft.name = next; })}
        />
      ),
    },
    {
      id: "id",
      header: "ID",
      width: 100,
      render: ({ entry, patch }) => (
        <IntCell
          value={entry?.id ?? 0}
          onChange={(next) => patch((draft) => { draft.id = clampU32(next); })}
        />
      ),
    },
    {
      id: "event_type",
      header: "Event Type",
      width: 100,
      render: ({ entry, patch }) => (
        <IntCell
          value={entry?.event_type ?? 0}
          onChange={(next) => patch((draft) => { draft.event_type = clampU32(next); })}
        />
      ),
    },
    {
      id: "line_number_per_group",
      header: "Line / Group",
      width: 100,
      render: ({ entry, patch }) => (
        <IntCell
          value={entry?.line_number_per_group ?? 0}
          onChange={(next) => patch((draft) => { draft.line_number_per_group = clampU32(next); })}
        />
      ),
    },
    {
      id: "event_duration",
      header: "Duration (Frames)",
      width: 110,
      render: ({ entry, patch }) => (
        <IntCell
          value={entry?.event_duration ?? 0}
          onChange={(next) => patch((draft) => { draft.event_duration = clampU32(next); })}
        />
      ),
    },
  ],
};
