import { computed, inject, Injectable, Injector, signal } from '@angular/core';
import { Logger } from '../core/logger';
import { DialogLabels } from '../dialog-windows/dialog-labels';
import { DockSlot, Point, Rect } from '../dock/dock-geometry';
import { DockTarget, PopoutDrag, PopoutDragEvent, popoutDragReduce, screenToClient } from '../dock/dock-target';
import { DockService } from '../dock/dock.service';
import { PoppedOutPanel } from '../dock/layout-store';
import { DragOutPreview, PanelWindows } from '../dock/panel-windows';
import { CommandRegistry } from '../extensions/command-registry';
import { ViewRegistry } from '../extensions/view-registry';
import { ProjectService } from '../projects/project.service';
import { errorMessage } from '../tools/tool-errors';
import { clampToWorkAreas, dragGrab, placeAtPointer, placeNextToHost, sizeForPanel } from './bounds';
import { DragGhostBackend, GhostRequest } from './drag-ghost-backend';
import { GhostSession } from './drag-ghost-session';
import { snapshotPanel, SNAPSHOT_TIMEOUT_MS } from './panel-snapshot';
import { ICON_PATHS, isDebriefIcon } from '../shared/material/debrief-icons';
import { DebriefView } from '../extensions/tokens';
import { PopoutBackend, PopoutClosed, PopoutInfo } from './popout-backend';
import { POPOUT_ROUTE } from './popout-route';
import { PopoutState } from './popout-state';
import { sharedUiState } from './shared-state';
import { SyncedValue, UiSync } from './ui-sync';

/** More pop-outs per window are refused (each is a whole webview). Mirrors `MAX_POPOUTS` in Rust. */
export const MAX_POPOUTS = 12;
export const BOUNDS_KEY = 'debrief.popout.bounds.v1';
/**
 * How often a drag over this window is re-evaluated (the release poll, or the settle rule where the release cannot be
 * observed), and when an idle drag whose release cannot be observed is given up.
 */
export const DRAG_TICK_MS = 100;
export const DRAG_IDLE_MS = 2000;
/** Commands a pop-out may run in its host (global undo). */
export const HOST_COMMANDS: ReadonlySet<string> = new Set(['edit.undo', 'edit.redo']);

/**
 * Commands that act on the main window rather than a pop-out (history, the layout and its panels, windows): run in
 * the host when a pop-out gets them (its shortcuts, the shared macOS menu).
 */
export function runsInHost(commandId: string): boolean {
  return HOST_COMMANDS.has(commandId) || commandId.startsWith('window.') || commandId.startsWith('view.');
}

interface Popped {
  label: string;
  viewId: string;
  slot: DockSlot;
  bounds: Rect | null;
  monitor: string | null;
  /** Closed on purpose (Close, a layout without it): do not dock back. */
  discard?: boolean;
}

interface RememberedBounds {
  bounds: Rect;
  monitor: string | null;
}

export interface PopOutOptions {
  /** Open exactly here (screen, logical px): where the ghost of a tab dragged out of the window was. */
  rect?: Rect;
  /** Open at this screen point (a tab dragged out of the window, without a ghost). */
  at?: Point;
  /** Saved bounds and monitor (restoring a layout). */
  bounds?: Rect | null;
  monitor?: string | null;
  /** Where the panel docks back to (restoring a layout; else where it is now). */
  slot?: DockSlot;
}

/** A pop-out shell's drag handle position (`{"label","x","y"}`, screen px); null when malformed. */
export function parseDragOver(arg: string): { label: string; x: number; y: number } | null {
  try {
    const v = JSON.parse(arg) as { label?: unknown; x?: unknown; y?: unknown };
    return typeof v?.label === 'string' && Number.isFinite(v.x) && Number.isFinite(v.y) ? { label: v.label, x: v.x as number, y: v.y as number } : null;
  } catch {
    return null;
  }
}

/** Remembered window bounds per panel id (a panel pops out where it was last time). Never throws. */
export function loadBounds(): Record<string, RememberedBounds> {
  try {
    const raw = JSON.parse(localStorage.getItem(BOUNDS_KEY) ?? '{}') as unknown;
    return raw && typeof raw === 'object' && !Array.isArray(raw) ? (raw as Record<string, RememberedBounds>) : {};
  } catch {
    return {};
  }
}

function saveBounds(all: Record<string, RememberedBounds>): void {
  try {
    localStorage.setItem(BOUNDS_KEY, JSON.stringify(all));
  } catch {
    /* a convenience only */
  }
}

