import { inject, Injectable } from '@angular/core';
import { invoke } from '../core/activity/ipc';
import { Logger, withFailureLogging } from '../core/logger';
import { isTauri } from '../core/platform';
import { Rect } from '../dock/dock-geometry';

/** Wire contract of the drag ghost commands (camelCase JSON), mirroring `app/src-tauri/src/drag_ghost/request.rs`. */
export interface GhostRequest {
  /** The panel's name, in the ghost's title bar. */
  title: string;
  /** The panel's icon: the `d` of its SVG path on Debrief's 24×24 icon grid. */
  icon?: string;
  /** A dimmed snapshot of the panel: `data:image/png;base64,…` (at most 2 MiB). */
  snapshot?: string;
  /** The window's colour scheme is dark. */
  dark?: boolean;
  /** The size the popped-out window will get, logical px. */
  width: number;
  height: number;
  /** Where the pointer holds the ghost, from its top-left corner (its title bar), logical px. */
  grabX: number;
  grabY: number;
}

/**
 * The shell's drag ghost: a native window that follows the pointer outside this window while a tab is dragged out,
 * showing where its pop-out will open (`drag_ghost.rs`). One session per drag: `start`, `show` as the pointer leaves
 * or re-enters the window, `end` (where the ghost was, or null when it was not showing).
 */
@Injectable({ providedIn: 'root', useFactory: createDragGhostBackend })
export abstract class DragGhostBackend {
  /** Is there a ghost here at all? (Not in a plain browser.) */
  abstract readonly available: boolean;
  /** Starts a session; false where the ghost cannot follow the pointer (Wayland, no monitors): keep the in-window preview. */
  abstract start(request: GhostRequest): Promise<boolean>;
  abstract show(visible: boolean): Promise<void>;
  /** Ends the session and hides the ghost: where it was (screen, logical px), or null when it was not showing. */
  abstract end(): Promise<Rect | null>;
}

/** `?mockGhost=1` in the browser build: the simulated ghost (end-to-end tests of the drag out of the window). */
export function mockGhostRequested(search: string): boolean {
  return new URLSearchParams(search).get('mockGhost') === '1';
}

/** Read when the app loads, before the router rewrites the URL. */
const MOCK_GHOST = typeof window !== 'undefined' && mockGhostRequested(window.location.search);

export function createDragGhostBackend(): DragGhostBackend {
  const real = isTauri()
    ? new TauriDragGhostBackend()
    : MOCK_GHOST
      ? new MockDragGhostBackend()
      : new NoDragGhost();
  return withFailureLogging<DragGhostBackend>(real, inject(Logger), { category: 'ui.popout' });
}

export class TauriDragGhostBackend extends DragGhostBackend {
  readonly available = true;
  constructor(private readonly call: typeof invoke = invoke) {
    super();
  }
  start(request: GhostRequest) {
    return this.call<boolean>('drag_ghost_start', { request });
  }
  show(visible: boolean) {
    return this.call<void>('drag_ghost_show', { visible });
  }
  end() {
    return this.call<Rect | null>('drag_ghost_end');
  }
}

/** No ghost (a browser): the dock draws its in-window preview. */
export class NoDragGhost extends DragGhostBackend {
  readonly available = false;
  async start(): Promise<boolean> {
    return false;
  }
  async show(): Promise<void> {}
  async end(): Promise<Rect | null> {
    return null;
  }
}

/** What the simulated ghost publishes as `window.__debriefMockGhost` (end-to-end tests read it). */
export interface MockGhostState {
  starts: number;
  ends: number;
  visible: boolean;
  /** Where the ghost is (screen px): the pointer at the request's grab point. */
  rect: Rect | null;
  request: (Omit<GhostRequest, 'snapshot'> & { hasSnapshot: boolean }) | null;
}

/**
 * The browser build's stand-in for the shell's ghost (`?mockGhost=1`): it follows the pointer's screen position as the
 * page sees it (the dragged tab captures the pointer, so moves outside the window are reported) and publishes its
 * state for end-to-end tests. Plausible, not a reference: the real placement is the shell's (`drag_ghost/geometry.rs`).
 */
export class MockDragGhostBackend extends DragGhostBackend {
  readonly available = true;
  readonly state: MockGhostState = { starts: 0, ends: 0, visible: false, rect: null, request: null };
  private pointer: { x: number; y: number } | null = null;
  private size: GhostRequest | null = null;

  constructor(private readonly win: Window = window) {
    super();
    // The window's position plus the client point: some engines report no screen position for synthetic moves.
    win.addEventListener('pointermove', (e) => this.moved(win.screenX + e.clientX, win.screenY + e.clientY), true);
    (win as unknown as { __debriefMockGhost?: MockGhostState }).__debriefMockGhost = this.state;
  }

  private moved(x: number, y: number): void {
    this.pointer = { x, y };
    if (this.state.visible) this.place();
  }

  private place(): void {
    const r = this.size;
    if (!r || !this.pointer) return;
    this.state.rect = { x: Math.round(this.pointer.x - r.grabX), y: Math.round(this.pointer.y - r.grabY), width: Math.round(r.width), height: Math.round(r.height) };
  }

  async start(request: GhostRequest): Promise<boolean> {
    const { snapshot, ...rest } = request;
    this.size = request;
    this.state.starts++;
    this.state.request = { ...rest, hasSnapshot: !!snapshot };
    this.state.visible = false;
    this.state.rect = null;
    return true;
  }

  async show(visible: boolean): Promise<void> {
    if (!this.size) return;
    this.state.visible = visible;
    if (visible) this.place();
  }

  async end(): Promise<Rect | null> {
    const rect = this.state.visible ? this.state.rect : null;
    this.state.ends++;
    this.state.visible = false;
    this.size = null;
    return rect ? { ...rect } : null;
  }
}
