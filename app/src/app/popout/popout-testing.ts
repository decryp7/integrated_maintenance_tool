// Test doubles for the pop-out feature (specs only).
import { PopoutBackend, PopoutClosed, PopoutInfo, PopoutRequest } from './popout-backend';
import { SyncMessage } from './sync-protocol';
import { SyncPort } from './sync-port';

/** An in-memory group of windows: what one port posts, every other port receives (as a structured clone). */
export class Bus {
  readonly ports: BusPort[] = [];
  readonly log: SyncMessage[] = [];
  port(needsRelay = false): BusPort {
    const p = new BusPort(this, needsRelay);
    this.ports.push(p);
    return p;
  }
}

export class BusPort implements SyncPort {
  handlers: ((m: unknown) => void)[] = [];
  closed = false;
  constructor(
    private readonly bus: Bus,
    readonly needsRelay: boolean,
  ) {}
  post(message: SyncMessage): void {
    this.bus.log.push(message);
    for (const p of this.bus.ports) if (p !== this && !p.closed) p.handlers.forEach((h) => h(structuredClone(message)));
  }
  listen(handler: (m: unknown) => void): () => void {
    this.handlers.push(handler);
    return () => (this.handlers = this.handlers.filter((h) => h !== handler));
  }
  close(): void {
    this.closed = true;
  }
}

/** A backend double: windows open (or fail), list, and close through `dock`. */
export class FakeBackend extends PopoutBackend {
  opened: PopoutRequest[] = [];
  docked: string[] = [];
  fail: Error | null = null;
  existing: PopoutInfo[] = [];
  bounds: Record<string, PopoutInfo['bounds']> = {};
  closed: ((e: PopoutClosed) => void) | null = null;
  changed: (() => void) | null = null;
  onTop: boolean[] = [];
  gone = false;
  bus = new Bus();
  private n = 0;
  private infos = new Map<string, PopoutInfo>();

  async label() {
    return 'main';
  }
  async open(r: PopoutRequest): Promise<PopoutInfo> {
    this.opened.push(r);
    if (this.fail) throw this.fail;
    const existing = [...this.infos.values()].find((i) => i.viewId === r.viewId);
    if (existing) return existing;
    const info: PopoutInfo = { label: `p-main-${++this.n}`, host: 'main', viewId: r.viewId, title: r.title ?? r.viewId, bounds: r.bounds ?? null, monitor: null, alwaysOnTop: false };
    this.infos.set(info.label, info);
    return info;
  }
  async dock(label: string) {
    const info = this.infos.get(label);
    if (!info) throw new Error('gone');
    this.docked.push(label);
    this.infos.delete(label);
    this.closed?.({ label, viewId: info.viewId });
  }
  async list() {
    return [...this.existing, ...[...this.infos.values()].map((i) => ({ ...i, bounds: this.bounds[i.viewId] ?? i.bounds }))];
  }
  async workAreas() {
    return [{ name: 'Built-in', primary: true, x: 0, y: 0, width: 1920, height: 1080 }];
  }
  async windowBounds() {
    return { x: 0, y: 0, width: 1200, height: 800 };
  }
  async onClosed(h: (e: PopoutClosed) => void) {
    this.closed = h;
    return () => undefined;
  }
  async onChanged(h: () => void) {
    this.changed = h;
    return () => undefined;
  }
  moved: ((e: { label: string; viewId: string; x: number; y: number }) => void) | null = null;
  origin = { x: 0, y: 0 };
  windowDrag = false;
  /** What `primaryButtonDown` answers (null: the platform cannot tell). */
  buttonDown: boolean | null = null;
  supportsAlwaysOnTop = true;
  chrome: 'overlay' | 'frameless' | 'browser' = 'frameless';
  windowCalls: string[] = [];
  maximized = false;
  resized: (() => void) | null = null;
  async minimize() {
    this.windowCalls.push('minimize');
  }
  async toggleMaximize() {
    this.maximized = !this.maximized;
    this.windowCalls.push('toggleMaximize');
  }
  async closeWindow() {
    this.windowCalls.push('close');
  }
  async isMaximized() {
    return this.maximized;
  }
  async onResized(h: () => void) {
    this.resized = h;
    return () => undefined;
  }
  menu: ((id: string) => void) | null = null;
  async onMenu(h: (id: string) => void) {
    this.menu = h;
    return () => undefined;
  }
  async startResize(direction: string) {
    this.windowCalls.push(`resize:${direction}`);
  }
  async onMoved(h: (e: { label: string; viewId: string; x: number; y: number }) => void) {
    this.moved = h;
    return () => undefined;
  }
  async contentOrigin() {
    return this.origin;
  }
  async startWindowDrag() {
    return this.windowDrag;
  }
  async primaryButtonDown() {
    return this.buttonDown;
  }
  syncPort() {
    return this.bus.port();
  }
  async dockSelf() {
    this.docked.push('self');
  }
  async setAlwaysOnTop(on: boolean) {
    if (this.fail) throw this.fail;
    this.onTop.push(on);
  }
  hostGone() {
    return this.gone;
  }
}

