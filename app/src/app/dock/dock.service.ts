import { ApplicationRef, effect, EnvironmentInjector, inject, Injectable, signal, untracked } from '@angular/core';
import { createDockview, type DockviewApi, type DockviewGroupPanel, type IDockviewPanel } from 'dockview';
import { Logger } from '../core/logger';
import { WorkspaceStore } from '../core/workspace.store';
import { ImtView, ViewPosition } from '../extensions/tokens';
import { ViewRegistry } from '../extensions/view-registry';
import { AngularContentRenderer, MissingViewRenderer } from './angular-panel';
import { GroupActions } from './group-actions';
import { PanelTab, TAB_COMPONENT, tabPanelId } from './panel-tab';
import { LAYOUT_STORE } from './layout-store';

/** Ids of the built-in panels the default layout is made of. */
export const PANEL = { timeline: 'timeline', events: 'events', inspector: 'inspector', metrics: 'metrics', profile: 'profile' } as const;

const PERSIST_DELAY_MS = 250;
const INSPECTOR_WIDTH = 372;
const TIMELINE_HEIGHT = 300;
const FLOAT_SIZE = { width: 560, height: 380 };

/** A view with a fixed place in the default layout, relative to a built-in panel. */
interface Pin {
  referencePanel: string;
  direction: 'above' | 'below' | 'left' | 'right';
  /** Height of the view's group in px, for views placed above or below. */
  height?: number;
}

/** Id of the query bar view (contributed by the analysis plugin). */
export const QUERY_VIEW = 'query';
/** Tab strip plus one line of query input; dockview groups are otherwise at least 100px tall. */
export const QUERY_HEIGHT = 80;

/**
 * Views that open in their own group at a fixed place instead of tabbed by `defaultPosition`: the
 * query bar sits right above the events grid it filters. Used by the default layout and whenever
 * such a view is (re)opened while its reference panel is in the layout.
 */
const PINNED: Readonly<Record<string, Pin>> = {
  [QUERY_VIEW]: { referencePanel: PANEL.events, direction: 'above', height: QUERY_HEIGHT },
};

/** Panels a new view with a given default position is placed next to (first one that exists wins). */
const ANCHORS: Record<ViewPosition, string[]> = {
  center: [PANEL.timeline, PANEL.metrics, PANEL.profile],
  right: [PANEL.inspector],
  bottom: [PANEL.events],
};

/**
 * Owns the dockview instance: builds the default layout, restores/persists the user's layout
 * through the `LayoutStore`, and opens, closes and focuses panels for the views in `ViewRegistry`.
 * Components use this service instead of touching dockview.
 */
@Injectable({ providedIn: 'root' })
export class DockService {
  private readonly views = inject(ViewRegistry);
  private readonly store = inject(WorkspaceStore);
  private readonly layoutStore = inject(LAYOUT_STORE);
  private readonly appRef = inject(ApplicationRef);
  private readonly injector = inject(EnvironmentInjector);
  private readonly logger = inject(Logger);

  private api: DockviewApi | null = null;
  private persistTimer: ReturnType<typeof setTimeout> | undefined;
  private restoring = false;
  private disposers: { dispose(): void }[] = [];
  /** True when the layout was built from defaults rather than restored from the store. */
  private fresh = false;
  /** Conditional views that were available at the last check, to detect "became available". */
  private readonly wasAvailable = new Set<string>();
  private evaluated = false;

  private readonly openSet = signal<ReadonlySet<string>>(new Set());
  /** Ids of the panels currently in the layout. */
  readonly openIds = this.openSet.asReadonly();
  private readonly apiReady = signal(false);

  constructor() {
    // Jumping to a record (marker click, "show context") must reveal the grid.
    effect(() => {
      const anchor = this.store.anchor();
      if (anchor && this.apiReady()) untracked(() => this.show(PANEL.events));
    });
    // Conditional views (e.g. the registry viewer) follow what the workspace contains.
    effect(() => {
      const summary = this.store.summary();
      this.views.available();
      if (summary && this.apiReady()) untracked(() => this.syncConditionalViews());
    });
  }

  isOpen(id: string): boolean {
    return this.openSet().has(id);
  }

