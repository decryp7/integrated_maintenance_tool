import { Lane, MarkerKind, Timeline } from '../core/api.types';
import { clamp, formatCount, formatTime } from '../core/format';
import { SourceMark } from '../sources/source-colors.store';
import { LaidMarker, layoutLane, MarkerTarget, MarkerVm } from './marker-layout';

/** Width of the lane labels (px) until the plot is measured; the theme's `--tl-label-w` sets the real one. */
export const LABEL_W = 232;
/** Height of a lane's density drawing space in viewBox units (CSS scales it to the lane). */
export const DENSITY_VB = 16;
/** A lane with fewer events than the plot is wide / this draws one tick per busy bucket instead of an area. */
export const SPARSE_PX_PER_EVENT = 6;
/** Kinds drawn as a thin tick strip when a lane is too busy for full shapes. */
const COMPACT_KINDS: ReadonlySet<string> = new Set(['warning']);

/** A marker hit area as the lanes draw it. */
export interface TargetVm extends MarkerTarget {
  /** Left edge in % of the plot. */
  leftPct: number;
  /** Names the hit area by its markers, the one it activates first. */
  aria: string;
}

/** Everything one lane row draws. */
export interface LaneVm {
  lane: Lane;
  left: number;
  width: number;
  /** Density bars per bucket, scaled to the lane's own busiest bucket (laneFill: barcode). */
  densityPath: string;
  /** Density as an area, scaled to the busiest bucket of all lanes so volumes compare (laneFill: neutral, bySource). */
  densityArea: string;
  /** The top edge of `densityArea` (stroked in the lane's colour when lanes are coloured by source). */
  densityEdge: string;
  /** Too few events for an area: one tick per busy bucket instead (`sparsePath`). */
  sparse: boolean;
  sparsePath: string;
  /** The lane's source colour (its first source), or null. */
  color: string | null;
  markers: LaidMarker[];
  targets: TargetVm[];
  ticksPath: string;
  tickCount: number;
  /** The lane is in the lane filter. */
  filtered: boolean;
  meta: string;
  /** Tooltip of the lane label (what clicking it does). */
  title: string;
  /** Source / instance colour dots. */
  marks: SourceMark[];
  /** Sources not shown as dots (0 when all are). */
  moreSources: number;
}

export interface LaneVmOptions {
  kinds: ReadonlyMap<string, MarkerKind>;
  groups: ReadonlySet<string>;
  timezone: Parameters<typeof formatTime>[1];
  plotWidth: number;
  /** The main timeline (instances) or an additional one (lanes): changes the label titles. */
  main: boolean;
  marks(lane: Lane): SourceMark[];
  moreSources(lane: Lane): number;
  markerAria(m: LaidMarker): string;
}

/** "12 of 30" when a filter hides some of the lane's events, else just the count. */
export function laneCount(lane: Lane): string {
  const matched = formatCount(lane.matched);
  if (lane.matched === lane.total) return matched;
  const total = formatCount(lane.total);
  return $localize`:@@timeline.lane.matchedOfTotal:${matched}:matched: of ${total}:total:`;
}

/** What clicking a lane label does. */
export function laneTitle(key: string, filtered: boolean, main: boolean): string {
  if (main) {
    return filtered
      ? $localize`:@@timeline.laneTitle.removeInstance:${key}:key: — click to remove the instance filter`
      : $localize`:@@timeline.laneTitle.onlyInstance:${key}:key: — click to show only this instance`;
  }
  return filtered
    ? $localize`:@@timeline.laneTitle.removeLane:${key}:key: — click to remove the lane filter`
    : $localize`:@@timeline.laneTitle.onlyLane:${key}:key: — click to show only this lane`;
}

/** The y of each bucket of a density area: √(count / max), so quiet lanes stay visible next to busy ones. */
function areaYs(density: readonly number[], max: number): number[] {
  const m = Math.max(1, max);
  return density.map((v) => (v > 0 ? DENSITY_VB - Math.max(1, Math.sqrt(v / m) * DENSITY_VB) : DENSITY_VB));
}

/** A step outline over the buckets (`H` runs merged), from x = 0 to the last bucket. */
function steps(ys: readonly number[]): string {
  if (!ys.length) return '';
  let d = `M0 ${(ys[0] ?? DENSITY_VB).toFixed(2)}`;
  for (let x = 1; x < ys.length; x++) if (ys[x] !== ys[x - 1]) d += `H${x}V${(ys[x] ?? DENSITY_VB).toFixed(2)}`;
  return `${d}H${ys.length}`;
}

