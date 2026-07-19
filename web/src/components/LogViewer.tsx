// Log viewer tab: pick a recorded .msl and read it as stacked uPlot strip
// charts — one per channel, cursor and x-zoom synced across all of them.
// Drag to zoom, double-click to reset. Channel picks persist per browser.

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import uPlot from "uplot";
import "uplot/dist/uPlot.min.css";
import { api, type LogDataJson, type LogFileJson } from "../api";

const PALETTE = [
  "#3987e5",
  "#fab219",
  "#0ca30c",
  "#d03b3b",
  "#9d5fe0",
  "#2fb3a8",
  "#e07f2f",
  "#c95fa4",
];

const DEFAULT_CHANNELS = ["RPM", "MAP", "AFR", "TPS", "CLT"];
const STORE_KEY = "rustytune-log-channels";

function loadChannelPrefs(): string[] | null {
  try {
    const raw = localStorage.getItem(STORE_KEY);
    return raw ? (JSON.parse(raw) as string[]) : null;
  } catch {
    return null;
  }
}

/// One strip chart. Instances register into `charts` so cursor moves and
/// x-zoom propagate to every sibling.
function Strip({
  x,
  y,
  label,
  unit,
  color,
  width,
  charts,
}: {
  x: number[];
  y: (number | null)[];
  label: string;
  unit: string;
  color: string;
  width: number;
  charts: React.MutableRefObject<Map<string, uPlot>>;
}) {
  const host = useRef<HTMLDivElement | null>(null);
  const syncing = useRef(false);

  useEffect(() => {
    if (!host.current || width === 0) return;
    const opts: uPlot.Options = {
      width,
      height: 150,
      scales: { x: { time: false } },
      cursor: {
        sync: { key: "rustytune-log" },
        drag: { x: true, y: false },
      },
      series: [
        { label: "Time (s)", value: (_u, v) => (v === null ? "–" : v.toFixed(2)) },
        {
          label: unit ? `${label} (${unit})` : label,
          stroke: color,
          width: 1.5,
          points: { show: false },
          value: (_u, v) => (v === null || v === undefined ? "–" : String(v)),
        },
      ],
      axes: [
        {
          stroke: "#898781",
          grid: { stroke: "#2c2c2a", width: 1 },
          ticks: { stroke: "#2c2c2a" },
        },
        {
          stroke: "#898781",
          grid: { stroke: "#2c2c2a", width: 1 },
          ticks: { stroke: "#2c2c2a" },
          size: 56,
        },
      ],
      hooks: {
        setScale: [
          (u, key) => {
            if (key !== "x" || syncing.current) return;
            const { min, max } = u.scales.x;
            for (const other of charts.current.values()) {
              if (other === u) continue;
              syncing.current = true;
              other.setScale("x", { min: min!, max: max! });
              syncing.current = false;
            }
          },
        ],
      },
    };
    const chart = new uPlot(opts, [x, y] as uPlot.AlignedData, host.current);
    charts.current.set(label, chart);
    return () => {
      charts.current.delete(label);
      chart.destroy();
    };
  }, [x, y, label, unit, color, width, charts]);

  return <div ref={host} className="log-strip" />;
}

