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

// Cursor sync uses uPlot; shared range state keeps every strip at the same zoom.
function Strip({
  x,
  y,
  label,
  unit,
  color,
  width,
  range,
  onZoom,
}: {
  x: number[];
  y: (number | null)[];
  label: string;
  unit: string;
  color: string;
  width: number;
  range: [number, number] | null;
  onZoom: (range: [number, number] | null) => void;
}) {
  const host = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    if (!host.current || width === 0) return;
    let ready = false;
    const opts: uPlot.Options = {
      width,
      height: 150,
      scales: { x: { time: false, ...(range ? { min: range[0], max: range[1] } : {}) } },
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
            if (key !== "x" || !ready) return;
            const { min, max } = u.scales.x;
            if (min != null && max != null) onZoom([min, max]);
          },
        ],
      },
    };
    const chart = new uPlot(opts, [x, y] as uPlot.AlignedData, host.current);
    ready = true;
    const reset = () => onZoom(null);
    host.current.addEventListener("dblclick", reset);
    const element = host.current;
    return () => {
      element.removeEventListener("dblclick", reset);
      chart.destroy();
    };
  }, [x, y, label, unit, color, width, range, onZoom]);

  return <div ref={host} className="log-strip" />;
}

export default function LogViewer() {
  const [files, setFiles] = useState<LogFileJson[]>([]);
  const [selected, setSelected] = useState<string | null>(null);
  const [data, setData] = useState<LogDataJson | null>(null);
  const [channels, setChannels] = useState<Set<string>>(new Set(loadChannelPrefs() ?? DEFAULT_CHANNELS));
  const [range, setRange] = useState<[number, number] | null>(null);
  const [reload, setReload] = useState(0);
  const [filter, setFilter] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [width, setWidth] = useState(0);
  const [importing, setImporting] = useState(false);
  const chartsArea = useRef<HTMLDivElement | null>(null);
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

  const onZoom = useCallback((next: [number, number] | null) => {
    setRange((prev) => prev?.[0] === next?.[0] && prev?.[1] === next?.[1] ? prev : next);
  }, []);

  useEffect(() => {
    if (!selected) return;
    const controller = new AbortController();
    const timer = window.setTimeout(() => {
      setLoading(true);
      setError(null);
      api.logData(selected, [...channels], range, 1500, controller.signal)
        .then((d) => { if (!controller.signal.aborted) setData(d); })
        .catch((e: Error) => { if (!controller.signal.aborted) setError(e.message); })
        .finally(() => { if (!controller.signal.aborted) setLoading(false); });
    }, 150);
    return () => { window.clearTimeout(timer); controller.abort(); };
  }, [selected, channels, range, reload]);

  const selectFile = (name: string) => {
    setData(null);
    setRange(null);
    setSelected(name);
    setReload((r) => r + 1);
  };

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
        selectFile(r.name);
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

  // The response contains only requested columns, aligned on valid timestamps.
  const plot = useMemo(() => {
    if (!data) return null;
    const timeCol = data.columns[0] ?? [];
    const keep: number[] = [];
    for (let i = 0; i < timeCol.length; i++) {
      if (timeCol[i] !== null) keep.push(i);
    }
    const x = keep.map((i) => timeCol[i] as number);
    const series = data.column_labels
      .map((label, ci) => ({ label, ci }))
      .filter(({ label, ci }) => ci > 0 && channels.has(label))
      .map(({ label, ci }, order) => ({
        label,
        unit: data.units[data.labels.indexOf(label)] ?? "",
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
          onChange={(e) => selectFile(e.target.value)}
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
        {files.length > 0 && (
          <a
            className="button-link"
            href="/api/logs/download.zip"
            download
            title="Download all saved logs as a ZIP; recordings include data written so far"
          >
            Download all logs
          </a>
        )}
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
            onClick={() => setReload((r) => r + 1)}
          >
            Reload
          </button>
        )}
        {data && (
          <span className="muted">
            {data.rows.toLocaleString()} rows · {data.sampled ? `${plot?.x.length.toLocaleString()} overview points` : `${data.visible_rows.toLocaleString()} full-detail rows`} · drag to zoom · double-click to reset
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
                  range={range ?? data.range}
                  onZoom={onZoom}
                />
              ))
            )}
          </div>
        </div>
      )}
    </div>
  );
}
