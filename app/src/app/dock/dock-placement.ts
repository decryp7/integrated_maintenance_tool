import { inject, Injectable } from '@angular/core';
import type { DockviewApi, DockviewGroupPanel, IDockviewPanel } from 'dockview';
import { ImtView, ViewPosition } from '../extensions/tokens';
import { DockContext, PANEL, rectOf } from './dock-context';
import { adjacentNeighbour, atEdge, DockSlot, fullSpanEdge, Point, Rect } from './dock-geometry';
import { DockTarget, GroupBox, hitDockTarget, outsideViewport, targetPreview } from './dock-target';
import { cancelHintRect, FLOAT_MAX, ghostRect, previewSize, TabDropOutcome, tabDropOutcome } from './float-cue';
import { AutoHideItem, autoHideReduce } from './auto-hide';
import { DockExtras, EMPTY_EXTRAS } from './layout-store';
import { analysisTabToClose, viewArea } from './view-area';

type AddPanel = { id: string; component: string; title: string; inactive: boolean };

const INSPECTOR_WIDTH = 372;
const TIMELINE_HEIGHT = 300;
/** The share of the dock's height the events grid keeps when a document opens beside the timeline. */
const DOCUMENT_GRID_SHARE = 0.35;
/** Size of a group floated with the "Float panel" button. */
const FLOAT_SIZE = FLOAT_MAX;
/** Size of a new tool window group at a window edge, px (a right-hand column as wide as the inspector's). */
const TOOL_SIZE = { side: INSPECTOR_WIDTH, bottom: 240 };

/** Height of a new analysis panel below the events grid, px. */
const ANALYSIS_HEIGHT = 280;
/** Height of a status tool window sliding out of the bottom edge, px. */
const STATUS_HEIGHT = 260;

/** The two panels a new view never covers: it opens beside them (documents) or below them (analysis). */
const MAIN_PANELS: ReadonlySet<string> = new Set([PANEL.timeline, PANEL.events]);

/** Panels a new view with a given default position is placed next to (first one that exists wins). */
const ANCHORS: Record<ViewPosition, string[]> = {
  center: [PANEL.timeline, PANEL.metrics, PANEL.profile],
  right: [PANEL.inspector],
  bottom: [PANEL.events],
};

/**
 * The built-in core panels' fixed places, used when one is (re)opened while the other is docked: the events grid
 * right below the timeline, the timeline right above the grid.
 */
const FIXED: Readonly<Record<string, { referencePanel: string; direction: 'above' | 'below' }>> = {
  [PANEL.events]: { referencePanel: PANEL.timeline, direction: 'below' },
  [PANEL.timeline]: { referencePanel: PANEL.events, direction: 'above' },
};

/**
 * The inspector, auto-hidden on the right edge. Pinning it docks it as a full-height tool window on the right
 * (no reference panel: it opens where a right-hand tool window opens by default).
 */
export function defaultAutoHidden(inspector: ImtView | undefined): AutoHideItem[] {
  if (!inspector) return [];
  return [{ id: inspector.id, title: inspector.title, side: 'right', size: INSPECTOR_WIDTH, slot: { referencePanel: null, direction: 'within' } }];
}

/**
 * Where panels go in the window: the default layout, where a newly opened view lands (pinned places, tool windows
 * at an edge, documents with the timeline), slots to put a panel back, floating, and docking at the guides (drags
 * from tabs and pop-out windows).
 */
@Injectable({ providedIn: 'root' })
export class DockPlacement {
  private readonly ctx = inject(DockContext);

