import { invoke } from '../core/activity/ipc';
import { inject, Injectable } from '@angular/core';
import { Rect } from '../dock/dock-geometry';
import { Logger, withFailureLogging } from '../core/logger';
import { isTauri } from '../core/platform';
import { isMac } from '../extensions/keybinding';
import { currentWindowLabel, listenHere, Unlisten } from '../windowing/window-events';
import { WorkArea } from './bounds';
import { popoutHash, PopoutRoute } from './popout-route';
import { BroadcastChannelPort, SyncPort, TauriEventPort } from './sync-port';

/** Wire contract of the pop-out commands (camelCase JSON), mirroring `app/src-tauri/src/panel_windows.rs`. */
export interface PopoutRequest {
  viewId: string;
  title?: string;
  params?: unknown;
  bounds?: Rect;
  alwaysOnTop?: boolean;
}

export interface PopoutInfo {
  label: string;
  host: string;
  viewId: string;
  title: string;
  params?: unknown;
  bounds: Rect | null;
  monitor: string | null;
  alwaysOnTop: boolean;
}

export interface PopoutClosed {
  label: string;
  viewId: string;
}

export type WindowChrome = 'overlay' | 'frameless' | 'browser';
export type ResizeDirection = 'East' | 'North' | 'NorthEast' | 'NorthWest' | 'South' | 'SouthEast' | 'SouthWest' | 'West';

export interface PopoutMoved {
  label: string;
  viewId: string;
  /** The cursor, logical screen pixels. */
  x: number;
  y: number;
}

export const POPOUT_CLOSED_EVENT = 'imt://popout-closed';
export const POPOUT_MOVED_EVENT = 'imt://popout-moved';
/** A native menu item was clicked in this window (macOS shares one menu bar). */
export const MENU_EVENT = 'imt://menu';
export const POPOUTS_CHANGED_EVENT = 'imt://popouts-changed';

/**
 * Pop-out windows: real OS windows inside Tauri, `window.open` popups in the browser build (e2e, dev). The host
 * side opens, lists and docks pop-outs; the pop-out side docks itself and keeps itself on top.
 */
@Injectable({ providedIn: 'root', useFactory: createPopoutBackend })
export abstract class PopoutBackend {
  /** Label of this window (the host's label in a normal window). */
  abstract label(): Promise<string>;
  abstract open(request: PopoutRequest): Promise<PopoutInfo>;
  abstract dock(label: string): Promise<void>;
  /** The pop-outs of this window's group. */
  abstract list(): Promise<PopoutInfo[]>;
  /** Usable areas of the monitors, logical px (empty when unknown). */
  abstract workAreas(): Promise<WorkArea[]>;
  /** This window's bounds on screen, logical px. */
  abstract windowBounds(): Promise<Rect | null>;
  abstract onClosed(handler: (e: PopoutClosed) => void): Promise<Unlisten>;
  abstract onChanged(handler: () => void): Promise<Unlisten>;
  /** A pop-out window of this group is being moved (Tauri: from the shell, with the cursor position). */
  abstract onMoved(handler: (e: PopoutMoved) => void): Promise<Unlisten>;
  /** Where this window's content (client 0,0) is on screen, logical px; null when unknown. */
  abstract contentOrigin(): Promise<{ x: number; y: number } | null>;
  /** In a pop-out: lets the OS move the window with the pointer (true), or false when the platform cannot. */
  abstract startWindowDrag(): Promise<boolean>;
  /**
   * Is the primary mouse button held anywhere on screen? null when the platform cannot tell. The host polls it while
   * a pop-out is moved by the OS (which has no "dropped" event) to dock only when the window is released.
   */
  abstract primaryButtonDown(): Promise<boolean | null>;
  /** Can a window be kept above the others here? (not in a browser) */
  abstract readonly supportsAlwaysOnTop: boolean;
  /**
   * How a pop-out window is framed: `overlay` (macOS: native traffic lights over our title bar), `frameless`
   * (Windows/Linux: our own minimise / maximise / close), `browser` (a popup with the browser's frame).
   */
  abstract readonly chrome: WindowChrome;
  abstract minimize(): Promise<void>;
  abstract toggleMaximize(): Promise<void>;
  abstract closeWindow(): Promise<void>;
  abstract isMaximized(): Promise<boolean>;
  /** Called after every resize (maximised state may have changed). */
  abstract onResized(handler: () => void): Promise<Unlisten>;
  /** The shared native menu (macOS) sends its clicks to the focused window: `imt://menu {commandId}`. */
  abstract onMenu(handler: (commandId: string) => void): Promise<Unlisten>;
  /** Frameless windows: resize from an edge (`North`, `SouthEast`, ...). */
  abstract startResize(direction: ResizeDirection): Promise<void>;
  /** The sync transport of the window group; `targets` gives the labels to address (Tauri). */
  abstract syncPort(group: string, isHost: boolean, targets: () => readonly string[]): SyncPort;
  /** In a pop-out: closes this window, which docks its panel back into the host. */
  abstract dockSelf(route: PopoutRoute): Promise<void>;
  abstract setAlwaysOnTop(on: boolean): Promise<void>;
  /** In a pop-out: is the host window gone (browser: the opener closed)? */
  abstract hostGone(): boolean;
}

