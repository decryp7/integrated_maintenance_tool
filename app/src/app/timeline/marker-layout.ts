import { MarkerCluster } from '../core/api.types';

/** A marker cluster positioned on a lane (left in % of the plot width). */
export interface MarkerVm {
  cluster: MarkerCluster;
  left: number;
  color: string | null;
  label: string;
}

/** A marker as drawn: shape plus whether its count badge fits. */
export interface LaidMarker extends MarkerVm {
  showCount: boolean;
}

/**
 * One clickable hit area on a lane (WCAG 2.2 SC 2.5.8, target size minimum): at least {@link TARGET_PX} px wide and
 * never overlapping another target. Markers whose 24px targets would overlap are aggregated into one target that
 * still draws every glyph at its own time; from the keyboard it activates its primary member, while a pointer can
 * still pick a glyph.
 */
export interface MarkerTarget {
  /** Left edge in px from the plot's left. */
  left: number;
  /** Width in px (at least TARGET_PX). */
  width: number;
  /** The member the target activates: the earliest of its most important kind. */
  primary: LaidMarker;
  /** Members in paint order (least important first), each with its glyph centre in px from the target's left. */
  members: { marker: LaidMarker; offset: number }[];
}

export interface LaneLayout {
  /** Full-shape markers, in paint order (least important first, so important ones stay on top). */
  shapes: LaidMarker[];
  /** Hit areas for the shapes, left to right. */
  targets: MarkerTarget[];
  /** Positions (%) of markers drawn as a thin tick strip because the lane is too busy. */
  ticks: number[];
  /** Events represented by the tick strip. */
  tickCount: number;
}

export interface LayoutOptions {
  plotWidth: number;
  /** Marker kinds in priority order (first = most important). */
  priority: string[];
  /** Kinds that collapse to ticks when the lane is busy. */
  compactKinds: ReadonlySet<string>;
  /** Same-kind markers closer than this many px merge into one cluster. */
  mergePx?: number;
  /** Above this many shapes in view, compact kinds become ticks. */
  maxShapes?: number;
}

/** Minimum hit-area size in px (WCAG 2.2 SC 2.5.8, target size minimum). */
export const TARGET_PX = 24;
const TARGET_HALF = TARGET_PX / 2;

/** Icon half-width plus a small gap, in px. */
const ICON_HALF = 8;
/** Badge sits this many px right of the marker centre. */
const BADGE_OFFSET = 9;
const BADGE_CHAR_PX = 5.6;
const BADGE_PAD_PX = 4;

/** Width in px of a count badge (9px monospace digits plus padding). */
export function badgeWidth(count: number): number {
  return BADGE_PAD_PX + String(count).length * BADGE_CHAR_PX;
}

/** Default busy threshold: roughly one shape per 18px of plot width, at least 24. */
export function defaultMaxShapes(plotWidth: number): number {
  return Math.max(24, Math.floor(plotWidth / 18));
}

/**
 * Merges same-kind markers whose positions (in %) are closer than minGap, keeping the
 * earliest record as the jump target and summing counts.
 */
export function mergeMarkers(markers: MarkerVm[], minGap: number): MarkerVm[] {
  const byKind = new Map<string, MarkerVm[]>();
  for (const m of markers) {
    const list = byKind.get(m.cluster.kind) ?? [];
    list.push(m);
    byKind.set(m.cluster.kind, list);
  }
  const out: MarkerVm[] = [];
  for (const list of byKind.values()) {
    list.sort((a, b) => a.left - b.left);
    const merged: MarkerVm[] = [];
    for (const m of list) {
      const last = merged[merged.length - 1];
      if (last && m.left - last.left < minGap) {
        merged[merged.length - 1] = { ...last, cluster: { ...last.cluster, count: last.cluster.count + m.cluster.count } };
      } else {
        merged.push(m);
      }
    }
    out.push(...merged);
  }
  return out;
}

interface Interval {
  from: number;
  to: number;
}

const overlaps = (a: Interval, b: Interval) => a.from < b.to && b.from < a.to;

