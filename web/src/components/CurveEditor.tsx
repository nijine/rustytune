// 1D curve editor for [CurveEditor] bins pairs (WUE, dwell correction,
// idle targets, ...): SVG chart with vertically draggable points, numeric
// x/y inputs, and a live operating-point cursor. Writes go through the
// curve points endpoint, so dirty tracking/flush/burn behave like tables.

import { useCallback, useEffect, useRef, useState } from "react";
import { api, type CurveJson } from "../api";
import type { TelemetryFeed } from "../feed";

const W = 540;
const H = 250;
const M = { l: 52, r: 14, t: 12, b: 34 };

function fmt(v: number, digits: number): string {
  return v.toFixed(digits);
}

/// ~5 round-numbered ticks across [lo, hi].
function ticks(lo: number, hi: number): number[] {
  const span = hi - lo;
  if (!(span > 0)) return [lo];
  const step = 10 ** Math.floor(Math.log10(span / 5));
  const scaled = [1, 2, 2.5, 5, 10].find((m) => span / (step * m) <= 6) ?? 10;
  const inc = step * scaled;
  const out: number[] = [];
  for (let t = Math.ceil(lo / inc) * inc; t <= hi + 1e-9; t += inc) {
    out.push(Math.abs(t) < 1e-9 ? 0 : t);
  }
  return out;
}

