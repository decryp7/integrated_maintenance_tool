import {
  ChartData,
  ChartDef,
  ChartRequest,
  ExtractPreview,
  Facet,
  FacetRequest,
  FieldFilter,
  FieldInfo,
  LEVELS,
  Lane,
  Level,
  LifecycleEvent,
  MarkerCluster,
  MarkerKind,
  Page,
  PageRequest,
  Query,
  RecordView,
  Timeline,
  TimelineRequest,
} from '../api.types';
import { parseBucket, valueToString } from '../format';
import { Condition, ExtractField, ExtractRule, ProfileModel, toJsRegex } from '../profile.model';
import { MockRecord, MockSource } from './mock-data';

/** In-memory query engine mirroring the Rust core's semantics, for browser development. */
export class MockEngine {
  records: MockRecord[] = [];
  sources: MockSource[] = [];
  derived = new Set<string>();
  markerKinds: MarkerKind[] = [];
  charts: ChartDef[] = [];
  defaultGroupBy: string[] = [];
  private sourceById = new Map<number, MockSource>();
  private cache: { key: string; ids: number[] } | null = null;

  /** Re-runs the profile pipeline (extractors + markers) over raw records. */
  rebuild(raw: MockRecord[], sources: MockSource[], profile: ProfileModel): void {
    this.sources = sources;
    this.sourceById = new Map(sources.map((s) => [s.id, s]));
    this.cache = null;
    this.derived.clear();

    const records = raw.map((r) => ({ ...r, fields: { ...r.fields }, markers: [] as string[] }));
    records.sort((a, b) => (a.ts ?? Number.MAX_SAFE_INTEGER) - (b.ts ?? Number.MAX_SAFE_INTEGER) || a.source - b.source || a.line - b.line);
    records.forEach((r, i) => (r.id = i));

    for (const rule of profile.extract ?? []) {
      const re = safeRegex(rule.regex);
      if (!re) continue;
      for (const name of Object.keys(rule.fields ?? {})) this.derived.add(name);
      const when = compileConditions(rule.when ?? []);
      for (const r of records) {
        if (!when((f) => this.fieldText(r, f))) continue;
        const text = this.fieldText(r, rule.from ?? '@message');
        if (text === null) continue;
        const m = re.exec(text);
        if (!m?.groups) continue;
        Object.assign(r.fields, convertCaptures(m.groups, rule));
      }
    }

    const markerRules = (profile.marker ?? []).map((m) => ({ kind: m.kind, when: compileConditions(m.when ?? []) }));
    for (const r of records) {
      for (const m of markerRules) if (m.when((f) => this.fieldText(r, f))) r.markers.push(m.kind);
    }

    this.markerKinds = (profile.marker ?? []).map((m) => ({
      kind: m.kind,
      label: m.label ?? m.kind,
      lifecycle: !!m.lifecycle,
      color: m.color ?? null,
    }));
    this.charts = (profile.chart ?? []).map((c) => ({
      id: c.id,
      title: c.title ?? c.id,
      y: c.y,
      bucket: c.bucket ?? 'auto',
      splitBy: c.split_by ?? [],
      type: c.type ?? 'line',
      unit: c.unit ?? '',
    }));
    this.records = records;
    this.defaultGroupBy = this.pickGroupBy(profile);
  }

  private pickGroupBy(profile: ProfileModel): string[] {
    const explicit = profile.instance?.key ?? [];
    if (explicit.length) return explicit;
    const candidates = profile.instance?.candidates ?? [];
    const chosen = candidates.filter((c) => {
      let present = 0;
      const distinct = new Set<string>();
      for (const r of this.records) {
        const v = r.fields[c];
        if (v !== undefined) {
          present++;
          if (distinct.size < 2) distinct.add(valueToString(v));
        }
      }
      // A field with a single value everywhere (e.g. EnvironmentName) does not distinguish instances.
      return present >= this.records.length / 2 && distinct.size > 1;
    });
    return chosen.length ? chosen : ['@source'];
  }

  fieldText(r: MockRecord, name: string): string | null {
    switch (name) {
      case '@level':
        return r.level;
      case '@message':
        return r.message;
      case '@exception':
        return r.exception;
      case '@template':
        return r.template;
      case '@source':
        return this.sourceById.get(r.source)?.name ?? null;
      case '@line':
        return String(r.line);
      default: {
        const v = r.fields[name];
        return v === undefined ? null : valueToString(v);
      }
    }
  }

