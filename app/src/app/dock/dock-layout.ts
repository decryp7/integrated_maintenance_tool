import { inject, Injectable, Injector } from '@angular/core';
import type { DockviewApi, DockviewGroupPanel } from 'dockview';
import { LayoutPlan, OpenView, ViewLayout, planLayout, restorable, SavedHints } from '../layout-plan/layout-plan';
import { NotificationsStore } from '../notifications/notifications.store';
import { autoHideReduce } from './auto-hide';
import { DockContext } from './dock-context';
import type { DockSlot } from './dock-geometry';
import type { DisplacedView } from './layout-store';
import { viewLayout, viewRow } from './view-layout';

/** What `fit` is asked to do. */
export interface FitOptions {
  /** Set the planned sizes of the non-anchor columns and rows (on open and on a load); else only tab/hide and constrain. */
  sizes?: boolean;
  /** Panels just opened: their current size is not a wish, and a tool window that has no room slides out for the user to see. */
  fresh?: readonly string[];
  /** The panels were opened in the background (auto-open): they never take the front from the view they join. */
  background?: boolean;
  /** Where a view was docked, captured before the plan moves it away (so it can return); `DockPlacement.captureSlot`. */
  capture?: (id: string) => DockSlot | null;
}

/** Grow-back hysteresis: a view the plan moved away returns only once the window is this much larger than where it first fits (px). */
export const GROW_BACK_MARGIN = { width: 64, height: 48 } as const;

/**
 * The thin edge between `planLayout` (pure, `layout-plan/`) and dockview (ADR 0056): reads the open panels as
 * `OpenView`s, and translates a `LayoutPlan` into `moveTo` (tabbed), auto-hide (hidden), `setConstraints` (every
 * group, from the views it holds) and `setSize`. Decides nothing itself.
 */
/** The Explorer side bar is a view of the plan that lives outside the dock (ADR 0056). */
export const EXPLORER_ID = 'explorer';
/** The sidebar has a fixed width, so its minimum is its preference: the plan opens or closes it, it never resizes it. */
export const EXPLORER_LAYOUT: ViewLayout = { area: 'side', edge: 'left', min: { width: 300, height: 240 }, pref: { width: 300, height: 400 }, priority: 80 };

const NO_SLOT: DockSlot = { referencePanel: null, direction: 'within' };

@Injectable({ providedIn: 'root' })
export class DockLayout {
  private readonly ctx = inject(DockContext);
  private readonly lazy = inject(Injector);
  /** Panels in the order they were last opened (newest first): a view opened in the background was still used just now. */
  private opened: string[] = [];

  /**
   * The views as the planner sees them: the docked (not floating) panels, plus the views the plan auto-hid earlier
   * (they still count as open: they come back when there is room), plus `extra` views that live outside the dock.
   */
  openViews(api: DockviewApi, extra: readonly OpenView[] = [], fresh: readonly string[] = []): OpenView[] {
    const out: OpenView[] = [];
    // A group that holds a full-height document of its own is a full-height column, whatever else was opened into it. A
    // view the plan itself tabbed in (displaced) is a guest: it does not make its host a full-height column.
    const columns = new Set<string>();
    for (const p of api.panels ?? []) if (this.ctx.views.find(p.id)?.fullHeight && !this.ctx.displaced.has(p.id)) columns.add(p.group.id);
    for (const p of api.panels ?? []) {
      if (p.group.api.location.type !== 'grid') continue;
      const view = this.ctx.views.find(p.id);
      if (!view) continue;
      const layout = viewLayout(view);
      const full = view.fullHeight || (layout.area === 'document' && columns.has(p.group.id) && !this.ctx.displaced.has(p.id));
      out.push({ id: p.id, layout, row: this.rowOf(p.id, view), group: this.ctx.displaced.get(p.id)?.into ? p.id : p.group.id, ...this.recencyOf(p.id, fresh), ...(full ? { fullHeight: true } : {}) });
    }
    for (const d of this.ctx.displaced.values()) {
      const view = this.ctx.views.find(d.id);
      if (d.into === null && view && !out.some((o) => o.id === d.id) && this.ctx.isAutoHidden(d.id)) out.push({ id: d.id, layout: viewLayout(view), row: viewRow(view), group: d.id, ...(view.fullHeight ? { fullHeight: true } : {}) });
    }
    return [...out, ...extra];
  }

  /** The row of the main column a panel sits in: an extra instance of a view (a second timeline) is stacked below the first, in a row of its own. */
  private rowOf(id: string, view: Parameters<typeof viewRow>[0]): string {
    return id === view.id ? viewRow(view) : id;
  }

