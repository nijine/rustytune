// Header controls: port picker, mode/baud, connect/disconnect, datalog
// start/stop + saved-log browser, connection status.

import { useCallback, useEffect, useRef, useState } from "react";
import { api, type LogListJson, type PortInfo, type Status } from "../api";
import type { TelemetryFeed } from "../feed";

const CUSTOM = "__custom__";
const BAUDS = [115200, 230400, 57600, 38400, 19200, 9600];

function fmtSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

/// "Logs ▾" dropdown: what's in the datalog directory, with download links.
function LogBrowser() {
  const [open, setOpen] = useState(false);
  const [list, setList] = useState<LogListJson | null>(null);
  const [error, setError] = useState<string | null>(null);
  const panel = useRef<HTMLDivElement | null>(null);

  const toggle = () => {
    const next = !open;
    setOpen(next);
    if (next) {
      api
        .logs()
        .then((l) => {
          setList(l);
          setError(null);
        })
        .catch((e: Error) => setError(e.message));
    }
  };

  useEffect(() => {
    if (!open) return;
    const close = (e: MouseEvent) => {
      if (panel.current && !panel.current.contains(e.target as Node)) {
        setOpen(false);
      }
    };
    document.addEventListener("mousedown", close);
    return () => document.removeEventListener("mousedown", close);
  }, [open]);

  return (
    <div className="log-browser" ref={panel}>
      <button className="ghost" onClick={toggle}>
        Logs ▾
      </button>
      {open && (
        <div className="log-panel">
          {error && <p className="error">{error}</p>}
          {list && (
            <>
              <p className="log-dir" title={list.dir}>
                stored in {list.dir}
              </p>
              {list.files.length === 0 ? (
                <p className="muted">
                  No logs yet — hit ● Log while connected.
                </p>
              ) : (
                <ul>
                  {list.files.map((f) => (
                    <li key={f.name}>
                      <a
                        href={`/api/logs/${encodeURIComponent(f.name)}`}
                        download
                      >
                        {f.name}
                      </a>
                      <span className="log-meta">
                        {f.active ? (
                          <span className="log-recording">● recording</span>
                        ) : (
                          `${fmtSize(f.size)} · ${f.modified}`
                        )}
                      </span>
                    </li>
                  ))}
                </ul>
              )}
            </>
          )}
        </div>
      )}
    </div>
  );
}

export default function ConnectBar({
  status,
  feed,
}: {
  status: Status | null;
  feed: TelemetryFeed;
}) {
  const [ports, setPorts] = useState<PortInfo[]>([]);
  const [port, setPort] = useState("");
  const [customPort, setCustomPort] = useState("");
  const [mode, setMode] = useState("primary");
  const [baud, setBaud] = useState(115200);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [logRows, setLogRows] = useState<number | null>(null);

  const refreshPorts = useCallback(() => {
    api
      .ports()
      .then((list) => {
        setPorts(list);
        setPort((current) =>
          current === "" ? (list[0]?.path ?? CUSTOM) : current,
        );
      })
      .catch(() => setPorts([]));
  }, []);

  useEffect(refreshPorts, [refreshPorts]);

  // Live row counter while logging rides along on telemetry frames.
  useEffect(
    () => feed.onFrame((frame) => setLogRows(frame.logRows ?? null)),
    [feed],
  );

  const connected = status?.connected ?? false;
  const logging = Boolean(status?.log);

  const run = (op: () => Promise<unknown>) => {
    setBusy(true);
    setError(null);
    op()
      .catch((e: Error) => setError(e.message))
      .finally(() => setBusy(false));
  };

  const chosenPort = port === CUSTOM ? customPort : port;

  return (
    <header className="bar">
      <div className="brand">
        <h1>rustytune</h1>
        <span
          className={`dot ${connected ? "ok" : ""}`}
          title={connected ? "connected" : "disconnected"}
        />
        {status?.offline && (
          <span className="offline-pill" title="Editing a .msq with no ECU">
            offline tune
          </span>
        )}
      </div>

      {!connected && (
        <div className="controls">
          <select
            value={port}
            onChange={(e) => setPort(e.target.value)}
            aria-label="Serial port"
          >
            {ports.map((p) => (
              <option key={p.path} value={p.path}>
                {p.path}
              </option>
            ))}
            <option value={CUSTOM}>other…</option>
          </select>
          {port === CUSTOM && (
            <input
              placeholder="/dev/…"
              value={customPort}
              onChange={(e) => setCustomPort(e.target.value)}
              aria-label="Custom port path"
            />
          )}
          <button className="ghost" onClick={refreshPorts} title="Rescan ports">
            ⟳
          </button>
          <select
            value={mode}
            onChange={(e) => setMode(e.target.value)}
            aria-label="Connection mode"
          >
            <option value="primary">USB (primary)</option>
            <option value="secondary">SER3 (secondary)</option>
          </select>
          <select
            value={baud}
            onChange={(e) => setBaud(Number(e.target.value))}
            aria-label="Baud rate"
          >
            {BAUDS.map((b) => (
              <option key={b} value={b}>
                {b}
              </option>
            ))}
          </select>
          <button
            className="primary"
            disabled={busy || !chosenPort}
            onClick={() => run(() => api.connect(chosenPort, mode, baud))}
          >
            Connect
          </button>
        </div>
      )}

      {connected && (
        <div className="controls">
          <span className="portname">
            {status?.port} · {status?.mode === "secondary" ? "SER3" : "USB"}
          </span>
          <button
            className={logging ? "danger" : "primary"}
            disabled={busy}
            onClick={() =>
              run(logging ? api.logStop : api.logStart)
            }
          >
            {logging
              ? `■ Stop log${logRows !== null ? ` (${logRows})` : ""}`
              : "● Log"}
          </button>
          <button
            className="ghost"
            disabled={busy}
            onClick={() => run(api.disconnect)}
          >
            Disconnect
          </button>
        </div>
      )}

      <LogBrowser />

      {(error || status?.lastError) && (
        <span className="error" role="alert">
          {error ?? status?.lastError}
        </span>
      )}
    </header>
  );
}
