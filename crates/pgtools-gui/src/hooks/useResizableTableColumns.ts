import { useEffect } from "react";

function autoFitColumn(
  table: HTMLTableElement,
  th: HTMLTableCellElement,
  colIdx: number
) {
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
}

/**
 * Global resizable + double-click auto-fit for tables that use
 * .struct-table or .file-table. Keeps the DOM behaviour out of App.
 */
export function useResizableTableColumns(deps: unknown[]) {
  // Auto-fit when the visible table content identity changes
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
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, deps);

  // Drag-to-resize and double-click auto-fit (one-time setup)
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
      const th = target?.closest(
        "table.struct-table th, table.file-table th"
      ) as HTMLTableCellElement | null;
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
      const th = target?.closest(
        "table.struct-table th, table.file-table th"
      ) as HTMLTableCellElement | null;
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
      const th = target?.closest(
        "table.struct-table th, table.file-table th"
      ) as HTMLTableCellElement | null;
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
}
