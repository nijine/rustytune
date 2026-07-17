// Color tokens for canvas drawing — a committed dark theme (automotive
// dashboard convention). Keep in sync with the CSS custom properties in
// index.css; canvas code can't read CSS vars cheaply per frame.
export const C = {
  page: "#0d0d0d",
  surface: "#1a1a19",
  ink: "#ffffff",
  secondary: "#c3c2b7",
  muted: "#898781",
  grid: "#2c2c2a",
  border: "rgba(255,255,255,0.10)",
  // Status palette (fixed, never used for series).
  good: "#0ca30c",
  warning: "#fab219",
  critical: "#d03b3b",
  accent: "#3987e5",
} as const;

/// INI indicator color names → theme colors.
export function iniColor(name: string, fallback: string): string {
  switch (name.trim().toLowerCase()) {
    case "green":
      return C.good;
    case "red":
      return C.critical;
    case "yellow":
      return C.warning;
    case "white":
      return C.surface;
    case "black":
      return C.ink;
    default:
      return fallback;
  }
}
