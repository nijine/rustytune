import { useEffect, useMemo, useState } from "react";
import { api, type Definition, type Status } from "./api";
import { TelemetryFeed } from "./feed";
import ConnectBar from "./components/ConnectBar";
import Gauge from "./components/Gauge";
import Indicators from "./components/Indicators";

export default function App() {
  const feed = useMemo(() => new TelemetryFeed(), []);
  const [definition, setDefinition] = useState<Definition | null>(null);
  const [status, setStatus] = useState<Status | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);

  useEffect(() => {
    api
      .definition()
      .then(setDefinition)
      .catch((e: Error) => setLoadError(e.message));
    api.status().then(setStatus).catch(() => {});
    feed.start();
    const off = feed.onStatus(setStatus);
    return () => {
      off();
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
          <Indicators defs={definition.indicators} feed={feed} />
          <div className="gauges">
            {definition.gauges.map((g) => (
              <Gauge key={g.name} def={g} feed={feed} />
            ))}
          </div>
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
