import { inject, Injectable, Injector, signal } from '@angular/core';
import type { DockviewApi } from 'dockview';
import { ViewRegistry } from '../extensions/view-registry';
import { AutoHideState, EMPTY_AUTO_HIDE } from './auto-hide';
import { Rect } from './dock-geometry';
import { DropGuides } from './drop-guides';
import { PANEL_WINDOWS, PanelWindows } from './panel-windows';
import { viewKind, ViewKind } from './view-kind';

/** Ids of the built-in panels the default layout is made of. */
export const PANEL = { timeline: 'timeline', events: 'events', inspector: 'inspector', metrics: 'metrics', profile: 'profile' } as const;

export function rectOf(el: Element | null | undefined): Rect | null {
  const r = el?.getBoundingClientRect?.();
  return r ? { x: r.left, y: r.top, width: r.width, height: r.height } : null;
}

/**
 * The state the dock's services share: the dockview instance and its container, which panels are docked, the
 * auto-hidden tool windows and the pinned tabs. Internal to the dock: components use `DockService`.
 */
@Injectable({ providedIn: 'root' })
export class DockContext {
  readonly views = inject(ViewRegistry);
  private readonly lazy = inject(Injector);

  api: DockviewApi | null = null;
  container: HTMLElement | null = null;
  guides: DropGuides | null = null;
  /** While a layout is being built or restored: changes are not persisted. */
  restoring = false;
  /** Panels by most recent use (Ctrl+Tab, the document well). */
  mru: string[] = [];

  readonly apiReady = signal(false);
  /** Ids of the panels in the layout (docked or floating). */
  readonly docked = signal<ReadonlySet<string>>(new Set());
  /** Ids of the panels showing now: each group's active tab (the others are tabs behind it). */
  readonly shown = signal<ReadonlySet<string>>(new Set());
  readonly autoHide = signal<AutoHideState>(EMPTY_AUTO_HIDE);
  /** Tabs pinned to the left of their strip (they survive "Close all"). */
  readonly pinned = signal<ReadonlySet<string>>(new Set());
  /** Ticks whenever dockview reports a layout change (a sash moved, a group floated, moved or was maximised). */
  readonly layoutVersion = signal(0);

  /** The pop-out windows (absent in a pop-out and in tests without them). */
  windows(): PanelWindows | null {
    return this.lazy.get(PANEL_WINDOWS, null);
  }

  kindOf(id: string): ViewKind {
    return viewKind(this.views.find(id));
  }

  isAutoHidden(id: string): boolean {
    return this.autoHide().items.some((i) => i.id === id);
  }

  refreshOpen(): void {
    this.docked.set(new Set((this.api?.panels ?? []).map((p) => p.id)));
    this.refreshShown();
  }

  /** Re-reads which tab is in front in each group. */
  refreshShown(): void {
    const ids = (this.api?.groups ?? []).flatMap((g) => (g.activePanel ? [g.activePanel.id] : []));
    const next = new Set(ids);
    const prev = this.shown();
    if (next.size !== prev.size || ids.some((id) => !prev.has(id))) this.shown.set(next);
  }

  /** Marks the tabs of pinned panels (dockview knows nothing about pinning). */
  markPinnedTabs(): void {
    const pinned = this.pinned();
    this.container?.querySelectorAll<HTMLElement>('.imt-tab[data-panel-id]').forEach((t) => {
      const on = pinned.has(t.dataset['panelId'] ?? '');
      t.classList.toggle('pinned', on);
      t.closest('.dv-tab')?.classList.toggle('imt-pinned-tab', on);
    });
  }

  /** The root rectangle of the dock, or null before attach. */
  rootRect(): Rect | null {
    return this.container ? rectOf(this.container) : null;
  }
}
