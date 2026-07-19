// Indicator lamp strip from the INI [FrontPage] indicator list. State is
// never color-alone: the label text itself switches with the state.

import { useEffect, useState } from "react";
import type { IndicatorUi } from "../api";
import type { TelemetryFeed } from "../feed";
import { C, iniColor } from "../tokens";

export default function Indicators({
  defs,
  feed,
  mode,
}: {
  defs: IndicatorUi[];
  feed: TelemetryFeed;
  /// "all": the full TunerStudio-style lamp strip. "active": a section
  /// that only shows lamps while they're on.
  mode: "all" | "active";
}) {
  const [states, setStates] = useState<boolean[]>([]);

  useEffect(
    () =>
      feed.onFrame((frame) => {
        // Only re-render when a lamp actually flips.
        setStates((prev) =>
          prev.length === frame.indicators.length &&
          prev.every((v, i) => v === frame.indicators[i])
            ? prev
            : frame.indicators,
        );
      }),
    [feed],
  );

  const lamp = (def: IndicatorUi, i: number) => {
    const on = states[i] ?? false;
    const bg = on ? iniColor(def.onBg, C.good) : "transparent";
    return (
      <span
        key={`${def.offLabel}-${i}`}
        className={`lamp ${on ? "on" : ""}`}
        style={
          on
            ? { background: bg, color: "#0d0d0d", borderColor: bg }
            : undefined
        }
      >
        {on ? def.onLabel : def.offLabel}
      </span>
    );
  };

  if (mode === "active") {
    const active = defs
      .map((def, i) => ({ def, i }))
      .filter(({ i }) => states[i]);
    return (
      <div className="indicators indicators-active">
        {active.length === 0 ? (
          <span className="no-active">no active statuses</span>
        ) : (
          active.map(({ def, i }) => lamp(def, i))
        )}
      </div>
    );
  }

  return <div className="indicators">{defs.map(lamp)}</div>;
}
