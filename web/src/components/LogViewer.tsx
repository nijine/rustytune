// Log viewer tab: pick a recorded .msl and read it as stacked uPlot strip
// charts with left/right scales, cursor and x-zoom synced across all of them.
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

function formatFileSize(bytes: number): string {
  const units = ["B", "KiB", "MiB", "GiB", "TiB"];
  let size = bytes;
  let unit = 0;
  while (size >= 1024 && unit < units.length - 1) {
    size /= 1024;
    unit++;
  }
  return `${unit === 0 ? size : size.toFixed(1)} ${units[unit]}`;
}

type Side = "left" | "right";
type Graph = { id: string; channels: { label: string; side: Side }[] };
type PlotSeries = { label: string; unit: string; color: string; y: (number | null)[]; side: Side };
const GRAPH_STORE_KEY = "rustytune-log-graphs";

function loadGraphPrefs(): Graph[] {
  try {
    const raw = localStorage.getItem(GRAPH_STORE_KEY);
    if (raw) {
      const parsed: unknown = JSON.parse(raw);
      if (Array.isArray(parsed) && parsed.every((g) =>
        g && typeof g.id === "string" && Array.isArray(g.channels) &&
        g.channels.every((c: { label?: unknown; side?: unknown }) =>
          c && typeof c.label === "string" && (c.side === "left" || c.side === "right"))
      ) && new Set(parsed.map((g) => g.id)).size === parsed.length &&
        parsed.every((g) => new Set(g.channels.map((c: { label: string }) => c.label)).size === g.channels.length)) return parsed;
    }
    const old = localStorage.getItem(STORE_KEY);
    const labels: unknown = old ? JSON.parse(old) : DEFAULT_CHANNELS;
    if (Array.isArray(labels) && labels.every((l) => typeof l === "string")) {
      return [...new Set(labels)].map((label, i) => ({ id: String(i), channels: [{ label, side: "left" }] }));
    }
  } catch { /* Fall back to the default layout if preferences are unavailable. */ }
  return DEFAULT_CHANNELS.map((label, i) => ({ id: String(i), channels: [{ label, side: "left" }] }));
}

function GraphChannelsDialog({ graph, number, data, onSave, onClose }: {
  graph: Graph;
  number: number;
  data: LogDataJson;
  onSave: (channels: Graph["channels"]) => void;
  onClose: () => void;
}) {
  const dialog = useRef<HTMLDialogElement | null>(null);
  const [draft, setDraft] = useState(graph.channels);
  const [filter, setFilter] = useState("");
  useEffect(() => {
    const element = dialog.current;
    element?.showModal();
    return () => { element?.close(); };
  }, []);

  const unitOf = (label: string) => data.units[data.labels.indexOf(label)] ?? "";
  const toggle = (label: string) => setDraft((prev) => {
    if (prev.some((c) => c.label === label)) return prev.filter((c) => c.label !== label);
    const matching = unitOf(label) ? prev.find((c) => unitOf(c.label) === unitOf(label)) : undefined;
    const side = matching?.side ?? (prev.some((c) => c.side === "left") ? "right" : "left");
    return [...prev, { label, side }];
  });
  // Include saved channels absent from this file so they can still be removed.
  const labels = [...new Set([...data.labels.slice(1), ...draft.map((c) => c.label)])];
  const shown = labels.filter((label) => label.toLowerCase().includes(filter.trim().toLowerCase()));

  return <dialog ref={dialog} className="log-channel-dialog" aria-labelledby="log-channel-dialog-title"
    onCancel={onClose} onClose={onClose}
    onClick={(e) => {
      if (e.target !== e.currentTarget) return;
      const bounds = e.currentTarget.getBoundingClientRect();
      if (e.clientX < bounds.left || e.clientX > bounds.right || e.clientY < bounds.top || e.clientY > bounds.bottom) onClose();
    }}>
    <div className="log-channel-dialog-header">
      <h3 id="log-channel-dialog-title">Graph {number} channels</h3>
      <button className="ghost" onClick={onClose} aria-label="Close channel picker">×</button>
    </div>
    <p className="muted">Select channels and choose their axis. Left and right scales adjust independently; put comparable units on the same axis.</p>
    <input autoFocus type="search" aria-label="Filter channels" placeholder="Filter channels…"
      value={filter} onChange={(e) => setFilter(e.target.value)} />
    <div className="log-channel-dialog-list">
      {shown.map((label) => {
        const channel = draft.find((c) => c.label === label);
        const available = data.labels.includes(label);
        return <div key={label} className="log-channel-row">
          <label>
            <input type="checkbox" checked={!!channel} onChange={() => toggle(label)} />
            <span>{label}{unitOf(label) && <span className="muted"> ({unitOf(label)})</span>}
              {!available && <span className="muted"> · unavailable in this log</span>}</span>
          </label>
          <select aria-label={`Axis for ${label}`} disabled={!channel} value={channel?.side ?? "left"}
            onChange={(e) => setDraft((prev) => prev.map((c) => c.label === label ? { ...c, side: e.target.value as Side } : c))}>
            <option value="left">Left axis</option>
            <option value="right">Right axis</option>
          </select>
        </div>;
      })}
      {shown.length === 0 && <p className="muted center-note">No channels match your filter.</p>}
    </div>
    <div className="log-channel-dialog-actions">
      <span className="muted">{draft.length} selected</span>
      <button className="ghost" onClick={onClose}>Cancel</button>
      <button onClick={() => { onSave(draft); onClose(); }}>Apply</button>
    </div>
  </dialog>;
}

