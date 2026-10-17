import { ApplicationRef, computed, effect, EnvironmentInjector, inject, Injectable, Injector, signal, untracked } from '@angular/core';
import type { DockviewApi, DockviewGroupPanel, IDockviewPanel } from 'dockview';
import { dockview, loadDockview } from './dockview-loader';
import { Logger } from '../core/logger';
import { WorkspaceStore } from '../core/workspace.store';
import { ImtView, ViewPosition } from '../extensions/tokens';
import { ViewRegistry } from '../extensions/view-registry';
import { ContextMenuService } from '../ui/context-menu.service';
import { AngularContentRenderer, MissingViewRenderer } from './angular-panel';
import { AutoHideEvent, AutoHideItem, autoHideReduce, AutoHideState, EMPTY_AUTO_HIDE } from './auto-hide';
import { adjacentNeighbour, DockSlot, isLayoutTarget, nearestSide, Point, Rect, resolveGuideDrop } from './dock-geometry';
import { DockTarget, GroupBox, hitDockTarget, targetPreview } from './dock-target';
import { TabDragOut } from './tab-drag-out';
import { DropGuides } from './drop-guides';
import { FullScreenPrefs, OsWindow } from './os-window';
import { GroupActions } from './group-actions';
import { DockExtras, EMPTY_EXTRAS, LAYOUT_STORE, SavedLayout } from './layout-store';
import { PanelAction, panelMenu, panelsToClose, pinnedIndex } from './panel-menu';
import { PANEL_WINDOWS, PanelWindows } from './panel-windows';
import { PanelTab, TAB_COMPONENT, tabPanelId } from './panel-tab';
import { cycle, openSwitcher, selected, SwitcherState, switcherItems, touchMru } from './switcher';
import { viewKind, ViewKind } from './view-kind';

/** Ids of the built-in panels the default layout is made of. */
export const PANEL = { timeline: 'timeline', events: 'events', inspector: 'inspector', metrics: 'metrics', profile: 'profile' } as const;

const PERSIST_DELAY_MS = 250;
const INSPECTOR_WIDTH = 372;
const TIMELINE_HEIGHT = 300;
const FLOAT_SIZE = { width: 560, height: 380 };
/** Size of a new tool window group at a window edge, px. */
const TOOL_SIZE = { side: 340, bottom: 240 };

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

/** An inline problem shown over the dock (e.g. a pop-out window could not be created), with an optional retry. */
export interface DockNotice {
  message: string;
  retry?: () => void;
}

/**
 * Owns the dockview instance: builds the default layout (documents in the centre, tool windows around them, as in
 * Visual Studio), restores/persists the user's layout through the `LayoutStore`, and opens, closes, focuses,
 * floats, pops out, auto-hides and pins panels for the views in `ViewRegistry`. Components use this service
 * instead of touching dockview.
 */
@Injectable({ providedIn: 'root' })
export class DockService {
  private readonly views = inject(ViewRegistry);
  private readonly store = inject(WorkspaceStore);
  private readonly layoutStore = inject(LAYOUT_STORE);
  private readonly appRef = inject(ApplicationRef);
  private readonly injector = inject(EnvironmentInjector);
  private readonly lazy = inject(Injector);
  private readonly logger = inject(Logger);
  private readonly menus = inject(ContextMenuService);

  private api: DockviewApi | null = null;
  private container: HTMLElement | null = null;
  private persistTimer: ReturnType<typeof setTimeout> | undefined;
  private restoring = false;
  private disposers: { dispose(): void }[] = [];
  /** True when the layout was built from defaults rather than restored from the store. */
  private fresh = false;
  /** Conditional views that were available at the last check, to detect "became available". */
  private readonly wasAvailable = new Set<string>();
  private evaluated = false;
  /** In a pop-out window: views are shown by the host (set by the pop-out shell). */
  private remote: ((id: string) => void) | null = null;
  private mru: string[] = [];
  private fullScreenGroup: DockviewGroupPanel | null = null;
  private guides: DropGuides | null = null;
  private pendingAttach: HTMLElement | null = null;
  private fullScreenOs = false;

  private readonly docked = signal<ReadonlySet<string>>(new Set());
  private readonly autoHideState = signal<AutoHideState>(EMPTY_AUTO_HIDE);
  private readonly pinnedSet = signal<ReadonlySet<string>>(new Set());
  private readonly saved = signal<SavedLayout | null>(null);

