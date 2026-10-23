import { ApplicationRef, computed, EnvironmentInjector, inject, Injectable, Injector, signal } from '@angular/core';
import type { DockviewApi, DockviewGroupPanel } from 'dockview';
import { dockview, loadDockview } from './dockview-loader';
import { Logger } from '../core/logger';
import { ContextMenuService } from '../ui/context-menu.service';
import { AngularContentRenderer, MissingViewRenderer } from './angular-panel';
import { AutoHideEvent, EMPTY_AUTO_HIDE } from './auto-hide';
import { DockAutoHide } from './dock-auto-hide';
import { DockContext } from './dock-context';
import { keepGroupsUnscrolled, labelTabOverflow, observeFloatingAria } from './dock-dom';
import { DockSlot, isLayoutTarget, Point, Rect, resolveGuideDrop, Side } from './dock-geometry';
import { DockPersistence } from './dock-persistence';
import { DockPlacement } from './dock-placement';
import { DockPolicy } from './dock-policy';
import { DockTarget } from './dock-target';
import { FloatCue, TabDropOutcome } from './float-cue';
import { FloatingMove } from './floating-move';
import { TabDragOut } from './tab-drag-out';
import { DragOutCue } from './drag-out-cue';
import { DropGuides } from './drop-guides';
import { FullScreenPrefs, OsWindow } from './os-window';
import { GroupActions } from './group-actions';
import { DockExtras, SavedLayout } from './layout-store';
import { PanelAction, panelMenu, panelsToClose, pinnedIndex } from './panel-menu';
import { PanelTab, TAB_COMPONENT, tabPanelId } from './panel-tab';
import { cycle, openSwitcher, selected, SwitcherState, switcherItems, touchMru } from './switcher';
import { viewKind, ViewKind } from './view-kind';
import { buildPerspective, BuiltInPerspective } from './perspective-layouts';

export { PANEL } from './dock-context';
export { defaultAutoHidden } from './dock-placement';
export { keepGroupsUnscrolled, observeFloatingAria } from './dock-dom';

/** An inline problem shown over the dock (e.g. a pop-out window could not be created), with an optional retry. */
export interface DockNotice {
  message: string;
  retry?: () => void;
}

/**
 * The dock, as components see it: creates the dockview instance, restores/persists the user's layout, and opens,
 * closes, focuses, floats, pops out, auto-hides and pins panels for the views in `ViewRegistry` (documents in the
 * centre, tool windows around them, as in Visual Studio). It coordinates focused services:
 * - `DockPlacement`: where panels go (default layout, new views, slots, floating, guides);
 * - `DockAutoHide`: auto-hidden tool windows;
 * - `DockPersistence`: saving the arrangement;
 * - `DockPolicy`: the workspace's "on X, show Y" rules.
 * Components use this service instead of touching dockview.
 */
@Injectable({ providedIn: 'root' })
export class DockService {
  private readonly ctx = inject(DockContext);
  private readonly placement = inject(DockPlacement);
  private readonly autoHider = inject(DockAutoHide);
  private readonly persistence = inject(DockPersistence);
  private readonly views = this.ctx.views;
  private readonly appRef = inject(ApplicationRef);
  private readonly injector = inject(EnvironmentInjector);
  private readonly lazy = inject(Injector);
  private readonly logger = inject(Logger);
  private readonly menus = inject(ContextMenuService);

  private disposers: { dispose(): void }[] = [];
  /** True when the layout was built from defaults rather than restored from the store. */
  private fresh = false;
  /** Conditional views that were available at the last check, to detect "became available". */
  private readonly wasAvailable = new Set<string>();
  private evaluated = false;
  /** In a pop-out window: views are shown by the host (set by the pop-out shell). */
  private remote: ((id: string) => void) | null = null;
  private fullScreenGroup: DockviewGroupPanel | null = null;
  private pendingAttach: HTMLElement | null = null;
  private fullScreenOs = false;