  fields(): FieldInfo[] {
    const stats = new Map<string, { count: number; distinct: Set<string>; numeric: boolean }>();
    for (const r of this.records) {
      for (const [k, v] of Object.entries(r.fields)) {
        let s = stats.get(k);
        if (!s) stats.set(k, (s = { count: 0, distinct: new Set(), numeric: true }));
        s.count++;
        if (s.distinct.size < 10000) s.distinct.add(valueToString(v));
        if (typeof v !== 'number') s.numeric = false;
      }
    }
    return [...stats.entries()]
      .map(([name, s]) => ({ name, count: s.count, distinct: s.distinct.size, numeric: s.numeric, derived: this.derived.has(name) }))
      .sort((a, b) => b.count - a.count || a.name.localeCompare(b.name));
  }

  groupKey(r: MockRecord, groupBy: string[]): string {
    return groupBy.map((g) => this.fieldText(r, g) ?? '–').join(' · ');
  }

  private effectiveGroupBy(q: Query): string[] {
    return q.groupBy.length ? q.groupBy : this.defaultGroupBy;
  }

  compile(q: Query) {
    const groupBy = this.effectiveGroupBy(q);
    const groups = q.groups.length ? new Set(q.groups) : null;
    const levels = q.levels.length ? new Set<Level>(q.levels) : null;
    const markers = q.markers.length ? new Set(q.markers) : null;
    const filters = q.filters.map((f) => compileFilter(f));
    let text: ((s: string) => boolean) | null = null;
    if (q.text.trim()) {
      if (q.regex) {
        const re = safeRegex(q.text, q.caseSensitive ? '' : 'i');
        text = re ? (s) => re.test(s) : () => false;
      } else {
        const needle = q.caseSensitive ? q.text : q.text.toLowerCase();
        text = q.caseSensitive ? (s) => s.includes(needle) : (s) => s.toLowerCase().includes(needle);
      }
    }
    const from = q.from ?? null;
    const to = q.to ?? null;
    const structural = (r: MockRecord): boolean => {
      if (from !== null && (r.ts === null || r.ts < from)) return false;
      if (to !== null && (r.ts === null || r.ts > to)) return false;
      if (groups && !groups.has(this.groupKey(r, groupBy))) return false;
      for (const f of filters) if (!f((name) => this.fieldText(r, name))) return false;
      return true;
    };
    const content = (r: MockRecord): boolean => {
      if (levels && !levels.has(r.level)) return false;
      if (markers && !r.markers.some((m) => markers.has(m))) return false;
      if (text && !(text(r.message) || (r.exception !== null && text(r.exception)) || (r.template !== null && text(r.template)))) return false;
      return true;
    };
    return { groupBy, structural, content };
  }

  matching(q: Query): number[] {
    const key = JSON.stringify({ ...q, groupBy: this.effectiveGroupBy(q) });
    if (this.cache?.key === key) return this.cache.ids;
    const c = this.compile(q);
    const ids: number[] = [];
    for (const r of this.records) if (c.structural(r) && c.content(r)) ids.push(r.id);
    this.cache = { key, ids };
    return ids;
  }

  view(r: MockRecord, groupBy: string[]): RecordView {
    return {
      id: r.id,
      ts: r.ts,
      level: r.level,
      message: r.message,
      template: r.template,
      exception: r.exception,
      fields: r.fields,
      source: r.source,
      sourceName: this.sourceById.get(r.source)?.name ?? '',
      line: r.line,
      group: this.groupKey(r, groupBy),
      markers: r.markers,
    };
  }

