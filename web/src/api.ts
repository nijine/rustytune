// Types mirroring the server's JSON (serde camelCase), plus REST helpers.

export interface GaugeUi {
  name: string;
  channel: string;
  title: string;
  units: string;
  lo: number;
  hi: number;
  loDanger: number;
  loWarn: number;
  hiWarn: number;
  hiDanger: number;
  valueDigits: number;
}

export interface IndicatorUi {
  offLabel: string;
  onLabel: string;
  offBg: string;
  offFg: string;
  onBg: string;
  onFg: string;
}

export interface Definition {
  signature: string;
  gauges: GaugeUi[];
  indicators: IndicatorUi[];
}

export interface LogStatus {
  path: string;
  rows: number;
}

export interface Status {
  connected: boolean;
  port: string | null;
  mode: string | null;
  baud: number | null;
  frames: number;
  crcErrors: number;
  timeouts: number;
  ecuSignature: string | null;
  lastError: string | null;
  log: LogStatus | null;
}

export interface Frame {
  t: number;
  channels: Record<string, number | string>;
  indicators: boolean[];
  logRows?: number;
}

export interface PortInfo {
  path: string;
  usb: boolean;
}

async function request<T>(path: string, init?: RequestInit): Promise<T> {
  const resp = await fetch(path, init);
  const body = await resp.json().catch(() => null);
  if (!resp.ok) {
    const msg =
      body && typeof body.error === "string"
        ? body.error
        : `${resp.status} ${resp.statusText}`;
    throw new Error(msg);
  }
  return body as T;
}

const post = (path: string, body?: unknown) =>
  request<unknown>(path, {
    method: "POST",
    headers: body ? { "Content-Type": "application/json" } : undefined,
    body: body ? JSON.stringify(body) : undefined,
  });

export const api = {
  definition: () => request<Definition>("/api/definition"),
  status: () => request<Status>("/api/status"),
  ports: () => request<PortInfo[]>("/api/ports"),
  connect: (port: string, mode: string, baud: number) =>
    post("/api/connect", { port, mode, baud }),
  disconnect: () => post("/api/disconnect"),
  logStart: () => post("/api/log/start"),
  logStop: () => post("/api/log/stop"),
};
