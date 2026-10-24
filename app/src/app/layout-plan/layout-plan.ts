/**
 * Layout by declared constraints (Debrief ADR 0056, Paddock ADR 0011).
 *
 * SELF-CONTAINED: this folder has no Debrief imports (no Angular, no dockview, no DOM, no `window`), so it moves to
 * `@decryp7/paddock-ui` unchanged. Debrief's dock adapter (`dock/dock-layout.ts`) translates a `LayoutPlan` into dockview calls.
 *
 * Every view declares a `ViewLayout` (area, min, pref, priority). `planLayout` decides, with all open views in
 * view, which view gets which size, and when there is no room it tabs or auto-hides the lowest-priority view
 * instead of squeezing anything below its minimum.
 */

export type LayoutArea = 'document' | 'side' | 'analysis' | 'status';

/** Where a side view docks; `left` is a column at the left edge (the Explorer), default `right`. */
export type SideEdge = 'left' | 'right';

export interface Size {
  width: number;
  height: number;
}

/** What a view declares about itself (one declaration per view). */
export interface ViewLayout {
  area: LayoutArea;
  /** Never go below, px. */
  min: Size;
  /** The opening size when there is room, px. */
  pref: Size;
  /** Higher stays visible; lower is tabbed or auto-hidden first. */
  priority: number;
  /** Side views only: the edge column the view lives in. Default `right`. */
  edge?: SideEdge;
}

/** A view that is open (or about to be). */
export interface OpenView {
  id: string;
  layout: ViewLayout;
  /** A document that spans the full height in a column of its own (Home, Registry). */
  fullHeight?: boolean;
  /** Views of the main column that share a row (tabs of one group) give the same key; default: the view's own id. */
  row?: string;
  /** Views in the same dock group (tabs of one group) give the same key; groups of one row sit side by side. Default: the view's own id. */
  group?: string;
}

/** Sizes the user or a saved layout asked for; they replace `pref` (still clamped to `min`). */
export interface SavedHints {
  sizes?: Readonly<Record<string, Partial<Size>>>;
}

export type ColumnKind = 'left' | 'main' | 'full' | 'side';
export type Edge = 'left' | 'right' | 'bottom';

export interface PlannedCell {
  views: string[];
  width: number;
  minWidth: number;
  /** The cell that takes the leftover width of its row. */
  anchor: boolean;
}

export interface PlannedRow {
  key: string;
  views: string[];
  /** The groups of the row, left to right by priority: side by side, each a stack of tabs. */
  cells: PlannedCell[];
  height: number;
  minHeight: number;
  /** The row that takes the leftover height (its size is not set; the container gives it the rest). */
  anchor: boolean;
}

export interface PlannedColumn {
  kind: ColumnKind;
  /** Views sharing the column's group, highest priority first. */
  views: string[];
  width: number;
  minWidth: number;
  /** The column that takes the leftover width (its size is not set; the container gives it the rest). */
  anchor: boolean;
  /** Only the main column: its rows from top to bottom. */
  rows: PlannedRow[];
}

export interface LayoutPlan {
  viewport: Size;
  /** Left to right: main, full-height documents, side tools. */
  columns: PlannedColumn[];
  /** Views that do not fit as their own column or row and are tabs of another view's group. */
  tabbed: { id: string; into: string }[];
  /** Tools that slid to an edge (status tools always; side tools when there is no room). */
  hidden: { id: string; edge: Edge }[];
  /** Ids whose size is below their `pref` because of the room (not tabbed or hidden). */
  shrunk: string[];
  /** True when even the highest-priority column alone does not fit the viewport (nothing more can be done). */
  overflow: boolean;
  /** Broken invariants (empty unless `overflow`): a placed view under its minimum, overlapping columns. */
  violations: string[];
}

interface Item {
  key: string;
  min: number;
  pref: number;
  priority: number;
}

interface Fit {
  sizes: Map<string, number>;
  dropped: string[];
  shrunk: Set<string>;
  overflow: boolean;
  /** The highest-priority item left: it takes the leftover room and is never dropped. */
  anchor: string | null;
}

const byPriorityThenId = (a: OpenView, b: OpenView): number => b.layout.priority - a.layout.priority || (a.id < b.id ? -1 : a.id > b.id ? 1 : 0);

/**
 * Fits items along one axis. Starts at each `pref`; when over `total` it takes room from the lowest priority first (down to
 * `min`, equal priorities proportionally); when that is not enough it drops the lowest-priority item that is not the anchor
 * and tries again. The anchor (highest priority, first on ties) is elastic: it counts at its `min` and takes all leftover room, and is never dropped.
 */
