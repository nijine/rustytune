import { useEffect, useMemo, useState } from "react";
import { api, type Definition, type Status } from "./api";
import { TelemetryFeed } from "./feed";
import ConnectBar from "./components/ConnectBar";
import Gauge from "./components/Gauge";
import Indicators from "./components/Indicators";
import SettingsView from "./components/SettingsView";
import TuneFileView from "./components/TuneFileView";
import TuneView from "./components/TuneView";

export default function App() {
  const feed = useMemo(() => new TelemetryFeed(), []);
  const [definition, setDefinition] = useState<Definition | null>(null);
  const [status, setStatus] = useState<Status | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [tab, setTab] = useState<"dash" | "tune" | "settings" | "file">(
    "dash",
  );

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
            <button
              className={tab === "dash" ? "tab active" : "tab"}
              onClick={() => setTab("dash")}
            >
              Dashboard
            </button>
            <button
              className={tab === "tune" ? "tab active" : "tab"}
              onClick={() => setTab("tune")}
            >
              Tuning
            </button>
            <button
              className={tab === "settings" ? "tab active" : "tab"}
              onClick={() => setTab("settings")}
            >
              Settings
            </button>
            <button
              className={tab === "file" ? "tab active" : "tab"}
              onClick={() => setTab("file")}
            >
              Tune File
            </button>
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
          {tab === "tune" && <TuneView feed={feed} />}
          {tab === "settings" && (
            <SettingsView feed={feed} tuneLoaded={status?.tuneLoaded ?? false} />
          )}
          {tab === "file" && (
            <TuneFileView feed={feed} tuneLoaded={status?.tuneLoaded ?? false} />
          )}
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