  page(req: PageRequest): Page {
    const ids = this.matching(req.query);
    const groupBy = this.effectiveGroupBy(req.query);
    const total = ids.length;
    const at = (pos: number) => ids[req.descending ? total - 1 - pos : pos];
    let offset = req.offset;
    let anchorIndex: number | null = null;
    if (req.anchor !== null && req.anchor !== undefined) {
      const idx = binarySearch(ids, req.anchor);
      if (idx >= 0) {
        anchorIndex = req.descending ? total - 1 - idx : idx;
        offset = Math.max(0, anchorIndex - Math.floor(req.limit / 2));
      }
    }
    offset = Math.max(0, Math.min(offset, Math.max(0, total - 1)));
    const rows: RecordView[] = [];
    for (let p = offset; p < Math.min(total, offset + req.limit); p++) rows.push(this.view(this.records[at(p)], groupBy));
    return { total, offset, rows, anchorIndex };
  }

  private extent(pred: (r: MockRecord) => boolean): [number, number] | null {
    let lo = Infinity;
    let hi = -Infinity;
    for (const r of this.records) {
      if (r.ts === null || !pred(r)) continue;
      if (r.ts < lo) lo = r.ts;
      if (r.ts > hi) hi = r.ts;
    }
    return lo === Infinity ? null : [lo, hi];
  }

  timeline(req: TimelineRequest): Timeline {
    const c = this.compile(req.query);
    const ext =
      req.query.from != null && req.query.to != null ? ([req.query.from, req.query.to] as [number, number]) : this.extent(c.structural);
    if (!ext) return { from: 0, to: 1, bucketMs: 1, histogram: [], lanes: [], truncatedLanes: 0 };
    const [from, to] = [req.query.from ?? ext[0], req.query.to ?? ext[1]];
    const buckets = Math.max(10, Math.min(2000, Math.floor(req.buckets)));
    const span = Math.max(1, to - from);
    const bucketMs = span / buckets;
    const histogram = Array.from({ length: buckets }, () => [0, 0, 0, 0, 0, 0]);
    const lifecycle = new Set(this.markerKinds.filter((m) => m.lifecycle).map((m) => m.kind));
    const lanes = new Map<string, Lane & { clusters: Map<string, MarkerCluster> }>();

    for (const r of this.records) {
      if (r.ts === null || r.ts < from || r.ts > to || !c.structural(r)) continue;
      const key = this.groupKey(r, c.groupBy);
      let lane = lanes.get(key);
      if (!lane) {
        lane = { key, first: r.ts, last: r.ts, total: 0, matched: 0, density: new Array(buckets).fill(0), markers: [], clusters: new Map() };
        lanes.set(key, lane);
      }
      lane.total++;
      lane.last = r.ts;
      const b = Math.min(buckets - 1, Math.floor((r.ts - from) / bucketMs));
      const pass = c.content(r);
      if (pass) {
        lane.matched++;
        lane.density[b]++;
        histogram[b][LEVELS.indexOf(r.level)]++;
      }
      const kind = r.markers[0];
      if (kind && (pass || (req.alwaysShowLifecycle && lifecycle.has(kind)))) {
        const ck = `${kind}|${b}`;
        const cl = lane.clusters.get(ck);
        if (cl) cl.count++;
        else lane.clusters.set(ck, { kind, bucket: b, ts: r.ts, id: r.id, count: 1, message: r.message.slice(0, 200) });
      }
    }
    const all = [...lanes.values()].sort((a, b) => a.first - b.first);
    const MAX = 300;
    return {
      from,
      to,
      bucketMs,
      histogram,
      lanes: all.slice(0, MAX).map(({ clusters, ...lane }) => ({ ...lane, markers: [...clusters.values()] })),
      truncatedLanes: Math.max(0, all.length - MAX),
    };
  }

  facet(req: FacetRequest): Facet {
    const ids = this.matching(req.query);
    const counts = new Map<string, number>();
    for (const id of ids) {
      const v = this.fieldText(this.records[id], req.field);
      if (v === null) continue;
      counts.set(v, (counts.get(v) ?? 0) + 1);
    }
    const values = [...counts.entries()].map(([value, count]) => ({ value, count })).sort((a, b) => b.count - a.count);
    return { field: req.field, values: values.slice(0, req.limit), distinct: values.length, total: ids.length };
  }

