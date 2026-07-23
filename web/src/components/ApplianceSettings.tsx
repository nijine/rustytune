import { useEffect, useState } from "react";
import { api, type RuntimeConfig } from "../api";

export default function ApplianceSettings() {
  const [config, setConfig] = useState<RuntimeConfig | null>(null);
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    api
      .applianceConfig()
      .then((value) => {
        if (value.server.admin_socket !== null) setConfig(value);
      })
      .catch(() => {});
  }, []);

  if (!config) return null;
  const shutdown = config.engine_shutdown;
  const update = (patch: Partial<typeof shutdown>) =>
    setConfig({
      ...config,
      engine_shutdown: { ...shutdown, ...patch },
    });
  const valid =
    Number.isFinite(shutdown.stop_rpm) &&
    shutdown.stop_rpm >= 0 &&
    Number.isFinite(shutdown.arm_rpm) &&
    shutdown.arm_rpm > shutdown.stop_rpm &&
    Number.isInteger(shutdown.delay_seconds) &&
    shutdown.delay_seconds >= 1 &&
    shutdown.delay_seconds <= 600;

  const save = async () => {
    setBusy(true);
    setError(null);
    setMessage(null);
    try {
      const result = await api.applianceConfigPut(config);
      setMessage(
        result.restartRequired
          ? "Saved. Restart the RustyTune service or reboot to apply."
          : "Saved.",
      );
    } catch (e) {
      setError(e instanceof Error ? e.message : "Could not save settings");
    } finally {
      setBusy(false);
    }
  };

  return (
    <section className="appliance-settings">
      <div>
        <h2>Appliance shutdown</h2>
        <p className="muted">
          After the engine has run, close the active log and safely power off
          when RPM remains low.
        </p>
      </div>
      <label className="shutdown-enabled">
        <input
          type="checkbox"
          checked={shutdown.enabled}
          onChange={(e) => update({ enabled: e.target.checked })}
        />
        Enable automatic engine-off shutdown
      </label>
      <div className="appliance-fields">
        <label>
          Arm above
          <span>
            <input
              type="number"
              min={1}
              step={50}
              value={shutdown.arm_rpm}
              onChange={(e) => update({ arm_rpm: Number(e.target.value) })}
            />
            RPM
          </span>
        </label>
        <label>
          Stopped at or below
          <span>
            <input
              type="number"
              min={0}
              step={10}
              value={shutdown.stop_rpm}
              onChange={(e) => update({ stop_rpm: Number(e.target.value) })}
            />
            RPM
          </span>
        </label>
        <label>
          Shutdown delay
          <span>
            <input
              type="number"
              min={1}
              max={600}
              step={1}
              value={shutdown.delay_seconds}
              onChange={(e) =>
                update({ delay_seconds: Number(e.target.value) })
              }
            />
            seconds
          </span>
        </label>
      </div>
      {!valid && (
        <p className="error">
          Arm RPM must exceed stopped RPM; delay must be 1–600 seconds.
        </p>
      )}
      {error && <p className="error">{error}</p>}
      {message && <p className="success">{message}</p>}
      <button className="primary" disabled={busy || !valid} onClick={save}>
        {busy ? "Saving…" : "Save appliance settings"}
      </button>
    </section>
  );
}