export function fitAxis(items: readonly Item[], total: number): Fit {
  const dropped: string[] = [];
  let live = [...items];
  const shrunk = new Set<string>();
  for (;;) {
    const anchor = live.reduce<Item | null>((best, i) => (!best || i.priority > best.priority ? i : best), null);
    // The anchor is the elastic one: it starts at its minimum and takes whatever the others leave.
    const sizes = new Map(live.map((i) => [i.key, i === anchor ? i.min : Math.max(i.min, Math.round(i.pref))] as const));
    let sum = [...sizes.values()].reduce((a, b) => a + b, 0);
    if (sum <= total) {
      if (anchor) sizes.set(anchor.key, (sizes.get(anchor.key) ?? 0) + (total - sum));
      return { sizes, dropped, shrunk, overflow: false, anchor: anchor?.key ?? null };
    }
    let deficit = sum - total;
    const levels = [...new Set(live.map((i) => i.priority))].sort((a, b) => a - b);
    for (const level of levels) {
      if (deficit <= 0) break;
      const group = live.filter((i) => i.priority === level);
      const slack = group.reduce((s, i) => s + ((sizes.get(i.key) ?? 0) - i.min), 0);
      if (slack <= 0) continue;
      const take = Math.min(deficit, slack);
      let given = 0;
      group.forEach((i, n) => {
        const room = (sizes.get(i.key) ?? 0) - i.min;
        const share = n === group.length - 1 ? take - given : Math.min(room, Math.floor((take * room) / slack));
        const cut = Math.min(room, share);
        given += cut;
        if (cut > 0) {
          sizes.set(i.key, (sizes.get(i.key) ?? 0) - cut);
          shrunk.add(i.key);
        }
      });
      deficit -= given;
    }
    sum = [...sizes.values()].reduce((a, b) => a + b, 0);
    if (sum <= total) return { sizes, dropped, shrunk, overflow: false, anchor: anchor?.key ?? null };
    const victims = live.filter((i) => i !== anchor);
    if (!victims.length) return { sizes, dropped, shrunk, overflow: true, anchor: anchor?.key ?? null };
    const lowest = victims.reduce((v, i) => (i.priority < v.priority || (i.priority === v.priority && i.key >= v.key) ? i : v));
    dropped.push(lowest.key);
    shrunk.delete(lowest.key);
    live = live.filter((i) => i !== lowest);
  }
}

function hinted(v: OpenView, hints: SavedHints | undefined, axis: keyof Size): number {
  const asked = hints?.sizes?.[v.id]?.[axis];
  const pref = typeof asked === 'number' && Number.isFinite(asked) ? asked : v.layout.pref[axis];
  return Math.max(v.layout.min[axis], pref);
}

