import { useEffect, useMemo, useState } from "react";
import { api, type Definition, type Status } from "./api";
import { TelemetryFeed } from "./feed";
import ConnectBar from "./components/ConnectBar";
import Gauge from "./components/Gauge";
import Indicators from "./components/Indicators";
import LogViewer from "./components/LogViewer";
import SettingsView from "./components/SettingsView";
import TuneFileView from "./components/TuneFileView";
import TuneView from "./components/TuneView";

type Tab = "dash" | "tune" | "settings" | "file" | "logs";
const TABS: [Tab, string][] = [
  ["dash", "Dashboard"],
  ["tune", "Tuning"],
  ["settings", "Settings"],
  ["file", "Tune File"],
  ["logs", "Log Viewer"],
];

export default function App() {
  const feed = useMemo(() => new TelemetryFeed(), []);
  const [definition, setDefinition] = useState<Definition | null>(null);
  const [status, setStatus] = useState<Status | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  // The active tab survives reloads (layout persistence).
  const [tab, setTabState] = useState<Tab>(() => {
    const saved = localStorage.getItem("rustytune-tab");
    return TABS.some(([t]) => t === saved) ? (saved as Tab) : "dash";
  });
  const setTab = (t: Tab) => {
    setTabState(t);
    localStorage.setItem("rustytune-tab", t);
  };

  useEffect(() => {
    api
      .definition()
      .then(setDefinition)
      .catch((e: Error) => setLoadError(e.message));
    api.status().then(setStatus).catch(() => {});
    feed.start();
    const offStatus = feed.onStatus(setStatus);
    // Editing gauge limits (PcVariables) re-resolves gauge bounds live.
    const offDefinition = feed.onDefinition(setDefinition);
    return () => {
      offStatus();
      offDefinition();
      feed.stop();
    };
  }, [feed]);

  return (
    <div className="app">
      <ConnectBar status={status} feed={feed} />

      {loadError && (
        <p className="error center">definition failed to load: {loadError}</p>
      )}

      {definition && (
        <main>
          <nav className="tabs">
            {TABS.map(([id, label]) => (
              <button
                key={id}
                className={tab === id ? "tab active" : "tab"}
                onClick={() => setTab(id)}
              >
                {label}
              </button>
            ))}
          </nav>
          {tab === "dash" && (
            <>
              <Indicators defs={definition.indicators} feed={feed} />
              <div className="gauges">
                {definition.gauges.map((g) => (
                  <Gauge key={g.name} def={g} feed={feed} />
                ))}
              </div>
            </>
          )}
          {tab === "tune" && (
            <TuneView feed={feed} offline={status?.offline ?? false} />
          )}
          {tab === "settings" && (
            <SettingsView
              feed={feed}
              tuneLoaded={status?.tuneLoaded ?? false}
              offline={status?.offline ?? false}
            />
          )}
          {tab === "file" && (
            <TuneFileView
              feed={feed}
              tuneLoaded={status?.tuneLoaded ?? false}
              offline={status?.offline ?? false}
            />
          )}
          {tab === "logs" && <LogViewer />}
          <footer>
            <span>{definition.signature}</span>
            {status && status.connected && (
              <span>
                {status.frames} frames · {status.crcErrors} CRC ·{" "}
                {status.timeouts} timeouts
              </span>
            )}
          </footer>
        </main>
      )}
    </div>
  );
}
