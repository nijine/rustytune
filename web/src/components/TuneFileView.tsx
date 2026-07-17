// Tune File tab: open a TunerStudio .msq, see exactly what differs from
// the ECU (and where), selectively push file values, save the ECU state
// back out as .msq.

import { useCallback, useEffect, useRef, useState } from "react";
import { api, type DiffEntryJson, type MsqDiffJson } from "../api";
import type { TelemetryFeed } from "../feed";

function fmt(v: number | string | null | undefined): string {
  if (v === null || v === undefined) return "–";
  if (typeof v === "string") return v;
  return Number.isInteger(v) ? String(v) : v.toFixed(3).replace(/\.?0+$/, "");
}

function CellsDetail({ entry }: { entry: DiffEntryJson }) {
  const cells = entry.cells ?? [];
  const shown = cells.slice(0, 8);
  const more = (entry.changedCount ?? 0) - shown.length;
  return (
    <span className="cells-detail">
      {shown.map((c) => (
        <span key={c.index} className="cell-change">
          {c.row !== undefined ? `[${c.row},${c.col}]` : `[${c.index}]`}{" "}
          {fmt(c.ecu)}→{fmt(c.file)}
        </span>
      ))}
      {more > 0 && <span className="muted"> +{more} more</span>}
    </span>
  );
}

export default function TuneFileView({
  feed,
  tuneLoaded,
}: {
  feed: TelemetryFeed;
  tuneLoaded: boolean;
}) {
  const [diff, setDiff] = useState<MsqDiffJson | null>(null);
  const [checked, setChecked] = useState<Set<string>>(new Set());
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [notice, setNotice] = useState<string | null>(null);
  const refreshTimer = useRef<ReturnType<typeof setTimeout> | null>(null);

  const refresh = useCallback(() => {
    api
      .msqDiff()
      .then((d) => {
        setDiff(d);
        setError(null);
        // Preselect everything that differs.
        setChecked(new Set(d.entries.map((e) => e.name)));
      })
      .catch((e: Error) => {
        if (!e.message.includes("no .msq uploaded")) setError(e.message);
      });
  }, []);

  // The server may already hold an uploaded file (page reload); pick it up.
  useEffect(refresh, [refresh]);

  // Edits elsewhere (table editor, another client) change the diff; refetch
  // shortly after the dust settles.
  useEffect(
    () =>
      feed.onTune(() => {
        if (!diff) return;
        if (refreshTimer.current) clearTimeout(refreshTimer.current);
        refreshTimer.current = setTimeout(refresh, 600);
      }),
    [feed, diff, refresh],
  );

  const openFile = async (file: File) => {
    setBusy(true);
    setNotice(null);
    try {
      const buffer = await file.arrayBuffer();
      const content = new TextDecoder("iso-8859-1").decode(buffer);
      await api.msqUpload(file.name, content);
      refresh();
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setBusy(false);
    }
  };

  const apply = (names?: string[]) => {
    setBusy(true);
    setNotice(null);
    api
      .msqApply(names)
      .then((report) => {
        setNotice(
          `applied ${report.applied} constants` +
            (report.skipped.length ? `, ${report.skipped.length} skipped` : ""),
        );
        refresh();
      })
      .catch((e: Error) => setError(e.message))
      .finally(() => setBusy(false));
  };

  const toggle = (name: string) => {
    setChecked((prev) => {
      const next = new Set(prev);
      if (next.has(name)) next.delete(name);
      else next.add(name);
      return next;
    });
  };

  if (!tuneLoaded) {
    return (
      <p className="muted center-note">
        Tune not loaded — connect over USB (primary serial) to compare and
        apply tune files.
      </p>
    );
  }

  return (
    <div className="tunefile">
      <div className="tune-bar">
        <label className="file-btn">
          Open .msq…
          <input
            type="file"
            accept=".msq,application/xml"
            onChange={(e) => {
              const f = e.target.files?.[0];
              if (f) void openFile(f);
              e.target.value = "";
            }}
          />
        </label>
        <a className="button-link" href="/api/msq/save" download>
          Save ECU tune as .msq
        </a>
        {diff && (
          <button className="ghost" disabled={busy} onClick={refresh}>
            ⟳ Refresh diff
          </button>
        )}
        {error && <span className="error">{error}</span>}
        {notice && <span className="muted">{notice}</span>}
      </div>

      {diff && (
        <>
          <div className="msq-meta">
            <span>
              <strong>{diff.meta.filename}</strong>
              {diff.meta.writeDate ? ` · saved ${diff.meta.writeDate}` : ""}
              {diff.meta.author ? ` · ${diff.meta.author}` : ""}
            </span>
            {!diff.meta.signatureMatch && (
              <span className="warn-pill">
                file is for “{diff.meta.signature}” — this ECU definition
                differs; unmatched settings are listed below
              </span>
            )}
          </div>

          {diff.entries.length === 0 ? (
            <p className="ok-note">
              ✓ No differences — the ECU matches the file
              {diff.onlyInFile.length > 0
                ? ` (${diff.onlyInFile.length} file-only settings not comparable)`
                : ""}
              .
            </p>
          ) : (
            <>
              <div className="diff-actions">
                <span>
                  {diff.entries.length} difference
                  {diff.entries.length === 1 ? "" : "s"}
                </span>
                <button
                  className="ghost"
                  onClick={() =>
                    setChecked(new Set(diff.entries.map((e) => e.name)))
                  }
                >
                  all
                </button>
                <button className="ghost" onClick={() => setChecked(new Set())}>
                  none
                </button>
                <button
                  className="primary"
                  disabled={busy || checked.size === 0}
                  onClick={() => apply([...checked])}
                >
                  Apply {checked.size} selected to ECU
                </button>
              </div>
              <div className="diff-scroll">
                <table className="diff-table">
                  <thead>
                    <tr>
                      <th></th>
                      <th>Setting</th>
                      <th>Where</th>
                      <th>ECU → File</th>
                    </tr>
                  </thead>
                  <tbody>
                    {diff.entries.map((entry) => (
                      <tr key={entry.name}>
                        <td>
                          <input
                            type="checkbox"
                            checked={checked.has(entry.name)}
                            onChange={() => toggle(entry.name)}
                          />
                        </td>
                        <td className="mono">{entry.name}</td>
                        <td>{entry.where}</td>
                        <td>
                          {entry.kind === "array" ? (
                            <>
                              <span>
                                {entry.changedCount}/{entry.len} values differ:{" "}
                              </span>
                              <CellsDetail entry={entry} />
                            </>
                          ) : (
                            <span className="mono">
                              {fmt(entry.ecu)} → {fmt(entry.file)}
                            </span>
                          )}
                        </td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            </>
          )}

          {(diff.onlyInFile.length > 0 || diff.unresolved.length > 0) && (
            <details className="diff-extra">
              <summary>
                {diff.onlyInFile.length} file-only ·{" "}
                {diff.unresolved.length} not comparable
              </summary>
              {diff.onlyInFile.length > 0 && (
                <p className="muted">
                  Not in this ECU definition: {diff.onlyInFile.join(", ")}
                </p>
              )}
              {diff.unresolved.length > 0 && (
                <p className="muted">
                  {diff.unresolved
                    .map(([name, why]) => `${name} (${why})`)
                    .join(", ")}
                </p>
              )}
            </details>
          )}
        </>
      )}

      {!diff && (
        <p className="muted center-note">
          Open a .msq tune file to compare it against what's on the ECU.
        </p>
      )}
    </div>
  );
}
