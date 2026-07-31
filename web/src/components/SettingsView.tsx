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
import CurveEditor from "./CurveEditor";

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

function DisplayOnlyRow({
  label,
  enabled,
  constant: c,
}: {
  label: string;
  enabled: boolean;
  constant: ConstantJson;
}) {
  const numeric = Number(c.value);
  const option = Number.isInteger(numeric) ? c.labels[numeric] : undefined;
  const value = option && option !== "INVALID"
    ? option
    : Number.isFinite(numeric)
      ? numeric.toFixed(c.digits)
      : String(c.value);

  return (
    <div className={`dlg-row${enabled ? "" : " disabled"}`}>
      <span className="dlg-label">{label}</span>
      <span className="dlg-value">{value}</span>
      <span className="units">{c.units ?? ""}</span>
    </div>
  );
}

function RequiredFuelRow({
  item,
  onCommit,
}: {
  item: Extract<DialogItemJson, { type: "requiredFuel" }>;
  onCommit: (c: ConstantJson, value: number) => void;
}) {
  const [open, setOpen] = useState(false);
  const [draft, setDraft] = useState<string | null>(null);
  const [displacement, setDisplacement] = useState(() =>
    localStorage.getItem("rustytune-engine-displacement") ?? "1200",
  );
  const [injectorFlow, setInjectorFlow] = useState(() =>
    localStorage.getItem("rustytune-injector-flow") ?? "255",
  );
  const [cylinders, setCylinders] = useState(String(Math.round(item.cylinders ?? 4)));
  const [afr, setAfr] = useState((item.afr ?? 14.7).toFixed(1));
  const [displacementUnit, setDisplacementUnit] = useState<"cc" | "cid">("cc");
  const [flowUnit, setFlowUnit] = useState<"cc/min" | "lb/hr">("cc/min");

  const displacementNumber = Number(displacement);
  const flowNumber = Number(injectorFlow);
  const cylinderNumber = Number(cylinders);
  const afrNumber = Number(afr);
  const displacementCc =
    displacementUnit === "cc" ? displacementNumber : displacementNumber * 16.387064;
  const flowCcMin = flowUnit === "cc/min" ? flowNumber : flowNumber * 10.5;
  // MegaSquirt/TunerStudio's standard-air-density equation simplifies to
  // this metric form at 100 kPa and 70°F.
  const calculated =
    (100 * displacementCc) / (cylinderNumber * afrNumber * flowCcMin);
  const factor = 10 ** item.constant.digits;
  const rounded = Math.round(calculated * factor) / factor;
  const valid =
    [displacementCc, flowCcMin, cylinderNumber, afrNumber, rounded].every(
      (value) => Number.isFinite(value) && value > 0,
    ) &&
    (item.constant.lo == null || rounded >= item.constant.lo) &&
    (item.constant.hi == null || rounded <= item.constant.hi);

  const changeDisplacementUnit = (unit: "cc" | "cid") => {
    if (unit === displacementUnit) return;
    const converted = unit === "cid" ? displacementNumber / 16.387064 : displacementNumber * 16.387064;
    setDisplacement(Number.isFinite(converted) ? converted.toFixed(1) : displacement);
    setDisplacementUnit(unit);
  };
  const changeFlowUnit = (unit: "cc/min" | "lb/hr") => {
    if (unit === flowUnit) return;
    const converted = unit === "lb/hr" ? flowNumber / 10.5 : flowNumber * 10.5;
    setInjectorFlow(Number.isFinite(converted) ? converted.toFixed(1) : injectorFlow);
    setFlowUnit(unit);
  };
  const apply = () => {
    if (!valid) return;
    localStorage.setItem("rustytune-engine-displacement", String(displacementCc));
    localStorage.setItem("rustytune-injector-flow", String(flowCcMin));
    onCommit(item.constant, rounded);
    setOpen(false);
  };
  const commit = (raw: string) => {
    setDraft(null);
    const value = Number(raw);
    if (!Number.isFinite(value) || value === item.constant.value) return;
    onCommit(item.constant, value);
  };

  return (
    <>
      <div className="dlg-row required-fuel-row">
        <span className="dlg-label">Required Fuel</span>
        <button type="button" onClick={() => setOpen(true)}>
          Calculate…
        </button>
        <input
          aria-label="Required Fuel"
          value={draft ?? Number(item.constant.value).toFixed(item.constant.digits)}
          onChange={(event) => setDraft(event.target.value)}
          onBlur={(event) => commit(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === "Enter") (event.target as HTMLInputElement).blur();
          }}
        />
        <span className="units">{item.constant.units ?? "ms"}</span>
      </div>
      {open && (
        <div className="modal-backdrop" role="presentation" onMouseDown={() => setOpen(false)}>
          <section
            className="required-fuel-modal"
            role="dialog"
            aria-modal="true"
            aria-labelledby="required-fuel-title"
            onMouseDown={(event) => event.stopPropagation()}
          >
            <h3 id="required-fuel-title">Required Fuel Calculator</h3>
            <div className="required-fuel-grid">
              <label>
                <span>Engine displacement</span>
                <input type="number" min="0" step="any" value={displacement} onChange={(e) => setDisplacement(e.target.value)} />
              </label>
              <div className="unit-choice">
                <label><input type="radio" checked={displacementUnit === "cid"} onChange={() => changeDisplacementUnit("cid")} /> CID</label>
                <label><input type="radio" checked={displacementUnit === "cc"} onChange={() => changeDisplacementUnit("cc")} /> CC</label>
              </div>
              <label>
                <span>Number of cylinders</span>
                <input type="number" min="1" step="1" value={cylinders} onChange={(e) => setCylinders(e.target.value)} />
              </label>
              <div />
              <label>
                <span>Injector flow</span>
                <input type="number" min="0" step="any" value={injectorFlow} onChange={(e) => setInjectorFlow(e.target.value)} />
              </label>
              <div className="unit-choice">
                <label><input type="radio" checked={flowUnit === "lb/hr"} onChange={() => changeFlowUnit("lb/hr")} /> lb/hr</label>
                <label><input type="radio" checked={flowUnit === "cc/min"} onChange={() => changeFlowUnit("cc/min")} /> cc/min</label>
              </div>
              <label>
                <span>Air-fuel ratio</span>
                <input type="number" min="0" step="any" value={afr} onChange={(e) => setAfr(e.target.value)} />
              </label>
              <div />
            </div>
            <p className="required-fuel-result">
              Calculated required fuel: <strong>{valid ? rounded.toFixed(item.constant.digits) : "—"} ms</strong>
            </p>
            {!valid && <p className="error">Enter positive values that produce a result within the ECU’s supported range.</p>}
            <div className="modal-actions">
              <button type="button" className="ghost" onClick={() => setOpen(false)}>Cancel</button>
              <button type="button" className="primary" disabled={!valid} onClick={apply}>Apply</button>
            </div>
          </section>
        </div>
      )}
    </>
  );
}