  /** How recently the user worked with a panel: just opened is the latest, else its place in the most-recently-used list. */
  private recencyOf(id: string, fresh: readonly string[]): { recency?: number } {
    if (fresh.includes(id)) return { recency: 0 };
    // Opened in the background (never activated) still counts as used just now: the later of "opened" and "activated".
    const at = [this.opened.indexOf(id), this.ctx.mru.indexOf(id)].filter((i) => i >= 0);
    return at.length ? { recency: Math.min(...at) } : {};
  }

  /** What the user asked for: the sizes they dragged views to (never the sizes a fit gave them). */
  hints(): SavedHints {
    return { sizes: this.ctx.userSizes };
  }

  /** Remembers the sizes the groups have now as the user's wish (called after a splitter drag, never after a fit). */
  rememberSizes(api: DockviewApi): void {
    const sizes: Record<string, { width?: number; height?: number }> = { ...this.ctx.userSizes };
    for (const p of api.panels ?? []) {
      if (p.group.api.location.type !== 'grid' || this.ctx.displaced.has(p.id)) continue;
      const { width, height } = p.group;
      if (width > 0 || height > 0) sizes[p.id] = { ...(width > 0 ? { width: Math.round(width) } : {}), ...(height > 0 ? { height: Math.round(height) } : {}) };
    }
    this.ctx.userSizes = sizes;
  }

  /** The plan for the panels docked now (null while the dock has no measured size). */
  plan(api: DockviewApi, fresh: readonly string[] = []): LayoutPlan | null {
    if (!(api.width > 0 && api.height > 0)) return null;
    // A container without a layout box (a detached or not yet laid out dock; dockview then reports its 100x100 default) is not a window.
    const box = this.ctx.container?.getBoundingClientRect();
    if (box && (box.width <= 0 || box.height <= 0)) return null;
    return planLayout({ width: api.width, height: api.height }, this.openViews(api, [], fresh), this.hints());
  }

  /**
   * Re-plans and applies: tabs and hides what the plan says yields, constrains every group and (with `sizes`) sets the
   * planned sizes. Returns the plan, or null when the dock is not measured yet (groups are still constrained).
   */
  fit(api: DockviewApi, options: FitOptions = {}): LayoutPlan | null {
    if (options.fresh?.length) this.opened = [...options.fresh, ...this.opened.filter((id) => !options.fresh!.includes(id))].slice(0, 16);
    const plan = this.plan(api, options.fresh);
    if (plan) this.displace(api, plan, options);
    this.constrain(api);
    if (plan && options.sizes) this.resize(api, plan);
    return plan;
  }

  /** Does the plan change something the layout has now: a view that yields, or a group under its minimum? */
  needsFit(api: DockviewApi, plan: LayoutPlan): boolean {
    if (plan.tabbed.some((t) => api.getPanel(t.id)?.group !== api.getPanel(t.into)?.group)) return true;
    if (plan.hidden.some((h) => h.edge === 'right' && api.getPanel(h.id))) return true;
    return plan.columns.some((c) => c.views.some((id) => (api.getPanel(id)?.group.width ?? Infinity) < c.minWidth - 1));
  }

  /** Every grid group may not be made smaller than the largest minimum of the views it holds, so a splitter cannot cross it. */
  constrain(api: DockviewApi): void {
    for (const g of api.groups ?? []) {
      if (g.api.location.type !== 'grid') continue;
      const mins = g.panels.map((p) => {
        const view = this.ctx.views.find(p.id);
        const l = viewLayout(view);
        // Side and full-height columns span the whole height; only rows of the main column have a height minimum.
        const rowed = (l.area === 'document' && !view?.fullHeight) || l.area === 'analysis';
        return { width: l.min.width, height: rowed ? l.min.height : 0 };
      });
      if (!mins.length) continue;
      const minimumWidth = Math.max(...mins.map((m) => m.width));
      const minimumHeight = Math.max(...mins.map((m) => m.height));
      g.api.setConstraints({ minimumWidth, ...(minimumHeight ? { minimumHeight } : {}) });
      // The declared minimum, readable from outside (the scenario tests assert every group against it).
      g.element.dataset['minWidth'] = String(minimumWidth);
      g.element.dataset['minHeight'] = String(minimumHeight);
    }
  }

