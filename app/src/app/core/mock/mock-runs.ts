/**
 * Plausible runs between lifecycle markers for the browser demo, shaped like the engine's `[[span]]` pairing
 * (crates/debrief-core/src/spans): per lane (or per field value within a lane), a start opens a run, a stop closes it,
 * a start while running ends the previous run as "no stop", a stop while idle is "no start", consecutive starts or
 * stops count as one, and a run still open is drawn to the group's last event. Not the reference for behaviour.
 */
import {
  AnnotationDto,
  FieldMatch,
  MarkerKind,
  Query,
  RUN_ANNOTATOR,
  RunEvent,
  RunsReply,
  RunsRequest,
  RunStatus,
  SpanCfgToml,
  SpanDraft,
  SpanIssue,
  SpanPreview,
  SpanSummary,
  SpanTrigger,
} from '../api.types';
import { ProfileModel, toJsRegex } from '../profile.model';
import { MockRecord } from './mock-data';

/** The rule the default profile ships (default_profile.toml). */
export const DEFAULT_RUN_RULE: SpanCfgToml = { kind: 'run', label: 'Running', start: 'start', stop: 'stop', per: 'instance' };

/** The profile's `[[span]]` rules; absent = the default rule (like the engine's merge over the defaults). */
export function spanRules(profile: ProfileModel): SpanCfgToml[] {
  return Array.isArray(profile.span) ? profile.span : [DEFAULT_RUN_RULE];
}

const list = (v: string | string[]): string[] => (Array.isArray(v) ? v : [v]);
const MAX_RUNS_PER_LANE = 1000;

/** `"12h"`, `"90m"`, `"30s"`, `"2d"`, `"500ms"` in ms; null when not a duration. */
export function mockDurationMs(text: string): number | null {
  const m = /^\s*(\d+(?:\.\d+)?)\s*(ms|s|m|h|d)?\s*$/i.exec(text);
  if (!m) return null;
  const f: Record<string, number> = { ms: 1, s: 1000, m: 60_000, h: 3_600_000, d: 86_400_000 };
  const ms = Number(m[1]) * (f[(m[2] ?? 'ms').toLowerCase()] ?? 1);
  return ms >= 1 ? ms : null;
}

type Edge = 'start' | 'stop' | 'other';

interface Group {
  open: RunEvent | null;
  last: Edge;
  lastTs: number;
  closed: number | null;
}

interface MockRun {
  lane: string;
  group: string | null;
  start: RunEvent | null;
  stop: RunEvent | null;
  from: number;
  to: number;
  status: RunStatus;
}

const event = (r: MockRecord): RunEvent => ({ id: r.id, ts: r.ts ?? 0, message: r.message.slice(0, 240) });

/** Pairs the records' edges into runs, by lane then start time. */
function pair(records: readonly MockRecord[], laneOf: (r: MockRecord) => string, valueOf: (r: MockRecord) => string | null, edgeOf: (r: MockRecord) => Edge, restart: boolean): MockRun[] {
  const groups = new Map<string, Group>();
  const runs: (MockRun & { seq: number })[] = [];
  const lanes: string[] = [];
  let seq = 0;
  for (const r of records) {
    if (r.ts === null) continue;
    const lane = laneOf(r);
    const value = valueOf(r);
    const key = `${lane}\u0000${value ?? ''}`;
    const edge = edgeOf(r);
    let g = groups.get(key);
    if (!g) {
      if (edge === 'other') continue;
      g = { open: null, last: 'other', lastTs: r.ts, closed: null };
      groups.set(key, g);
      if (!lanes.includes(lane)) lanes.push(lane);
    }
    const push = (run: MockRun) => runs.push({ ...run, seq: ++seq });
    let closedNow = false;
    let extended = false;
    if (edge === 'start') {
      if (g.open && g.last !== 'start' && restart) {
        push({ lane, group: value, start: g.open, stop: null, from: g.open.ts, to: Math.max(g.lastTs, g.open.ts), status: 'noStop' });
        g.open = event(r);
      } else if (!g.open) g.open = event(r);
    } else if (edge === 'stop') {
      if (g.open) {
        push({ lane, group: value, start: g.open, stop: event(r), from: g.open.ts, to: r.ts, status: 'closed' });
        g.open = null;
        closedNow = true;
      } else if (g.last === 'stop' && g.closed !== null) {
        const prev = runs.find((x) => x.seq === g!.closed);
        if (prev) {
          prev.stop = event(r);
          prev.to = Math.max(prev.to, r.ts);
        }
        extended = true;
      } else push({ lane, group: value, start: null, stop: event(r), from: r.ts, to: r.ts, status: 'noStart' });
    }
    if (closedNow) g.closed = seq;
    else if (!extended) g.closed = null;
    g.last = edge;
    g.lastTs = Math.max(g.lastTs, r.ts);
  }
  for (const [key, g] of groups) {
    if (!g.open) continue;
    const [lane = '', value = ''] = key.split('\u0000');
    runs.push({ lane, group: value || null, start: g.open, stop: null, from: g.open.ts, to: Math.max(g.lastTs, g.open.ts), status: 'open', seq: ++seq });
  }
  const out: MockRun[] = [];
  for (const lane of lanes) out.push(...runs.filter((r) => r.lane === lane).sort((a, b) => a.from - b.from).slice(-MAX_RUNS_PER_LANE));
  return out;
}

