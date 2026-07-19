import { useEffect, useMemo, useState } from "react";
import { api, type Definition, type Status } from "./api";
import { TelemetryFeed } from "./feed";
import ConnectBar from "./components/ConnectBar";
import Gauge from "./components/Gauge";
import GaugeTile from "./components/GaugeTile";
import Indicators from "./components/Indicators";
import LogViewer from "./components/LogViewer";
import SettingsView from "./components/SettingsView";
import TuneFileView from "./components/TuneFileView";
import TuneView from "./components/TuneView";

type Tab = "dash" | "tune" | "settings" | "file" | "logs";

/// Dashboard look preferences, persisted per browser.
interface DashPrefs {
  gauges: "dials" | "tiles";
  indicators: "all" | "active";
}

const DASH_PREFS_KEY = "rustytune-dash-prefs";

function loadDashPrefs(): DashPrefs {
  try {
    const raw = localStorage.getItem(DASH_PREFS_KEY);
    const p = raw ? (JSON.parse(raw) as Partial<DashPrefs>) : {};
    return {
      gauges: p.gauges === "tiles" ? "tiles" : "dials",
      indicators: p.indicators === "active" ? "active" : "all",
    };
  } catch {
    return { gauges: "dials", indicators: "all" };
  }
}

function Seg<T extends string>({
  value,
  options,
  onChange,
}: {
  value: T;
  options: [T, string][];
  onChange: (v: T) => void;
}) {
  return (
    <span className="seg">
      {options.map(([v, label]) => (
        <button
          key={v}
          className={value === v ? "on" : ""}
          onClick={() => onChange(v)}
        >
          {label}
        </button>
      ))}
    </span>
  );
}
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
  const [dashPrefs, setDashPrefs] = useState<DashPrefs>(loadDashPrefs);
  const updateDashPrefs = (patch: Partial<DashPrefs>) => {
    setDashPrefs((prev) => {
      const next = { ...prev, ...patch };
      localStorage.setItem(DASH_PREFS_KEY, JSON.stringify(next));
      return next;
    });
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
              <div className="dash-options">
                <label>
                  Gauges
                  <Seg
                    value={dashPrefs.gauges}
                    options={[
                      ["dials", "Dials"],
                      ["tiles", "Tiles"],
                    ]}
                    onChange={(gauges) => updateDashPrefs({ gauges })}
                  />
                </label>
                <label>
                  Status
                  <Seg
                    value={dashPrefs.indicators}
                    options={[
                      ["all", "All"],
                      ["active", "Active only"],
                    ]}
                    onChange={(indicators) => updateDashPrefs({ indicators })}
                  />
                </label>
              </div>
              <Indicators
                defs={definition.indicators}
                feed={feed}
                mode={dashPrefs.indicators}
              />
              {dashPrefs.gauges === "dials" ? (
                <div className="gauges">
                  {definition.gauges.map((g) => (
                    <Gauge key={g.name} def={g} feed={feed} />
                  ))}
                </div>
              ) : (
                <div className="gauge-tiles">
                  {definition.gauges.map((g) => (
                    <GaugeTile key={g.name} def={g} feed={feed} />
                  ))}
                </div>
              )}
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