  /** Ids of the panels currently open: docked, auto-hidden or popped out to a window. */
  readonly openIds = computed<ReadonlySet<string>>(() => {
    const ids = new Set(this.docked());
    for (const i of this.autoHideState().items) ids.add(i.id);
    for (const id of this.windows()?.poppedIds() ?? []) ids.add(id);
    return ids;
  });
  /** Auto-hidden tool windows and which one is slid out. */
  readonly autoHide = this.autoHideState.asReadonly();
  /** Tabs pinned to the left of their strip (they survive "Close all"). */
  readonly pinnedTabs = this.pinnedSet.asReadonly();
  /** The layout as last saved (projects and sessions save it too). */
  readonly savedLayout = this.saved.asReadonly();
  /** An inline problem to show over the dock. */
  readonly notice = signal<DockNotice | null>(null);
  /** The Ctrl+Tab navigator, when open. */
  readonly switcher = signal<SwitcherState | null>(null);
  readonly fullScreen = signal(false);
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

  private windows(): PanelWindows | null {
    return this.lazy.get(PANEL_WINDOWS, null);
  }

  isOpen(id: string): boolean {
    return this.openIds().has(id);
  }

  kindOf(id: string): ViewKind {
    return viewKind(this.views.find(id));
  }

  /** In a pop-out window: showing a view asks the host to show it (the pop-out has no layout of its own). */
  setRemote(remote: ((id: string) => void) | null): void {
    this.remote = remote;
  }