  private displace(api: DockviewApi, plan: LayoutPlan, options: FitOptions): void {
    const fresh = options.fresh ?? [];
    for (const t of plan.tabbed) {
      const panel = api.getPanel(t.id);
      const target = api.getPanel(t.into);
      if (!panel || !target || panel.group === target.group) continue;
      // A view opened just now has no earlier place to capture, but it is still recorded as a guest of its host (so the host is not
      // mistaken for a full-height column on the next plan) and can return to its default place when there is room.
      if (!this.ctx.displaced.has(t.id)) this.record({ id: t.id, slot: fresh.includes(t.id) ? NO_SLOT : (options.capture?.(t.id) ?? NO_SLOT), into: t.into });
      // A view that yields joins the group as a background tab; the tab in front stays, unless the user just opened it.
      const front = target.group.activePanel;
      panel.api.moveTo({ group: target.group, position: 'center' });
      if (fresh.includes(t.id) && !options.background) panel.api.setActive();
      else front?.api.setActive();
    }
    for (const h of plan.hidden) {
      const panel = api.getPanel(h.id);
      if (!panel || h.edge !== 'right') continue;
      const view = this.ctx.views.find(h.id);
      if (!view) continue;
      if (!fresh.includes(h.id)) this.record({ id: h.id, slot: options.capture?.(h.id) ?? NO_SLOT, into: null });
      api.removePanel(panel);
      const item = { id: h.id, title: view.title, side: 'right' as const, size: viewLayout(view).pref.width, slot: { referencePanel: null, direction: 'within' as const } };
      this.ctx.autoHide.update((s) => {
        const hidden = autoHideReduce(s, { type: 'unpin', item });
        return fresh.includes(h.id) && !options.background ? autoHideReduce(hidden, { type: 'activate', id: h.id }) : hidden;
      });
      this.ctx.refreshOpen();
    }
  }

  private groupOf(api: DockviewApi, ids: readonly string[]): DockviewGroupPanel | undefined {
    for (const id of ids) {
      const g = api.getPanel(id)?.group;
      if (g && g.api.location.type === 'grid') return g;
    }
    return undefined;
  }

  private resize(api: DockviewApi, plan: LayoutPlan): void {
    for (const c of plan.columns) {
      if (!c.anchor) {
        const g = this.groupOf(api, c.views);
        if (g && Math.abs(g.width - c.width) > 1) g.api.setSize({ width: c.width });
      }
      for (const r of c.rows) {
        for (const cell of r.cells) {
          if (cell.anchor) continue;
          const g = this.groupOf(api, cell.views);
          if (g && Math.abs(g.width - cell.width) > 1) g.api.setSize({ width: cell.width });
        }
        if (r.anchor) continue;
        const g = this.groupOf(api, r.views);
        if (g && Math.abs(g.height - r.height) > 1) g.api.setSize({ height: r.height });
      }
    }
  }

  private record(d: DisplacedView): void {
    this.ctx.displaced.set(d.id, d);
  }

  /** Ids of displaced views that may return now: they are placed even with `GROW_BACK_MARGIN` less room. */
  growBackIds(api: DockviewApi): string[] {
    if (!this.ctx.displaced.size || !(api.width > 0 && api.height > 0)) return [];
    const box = this.ctx.container?.getBoundingClientRect();
    if (box && (box.width <= 0 || box.height <= 0)) return [];
    return restorable({ width: api.width, height: api.height }, this.openViews(api), [...this.ctx.displaced.keys()], GROW_BACK_MARGIN, this.hints());
  }

  /** The order displaced views return in: one whose slot is relative to another returning view goes after it (Home beside Registry beside the assistant). */
  restoreOrder(ids: readonly string[]): string[] {
    const ordered: string[] = [];
    const waiting = new Set(ids);
    while (waiting.size) {
      const next = [...waiting].filter((id) => !waiting.has(this.ctx.displaced.get(id)?.slot.referencePanel ?? '')).sort();
      // A cycle cannot be ordered: take what is left, in a stable order.
      for (const id of next.length ? next : [...waiting].sort()) {
        waiting.delete(id);
        ordered.push(id);
      }
    }
    return ordered;
  }

  /**
   * Puts a view that was tabbed into another group back where it was docked. `moved`: done (next to its old neighbour, or
   * tabbed with it); `readd`: it was a column of its own at a window edge, so the caller docks it there again (dockview
   * cannot move a panel to the root by itself); `stale`: the record no longer applies (the view or its host is gone, or
   * the user moved it).
   */
  moveBack(api: DockviewApi, d: DisplacedView): 'moved' | 'readd' | 'stale' {
    const panel = api.getPanel(d.id);
    const host = d.into ? api.getPanel(d.into) : undefined;
    if (!panel || !host || panel.group !== host.group || panel.group.panels.length < 2) return 'stale';
    if (d.slot.edge) return 'readd';
    const ref = d.slot.referencePanel ? api.getPanel(d.slot.referencePanel) : undefined;
    const position = (dir: DockSlot['direction']) => ({ left: 'left', right: 'right', above: 'top', below: 'bottom', within: 'center' })[dir] as 'left' | 'right' | 'top' | 'bottom' | 'center';
    if (ref && ref.group !== panel.group) {
      panel.api.moveTo({ group: ref.group, position: position(d.slot.direction) });
      return 'moved';
    }
    return 'readd';
  }