export function createPopoutBackend(): PopoutBackend {
  const real = isTauri() ? new TauriPopoutBackend() : new BrowserPopoutBackend();
  return withFailureLogging<PopoutBackend>(real, inject(Logger), { category: 'ui.popout' });
}

/** Talks to the Rust shell (`panel_windows.rs`). Dependencies are injectable for tests. */
export class TauriPopoutBackend extends PopoutBackend {
  constructor(
    private readonly call: typeof invoke = invoke,
    private readonly listen: (event: string, handler: (payload: unknown) => void) => Promise<Unlisten> = listenHere,
    private readonly labelOf: () => Promise<string> = currentWindowLabel,
  ) {
    super();
  }

  label() {
    return this.labelOf();
  }
  open(request: PopoutRequest) {
    return this.call<PopoutInfo>('popout_panel', { request });
  }
  dock(label: string) {
    return this.call<void>('dock_panel', { popoutLabel: label });
  }
  list() {
    return this.call<PopoutInfo[]>('list_popouts');
  }
  async workAreas(): Promise<WorkArea[]> {
    const { availableMonitors, primaryMonitor } = await import('@tauri-apps/api/window');
    const [monitors, primary] = await Promise.all([availableMonitors(), primaryMonitor()]);
    return monitors.map((m) => {
      const s = m.scaleFactor || 1;
      const wa = m.workArea ?? { position: m.position, size: m.size };
      return {
        name: m.name ?? null,
        primary: !!primary && primary.name === m.name,
        x: wa.position.x / s,
        y: wa.position.y / s,
        width: wa.size.width / s,
        height: wa.size.height / s,
      };
    });
  }
  async windowBounds(): Promise<Rect | null> {
    const { getCurrentWindow } = await import('@tauri-apps/api/window');
    const w = getCurrentWindow();
    const [pos, size, scale] = await Promise.all([w.outerPosition(), w.outerSize(), w.scaleFactor()]);
    return { x: pos.x / scale, y: pos.y / scale, width: size.width / scale, height: size.height / scale };
  }
  onClosed(handler: (e: PopoutClosed) => void) {
    return this.listen(POPOUT_CLOSED_EVENT, (p) => {
      const e = p as Partial<PopoutClosed> | null;
      if (e && typeof e.label === 'string' && typeof e.viewId === 'string') handler({ label: e.label, viewId: e.viewId });
    });
  }
  onChanged(handler: () => void) {
    return this.listen(POPOUTS_CHANGED_EVENT, () => handler());
  }
  onMoved(handler: (e: PopoutMoved) => void) {
    return this.listen(POPOUT_MOVED_EVENT, (p) => {
      const e = parseMoved(p);
      if (e) handler(e);
    });
  }
  async contentOrigin() {
    const { getCurrentWindow } = await import('@tauri-apps/api/window');
    const w = getCurrentWindow();
    const [outer, outerSize, scale] = await Promise.all([w.outerPosition(), w.outerSize(), w.scaleFactor()]);
    // The webview's own viewport is the content: Tauri's inner size includes the title bar on macOS.
    return contentOriginOf(outer, outerSize, { width: window.innerWidth * scale, height: window.innerHeight * scale }, scale);
  }
  async startWindowDrag() {
    const { getCurrentWindow } = await import('@tauri-apps/api/window');
    await getCurrentWindow().startDragging();
    return true;
  }
  primaryButtonDown() {
    return this.call<boolean | null>('pointer_button_down');
  }
  readonly supportsAlwaysOnTop = true;
  readonly chrome: WindowChrome = isMac() ? 'overlay' : 'frameless';
  private async win() {
    const { getCurrentWindow } = await import('@tauri-apps/api/window');
    return getCurrentWindow();
  }
  async minimize() {
    await (await this.win()).minimize();
  }
  async toggleMaximize() {
    await (await this.win()).toggleMaximize();
  }
  async closeWindow() {
    await (await this.win()).close();
  }
  async isMaximized() {
    return (await this.win()).isMaximized();
  }
  async onResized(handler: () => void) {
    return (await this.win()).onResized(() => handler());
  }
  onMenu(handler: (commandId: string) => void) {
    return this.listen(MENU_EVENT, (p) => {
      const id = (p as { commandId?: unknown } | null)?.commandId;
      if (typeof id === 'string' && id) handler(id);
    });
  }
  async startResize(direction: ResizeDirection) {
    await (await this.win()).startResizeDragging(direction as never);
  }
  syncPort(_group: string, isHost: boolean, targets: () => readonly string[]): SyncPort {
    const emitTo = async (label: string, event: string, payload: unknown) => {
      const { emitTo: emit } = await import('@tauri-apps/api/event');
      await emit(label, event, payload);
    };
    return new TauriEventPort(targets, emitTo, this.listen, isHost);
  }
  dockSelf(route: PopoutRoute) {
    return this.dock(route.label);
  }
  setAlwaysOnTop(on: boolean) {
    return this.call<void>('set_popout_on_top', { on });
  }
  hostGone(): boolean {
    // The shell closes a host's pop-outs with it.
    return false;
  }
}