  /**
   * Creates the dock inside `container`, restoring the saved layout when there is a compatible one. dockview is
   * loaded on first use: until then this attaches as soon as it is there (synchronously once loaded).
   */
  attach(container: HTMLElement): void {
    const lib = dockview();
    if (!lib) {
      this.pendingAttach = container;
      void loadDockview().then(() => {
        if (this.pendingAttach === container) this.attach(container);
      });
      return;
    }
    this.detach();
    this.container = container;
    const api = lib.createDockview(container, {
      theme: { name: 'imt', className: 'imt-dockview' },
      floatingGroupBounds: 'boundedWithinViewport',
      disableTabsOverflowList: false,
      // Tabs and groups are dragged with pointer events, not HTML5 drag-and-drop: in Tauri on Windows the native
      // file-drop handler (which gives dropped files' paths) swallows HTML5 drags, and pointer events also tell
      // when a tab is released outside the window (to pop it out).
      dndStrategy: 'pointer',
      // VS-style docking: drops only land on the compass of a group and the guides at the layout's edges.
      dropPositionResolver: {
        // dockview's root target (the whole layout) fills the container; group content targets are smaller.
        resolve: (args) => {
          const box = container.getBoundingClientRect();
          const root = { width: container.offsetWidth || box.width, height: container.offsetHeight || box.height };
          const event = args.event as { clientX?: number; clientY?: number } | undefined;
          const inRoot = event?.clientX !== undefined && event.clientY !== undefined ? { x: event.clientX - box.left, y: event.clientY - box.top } : null;
          return resolveGuideDrop(args, isLayoutTarget(args, root), inRoot, root);
        },
      },
      createComponent: (o) => {
        const view = this.views.find(o.name);
        return view ? new AngularContentRenderer(view.component, this.appRef, this.injector, view.title) : new MissingViewRenderer(o.name);
      },
      defaultTabComponent: TAB_COMPONENT,
      createTabComponent: (o) => (o.name === TAB_COMPONENT ? new PanelTab((id, el) => this.openPanelMenu(id, el)) : undefined),
      createRightHeaderActionComponent: () =>
        new GroupActions({
          toggleFloat: (g) => this.toggleFloat(g),
          popOut: (g) => this.popOut(g.activePanel?.id ?? ''),
          autoHide: (g) => this.unpin(g.activePanel?.id ?? ''),
          isTool: (g) => !!g.activePanel && this.kindOf(g.activePanel.id) === 'tool',
          canPopOut: () => this.windows()?.canPopOut() ?? false,
          openMenu: (g, anchor) => g.activePanel && this.openPanelMenu(g.activePanel.id, anchor),
        }),
    });
    this.api = api;
    const onKey = (e: KeyboardEvent) => {
      const id = tabPanelId(e.target);
      if (!id) return;
      if (e.key === 'Delete') {
        e.preventDefault();
        this.close(id);
      } else if (e.key === 'ContextMenu' || (e.key === 'F10' && e.shiftKey)) {
        e.preventDefault();
        this.openPanelMenu(id, e.target as HTMLElement);
      }
    };
    const onContextMenu = (e: MouseEvent) => {
      const id = tabPanelId(e.target);
      if (!id) return;
      e.preventDefault();
      this.openPanelMenu(id, e.target as HTMLElement, { x: e.clientX, y: e.clientY });
    };
    // A tab dragged out of the window pops out where it is released (pointer events: see `dndStrategy`).
    const tabDrag = new TabDragOut(container, (id, at) => void this.popOut(id, at));
    container.addEventListener('keydown', onKey);
    container.addEventListener('contextmenu', onContextMenu);
    const guides = new DropGuides(container);
    this.guides = guides;
    this.disposers = [
      { dispose: () => container.removeEventListener('keydown', onKey) },
      { dispose: () => container.removeEventListener('contextmenu', onContextMenu) },
      tabDrag,
      guides,
      observeFloatingAria(container),
      api.onDidAddPanel(() => this.refreshOpen()),
      api.onDidRemovePanel(() => this.refreshOpen()),
      api.onDidLayoutFromJSON(() => this.refreshOpen()),
      api.onDidLayoutChange(() => {
        this.markPinnedTabs();
        this.schedulePersist();
      }),
      api.onDidActivePanelChange((e) => {
        if (e.panel) this.mru = touchMru(this.mru, e.panel.id);
      }),
    ];
    this.restoring = true;
    let extras: DockExtras = EMPTY_EXTRAS;
    try {
      const saved = this.layoutStore.load();
      if (saved) {
        try {
          api.fromJSON(saved.layout);
          extras = saved.extras;
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
      this.applyExtras(extras);
    } finally {
      this.restoring = false;
    }
    this.refreshOpen();
    this.evaluated = false;
    this.wasAvailable.clear();
    this.apiReady.set(true);
    if (extras.poppedOut.length) void this.windows()?.restore(extras.poppedOut);
  }

  detach(): void {
    this.pendingAttach = null;
    clearTimeout(this.persistTimer);
    this.disposers.forEach((d) => d.dispose());
    this.disposers = [];
    this.api?.dispose();
    this.api = null;
    this.container = null;
    this.guides = null;
    this.apiReady.set(false);
    this.docked.set(new Set());
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
      this.applyExtras(EMPTY_EXTRAS);
    } finally {
      this.restoring = false;
    }
    this.exitFullScreen();
    this.fresh = true;
    this.evaluated = false;
    this.wasAvailable.clear();
    this.refreshOpen();
    this.syncConditionalViews();
    this.schedulePersist();
  }

  /** Changes the tab title of an open panel (e.g. to show a count); does nothing when it is not open. */
  setTitle(id: string, title: string): void {
    const panel = this.api?.getPanel(id);
    if (panel && panel.title !== title) panel.api.setTitle(title);
  }

  /** Opens the view if needed and brings it to the front. Unknown or unavailable views are ignored. */
  show(id: string, options: { inactive?: boolean } = {}): void {
    if (this.remote) {
      this.remote(id);
      return;
    }
    const api = this.api;
    const view = this.views.find(id);
    if (!api || !view) return;
    if (this.windows()?.poppedIds().has(id)) {
      if (!options.inactive) this.windows()?.focus(id);
      return;
    }
    if (this.autoHideState().items.some((i) => i.id === id)) {
      if (!options.inactive) this.autoHideEvent({ type: 'activate', id });
      return;
    }
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
    if (this.windows()?.poppedIds().has(id)) {
      void this.windows()?.discard(id);
      return;
    }
    if (this.autoHideState().items.some((i) => i.id === id)) {
      this.autoHideState.update((s) => autoHideReduce(s, { type: 'pin', id }));
      this.schedulePersist();
      return;
    }
    const panel = this.api?.getPanel(id);
    if (panel) this.api?.removePanel(panel);
    this.setPinned(id, false);
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

  /* ------------------------------------------------------------------ slots */

  /** Where a docked panel sits, so it can be put back after a pop-out or auto-hide. */
  captureSlot(id: string): DockSlot | null {
    const api = this.api;
    const panel = api?.getPanel(id);
    if (!api || !panel) return null;
    const group = panel.group;
    const siblings = group.panels.filter((p) => p.id !== id);
    if (siblings.length) {
      const index = group.panels.findIndex((p) => p.id === id);
      return { referencePanel: (siblings[Math.max(0, index - 1)] ?? siblings[0]).id, direction: 'within', index };
    }
    const rect = rectOf(group.element);
    const others = api.groups
      .filter((g) => g !== group && g.api.location.type === 'grid' && g.activePanel)
      .map((g) => ({ rect: rectOf(g.element), value: g.activePanel!.id }));
    const near = rect ? adjacentNeighbour(rect, others.filter((o): o is { rect: Rect; value: string } => !!o.rect)) : null;
    if (!near) return { referencePanel: null, direction: 'within' };
    const size = near.direction === 'left' || near.direction === 'right' ? rect?.width : rect?.height;
    return { referencePanel: near.value, direction: near.direction, ...(size ? { size: Math.round(size) } : {}) };
  }

  /** Puts a panel back at a slot (or where its view opens by default when the reference is gone). */
  restoreSlot(id: string, slot: DockSlot | null, options: { inactive?: boolean } = {}): void {
    const api = this.api;
    const view = this.views.find(id);
    if (!api || !view || api.getPanel(id)) return;
    const ref = slot?.referencePanel ? api.getPanel(slot.referencePanel) : undefined;
    if (!slot || !ref) {
      this.addView(api, view, options.inactive ?? false);
      return;
    }
    const panel = api.addPanel({
      id,
      component: id,
      title: view.title,
      inactive: options.inactive ?? false,
      position: { referencePanel: ref.id, direction: slot.direction, ...(slot.direction === 'within' && slot.index !== undefined ? { index: slot.index } : {}) },
    });
    if (slot.size && slot.direction !== 'within') panel.group.api.setSize(slot.direction === 'left' || slot.direction === 'right' ? { width: slot.size } : { height: slot.size });
  }

  /** The panel's content size on screen (for the pop-out window size). */
  panelRect(id: string): Rect | null {
    const panel = this.api?.getPanel(id);
    return panel ? rectOf(panel.group.element) : null;
  }

  /** Removes a panel from the layout without closing its view (it moved to a pop-out window). */
  takeOut(id: string): void {
    const panel = this.api?.getPanel(id);
    if (panel) this.api?.removePanel(panel);
    this.refreshOpen();
    this.schedulePersist();
  }

  /* --------------------------------------------------------------- pop-outs */

  /** Pops a panel out into a real window ("Pop out to window"). */
  async popOut(id: string, at?: Point): Promise<boolean> {
    const windows = this.windows();
    if (!windows || !id || !this.api?.getPanel(id)) return false;
    return windows.popOut(id, at ? { at } : {});
  }

  /** Pops out the active panel (keyboard command). */
  popOutActive(): Promise<boolean> {
    return this.popOut(this.api?.activePanel?.id ?? '');
  }

  /* ------------------------------------------- drops from outside dockview */

  /** The guide under a point (client coordinates): where a pop-out dropped there would dock. */
  dropTargetAt(point: Point): DockTarget | null {
    const root = this.container ? rectOf(this.container) : null;
    return root ? hitDockTarget(point, root, this.groupBoxes()) : null;
  }

  /** Shows the dock guides for a pop-out window dragged over this one (null hides them). */
  showDropGuides(point: Point | null): void {
    if (!point) {
      this.guides?.hide();
      return;
    }
    const target = this.dropTargetAt(point);
    const root = this.container ? rectOf(this.container) : null;
    this.guides?.showAt(point, target && root ? targetPreview(target, root, this.groupBoxes()) : null);
  }

  /** Docks a panel that is not in the layout (a pop-out dropped on a guide) at that guide. */
  dockAt(id: string, target: DockTarget): boolean {
    const api = this.api;
    const view = this.views.find(id);
    if (!api || !view || api.getPanel(id)) return false;
    const direction = ({ center: 'within', left: 'left', right: 'right', top: 'above', bottom: 'below' } as const)[target.position];
    const group = target.kind === 'group' ? api.getGroup(target.groupId) : undefined;
    if (target.kind === 'group' && !group) return false;
    const panel = api.addPanel({ id, component: id, title: view.title, position: group ? { referenceGroup: group.id, direction } : { direction: direction === 'within' ? 'right' : direction } });
    panel.api.setActive();
    return true;
  }

  private groupBoxes(): GroupBox[] {
    return (this.api?.groups ?? [])
      .filter((g) => g.api.location.type === 'grid')
      .map((g) => ({ id: g.id, rect: rectOf(g.element.querySelector('.dv-content-container')) }))
      .filter((g): g is GroupBox => !!g.rect);
  }

  /* -------------------------------------------------------------- auto-hide */

  /** Collapses a docked tool window into a tab on the nearest side bar ("Auto hide"). */
  unpin(id: string): void {
    const api = this.api;
    const panel = api?.getPanel(id);
    if (!api || !panel || this.kindOf(id) !== 'tool' || panel.group.api.location.type !== 'grid') return;
    const rect = rectOf(panel.group.element);
    const root = this.container ? rectOf(this.container) : null;
    const side = rect && root ? nearestSide(rect, root) : 'right';
    const item: AutoHideItem = {
      id,
      title: this.views.find(id)?.title ?? id,
      side,
      slot: this.captureSlot(id) ?? { referencePanel: null, direction: 'within' },
      size: rect ? (side === 'bottom' ? rect.height : rect.width) : 320,
    };
    api.removePanel(panel);
    this.autoHideState.update((s) => autoHideReduce(s, { type: 'unpin', item }));
    this.refreshOpen();
    this.schedulePersist();
  }

  /** Docks an auto-hidden tool window back where it was ("Pin"). */
  pin(id: string): void {
    const item = this.autoHideState().items.find((i) => i.id === id);
    if (!item) return;
    this.autoHideState.update((s) => autoHideReduce(s, { type: 'pin', id }));
    this.restoreSlot(id, item.slot);
    this.api?.getPanel(id)?.api.setActive();
    this.refreshOpen();
    this.schedulePersist();
  }

  autoHideEvent(event: AutoHideEvent): void {
    this.autoHideState.update((s) => autoHideReduce(s, event));
    if (event.type === 'resize') this.schedulePersist();
  }

  /** Toggles auto-hide of the active tool window (or of the slid-out one). */
  toggleAutoHideActive(): void {
    const open = this.autoHideState().open;
    if (open) this.pin(open);
    else if (this.api?.activePanel) this.unpin(this.api.activePanel.id);
  }

  autoHideAll(): void {
    for (const p of [...(this.api?.panels ?? [])]) if (this.kindOf(p.id) === 'tool' && p.group.api.location.type === 'grid') this.unpin(p.id);
  }

  pinAll(): void {
    for (const i of [...this.autoHideState().items]) this.pin(i.id);
  }

  /* ------------------------------------------------------------ pinned tabs */

  setPinned(id: string, pinned: boolean): void {
    const was = this.pinnedSet().has(id);
    if (was === pinned) return;
    this.pinnedSet.update((s) => {
      const next = new Set(s);
      if (pinned) next.add(id);
      else next.delete(id);
      return next;
    });
    const panel = this.api?.getPanel(id);
    if (pinned && panel) {
      const order = panel.group.panels.map((p) => p.id);
      panel.api.moveTo({ group: panel.group, position: 'center', index: pinnedIndex(order, id, this.pinnedSet()) });
    }
    this.markPinnedTabs();
    this.schedulePersist();
  }

  private markPinnedTabs(): void {
    const pinned = this.pinnedSet();
    this.container?.querySelectorAll<HTMLElement>('.imt-tab[data-panel-id]').forEach((t) => {
      const on = pinned.has(t.dataset['panelId'] ?? '');
      t.classList.toggle('pinned', on);
      t.closest('.dv-tab')?.classList.toggle('imt-pinned-tab', on);
    });
  }

  /* ------------------------------------------------------------ panel menu */

  /** The panel menu (tab ▾, right-click, Shift+F10, Alt+-): VS's Window Position menu. */
  openPanelMenu(id: string, anchor: HTMLElement | null, at?: Point): void {
    const api = this.api;
    const panel = api?.getPanel(id);
    if (!api || !panel) return;
    const group = panel.group;
    const order = group.panels.map((p) => p.id);
    const floating = group.api.location.type === 'floating';
    const items = panelMenu({
      kind: this.kindOf(id),
      floating,
      hasSiblings: order.length > 1,
      hasRight: order.indexOf(id) < order.length - 1,
      groups: api.groups.filter((g) => g.api.location.type === 'grid').length,
      pinnedTab: this.pinnedSet().has(id),
      canPopout: this.windows()?.canPopOut() ?? false,
      inDocumentWell: group.panels.some((p) => p.id !== id && this.kindOf(p.id) === 'document'),
    });
    const box = anchor?.getBoundingClientRect();
    const x = at?.x ?? (box ? box.left : 100);
    const y = at?.y ?? (box ? box.bottom : 100);
    this.menus.open(
      x,
      y,
      items.map((i) => ({ label: i.label, disabled: i.disabled, run: () => this.runPanelAction(id, i.action) })),
      anchor,
    );
  }

  /** Opens the panel menu of the active panel (keyboard). */
  openActivePanelMenu(): void {
    const panel = this.api?.activePanel;
    if (!panel) return;
    const tab = this.container?.querySelector<HTMLElement>(`.imt-tab[data-panel-id="${CSS.escape(panel.id)}"]`) ?? null;
    this.openPanelMenu(panel.id, tab?.closest<HTMLElement>('[role="tab"]') ?? tab);
  }

  runPanelAction(id: string, action: PanelAction): void {
    const api = this.api;
    const panel = api?.getPanel(id);
    if (!api || !panel) return;
    const group = panel.group;
    const order = group.panels.map((p) => p.id);
    const closeAll = (ids: string[]) => ids.forEach((p) => this.close(p));
    switch (action) {
      case 'float':
        if (group.api.location.type !== 'floating') api.addFloatingGroup(group.panels.length > 1 ? panel : group, { ...FLOAT_SIZE, x: 140, y: 100 });
        break;
      case 'dock':
        if (group.api.location.type === 'floating') this.toggleFloat(group);
        break;
      case 'popout':
        void this.popOut(id);
        break;
      case 'dockAsDocument': {
        const well = this.documentWell(id);
        if (well && well !== group) panel.api.moveTo({ group: well, position: 'center' });
        else if (!well) panel.api.moveTo({ group, position: 'center' });
        break;
      }
      case 'autoHide':
        this.unpin(id);
        break;
      case 'close':
        this.close(id);
        break;
      case 'closeOthers':
        closeAll(panelsToClose(order, id, 'others', this.pinnedSet()));
        break;
      case 'closeRight':
        closeAll(panelsToClose(order, id, 'right', this.pinnedSet()));
        break;
      case 'closeAllDocuments':
        this.closeAllDocuments();
        break;
      case 'newHorizontalGroup':
        panel.api.moveTo({ group, position: 'bottom' });
        break;
      case 'newVerticalGroup':
        panel.api.moveTo({ group, position: 'right' });
        break;
      case 'moveToNextGroup': {
        const grid = api.groups.filter((g) => g.api.location.type === 'grid');
        const next = grid[(grid.indexOf(group) + 1) % grid.length];
        if (next && next !== group) panel.api.moveTo({ group: next, position: 'center' });
        break;
      }
      case 'pinTab':
        this.setPinned(id, true);
        break;
      case 'unpinTab':
        this.setPinned(id, false);
        break;
    }
  }

  /** Closes every document except pinned tabs ("Close all documents"). */
  closeAllDocuments(): void {
    const ids = (this.api?.panels ?? []).map((p) => p.id).filter((id) => this.kindOf(id) === 'document');
    for (const id of panelsToClose(ids, '', 'all', this.pinnedSet())) this.close(id);
  }

  /** The group documents go to: that of the most recently used document, else the one holding the timeline. */
  private documentWell(excluding?: string): DockviewGroupPanel | null {
    const api = this.api;
    if (!api) return null;
    const isWell = (g: DockviewGroupPanel) => g.api.location.type === 'grid' && g.panels.some((p) => p.id !== excluding && this.kindOf(p.id) === 'document');
    for (const id of this.mru) {
      const g = api.getPanel(id)?.group;
      if (g && id !== excluding && this.kindOf(id) === 'document' && isWell(g)) return g;
    }
    return api.getPanel(PANEL.timeline)?.group ?? api.groups.find(isWell) ?? null;
  }

  /* -------------------------------------------------------------- switcher */

  /** Opens the Ctrl+Tab navigator, or moves its selection when open. */
  switch(backwards = false): void {
    const current = this.switcher();
    if (current) {
      this.switcher.set(cycle(current, backwards ? -1 : 1));
      return;
    }
    const open = (this.api?.panels ?? []).map((p) => ({ id: p.id, title: p.title ?? p.id, kind: this.kindOf(p.id) }));
    const active = this.api?.activePanel?.id;
    const mru = active ? touchMru(this.mru, active) : this.mru;
    this.switcher.set(openSwitcher(switcherItems(mru, open), backwards));
  }

  /** Activates the navigator's selection and closes it. */
  commitSwitch(): void {
    const state = this.switcher();
    this.switcher.set(null);
    const item = state && selected(state);
    if (item) {
      this.show(item.id);
      this.focusPanel(item.id);
    }
  }

  cancelSwitch(): void {
    this.switcher.set(null);
  }

  /** Moves keyboard focus to a panel's tab (after switching to it). */
  private focusPanel(id: string): void {
    queueMicrotask(() => this.container?.querySelector<HTMLElement>(`.imt-tab[data-panel-id="${CSS.escape(id)}"]`)?.closest<HTMLElement>('[role="tab"]')?.focus());
  }

  /* ----------------------------------------------------------- full screen */

  /**
   * Window › Full screen (Shift+Alt+Enter), as in VS: shows only the document well with the app's chrome hidden,
   * and/or makes the OS window full screen (the `FullScreenPrefs` mode). The same shortcut toggles back; Escape
   * does not exit.
   */
  toggleFullScreen(): void {
    if (this.fullScreen()) {
      this.exitFullScreen();
      return;
    }
    const mode = this.lazy.get(FullScreenPrefs).mode();
    this.fullScreen.set(true);
    this.fullScreenOs = mode !== 'chrome';
    if (mode !== 'os') {
      const group = this.documentWell();
      document.documentElement.classList.add('imt-full-screen');
      if (group && !group.api.isMaximized()) {
        group.api.maximize();
        this.fullScreenGroup = group;
      }
    }
    if (this.fullScreenOs) void this.lazy.get(OsWindow).setFullscreen(true);
  }

  private exitFullScreen(): void {
    if (!this.fullScreen()) return;
    this.fullScreen.set(false);
    document.documentElement.classList.remove('imt-full-screen');
    if (this.fullScreenGroup?.api.isMaximized()) this.fullScreenGroup.api.exitMaximized();
    this.fullScreenGroup = null;
    if (this.fullScreenOs) void this.lazy.get(OsWindow).setFullscreen(false);
    this.fullScreenOs = false;
  }

  /* --------------------------------------------------------------- layouts */

  /** The whole arrangement (grid, auto-hidden tool windows, pinned tabs, pop-outs), or null before attach. */
  currentLayout(): SavedLayout | null {
    const api = this.api;
    if (!api) return null;
    return {
      layout: api.toJSON(),
      extras: {
        autoHidden: [...this.autoHideState().items],
        pinnedTabs: [...this.pinnedSet()],
        poppedOut: this.windows()?.records() ?? [],
      },
    };
  }

  /** Applies a saved arrangement (named layout, project, session). Falls back to the default when it is broken. */
  applyLayout(saved: SavedLayout): boolean {
    const api = this.api;
    if (!api) return false;
    const windows = this.windows();
    for (const id of [...(windows?.poppedIds() ?? [])]) if (!saved.extras.poppedOut.some((p) => p.id === id)) void windows?.discard(id);
    this.restoring = true;
    let ok = true;
    try {
      try {
        api.fromJSON(saved.layout);
        this.applyExtras(saved.extras);
      } catch (e) {
        ok = false;
        this.logger.warn('ui.layout', 'Window layout could not be applied; using the default', { Error: String(e) });
        api.clear();
        this.buildDefault(api);
        this.applyExtras(EMPTY_EXTRAS);
      }
    } finally {
      this.restoring = false;
    }
    this.exitFullScreen();
    this.refreshOpen();
    this.syncConditionalViews();
    this.schedulePersist();
    const popped = windows?.poppedIds() ?? new Set<string>();
    if (ok) void windows?.restore(saved.extras.poppedOut.filter((p) => !popped.has(p.id)));
    return ok;
  }

  private applyExtras(extras: DockExtras): void {
    const docked = new Set((this.api?.panels ?? []).map((p) => p.id));
    const known = (id: string) => !!this.views.find(id);
    this.autoHideState.set({ items: extras.autoHidden.filter((i) => known(i.id) && !docked.has(i.id)), open: null, openedBy: null });
    this.pinnedSet.set(new Set(extras.pinnedTabs.filter((id) => docked.has(id))));
    this.markPinnedTabs();
  }

  /** Persists the layout soon (pop-outs moved, auto-hide resized). */
  persistLater(): void {
    this.schedulePersist();
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
    // A tool window without a place opens in a tool group at its window edge (VS: right, or bottom).
    if (viewKind(view) === 'tool') {
      const sameKind = this.toolGroupFor(api, view.defaultPosition ?? 'right');
      if (sameKind) return api.addPanel({ id: view.id, component: view.id, title: view.title, inactive, position: { referencePanel: sameKind, direction: 'within' } });
      if (view.defaultPosition === 'bottom' || !api.getPanel(PANEL.inspector)) {
        const bottom = view.defaultPosition === 'bottom';
        const panel = api.addPanel({ id: view.id, component: view.id, title: view.title, inactive, position: { direction: bottom ? 'below' : 'right' } });
        panel.group.api.setSize(bottom ? { height: TOOL_SIZE.bottom } : { width: TOOL_SIZE.side });
        return panel;
      }
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

  /** A docked tool window already open on that side, to tab a new one with. */
  private toolGroupFor(api: DockviewApi, position: ViewPosition): string | null {
    const root = this.container ? rectOf(this.container) : null;
    const want = position === 'bottom' ? 'bottom' : 'right';
    for (const p of api.panels) {
      if (this.kindOf(p.id) !== 'tool' || p.group.api.location.type !== 'grid') continue;
      const rect = rectOf(p.group.element);
      if (!root || !rect || nearestSide(rect, root) === want) return p.id;
    }
    return null;
  }

  private anchorFor(api: DockviewApi, position: ViewPosition, exclude: ReadonlySet<string> = new Set()): string | null {
    const id = ANCHORS[position].find((a) => !exclude.has(a) && api.getPanel(a));
    return id ?? api.panels.find((p) => !exclude.has(p.id))?.id ?? null;
  }

  /**
   * The clean default: documents in the centre (timeline and metrics tabbed; the query bar over the events), the
   * inspector tool window on the right. Everything else (profile editor, analysis, tools, ...) opens on demand.
   */
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
      const open = this.isOpen(view.id);
      if (!available.has(view.id)) {
        if (open) this.close(view.id);
        this.wasAvailable.delete(view.id);
        continue;
      }
      const becameAvailable = !this.wasAvailable.has(view.id);
      this.wasAvailable.add(view.id);
      // Only views that ask for it open by themselves (the rest are offered); after a restore the user's layout
      // wins, so only newly available views are opened (in the background).
      if (view.autoOpen && becameAvailable && !open && (this.evaluated || this.fresh)) this.addView(api, view, true);
    }
    this.evaluated = true;
  }

  private refreshOpen(): void {
    this.docked.set(new Set((this.api?.panels ?? []).map((p) => p.id)));
  }

  private schedulePersist(): void {
    if (this.restoring || !this.api) return;
    clearTimeout(this.persistTimer);
    this.persistTimer = setTimeout(() => {
      const layout = this.currentLayout();
      if (!layout) return;
      this.layoutStore.save(layout);
      this.saved.set(layout);
    }, PERSIST_DELAY_MS);
  }
}

function rectOf(el: Element | null | undefined): Rect | null {
  const r = el?.getBoundingClientRect?.();
  return r ? { x: r.left, y: r.top, width: r.width, height: r.height } : null;
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