  /** Ids of the panels currently open: docked, auto-hidden or popped out to a window. */
  readonly openIds = computed<ReadonlySet<string>>(() => {
    const ids = new Set(this.ctx.docked());
    for (const i of this.ctx.autoHide().items) ids.add(i.id);
    for (const id of this.ctx.windows()?.poppedIds() ?? []) ids.add(id);
    return ids;
  });
  /** Auto-hidden tool windows and which one is slid out. */
  readonly autoHide = this.autoHider.state;
  /** Tabs pinned to the left of their strip (they survive "Close all"). */
  readonly pinnedTabs = this.ctx.pinned.asReadonly();
  /** The layout as last saved (projects and sessions save it too). */
  readonly savedLayout = this.persistence.savedLayout;
  /** An inline problem to show over the dock. */
  readonly notice = signal<DockNotice | null>(null);
  /** The Ctrl+Tab navigator, when open. */
  readonly switcher = signal<SwitcherState | null>(null);
  readonly fullScreen = signal(false);
  private readonly activeView = signal<string | null>(null);
  /** The view whose tab was brought to the front last (null before any). */
  readonly activeViewId = this.activeView.asReadonly();
  /** Ticks on every layout change of the dock (panels that keep native content in place follow it). */
  readonly layoutChanged = this.ctx.layoutVersion.asReadonly();

  constructor() {
    inject(DockPolicy).start({
      ready: this.ctx.apiReady,
      show: (id) => this.show(id),
      revealInspector: () => this.revealInspector(),
      syncConditionalViews: () => this.syncConditionalViews(),
    });
  }

  /** The dockview instance (null before attach). */
  private get api(): DockviewApi | null {
    return this.ctx.api;
  }

  /** Slides the auto-hidden inspector out (see `DockAutoHide.revealInspector`); nothing in a pop-out. */
  revealInspector(): void {
    if (!this.remote) this.autoHider.revealInspector();
  }

  isOpen(id: string): boolean {
    return this.openIds().has(id);
  }

  /**
   * Is the view in front: the active tab of its group, or the slid-out auto-hidden window? (`isOpen` is also true for
   * a tab behind another, an auto-hidden window that is not slid out, and a pop-out.)
   */
  isVisible(id: string): boolean {
    return this.ctx.shown().has(id) || this.ctx.autoHide().open === id || (this.ctx.windows()?.poppedIds().has(id) ?? false);
  }

