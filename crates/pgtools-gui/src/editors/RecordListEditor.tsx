import React, {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { createPortal } from "react-dom";

export interface RecordCellCtx {
  index: number;
  entry: any;
  /** Clone-and-replace the entry at `index` through the draft. */
  patch: (mutator: (entry: any) => void) => void;
}

export interface RecordColumn {
  id: string;
  header: React.ReactNode;
  width?: number;
  align?: "left" | "center" | "right";
  render: (ctx: RecordCellCtx) => React.ReactNode;
}

/** Per-format description of a flat record list (COS / ATT / OBI / MSQ). */
export interface RecordListConfig {
  /** Panel heading, e.g. "Costume Palettes". */
  title: string;
  /** Key holding the array inside the struct draft (`entries` / `points` / `events`). */
  listKey: string;
  /** Singular noun used by the add/remove status messages. */
  entryNoun: string;
  columns: RecordColumn[];
  /** Default record for the "+ Add" button. Must carry `src_index: -1` where the format uses it. */
  makeEntry: (index: number, count: number) => any;
  /** Values captured by the row "copy" button. Never include the row's identity fields. */
  copyValues: (entry: any) => any;
  /** Apply copied values onto `entry` in place. Identity fields must be left alone. */
  pasteValues: (entry: any, values: any) => void;
  /** Short label for the drag ghost and status messages, e.g. "#3 Jeanne". */
  describe: (entry: any, index: number) => string;
  /** Total serialized size in bytes for `count` records, when the file is fully regenerated. */
  sizeBytes?: (count: number) => number | null;
}

export interface RecordListEditorProps {
  config: RecordListConfig;
  draft: any;
  /** Original struct JSON from the inspector, for the unsaved-changes check and Revert. */
  baseline: any;
  onChange: (nextDraft: any) => void;
  onCommit: () => Promise<void> | void;
  onRevert: () => void;
  onStatus: (message: string) => void;
}

const DRAG_THRESHOLD = 6;
const FLASH_MS = 900;

const IconCopy = () => (
  <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
    <rect x="9" y="9" width="13" height="13" rx="2" />
    <path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1" />
  </svg>
);

const IconPaste = () => (
  <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
    <path d="M16 4h2a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2h2" />
    <rect x="8" y="2" width="8" height="4" rx="1" />
  </svg>
);

const IconDelete = () => (
  <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
    <path d="M18 6L6 18" />
    <path d="M6 6l12 12" />
  </svg>
);

interface PointerDragState {
  from: number;
  startX: number;
  startY: number;
  active: boolean;
  insertAt: number | null;
  label: string;
}

export const RecordListEditor: React.FC<RecordListEditorProps> = ({
  config,
  draft,
  baseline,
  onChange,
  onCommit,
  onRevert,
  onStatus,
}) => {
  const entries: any[] = useMemo(
    () => (Array.isArray(draft?.[config.listKey]) ? draft[config.listKey] : []),
    [draft, config.listKey]
  );

  const wrapRef = useRef<HTMLDivElement | null>(null);
  const dragRef = useRef<PointerDragState | null>(null);

  const [selectedIndex, setSelectedIndex] = useState<number | null>(null);
  const [filter, setFilter] = useState("");
  const [clipboard, setClipboard] = useState<{ values: any; label: string } | null>(null);
  const [flashIndex, setFlashIndex] = useState<number | null>(null);
  const [draggingFrom, setDraggingFrom] = useState<number | null>(null);
  const [ghost, setGhost] = useState<{ x: number; y: number; label: string } | null>(null);
  const [dropLine, setDropLine] = useState<{ index: number; top: number } | null>(null);
  const [committing, setCommitting] = useState(false);

  const dirty = useMemo(
    () => JSON.stringify(draft ?? null) !== JSON.stringify(baseline ?? null),
    [draft, baseline]
  );

  const commitList = useCallback(
    (nextList: any[]) => {
      onChange({ ...(draft ?? {}), [config.listKey]: nextList });
    },
    [draft, onChange, config.listKey]
  );

  const patchEntry = useCallback(
    (index: number, mutator: (entry: any) => void) => {
      if (index < 0 || index >= entries.length) return;
      const cloned = structuredClone(entries[index]);
      mutator(cloned);
      const next = entries.slice();
      next[index] = cloned;
      commitList(next);
    },
    [entries, commitList]
  );

  const moveRow = useCallback(
    (from: number, to: number) => {
      if (
        from === to ||
        from < 0 ||
        to < 0 ||
        from >= entries.length ||
        to >= entries.length
      ) {
        return;
      }
      const next = entries.slice();
      const [item] = next.splice(from, 1);
      next.splice(to, 0, item);
      commitList(next);
    },
    [entries, commitList]
  );

  const filtering = filter.trim().length > 0;

  const visibleRows = useMemo(() => {
    const rows = entries.map((entry, index) => ({ entry, index }));
    const needle = filter.trim().toLowerCase();
    if (!needle) return rows;
    return rows.filter(({ entry, index }) =>
      config.describe(entry, index).toLowerCase().includes(needle)
    );
  }, [entries, filter, config]);

  /* ---------------- pointer drag reorder ---------------- */

  useEffect(() => {
    const onPointerMove = (e: PointerEvent) => {
      const drag = dragRef.current;
      if (!drag) return;

      if (!drag.active) {
        if (
          Math.hypot(e.clientX - drag.startX, e.clientY - drag.startY) <
          DRAG_THRESHOLD
        ) {
          return;
        }
        drag.active = true;
        setDraggingFrom(drag.from);
      }

      setGhost({ x: e.clientX, y: e.clientY, label: drag.label });

      const scroller = wrapRef.current?.closest(
        ".struct-table-container"
      ) as HTMLElement | null;
      if (scroller) {
        const rect = scroller.getBoundingClientRect();
        if (e.clientY < rect.top + 24) {
          scroller.scrollTop = Math.max(0, scroller.scrollTop - 16);
        } else if (e.clientY > rect.bottom - 24) {
          scroller.scrollTop += 16;
        }
      }

      const element = document.elementFromPoint(e.clientX, e.clientY) as
        | HTMLElement
        | null;
      const rowEl = element?.closest<HTMLElement>("[data-record-row]") ?? null;
      const wrapRect = wrapRef.current?.getBoundingClientRect();

      if (rowEl && rowEl.dataset.recordRow !== undefined && wrapRect) {
        const index = Number(rowEl.dataset.recordRow);
        const rect = rowEl.getBoundingClientRect();
        const before = e.clientY < rect.top + rect.height / 2;
        drag.insertAt = before ? index : index + 1;
        setDropLine({
          index: drag.insertAt,
          top: (before ? rect.top : rect.bottom) - wrapRect.top,
        });
      } else if (wrapRect) {
        if (e.clientY < wrapRect.top) {
          drag.insertAt = 0;
          setDropLine({ index: 0, top: 0 });
        } else if (e.clientY > wrapRect.bottom) {
          drag.insertAt = entries.length;
          setDropLine({ index: entries.length, top: wrapRect.height });
        }
      }
    };

    const cancelDrag = () => {
      if (!dragRef.current) return;
      dragRef.current = null;
      setGhost(null);
      setDropLine(null);
      setDraggingFrom(null);
    };

    const finishDrag = () => {
      const drag = dragRef.current;
      dragRef.current = null;
      setGhost(null);
      setDropLine(null);
      setDraggingFrom(null);
      if (!drag || !drag.active || drag.insertAt === null) return;
      const to = drag.insertAt > drag.from ? drag.insertAt - 1 : drag.insertAt;
      if (to !== drag.from) {
        moveRow(drag.from, to);
        setSelectedIndex(to);
      }
    };

    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        cancelDrag();
      }
    };

    window.addEventListener("pointermove", onPointerMove);
    window.addEventListener("pointerup", finishDrag);
    window.addEventListener("pointercancel", cancelDrag);
    window.addEventListener("keydown", onKeyDown, true);
    return () => {
      window.removeEventListener("pointermove", onPointerMove);
      window.removeEventListener("pointerup", finishDrag);
      window.removeEventListener("pointercancel", cancelDrag);
      window.removeEventListener("keydown", onKeyDown, true);
    };
  }, [entries.length, moveRow]);

  useEffect(() => {
    if (flashIndex === null) return;
    const timer = window.setTimeout(() => setFlashIndex(null), FLASH_MS);
    return () => window.clearTimeout(timer);
  }, [flashIndex]);

  /* ---------------- actions ---------------- */

  const handleAdd = useCallback(() => {
    const next = entries.concat([config.makeEntry(entries.length, entries.length)]);
    commitList(next);
    const newIndex = next.length - 1;
    setSelectedIndex(newIndex);
    setFilter("");
    window.requestAnimationFrame(() => {
      const row = wrapRef.current?.querySelector<HTMLElement>(
        `[data-record-row="${newIndex}"]`
      );
      row?.querySelector<HTMLInputElement>("input")?.focus();
      row?.scrollIntoView({ block: "nearest" });
    });
    onStatus(`Added ${config.entryNoun} #${newIndex}`);
  }, [entries, config, commitList, onStatus]);

  const handleDelete = useCallback(
    (index: number) => {
      const next = entries.slice();
      next.splice(index, 1);
      commitList(next);
      setSelectedIndex((prev) => {
        if (prev === null) return null;
        if (prev > index) return prev - 1;
        if (prev === index) return next.length === 0 ? null : Math.min(index, next.length - 1);
        return prev;
      });
      onStatus(`Removed ${config.entryNoun} #${index}`);
    },
    [entries, config, commitList, onStatus]
  );

  const handleCopy = useCallback(
    (entry: any, index: number) => {
      const label = config.describe(entry, index);
      setClipboard({ values: config.copyValues(entry), label });
      onStatus(`Copied values from ${label}`);
    },
    [config, onStatus]
  );

  const handlePaste = useCallback(
    (entry: any, index: number) => {
      if (!clipboard) return;
      const target = config.describe(entry, index);
      patchEntry(index, (draftEntry) =>
        config.pasteValues(draftEntry, structuredClone(clipboard.values))
      );
      setFlashIndex(index);
      onStatus(`Pasted values from ${clipboard.label} into ${target}`);
    },
    [clipboard, config, patchEntry, onStatus]
  );

  const handleCommit = useCallback(async () => {
    if (committing || !dirty) return;
    setCommitting(true);
    try {
      await onCommit();
    } finally {
      setCommitting(false);
    }
  }, [committing, dirty, onCommit]);

  const handleRevert = useCallback(() => {
    onRevert();
    setSelectedIndex(null);
    setFilter("");
    setFlashIndex(null);
  }, [onRevert]);

  const handleHandlePointerDown = useCallback(
    (e: React.PointerEvent<HTMLTableCellElement>, index: number) => {
      if (e.button !== 0 || filtering) return;
      e.preventDefault();
      e.stopPropagation();
      dragRef.current = {
        from: index,
        startX: e.clientX,
        startY: e.clientY,
        active: false,
        insertAt: null,
        label: config.describe(entries[index], index),
      };
    },
    [config, entries, filtering]
  );

  const totalBytes = config.sizeBytes ? config.sizeBytes(entries.length) : null;

  return (
    <div className="record-list-root">
      <div className="record-list-toolbar">
        <div className="record-list-heading">
          <span className="record-list-title">{config.title}</span>
          <span className="record-list-meta mono">
            {entries.length} {config.entryNoun}
            {entries.length === 1 ? "" : "s"}
            {totalBytes !== null
              ? ` · 0x${totalBytes.toString(16).toUpperCase()} B`
              : ""}
            {filtering ? ` · ${visibleRows.length} shown` : ""}
          </span>
          {dirty && <span className="record-list-dirty mono">Unsaved</span>}
        </div>
        <div className="record-list-actions">
          <input
            type="text"
            className="field-box mono record-list-filter"
            placeholder={`Filter ${config.entryNoun}s...`}
            value={filter}
            spellCheck={false}
            onChange={(e) => setFilter(e.target.value)}
          />
          <button
            type="button"
            className="pill-button record-list-btn"
            onClick={handleAdd}
            data-tooltip={`Append a new ${config.entryNoun}`}
          >
            + Add
          </button>
          <button
            type="button"
            className="pill-button record-list-btn"
            onClick={handleRevert}
            disabled={!dirty}
            data-tooltip="Discard local edits and reload from the document"
          >
            Revert
          </button>
          <button
            type="button"
            className="pill-button accent record-list-btn"
            onClick={() => void handleCommit()}
            disabled={!dirty || committing}
            data-tooltip="Write these changes back to the document"
          >
            {committing ? "Committing..." : "Commit Changes"}
          </button>
        </div>
      </div>

      <div className="record-list-wrap" ref={wrapRef}>
        {visibleRows.length > 0 && (
        <table className="struct-table record-list-table">
          <thead>
            <tr>
              <th className="record-col-handle" />
              <th className="record-col-index">#</th>
              {config.columns.map((col) => (
                <th
                  key={col.id}
                  style={{
                    width: col.width,
                    textAlign: col.align ?? "left",
                  }}
                >
                  {col.header}
                </th>
              ))}
              <th className="record-col-actions" />
            </tr>
          </thead>
          <tbody>
            {visibleRows.map(({ entry, index }) => {
              const rowClasses = ["record-row"];
              if (selectedIndex === index) rowClasses.push("is-selected");
              if (flashIndex === index) rowClasses.push("is-flashed");
              if (draggingFrom === index) rowClasses.push("is-dragging");
              return (
                <tr
                  key={index}
                  className={rowClasses.join(" ")}
                  data-record-row={index}
                  onMouseDown={(e) => {
                    const target = e.target as HTMLElement | null;
                    if (target?.closest("input, button, textarea, select")) return;
                    setSelectedIndex(index);
                  }}
                >
                  <td
                    className="record-cell-handle"
                    onPointerDown={(e) => handleHandlePointerDown(e, index)}
                    data-tooltip={
                      filtering
                        ? "Clear the filter to reorder"
                        : "Drag to reorder"
                    }
                  >
                    ⠿
                  </td>
                  <td className="record-index mono">#{index}</td>
                  {config.columns.map((col) => (
                    <td
                      key={col.id}
                      style={{
                        width: col.width,
                        textAlign: col.align ?? "left",
                      }}
                    >
                      {col.render({ index, entry, patch: (mutator) => patchEntry(index, mutator) })}
                    </td>
                  ))}
                  <td>
                    <div className="record-row-actions">
                      <button
                        type="button"
                        className="record-mini-btn"
                        onClick={() => handleCopy(entry, index)}
                        aria-label={`Copy values from ${config.entryNoun} #${index}`}
                        data-tooltip="Copy values from this row"
                      >
                        <IconCopy />
                      </button>
                      <button
                        type="button"
                        className="record-mini-btn"
                        onClick={() => handlePaste(entry, index)}
                        disabled={!clipboard}
                        aria-label={`Paste values into ${config.entryNoun} #${index}`}
                        data-tooltip={
                          clipboard
                            ? `Paste values from ${clipboard.label}`
                            : "Clipboard is empty"
                        }
                      >
                        <IconPaste />
                      </button>
                      <button
                        type="button"
                        className="record-mini-btn danger"
                        onClick={() => handleDelete(index)}
                        aria-label={`Remove ${config.entryNoun} #${index}`}
                        data-tooltip={`Remove ${config.entryNoun} #${index}`}
                      >
                        <IconDelete />
                      </button>
                    </div>
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
        )}

        {dropLine && (
          <div className="record-drop-line" style={{ top: dropLine.top }} />
        )}

        {visibleRows.length === 0 && (
          <div className="record-list-empty">
            {entries.length === 0 ? (
              <>
                <span className="record-list-empty-title">
                  No {config.entryNoun}s in this file
                </span>
                <span>Add one to start editing.</span>
                <button
                  type="button"
                  className="pill-button accent record-list-btn"
                  onClick={handleAdd}
                >
                  + Add {config.entryNoun}
                </button>
              </>
            ) : (
              <>
                <span className="record-list-empty-title">
                  No {config.entryNoun} matches “{filter.trim()}”
                </span>
                <span>Clear the filter to see all {entries.length}.</span>
                <button
                  type="button"
                  className="pill-button record-list-btn"
                  onClick={() => setFilter("")}
                >
                  Clear filter
                </button>
              </>
            )}
          </div>
        )}
      </div>

      {ghost &&
        createPortal(
          <div
            className="pointer-drag-ghost"
            style={{ left: ghost.x + 14, top: ghost.y + 12 }}
          >
            <span className="mono">⠿ {ghost.label}</span>
          </div>,
          document.body
        )}
    </div>
  );
};
