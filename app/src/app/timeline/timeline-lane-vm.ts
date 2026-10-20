import { Lane, MarkerKind, Timeline } from '../core/api.types';
import { clamp, formatCount, formatTime } from '../core/format';
import { SourceMark } from '../sources/source-colors.store';
import { LaidMarker, layoutLane, MarkerTarget, MarkerVm } from './marker-layout';

/** Width of the lane labels (px). */
export const LABEL_W = 250;
/** Height of the events-per-bucket histogram (px). */
export const HIST_H = 30;
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
  densityPath: string;
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
  return lanes.map((lane) => {
    const layout = layoutLane(
      lane.markers.map((cluster): MarkerVm => {
        const k = o.kinds.get(cluster.kind);
        return { cluster, left: clamp(((cluster.ts - t.from) / span) * 100, 0, 100), color: k?.color ?? null, label: k?.label ?? cluster.kind };
      }),
      { plotWidth: o.plotWidth, priority, compactKinds: COMPACT_KINDS },
    );
    const filtered = o.groups.has(lane.key);
    return {
      lane,
      left: clamp(((lane.first - t.from) / span) * 100, 0, 100),
      width: Math.max(0.2, ((lane.last - lane.first) / span) * 100),
      densityPath: densityPath(lane.density),
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
      marks: o.marks(lane),
      moreSources: o.moreSources(lane),
    };
  });
}
