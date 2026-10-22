import { inject, Injectable } from '@angular/core';
import type { DockviewApi, DockviewGroupPanel, IDockviewPanel } from 'dockview';
import { ImtView, ViewPosition } from '../extensions/tokens';
import { DockContext, PANEL, rectOf } from './dock-context';
import { adjacentNeighbour, atEdge, DockSlot, Point, Rect } from './dock-geometry';
import { DockTarget, GroupBox, hitDockTarget, outsideViewport, targetPreview } from './dock-target';
import { cancelHintRect, FLOAT_MAX, ghostRect, previewSize, TabDropOutcome, tabDropOutcome } from './float-cue';
import { AutoHideItem } from './auto-hide';
import { DockExtras, EMPTY_EXTRAS } from './layout-store';
import { viewKind } from './view-kind';
import { LayoutModeStore } from '../core/workspace/layout-mode';

const INSPECTOR_WIDTH = 372;
const TIMELINE_HEIGHT = 300;
/** Size of a group floated with the "Float panel" button. */
const FLOAT_SIZE = FLOAT_MAX;
/** Size of a new tool window group at a window edge, px (a right-hand column as wide as the inspector's). */
const TOOL_SIZE = { side: INSPECTOR_WIDTH, bottom: 240 };

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
  private readonly layout = inject(LayoutModeStore);

  /**
   * The minimal default: the timeline over the events (in the Full layout with the query bar pinned above them); the
   * inspector is an auto-hidden tool window on the right edge that slides out when an event is selected (pin it to keep it docked).
   * Everything else (metrics, profile editor, analysis, tools, ...) opens on demand. Returns the extras to apply.
   */
  buildDefault(api: DockviewApi): DockExtras {
    const views = this.ctx.views;
    const add = (id: string, position?: Parameters<DockviewApi['addPanel']>[0]['position']) => {
      const view = views.find(id);
      if (view) api.addPanel({ id, component: id, title: view.title, ...(position ? { position } : {}) });
    };
    add(PANEL.timeline);
    add(PANEL.events, { referencePanel: PANEL.timeline, direction: 'below' });
    const query = this.layout.full() ? views.find(QUERY_VIEW) : undefined;
    if (query) this.addView(api, query, false);
    api.getPanel(PANEL.timeline)?.api.setActive();
    api.getPanel(PANEL.timeline)?.group.api.setSize({ height: TIMELINE_HEIGHT });
    api.getPanel(QUERY_VIEW)?.group.api.setSize({ height: QUERY_HEIGHT });
    return { ...EMPTY_EXTRAS, autoHidden: defaultAutoHidden(views.find(PANEL.inspector)) };
  }

  /** Adds a panel for the view; `id` differs from the view id for one more panel of a view that allows several. */
  addView(api: DockviewApi, view: ImtView, inactive: boolean, id = view.id): IDockviewPanel {
    const pin = PINNED[view.id];
    if (pin && api.getPanel(pin.referencePanel)) {
      // dockview's default minimum group height (100px) is taller than a query bar; the panel's own
      // minimum (saved with the layout) lets its group be that short.
      const size = pin.height ? { initialHeight: pin.height, minimumHeight: pin.height } : {};
      const panel = api.addPanel({ id, component: view.id, title: view.title, inactive, ...size, position: { referencePanel: pin.referencePanel, direction: pin.direction } });
      if (pin.height) panel.group.api.setSize({ height: pin.height });
      return panel;
    }
    // A tool window without a place opens in a tool group at its window edge (VS: right, or bottom).
    if (viewKind(view) === 'tool') {
      const sameKind = this.toolGroupFor(api, view.defaultPosition ?? 'right');
      if (sameKind) return api.addPanel({ id, component: view.id, title: view.title, inactive, position: { referencePanel: sameKind, direction: 'within' } });
      if (view.defaultPosition === 'bottom' || !api.getPanel(PANEL.inspector)) {
        const bottom = view.defaultPosition === 'bottom';
        const panel = api.addPanel({ id, component: view.id, title: view.title, inactive, position: { direction: bottom ? 'below' : 'right' } });
        panel.group.api.setSize(bottom ? { height: TOOL_SIZE.bottom } : { width: TOOL_SIZE.side });
        return panel;
      }
    }
    const ref = this.anchorFor(api, view.defaultPosition ?? 'center');
    return api.addPanel({
      id,
      component: view.id,
      title: view.title,
      inactive,
      ...(ref ? { position: { referencePanel: ref, direction: 'within' as const } } : {}),
    });
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

  /** The group documents go to: that of the most recently used document, else the one holding the timeline. */
  documentWell(excluding?: string): DockviewGroupPanel | null {
    const api = this.ctx.api;
    if (!api) return null;
    const isDocument = (id: string) => this.ctx.kindOf(id) === 'document';
    const isWell = (g: DockviewGroupPanel) => g.api.location.type === 'grid' && g.panels.some((p) => p.id !== excluding && isDocument(p.id));
    for (const id of this.ctx.mru) {
      const g = api.getPanel(id)?.group;
      if (g && id !== excluding && isDocument(id) && isWell(g)) return g;
    }
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