/**
 * The host side of pop-out windows: pops panels out (placing the window next to the host, under the pointer, or
 * where the panel was last time, always on a visible monitor), docks them back when their window closes, keeps
 * the window group's UI state in sync and runs the commands pop-outs forward (global undo).
 */
@Injectable({ providedIn: 'root' })
export class PopoutService implements PanelWindows {
  private readonly backend = inject(PopoutBackend);
  private readonly ghost = inject(DragGhostBackend);
  private readonly dock = inject(DockService);
  private readonly views = inject(ViewRegistry);
  private readonly sync = inject(UiSync);
  private readonly injector = inject(Injector);
  private readonly logger = inject(Logger);
  private readonly route = inject(POPOUT_ROUTE);
  private readonly dialogLabels = inject(DialogLabels);
  private readonly popped = signal<ReadonlyMap<string, Popped>>(new Map());
  private readonly opening = new Set<string>();
  private started: Promise<void> | null = null;
  private drag: PopoutDrag | null = null;
  private dragTimer: ReturnType<typeof setInterval> | null = null;
  private origin: Point | null = null;
  private lastMove = 0;
  /** The current drag is an OS window move whose release is observed by polling the mouse button. */
  private polling = false;
  private pollBusy = false;

  readonly poppedIds = computed<ReadonlySet<string>>(() => new Set(this.popped().keys()));

  /** Starts the host side once: listens to pop-out windows closing and starts the UI state sync. */
  startHost(): Promise<void> {
    if (this.route) return Promise.resolve();
    this.started ??= this.start();
    return this.started;
  }

  private async start(): Promise<void> {
    const label = await this.backend.label();
    await Promise.allSettled([
      this.backend.onClosed((e) => this.closed(e)),
      this.backend.onChanged(() => void this.refresh()),
      this.backend.onMoved((e) => void this.draggedOver(e.label, { x: e.x, y: e.y }, 'os')),
    ]);
    const port = this.backend.syncPort(label, true, () => [...[...this.popped().values()].map((p) => p.label), ...this.dialogLabels.labels()]);
    this.sync.start(port, true, label, [...sharedUiState(this.injector), hostTitle(this.injector)]);
    this.sync.onCommand((command, arg) => this.hostCommand(command, arg));
    if (typeof window !== 'undefined') window.addEventListener('pagehide', () => this.sync.goodbye());
    // After a reload of this window's UI the shell may still have its pop-outs: adopt them.
    const existing = await this.backend.list().catch(() => [] as PopoutInfo[]);
    for (const info of existing) this.adopt(info);
  }

  canPopOut(): boolean {
    return !this.route && this.popped().size + this.opening.size < MAX_POPOUTS;
  }

  async popOut(id: string, options: PopOutOptions = {}): Promise<boolean> {
    if (this.route) return false;
    await this.startHost();
    const view = this.views.find(id);
    if (!view) return false;
    if (this.popped().has(id)) {
      this.focus(id);
      return true;
    }
    if (this.opening.has(id)) return false;
    if (!this.canPopOut()) {
      this.dock.notice.set({ message: $localize`:@@popout.service.limit:At most ${MAX_POPOUTS}:max: panels can be popped out of one window; dock one back first.` });
      if (options.slot) this.dock.restoreSlot(id, options.slot);
      return false;
    }
    const slot = options.slot ?? this.dock.captureSlot(id) ?? { referencePanel: null, direction: 'within' };
    this.opening.add(id);
    try {
      const bounds = await this.placement(id, options);
      const info = await this.backend.open({ viewId: id, title: view.title, bounds });
      this.popped.update((m) => new Map(m).set(id, { label: info.label, viewId: id, slot, bounds: info.bounds ?? bounds, monitor: info.monitor }));
      this.dock.takeOut(id);
      this.dock.notice.set(null);
      this.logger.info('ui.popout', 'Panel popped out', { View: id });
      return true;
    } catch (e) {
      // The panel stays docked (or docks where it belongs when this was a restore).
      this.dock.restoreSlot(id, slot, { inactive: true });
      this.dock.notice.set({ message: $localize`:@@popout.service.failed:Could not pop out ${view.title}:title:: ${errorMessage(e)}:error:. It stays docked.`, retry: () => void this.popOut(id, options) });
      return false;
    } finally {
      this.opening.delete(id);
    }
  }

