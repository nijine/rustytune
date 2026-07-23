// One canvas dial gauge: 270° sweep, warn/danger zone arcs from the INI's
// gauge configuration, needle eased toward the live value each frame.

import { useEffect, useRef, useState } from "react";
import type { GaugeUi } from "../api";
import type { TelemetryFeed } from "../feed";
import { C } from "../tokens";

const START = 0.75 * Math.PI; // 135°, lower-left
const SWEEP = 1.5 * Math.PI; // 270° clockwise
const SIZE = 210; // css px, square

function tickLabel(v: number): string {
  if (Math.abs(v) >= 1000) {
    const k = v / 1000;
    return `${Number.isInteger(k) ? k : k.toFixed(1)}k`;
  }
  return `${Math.round(v)}`;
}

export default function Gauge({
  def,
  feed,
}: {
  def: GaugeUi;
  feed: TelemetryFeed;
}) {
  const figureRef = useRef<HTMLElement>(null);
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const [active, setActive] = useState(false);

  // A full Speeduino definition can contain around 90 gauges. Allocating a
  // high-DPI canvas and animation loop for every off-screen gauge at once can
  // stall mobile browsers immediately after pairing. Keep a generous preload
  // margin so scrolling is seamless while only nearby gauges consume memory.
  useEffect(() => {
    const figure = figureRef.current;
    if (!figure) return;
    if (!("IntersectionObserver" in window)) {
      setActive(true);
      return;
    }
    const observer = new IntersectionObserver(
      ([entry]) => setActive(entry.isIntersecting),
      { rootMargin: "400px 0px" },
    );
    observer.observe(figure);
    return () => observer.disconnect();
  }, []);

  useEffect(() => {
    if (!active) return;
    const canvas = canvasRef.current;
    if (!canvas) return;
    const ctx = canvas.getContext("2d");
    if (!ctx) return;

    const dpr = window.devicePixelRatio || 1;
    canvas.width = SIZE * dpr;
    canvas.height = SIZE * dpr;
    ctx.scale(dpr, dpr);

    const span = def.hi - def.lo || 1;
    const frac = (v: number) =>
      Math.min(1, Math.max(0, (v - def.lo) / span));
    const angle = (v: number) => START + frac(v) * SWEEP;

    const cx = SIZE / 2;
    const cy = SIZE / 2;
    const r = SIZE / 2 - 16;

    let shown: number | null = null; // eased needle position
    let raf = 0;

    const zone = (from: number, to: number, color: string) => {
      if (to - from <= 0) return;
      ctx.beginPath();
      ctx.arc(cx, cy, r, angle(from), angle(to));
      ctx.strokeStyle = color;
      ctx.lineWidth = 6;
      ctx.lineCap = "butt";
      ctx.stroke();
    };

    const draw = () => {
      const value = feed.latest?.channels[def.channel];
      const target = typeof value === "number" ? value : null;
      if (target === null) {
        shown = null;
      } else if (shown === null) {
        shown = target;
      } else {
        shown += (target - shown) * 0.25; // low-pass, settles in ~5 frames
      }

      ctx.clearRect(0, 0, SIZE, SIZE);

      // Track, then status zones over it.
      ctx.beginPath();
      ctx.arc(cx, cy, r, START, START + SWEEP);
      ctx.strokeStyle = C.grid;
      ctx.lineWidth = 6;
      ctx.lineCap = "round";
      ctx.stroke();
      zone(def.lo, def.loDanger, C.critical);
      zone(def.loDanger, def.loWarn, C.warning);
      zone(def.hiWarn, def.hiDanger, C.warning);
      zone(def.hiDanger, def.hi, C.critical);

      // Ticks + labels, 5 majors.
      ctx.font = "10px system-ui, -apple-system, sans-serif";
      ctx.fillStyle = C.muted;
      ctx.textAlign = "center";
      ctx.textBaseline = "middle";
      for (let i = 0; i <= 4; i++) {
        const v = def.lo + (span * i) / 4;
        const a = angle(v);
        const cosA = Math.cos(a);
        const sinA = Math.sin(a);
        ctx.beginPath();
        ctx.moveTo(cx + cosA * (r - 8), cy + sinA * (r - 8));
        ctx.lineTo(cx + cosA * (r - 14), cy + sinA * (r - 14));
        ctx.strokeStyle = C.muted;
        ctx.lineWidth = 1;
        ctx.stroke();
        ctx.fillText(tickLabel(v), cx + cosA * (r - 26), cy + sinA * (r - 26));
      }

      // Needle + hub.
      if (shown !== null) {
        const a = angle(shown);
        ctx.beginPath();
        ctx.moveTo(cx, cy);
        ctx.lineTo(cx + Math.cos(a) * (r - 18), cy + Math.sin(a) * (r - 18));
        ctx.strokeStyle = C.ink;
        ctx.lineWidth = 2;
        ctx.lineCap = "round";
        ctx.stroke();
      }
      ctx.beginPath();
      ctx.arc(cx, cy, 4, 0, 2 * Math.PI);
      ctx.fillStyle = C.secondary;
      ctx.fill();

      // Value readout under the hub (the dial's open quadrant).
      ctx.textAlign = "center";
      ctx.fillStyle = shown === null ? C.muted : C.ink;
      ctx.font =
        "600 24px system-ui, -apple-system, sans-serif";
      const text =
        shown === null ? "–" : shown.toFixed(def.valueDigits);
      ctx.fillText(text, cx, cy + r - 24);
      if (def.units) {
        ctx.fillStyle = C.muted;
        ctx.font = "11px system-ui, -apple-system, sans-serif";
        ctx.fillText(def.units, cx, cy + r - 6);
      }

      raf = requestAnimationFrame(draw);
    };
    raf = requestAnimationFrame(draw);
    return () => cancelAnimationFrame(raf);
  }, [active, def, feed]);

  return (
    <figure className="gauge" ref={figureRef}>
      {active ? (
        <canvas
          ref={canvasRef}
          style={{ width: SIZE, height: SIZE }}
          role="img"
          aria-label={`${def.title} gauge`}
        />
      ) : (
        <div
          className="gauge-placeholder"
          style={{ width: SIZE, height: SIZE }}
          aria-label={`${def.title} gauge loading`}
        />
      )}
      <figcaption>{def.title}</figcaption>
    </figure>
  );
}