interface BrowserPopout {
  win: Window;
  info: PopoutInfo;
}

/**
 * Where a window's content starts on screen (logical px), from its outer frame and the sizes: the side borders
 * are equal, and the bottom border is as wide as a side one, so the rest of the height is the title bar. (The
 * platforms disagree on what the window's inner position and size mean; the webview viewport is the content.)
 */
export function contentOriginOf(
  outer: { x: number; y: number },
  outerSize: { width: number; height: number },
  innerSize: { width: number; height: number },
  scale: number,
): { x: number; y: number } {
  const s = scale > 0 && Number.isFinite(scale) ? scale : 1;
  const side = Math.max(0, (outerSize.width - innerSize.width) / 2);
  const top = Math.max(0, outerSize.height - innerSize.height - side);
  return { x: (outer.x + side) / s, y: (outer.y + top) / s };
}

/** Parses a pop-out moved payload; null when it is malformed. */
export function parseMoved(p: unknown): PopoutMoved | null {
  const e = p as Partial<PopoutMoved> | null;
  return e && typeof e.label === 'string' && typeof e.viewId === 'string' && Number.isFinite(e.x) && Number.isFinite(e.y)
    ? { label: e.label, viewId: e.viewId, x: e.x as number, y: e.y as number }
    : null;
}

/** How often the browser adapter checks whether a popup was closed. */
export const CLOSE_POLL_MS = 300;

/**
 * Browser build: pop-outs are `window.open` popups of the same app (same origin, so the mock backend and the
 * BroadcastChannel sync work). A closed popup is noticed by polling, like the shell's Destroyed event.
 */
export class BrowserPopoutBackend extends PopoutBackend {
  private readonly windows = new Map<string, BrowserPopout>();
  private readonly closedHandlers = new Set<(e: PopoutClosed) => void>();
  private readonly changedHandlers = new Set<() => void>();
  private timer: ReturnType<typeof setInterval> | null = null;
  private next = 1;
  /** Browser windows have no label; a random one per page keeps two tabs' groups apart. */
  private readonly ownLabel: string;

  constructor(
    private readonly win: Window = window,
    random: () => number = Math.random,
  ) {
    super();
    this.ownLabel = `b${Math.floor(random() * 2 ** 32).toString(36)}`;
  }

  async label(): Promise<string> {
    return this.route()?.label ?? this.ownLabel;
  }

  private route(): PopoutRoute | null {
    const m = /[?&]label=([a-z0-9-]+)/i.exec(this.win.location.hash);
    return m ? ({ label: m[1] } as PopoutRoute) : null;
  }

