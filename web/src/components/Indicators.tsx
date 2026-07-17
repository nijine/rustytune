// Indicator lamp strip from the INI [FrontPage] indicator list. State is
// never color-alone: the label text itself switches with the state.

import { useEffect, useState } from "react";
import type { IndicatorUi } from "../api";
import type { TelemetryFeed } from "../feed";
import { C, iniColor } from "../tokens";

export default function Indicators({
  defs,
  feed,
}: {
  defs: IndicatorUi[];
  feed: TelemetryFeed;
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

  return (
    <div className="indicators">
      {defs.map((def, i) => {
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
      })}
    </div>
  );
}