  /**
   * Where the window goes: where its drag ghost was (the shell already kept that on a monitor), saved bounds (clamped
   * to the visible monitors), under the pointer, or next to this window.
   */
  private async placement(id: string, options: PopOutOptions): Promise<Rect> {
    if (options.rect) return { ...options.rect };
    const areas = await this.backend.workAreas().catch(() => []);
    const remembered = options.bounds ? { bounds: options.bounds, monitor: options.monitor ?? null } : options.at ? null : (loadBounds()[id] ?? null);
    if (remembered?.bounds) return clampToWorkAreas(remembered.bounds, areas, remembered.monitor);
    const size = sizeForPanel(this.dock.panelRect(id));
    if (options.at) return placeAtPointer(options.at, size, areas);
    return placeNextToHost(await this.backend.windowBounds().catch(() => null), size, areas);
  }

  beginDragOut(id: string): DragOutPreview | null {
    const view = this.views.find(id);
    if (this.route || !view || !this.ghost.available || this.popped().has(id) || !this.canPopOut()) return null;
    return new GhostSession(this.ghost, this.ghostRequest(id, view));
  }

  /** The ghost of a dragged panel: the size its window will get, its icon and name, and a snapshot taken now. */
  private async ghostRequest(id: string, view: DebriefView): Promise<GhostRequest> {
    const size = sizeForPanel(this.dock.panelRect(id));
    const grab = dragGrab(size);
    const snapshot = await snapshotPanel(this.dock.panelContent(id), size, undefined, SNAPSHOT_TIMEOUT_MS);
    return {
      title: view.title,
      ...(isDebriefIcon(view.icon) && ICON_PATHS[view.icon] ? { icon: ICON_PATHS[view.icon] } : {}),
      ...(snapshot ? { snapshot } : {}),
      dark: typeof document !== 'undefined' && document.documentElement.getAttribute('data-scheme') === 'dark',
      width: size.width,
      height: size.height,
      grabX: grab.x,
      grabY: grab.y,
    };
  }

  focus(id: string): void {
    const p = this.popped().get(id);
    const view = this.views.find(id);
    if (p && view) void this.backend.open({ viewId: id, title: view.title }).catch(() => undefined);
  }

  async dockBack(id: string): Promise<void> {
    const p = this.popped().get(id);
    if (!p) return;
    try {
      await this.backend.dock(p.label);
    } catch {
      // The window is already gone: dock the panel back now.
      this.closed({ label: p.label, viewId: id });
    }
  }

  async discard(id: string): Promise<void> {
    const p = this.popped().get(id);
    if (!p) return;
    this.popped.update((m) => new Map(m).set(id, { ...p, discard: true }));
    await this.dockBack(id);
  }

  records(): PoppedOutPanel[] {
    return [...this.popped().values()].map((p) => ({ id: p.viewId, bounds: p.bounds, monitor: p.monitor, slot: p.slot }));
  }

  async restore(records: readonly PoppedOutPanel[]): Promise<void> {
    await this.startHost();
    for (const r of records) {
      if (this.popped().has(r.id) || !this.views.find(r.id)) continue;
      await this.popOut(r.id, { bounds: r.bounds, monitor: r.monitor, slot: r.slot });
    }
  }

  /** A pop-out window closed (OS close button, Dock back, crash): its panel docks back where it was. */
  private closed(e: PopoutClosed): void {
    const p = this.popped().get(e.viewId);
    if (!p || p.label !== e.label) return;
    this.popped.update((m) => {
      const next = new Map(m);
      next.delete(e.viewId);
      return next;
    });
    this.remember(p);
    if (!p.discard) {
      this.dock.restoreSlot(e.viewId, p.slot);
      this.dock.show(e.viewId);
    }
    this.dock.persistLater();
    this.logger.info('ui.popout', p.discard ? 'Pop-out closed' : 'Panel docked back', { View: e.viewId });
  }

  private adopt(info: PopoutInfo): void {
    if (this.popped().has(info.viewId) || !this.views.find(info.viewId)) return;
    const slot = this.dock.captureSlot(info.viewId) ?? { referencePanel: null, direction: 'within' as const };
    this.popped.update((m) => new Map(m).set(info.viewId, { label: info.label, viewId: info.viewId, slot, bounds: info.bounds, monitor: info.monitor }));
    this.dock.takeOut(info.viewId);
  }

  /** Pop-outs moved or resized: remember where (per panel and in the layout). */
  private async refresh(): Promise<void> {
    const list = await this.backend.list().catch(() => null);
    if (!list) return;
    this.popped.update((m) => {
      const next = new Map(m);
      for (const info of list) {
        const p = next.get(info.viewId);
        if (p && p.label === info.label) next.set(info.viewId, { ...p, bounds: info.bounds ?? p.bounds, monitor: info.monitor ?? p.monitor });
      }
      return next;
    });
    for (const p of this.popped().values()) this.remember(p);
    this.dock.persistLater();
  }