const emptyQuery = (extra: Partial<Query>): Query => ({ levels: [], text: '', regex: false, caseSensitive: false, filters: [], groupBy: [], groups: [], markers: [], ...extra });

function annotate(rule: SpanCfgToml, run: MockRun, groupBy: string[]): AnnotationDto {
  const label = rule.label || rule.kind;
  const max = rule.max_duration ? mockDurationMs(rule.max_duration) : null;
  const duration = Math.max(0, run.to - run.from);
  const tooLong = run.status !== 'noStart' && max !== null && duration > max;
  const field = rule.per?.startsWith('field:') ? rule.per.slice(6).trim() : null;
  const group: FieldMatch | null = field && run.group !== null ? { field, value: run.group } : null;
  const texts: Record<RunStatus, string> = { closed: label, open: `${label} (still running)`, noStop: `${label} (no stop seen)`, noStart: `${label}: stop without start` };
  return {
    annotator: RUN_ANNOTATOR,
    owner: 'profile',
    kind: rule.kind,
    from: run.from,
    to: Math.max(run.to, run.from),
    instance: run.lane,
    on: null,
    label: texts[run.status],
    severity: run.status === 'noStop' || run.status === 'noStart' || tooLong ? 'warning' : 'info',
    evidenceQuery: emptyQuery({ from: run.from, to: Math.max(run.to, run.from), groupBy, groups: [run.lane], filters: group ? [{ field: group.field, op: 'eq', value: group.value }] : [] }),
    run: { span: rule.kind, spanLabel: label, status: run.status, start: run.start, stop: run.stop, durationMs: run.status === 'noStart' ? 0 : duration, tooLong, group, color: rule.color ?? null },
  };
}

export interface RunContext {
  records: readonly MockRecord[];
  laneOf: (r: MockRecord) => string;
  groupBy: string[];
}

const fieldOf = (r: MockRecord, field: string): string | null => {
  const v = r.fields[field];
  return v === undefined || v === null || v === '' ? null : String(v);
};

/** Runs of one rule over `ctx`, with an explicit edge classifier (markers or patterns). */
function runsOf(rule: SpanCfgToml, ctx: RunContext, edgeOf: (r: MockRecord) => Edge): AnnotationDto[] {
  const field = rule.per?.startsWith('field:') ? rule.per.slice(6).trim() : null;
  const runs = pair(ctx.records, ctx.laneOf, (r) => (field ? fieldOf(r, field) : null), edgeOf, rule.restart_on_start !== false);
  return runs.map((run) => annotate(rule, run, ctx.groupBy));
}

function markerEdge(rule: SpanCfgToml): (r: MockRecord) => Edge {
  const start = new Set(list(rule.start));
  const stop = new Set(list(rule.stop));
  return (r) => (r.markers.some((m) => start.has(m)) ? 'start' : r.markers.some((m) => stop.has(m)) ? 'stop' : 'other');
}

/** Every enabled rule's runs. */
export function mockRunAnnotations(rules: readonly SpanCfgToml[], ctx: RunContext): AnnotationDto[][] {
  return rules.map((rule) => (rule.enabled === false ? [] : runsOf(rule, ctx, markerEdge(rule))));
}

