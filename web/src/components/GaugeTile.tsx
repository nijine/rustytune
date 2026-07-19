// Simple numeric gauge tile (the pi-speeduino-dash look): big value,
// small label and unit, value colored by the INI's warn/danger zones.

import { useEffect, useState } from "react";
import type { GaugeUi } from "../api";
import type { TelemetryFeed } from "../feed";

export default function GaugeTile({
  def,
  feed,
}: {
  def: GaugeUi;
  feed: TelemetryFeed;
}) {
  const [value, setValue] = useState<number | null>(null);

  useEffect(
    () =>
      feed.onFrame((frame) => {
        const v = frame.channels[def.channel];
        setValue(typeof v === "number" ? v : null);
      }),
    [feed, def.channel],
  );

  let zone = "";
  if (value !== null) {
    if (value < def.loDanger || value > def.hiDanger) zone = " danger";
    else if (value < def.loWarn || value > def.hiWarn) zone = " warn";
  }

  return (
    <div className={`gauge-tile${zone}`}>
      <span className="tile-label">{def.title}</span>
      <span className="tile-reading">
        <span className="tile-value">
          {value === null ? "–" : value.toFixed(def.valueDigits)}
        </span>
        {def.units && <span className="tile-unit">{def.units}</span>}
      </span>
    </div>
  );
}
