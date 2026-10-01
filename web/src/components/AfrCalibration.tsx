import { useCallback, useEffect, useState } from "react";
import { api, type AfrCalibrationJson, type AfrCurve } from "../api";
import type { TelemetryFeed } from "../feed";

const POINT_FIELDS = [
  ["voltsLow", "Low voltage", "V"],
  ["afrLow", "AFR at low voltage", "AFR"],
  ["voltsHigh", "High voltage", "V"],
  ["afrHigh", "AFR at high voltage", "AFR"],
] as const;

function points(curve: AfrCurve): AfrCurve {
  return {
    voltsLow: curve.voltsLow,
    afrLow: curve.afrLow,
    voltsHigh: curve.voltsHigh,
    afrHigh: curve.afrHigh,
  };
}

export default function AfrCalibration({
  offline,
  feed,
}: {
  offline: boolean;
  feed: TelemetryFeed;
}) {
  const [current, setCurrent] = useState<AfrCalibrationJson | null>(null);
  const [curve, setCurve] = useState<AfrCurve>({
    voltsLow: 0.5,
    afrLow: 8.5,
    voltsHigh: 4.5,
    afrHigh: 18,
  });
  const [preset, setPreset] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [saved, setSaved] = useState(false);
  const [remembered, setRemembered] = useState(false);
  const [rpm, setRpm] = useState<number | null>(null);

  useEffect(
    () => feed.onFrame((frame) => {
      const value = frame.channels.rpm;
      setRpm(typeof value === "number" ? value : null);
    }),
    [feed],
  );

  const read = useCallback(() => {
    setBusy(true);
    setError(null);
    setSaved(false);
    api.afrCalibration()
      .then((result) => {
        setCurrent(result);
        setRemembered(false);
        const match = result.presets.find((p) => result.matchingPresets.includes(p.name));
        if (match) {
          setPreset(match.name);
          setCurve(points(match));
        } else {
          setPreset("");
          try {
            const stored = JSON.parse(
              localStorage.getItem(`rustytune-afr-${result.crc}`) ?? "null",
            ) as AfrCurve | null;
            if (stored && POINT_FIELDS.every(([key]) => Number.isFinite(stored[key]))) {
              setCurve(points(stored));
              setRemembered(true);
            }
          } catch {
            // Remembered points are optional; the ECU checksum remains authoritative.
          }
        }
      })
      .catch((e: Error) => {
        setCurrent(null);
        setError(e.message);
      })
      .finally(() => setBusy(false));
  }, []);

  useEffect(() => {
    if (!offline) read();
  }, [offline, read]);

  const slope = (curve.afrHigh - curve.afrLow) / (curve.voltsHigh - curve.voltsLow);
  const intercept = curve.afrLow - curve.voltsLow * slope;
  const valid = POINT_FIELDS.every(([key]) => Number.isFinite(curve[key]))
    && curve.voltsLow >= 0 && curve.voltsHigh <= 5
    && curve.voltsLow < curve.voltsHigh
    && curve.afrLow > 0 && curve.afrHigh > curve.afrLow
    && intercept >= 0 && intercept + 5 * slope <= 25.5;

  const write = () => {
    setBusy(true);
    setError(null);
    setSaved(false);
    api.writeAfrCalibration({ ...points(curve), preset: preset || undefined })
      .then((result) => {
        setCurrent(result);
        setSaved(true);
        setRemembered(!preset);
        try {
          localStorage.setItem(`rustytune-afr-${result.crc}`, JSON.stringify(points(curve)));
        } catch {
          // Writing the ECU does not depend on browser storage.
        }
      })
      .catch((e: Error) => setError(e.message))
      .finally(() => setBusy(false));
  };

  const currentLabel = current
    ? current.matchingPresets.join(" / ") || (remembered
      ? "Custom curve saved by this browser (checksum matches)"
      : "Unknown or custom curve")
    : "Not read";

  return (
    <div className="dlg-body">
      <h2>AFR input calibration</h2>
      <p>
        This curve converts the wideband controller’s 0–5 V signal into the
        AFR used by Speeduino and the dashboard.
      </p>
      {offline ? (
        <p className="muted">
          Connect over primary USB serial to read or write calibration.
          AFR calibration is stored separately from the .msq tune.
        </p>
      ) : (
        <>
          <p><strong>Current ECU calibration: </strong>{currentLabel}</p>
          {current && (
            <p className="muted">
              Checksum: {current.crc}. The ECU supplies a checksum, not the
              full curve. An unmatched custom curve cannot be reconstructed.
            </p>
          )}
          <button disabled={busy} onClick={read}>Read ECU calibration</button>
          <fieldset disabled={busy || !current} className="dlg-panel">
            <legend>New calibration</legend>
            <label className="dlg-row">
              <span className="dlg-label">Controller</span>
              <select
                value={preset}
                onChange={(e) => {
                  setPreset(e.target.value);
                  setSaved(false);
                  const p = current?.presets.find((p) => p.name === e.target.value);
                  if (p) setCurve(points(p));
                }}
              >
                <option value="">Custom Linear WB</option>
                {current?.presets.map((p) => <option key={p.name}>{p.name}</option>)}
              </select>
            </label>
            {POINT_FIELDS.map(([key, label, units]) => (
              <label className="dlg-row" key={key}>
                <span className="dlg-label">{label}</span>
                <input
                  type="number"
                  step="any"
                  value={Number.isFinite(curve[key]) ? curve[key] : ""}
                  onChange={(e) => {
                    setPreset("");
                    setSaved(false);
                    setCurve({
                      ...curve,
                      [key]: e.target.value === "" ? NaN : Number(e.target.value),
                    });
                  }}
                />
                <span className="units">{units}</span>
              </label>
            ))}
            {valid ? (
              <p>Curve: AFR = {slope.toFixed(4)} × volts + {intercept.toFixed(4)}</p>
            ) : (
              <p className="error">
                Enter increasing voltage points within 0–5 V and positive,
                increasing AFR points. The full curve must remain within 0–25.5 AFR.
              </p>
            )}
            <p>
              Stop the engine before writing. Write to ECU saves the calibration
              directly to EEPROM; no separate Burn is needed.
            </p>
            <button disabled={!valid || rpm !== 0 || busy} onClick={write}>
              {busy ? "Working…" : "Write to ECU"}
            </button>
            {rpm !== 0 && <p className="muted">Writing requires a live reading of 0 RPM.</p>}
          </fieldset>
          {saved && <p role="status">Calibration saved to ECU and checksum verified.</p>}
        </>
      )}
      {error && <p role="alert" className="error">{error}</p>}
    </div>
  );
}
