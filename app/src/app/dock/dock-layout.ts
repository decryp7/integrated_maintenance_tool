import { inject, Injectable, Injector } from '@angular/core';
import type { DockviewApi, DockviewGroupPanel } from 'dockview';
import { LayoutPlan, OpenView, planLayout, SavedHints } from '../layout-plan/layout-plan';
import { NotificationsStore } from '../notifications/notifications.store';
import { autoHideReduce } from './auto-hide';
import { DockContext } from './dock-context';
import { viewLayout, viewRow } from './view-layout';

/** What `fit` is asked to do. */
export interface FitOptions {
  /** Set the planned sizes of the non-anchor columns and rows (on open and on a load); else only tab/hide and constrain. */
  sizes?: boolean;
  /** Panels just opened: their current size is not a wish, and a tool window that has no room slides out for the user to see. */
  fresh?: readonly string[];
}

/**
 * The thin edge between `planLayout` (pure, `layout-plan/`) and dockview (ADR 0056): reads the open panels as
 * `OpenView`s, and translates a `LayoutPlan` into `moveTo` (tabbed), auto-hide (hidden), `setConstraints` (every
 * group, from the views it holds) and `setSize`. Decides nothing itself.
 */
@Injectable({ providedIn: 'root' })
export class DockLayout {
  private readonly ctx = inject(DockContext);
  private readonly lazy = inject(Injector);

  /** The docked (not floating) panels as the planner sees them. */
  openViews(api: DockviewApi): OpenView[] {
    const out: OpenView[] = [];
    for (const p of api.panels ?? []) {
      if (p.group.api.location.type !== 'grid') continue;
      const view = this.ctx.views.find(p.id);
      if (!view) continue;
      out.push({ id: p.id, layout: viewLayout(view), row: viewRow(view), ...(view.fullHeight ? { fullHeight: true } : {}) });
    }
    return out;
  }

  /** The sizes the groups have now, as the planner's preferences (a user's drag is the new `pref`). */
  hints(api: DockviewApi, unhinted: readonly string[] = []): SavedHints {
    const sizes: Record<string, { width?: number; height?: number }> = {};
    for (const p of api.panels ?? []) {
      if (unhinted.includes(p.id) || p.group.api.location.type !== 'grid') continue;
      const { width, height } = p.group;
      sizes[p.id] = { ...(width > 0 ? { width } : {}), ...(height > 0 ? { height } : {}) };
    }
    return { sizes };
  }

  /** The plan for the panels docked now (null while the dock has no measured size). */
  plan(api: DockviewApi, options: FitOptions = {}): LayoutPlan | null {
    if (!(api.width > 0 && api.height > 0)) return null;
    // A container without a layout box (a detached or not yet laid out dock; dockview then reports its 100x100 default) is not a window.
    const box = this.ctx.container?.getBoundingClientRect();
    if (box && (box.width <= 0 || box.height <= 0)) return null;
    return planLayout({ width: api.width, height: api.height }, this.openViews(api), this.hints(api, options.fresh));
  }

  /**
   * Re-plans and applies: tabs and hides what the plan says yields, constrains every group and (with `sizes`) sets the
   * planned sizes. Returns the plan, or null when the dock is not measured yet (groups are still constrained).
   */
  fit(api: DockviewApi, options: FitOptions = {}): LayoutPlan | null {
    const plan = this.plan(api, options);
    if (plan) this.displace(api, plan, options.fresh ?? []);
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

  private displace(api: DockviewApi, plan: LayoutPlan, fresh: readonly string[]): void {
    for (const t of plan.tabbed) {
      const panel = api.getPanel(t.id);
      const target = api.getPanel(t.into);
      if (!panel || !target || panel.group === target.group) continue;
      // A view that yields joins the group as a background tab; the tab in front stays, unless the user just opened it.
      const front = target.group.activePanel;
      panel.api.moveTo({ group: target.group, position: 'center' });
      if (fresh.includes(t.id)) panel.api.setActive();
      else front?.api.setActive();
    }
    for (const h of plan.hidden) {
      const panel = api.getPanel(h.id);
      if (!panel || h.edge !== 'right') continue;
      const view = this.ctx.views.find(h.id);
      if (!view) continue;
      api.removePanel(panel);
      const item = { id: h.id, title: view.title, side: 'right' as const, size: viewLayout(view).pref.width, slot: { referencePanel: null, direction: 'within' as const } };
      this.ctx.autoHide.update((s) => {
        const hidden = autoHideReduce(s, { type: 'unpin', item });
        return fresh.includes(h.id) ? autoHideReduce(hidden, { type: 'activate', id: h.id }) : hidden;
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
        if (r.anchor) continue;
        const g = this.groupOf(api, r.views);
        if (g && Math.abs(g.height - r.height) > 1) g.api.setSize({ height: r.height });
      }
    }
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

  /** The one notification for a saved layout that was fitted to this window, with Undo (ADR 0056, section 3). */
  notifyFitted(undo: () => void): void {
    const store = this.lazy.get(NotificationsStore, null);
    store?.postLocal({
      key: 'layout:fit',
      source: 'workspace',
      severity: 'info',
      title: $localize`:@@dock.layout.fit.title:Some panes were tabbed to fit this window`,
      body: $localize`:@@dock.layout.fit.body:Your saved layout is kept; it returns when the window is large enough.`,
      actionLabel: $localize`:@@dock.layout.fit.undo:Undo`,
      run: undo,
    });
  }
}