  chart(req: ChartRequest): ChartData {
    const def = req.def ?? this.charts.find((c) => c.id === req.chartId);
    if (!def) throw new Error(`Unknown chart '${req.chartId}'`);
    const m = /^(count|avg|min|max|sum|last)\(\s*([^)]*)\s*\)$/.exec(def.y.trim());
    if (!m) throw new Error(`Invalid y expression '${def.y}'`);
    const agg = m[1];
    const field = m[2];
    const c = this.compile(req.query);
    const ids = this.matching(req.query);
    let from = req.query.from ?? null;
    let to = req.query.to ?? null;
    if (from === null || to === null) {
      const ext = this.extent(c.structural);
      if (!ext) return { def, from: 0, to: 1, bucketMs: 1, series: [], lifecycle: [] };
      from ??= ext[0];
      to ??= ext[1];
    }
    const span = Math.max(1, to - from);
    let bucketMs = parseBucket(def.bucket) ?? span / Math.max(10, Math.min(2000, req.buckets));
    if (span / bucketMs > 5000) bucketMs = span / 5000;
    const n = Math.max(1, Math.ceil(span / bucketMs));
    type Acc = { count: number; sum: number; min: number; max: number; last: number };
    const series = new Map<string, { acc: Acc[]; total: number }>();
    for (const id of ids) {
      const r = this.records[id];
      if (r.ts === null || r.ts < from || r.ts > to) continue;
      let value = 1;
      if (agg !== 'count') {
        const raw = r.fields[field];
        const num = typeof raw === 'number' ? raw : typeof raw === 'string' && raw.trim() !== '' ? Number(raw) : NaN;
        if (!Number.isFinite(num)) continue;
        value = num;
      }
      const key = def.splitBy.length ? def.splitBy.map((s) => this.fieldText(r, s) ?? '–').join(' · ') : def.y;
      let s = series.get(key);
      if (!s) series.set(key, (s = { acc: [], total: 0 }));
      s.total++;
      const b = Math.min(n - 1, Math.floor((r.ts - from) / bucketMs));
      const a = (s.acc[b] ??= { count: 0, sum: 0, min: Infinity, max: -Infinity, last: 0 });
      a.count++;
      a.sum += value;
      a.min = Math.min(a.min, value);
      a.max = Math.max(a.max, value);
      a.last = value;
    }
    const out = [...series.entries()]
      .sort((a, b) => b[1].total - a[1].total)
      .slice(0, 20)
      .map(([key, s]) => ({
        key,
        points: Array.from({ length: n }, (_, b): [number, number | null] => {
          const a = s.acc[b];
          const start = from! + b * bucketMs;
          if (!a) return [start, agg === 'count' ? 0 : null];
          const byAgg: Record<string, number> = { count: a.count, sum: a.sum, avg: a.sum / a.count, min: a.min, max: a.max, last: a.last };
          return [start, byAgg[agg]];
        }),
      }));
    const kinds = new Set(this.markerKinds.filter((k) => k.lifecycle || k.kind === 'fatal').map((k) => k.kind));
    const lifecycle: LifecycleEvent[] = [];
    for (const r of this.records) {
      if (r.ts === null || r.ts < from || r.ts > to || !r.markers.length || !kinds.has(r.markers[0]) || !c.structural(r)) continue;
      lifecycle.push({ kind: r.markers[0], ts: r.ts, group: this.groupKey(r, c.groupBy), id: r.id });
    }
    return { def, from, to, bucketMs, series: out, lifecycle };
  }

  previewExtractor(rule: ExtractRule, q: Query, limit: number): ExtractPreview {
    const re = safeRegex(rule.regex, 'd');
    if (!re) return { candidates: 0, matches: 0, failed: 0, samples: [], error: 'Invalid regular expression' };
    const when = compileConditions(rule.when ?? []);
    let candidates = 0;
    let matches = 0;
    const samples: ExtractPreview['samples'] = [];
    for (const id of this.matching(q)) {
      const r = this.records[id];
      if (!when((f) => this.fieldText(r, f))) continue;
      const text = this.fieldText(r, rule.from ?? '@message');
      if (text === null) continue;
      candidates++;
      const m = re.exec(text);
      if (!m) continue;
      matches++;
      if (samples.length < limit) {
        const spans = Object.entries(m.indices?.groups ?? {})
          .filter((e): e is [string, [number, number]] => !!e[1])
          .map(([name, [start, end]]) => ({ name, start, end }));
        samples.push({ recordId: r.id, text, spans, values: m.groups ? convertCaptures(m.groups, rule) : {} });
      }
    }
    return { candidates, matches, failed: candidates - matches, samples, error: null };
  }
}

