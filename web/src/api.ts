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
  tuneLoaded: boolean;
  offline: boolean;
  lastError: string | null;
  log: LogStatus | null;
}

export interface RuntimeConfig {
  server: {
    bind: string;
    port: number;
    open_browser: boolean;
    admin_socket: string | null;
  };
  ecu: {
    device: string;
    mode: string;
    baud: number;
    poll_ms: number;
    auto_connect: boolean;
    ini: string | null;
  };
  logging: {
    directory: string;
    auto: boolean;
    retention_bytes: number;
  };
  engine_shutdown: {
    enabled: boolean;
    arm_rpm: number;
    stop_rpm: number;
    delay_seconds: number;
  };
  authentication: {
    required: boolean;
    state_directory: string;
  };
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

export interface TableInfo {
  id: string;
  title: string;
}

export interface TuneSummary {
  loaded: boolean;
  dirty: boolean;
  burnPending: boolean;
  writer: string | null;
  tables: TableInfo[];
}

export interface TableJson {
  id: string;
  title: string;
  x: number[];
  y: number[];
  z: number[][];
  zLo: number;
  zHi: number;
  zDigits: number;
  zScale: number;
  zTranslate: number;
  xLabel: string | null;
  yLabel: string | null;
  xChannel: string | null;
  yChannel: string | null;
  xMeta: TableAxisMeta | null;
  yMeta: TableAxisMeta | null;
}

export interface TableAxisMeta {
  lo: number | null;
  hi: number | null;
  digits: number;
  scale: number;
  translate: number;
}

export interface CurveJson {
  id: string;
  title: string;
  xLabel: string | null;
  yLabel: string | null;
  x: number[];
  y: number[];
  xMin: number;
  xMax: number;
  yMin: number;
  yMax: number;
  xLo: number | null;
  xHi: number | null;
  yLo: number | null;
  yHi: number | null;
  xDigits: number;
  yDigits: number;
  xUnits: string | null;
  yUnits: string | null;
  xChannel: string | null;
}

/// `{"type":"tune"}` WS message: dirty/burn state for all clients.
export interface TuneState {
  loaded: boolean;
  dirty: boolean;
  burnPending: boolean;
}

export interface ConstantJson {
  name: string;
  value: number | string;
  units: string | null;
  digits: number;
  lo: number | null;
  hi: number | null;
  labels: string[];
  requiresPowerCycle: boolean;
}

export interface LogFileJson {
  name: string;
  size: number;
  modified: string;
  active: boolean;
}

export interface LogListJson {
  dir: string;
  files: LogFileJson[];
}

export interface LogDataJson {
  name: string;
  title: string;
  labels: string[];
  units: string[];
  rows: number;
  /// Column-major; null = empty/non-numeric cell.
  columns: (number | null)[][];
}

export interface MenuEntryJson {
  type: "dialog" | "table" | "curve";
  name: string;
  label: string;
  enabled: boolean;
}

export type MenuItemJson =
  | MenuEntryJson
  | { type: "separator" }
  | { type: "group"; label: string; items: MenuEntryJson[] };

export interface MenuJson {
  title: string;
  items: MenuItemJson[];
}

export type DialogItemJson =
  | { type: "header"; label: string }
  | { type: "constant"; label: string; enabled: boolean; constant: ConstantJson }
  | { type: "displayOnly"; label: string; enabled: boolean; constant: ConstantJson }
  | {
      type: "requiredFuel";
      constant: ConstantJson;
      cylinders: number | null;
      afr: number | null;
    }
  | {
      type: "panel";
      name: string;
      title: string;
      enabled: boolean;
      items: DialogItemJson[];
    }
  | { type: "curve"; name: string; title: string }
  | { type: "table"; name: string; title: string }
  | { type: "unsupported"; label: string; name: string };

export interface DialogJson {
  name: string;
  title: string;
  help: string | null;
  items: DialogItemJson[];
}

export interface MsqMeta {
  filename: string;
  signature: string | null;
  signatureMatch: boolean;
  writeDate: string | null;
  author: string | null;
  settings: string[];
  constants: number;
}

export interface DiffCell {
  index: number;
  row?: number;
  col?: number;
  ecu: number | null;
  file: number | null;
}

export interface DiffEntryJson {
  name: string;
  page: number | null;
  where: string;
  kind: "scalar" | "bits" | "array";
  ecu?: number | string;
  file?: number | string;
  changedCount?: number;
  len?: number;
  cells?: DiffCell[];
}

export interface MsqDiffJson {
  meta: MsqMeta;
  entries: DiffEntryJson[];
  onlyInFile: string[];
  unresolved: [string, string][];
}

/// Stable per-browser id for the single-writer tuning lock.
export function clientId(): string {
  let id = localStorage.getItem("rustytune-client-id");
  if (!id) {
    // `crypto.randomUUID()` is restricted to secure contexts by some mobile
    // browsers. Appliance mode is intentionally served over plain HTTP, so use
    // getRandomValues when randomUUID is unavailable.
    if (typeof crypto.randomUUID === "function") {
      id = crypto.randomUUID();
    } else {
      const bytes = new Uint8Array(16);
      crypto.getRandomValues(bytes);
      bytes[6] = (bytes[6] & 0x0f) | 0x40;
      bytes[8] = (bytes[8] & 0x3f) | 0x80;
      const hex = Array.from(bytes, (byte) =>
        byte.toString(16).padStart(2, "0"),
      ).join("");
      id = `${hex.slice(0, 8)}-${hex.slice(8, 12)}-${hex.slice(12, 16)}-${hex.slice(16, 20)}-${hex.slice(20)}`;
    }
    localStorage.setItem("rustytune-client-id", id);
  }
  return id;
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

const post = <T = unknown,>(path: string, body?: unknown) =>
  request<T>(path, {
    method: "POST",
    headers: {
      ...(body ? { "Content-Type": "application/json" } : {}),
      "X-Client-Id": clientId(),
    },
    body: body ? JSON.stringify(body) : undefined,
  });

export const api = {
  pair: (code: string) => post<{ paired: boolean }>("/api/pair", { code }),
  definition: () => request<Definition>("/api/definition"),
  status: () => request<Status>("/api/status"),
  applianceConfig: () =>
    request<RuntimeConfig>("/api/appliance/config"),
  applianceConfigPut: (config: RuntimeConfig) =>
    request<{ saved: boolean; restartRequired: boolean }>(
      "/api/appliance/config",
      {
        method: "PUT",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(config),
      },
    ),
  ports: () => request<PortInfo[]>("/api/ports"),
  connect: (port: string, mode: string, baud: number) =>
    post("/api/connect", { port, mode, baud }),
  disconnect: () => post("/api/disconnect"),
  logStart: () => post("/api/log/start"),
  logStop: () => post("/api/log/stop"),
  logs: () => request<LogListJson>("/api/logs"),
  logData: (name: string) =>
    request<LogDataJson>(`/api/logs/${encodeURIComponent(name)}/data`),
  logImport: (name: string, content: string) =>
    request<{ name: string; rows: number }>(
      `/api/logs/${encodeURIComponent(name)}`,
      {
        method: "POST",
        headers: { "Content-Type": "text/plain" },
        body: content,
      },
    ),
  tune: () => request<TuneSummary>("/api/tune"),
  table: (id: string) => request<TableJson>(`/api/tune/table/${id}`),
  setCells: (id: string, cells: { row: number; col: number; value: number }[]) =>
    post(`/api/tune/table/${id}/cells`, { cells }),
  setTableAxis: (id: string, axis: "x" | "y", index: number, value: number) =>
    post(`/api/tune/table/${id}/axis`, { axis, index, value }),
  constants: (names: string[]) =>
    request<ConstantJson[]>(`/api/tune/constants?names=${names.join(",")}`),
  setConstant: (name: string, value: number) =>
    post<ConstantJson>(`/api/tune/constant/${name}`, { value }),
  curve: (id: string) => request<CurveJson>(`/api/tune/curve/${id}`),
  setCurvePoints: (
    id: string,
    points: { axis: "x" | "y"; index: number; value: number }[],
  ) => post(`/api/tune/curve/${id}/points`, { points }),
  menus: () => request<MenuJson[]>("/api/tune/menus"),
  dialog: (name: string) => request<DialogJson>(`/api/tune/dialog/${name}`),
  burn: () => post<{ burnedPages: number[] }>("/api/tune/burn"),
  msqUpload: (filename: string, content: string) =>
    post<MsqMeta>("/api/msq", { filename, content }),
  offlineOpen: () =>
    post<{ applied: number; skipped: [string, string][]; status: Status }>(
      "/api/offline",
    ),
  offlineClose: () => post<Status>("/api/offline/close"),
  msqDiff: () => request<MsqDiffJson>("/api/msq/diff"),
  msqApply: (names?: string[]) =>
    post<{ applied: number; skipped: [string, string][] }>("/api/msq/apply", {
      names: names ?? null,
    }),
};
