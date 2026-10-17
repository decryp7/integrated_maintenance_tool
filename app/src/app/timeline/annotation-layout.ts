/**
 * Where plugin annotations go on the timeline (a purely visual transform of what the engine returned):
 * an annotation for one instance is drawn on that lane; one for every instance on a machine (`on`) on every
 * lane of that machine; one for every instance with nothing to match on as a full-height band. Spans are
 * bands, points (`from == to`) glyphs.
 */
import { AnnotationDto, FieldMatch, MarkerKind, Severity } from '../core/api.types';
import { clamp } from '../core/format';

/** Joins the values of the group-by fields into a lane key (crates/imt-core/src/query.rs GROUP_SEPARATOR). */
export const GROUP_SEPARATOR = ' · ';
/** Narrowest band drawn, in percent of the plot, so short spans stay visible and clickable. */
export const MIN_BAND_PCT = 0.4;

export interface AnnotationVm {
  ann: AnnotationDto;
  /** Stable key for `@for` tracking. */
  id: string;
  /** Start, in percent of the visible range (clamped to 0..100). */
  left: number;
  /** Width in percent (0 for points). */
  width: number;
  point: boolean;
}

export interface AnnotationLayout {
  /** By lane key, in time order. */
  lanes: Map<string, AnnotationVm[]>;
  /** Full-height bands and points (no lane to put them on). */
  global: AnnotationVm[];
}

export interface LayoutOptions {
  laneKeys: readonly string[];
  view: { from: number; to: number };
  /** Fields the lane keys are made of (the effective group-by). */
  groupBy: readonly string[];
  /** Kinds the user switched off in the legend. */
  hidden: ReadonlySet<string>;
}

/**
 * Whether a lane belongs to the machine (or other field value) of `on`: true/false when `on.field` is one of
 * the group-by fields, null when the lane key does not tell.
 */
export function laneMatches(key: string, groupBy: readonly string[], on: FieldMatch): boolean | null {
  const i = groupBy.indexOf(on.field);
  if (i < 0) return null;
  const parts = key.split(GROUP_SEPARATOR);
  return parts.length === groupBy.length ? parts[i] === on.value : null;
}

function vm(ann: AnnotationDto, view: { from: number; to: number }): AnnotationVm {
  const span = Math.max(1, view.to - view.from);
  const point = ann.from === ann.to;
  const left = clamp(((ann.from - view.from) / span) * 100, 0, 100);
  const right = clamp(((ann.to - view.from) / span) * 100, 0, 100);
  return {
    ann,
    id: `${ann.annotator}|${ann.kind}|${ann.instance ?? ''}|${ann.on?.value ?? ''}|${ann.from}|${ann.to}`,
    left: point ? left : Math.min(left, 100 - MIN_BAND_PCT),
    width: point ? 0 : Math.max(MIN_BAND_PCT, right - left),
    point,
  };
}

/** Lays annotations out over the lanes; those outside the view, hidden, or for a lane not shown are left out. */
export function layoutAnnotations(anns: readonly AnnotationDto[], o: LayoutOptions): AnnotationLayout {
  const lanes = new Map<string, AnnotationVm[]>();
  const global: AnnotationVm[] = [];
  const known = new Set(o.laneKeys);
  const add = (key: string, v: AnnotationVm) => {
    const list = lanes.get(key);
    if (list) list.push(v);
    else lanes.set(key, [v]);
  };
  for (const a of anns) {
    if (o.hidden.has(a.kind) || a.to < o.view.from || a.from > o.view.to) continue;
    const v = vm(a, o.view);
    if (a.instance !== null) {
      if (known.has(a.instance)) add(a.instance, v);
      continue;
    }
    if (a.on) {
      const on = a.on;
      const verdicts = o.laneKeys.map((k) => [k, laneMatches(k, o.groupBy, on)] as const);
      if (verdicts.some(([, m]) => m === null)) global.push(v);
      else for (const [k, m] of verdicts) if (m) add(k, v);
      continue;
    }
    global.push(v);
  }
  return { lanes, global };
}

const RANK: Record<Severity, number> = { critical: 0, warning: 1, info: 2 };

/** One legend entry per annotation kind. */
export interface AnnotationKindVm {
  kind: string;
  label: string;
  owner: string;
  count: number;
  /** Most severe severity among its annotations. */
  severity: Severity;
}

/** The kinds present, labelled like the plugin's marker kind of the same name when there is one. */
export function annotationKinds(anns: readonly AnnotationDto[], markerKinds: ReadonlyMap<string, MarkerKind>): AnnotationKindVm[] {
  const by = new Map<string, AnnotationKindVm>();
  for (const a of anns) {
    const k = by.get(a.kind);
    if (k) {
      k.count++;
      if (RANK[a.severity] < RANK[k.severity]) k.severity = a.severity;
    } else {
      const label = markerKinds.get(a.kind)?.label ?? a.kind.slice(a.kind.lastIndexOf('/') + 1).replace(/[-_]/g, ' ');
      by.set(a.kind, { kind: a.kind, label, owner: a.owner, count: 1, severity: a.severity });
    }
  }
  return [...by.values()].sort((a, b) => a.label.localeCompare(b.label));
}