export default function LogViewer() {
  const [files, setFiles] = useState<LogFileJson[]>([]);
  const [selected, setSelected] = useState<string | null>(null);
  const [data, setData] = useState<LogDataJson | null>(null);
  const [channels, setChannels] = useState<Set<string>>(new Set());
  const [filter, setFilter] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [width, setWidth] = useState(0);
  const [importing, setImporting] = useState(false);
  const chartsArea = useRef<HTMLDivElement | null>(null);
  const charts = useRef<Map<string, uPlot>>(new Map());
  const filePicker = useRef<HTMLInputElement | null>(null);

  const refreshFiles = useCallback(() => {
    api
      .logs()
      .then((l) => {
        setFiles(l.files);
        setSelected((cur) => cur ?? l.files[0]?.name ?? null);
      })
      .catch((e: Error) => setError(e.message));
  }, []);

  useEffect(refreshFiles, [refreshFiles]);

  const open = useCallback((name: string) => {
    setLoading(true);
    setError(null);
    api
      .logData(name)
      .then((d) => {
        setData(d);
        // Restore saved picks; fall back to common channels present.
        const saved = loadChannelPrefs()?.filter((c) => d.labels.includes(c));
        const defaults = DEFAULT_CHANNELS.filter((c) => d.labels.includes(c));
        const pick = saved?.length ? saved : defaults;
        setChannels(new Set(pick.length ? pick : d.labels.slice(1, 4)));
      })
      .catch((e: Error) => setError(e.message))
      .finally(() => setLoading(false));
  }, []);

  useEffect(() => {
    if (selected) open(selected);
  }, [selected, open]);

  // Copy an existing .msl (e.g. a TunerStudio log) into the server's log
  // dir, then open it.
  const importFile = (file: File) => {
    setImporting(true);
    setError(null);
    file
      .text()
      .then((text) => api.logImport(file.name, text))
      .then((r) => {
        refreshFiles();
        setSelected(r.name);
      })
      .catch((e: Error) => setError(e.message))
      .finally(() => setImporting(false));
  };

  // Charts fill the available width; re-measure on window resize.
  useEffect(() => {
    const measure = () =>
      setWidth(chartsArea.current?.clientWidth ?? 0);
    measure();
    window.addEventListener("resize", measure);
    return () => window.removeEventListener("resize", measure);
  }, [data]);

  const toggle = (label: string) => {
    setChannels((prev) => {
      const next = new Set(prev);
      if (next.has(label)) next.delete(label);
      else next.add(label);
      localStorage.setItem(STORE_KEY, JSON.stringify([...next]));
      return next;
    });
  };

  // Time axis: rows whose time cell parsed; y columns follow that index.
  const plot = useMemo(() => {
    if (!data) return null;
    const timeCol = data.columns[0] ?? [];
    const keep: number[] = [];
    for (let i = 0; i < timeCol.length; i++) {
      if (timeCol[i] !== null) keep.push(i);
    }
    const x = keep.map((i) => timeCol[i] as number);
    const series = data.labels
      .map((label, ci) => ({ label, ci }))
      .filter(({ label, ci }) => ci > 0 && channels.has(label))
      .map(({ label, ci }, order) => ({
        label,
        unit: data.units[ci] ?? "",
        color: PALETTE[order % PALETTE.length],
        y: keep.map((i) => data.columns[ci][i]),
      }));
    return { x, series };
  }, [data, channels]);

  const shownFilter = filter.trim().toLowerCase();

  return (
    <div className="log-viewer">
      <div className="tune-bar">
        <select
          value={selected ?? ""}
          onChange={(e) => setSelected(e.target.value)}
          aria-label="Log file"
        >
          {files.map((f) => (
            <option key={f.name} value={f.name}>
              {f.name}
              {f.active ? " (recording)" : ""}
            </option>
          ))}
        </select>
        <button className="ghost" onClick={refreshFiles} title="Rescan logs">
          ⟳
        </button>
        <button
          className="ghost"
          disabled={importing}
          onClick={() => filePicker.current?.click()}
          title="Copy an existing .msl into the log folder"
        >
          {importing ? "Importing…" : "Import…"}
        </button>
        <input
          ref={filePicker}
          type="file"
          accept=".msl"
          hidden
          onChange={(e) => {
            const f = e.target.files?.[0];
            if (f) importFile(f);
            e.target.value = "";
          }}
        />
        {selected && (
          <button
            className="ghost"
            disabled={loading}
            onClick={() => open(selected)}
          >
            Reload
          </button>
        )}
        {data && (
          <span className="muted">
            {data.rows} rows · drag to zoom · double-click to reset
          </span>
        )}
        {loading && <span className="muted">loading…</span>}
        {error && <span className="error">{error}</span>}
      </div>

      {files.length === 0 && !error && (
        <p className="muted center-note">
          No datalogs yet — hit ● Log while connected, or Import… an
          existing .msl.
        </p>
      )}

      {data && plot && (
        <div className="log-layout">
          <aside className="log-channels">
            <input
              placeholder="filter channels…"
              value={filter}
              onChange={(e) => setFilter(e.target.value)}
            />
            {data.labels.slice(1).map((label) => {
              if (shownFilter && !label.toLowerCase().includes(shownFilter)) {
                return null;
              }
              return (
                <label key={label} className="log-channel">
                  <input
                    type="checkbox"
                    checked={channels.has(label)}
                    onChange={() => toggle(label)}
                  />
                  {label}
                </label>
              );
            })}
          </aside>
          <div className="log-charts" ref={chartsArea}>
            {plot.series.length === 0 ? (
              <p className="muted center-note">
                Pick channels on the left to plot them.
              </p>
            ) : (
              plot.series.map((s) => (
                <Strip
                  key={`${data.name}:${s.label}`}
                  x={plot.x}
                  y={s.y}
                  label={s.label}
                  unit={s.unit}
                  color={s.color}
                  width={width}
                  charts={charts}
                />
              ))
            )}
          </div>
        </div>
      )}
    </div>
  );
}