  async open(request: PopoutRequest): Promise<PopoutInfo> {
    const host = await this.label();
    const existing = [...this.windows.values()].find((p) => p.info.viewId === request.viewId && !p.win.closed);
    if (existing) {
      existing.win.focus();
      return existing.info;
    }
    const label = `p-${host}-${this.next++}`;
    const route = { view: request.viewId, host, label };
    const url = `${this.win.location.pathname}${this.win.location.search}${popoutHash(route)}`;
    const b = request.bounds;
    const features = b ? `popup,left=${Math.round(b.x)},top=${Math.round(b.y)},width=${Math.round(b.width)},height=${Math.round(b.height)}` : 'popup,width=560,height=420';
    const win = this.win.open(url, label, features);
    if (!win) throw new Error($localize`:@@popout.backend.blocked:The browser blocked the pop-out window; allow pop-ups for IMT`);
    const info: PopoutInfo = { label, host, viewId: request.viewId, title: request.title ?? request.viewId, params: request.params, bounds: b ?? null, monitor: null, alwaysOnTop: false };
    this.windows.set(label, { win, info });
    this.watch();
    this.changed();
    return info;
  }

  async dock(label: string): Promise<void> {
    const p = this.windows.get(label);
    if (!p) throw new Error($localize`:@@popout.backend.alreadyClosed:the pop-out was already closed`);
    p.win.close();
    this.poll();
  }

  async list(): Promise<PopoutInfo[]> {
    return [...this.windows.values()].filter((p) => !p.win.closed).map((p) => ({ ...p.info, bounds: this.boundsOf(p.win) ?? p.info.bounds }));
  }

  private boundsOf(w: Window): Rect | null {
    try {
      return { x: w.screenX, y: w.screenY, width: w.outerWidth, height: w.outerHeight };
    } catch {
      return null;
    }
  }

  async workAreas(): Promise<WorkArea[]> {
    const s = this.win.screen as Screen & { availLeft?: number; availTop?: number };
    if (!s?.availWidth) return [];
    return [{ name: null, primary: true, x: s.availLeft ?? 0, y: s.availTop ?? 0, width: s.availWidth, height: s.availHeight }];
  }

  async windowBounds(): Promise<Rect | null> {
    return this.boundsOf(this.win);
  }

  async onClosed(handler: (e: PopoutClosed) => void): Promise<Unlisten> {
    this.closedHandlers.add(handler);
    return () => this.closedHandlers.delete(handler);
  }

  async onChanged(handler: () => void): Promise<Unlisten> {
    this.changedHandlers.add(handler);
    return () => this.changedHandlers.delete(handler);
  }

  async onMoved(): Promise<Unlisten> {
    // Browser windows cannot be moved by the page; the pop-out shell's drag handle reports its pointer instead.
    return () => undefined;
  }

  async contentOrigin() {
    const w = this.win;
    return { x: w.screenX + Math.max(0, (w.outerWidth - w.innerWidth) / 2), y: w.screenY + Math.max(0, w.outerHeight - w.innerHeight) };
  }

  async startWindowDrag(): Promise<boolean> {
    return false;
  }

  async primaryButtonDown(): Promise<boolean | null> {
    // Pop-outs here are moved with the shell's drag handle, which reports its own release.
    return null;
  }

  readonly supportsAlwaysOnTop = false;
  readonly chrome: WindowChrome = 'browser';

  // The browser frames the popup: these are never offered.
  async minimize(): Promise<void> {}
  async toggleMaximize(): Promise<void> {}
  async closeWindow(): Promise<void> {
    this.win.close();
  }
  async isMaximized(): Promise<boolean> {
    return false;
  }
  async onResized(handler: () => void): Promise<Unlisten> {
    this.win.addEventListener('resize', handler);
    return () => this.win.removeEventListener('resize', handler);
  }
  async startResize(): Promise<void> {}

  async onMenu(): Promise<Unlisten> {
    // A browser popup has no native menu.
    return () => undefined;
  }

  syncPort(group: string): SyncPort {
    return new BroadcastChannelPort(group);
  }

  async dockSelf(): Promise<void> {
    this.win.close();
  }

  async setAlwaysOnTop(): Promise<void> {
    /* not possible for a browser window; the toggle only remembers the choice */
  }

  hostGone(): boolean {
    const opener = this.win.opener as Window | null;
    return !opener || opener.closed;
  }

  private watch(): void {
    this.timer ??= setInterval(() => this.poll(), CLOSE_POLL_MS);
  }

  /** Reports popups that were closed (by the user, or by `dock`). */
  poll(): void {
    for (const [label, p] of this.windows) {
      if (!p.win.closed) continue;
      this.windows.delete(label);
      for (const h of this.closedHandlers) h({ label, viewId: p.info.viewId });
      this.changed();
    }
    if (!this.windows.size && this.timer !== null) {
      clearInterval(this.timer);
      this.timer = null;
    }
  }

  private changed(): void {
    for (const h of this.changedHandlers) h();
  }
}