export default function CurveEditor({
  id,
  feed,
  onError,
}: {
  id: string;
  feed: TelemetryFeed;
  onError?: (msg: string) => void;
}) {
  const [curve, setCurve] = useState<CurveJson | null>(null);
  const [drag, setDrag] = useState<number | null>(null);
  const [draft, setDraft] = useState<{ key: string; text: string } | null>(
    null,
  );
  const [live, setLive] = useState<number | null>(null);
  const svgRef = useRef<SVGSVGElement | null>(null);
  const dragging = useRef(false);
  const refetchTimer = useRef<ReturnType<typeof setTimeout> | null>(null);

  const load = useCallback(() => {
    api
      .curve(id)
      .then(setCurve)
      .catch((e: Error) => onError?.(e.message));
  }, [id, onError]);

  useEffect(load, [load]);

  // Other clients / the .msq apply path may change the bins under us.
  useEffect(
    () =>
      feed.onTune(() => {
        if (refetchTimer.current) clearTimeout(refetchTimer.current);
        refetchTimer.current = setTimeout(() => {
          if (!dragging.current) load();
        }, 600);
      }),
    [feed, load],
  );

  // Live operating point from telemetry (absent offline: no frames).
  useEffect(() => {
    return feed.onFrame((frame) => {
      const ch = curve?.xChannel;
      if (!ch) return;
      const v = frame.channels[ch];
      setLive(typeof v === "number" ? v : null);
    });
  }, [feed, curve?.xChannel]);

  if (!curve) return null;

  // Chart bounds: the INI's axis ranges, stretched to include the data.
  const xLo = Math.min(curve.xMin, ...curve.x);
  const xHi = Math.max(curve.xMax, ...curve.x);
  const yLo = Math.min(curve.yMin, ...curve.y);
  const yHi = Math.max(curve.yMax, ...curve.y);
  const px = (v: number) =>
    M.l + ((v - xLo) / (xHi - xLo || 1)) * (W - M.l - M.r);
  const py = (v: number) =>
    H - M.b - ((v - yLo) / (yHi - yLo || 1)) * (H - M.t - M.b);

  const clampY = (v: number) => {
    let out = v;
    if (curve.yLo !== null) out = Math.max(out, curve.yLo);
    if (curve.yHi !== null) out = Math.min(out, curve.yHi);
    return out;
  };

  const commit = (axis: "x" | "y", index: number, value: number) => {
    const current = axis === "x" ? curve.x[index] : curve.y[index];
    if (!Number.isFinite(value) || value === current) return;
    api
      .setCurvePoints(curve.id, [{ axis, index, value }])
      .then(load)
      .catch((e: Error) => onError?.(e.message));
  };

  const dragMove = (e: React.PointerEvent) => {
    if (drag === null || !svgRef.current) return;
    const rect = svgRef.current.getBoundingClientRect();
    // clientY -> viewBox units -> chart fraction inside the margins.
    const svgY = ((e.clientY - rect.top) / rect.height) * H;
    const frac = Math.max(0, Math.min(1, (H - M.b - svgY) / (H - M.t - M.b)));
    const value = clampY(yLo + frac * (yHi - yLo));
    const snapped = Number(value.toFixed(curve.yDigits));
    setCurve((c) =>
      c ? { ...c, y: c.y.map((v, i) => (i === drag ? snapped : v)) } : c,
    );
  };

  const dragEnd = () => {
    if (drag === null) return;
    dragging.current = false;
    commit("y", drag, curve.y[drag]);
    setDrag(null);
  };

  const liveX = live !== null && live >= xLo && live <= xHi ? live : null;

  return (
    <div className="curve-editor">
      <h4 className="dlg-header">{curve.title}</h4>
      <svg
        ref={svgRef}
        viewBox={`0 0 ${W} ${H}`}
        className="curve-chart"
        onPointerMove={dragMove}
        onPointerUp={dragEnd}
        onPointerLeave={dragEnd}
      >
        {ticks(yLo, yHi).map((t) => (
          <g key={`y${t}`}>
            <line
              x1={M.l}
              x2={W - M.r}
              y1={py(t)}
              y2={py(t)}
              className="curve-grid"
            />
            <text x={M.l - 6} y={py(t) + 3} className="curve-tick" textAnchor="end">
              {fmt(t, 0)}
            </text>
          </g>
        ))}
        {ticks(xLo, xHi).map((t) => (
          <g key={`x${t}`}>
            <line
              y1={M.t}
              y2={H - M.b}
              x1={px(t)}
              x2={px(t)}
              className="curve-grid"
            />
            <text
              x={px(t)}
              y={H - M.b + 14}
              className="curve-tick"
              textAnchor="middle"
            >
              {fmt(t, 0)}
            </text>
          </g>
        ))}
        {curve.xLabel && (
          <text x={(M.l + W - M.r) / 2} y={H - 4} className="curve-axis" textAnchor="middle">
            {curve.xLabel}
            {curve.xUnits ? ` (${curve.xUnits})` : ""}
          </text>
        )}
        {curve.yLabel && (
          <text
            x={12}
            y={(M.t + H - M.b) / 2}
            className="curve-axis"
            textAnchor="middle"
            transform={`rotate(-90 12 ${(M.t + H - M.b) / 2})`}
          >
            {curve.yLabel}
            {curve.yUnits ? ` (${curve.yUnits})` : ""}
          </text>
        )}
        {liveX !== null && (
          <line
            x1={px(liveX)}
            x2={px(liveX)}
            y1={M.t}
            y2={H - M.b}
            className="curve-live"
          />
        )}
        <polyline
          className="curve-line"
          points={curve.x.map((x, i) => `${px(x)},${py(curve.y[i])}`).join(" ")}
        />
        {curve.x.map((x, i) => (
          <circle
            key={i}
            cx={px(x)}
            cy={py(curve.y[i])}
            r={drag === i ? 7 : 5}
            className="curve-point"
            onPointerDown={(e) => {
              (e.target as Element).setPointerCapture?.(e.pointerId);
              dragging.current = true;
              setDrag(i);
            }}
          >
            <title>
              {fmt(x, curve.xDigits)} → {fmt(curve.y[i], curve.yDigits)}
            </title>
          </circle>
        ))}
      </svg>

      <div className="curve-values">
        <table>
          <tbody>
            {(["x", "y"] as const).map((axis) => (
              <tr key={axis}>
                <th>
                  {(axis === "x" ? curve.xLabel : curve.yLabel) ?? axis}
                  {(axis === "x" ? curve.xUnits : curve.yUnits)
                    ? ` (${axis === "x" ? curve.xUnits : curve.yUnits})`
                    : ""}
                </th>
                {(axis === "x" ? curve.x : curve.y).map((v, i) => {
                  const key = `${axis}${i}`;
                  const digits = axis === "x" ? curve.xDigits : curve.yDigits;
                  return (
                    <td key={i}>
                      <input
                        value={draft?.key === key ? draft.text : fmt(v, digits)}
                        onChange={(e) =>
                          setDraft({ key, text: e.target.value })
                        }
                        onBlur={(e) => {
                          setDraft(null);
                          commit(axis, i, Number(e.target.value));
                        }}
                        onKeyDown={(e) => {
                          if (e.key === "Enter") {
                            (e.target as HTMLInputElement).blur();
                          }
                        }}
                      />
                    </td>
                  );
                })}
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </div>
  );
}
