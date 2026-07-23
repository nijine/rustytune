// Table grid editor: heatmap cells, rectangle selection, type-to-edit,
// +/- nudging, and a live operating-point cursor driven by telemetry.
//
// Rows display high-load-at-top (row ny-1 first), TunerStudio-style.
// Every edit POSTs to the server (the single source of truth), then the
// grid refetches so clamping/raw rounding is always reflected.

import { useCallback, useEffect, useRef, useState } from "react";
import { api, type TableJson } from "../api";
import type { TelemetryFeed } from "../feed";

interface Sel {
  r0: number;
  c0: number;
  r1: number;
  c1: number;
}

/** Heatmap: dark→light blue over the table's own value range. */
function cellColor(v: number, lo: number, hi: number): [string, string] {
  const t = hi > lo ? Math.min(1, Math.max(0, (v - lo) / (hi - lo))) : 0;
  // Stops: #0d366b -> #3987e5 -> #cde2fb
  const stops: [number, number, number][] = [
    [13, 54, 107],
    [57, 135, 229],
    [205, 226, 251],
  ];
  const scaled = t * 2;
  const i = Math.min(1, Math.floor(scaled));
  const f = scaled - i;
  const mix = (a: number, b: number) => Math.round(a + (b - a) * f);
  const [r, g, b] = [0, 1, 2].map((k) => mix(stops[i][k], stops[i + 1][k]));
  const fg = t > 0.55 ? "#0d0d0d" : "#ffffff";
  return [`rgb(${r},${g},${b})`, fg];
}

/** Fractional bin index of a live value along an ascending bins axis. */
function binPos(bins: number[], v: number): number | null {
  if (bins.length < 2 || !Number.isFinite(v)) return null;
  if (v <= bins[0]) return 0;
  if (v >= bins[bins.length - 1]) return bins.length - 1;
  for (let i = 0; i < bins.length - 1; i++) {
    if (v <= bins[i + 1]) {
      const span = bins[i + 1] - bins[i];
      return i + (span > 0 ? (v - bins[i]) / span : 0);
    }
  }
  return null;
}