  /** Creates the dock inside `container`, restoring the saved layout when there is a compatible one. */
  attach(container: HTMLElement): void {
    this.detach();
    const api = createDockview(container, {
      theme: { name: 'imt', className: 'imt-dockview' },
      floatingGroupBounds: 'boundedWithinViewport',
      disableTabsOverflowList: false,
      createComponent: (o) => {
        const view = this.views.find(o.name);
        return view ? new AngularContentRenderer(view.component, this.appRef, this.injector, view.title) : new MissingViewRenderer(o.name);
      },
      defaultTabComponent: TAB_COMPONENT,
      createTabComponent: (o) => (o.name === TAB_COMPONENT ? new PanelTab() : undefined),
      createRightHeaderActionComponent: () => new GroupActions({ toggleFloat: (g) => this.toggleFloat(g) }),
    });
    this.api = api;
    const onKey = (e: KeyboardEvent) => {
      const id = e.key === 'Delete' ? tabPanelId(e.target) : null;
      if (id) {
        e.preventDefault();
        this.close(id);
      }
    };
    container.addEventListener('keydown', onKey);
    this.disposers = [
      { dispose: () => container.removeEventListener('keydown', onKey) },
      observeFloatingAria(container),
      api.onDidAddPanel(() => this.refreshOpen()),
      api.onDidRemovePanel(() => this.refreshOpen()),
      api.onDidLayoutFromJSON(() => this.refreshOpen()),
      api.onDidLayoutChange(() => this.schedulePersist()),
    ];
    this.restoring = true;
    try {
      const saved = this.layoutStore.load();
      if (saved) {
        try {
          api.fromJSON(saved);
        } catch (e) {
          this.logger.warn('ui.layout', 'Saved layout could not be restored; using the default', { Error: String(e) });
          this.layoutStore.clear();
          api.clear();
          this.fresh = true;
          this.buildDefault(api);
        }
      } else {
        this.fresh = true;
        this.buildDefault(api);
      }
    } finally {
      this.restoring = false;
    }
    this.refreshOpen();
    this.evaluated = false;
    this.wasAvailable.clear();
    this.apiReady.set(true);
  }

  detach(): void {
    clearTimeout(this.persistTimer);
    this.disposers.forEach((d) => d.dispose());
    this.disposers = [];
    this.api?.dispose();
    this.api = null;
    this.apiReady.set(false);
    this.openSet.set(new Set());
  }

  /** Restores the default arrangement and forgets the saved one. */
  reset(): void {
    const api = this.api;
    if (!api) return;
    this.layoutStore.clear();
    this.restoring = true;
    try {
      api.clear();
      this.buildDefault(api);
    } finally {
      this.restoring = false;
    }
    this.fresh = true;
    this.evaluated = false;
    this.wasAvailable.clear();
    this.refreshOpen();
    this.syncConditionalViews();
    this.schedulePersist();
  }

  /** Opens the view if needed and brings it to the front. Unknown or unavailable views are ignored. */
  /** Changes the tab title of an open panel (e.g. to show a count); does nothing when it is not open. */
  setTitle(id: string, title: string): void {
    const panel = this.api?.getPanel(id);
    if (panel && panel.title !== title) panel.api.setTitle(title);
  }

  show(id: string, options: { inactive?: boolean } = {}): void {
    const api = this.api;
    const view = this.views.find(id);
    if (!api || !view) return;
    const existing = api.getPanel(id);
    if (existing) {
      if (api.hasMaximizedGroup() && !existing.group.api.isMaximized()) api.exitMaximizedGroup();
      if (!options.inactive) existing.api.setActive();
      return;
    }
    if (!this.views.available().includes(view)) return;
    this.addView(api, view, options.inactive ?? false);
  }

  close(id: string): void {
    const panel = this.api?.getPanel(id);
    if (panel) this.api?.removePanel(panel);
  }

  toggle(id: string): void {
    if (this.isOpen(id)) this.close(id);
    else this.show(id);
  }

  /** Floats a docked group in the window, or docks a floating group back into the grid. */
  toggleFloat(group: DockviewGroupPanel): void {
    const api = this.api;
    if (!api) return;
    if (group.api.location.type === 'floating') {
      // Dock next to where the view would open by default: tabbed with the timeline, below it, or to its right.
      const position = this.views.find(group.panels[0]?.id ?? '')?.defaultPosition ?? 'center';
      const exclude = new Set(group.panels.map((p) => p.id));
      const anchor = api.getPanel(this.anchorFor(api, position, exclude) ?? '');
      if (anchor) group.api.moveTo({ group: anchor.group, position: position === 'center' ? 'center' : position });
      else group.api.moveTo({ position: 'right' });
    } else {
      api.addFloatingGroup(group, { ...FLOAT_SIZE, x: 140, y: 100 });
    }
  }