// Cursor sync uses uPlot; shared range state keeps every strip at the same zoom.
function Strip({
  x,
  series,
  width,
  range,
  onZoom,
}: {
  x: number[];
  series: PlotSeries[];
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
      height: 200,
      scales: {
        x: { time: false, ...(range ? { min: range[0], max: range[1] } : {}) },
        left: { auto: true },
        right: { auto: true },
      },
      cursor: {
        sync: { key: "rustytune-log" },
        drag: { x: true, y: false },
      },
      series: [
        { label: "Time (s)", value: (_u, v) => (v === null ? "–" : v.toFixed(2)) },
        ...series.map((s) => ({
          label: s.unit ? `${s.label} (${s.unit})` : s.label,
          scale: s.side,
          stroke: s.color,
          width: 1.5,
          points: { show: false },
          value: (_u: uPlot, v: number | null) => (v == null ? "–" : String(v)),
        })),
      ],
      axes: [
        {
          stroke: "#898781",
          grid: { stroke: "#2c2c2a", width: 1 },
          ticks: { stroke: "#2c2c2a" },
        },
        ...(["left", "right"] as const).map((side): uPlot.Axis => ({
          scale: side,
          side: side === "left" ? 3 : 1,
          show: series.some((s) => s.side === side),
          stroke: "#898781",
          grid: { show: side === "left", stroke: "#2c2c2a", width: 1 },
          ticks: { stroke: "#2c2c2a" },
          size: 64,
        })),
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
    const chart = new uPlot(opts, [x, ...series.map((s) => s.y)] as uPlot.AlignedData, host.current);
    ready = true;
    const reset = () => onZoom(null);
    host.current.addEventListener("dblclick", reset);
    const element = host.current;
    return () => {
      element.removeEventListener("dblclick", reset);
      chart.destroy();
    };
  }, [x, series, width, range, onZoom]);

  return <div ref={host} className="log-strip" />;
}

export default function LogViewer() {
  const [files, setFiles] = useState<LogFileJson[]>([]);
  const [selected, setSelected] = useState<string | null>(null);
  const [data, setData] = useState<LogDataJson | null>(null);
  const [graphs, setGraphs] = useState<Graph[]>(loadGraphPrefs);
  const channels = useMemo(() => new Set(graphs.flatMap((g) => g.channels.map((c) => c.label))), [graphs]);
  useEffect(() => {
    try { localStorage.setItem(GRAPH_STORE_KEY, JSON.stringify(graphs)); } catch { /* Storage may be disabled. */ }
  }, [graphs]);
  const [range, setRange] = useState<[number, number] | null>(null);
  const [reload, setReload] = useState(0);
  const [editingGraph, setEditingGraph] = useState<string | null>(null);
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
    setEditingGraph(null);
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

  const addGraph = () => {
    const id = `${Date.now()}-${Math.random().toString(36).slice(2)}`;
    setGraphs((prev) => [...prev, { id, channels: [] }]);
    setEditingGraph(id);
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

  const activeGraph = graphs.find((g) => g.id === editingGraph);

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
              {f.name} ({formatFileSize(f.size)})
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
        <div className="log-charts" ref={chartsArea}>
          <button className="ghost" onClick={addGraph}>Add graph</button>
          {graphs.length === 0 && <p className="muted center-note">Add a graph to start plotting channels.</p>}
          {graphs.map((graph, i) => {
            const series = graph.channels.flatMap((c) => {
              const s = plot.series.find((s) => s.label === c.label);
              return s ? [{ ...s, side: c.side }] : [];
            });
            return <section key={`${data.name}:${graph.id}`} className="log-graph">
              {series.length ? <>
                <div className="log-scale-labels">
                  {(["left", "right"] as const).map((side) => <span key={side}>
                    {series.filter((s) => s.side === side).map((s, i) =>
                      <span key={s.label}>{i > 0 && " · "}<span style={{ color: s.color }}>{s.label}{s.unit ? ` (${s.unit})` : ""}</span></span>)}
                  </span>)}
                </div>
                <Strip x={plot.x} series={series} width={Math.max(0, width - 18)} range={range ?? data.range} onZoom={onZoom} />
              </> : <p className="muted center-note">Use Channels to choose what to show in this graph.</p>}
              <div className="log-graph-footer">
                <strong>Graph {i + 1}</strong>
                <div className="log-graph-actions">
                  <button className="ghost" onClick={() => setEditingGraph(graph.id)}
                    aria-label={`Choose channels for graph ${i + 1}`}>Channels</button>
                  <button className="ghost" onClick={() => setGraphs((prev) => prev.filter((g) => g.id !== graph.id))}
                    aria-label={`Remove graph ${i + 1}`}>Remove</button>
                </div>
              </div>
            </section>;
          })}
        </div>
      )}
      {data && activeGraph && <GraphChannelsDialog key={`${data.name}:${activeGraph.id}`}
        graph={activeGraph} number={graphs.indexOf(activeGraph) + 1} data={data}
        onSave={(channels) => setGraphs((prev) => prev.map((g) => g.id === activeGraph.id ? { ...g, channels } : g))}
        onClose={() => setEditingGraph(null)} />}
    </div>
  );
}
