/**
 * Where runs between lifecycle markers go on the timeline (a purely visual transform of what the engine returned):
 * each run is a band on its lane from its start to its stop; a run still open fades out at its right end; a stop
 * without a start is a warning glyph at its time; a run that ended without a stop, or ran longer than allowed, gets a
 * warning glyph at its end. Runs outside the view, on lanes not shown, or of hidden kinds are left out.
 *
 * Runs that overlap in time (several views open at once, nested "X started" / "X ended") are stacked on labelled
 * sub-tracks below the lane's markers, so none hides another (`packRows`).
 */
import { AnnotationDto, RunInfo, RunStatus } from '../core/api.types';
import { clamp } from '../core/format';
import { packRows } from './sub-tracks';

/** Narrowest band drawn, in percent of the plot, so short runs stay visible and clickable. */
export const MIN_RUN_PCT = 0.3;
/** Space kept between two runs on one sub-track (px); back to back (a restart) shares one. */
export const RUN_GAP_PX = 0;
/** Sub-tracks per lane at most; runs beyond share the one that frees up first. */
export const MAX_RUN_ROWS = 6;
/** Width of the hit area of a stop without start (px; WCAG 2.5.8). */
const POINT_PX = 24;
/** A band shows its label from this width (px); narrower ones have it in the tooltip only. */
export const RUN_LABEL_MIN_PX = 44;
/** An open band shows "still running" at its end from this width (px); narrower ones leave it to the tooltip. */
export const RUN_STILL_MIN_PX = 140;

/** Why a run carries a warning glyph. */
export type RunAnomaly = 'noStart' | 'noStop' | 'tooLong';

export interface RunBandVm {
  ann: AnnotationDto;
  run: RunInfo;
  /** Stable key for `@for` tracking. */
  id: string;
  /** Start in percent of the visible range (clamped to 0..100). */
  left: number;
  /** Width in percent (0 for a stop without start). */
  width: number;
  status: RunStatus;
  /** A point (stop without start). */
  point: boolean;
  /** The run starts before / ends after the visible range. */
  clippedLeft: boolean;
  clippedRight: boolean;
  /** Warning glyph, if any (drawn at the run's end, or at the point). */
  anomaly: RunAnomaly | null;
  /** The sub-track (0 = first below the lane's markers). */
  row: number;
  /** What the band says: the paired field value (e.g. the view's name), else the rule's label. */
  label: string;
  /** The band is wide enough to show its label. */
  labelFits: boolean;
  /** The band is wide enough for its label and "still running" at its open end. */
  stillFits: boolean;
}

export interface RunLayoutOptions {
  laneKeys: readonly string[];
  view: { from: number; to: number };
  /** Span kinds switched off in the legend. */
  hidden: ReadonlySet<string>;
  /** False when the "Runs" legend toggle is off. */
  shown: boolean;
  /** Width of the plot (px), for gaps and labels; 1000 when unknown. */
  plotWidth?: number;
}

/** The anomaly a run shows, most telling first. */
export function anomalyOf(run: RunInfo): RunAnomaly | null {
  if (run.status === 'noStart') return 'noStart';
  if (run.status === 'noStop') return 'noStop';
  return run.tooLong ? 'tooLong' : null;
}

/** What a run's band says: the value it was paired by (a view, a job), else the rule's label. */
export function runLabel(ann: AnnotationDto, run: RunInfo): string {
  return run.group?.value || run.spanLabel || ann.label;
}

function band(ann: AnnotationDto, run: RunInfo, view: { from: number; to: number }): RunBandVm {
  const span = Math.max(1, view.to - view.from);
  const point = run.status === 'noStart';
  const rawLeft = ((ann.from - view.from) / span) * 100;
  const rawRight = ((ann.to - view.from) / span) * 100;
  const left = clamp(rawLeft, 0, 100);
  const right = clamp(rawRight, 0, 100);
  return {
    ann,
    run,
    id: `${ann.kind}|${ann.instance ?? ''}|${run.group?.value ?? ''}|${ann.from}|${run.status}`,
    left: point ? left : Math.min(left, 100 - MIN_RUN_PCT),
    width: point ? 0 : Math.max(MIN_RUN_PCT, right - left),
    status: run.status,
    point,
    clippedLeft: rawLeft < 0,
    clippedRight: rawRight > 100,
    anomaly: anomalyOf(run),
    row: 0,
    label: runLabel(ann, run),
    labelFits: false,
    stillFits: false,
  };
}

/** Stacks one lane's bands on sub-tracks (in place) and decides which can show their label. */
function stack(list: RunBandVm[], plotWidth: number): void {
  const px = plotWidth / 100;
  const spans = list.map((v) => {
    const start = v.left * px;
    return { start: v.point ? start - POINT_PX / 2 : start, end: v.point ? start + POINT_PX / 2 : (v.left + v.width) * px };
  });
  const { rows } = packRows(spans, { gap: RUN_GAP_PX, maxRows: MAX_RUN_ROWS });
  list.forEach((v, i) => {
    v.row = rows[i] ?? 0;
    v.labelFits = !v.point && v.width * px >= RUN_LABEL_MIN_PX;
    v.stillFits = !v.point && v.width * px >= RUN_STILL_MIN_PX;
  });
}

/** The sub-tracks a lane's bands use (0 for none). */
export function runRowCount(bands: readonly RunBandVm[] | undefined): number {
  let n = 0;
  for (const b of bands ?? []) n = Math.max(n, b.row + 1);
  return n;
}

/** Run bands by lane key, in time order. */
export function layoutRuns(anns: readonly AnnotationDto[], o: RunLayoutOptions): Map<string, RunBandVm[]> {
  const out = new Map<string, RunBandVm[]>();
  if (!o.shown) return out;
  const known = new Set(o.laneKeys);
  for (const a of anns) {
    const run = a.run;
    if (!run || a.instance === null || !known.has(a.instance) || o.hidden.has(a.kind)) continue;
    if (a.to < o.view.from || a.from > o.view.to) continue;
    const list = out.get(a.instance);
    const vm = band(a, run, o.view);
    if (list) list.push(vm);
    else out.set(a.instance, [vm]);
  }
  const width = o.plotWidth && o.plotWidth > 0 ? o.plotWidth : 1000;
  for (const list of out.values()) {
    list.sort((a, b) => a.ann.from - b.ann.from);
    stack(list, width);
  }
  return out;
}

/** One legend entry per span kind. */
export interface RunKindVm {
  kind: string;
  label: string;
  count: number;
  /** Runs without a start or stop, or over the maximum duration. */
  anomalies: number;
}

/** The span kinds present among the runs, by label. */
export function runKinds(anns: readonly AnnotationDto[]): RunKindVm[] {
  const by = new Map<string, RunKindVm>();
  for (const a of anns) {
    if (!a.run) continue;
    const k = by.get(a.kind) ?? { kind: a.kind, label: a.run.spanLabel || a.kind, count: 0, anomalies: 0 };
    k.count++;
    if (anomalyOf(a.run)) k.anomalies++;
    by.set(a.kind, k);
  }
  return [...by.values()].sort((a, b) => a.label.localeCompare(b.label));
}

/** The time range a click on a run filters to (never empty, so a point still selects its moment). */
export function runRange(a: AnnotationDto): { from: number; to: number } {
  const from = a.evidenceQuery.from ?? a.from;
  return { from, to: Math.max(a.evidenceQuery.to ?? a.to, from + 1) };
}