/** The pure planner: where every open view goes in `viewport`, and what yields when there is no room. */
export function planLayout(viewport: Size, open: readonly OpenView[], saved?: SavedHints): LayoutPlan {
  const views = [...open].sort(byPriorityThenId);
  const hidden: LayoutPlan['hidden'] = [];
  const tabbed: LayoutPlan['tabbed'] = [];
  const shrunk = new Set<string>();

  const groups: Record<ColumnKind, OpenView[]> = { left: [], main: [], full: [], side: [] };
  for (const v of views) {
    switch (v.layout.area) {
      case 'status':
        hidden.push({ id: v.id, edge: 'bottom' });
        break;
      case 'side':
        (v.layout.edge === 'left' ? groups.left : groups.side).push(v);
        break;
      case 'analysis':
        groups.main.push(v);
        break;
      case 'document':
        (v.fullHeight ? groups.full : groups.main).push(v);
        break;
    }
  }

  const kinds = (['left', 'main', 'full', 'side'] as const).filter((k) => groups[k].length);
  const colItems: Item[] = kinds.map((k) => ({
    key: k,
    min: Math.max(...groups[k].map((v) => v.layout.min.width)),
    pref: Math.max(...groups[k].map((v) => hinted(v, saved, 'width'))),
    priority: Math.max(...groups[k].map((v) => v.layout.priority)),
  }));
  const colFit = fitAxis(colItems, Math.max(0, viewport.width));
  const anchorKind = colFit.anchor as ColumnKind | null;
  const anchorTarget = (): string | undefined => (anchorKind ? groups[anchorKind][0]?.id : undefined);

  for (const k of colFit.dropped as ColumnKind[]) {
    for (const v of groups[k]) {
      const into = anchorTarget();
      if (k === 'left') hidden.push({ id: v.id, edge: 'left' });
      else if (k === 'side' || !into || v.layout.area === 'side') hidden.push({ id: v.id, edge: 'right' });
      else tabbed.push({ id: v.id, into });
    }
  }

  const columns: PlannedColumn[] = [];
  for (const k of kinds) {
    if (colFit.dropped.includes(k)) continue;
    const members = groups[k];
    const col: PlannedColumn = {
      kind: k,
      views: members.map((v) => v.id),
      width: colFit.sizes.get(k) ?? 0,
      minWidth: Math.max(...members.map((v) => v.layout.min.width)),
      anchor: k === anchorKind,
      rows: [],
    };
    if (colFit.shrunk.has(k)) members.forEach((v) => shrunk.add(v.id));
    if (k === 'main') {
      const rowKeys: string[] = [];
      const rowViews = new Map<string, OpenView[]>();
      for (const v of members) {
        const key = v.row ?? v.id;
        if (!rowViews.has(key)) {
          rowKeys.push(key);
          rowViews.set(key, []);
        }
        rowViews.get(key)!.push(v);
      }
      const rowItems: Item[] = rowKeys.map((key) => {
        const vs = rowViews.get(key)!;
        return {
          key,
          min: Math.max(...vs.map((v) => v.layout.min.height)),
          pref: Math.max(...vs.map((v) => hinted(v, saved, 'height'))),
          priority: Math.max(...vs.map((v) => v.layout.priority)),
        };
      });
      const rowFit = fitAxis(rowItems, Math.max(0, viewport.height));
      const topRow = rowFit.anchor;
      for (const key of rowFit.dropped) {
        const into = topRow ? rowViews.get(topRow)![0]?.id : undefined;
        for (const v of rowViews.get(key)!) if (into) tabbed.push({ id: v.id, into });
      }
      const kept = rowKeys.filter((key) => !rowFit.dropped.includes(key));
      // The dropped rows' views are tabs of the top row, not members of their own row.
      col.rows = kept.map((key) => {
        const vs = rowViews.get(key)!;
        // Groups of one row sit side by side: they share the column's width like columns share the window's.
        const cellKeys: string[] = [];
        const cellViews = new Map<string, OpenView[]>();
        for (const v of vs) {
          const ck = v.group ?? v.id;
          if (!cellViews.has(ck)) {
            cellKeys.push(ck);
            cellViews.set(ck, []);
          }
          cellViews.get(ck)!.push(v);
        }
        const cellItems: Item[] = cellKeys.map((ck) => {
          const cv = cellViews.get(ck)!;
          return { key: ck, min: Math.max(...cv.map((v) => v.layout.min.width)), pref: Math.max(...cv.map((v) => hinted(v, saved, 'width'))), priority: Math.max(...cv.map((v) => v.layout.priority)) };
        });
        const cellFit = fitAxis(cellItems, col.width);
        const keptCells = cellKeys.filter((ck) => !cellFit.dropped.includes(ck));
        const home = cellFit.anchor ? cellViews.get(cellFit.anchor)![0]?.id : undefined;
        for (const ck of cellFit.dropped) for (const v of cellViews.get(ck)!) if (home) tabbed.push({ id: v.id, into: home });
        for (const ck of keptCells) if (cellFit.shrunk.has(ck)) cellViews.get(ck)!.forEach((v) => shrunk.add(v.id));
        if (cellFit.overflow) colFit.overflow = true;
        const keptViews = keptCells.flatMap((ck) => cellViews.get(ck)!.map((v) => v.id));
        return {
          key,
          views: keptViews,
          cells: keptCells.map((ck) => ({ views: cellViews.get(ck)!.map((v) => v.id), width: cellFit.sizes.get(ck) ?? 0, minWidth: cellItems.find((i) => i.key === ck)!.min, anchor: ck === cellFit.anchor })),
          height: rowFit.sizes.get(key) ?? 0,
          minHeight: rowItems.find((i) => i.key === key)!.min,
          anchor: key === topRow,
        };
      });
      col.views = col.rows.flatMap((r) => r.views);
      for (const key of kept) if (rowFit.shrunk.has(key)) rowViews.get(key)!.forEach((v) => shrunk.add(v.id));
      if (rowFit.overflow) colFit.overflow = true;
    }
    columns.push(col);
  }

  const plan: LayoutPlan = {
    viewport,
    columns,
    tabbed,
    hidden: hidden.sort((a, b) => (a.id < b.id ? -1 : 1)),
    shrunk: [...shrunk].sort(),
    overflow: colFit.overflow,
    violations: [],
  };
  plan.violations = checkPlan(plan, open);
  return plan;
}

/**
 * The invariants of a plan as messages (empty when it is sound): every column and row is at or above its minimum,
 * columns do not overlap (their widths fit the viewport), rows fit the viewport height, nothing is both placed
 * and tabbed or hidden. A viewport too small for the single highest-priority column is `overflow`, reported
 * separately and not as a violation.
 */