  kindOf(id: string): ViewKind {
    return this.ctx.kindOf(id);
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
    this.ctx.container = container;
    const api = lib.createDockview(container, {
      theme: { name: 'debrief', className: 'debrief-dockview' },
      floatingGroupBounds: 'boundedWithinViewport',
      // A floating panel moves by the empty part of its tab strip, as a window by its title bar. dockview's default
      // is a separate blank bar above the tabs, which nothing marks as a handle: the panel looked stuck.
      floatingGroupDragHandle: 'tabbar',
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
          openMenu: (g, anchor) => g.activePanel && this.openPanelMenu(g.activePanel.id, anchor),
          autoHide: {
            available: (g) => !!g.activePanel && this.canAutoHide(g.activePanel.id),
            run: (g) => g.activePanel && this.unpin(g.activePanel.id),
            chooseSide: (g, anchor, at) => g.activePanel && this.openAutoHideSideMenu(g.activePanel.id, anchor, at),
          },
        }),
    });
    this.ctx.api = api;
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
    // A tab dragged outside the window pops out where it is released; released inside but off the drop zones the
    // drag is cancelled. A cue says which while dragging (pointer events: see `dndStrategy`).
    const cue = new FloatCue();
    const floatingMove = new FloatingMove(container);
    // Outside the window the shell's ghost window shows where the pop-out will open (where it has one); a release
    // there opens the window exactly where the ghost was.
    const dragCue = new DragOutCue(cue, {
      cueRect: (id, p, outcome) => this.placement.cueRect(id, p, outcome),
      beginDragOut: (id) => this.ctx.windows()?.beginDragOut(id) ?? null,
    });
    const tabDrag = new TabDragOut(container, {
      outcome: (id, p) => this.tabDropOutcome(id, p),
      popOut: (id, at) => {
        const ending = dragCue.takeEnd();
        if (!ending) void this.popOut(id, { at });
        else void ending.then((rect) => this.popOut(id, rect ? { rect } : { at }));
      },
      cue: (id, p, outcome) => dragCue.update(id, p, outcome),
    });
    container.addEventListener('keydown', onKey);
    container.addEventListener('contextmenu', onContextMenu);
    const guides = new DropGuides(container);
    this.ctx.guides = guides;
    this.disposers = [
      { dispose: () => container.removeEventListener('keydown', onKey) },
      { dispose: () => container.removeEventListener('contextmenu', onContextMenu) },
      tabDrag,
      dragCue,
      floatingMove,
      cue,
      guides,
      observeFloatingAria(container),
      keepGroupsUnscrolled(container),
      labelTabOverflow(container),
      api.onDidAddPanel(() => this.ctx.refreshOpen()),
      api.onDidRemovePanel(() => this.ctx.refreshOpen()),
      api.onDidLayoutFromJSON(() => this.ctx.refreshOpen()),
      api.onDidLayoutChange(() => {
        this.ctx.layoutVersion.update((n) => n + 1);
        this.ctx.markPinnedTabs();
        this.persistence.schedule();
      }),
      api.onDidActivePanelChange((e) => {
        this.ctx.refreshShown();
        this.activeView.set(e.panel?.id ?? null);
        if (e.panel) this.ctx.mru = touchMru(this.ctx.mru, e.panel.id);
      }),
    ];
    this.ctx.restoring = true;
    let extras: DockExtras;
    try {
      const saved = this.persistence.load();
      if (saved) {
        try {
          api.fromJSON(saved.layout);
          extras = saved.extras;
        } catch (e) {
          this.logger.warn('ui.layout', 'Saved layout could not be restored; using the default', { Error: String(e) });
          this.persistence.clear();
          api.clear();
          this.fresh = true;
          extras = this.placement.buildDefault(api);
        }
      } else {
        this.fresh = true;
        extras = this.placement.buildDefault(api);
      }
      this.applyExtras(extras);
    } finally {
      this.ctx.restoring = false;
    }
    this.ctx.refreshOpen();
    this.evaluated = false;
    this.wasAvailable.clear();
    this.ctx.apiReady.set(true);
    if (extras.poppedOut.length) void this.ctx.windows()?.restore(extras.poppedOut);
  }

  detach(): void {
    this.pendingAttach = null;
    this.persistence.cancel();
    this.disposers.forEach((d) => d.dispose());
    this.disposers = [];
    this.ctx.api?.dispose();
    this.ctx.api = null;
    this.ctx.container = null;
    this.ctx.guides = null;
    this.ctx.apiReady.set(false);
    this.ctx.docked.set(new Set());
    this.ctx.autoHide.set(EMPTY_AUTO_HIDE);
  }

  /** Restores the default arrangement (Investigate) and forgets the saved one. */
  reset(): void {
    this.persistence.clear();
    this.rebuild((api) => this.placement.buildDefault(api));
  }

  /** Replaces the arrangement with a built-in perspective's, built from scratch (see `buildPerspective`). */
  buildPerspective(id: BuiltInPerspective): void {
    this.rebuild((api) =>
      buildPerspective(id, { api, buildDefault: () => this.placement.buildDefault(api), find: (v) => this.views.find(v) }),
    );
  }

  /** Clears the dock (and pop-outs) and builds an arrangement into it; it is then persisted as the current one. */
  private rebuild(build: (api: DockviewApi) => DockExtras): void {
    const api = this.api;
    if (!api) return;
    const windows = this.ctx.windows();
    for (const id of [...(windows?.poppedIds() ?? [])]) void windows?.discard(id);
    this.ctx.restoring = true;
    try {
      api.clear();
      this.applyExtras(build(api));
    } finally {
      this.ctx.restoring = false;
    }
    this.exitFullScreen();
    this.fresh = true;
    this.evaluated = false;
    this.wasAvailable.clear();
    this.ctx.refreshOpen();
    this.syncConditionalViews();
    this.persistence.schedule();
  }

  /** Changes the tab title of an open panel (e.g. to show a count); does nothing when it is not open. */
  setTitle(id: string, title: string): void {
    const panel = this.api?.getPanel(id);
    if (panel && panel.title !== title) panel.api.setTitle(title);
    // A tool window collapsed to a side-bar tab keeps counting too.
    if (this.ctx.autoHide().items.some((i) => i.id === id && i.title !== title)) {
      this.ctx.autoHide.update((s) => ({ ...s, items: s.items.map((i) => (i.id === id ? { ...i, title } : i)) }));
    }
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
    const windows = this.ctx.windows();
    if (windows?.poppedIds().has(id)) {
      if (!options.inactive) windows.focus(id);
      return;
    }
    if (this.ctx.isAutoHidden(id)) {
      if (!options.inactive) this.autoHider.event({ type: 'activate', id });
      return;
    }
    // A tool window docks at a window edge, where a slid-out one would cover it.
    const open = this.ctx.autoHide().open;
    if (open && !options.inactive && viewKind(view) === 'tool') this.autoHider.event({ type: 'blur' });
    const existing = api.getPanel(id);
    if (existing) {
      if (api.hasMaximizedGroup() && !existing.group.api.isMaximized()) api.exitMaximizedGroup();
      if (!options.inactive) existing.api.setActive();
      return;
    }
    if (!this.views.available().includes(view)) return;
    this.placement.addView(api, view, options.inactive ?? false, id);
  }

  /**
   * Opens a panel next to another docked one (a new timeline below the main timeline, so their time axes line up).
   * Behaves like `show` when the panel is already open or the reference is not docked.
   */
  showBeside(id: string, referencePanel: string, direction: 'below' | 'right'): void {
    const api = this.api;
    const view = this.views.find(id);
    if (this.remote || !api || !view || api.getPanel(id) || !api.getPanel(referencePanel) || !this.views.available().includes(view)) {
      this.show(id);
      return;
    }
    if (api.hasMaximizedGroup()) api.exitMaximizedGroup();
    api.addPanel({ id, component: view.id, title: view.title, position: { referencePanel, direction } });
  }

  close(id: string): void {
    const windows = this.ctx.windows();
    if (windows?.poppedIds().has(id)) {
      void windows.discard(id);
      return;
    }
    if (this.ctx.isAutoHidden(id)) {
      this.autoHider.remove(id);
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
    this.placement.toggleFloat(group);
  }

  /* ------------------------------------------------------------------ slots */

  /** Where a docked panel sits, so it can be put back after a pop-out or auto-hide. */
  captureSlot(id: string): DockSlot | null {
    return this.placement.captureSlot(id);
  }

  /** Puts a panel back at a slot (or where its view opens by default when the reference is gone). */
  restoreSlot(id: string, slot: DockSlot | null, options: { inactive?: boolean } = {}): void {
    this.placement.restoreSlot(id, slot, options);
  }

  /** The panel's content size on screen (for the pop-out window size). */
  panelRect(id: string): Rect | null {
    return this.placement.panelRect(id);
  }

  /** The panel's content as shown (for a snapshot of it); null when another panel of its group is in front. */
  panelContent(id: string): HTMLElement | null {
    const panel = this.api?.getPanel(id);
    if (!panel || panel.group.activePanel?.id !== id) return null;
    return panel.group.element.querySelector<HTMLElement>('.dv-content-container');
  }

  /** Removes a panel from the layout without closing its view (it moved to a pop-out window). */
  takeOut(id: string): void {
    const panel = this.api?.getPanel(id);
    if (panel) this.api?.removePanel(panel);
    this.ctx.refreshOpen();
    this.persistence.schedule();
  }

  /* --------------------------------------------------------------- pop-outs */

  /** Pops a panel out into a real window ("Pop out to window"), exactly at `rect` or under the screen point `at` (a dragged tab). */
  async popOut(id: string, where: { at?: Point; rect?: Rect } = {}): Promise<boolean> {
    const windows = this.ctx.windows();
    if (!windows || !id || !this.api?.getPanel(id)) return false;
    return windows.popOut(id, where.rect ? { rect: where.rect } : where.at ? { at: where.at } : {});
  }

  /** Pops out the active panel (keyboard command). */
  popOutActive(): Promise<boolean> {
    return this.popOut(this.api?.activePanel?.id ?? '');
  }

  /** Can the active panel float inside the window (it is docked)? */
  canFloatActive(): boolean {
    return this.api?.activePanel?.group.api.location.type === 'grid';
  }

  /** Floats the active panel inside the window (Window ▸ Float panel): the deliberate way to float, never a drag. */
  floatActive(): boolean {
    const panel = this.api?.activePanel;
    if (!panel || !this.canFloatActive()) return false;
    this.placement.floatPanel(panel);
    return true;
  }

  /* ------------------------------------------- tabs dragged out, drops from outside */

  /** What releasing a dragged tab at a client point does: dockview docks it on a drop zone, outside it pops out, else nothing. */
  tabDropOutcome(id: string, p: Point): TabDropOutcome {
    return this.placement.tabDropOutcome(id, p);
  }

  /** The guide under a point (client coordinates): where a pop-out dropped there would dock. */
  dropTargetAt(point: Point): DockTarget | null {
    return this.placement.dropTargetAt(point);
  }

  /** Shows the dock guides for a pop-out window dragged over this one (null hides them). */
  showDropGuides(point: Point | null): void {
    this.placement.showDropGuides(point);
  }

  /** Docks a panel that is not in the layout (a pop-out dropped on a guide) at that guide. */
  dockAt(id: string, target: DockTarget): boolean {
    return this.placement.dockAt(id, target);
  }

  /* -------------------------------------------------------------- auto-hide */

  /** Collapses a docked tool window into a tab on the nearest side bar ("Auto hide"). */
  unpin(id: string, side?: Side): void {
    this.autoHider.unpin(id, side);
  }

  /** Is this panel a docked tool window the auto-hide button applies to (not a document, not floating)? */
  canAutoHide(id: string): boolean {
    return this.autoHider.canAutoHide(id);
  }

  /** The left / right / bottom choice for auto-hiding a tool window (never the top). */
  openAutoHideSideMenu(id: string, anchor: HTMLElement | null, at?: Point): void {
    const box = anchor?.getBoundingClientRect();
    const items: { side: Side; label: string }[] = [
      { side: 'left', label: $localize`:@@dock.autoHide.toLeft:Auto-hide to the left` },
      { side: 'right', label: $localize`:@@dock.autoHide.toRight:Auto-hide to the right` },
      { side: 'bottom', label: $localize`:@@dock.autoHide.toBottom:Auto-hide to the bottom` },
    ];
    this.menus.open(
      at?.x ?? box?.left ?? 100,
      at?.y ?? box?.bottom ?? 100,
      items.map((i) => ({ label: i.label, run: () => this.unpin(id, i.side) })),
      anchor,
    );
  }

  /** Docks an auto-hidden tool window back where it was ("Pin"). */
  pin(id: string): void {
    this.autoHider.pin(id);
  }

  autoHideEvent(event: AutoHideEvent): void {
    this.autoHider.event(event);
  }

  /** Toggles auto-hide of the active tool window (or of the slid-out one). */
  toggleAutoHideActive(): void {
    this.autoHider.toggleActive();
  }

  /** Auto-hides every docked tool window and returns their ids. */
  autoHideAll(): string[] {
    return this.autoHider.unpinAll();
  }

  /** Pins these tool windows back if they are still auto-hidden. */
  pinIds(ids: readonly string[]): void {
    this.autoHider.pinIds(ids);
  }

  pinAll(): void {
    this.autoHider.pinAll();
  }

  /* ------------------------------------------------------------ pinned tabs */

  setPinned(id: string, pinned: boolean): void {
    const was = this.ctx.pinned().has(id);
    if (was === pinned) return;
    this.ctx.pinned.update((s) => {
      const next = new Set(s);
      if (pinned) next.add(id);
      else next.delete(id);
      return next;
    });
    const panel = this.api?.getPanel(id);
    if (pinned && panel) {
      const order = panel.group.panels.map((p) => p.id);
      panel.api.moveTo({ group: panel.group, position: 'center', index: pinnedIndex(order, id, this.ctx.pinned()) });
    }
    this.ctx.markPinnedTabs();
    this.persistence.schedule();
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
      pinnedTab: this.ctx.pinned().has(id),
      canPopout: this.ctx.windows()?.canPopOut() ?? false,
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
    const tab = this.ctx.container?.querySelector<HTMLElement>(`.debrief-tab[data-panel-id="${CSS.escape(panel.id)}"]`) ?? null;
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
        this.placement.floatPanel(panel);
        break;
      case 'dock':
        if (group.api.location.type === 'floating') this.toggleFloat(group);
        break;
      case 'popout':
        void this.popOut(id);
        break;
      case 'dockAsDocument': {
        const well = this.placement.documentWell(id);
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
        closeAll(panelsToClose(order, id, 'others', this.ctx.pinned()));
        break;
      case 'closeRight':
        closeAll(panelsToClose(order, id, 'right', this.ctx.pinned()));
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
    for (const id of panelsToClose(ids, '', 'all', this.ctx.pinned())) this.close(id);
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
    const mru = active ? touchMru(this.ctx.mru, active) : this.ctx.mru;
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
    queueMicrotask(() => this.ctx.container?.querySelector<HTMLElement>(`.debrief-tab[data-panel-id="${CSS.escape(id)}"]`)?.closest<HTMLElement>('[role="tab"]')?.focus());
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
      const group = this.placement.documentWell();
      document.documentElement.classList.add('debrief-full-screen');
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
    document.documentElement.classList.remove('debrief-full-screen');
    if (this.fullScreenGroup?.api.isMaximized()) this.fullScreenGroup.api.exitMaximized();
    this.fullScreenGroup = null;
    if (this.fullScreenOs) void this.lazy.get(OsWindow).setFullscreen(false);
    this.fullScreenOs = false;
  }

  /* --------------------------------------------------------------- layouts */

  /** The whole arrangement (grid, auto-hidden tool windows, pinned tabs, pop-outs), or null before attach. */
  currentLayout(): SavedLayout | null {
    return this.persistence.current();
  }

  /** Applies a saved arrangement (named layout, project, session). Falls back to the default when it is broken. */
  applyLayout(saved: SavedLayout): boolean {
    const api = this.api;
    if (!api) return false;
    const windows = this.ctx.windows();
    for (const id of [...(windows?.poppedIds() ?? [])]) if (!saved.extras.poppedOut.some((p) => p.id === id)) void windows?.discard(id);
    this.ctx.restoring = true;
    let ok = true;
    try {
      try {
        api.fromJSON(saved.layout);
        this.applyExtras(saved.extras);
      } catch (e) {
        ok = false;
        this.logger.warn('ui.layout', 'Window layout could not be applied; using the default', { Error: String(e) });
        api.clear();
        this.applyExtras(this.placement.buildDefault(api));
      }
    } finally {
      this.ctx.restoring = false;
    }
    this.exitFullScreen();
    this.ctx.refreshOpen();
    this.syncConditionalViews();
    this.persistence.schedule();
    const popped = windows?.poppedIds() ?? new Set<string>();
    if (ok) void windows?.restore(saved.extras.poppedOut.filter((p) => !popped.has(p.id)));
    return ok;
  }

  private applyExtras(extras: DockExtras): void {
    const docked = new Set((this.api?.panels ?? []).map((p) => p.id));
    const known = (id: string) => !!this.views.find(id);
    this.ctx.autoHide.set({ items: extras.autoHidden.filter((i) => known(i.id) && !docked.has(i.id)), open: null, openedBy: null });
    this.ctx.autoHideSides.set({ ...(extras.autoHideSides ?? {}) });
    this.ctx.pinned.set(new Set(extras.pinnedTabs.filter((id) => docked.has(id))));
    this.ctx.markPinnedTabs();
  }

  /** Persists the layout soon (pop-outs moved, auto-hide resized). */
  persistLater(): void {
    this.persistence.schedule();
  }

  /** Opens/closes views that depend on the workspace (`DebriefView.when`). */
  syncConditionalViews(): void {
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
      const autoOpen = view.autoOpen === true;
      if (autoOpen && becameAvailable && !open && (this.evaluated || this.fresh)) this.placement.addView(api, view, true);
    }
    this.evaluated = true;
  }
}