  /**
   * The minimal default (the Investigate perspective): the timeline over the events; the inspector is an auto-hidden
   * tool window on the right edge that slides out when an event is selected (pin it to keep it docked). Everything
   * else (metrics, profile editor, analysis, tools, ...) opens on demand in its home area. Returns the extras to apply.
   */
  buildDefault(api: DockviewApi): DockExtras {
    const views = this.ctx.views;
    const add = (id: string, position?: Parameters<DockviewApi['addPanel']>[0]['position']) => {
      const view = views.find(id);
      if (view) api.addPanel({ id, component: id, title: view.title, ...(position ? { position } : {}) });
    };
    add(PANEL.timeline);
    add(PANEL.events, { referencePanel: PANEL.timeline, direction: 'below' });
    api.getPanel(PANEL.timeline)?.api.setActive();
    this.whenSized(api, () => api.getPanel(PANEL.timeline)?.group.api.setSize({ height: TIMELINE_HEIGHT }));
    return { ...EMPTY_EXTRAS, autoHidden: defaultAutoHidden(views.find(PANEL.inspector)) };
  }

  /**
   * Runs `apply` once the dock has a measured size. A size set before the container is measured is rescaled
   * proportionally when dockview first lays out, so the split landed on a different height from run to run; applied
   * after that layout it is exactly the requested one.
   */
  private whenSized(api: DockviewApi, apply: () => void): void {
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
   * Adds a panel for the view in its home area (`viewArea`); `id` differs from the view id for one more panel of a
   * view that allows several. Documents open beside the timeline and the events grid (never over them), analysis
   * views in the one analysis panel below the grid (at most `ANALYSIS_TAB_CAP` tabs), side tools in the right-hand
   * tool group, and status tools slide out of the bottom edge (auto-hidden) unless one is docked there. Returns the
   * docked panel, or null when the view went to the bottom side bar.
   */
  addView(api: DockviewApi, view: ImtView, inactive: boolean, id = view.id): IDockviewPanel | null {
    const panel = this.place(api, view, inactive, id);
    // The view's minimum size (A4): the dock refuses a split that would make the group smaller.
    const min = view.minSize;
    if (panel && min) panel.group.api.setConstraints({ ...(min.width ? { minimumWidth: min.width } : {}), ...(min.height ? { minimumHeight: min.height } : {}) });
    return panel;
  }

  private place(api: DockviewApi, view: ImtView, inactive: boolean, id: string): IDockviewPanel | null {
    const base: AddPanel = { id, component: view.id, title: view.title, inactive };
    const fixed = FIXED[view.id];
    if (fixed && api.getPanel(fixed.referencePanel)) return api.addPanel({ ...base, position: fixed });
    switch (viewArea(view)) {
      case 'side':
        return this.addTool(api, base);
      case 'status': {
        const docked = this.toolGroupFor(api, 'bottom');
        if (docked) return api.addPanel({ ...base, position: { referencePanel: docked, direction: 'within' } });
        this.slideOut(id, view.title, inactive);
        return null;
      }
      case 'analysis':
        return this.addAnalysis(api, base);
      case 'document':
        return this.addDocument(api, base);
    }
  }

  /** A side tool window in the tool group at the right edge, else a new group there. */
  private addTool(api: DockviewApi, base: AddPanel): IDockviewPanel {
    const sameKind = this.toolGroupFor(api, 'right');
    if (sameKind) return api.addPanel({ ...base, position: { referencePanel: sameKind, direction: 'within' } });
    const panel = api.addPanel({ ...base, position: { direction: 'right' } });
    panel.group.api.setSize({ width: TOOL_SIZE.side });
    return panel;
  }

  /** A status tool window as a tab on the bottom side bar, slid out unless opened in the background. */
  private slideOut(id: string, title: string, inactive: boolean): void {
    const anchor = this.ctx.api?.getPanel(PANEL.events) ? PANEL.events : null;
    const item: AutoHideItem = { id, title, side: 'bottom', size: STATUS_HEIGHT, slot: { referencePanel: anchor, direction: anchor ? 'below' : 'within' } };
    this.ctx.autoHide.update((s) => {
      const hidden = autoHideReduce(s, { type: 'unpin', item });
      return inactive ? hidden : autoHideReduce(hidden, { type: 'activate', id });
    });
    this.ctx.refreshOpen();
  }

  /** The analysis panel: tabbed into it (closing its oldest tab when full), else a new group below the events grid. */
  private addAnalysis(api: DockviewApi, base: AddPanel): IDockviewPanel {
    const group = this.analysisGroup(api);
    if (group) {
      const victim = analysisTabToClose(
        group.panels.map((p) => p.id),
        group.activePanel?.id ?? null,
        this.ctx.pinned(),
        this.ctx.mru,
      );
      const old = victim ? api.getPanel(victim) : undefined;
      if (old) api.removePanel(old);
      return api.addPanel({ ...base, position: { referenceGroup: group.id, direction: 'within' } });
    }
    const below = api.getPanel(PANEL.events) ?? this.documentWell()?.activePanel;
    if (!below) return api.addPanel(base);
    const panel = api.addPanel({ ...base, position: { referenceGroup: below.group.id, direction: 'below' } });
    panel.group.api.setSize({ height: ANALYSIS_HEIGHT });
    return panel;
  }

  /** The docked group holding analysis views (the most recently used one when the user made several). */
  analysisGroup(api: DockviewApi): DockviewGroupPanel | null {
    const isAnalysis = (g: DockviewGroupPanel) => g.api.location.type === 'grid' && g.panels.some((p) => viewArea(this.ctx.views.find(p.id)) === 'analysis');
    for (const id of this.ctx.mru) {
      const g = api.getPanel(id)?.group;
      if (g && isAnalysis(g)) return g;
    }
    return api.groups.find(isAnalysis) ?? null;
  }

  /**
   * A document beside the timeline and the grid: tabbed into the most recently used document group that holds
   * neither, else a new group to the right of the timeline (or the grid).
   */
  private addDocument(api: DockviewApi, base: AddPanel): IDockviewPanel {
    const well = this.documentWell(undefined, true);
    if (well) return api.addPanel({ ...base, position: { referenceGroup: well.id, direction: 'within' } });
    const main = api.getPanel(PANEL.timeline) ?? api.getPanel(PANEL.events);
    if (main && main.group.api.location.type === 'grid') {
      const panel = api.addPanel({ ...base, position: { referenceGroup: main.group.id, direction: 'right' } });
      this.giveDocumentsRoom(api);
      return panel;
    }
    const ref = this.anchorFor(api, 'center');
    return api.addPanel({ ...base, ...(ref ? { position: { referencePanel: ref, direction: 'within' as const } } : {}) });
  }

  /**
   * A document opened beside the timeline shares the timeline's row, which is a strip (300 px) next to the grid. Dumps,
   * registries and reports need height, so the row grows to the share of the dock the grid gives up (the grid keeps
   * its share when it is already smaller). Only the grid's height is set: dockview resizes along the split's own axis.
   */
  private giveDocumentsRoom(api: DockviewApi): void {
    const events = api.getPanel(PANEL.events)?.group;
    if (!events || events.api.location.type !== 'grid' || api.height <= 0) return;
    const wanted = Math.round(api.height * DOCUMENT_GRID_SHARE);
    if (events.height > wanted) events.api.setSize({ height: wanted });
  }

  /** A docked tool window already open on that side, to tab a new one with. */
  toolGroupFor(api: DockviewApi, position: ViewPosition): string | null {
    const root = this.ctx.rootRect();
    const want = position === 'bottom' ? 'bottom' : 'right';
    for (const p of api.panels) {
      if (this.ctx.kindOf(p.id) !== 'tool' || p.group.api.location.type !== 'grid') continue;
      const rect = rectOf(p.group.element);
      // Without a layout (nothing measured yet) any docked tool window counts.
      if (!root || !rect || !root.width || atEdge(rect, root, want)) return p.id;
    }
    return null;
  }

  private anchorFor(api: DockviewApi, position: ViewPosition, exclude: ReadonlySet<string> = new Set()): string | null {
    const id = ANCHORS[position].find((a) => !exclude.has(a) && api.getPanel(a));
    return id ?? api.panels.find((p) => !exclude.has(p.id))?.id ?? null;
  }

  /**
   * The group documents go to: that of the most recently used document, else the one holding the timeline. With
   * `besideMain`, only a group holding neither the timeline nor the events grid (null when there is none).
   */
  documentWell(excluding?: string, besideMain = false): DockviewGroupPanel | null {
    const api = this.ctx.api;
    if (!api) return null;
    const isDocument = (id: string) => this.ctx.kindOf(id) === 'document' && viewArea(this.ctx.views.find(id)) === 'document';
    const isWell = (g: DockviewGroupPanel) =>
      g.api.location.type === 'grid' && g.panels.some((p) => p.id !== excluding && isDocument(p.id)) && (!besideMain || !g.panels.some((p) => MAIN_PANELS.has(p.id)));
    for (const id of this.ctx.mru) {
      const g = api.getPanel(id)?.group;
      if (g && id !== excluding && isDocument(id) && isWell(g)) return g;
    }
    if (besideMain) return api.groups.find(isWell) ?? null;
    return api.getPanel(PANEL.timeline)?.group ?? api.groups.find(isWell) ?? null;
  }

  /* ------------------------------------------------------------------ slots */

  /** Where a docked panel sits, so it can be put back after a pop-out or auto-hide. */
  captureSlot(id: string): DockSlot | null {
    const api = this.ctx.api;
    const panel = api?.getPanel(id);
    if (!api || !panel) return null;
    const group = panel.group;
    const siblings = group.panels.filter((p) => p.id !== id);
    const index = group.panels.findIndex((p) => p.id === id);
    const reference = siblings[Math.max(0, index - 1)] ?? siblings[0];
    if (reference) {
      return { referencePanel: reference.id, direction: 'within', index };
    }
    const rect = rectOf(group.element);
    const root = this.ctx.rootRect();
    const edge = rect && root ? fullSpanEdge(rect, root) : null;
    if (edge) {
      const size = edge === 'bottom' ? rect?.height : rect?.width;
      return { referencePanel: null, direction: 'within', edge, ...(size ? { size: Math.round(size) } : {}) };
    }
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
    const api = this.ctx.api;
    const view = this.ctx.views.find(id);
    if (!api || !view || api.getPanel(id)) return;
    if (slot?.edge) {
      // A full-span column or strip goes back to its window edge (a split against one neighbour would only span that neighbour).
      const panel = api.addPanel({ id, component: view.id, title: view.title, inactive: options.inactive ?? false, position: { direction: slot.edge === 'bottom' ? 'below' : slot.edge } });
      if (slot.size) panel.group.api.setSize(slot.edge === 'bottom' ? { height: slot.size } : { width: slot.size });
      return;
    }
    const ref = slot?.referencePanel ? api.getPanel(slot.referencePanel) : undefined;
    if (!slot || !ref) {
      this.addView(api, view, options.inactive ?? false, id);
      return;
    }
    const panel = api.addPanel({
      id,
      component: view.id,
      title: view.title,
      inactive: options.inactive ?? false,
      position: { referencePanel: ref.id, direction: slot.direction, ...(slot.direction === 'within' && slot.index !== undefined ? { index: slot.index } : {}) },
    });
    if (slot.size && slot.direction !== 'within') panel.group.api.setSize(slot.direction === 'left' || slot.direction === 'right' ? { width: slot.size } : { height: slot.size });
  }

  /** The panel's content size on screen (for the pop-out window size). */
  panelRect(id: string): Rect | null {
    const panel = this.ctx.api?.getPanel(id);
    return panel ? rectOf(panel.group.element) : null;
  }

  /* --------------------------------------------------------------- floating */

  /** Floats a docked group in the window, or docks a floating group back into the grid. */
  toggleFloat(group: DockviewGroupPanel): void {
    const api = this.ctx.api;
    if (!api) return;
    if (group.api.location.type === 'floating') {
      // Dock next to where the view would open by default: tabbed with the timeline, below it, or to its right.
      const position = this.ctx.views.find(group.panels[0]?.id ?? '')?.defaultPosition ?? 'center';
      const exclude = new Set(group.panels.map((p) => p.id));
      const anchor = api.getPanel(this.anchorFor(api, position, exclude) ?? '');
      if (anchor) group.api.moveTo({ group: anchor.group, position: position === 'center' ? 'center' : position });
      else group.api.moveTo({ position: 'right' });
    } else {
      api.addFloatingGroup(group, { ...FLOAT_SIZE, x: 140, y: 100 });
    }
  }

  /** Floats a panel ("Float" in the panel menu): the panel alone when its group has others, else the group. */
  floatPanel(panel: IDockviewPanel): void {
    const api = this.ctx.api;
    const group = panel.group;
    if (api && group.api.location.type !== 'floating') api.addFloatingGroup(group.panels.length > 1 ? panel : group, { ...FLOAT_SIZE, x: 140, y: 100 });
  }

  /* ------------------------------------------- a tab dragged off the drop zones */

  /** What releasing a dragged tab at a client point does: dockview docks it on a drop zone, outside it pops out, else nothing. */
  tabDropOutcome(id: string, p: Point): TabDropOutcome {
    const container = this.ctx.container;
    const root = this.ctx.rootRect();
    const under = document.elementFromPoint?.(p.x, p.y) ?? null;
    const onStrip = !!under?.closest('.dv-tabs-and-actions-container') && !!container?.contains(under);
    const onGuide = !!root && !!hitDockTarget(p, root, this.groupBoxes(true));
    return tabDropOutcome({
      inside: !outsideViewport(p, { width: window.innerWidth, height: window.innerHeight }),
      overDropZone: onStrip || onGuide,
      canPopOut: this.ctx.windows()?.canPopOut() ?? false,
    });
  }

  /**
   * Where the drag cue is drawn: the panel's preview at (about) its size at the pointer, held inside the window (a new
   * window), or the anchor of the "Release to cancel" hint; null for no cue.
   */
  cueRect(id: string, p: Point, outcome: TabDropOutcome | null): Rect | null {
    if (outcome === 'window') {
      const viewport = { width: window.innerWidth, height: window.innerHeight };
      return ghostRect(p, previewSize(this.panelRect(id), viewport), { x: 0, y: 0, ...viewport });
    }
    if (outcome === 'cancel') return cancelHintRect(p);
    return null;
  }

  /* ------------------------------------------- drops from outside dockview */

  /** The guide under a point (client coordinates): where a pop-out dropped there would dock. */
  dropTargetAt(point: Point): DockTarget | null {
    const root = this.ctx.rootRect();
    return root ? hitDockTarget(point, root, this.groupBoxes()) : null;
  }

  /** Shows the dock guides for a pop-out window dragged over this one (null hides them). */
  showDropGuides(point: Point | null): void {
    if (!point) {
      this.ctx.guides?.hide();
      return;
    }
    const target = this.dropTargetAt(point);
    const root = this.ctx.rootRect();
    this.ctx.guides?.showAt(point, target && root ? targetPreview(target, root, this.groupBoxes()) : null);
  }

  /** Docks a panel that is not in the layout (a pop-out dropped on a guide) at that guide. */
  dockAt(id: string, target: DockTarget): boolean {
    const api = this.ctx.api;
    const view = this.ctx.views.find(id);
    if (!api || !view || api.getPanel(id)) return false;
    const direction = ({ center: 'within', left: 'left', right: 'right', top: 'above', bottom: 'below' } as const)[target.position];
    const group = target.kind === 'group' ? api.getGroup(target.groupId) : undefined;
    if (target.kind === 'group' && !group) return false;
    const panel = api.addPanel({ id, component: view.id, title: view.title, position: group ? { referenceGroup: group.id, direction } : { direction: direction === 'within' ? 'right' : direction } });
    panel.api.setActive();
    return true;
  }

  /** The groups' content boxes: docked ones, and with `floating` the floating ones first (they are on top). */
  private groupBoxes(floating = false): GroupBox[] {
    const groups = this.ctx.api?.groups ?? [];
    const on = (type: string) => groups.filter((g) => g.api.location.type === type);
    return [...(floating ? on('floating') : []), ...on('grid')]
      .map((g) => ({ id: g.id, rect: rectOf(g.element.querySelector('.dv-content-container')) }))
      .filter((g): g is GroupBox => !!g.rect);
  }
}