  private addView(api: DockviewApi, view: ImtView, inactive: boolean): IDockviewPanel {
    const pin = PINNED[view.id];
    if (pin && api.getPanel(pin.referencePanel)) {
      // dockview's default minimum group height (100px) is taller than a query bar; the panel's own
      // minimum (saved with the layout) lets its group be that short.
      const size = pin.height ? { initialHeight: pin.height, minimumHeight: pin.height } : {};
      const panel = api.addPanel({ id: view.id, component: view.id, title: view.title, inactive, ...size, position: { referencePanel: pin.referencePanel, direction: pin.direction } });
      if (pin.height) panel.group.api.setSize({ height: pin.height });
      return panel;
    }
    const ref = this.anchorFor(api, view.defaultPosition ?? 'center');
    return api.addPanel({
      id: view.id,
      component: view.id,
      title: view.title,
      inactive,
      ...(ref ? { position: { referencePanel: ref, direction: 'within' as const } } : {}),
    });
  }

  private anchorFor(api: DockviewApi, position: ViewPosition, exclude: ReadonlySet<string> = new Set()): string | null {
    const id = ANCHORS[position].find((a) => !exclude.has(a) && api.getPanel(a));
    return id ?? api.panels.find((p) => !exclude.has(p.id))?.id ?? null;
  }

  private buildDefault(api: DockviewApi): void {
    const add = (id: string, position?: Parameters<DockviewApi['addPanel']>[0]['position']) => {
      const view = this.views.find(id);
      if (view) api.addPanel({ id, component: id, title: view.title, ...(position ? { position } : {}) });
    };
    add(PANEL.timeline);
    add(PANEL.inspector, { referencePanel: PANEL.timeline, direction: 'right' });
    add(PANEL.events, { referencePanel: PANEL.timeline, direction: 'below' });
    const query = this.views.find(QUERY_VIEW);
    if (query) this.addView(api, query, false);
    add(PANEL.metrics, { referencePanel: PANEL.timeline, direction: 'within' });
    add(PANEL.profile, { referencePanel: PANEL.timeline, direction: 'within' });
    api.getPanel(PANEL.timeline)?.api.setActive();
    api.getPanel(PANEL.inspector)?.group.api.setSize({ width: INSPECTOR_WIDTH });
    api.getPanel(PANEL.timeline)?.group.api.setSize({ height: TIMELINE_HEIGHT });
    api.getPanel(QUERY_VIEW)?.group.api.setSize({ height: QUERY_HEIGHT });
  }

  /** Opens/closes views that depend on the workspace (`ImtView.when`). */
  private syncConditionalViews(): void {
    const api = this.api;
    if (!api) return;
    const available = new Set(this.views.available().map((v) => v.id));
    for (const view of this.views.all) {
      if (!view.when) continue;
      const open = !!api.getPanel(view.id);
      if (!available.has(view.id)) {
        if (open) this.close(view.id);
        this.wasAvailable.delete(view.id);
        continue;
      }
      const becameAvailable = !this.wasAvailable.has(view.id);
      this.wasAvailable.add(view.id);
      // After a restore the user's layout wins; only newly available views are opened (in the background).
      if (becameAvailable && !open && (this.evaluated || this.fresh)) this.addView(api, view, true);
    }
    this.evaluated = true;
  }

  private refreshOpen(): void {
    this.openSet.set(new Set((this.api?.panels ?? []).map((p) => p.id)));
  }

  private schedulePersist(): void {
    if (this.restoring || !this.api) return;
    clearTimeout(this.persistTimer);
    this.persistTimer = setTimeout(() => {
      if (this.api) this.layoutStore.save(this.api.toJSON());
    }, PERSIST_DELAY_MS);
  }
}

/**
 * Dockview marks floating groups `role="dialog"` with `aria-modal="false"` and a stacking
 * `aria-level` (0, 1, ...). axe rejects both on a dialog (aria-level is not allowed there and 0 is
 * invalid; aria-modal=false is the default), so they are removed as groups are floated or restored.
 */
export function observeFloatingAria(root: HTMLElement): { dispose(): void } {
  const fix = () =>
    root.querySelectorAll('.dv-resize-container[aria-modal], .dv-resize-container[aria-level]').forEach((el) => {
      el.removeAttribute('aria-modal');
      el.removeAttribute('aria-level');
    });
  const observer = new MutationObserver(fix);
  observer.observe(root, { childList: true, subtree: true, attributes: true, attributeFilter: ['aria-modal', 'aria-level'] });
  fix();
  return { dispose: () => observer.disconnect() };
}