export default function TableEditor({
  tableId,
  feed,
  onError,
}: {
  tableId: string;
  feed: TelemetryFeed;
  onError: (msg: string | null) => void;
}) {
  const [table, setTable] = useState<TableJson | null>(null);
  const [sel, setSel] = useState<Sel | null>(null);
  const [editText, setEditText] = useState<string | null>(null);
  const [cursor, setCursor] = useState<{ x: number; y: number } | null>(null);
  const dragging = useRef(false);
  const containerRef = useRef<HTMLDivElement>(null);

  const load = useCallback(() => {
    api
      .table(tableId)
      .then((t) => {
        setTable(t);
        onError(null);
      })
      .catch((e: Error) => onError(e.message));
  }, [tableId, onError]);

  useEffect(() => {
    setSel(null);
    setEditText(null);
    load();
  }, [load]);

  // Live operating point from telemetry.
  useEffect(() => {
    if (!table?.xChannel || !table.yChannel) {
      setCursor(null);
      return;
    }
    const { xChannel, yChannel, x, y } = table;
    return feed.onFrame((frame) => {
      const xv = frame.channels[xChannel];
      const yv = frame.channels[yChannel];
      if (typeof xv !== "number" || typeof yv !== "number") return;
      const xi = binPos(x, xv);
      const yi = binPos(y, yv);
      setCursor(xi !== null && yi !== null ? { x: xi, y: yi } : null);
    });
  }, [table, feed]);

  if (!table) return <p className="muted">loading table…</p>;

  const ny = table.z.length;
  const nx = ny > 0 ? table.z[0].length : 0;
  const digits = table.zDigits;
  const step = Math.pow(10, -digits);

  const norm = (s: Sel): Sel => ({
    r0: Math.min(s.r0, s.r1),
    r1: Math.max(s.r0, s.r1),
    c0: Math.min(s.c0, s.c1),
    c1: Math.max(s.c0, s.c1),
  });
  const inSel = (r: number, c: number) => {
    if (!sel) return false;
    const n = norm(sel);
    return r >= n.r0 && r <= n.r1 && c >= n.c0 && c <= n.c1;
  };

  const zMin = Math.min(...table.z.flat());
  const zMax = Math.max(...table.z.flat());

  const post = (cells: { row: number; col: number; value: number }[]) => {
    api
      .setCells(table.id, cells)
      .then(load)
      .catch((e: Error) => onError(e.message));
  };

  const commitEdit = () => {
    if (editText === null || !sel) return;
    const value = Number(editText);
    setEditText(null);
    if (!Number.isFinite(value)) return;
    const n = norm(sel);
    const cells = [];
    for (let r = n.r0; r <= n.r1; r++)
      for (let c = n.c0; c <= n.c1; c++) cells.push({ row: r, col: c, value });
    post(cells);
  };

  const nudge = (delta: number) => {
    if (!sel) return;
    const n = norm(sel);
    const cells = [];
    for (let r = n.r0; r <= n.r1; r++)
      for (let c = n.c0; c <= n.c1; c++)
        cells.push({ row: r, col: c, value: table.z[r][c] + delta });
    post(cells);
  };

  const onKeyDown = (e: React.KeyboardEvent) => {
    if (editText !== null) {
      if (e.key === "Enter") commitEdit();
      if (e.key === "Escape") setEditText(null);
      return;
    }
    if (!sel) return;
    const move = (dr: number, dc: number, extend: boolean) => {
      e.preventDefault();
      setSel((s) => {
        if (!s) return s;
        const r1 = Math.min(ny - 1, Math.max(0, s.r1 + dr));
        const c1 = Math.min(nx - 1, Math.max(0, s.c1 + dc));
        return extend ? { ...s, r1, c1 } : { r0: r1, c0: c1, r1, c1 };
      });
    };
    switch (e.key) {
      // Screen-up = higher y index (rows display top-down from ny-1).
      case "ArrowUp":
        return move(1, 0, e.shiftKey);
      case "ArrowDown":
        return move(-1, 0, e.shiftKey);
      case "ArrowLeft":
        return move(0, -1, e.shiftKey);
      case "ArrowRight":
        return move(0, 1, e.shiftKey);
      case "+":
      case "=":
        e.preventDefault();
        return nudge(step);
      case "-":
      case "_":
        e.preventDefault();
        return nudge(-step);
      default:
        if (/^[0-9.]$/.test(e.key)) {
          e.preventDefault();
          setEditText(e.key);
        }
    }
  };

  // Display rows: r = ny-1 .. 0 top to bottom.
  const rows = Array.from({ length: ny }, (_, i) => ny - 1 - i);

  return (
    <div
      className="table-editor"
      tabIndex={0}
      ref={containerRef}
      onKeyDown={onKeyDown}
      onPointerUp={(e) => { dragging.current = false; (e.currentTarget as HTMLElement).releasePointerCapture?.(e.pointerId); }}
      onPointerCancel={() => (dragging.current = false)}
      onPointerMove={(e) => {
        if (!dragging.current) return;
        const cell=document.elementFromPoint(e.clientX,e.clientY)?.closest<HTMLElement>(".cell[data-row]");
        if(cell) { const r=Number(cell.dataset.row), c=Number(cell.dataset.col); setSel(s=>s?{...s,r1:r,c1:c}:s); }
      }}
    >
      <div className="table-flex">
        <div className="ylabels">
          {rows.map((r) => (
            <div key={r}>{table.y[r]?.toFixed(0)}</div>
          ))}
          <div className="corner">{table.yLabel ?? ""}</div>
        </div>
        <div className="zwrap">
          <div
            className="zgrid"
            style={{ gridTemplateColumns: `repeat(${nx}, 1fr)` }}
          >
            {rows.map((r) =>
              Array.from({ length: nx }, (_, c) => {
                const v = table.z[r][c];
                const [bg, fg] = cellColor(v, zMin, zMax);
                const selected = inSel(r, c);
                const active = sel && sel.r1 === r && sel.c1 === c;
                return (
                  <div
                    key={`${r}-${c}`}
                    className={`cell ${selected ? "sel" : ""} ${active ? "active" : ""}`}
                    style={{ background: bg, color: fg }}
                    data-row={r}
                    data-col={c}
                    onPointerDown={(e) => {
                      e.preventDefault();
                      dragging.current = true;
                      containerRef.current?.setPointerCapture?.(e.pointerId);
                      setEditText(null);
                      setSel({ r0: r, c0: c, r1: r, c1: c });
                      containerRef.current?.focus();
                    }}
                  >
                    {active && editText !== null ? (
                      <input
                        className="cell-edit"
                        autoFocus
                        value={editText}
                        onChange={(e) => setEditText(e.target.value)}
                        onBlur={commitEdit}
                      />
                    ) : (
                      v.toFixed(digits)
                    )}
                  </div>
                );
              }),
            )}
            {cursor && (
              <div
                className="op-cursor"
                style={{
                  left: `${((cursor.x + 0.5) / nx) * 100}%`,
                  top: `${((ny - 1 - cursor.y + 0.5) / ny) * 100}%`,
                }}
              />
            )}
          </div>
          <div
            className="xlabels"
            style={{ gridTemplateColumns: `repeat(${nx}, 1fr)` }}
          >
            {Array.from({ length: nx }, (_, c) => (
              <div key={c}>
                {table.x[c] >= 1000
                  ? `${(table.x[c] / 1000).toFixed(1)}k`
                  : table.x[c]?.toFixed(0)}
              </div>
            ))}
          </div>
          <div className="axis-label">{table.xLabel ?? ""}</div>
        </div>
      </div>
      <p className="hint muted">
        drag to select · type a value + Enter · +/− to nudge · arrows to move
      </p>
    </div>
  );
}