/** The density of a lane as a closed area (empty when the lane has no events). */
export function densityArea(density: readonly number[], max: number): string {
  if (!density.some((v) => v > 0)) return '';
  return `M0 ${DENSITY_VB}L${steps(areaYs(density, max)).slice(1)}V${DENSITY_VB}Z`;
}

/** The top edge of `densityArea`, only over buckets with events (never along the empty baseline). */
export function densityEdge(density: readonly number[], max: number): string {
  const ys = areaYs(density, max);
  let d = '';
  for (let x = 0; x < ys.length; ) {
    if (!((density[x] ?? 0) > 0)) {
      x++;
      continue;
    }
    // One sub-path per run of busy buckets.
    let end = x;
    while (end < ys.length && (density[end] ?? 0) > 0) end++;
    d += steps(ys.slice(x, end)).replace(/^M0 /, `M${x} `).replace(/H(\d+)/g, (_, n: string) => `H${Number(n) + x}`);
    x = end;
  }
  return d;
}

/** One tick per bucket with events, for lanes too quiet for an area (viewBox `0 0 buckets 16`). */
export function sparsePath(density: readonly number[]): string {
  let d = '';
  density.forEach((v, x) => {
    if (v > 0) d += `M${(x + 0.5).toFixed(1)} 3.5V12.5`;
  });
  return d;
}

/** The SVG path of a lane's density bars (bucket x, height 2..16). */
export function densityPath(density: readonly number[]): string {
  const max = Math.max(1, ...density);
  let d = '';
  density.forEach((v, x) => {
    if (!v) return;
    const h = 2 + (v / max) * 14;
    d += `M${x + 0.1} ${(16 - h).toFixed(2)}h0.8v${h.toFixed(2)}h-0.8z`;
  });
  return d;
}

/** Lays out the lanes of `t` (already in display order) for drawing. Pure: no signals, no DOM. */
export function buildLaneVms(lanes: readonly Lane[], t: Timeline, o: LaneVmOptions): LaneVm[] {
  const span = Math.max(1, t.to - t.from);
  const priority = [...o.kinds.keys()];
  let busiest = 1;
  for (const l of lanes) for (const v of l.density) busiest = Math.max(busiest, v);
  return lanes.map((lane) => {
    const layout = layoutLane(
      lane.markers.map((cluster): MarkerVm => {
        const k = o.kinds.get(cluster.kind);
        return { cluster, left: clamp(((cluster.ts - t.from) / span) * 100, 0, 100), color: k?.color ?? null, label: k?.label ?? cluster.kind };
      }),
      { plotWidth: o.plotWidth, priority, compactKinds: COMPACT_KINDS },
    );
    const filtered = o.groups.has(lane.key);
    const marks = o.marks(lane);
    const sparse = lane.matched > 0 && lane.matched < o.plotWidth / SPARSE_PX_PER_EVENT;
    return {
      lane,
      left: clamp(((lane.first - t.from) / span) * 100, 0, 100),
      width: Math.max(0.2, ((lane.last - lane.first) / span) * 100),
      densityPath: densityPath(lane.density),
      densityArea: sparse ? '' : densityArea(lane.density, busiest),
      densityEdge: sparse ? '' : densityEdge(lane.density, busiest),
      sparse,
      sparsePath: sparse ? sparsePath(lane.density) : '',
      color: marks[0]?.swatch.color ?? null,
      markers: layout.shapes,
      targets: layout.targets.map((target) => ({
        ...target,
        leftPct: (target.left / o.plotWidth) * 100,
        aria: [target.primary, ...target.members.filter((g) => g.marker !== target.primary).map((g) => g.marker)].map((m) => o.markerAria(m)).join('; '),
      })),
      ticksPath: layout.ticks.map((x) => `M${(x * 10).toFixed(1)} 0v6`).join(''),
      tickCount: layout.tickCount,
      filtered,
      meta: `${laneCount(lane)} · ${formatTime(lane.first, o.timezone).slice(0, 8)} → ${formatTime(lane.last, o.timezone).slice(0, 8)}`,
      title: laneTitle(lane.key, filtered, o.main),
      marks,
      moreSources: o.moreSources(lane),
    };
  });
}