export function safeRegex(pattern: string, flags = ''): RegExp | null {
  try {
    return toJsRegex(pattern, flags);
  } catch {
    return null;
  }
}

function binarySearch(xs: number[], v: number): number {
  let lo = 0;
  let hi = xs.length - 1;
  while (lo <= hi) {
    const mid = (lo + hi) >> 1;
    if (xs[mid] === v) return mid;
    if (xs[mid] < v) lo = mid + 1;
    else hi = mid - 1;
  }
  return -1;
}

type Lookup = (field: string) => string | null;

function compileFilter(f: FieldFilter): (get: Lookup) => boolean {
  return compileCondition({ field: f.field, op: f.op, value: f.value });
}

export function compileConditions(conds: Condition[]): (get: Lookup) => boolean {
  const compiled = conds.map(compileCondition);
  return (get) => compiled.every((c) => c(get));
}

function compileCondition(c: Condition): (get: Lookup) => boolean {
  const value = c.value ?? '';
  switch (c.op) {
    case 'eq':
      return (get) => get(c.field) === value;
    case 'ne':
      return (get) => get(c.field) !== value;
    case 'contains': {
      const n = value.toLowerCase();
      return (get) => (get(c.field) ?? '').toLowerCase().includes(n);
    }
    case 'notContains': {
      const n = value.toLowerCase();
      return (get) => !(get(c.field) ?? '').toLowerCase().includes(n);
    }
    case 'matches': {
      const re = safeRegex(value);
      return (get) => {
        const v = get(c.field);
        return v !== null && !!re && re.test(v);
      };
    }
    case 'exists':
      return (get) => get(c.field) !== null;
    case 'missing':
      return (get) => get(c.field) === null;
    case 'gt':
    case 'lt': {
      const limit = Number(value);
      return (get) => {
        const raw = get(c.field);
        if (raw === null || raw.trim() === '') return false;
        const n = Number(raw);
        return c.op === 'gt' ? n > limit : n < limit;
      };
    }
    case 'in': {
      const set = new Set(value.split(',').map((s) => s.trim()));
      return (get) => set.has(get(c.field) ?? '');
    }
    default:
      return () => true;
  }
}

const BYTE_UNITS: Record<string, number> = { B: 1, KB: 1024, MB: 1024 ** 2, GB: 1024 ** 3, TB: 1024 ** 4 };
const DURATION_UNITS: Record<string, number> = { ms: 1, s: 1000, m: 60000, min: 60000, h: 3600000 };

function convertCaptures(groups: Record<string, string | undefined>, rule: ExtractRule): Record<string, unknown> {
  const out: Record<string, unknown> = {};
  for (const [name, spec] of Object.entries(rule.fields ?? {})) {
    const raw = groups[name];
    if (raw === undefined) continue;
    out[name] = convertValue(raw, spec, groups);
  }
  return out;
}

function convertValue(raw: string, spec: ExtractField, groups: Record<string, string | undefined>): unknown {
  switch (spec.type) {
    case 'number':
      return Number(raw);
    case 'integer':
      return parseInt(raw, 10);
    case 'bool':
      return /^(true|yes|1|on)$/i.test(raw);
    case 'bytes': {
      const unit = (spec.unit_from ? groups[spec.unit_from] : spec.unit) ?? 'B';
      const bytes = Number(raw) * (BYTE_UNITS[unit.toUpperCase()] ?? 1);
      return +(bytes / (BYTE_UNITS[(spec.to ?? 'B').toUpperCase()] ?? 1)).toFixed(3);
    }
    case 'duration': {
      const ts = /^(?:(\d+)\.)?(\d{1,2}):(\d{2}):(\d{2}(?:\.\d+)?)$/.exec(raw);
      let ms: number;
      if (ts) ms = ((+(ts[1] ?? 0) * 24 + +ts[2]) * 60 + +ts[3]) * 60000 + +ts[4] * 1000;
      else ms = Number(raw) * (DURATION_UNITS[spec.unit ?? 'ms'] ?? 1);
      return spec.to === 's' ? ms / 1000 : ms;
    }
    default:
      return raw;
  }
}
