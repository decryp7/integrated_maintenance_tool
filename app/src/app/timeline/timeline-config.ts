import { Query } from '../core/api.types';
import { EMPTY_QUERY } from '../core/workspace.store';
import { Range } from './zoom';

/** Id of the view (and dock panel) of the main timeline, which is bound to the workspace filter. */
export const MAIN_TIMELINE = 'timeline';

/** Separates a view id from an instance key in the panel id of an additional timeline (`timeline#2`). */
export const INSTANCE_SEP = '#';

/** Upper bound on additional timeline panels (each one is a timeline request per change). */
export const MAX_TIMELINES = 12;

/** Longest title and query text kept from saved state (the engine caps the query again). */
const MAX_TITLE = 80;
const MAX_QL = 4000;
const MAX_LIST = 200;

/** What an additional timeline shows: the workspace filter, or its own query-language expression. */
export type TimelineScope = 'global' | 'own';

/** How lanes are formed: the workspace's instances, or by source, level or one field. */
export type LaneMode = 'instance' | 'source' | 'level' | 'field';

export const LANE_MODES: readonly { mode: LaneMode; label: string }[] = [
  { mode: 'instance', label: $localize`:@@timeline.lanes.instance:Instance` },
  { mode: 'source', label: $localize`:@@timeline.lanes.source:Source` },
  { mode: 'level', label: $localize`:timeline lane mode|Group lanes by log level@@timeline.lanes.level:Level` },
  { mode: 'field', label: $localize`:@@timeline.lanes.field:Field` },
];

/** The saved settings of one additional timeline panel. */
export interface TimelineConfig {
  /** The dock panel id (`timeline#<n>`). */
  id: string;
  title: string;
  scope: TimelineScope;
  /** Query-language expression used when `scope` is `own` (empty = all events). */
  ql: string;
  lanes: LaneMode;
  /** Field the lanes are grouped by when `lanes` is `field`. */
  laneField: string;
  /** Marker kinds to restrict to; null = as the workspace filter (scope `global`) or none (scope `own`). */
  markers: string[] | null;
  /** Lanes (keys under this panel's grouping) to show only; empty = all. */
  groups: string[];
  /** Zoom, pan and the time cursor follow the main timeline. */
  sync: boolean;
}

/** True for the panel id of an additional timeline. */
export function isTimelineInstance(id: string | null | undefined): id is string {
  return !!id && id.startsWith(MAIN_TIMELINE + INSTANCE_SEP) && id.length > MAIN_TIMELINE.length + 1;
}

/** The number in an instance id (`timeline#3` → 3), or null when it has none. */
function instanceNumber(id: string): number | null {
  const n = Number(id.slice(MAIN_TIMELINE.length + 1));
  return Number.isInteger(n) && n > 0 ? n : null;
}

/** The first free panel id, numbered from 2 (the main timeline is the first). */
export function nextTimelineId(taken: Iterable<string>): string {
  const used = new Set<number>();
  for (const id of taken) {
    const n = isTimelineInstance(id) ? instanceNumber(id) : null;
    if (n !== null) used.add(n);
  }
  let n = 2;
  while (used.has(n)) n++;
  return MAIN_TIMELINE + INSTANCE_SEP + n;
}

/** "Timeline 2" for `timeline#2`. */
export function defaultTitle(id: string): string {
  const n = instanceNumber(id);
  return n === null ? $localize`:@@timeline.defaultTitle:Timeline` : $localize`:@@timeline.defaultTitleN:Timeline ${n}:number:`;
}

export function defaultConfig(id: string): TimelineConfig {
  return { id, title: defaultTitle(id), scope: 'global', ql: '', lanes: 'instance', laneField: '', markers: null, groups: [], sync: true };
}

const strings = (v: unknown): string[] | null =>
  Array.isArray(v) ? [...new Set(v.filter((x): x is string => typeof x === 'string' && !!x))].slice(0, MAX_LIST) : null;