  private remember(p: Popped): void {
    if (!p.bounds) return;
    const all = loadBounds();
    all[p.viewId] = { bounds: p.bounds, monitor: p.monitor };
    saveBounds(all);
  }

  private hostCommand(command: string, arg: string | undefined): void {
    if (command === 'showView' && arg && this.views.find(arg)) this.dock.show(arg);
    else if (command === 'run' && arg && runsInHost(arg)) void this.injector.get(CommandRegistry).run(arg);
    // A dialog window's command (its own window has none of the app's commands): run for the user, here.
    else if (command === 'dialogRun' && arg) void this.injector.get(CommandRegistry).run(arg);
    else if (command === 'dragOver' && arg) {
      const over = parseDragOver(arg);
      if (over) void this.draggedOver(over.label, over, 'handle');
    } else if (command === 'dragEnd' && arg) this.dragStep({ type: 'end', label: arg, now: Date.now() });
  }

  /* ------------------------------------------------ a pop-out dragged back over this window */

  /**
   * A pop-out window (moved by the OS, or by its shell's drag handle) is at a screen point: show this window's dock
   * guides under it, and dock the panel when it is released on one. The handle reports its release (`dragEnd`); an
   * OS move does not, so the mouse button is polled, and only where it cannot be the panel docks when the pointer
   * rests on a guide.
   */
  private async draggedOver(label: string, screen: Point, source: 'os' | 'handle'): Promise<void> {
    const entry = [...this.popped().values()].find((p) => p.label === label);
    if (!entry) return;
    if (!this.drag) {
      if (source === 'os') {
        const down = await this.backend.primaryButtonDown().catch(() => null);
        // Moved without the button held (snapping, keyboard, restore): not a drag.
        if (down === false) return;
        this.polling = down === true;
      } else this.polling = false;
      this.origin = await this.backend.contentOrigin().catch(() => null);
    }
    const settle = source === 'os' && !this.polling;
    const origin = this.origin ?? { x: 0, y: 0 };
    const p = screenToClient(screen, origin);
    const inside = p.x >= 0 && p.y >= 0 && p.x <= window.innerWidth && p.y <= window.innerHeight;
    const point = inside ? p : null;
    this.dock.showDropGuides(point);
    this.lastMove = Date.now();
    this.dragStep({ type: 'move', label, viewId: entry.viewId, point, target: point ? this.dock.dropTargetAt(point) : null, now: Date.now(), settle });
    this.dragTimer ??= setInterval(() => this.tick(), DRAG_TICK_MS);
  }

  private tick(): void {
    this.dragStep({ type: 'tick', now: Date.now() });
    const drag = this.drag;
    if (!drag || !this.polling || this.pollBusy) return;
    this.pollBusy = true;
    void this.backend
      .primaryButtonDown()
      .catch(() => false)
      .then((down) => {
        if (down !== true && this.drag === drag) this.dragStep({ type: 'end', label: drag.label, now: Date.now() });
      })
      .finally(() => (this.pollBusy = false));
  }

  private dragStep(e: PopoutDragEvent): void {
    const { state, dock } = popoutDragReduce(this.drag, e);
    this.drag = state;
    // Gone (docked, ended, cancelled), or idle where the release cannot be observed: a drag that stopped somewhere
    // without a guide is given up after a while. A polled drag lasts as long as the button is held.
    if (!state || (e.type === 'tick' && state.settle && e.now - this.lastMove > DRAG_IDLE_MS)) {
      this.drag = null;
      this.dock.showDropGuides(null);
      if (this.dragTimer !== null) clearInterval(this.dragTimer);
      this.dragTimer = null;
    }
    if (dock) this.dockDropped(dock.label, dock.viewId, dock.target);
  }

  /** The pop-out was dropped on a guide: dock the panel there and close its window. */
  private dockDropped(label: string, viewId: string, target: DockTarget): void {
    const p = this.popped().get(viewId);
    if (!p || p.label !== label || !this.dock.dockAt(viewId, target)) return;
    this.logger.info('ui.popout', 'Pop-out dropped on the host', { View: viewId, Target: target.kind });
    void this.backend.dock(label).catch(() => this.closed({ label, viewId }));
  }
}

/** The host's project name, shown in its pop-outs' title bars. */
export function hostTitle(injector: Injector): SyncedValue {
  const projects = injector.get(ProjectService);
  const state = injector.get(PopoutState);
  return { key: 'hostTitle', direction: 'fromHost', read: () => projects.name(), apply: (v) => state.hostTitle.set(typeof v === 'string' ? v : '') };
}