  /**
   * Runs `apply` once the dock has a measured size. A size set before the container is measured is rescaled
   * proportionally when dockview first lays out, so the split landed on a different height from run to run; applied
   * after that layout it is exactly the requested one.
   */
  whenSized(api: DockviewApi, apply: () => void): void {
    const container = this.ctx.container;
    const sized = () => api.width > 0 && api.height > 0;
    if (sized() || !container || typeof ResizeObserver === 'undefined') {
      apply();
      return;
    }
    const observer = new ResizeObserver(() => {
      if (!sized()) return;
      observer.disconnect();
      if (this.ctx.api === api) apply();
    });
    observer.observe(container);
  }

  /**
   * Calls `onResize` (debounced) when the dock's size changes. Returns the disposer. Only used to re-plan, so a burst of
   * resize events costs one plan.
   */
  watchResize(api: DockviewApi, onResize: () => void, delayMs = 150): () => void {
    const container = this.ctx.container;
    if (!container || typeof ResizeObserver === 'undefined') return () => undefined;
    let timer: ReturnType<typeof setTimeout> | undefined;
    // The size the dock had when watching began: the observer's first callback (same size) is not a resize.
    const start = container.getBoundingClientRect();
    let last = `${Math.round(start.width)}x${Math.round(start.height)}`;
    const observer = new ResizeObserver(() => {
      const box = container.getBoundingClientRect();
      const now = `${Math.round(box.width)}x${Math.round(box.height)}`;
      if (now === last) return;
      last = now;
      clearTimeout(timer);
      timer = setTimeout(() => {
        if (this.ctx.api === api && !this.ctx.restoring) onResize();
      }, delayMs);
    });
    observer.observe(container);
    return () => {
      clearTimeout(timer);
      observer.disconnect();
    };
  }

  /** Withdraws the "tabbed to fit" notification (everything is back where it was). */
  withdrawFitted(): void {
    this.lazy.get(NotificationsStore, null)?.withdrawLocal('layout.fit');
  }

  /** Calls `onDragEnd` after the user releases a splitter they pressed (the only moment their sizes are a wish). Returns the disposer. */
  watchSashDrag(onDragEnd: () => void): () => void {
    const container = this.ctx.container;
    if (!container) return () => undefined;
    let sash = false;
    const down = (e: Event) => {
      sash = !!(e.target as Element | null)?.closest?.('.dv-sash');
    };
    const up = () => {
      if (!sash) return;
      sash = false;
      onDragEnd();
    };
    container.addEventListener('pointerdown', down, true);
    container.ownerDocument.addEventListener('pointerup', up, true);
    return () => {
      container.removeEventListener('pointerdown', down, true);
      container.ownerDocument.removeEventListener('pointerup', up, true);
    };
  }

  /** The verdict for the Explorer (see `DockService.explorerVerdict`). */
  explorerVerdict(api: DockviewApi, open: boolean, explorerWidth: number): 'hide' | 'show' | 'keep' {
    if (!(api.width > 0 && api.height > 0)) return 'keep';
    const box = this.ctx.container?.getBoundingClientRect();
    if (box && (box.width <= 0 || box.height <= 0)) return 'keep';
    const viewport = { width: api.width + (open ? explorerWidth : 0), height: api.height };
    const explorer: OpenView = { id: EXPLORER_ID, layout: EXPLORER_LAYOUT, group: EXPLORER_ID };
    const views = this.openViews(api, [explorer]);
    if (open) return planLayout(viewport, views, this.hints()).hidden.some((h) => h.id === EXPLORER_ID) ? 'hide' : 'keep';
    return restorable(viewport, views, [EXPLORER_ID], GROW_BACK_MARGIN, this.hints()).includes(EXPLORER_ID) ? 'show' : 'keep';
  }

  /** The one notification for a saved layout that was fitted to this window, with Undo (ADR 0056, section 3). */
  notifyFitted(undo: () => void): void {
    const store = this.lazy.get(NotificationsStore, null);
    store?.postLocal({
      key: 'layout.fit',
      source: 'workspace',
      severity: 'info',
      title: $localize`:@@dock.layout.fit.title:Some panes were tabbed to fit this window`,
      body: $localize`:@@dock.layout.fit.body:Your saved layout is kept; it returns when the window is large enough.`,
      actionLabel: $localize`:@@dock.layout.fit.undo:Undo`,
      run: undo,
    });
  }
}
