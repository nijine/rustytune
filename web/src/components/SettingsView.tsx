// Settings tab: the INI's own [Menu]/[UserDefined] dialogs rendered as
// forms — acceleration enrichment, idle control, fan control, ... Every
// field writes through the same constant endpoint as the Tuning tab, so
// dirty tracking, flushing, and the Burn indicator behave identically.

import { useCallback, useEffect, useRef, useState } from "react";
import {
  api,
  type ConstantJson,
  type DialogItemJson,
  type DialogJson,
  type MenuEntryJson,
  type MenuJson,
  type TuneState,
} from "../api";
import type { TelemetryFeed } from "../feed";

function ConstantRow({
  label,
  enabled,
  constant: c,
  onCommit,
}: {
  label: string;
  enabled: boolean;
  constant: ConstantJson;
  onCommit: (c: ConstantJson, value: number) => void;
}) {
  const [draft, setDraft] = useState<string | null>(null);

  const commit = (raw: string) => {
    setDraft(null);
    const value = Number(raw);
    if (!Number.isFinite(value) || value === c.value) return;
    onCommit(c, value);
  };

  return (
    <label className={`dlg-row${enabled ? "" : " disabled"}`}>
      <span className="dlg-label">
        {label}
        {c.requiresPowerCycle ? " ⚡" : ""}
      </span>
      {c.labels.length > 0 ? (
        <select
          disabled={!enabled}
          value={String(c.value)}
          onChange={(e) => commit(e.target.value)}
        >
          {c.labels.map((option, i) =>
            option === "INVALID" ? null : (
              <option key={i} value={i}>
                {option}
              </option>
            ),
          )}
        </select>
      ) : (
        <input
          disabled={!enabled}
          value={draft ?? Number(c.value).toFixed(c.digits)}
          onChange={(e) => setDraft(e.target.value)}
          onBlur={(e) => commit(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") (e.target as HTMLInputElement).blur();
          }}
        />
      )}
      <span className="units">{c.units ?? ""}</span>
    </label>
  );
}

function DialogItems({
  items,
  onCommit,
}: {
  items: DialogItemJson[];
  onCommit: (c: ConstantJson, value: number) => void;
}) {
  return (
    <>
      {items.map((item, i) => {
        switch (item.type) {
          case "header":
            return (
              <h4 key={i} className="dlg-header">
                {item.label}
              </h4>
            );
          case "constant":
            return (
              <ConstantRow
                key={item.constant.name}
                label={item.label}
                enabled={item.enabled}
                constant={item.constant}
                onCommit={onCommit}
              />
            );
          case "panel":
            return (
              <fieldset
                key={`${item.name}-${i}`}
                className={`dlg-panel${item.enabled ? "" : " disabled"}`}
              >
                {item.title && <legend>{item.title}</legend>}
                <DialogItems items={item.items} onCommit={onCommit} />
              </fieldset>
            );
          case "curve":
          case "table":
            return (
              <p key={`${item.name}-${i}`} className="muted dlg-ref">
                ⤷ {item.title || item.name} —{" "}
                {item.type === "table"
                  ? "edit in the Tuning tab"
                  : "curve editor not yet available"}
              </p>
            );
          case "unsupported":
            return (
              <div key={`${item.name}-${i}`} className="dlg-row disabled">
                <span className="dlg-label">{item.label}</span>
                <span className="muted">–</span>
              </div>
            );
        }
      })}
    </>
  );
}

function MenuEntryButton({
  entry,
  selected,
  onSelect,
}: {
  entry: MenuEntryJson;
  selected: boolean;
  onSelect: (name: string) => void;
}) {
  if (entry.type !== "dialog") return null;
  return (
    <button
      className={`menu-entry${selected ? " selected" : ""}`}
      disabled={!entry.enabled}
      onClick={() => onSelect(entry.name)}
    >
      {entry.label}
    </button>
  );
}

