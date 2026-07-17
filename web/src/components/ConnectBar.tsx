// Header controls: port picker, mode/baud, connect/disconnect, datalog
// start/stop, connection status.

import { useCallback, useEffect, useState } from "react";
import { api, type PortInfo, type Status } from "../api";
import type { TelemetryFeed } from "../feed";

const CUSTOM = "__custom__";
const BAUDS = [115200, 230400, 57600, 38400, 19200, 9600];

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

      {(error || status?.lastError) && (
        <span className="error" role="alert">
          {error ?? status?.lastError}
        </span>
      )}
    </header>
  );
}