export function mockSummary(rule: SpanCfgToml, anns: readonly AnnotationDto[]): SpanSummary {
  const s: SpanSummary = { kind: rule.kind, label: rule.label || rule.kind, color: rule.color ?? null, enabled: rule.enabled !== false, runs: 0, closed: 0, open: 0, noStop: 0, noStart: 0, tooLong: 0, lanes: 0, totalMs: 0, avgMs: 0, maxMs: 0, dropped: 0, missingMarkers: [] };
  const lanes = new Set<string | null>();
  for (const a of anns) {
    const r = a.run;
    if (!r) continue;
    s.runs++;
    s[r.status]++;
    if (r.tooLong) s.tooLong++;
    lanes.add(a.instance);
    if (r.status !== 'noStart') {
      s.totalMs += r.durationMs;
      s.maxMs = Math.max(s.maxMs, r.durationMs);
    }
  }
  s.lanes = lanes.size;
  const timed = s.runs - s.noStart;
  s.avgMs = timed ? Math.floor(s.totalMs / timed) : 0;
  return s;
}

const STATUS_RANK: Record<RunStatus, number> = { noStart: 0, noStop: 1, open: 2, closed: 3 };

/** `query_runs`. */
export function mockQueryRuns(rules: readonly SpanCfgToml[], perRule: readonly AnnotationDto[][], req: RunsRequest, markerKinds: readonly MarkerKind[]): RunsReply {
  const inst = new Set(req.instances ?? []);
  const sel = (a: AnnotationDto) => (req.from == null || a.to >= req.from) && (req.to == null || a.from <= req.to) && (!inst.size || inst.has(a.instance ?? ''));
  const kinds = new Set(markerKinds.map((k) => k.kind));
  const summaries: SpanSummary[] = [];
  const rows: AnnotationDto[] = [];
  rules.forEach((rule, i) => {
    const chosen = (perRule[i] ?? []).filter(sel);
    const s = mockSummary(rule, chosen);
    s.missingMarkers = [...list(rule.start), ...list(rule.stop)].filter((k) => !kinds.has(k));
    summaries.push(s);
    if (rule.enabled === false || (req.kind && req.kind !== rule.kind)) return;
    rows.push(...chosen.filter((a) => !req.statuses?.length || (a.run && req.statuses.includes(a.run.status))));
  });
  const stop = (a: AnnotationDto) => a.run?.stop?.ts ?? Number.MAX_SAFE_INTEGER;
  const by: Record<string, (a: AnnotationDto, b: AnnotationDto) => number> = {
    start: (a, b) => a.from - b.from,
    stop: (a, b) => stop(a) - stop(b),
    duration: (a, b) => (a.run?.durationMs ?? 0) - (b.run?.durationMs ?? 0),
    status: (a, b) => STATUS_RANK[a.run?.status ?? 'closed'] - STATUS_RANK[b.run?.status ?? 'closed'],
    lane: (a, b) => (a.instance ?? '').localeCompare(b.instance ?? ''),
  };
  const cmp = by[req.sort ?? 'start'] ?? by['start']!;
  rows.sort((a, b) => (req.descending ? -cmp(a, b) : cmp(a, b)) || a.from - b.from);
  const offset = Math.min(req.offset ?? 0, rows.length);
  const limit = Math.min(Math.max(1, req.limit ?? 200), 1000);
  return { summaries, rules: [...rules], runs: rows.slice(offset, offset + limit), total: rows.length, offset };
}