function DialogItems({
  items,
  onCommit,
  feed,
  onError,
}: {
  items: DialogItemJson[];
  onCommit: (c: ConstantJson, value: number) => void;
  feed: TelemetryFeed;
  onError: (msg: string) => void;
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
          case "displayOnly":
            return (
              <DisplayOnlyRow
                key={`display-${item.constant.name}-${i}`}
                label={item.label}
                enabled={item.enabled}
                constant={item.constant}
              />
            );
          case "requiredFuel":
            return <RequiredFuelRow key={`required-fuel-${i}`} item={item} onCommit={onCommit} />;
          case "panel":
            return (
              <fieldset
                key={`${item.name}-${i}`}
                className={`dlg-panel${item.enabled ? "" : " disabled"}`}
              >
                {item.title && <legend>{item.title}</legend>}
                <DialogItems
                  items={item.items}
                  onCommit={onCommit}
                  feed={feed}
                  onError={onError}
                />
              </fieldset>
            );
          case "curve":
            return (
              <CurveEditor
                key={item.name}
                id={item.name}
                feed={feed}
                onError={onError}
              />
            );
          case "table":
            return (
              <p key={`${item.name}-${i}`} className="muted dlg-ref">
                ⤷ {item.title || item.name} — edit in the Tuning tab
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
  onSelect: (entry: MenuEntryJson) => void;
}) {
  if (entry.type !== "dialog" && entry.type !== "curve") return null;
  return (
    <button
      className={`menu-entry${selected ? " selected" : ""}`}
      disabled={!entry.enabled}
      onClick={() => onSelect(entry)}
    >
      {entry.type === "curve" ? "∿ " : ""}
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
  const [selected, setSelected] = useState<MenuEntryJson | null>(null);
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
          if (selected?.type === "dialog") loadDialog(selected.name);
        }, 600);
      }),
    [feed, selected, loadMenus, loadDialog],
  );

  const select = (entry: MenuEntryJson) => {
    setSelected(entry);
    setNotice(null);
    if (entry.type === "dialog") {
      loadDialog(entry.name);
    } else {
      setDialog(null);
    }
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
        if (selected?.type === "dialog") loadDialog(selected.name);
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
                          selected={selected?.name === child.name}
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
                      selected={selected?.name === item.name}
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
        {selected?.type === "curve" ? (
          <div className="dlg-body">
            <CurveEditor
              key={selected.name}
              id={selected.name}
              feed={feed}
              onError={setError}
            />
          </div>
        ) : dialog ? (
          <div className="dlg-body">
            <DialogItems
              items={dialog.items}
              onCommit={commit}
              feed={feed}
              onError={setError}
            />
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
