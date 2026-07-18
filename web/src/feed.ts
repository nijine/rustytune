// WebSocket telemetry feed. Frames arrive at the ECU poll rate (~20 Hz);
// gauges read `latest` from their own requestAnimationFrame loops instead
// of going through React state, so a frame never re-renders the tree.

import type { Definition, Frame, Status, TuneState } from "./api";

type FrameListener = (frame: Frame) => void;
type StatusListener = (status: Status) => void;
type TuneListener = (state: TuneState) => void;
type DefinitionListener = (definition: Definition) => void;

export class TelemetryFeed {
  latest: Frame | null = null;

  private ws: WebSocket | null = null;
  private closed = false;
  private retry: ReturnType<typeof setTimeout> | null = null;
  private frameListeners = new Set<FrameListener>();
  private statusListeners = new Set<StatusListener>();
  private tuneListeners = new Set<TuneListener>();
  private definitionListeners = new Set<DefinitionListener>();

  start() {
    this.closed = false;
    this.open();
  }

  stop() {
    this.closed = true;
    if (this.retry) clearTimeout(this.retry);
    this.ws?.close();
    this.ws = null;
  }

  onFrame(fn: FrameListener): () => void {
    this.frameListeners.add(fn);
    return () => this.frameListeners.delete(fn);
  }

  onStatus(fn: StatusListener): () => void {
    this.statusListeners.add(fn);
    return () => this.statusListeners.delete(fn);
  }

  onTune(fn: TuneListener): () => void {
    this.tuneListeners.add(fn);
    return () => this.tuneListeners.delete(fn);
  }

  /// Server re-resolved the definition (e.g. gauge limits edited).
  onDefinition(fn: DefinitionListener): () => void {
    this.definitionListeners.add(fn);
    return () => this.definitionListeners.delete(fn);
  }

  private open() {
    const proto = location.protocol === "https:" ? "wss" : "ws";
    const ws = new WebSocket(`${proto}://${location.host}/api/ws`);
    this.ws = ws;

    ws.onmessage = (event) => {
      const msg = JSON.parse(event.data as string);
      if (msg.type === "frame") {
        this.latest = msg as Frame;
        for (const fn of this.frameListeners) fn(this.latest);
      } else if (msg.type === "status") {
        for (const fn of this.statusListeners) fn(msg as Status);
      } else if (msg.type === "tune") {
        for (const fn of this.tuneListeners) fn(msg as TuneState);
      } else if (msg.type === "definition") {
        for (const fn of this.definitionListeners)
          fn(msg.definition as Definition);
      }
    };
    ws.onclose = () => {
      if (this.closed) return;
      this.latest = null;
      this.retry = setTimeout(() => this.open(), 1000);
    };
    ws.onerror = () => ws.close();
  }
}