export default function SettingsView({
  feed,
  tuneLoaded,
  offline,
}: {
  feed: TelemetryFeed;
  tuneLoaded: boolean;
  offline: boolean;
}) {
  const [menus, setMenus] = useState<MenuJson[] | null>(null);
  const [selected, setSelected] = useState<string | null>(null);
  const [dialog, setDialog] = useState<DialogJson | null>(null);
  const [tuneState, setTuneState] = useState<TuneState | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [burning, setBurning] = useState(false);
  const refreshTimer = useRef<ReturnType<typeof setTimeout> | null>(null);

  const loadMenus = useCallback(() => {
    api.menus().then(setMenus).catch(() => {});
  }, []);

  const loadDialog = useCallback((name: string) => {
    api
      .dialog(name)
      .then((d) => {
        setDialog(d);
        setError(null);
      })
      .catch((e: Error) => setError(e.message));
  }, []);

  useEffect(() => {
    if (tuneLoaded) loadMenus();
  }, [tuneLoaded, loadMenus]);

  // Edits change enable conditions (e.g. picking an idle algorithm swaps
  // which panels apply) — refetch after the dust settles.
  useEffect(
    () =>
      feed.onTune((t) => {
        setTuneState(t);
        if (refreshTimer.current) clearTimeout(refreshTimer.current);
        refreshTimer.current = setTimeout(() => {
          loadMenus();
          if (selected) loadDialog(selected);
        }, 600);
      }),
    [feed, selected, loadMenus, loadDialog],
  );

  const select = (name: string) => {
    setSelected(name);
    setNotice(null);
    loadDialog(name);
  };

  const commit = (c: ConstantJson, value: number) => {
    api
      .setConstant(c.name, value)
      .then((updated) => {
        setNotice(
          updated.requiresPowerCycle
            ? `${updated.name} requires an ECU power cycle to take effect`
            : null,
        );
        setError(null);
        if (selected) loadDialog(selected);
      })
      .catch((e: Error) => setError(e.message));
  };

  const burn = () => {
    setBurning(true);
    api
      .burn()
      .then(() => setError(null))
      .catch((e: Error) => setError(e.message))
      .finally(() => setBurning(false));
  };

  if (!tuneLoaded) {
    return (
      <p className="muted center-note">
        Tune not loaded — connect over USB (primary serial) to edit
        settings, or open a .msq offline from the Tune File tab.
      </p>
    );
  }

  return (
    <div className="settings-view">
      <aside className="settings-menu">
        {(menus ?? []).map((menu) => (
          <div key={menu.title} className="menu-section">
            <h3>{menu.title}</h3>
            {menu.items.map((item, i) => {
              switch (item.type) {
                case "separator":
                  return <hr key={i} />;
                case "group":
                  return (
                    <div key={item.label} className="menu-group">
                      <span className="menu-group-label">{item.label}</span>
                      {item.items.map((child) => (
                        <MenuEntryButton
                          key={child.name}
                          entry={child}
                          selected={selected === child.name}
                          onSelect={select}
                        />
                      ))}
                    </div>
                  );
                default:
                  return (
                    <MenuEntryButton
                      key={item.name}
                      entry={item}
                      selected={selected === item.name}
                      onSelect={select}
                    />
                  );
              }
            })}
          </div>
        ))}
      </aside>
      <section className="settings-form">
        <div className="tune-bar">
          {dialog && <h2>{dialog.title || dialog.name}</h2>}
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
          {notice && <span className="warn-note">⚡ {notice}</span>}
        </div>
        {dialog ? (
          <div className="dlg-body">
            <DialogItems items={dialog.items} onCommit={commit} />
            {dialog.help && (
              <p className="muted dlg-help">
                <a href={dialog.help} target="_blank" rel="noreferrer">
                  Speeduino wiki: {dialog.title || dialog.name}
                </a>
              </p>
            )}
          </div>
        ) : (
          <p className="muted center-note">
            Pick a settings page on the left — acceleration enrichment, idle
            control, fan control, and everything else the ECU definition
            offers.
          </p>
        )}
      </section>
    </div>
  );
}
