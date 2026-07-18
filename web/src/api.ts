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
  xLabel: string | null;
  yLabel: string | null;
  xChannel: string | null;
  yChannel: string | null;
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
    id = crypto.randomUUID();
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
  definition: () => request<Definition>("/api/definition"),
  status: () => request<Status>("/api/status"),
  ports: () => request<PortInfo[]>("/api/ports"),
  connect: (port: string, mode: string, baud: number) =>
    post("/api/connect", { port, mode, baud }),
  disconnect: () => post("/api/disconnect"),
  logStart: () => post("/api/log/start"),
  logStop: () => post("/api/log/stop"),
  logs: () => request<LogListJson>("/api/logs"),
  tune: () => request<TuneSummary>("/api/tune"),
  table: (id: string) => request<TableJson>(`/api/tune/table/${id}`),
  setCells: (id: string, cells: { row: number; col: number; value: number }[]) =>
    post(`/api/tune/table/${id}/cells`, { cells }),
  constants: (names: string[]) =>
    request<ConstantJson[]>(`/api/tune/constants?names=${names.join(",")}`),
  setConstant: (name: string, value: number) =>
    post<ConstantJson>(`/api/tune/constant/${name}`, { value }),
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