/**
 * Decides which count badges fit: badges never overlap another marker's icon or another
 * badge, and higher counts win the space (low counts are dropped first).
 */
export function placeBadges(markers: MarkerVm[], plotWidth: number): Set<MarkerVm> {
  const x = (m: MarkerVm) => (m.left / 100) * plotWidth;
  const icons = markers.map((m) => ({ m, from: x(m) - ICON_HALF, to: x(m) + ICON_HALF }));
  const placed: Interval[] = [];
  const shown = new Set<MarkerVm>();
  const candidates = markers.filter((m) => m.cluster.count > 1).sort((a, b) => b.cluster.count - a.cluster.count || a.left - b.left);
  for (const m of candidates) {
    const from = x(m) + BADGE_OFFSET;
    const badge = { from, to: from + badgeWidth(m.cluster.count) };
    if (badge.to > plotWidth + BADGE_OFFSET) continue;
    if (icons.some((i) => i.m !== m && overlaps(i, badge))) continue;
    if (placed.some((p) => overlaps(p, badge))) continue;
    placed.push(badge);
    shown.add(m);
  }
  return shown;
}

/**
 * Groups shapes (given in paint order) into non-overlapping hit areas of at least TARGET_PX. A target is centred on
 * its first marker (clamped to the plot's left edge) and absorbs the following markers whose own target would start
 * inside it, so one target never spans 2 × TARGET_PX; it stretches right over its members up to where the next
 * target begins.
 */
export function layoutTargets(shapes: LaidMarker[], plotWidth: number): MarkerTarget[] {
  const items = shapes.map((marker, paint) => ({ marker, paint, x: (marker.left / 100) * plotWidth })).sort((a, b) => a.x - b.x || a.paint - b.paint);
  const groups: { left: number; items: typeof items }[] = [];
  for (const item of items) {
    const cur = groups[groups.length - 1];
    if (cur && item.x - TARGET_HALF < cur.left + TARGET_PX) cur.items.push(item);
    else groups.push({ left: Math.max(0, item.x - TARGET_HALF), items: [item] });
  }
  return groups.map((g, i) => {
    const last = g.items[g.items.length - 1]!;
    const next = groups[i + 1];
    const right = Math.max(g.left + TARGET_PX, Math.min(last.x + TARGET_HALF, next ? next.left : Infinity));
    const painted = [...g.items].sort((a, b) => a.paint - b.paint);
    // Paint order is least important first, so the last member has the most important kind; g.items is by time.
    const topKind = painted[painted.length - 1]!.marker.cluster.kind;
    const primary = g.items.find((p) => p.marker.cluster.kind === topKind)!;
    return { left: g.left, width: right - g.left, primary: primary.marker, members: painted.map((p) => ({ marker: p.marker, offset: p.x - g.left })) };
  });
}

/** Lays out one lane's markers: merge, collapse busy kinds to ticks, order by priority, fit badges. */
export function layoutLane(markers: MarkerVm[], opts: LayoutOptions): LaneLayout {
  const width = Math.max(1, opts.plotWidth);
  const merged = mergeMarkers(markers, ((opts.mergePx ?? 12) / width) * 100);
  const busy = merged.length > (opts.maxShapes ?? defaultMaxShapes(width));
  const compact = busy ? merged.filter((m) => opts.compactKinds.has(m.cluster.kind)) : [];
  const shapes = busy ? merged.filter((m) => !opts.compactKinds.has(m.cluster.kind)) : merged;
  const rank = (kind: string) => {
    const i = opts.priority.indexOf(kind);
    return i < 0 ? opts.priority.length : i;
  };
  shapes.sort((a, b) => rank(b.cluster.kind) - rank(a.cluster.kind) || a.left - b.left);
  const badges = placeBadges(shapes, width);
  const laid = shapes.map((m) => ({ ...m, showCount: badges.has(m) }));
  return {
    shapes: laid,
    targets: layoutTargets(laid, width),
    ticks: compact.map((m) => m.left).sort((a, b) => a - b),
    tickCount: compact.reduce((n, m) => n + m.cluster.count, 0),
  };
}