/** Parses one saved config (untrusted project, session or storage state); null when it is not a timeline. */
export function sanitizeConfig(raw: unknown): TimelineConfig | null {
  if (!raw || typeof raw !== 'object' || Array.isArray(raw)) return null;
  const v = raw as Record<string, unknown>;
  const id = v['id'];
  if (typeof id !== 'string' || !isTimelineInstance(id) || id.length > 64) return null;
  const base = defaultConfig(id);
  const title = typeof v['title'] === 'string' ? v['title'].trim().slice(0, MAX_TITLE) : '';
  const lanes = LANE_MODES.some((m) => m.mode === v['lanes']) ? (v['lanes'] as LaneMode) : base.lanes;
  return {
    id,
    title: title || base.title,
    scope: v['scope'] === 'own' ? 'own' : 'global',
    ql: typeof v['ql'] === 'string' ? v['ql'].slice(0, MAX_QL) : '',
    lanes,
    laneField: typeof v['laneField'] === 'string' ? v['laneField'].slice(0, MAX_TITLE) : '',
    markers: v['markers'] === null || v['markers'] === undefined ? null : strings(v['markers']),
    groups: strings(v['groups']) ?? [],
    sync: v['sync'] !== false,
  };
}

/** Parses a saved list of configs, dropping malformed and duplicate entries and keeping at most `MAX_TIMELINES`. */
export function sanitizeConfigs(raw: unknown): TimelineConfig[] {
  const out: TimelineConfig[] = [];
  for (const item of Array.isArray(raw) ? raw : []) {
    const c = sanitizeConfig(item);
    if (c && !out.some((o) => o.id === c.id)) out.push(c);
    if (out.length >= MAX_TIMELINES) break;
  }
  return out;
}

/** The fields the lanes are formed by; `instance` (and a field mode without a field) follows the workspace's instances. */
export function laneGroupBy(cfg: Pick<TimelineConfig, 'lanes' | 'laneField'>, workspaceGroupBy: readonly string[]): string[] {
  switch (cfg.lanes) {
    case 'source':
      return ['@source'];
    case 'level':
      return ['@level'];
    case 'field':
      return cfg.laneField ? [cfg.laneField] : [...workspaceGroupBy];
    default:
      return [...workspaceGroupBy];
  }
}

const sameList = (a: readonly string[], b: readonly string[]) => a.length === b.length && a.every((x, i) => x === b[i]);

/**
 * The query an additional timeline asks the engine for. Pure: composes the request from the workspace query and
 * the panel's settings; filtering, grouping and bucketing all happen in the engine.
 *
 * - scope `global` keeps the workspace filter (levels, search, field filters, query-language filter); `own` uses
 *   only the panel's query-language expression.
 * - the lanes come from the panel's lane mode; the workspace's instance filter applies only while the panel
 *   groups the same way (its keys mean nothing under another grouping); the panel's own lane filter wins.
 * - markers: the panel's own selection, else the workspace's (scope `global`) or none.
 * - time: the workspace range or live window while synced, else the panel's own range (null = everything).
 */
export function instanceQuery(workspace: Query, workspaceGroupBy: readonly string[], cfg: TimelineConfig, ownRange: Range | null): Query {
  const groupBy = laneGroupBy(cfg, workspaceGroupBy);
  const global = cfg.scope === 'global';
  const base: Query = global ? { ...workspace } : { ...EMPTY_QUERY, ql: cfg.ql.trim() };
  const inheritedGroups = global && sameList(groupBy, workspaceGroupBy) ? workspace.groups : [];
  const time = cfg.sync
    ? { from: workspace.from ?? null, to: workspace.to ?? null, window: workspace.window ?? null }
    : { from: ownRange?.from ?? null, to: ownRange?.to ?? null, window: null };
  return {
    ...base,
    groupBy,
    groups: cfg.groups.length ? [...cfg.groups] : [...inheritedGroups],
    markers: cfg.markers ? [...cfg.markers] : global ? [...workspace.markers] : [],
    ...time,
  };
}