export function checkPlan(plan: LayoutPlan, open: readonly OpenView[]): string[] {
  const out: string[] = [];
  if (!plan.overflow) {
    for (const c of plan.columns) if (c.width < c.minWidth) out.push(`column ${c.kind} is ${c.width} px, under its minimum ${c.minWidth}`);
    const total = plan.columns.reduce((s, c) => s + c.width, 0);
    if (total > plan.viewport.width) out.push(`columns overlap: ${total} px in ${plan.viewport.width} px`);
    for (const c of plan.columns) {
      for (const r of c.rows) for (const cell of r.cells) if (cell.width < cell.minWidth) out.push(`a cell of row ${r.key} is ${cell.width} px, under its minimum ${cell.minWidth}`);
      for (const r of c.rows) if (r.height < r.minHeight) out.push(`row ${r.key} is ${r.height} px, under its minimum ${r.minHeight}`);
      if (c.rows.length && c.rows.reduce((s, r) => s + r.height, 0) > plan.viewport.height) out.push('rows overlap');
    }
  }
  const placed = new Set(plan.columns.flatMap((c) => c.views));
  const away = [...plan.tabbed.map((t) => t.id), ...plan.hidden.map((h) => h.id)];
  for (const id of away) if (placed.has(id)) out.push(`${id} is both placed and tabbed or hidden`);
  if (new Set(away).size !== away.length) out.push('a view is both tabbed and hidden');
  for (const v of open) if (!placed.has(v.id) && !away.includes(v.id)) out.push(`${v.id} is nowhere in the plan`);
  return out;
}

/** Everything that is not simply placed: the views a plan moved away from their own column or row. */
export function displaced(plan: LayoutPlan, area?: LayoutArea, open: readonly OpenView[] = []): string[] {
  const ids = [...plan.tabbed.map((t) => t.id), ...plan.hidden.filter((h) => h.edge === 'right').map((h) => h.id)];
  return area ? ids.filter((id) => open.find((v) => v.id === id)?.layout.area === area) : ids;
}

/** Default declarations for views that declare none (plugin views): conservative per area (ADR 0056 section 1). */
export function defaultViewLayout(area: LayoutArea): ViewLayout {
  switch (area) {
    case 'side':
      return { area, min: { width: 320, height: 240 }, pref: { width: 372, height: 240 }, priority: 50 };
    case 'analysis':
      return { area, min: { width: 420, height: 180 }, pref: { width: 420, height: 280 }, priority: 55 };
    case 'status':
      return { area, min: { width: 320, height: 180 }, pref: { width: 480, height: 360 }, priority: 20 };
    case 'document':
      return { area, min: { width: 420, height: 240 }, pref: { width: 640, height: 400 }, priority: 60 };
  }
}

/**
 * Grow-back with hysteresis: of the views the plan moved away (`displaced`), the ones that would still be placed with
 * `margin` px less room in each direction. A view tabbed or hidden at width W therefore returns only once the window is
 * `margin` wider than the width where it first fits, so a few pixels of jitter around the threshold cannot flap it.
 */
export function restorable(viewport: Size, open: readonly OpenView[], displaced: readonly string[], margin: Size, saved?: SavedHints): string[] {
  const tight = planLayout({ width: viewport.width - margin.width, height: viewport.height - margin.height }, open, saved);
  const away = new Set([...tight.tabbed.map((t) => t.id), ...tight.hidden.map((h) => h.id)]);
  return displaced.filter((id) => !away.has(id) && open.some((v) => v.id === id)).sort();
}

const AREAS: readonly LayoutArea[] = ['document', 'side', 'analysis', 'status'];
const MAX_PX = 8000;
const num = (v: unknown, fallback: number): number => (typeof v === 'number' && Number.isFinite(v) ? Math.min(MAX_PX, Math.max(0, Math.round(v))) : fallback);

/**
 * Parses an untrusted `ViewLayout` (a plugin's declaration) once, at the edge: null when there is none or its area is
 * unknown; otherwise finite, bounded numbers with `min <= pref` and a priority between 1 and 100. Missing parts take the
 * area's default, so a plugin may declare only what it cares about.
 */
export function sanitizeViewLayout(raw: unknown): ViewLayout | null {
  if (typeof raw !== 'object' || raw === null) return null;
  const r = raw as Record<string, unknown>;
  const area = r['area'];
  if (!AREAS.includes(area as LayoutArea)) return null;
  const d = defaultViewLayout(area as LayoutArea);
  const size = (v: unknown, fb: Size): Size => {
    const o = typeof v === 'object' && v !== null ? (v as Record<string, unknown>) : {};
    return { width: num(o['width'], fb.width), height: num(o['height'], fb.height) };
  };
  const min = size(r['min'], d.min);
  const pref = size(r['pref'], d.pref);
  return {
    area: area as LayoutArea,
    min,
    pref: { width: Math.max(min.width, pref.width), height: Math.max(min.height, pref.height) },
    priority: Math.min(100, Math.max(1, num(r['priority'], d.priority))),
    ...(r['edge'] === 'left' && area === 'side' ? { edge: 'left' as const } : {}),
  };
}