/** `preview_spans`: field errors, counts, the first runs and anomalies, and the profile changes saving writes. */
export function mockPreviewSpans(draft: SpanDraft, profile: ProfileModel, ctx: RunContext, markerKinds: readonly MarkerKind[]): SpanPreview {
  const errors: SpanIssue[] = [];
  const warnings: string[] = [];
  const kind = draft.kind.trim();
  const label = draft.label.trim();
  if (!/^[A-Za-z0-9_.-]{1,64}$/.test(kind)) errors.push({ field: 'kind', message: `kind "${kind}" must be 1–64 letters, digits, '-', '_' or '.' (e.g. "run")` });
  const known = new Set(markerKinds.map((k) => k.kind));
  const markers: (Record<string, unknown> & { kind: string })[] = [];
  const side = (t: SpanTrigger, field: 'start' | 'stop'): { kinds: string[]; test: ((r: MockRecord) => boolean) | null } => {
    if (t.type === 'markers') {
      const kinds = t.kinds.map((k) => k.trim()).filter(Boolean);
      if (!kinds.length) {
        errors.push({ field, message: `${field} needs at least one marker kind` });
        return { kinds, test: null };
      }
      const missing = kinds.filter((k) => !known.has(k));
      if (missing.length) {
        errors.push({ field, message: `marker "${missing.join('", "')}" is not defined (choose one from the list)` });
        return { kinds, test: null };
      }
      return { kinds, test: (r) => r.markers.some((m) => kinds.includes(m)) };
    }
    const markerKind = `${kind}-${field}`;
    if (!t.pattern.trim()) {
      errors.push({ field, message: `enter the text that marks a ${field}, or choose a marker` });
      return { kinds: [markerKind], test: null };
    }
    let re: RegExp;
    try {
      re = toJsRegex(t.pattern, t.caseSensitive ? '' : 'i');
    } catch (e) {
      errors.push({ field, message: `the ${field} pattern is not a valid regex: ${e instanceof Error ? e.message : String(e)}` });
      return { kinds: [markerKind], test: null };
    }
    const name = label || kind;
    markers.push({ kind: markerKind, label: `${name}: ${field}`, lifecycle: true, color: field === 'start' ? '#3FBF7F' : '#A7B0C2', description: `The ${field} of a "${name}" run (created by the Runs editor)`, when: [{ field: '@message', op: 'matches', value: t.caseSensitive ? t.pattern : `(?i)${t.pattern}` }] });
    return { kinds: [markerKind], test: (r) => re.test(r.message) };
  };
  const start = side(draft.start, 'start');
  const stop = side(draft.stop, 'stop');
  const per = draft.per.trim();
  if (!(per === 'instance' || /^field:\s*\S/.test(per))) errors.push({ field: 'per', message: `per "${per}" must be "instance" or "field:<Name>" (e.g. "field:ProcessId")` });
  if (draft.color && !/^#([0-9a-f]{3}|[0-9a-f]{6})$/i.test(draft.color)) errors.push({ field: 'color', message: `color "${draft.color}" must be #RGB or #RRGGBB` });
  if (draft.maxDuration && mockDurationMs(draft.maxDuration) === null) errors.push({ field: 'maxDuration', message: `max_duration: "${draft.maxDuration}" is not a duration (e.g. "60s", "5m", "1h")` });
  const rules = spanRules(profile);
  if (rules.some((r) => r.kind === kind)) warnings.push(`saving replaces the existing run rule "${kind}"`);
  for (const m of markers) if ((profile.marker ?? []).some((x) => x.kind === m.kind)) warnings.push(`saving updates the marker rule "${m.kind}"`);
  if (errors.length || !start.test || !stop.test) return { errors, warnings, summary: null, runs: [], anomalies: [], plan: null };

  const one = (k: string[]): string | string[] => (k.length === 1 ? k[0]! : k);
  const span: SpanCfgToml = { kind, ...(label ? { label } : {}), start: one(start.kinds), stop: one(stop.kinds), per, ...(draft.color ? { color: draft.color } : {}), ...(draft.maxDuration ? { max_duration: draft.maxDuration } : {}), ...(draft.restartOnStart === false ? { restart_on_start: false } : {}) };
  const s = start.test;
  const t = stop.test;
  const all = runsOf(span, ctx, (r) => (s(r) ? 'start' : t(r) ? 'stop' : 'other')).sort((a, b) => a.from - b.from);
  const spans = rules.some((r) => r.kind === kind) ? rules.map((r) => (r.kind === kind ? span : r)) : [...rules, span];
  return {
    errors,
    warnings,
    summary: mockSummary(span, all),
    runs: all.slice(0, 20),
    anomalies: all.filter((a) => a.run && (a.run.tooLong || a.run.status === 'noStop' || a.run.status === 'noStart')).slice(0, 20),
    plan: { span, spans, markers },
  };
}
