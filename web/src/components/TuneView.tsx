// Tuning tab: table picker + grid editor + Burn button.

import { useEffect, useState } from "react";
import { api, type TuneState, type TuneSummary } from "../api";
import type { TelemetryFeed } from "../feed";
import TableEditor from "./TableEditor";

const PREFERRED_TABLES = ["veTable1Tbl", "sparkTbl", "afrTable1Tbl"];

export default function TuneView({
  feed,
  offline,
}: {
  feed: TelemetryFeed;
  offline: boolean;
}) {
  const [summary, setSummary] = useState<TuneSummary | null>(null);
  const [tuneState, setTuneState] = useState<TuneState | null>(null);
  // Last-viewed table survives reloads.
  const [tableId, setTableIdState] = useState<string | null>(() =>
    localStorage.getItem("rustytune-table"),
  );
  const setTableId = (id: string | null) => {
    setTableIdState(id);
    if (id) localStorage.setItem("rustytune-table", id);
  };
  const [error, setError] = useState<string | null>(null);
  const [burning, setBurning] = useState(false);

  useEffect(() => {
    api
      .tune()
      .then((s) => {
        setSummary(s);
        setTuneState(s);
        // A saved pick only counts if this definition still has the table.
        if (
          (!tableId || !s.tables.some((t) => t.id === tableId)) &&
          s.tables.length > 0
        ) {
          const preferred = PREFERRED_TABLES.find((id) =>
            s.tables.some((t) => t.id === id),
          );
          setTableId(preferred ?? s.tables[0].id);
        }
      })
      .catch((e: Error) => setError(e.message));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => feed.onTune(setTuneState), [feed]);

  const burn = () => {
    setBurning(true);
    api
      .burn()
      .then(() => setError(null))
      .catch((e: Error) => setError(e.message))
      .finally(() => setBurning(false));
  };

  if (!summary) {
    return <p className="muted center-note">{error ?? "loading tune…"}</p>;
  }
  if (!(tuneState?.loaded ?? summary.loaded)) {
    return (
      <p className="muted center-note">
        Tune not loaded — connect over USB (primary serial) to edit tables,
        or open a .msq offline from the Tune File tab. SER3 is
        telemetry-only.
      </p>
    );
  }

  return (
    <div className="tune-view">
      <div className="tune-bar">
        <select
          value={tableId ?? ""}
          onChange={(e) => setTableId(e.target.value)}
          aria-label="Table"
        >
          {summary.tables.map((t) => (
            <option key={t.id} value={t.id}>
              {t.title}
            </option>
          ))}
        </select>
        {tuneState?.dirty && (
          <span className="pill sending">
            {offline ? "unsaved changes" : "sending…"}
          </span>
        )}
        {offline ? (
          <a className="button-link" href="/api/msq/save" download>
            Save tune as .msq
          </a>
        ) : (
          <button
            className={tuneState?.burnPending ? "burn-needed" : "ghost"}
            disabled={burning || !tuneState?.burnPending}
            onClick={burn}
          >
            {burning
              ? "Burning…"
              : tuneState?.burnPending
                ? "Burn to ECU"
                : "Burned"}
          </button>
        )}
        {error && <span className="error">{error}</span>}
      </div>
      {tableId && (
        <TableEditor tableId={tableId} feed={feed} onError={setError} />
      )}
    </div>
  );
}
