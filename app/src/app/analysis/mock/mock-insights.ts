import { LEVELS, Level, RecordView } from '../../core/api.types';
import {
  ActivityItem,
  Anomaly,
  Comparison,
  ExceptionGroup,
  FieldShift,
  FieldStats,
  MetricWindow,
  MomentSummary,
  NumberSummary,
  NumericShift,
  Pattern,
  QlQuery,
  RequestSpan,
  SideStats,
  TraceSummary,
  TracesResult,
  ValueAppearance,
  UserCount,
  ValueCount,
  WaterfallRow,
} from '../../core/api.types';
import { fieldValue, mockQlFilter } from '../../core/mock/mock-ql';

/** Plausible analysis results computed from the demo records. Mock only: the engine is the reference. */

const MIN = 60_000;

export function filterRecords(records: RecordView[], q: QlQuery): RecordView[] {
  const out = records.filter((r) => {
    if (q.from != null && (r.ts === null || r.ts < q.from)) return false;
    if (q.to != null && (r.ts === null || r.ts > q.to)) return false;
    if (q.levels?.length && !q.levels.includes(r.level)) return false;
    if (q.text && !`${r.message} ${r.exception ?? ''}`.toLowerCase().includes(q.text.toLowerCase())) return false;
    if (q.groups?.length && !q.groups.includes(r.group)) return false;
    return (q.filters ?? []).every((f) => {
      const v = fieldValue(r, f.field);
      const s = v === undefined || v === null ? '' : String(v);
      switch (f.op) {
        case 'eq': return s === f.value;
        case 'ne': return s !== f.value;
        case 'contains': return s.toLowerCase().includes((f.value ?? '').toLowerCase());
        case 'notContains': return !s.toLowerCase().includes((f.value ?? '').toLowerCase());
        case 'exists': return s !== '';
        case 'missing': return s === '';
        case 'gt': return Number(s) > Number(f.value);
        case 'lt': return Number(s) < Number(f.value);
        default: return true;
      }
    });
  });
  const ql = mockQlFilter(q.ql ?? '');
  return ql ? out.filter(ql) : out;
}

const NUMBER = /\d+(\.\d+)?/g;

export function mockPatterns(records: RecordView[]): Pattern[] {
  const map = new Map<string, Pattern>();
  for (const r of records) {
    const template = r.template ?? r.message.replace(NUMBER, '<*>');
    let p = map.get(template);
    if (!p) {
      p = { id: map.size, template, count: 0, levels: [0, 0, 0, 0, 0, 0], firstTs: null, lastTs: null, sampleIds: [], source: r.template ? 'Serilog' : 'Mined' };
      map.set(template, p);
    }
    p.count++;
    p.levels[LEVELS.indexOf(r.level)]++;
    if (r.ts !== null) {
      p.firstTs = p.firstTs === null ? r.ts : Math.min(p.firstTs, r.ts);
      p.lastTs = p.lastTs === null ? r.ts : Math.max(p.lastTs, r.ts);
    }
    if (p.sampleIds.length < 5) p.sampleIds.push(r.id);
  }
  return [...map.values()].sort((a, b) => b.count - a.count);
}

export function mockExceptions(records: RecordView[]): ExceptionGroup[] {
  const map = new Map<string, ExceptionGroup & { by: Map<string, number> }>();
  for (const r of records) {
    if (!r.exception) continue;
    const lines = r.exception.split('\n');
    const m = /^([\w.]+(?:Exception|Error)):\s*(.*)$/.exec(lines[0]);
    const typeName = m?.[1] ?? lines[0].split(':')[0];
    const message = m?.[2] ?? '';
    const topFrame = lines.find((l) => l.trim().startsWith('at '))?.trim().slice(3).split(' in ')[0] ?? null;
    const key = `${typeName}|${topFrame}`;
    let g = map.get(key);
    if (!g) {
      g = { fingerprint: key, typeName, message, topFrame, count: 0, instances: [], firstTs: null, lastTs: null, sampleIds: [], by: new Map() };
      map.set(key, g);
    }
    g.count++;
    g.by.set(r.group, (g.by.get(r.group) ?? 0) + 1);
    if (r.ts !== null) {
      g.firstTs = g.firstTs === null ? r.ts : Math.min(g.firstTs, r.ts);
      g.lastTs = g.lastTs === null ? r.ts : Math.max(g.lastTs, r.ts);
    }
    if (g.sampleIds.length < 5) g.sampleIds.push(r.id);
  }
  return [...map.values()]
    .map(({ by, ...g }) => ({ ...g, instances: [...by.entries()].sort((a, b) => b[1] - a[1]) as [string, number][] }))
    .sort((a, b) => b.count - a.count);
}

const median = (xs: number[]): number => {
  const s = [...xs].sort((a, b) => a - b);
  return s.length ? s[Math.floor(s.length / 2)] : 0;
};

export function mockAnomalies(records: RecordView[]): Anomaly[] {
  const out: Anomaly[] = [];
  const groups = new Map<string, RecordView[]>();
  for (const r of records) if (r.ts !== null) (groups.get(r.group) ?? groups.set(r.group, []).get(r.group)!).push(r);
  for (const [group, rs] of groups) {
    const fatal = rs.find((r) => r.level === 'Fatal');
    if (fatal) out.push({ kind: 'crash', instance: group, from: fatal.ts!, to: fatal.ts!, score: 10, message: `${group} terminated: ${fatal.message}`, sampleIds: [fatal.id] });
    const buckets = new Map<number, RecordView[]>();
    for (const r of rs) if (r.level === 'Error' || r.level === 'Fatal') (buckets.get(Math.floor(r.ts! / (5 * MIN))) ?? buckets.set(Math.floor(r.ts! / (5 * MIN)), []).get(Math.floor(r.ts! / (5 * MIN)))!).push(r);
    const base = median([...buckets.values()].map((b) => b.length));
    for (const [b, es] of buckets) {
      if (es.length >= 3 && es.length >= 2 * Math.max(1, base)) {
        out.push({ kind: 'errorSpike', instance: group, from: b * 5 * MIN, to: (b + 1) * 5 * MIN, score: +(es.length / Math.max(1, base)).toFixed(1), message: `${es.length} errors in 5 min (usual ${Math.max(1, base)})`, sampleIds: es.slice(0, 5).map((r) => r.id) });
      }
    }
    const slow = rs.filter((r) => Number(r.fields['ElapsedMs']) >= 5000);
    if (slow.length >= 3) out.push({ kind: 'slowOutlier', instance: group, from: slow[0].ts!, to: slow[slow.length - 1].ts!, score: 4.2, message: `${slow.length} requests took 5 s or more`, sampleIds: slow.slice(0, 5).map((r) => r.id) });
  }
  return out.sort((a, b) => b.score - a.score);
}

const sideStats = (rs: RecordView[]): SideStats => {
  const ts = rs.map((r) => r.ts).filter((t): t is number => t !== null);
  const from = ts.length ? Math.min(...ts) : null;
  const to = ts.length ? Math.max(...ts) : null;
  const durationMs = from !== null && to !== null ? to - from : 0;
  const mins = Math.max(durationMs / MIN, 1 / 60);
  return {
    count: rs.length,
    from,
    to,
    durationMs,
    perMinute: +(rs.length / mins).toFixed(2),
    levels: LEVELS.map((level) => {
      const count = rs.filter((r) => r.level === level).length;
      return { level, count, perMinute: +(count / mins).toFixed(2) };
    }).filter((l) => l.count > 0),
  };
};

const counts = (rs: RecordView[], field: string): Map<string, number> => {
  const m = new Map<string, number>();
  for (const r of rs) {
    const v = fieldValue(r, field);
    if (v !== undefined && v !== null) m.set(String(v), (m.get(String(v)) ?? 0) + 1);
  }
  return m;
};

const numbers = (rs: RecordView[], field: string): number[] =>
  rs.map((r) => r.fields[field]).filter((v): v is number => typeof v === 'number').sort((a, b) => a - b);

const pct = (sorted: number[], p: number) => (sorted.length ? sorted[Math.min(sorted.length - 1, Math.floor((p / 100) * sorted.length))] : 0);

export function mockCompare(a: RecordView[], b: RecordView[]): Comparison {
  const sa = sideStats(a);
  const sb = sideStats(b);
  const fieldNames = ['@level', 'MachineName', 'SourceContext', 'StatusCode', 'Outcome'];
  const changedFields: FieldShift[] = [];
  const newValues: ValueAppearance[] = [];
  const goneValues: ValueAppearance[] = [];
  for (const f of fieldNames) {
    const ca = counts(a, f);
    const cb = counts(b, f);
    const ta = [...ca.values()].reduce((s, x) => s + x, 0);
    const tb = [...cb.values()].reduce((s, x) => s + x, 0);
    if (!ta && !tb) continue;
    const values = new Set([...ca.keys(), ...cb.keys()]);
    const top = [...values].map((value) => {
      const countA = ca.get(value) ?? 0;
      const countB = cb.get(value) ?? 0;
      return { value, countA, countB, shareA: ta ? countA / ta : 0, shareB: tb ? countB / tb : 0 };
    });
    const distance = top.reduce((s, v) => s + Math.abs(v.shareA - v.shareB), 0) / 2;
    if (distance >= 0.05) changedFields.push({ field: f, distance: +distance.toFixed(3), top: top.sort((x, y) => Math.abs(y.shareB - y.shareA) - Math.abs(x.shareB - x.shareA)).slice(0, 6) });
    const added: ValueCount[] = [...cb].filter(([v]) => !ca.has(v)).map(([value, count]) => ({ value, count }));
    const gone: ValueCount[] = [...ca].filter(([v]) => !cb.has(v)).map(([value, count]) => ({ value, count }));
    if (added.length && f !== '@level') newValues.push({ field: f, values: added.slice(0, 6) });
    if (gone.length && f !== '@level') goneValues.push({ field: f, values: gone.slice(0, 6) });
  }
  const numericChanges: NumericShift[] = [];
  for (const f of ['ElapsedMs', 'Elapsed', 'Cpu']) {
    const na = numbers(a, f);
    const nb = numbers(b, f);
    if (!na.length || !nb.length) continue;
    const sum = (xs: number[]): NumberSummary => ({ count: xs.length, median: pct(xs, 50), p95: pct(xs, 95) });
    const x = sum(na);
    const y = sum(nb);
    const ch = (p: number, q: number) => (p ? +(((q - p) / p) * 100).toFixed(1) : null);
    numericChanges.push({ field: f, a: x, b: y, medianChangePct: ch(x.median, y.median), p95ChangePct: ch(x.p95, y.p95) });
  }
  const summary = [`Window B has ${sb.count} events vs ${sa.count} in window A.`];
  const errA = sa.levels.find((l) => l.level === 'Error')?.count ?? 0;
  const errB = sb.levels.find((l) => l.level === 'Error')?.count ?? 0;
  if (errA !== errB) summary.push(`Errors changed from ${errA} to ${errB}.`);
  for (const n of newValues.slice(0, 2)) summary.push(`New ${n.field} values: ${n.values.map((v) => v.value).join(', ')}.`);
  return { a: sa, b: sb, changedFields, numericChanges, newValues, goneValues, summary };
}

export function mockFieldStats(records: RecordView[], field: string, buckets: number): FieldStats {
  const values = records.map((r) => fieldValue(r, field)).filter((v) => v !== undefined && v !== null);
  const missing = records.length - values.length;
  const c = new Map<string, number>();
  for (const v of values) c.set(String(v), (c.get(String(v)) ?? 0) + 1);
  const top = [...c].map(([value, count]) => ({ value, count })).sort((a, b) => b.count - a.count).slice(0, 10);
  const nums = values.filter((v): v is number => typeof v === 'number').sort((a, b) => a - b);
  let numeric: FieldStats['numeric'] = null;
  if (nums.length && nums.length === values.length) {
    const min = nums[0];
    const max = nums[nums.length - 1];
    const mean = nums.reduce((s, x) => s + x, 0) / nums.length;
    const width = (max - min) / Math.max(1, buckets) || 1;
    const histogram = Array.from({ length: Math.max(1, buckets) }, (_, i) => ({ from: min + i * width, to: min + (i + 1) * width, count: 0 }));
    for (const n of nums) histogram[Math.min(histogram.length - 1, Math.floor((n - min) / width))].count++;
    numeric = {
      min, max, mean,
      stddev: Math.sqrt(nums.reduce((s, x) => s + (x - mean) ** 2, 0) / nums.length),
      p50: pct(nums, 50), p90: pct(nums, 90), p95: pct(nums, 95), p99: pct(nums, 99),
      histogram,
    };
  }
  return { count: values.length, missing, distinct: c.size, numeric, top };
}

export function mockCorrelate(records: RecordView[], field: string, value: string): number[] {
  return records.filter((r) => String(fieldValue(r, field)) === value).map((r) => r.id);
}

export function mockTraces(records: RecordView[], traceId: string): TracesResult {
  const traces = new Map<string, RecordView[]>();
  for (const r of records) {
    const id = r.fields['CorrelationId'];
    if (typeof id === 'string' && r.ts !== null) (traces.get(id) ?? traces.set(id, []).get(id)!).push(r);
  }
  const summaries: TraceSummary[] = [...traces].map(([id, rs]) => {
    const spans = new Set(rs.map((r) => r.group)).size;
    const start = Math.min(...rs.map((r) => r.ts!));
    const end = Math.max(...rs.map((r) => r.ts! + (Number(r.fields['ElapsedMs']) || 0)));
    return { traceId: id, spans, errors: rs.filter((r) => r.level === 'Error' || r.level === 'Fatal').length, start, durationMs: end - start, root: rs[0].fields['RequestPath'] ? String(rs[0].fields['RequestPath']) : rs[0].group };
  });
  summaries.sort((a, b) => b.durationMs - a.durationMs);
  let waterfall: WaterfallRow[] = [];
  const rs = traceId ? traces.get(traceId) : undefined;
  if (rs) {
    const start = Math.min(...rs.map((r) => r.ts!));
    const byGroup = new Map<string, RecordView[]>();
    for (const r of rs) (byGroup.get(r.group) ?? byGroup.set(r.group, []).get(r.group)!).push(r);
    waterfall = [...byGroup].sort((a, b) => a[1][0].ts! - b[1][0].ts!).map(([group, items], i, all) => {
      const s = items[0].ts!;
      const e = Math.max(...items.map((r) => r.ts! + (Number(r.fields['ElapsedMs']) || 0)));
      return {
        spanId: group, parent: i === 0 ? null : all[0][0], depth: i === 0 ? 0 : 1, name: `${group} · ${items[0].message.slice(0, 40)}`,
        startOffsetMs: s - start, durationMs: Math.max(1, e - s), errors: items.filter((r) => r.level === 'Error' || r.level === 'Fatal').length,
      };
    });
  }
  return { traces: summaries.slice(0, 200), waterfall };
}

export function mockRequests(records: RecordView[], idField: string): RequestSpan[] {
  const names = idField.split('|').map((n) => n.trim()).filter(Boolean);
  const map = new Map<string, RequestSpan>();
  for (const r of records) {
    if (r.ts === null) continue;
    const id = names.map((n) => r.fields[n]).find((v) => v !== undefined && v !== null);
    if (id === undefined) continue;
    const key = String(id);
    const s = map.get(key) ?? { id: key, first: r.ts, last: r.ts, events: 0, errors: 0, instance: r.group };
    s.first = Math.min(s.first, r.ts);
    s.last = Math.max(s.last, r.ts);
    s.events++;
    if (r.level === 'Error' || r.level === 'Fatal') s.errors++;
    map.set(key, s);
  }
  return [...map.values()].sort((a, b) => b.last - b.first - (a.last - a.first)).slice(0, 500);
}

const avg = (xs: number[]) => (xs.length ? xs.reduce((s, x) => s + x, 0) / xs.length : 0);

export function mockMoment(all: RecordView[], base: QlQuery, from: number, to: number): MomentSummary {
  const filtered = filterRecords(all, { ...base, from: null, to: null });
  const inWin = filtered.filter((r) => r.ts !== null && r.ts >= from && r.ts <= to);
  const before = filtered.filter((r) => r.ts !== null && r.ts >= from - (to - from) && r.ts < from);
  const levels: Partial<Record<Level, number>> = {};
  for (const r of inWin) levels[r.level] = (levels[r.level] ?? 0) + 1;
  const groups = new Map<string, RecordView[]>();
  for (const r of inWin) (groups.get(r.group) ?? groups.set(r.group, []).get(r.group)!).push(r);
  const instances = [...groups].map(([key, rs]) => {
    const firstError = rs.find((r) => r.level === 'Error' || r.level === 'Fatal');
    return {
      key,
      events: rs.length,
      warnings: rs.filter((r) => r.level === 'Warning').length,
      errors: rs.filter((r) => r.level === 'Error' || r.level === 'Fatal').length,
      patterns: mockPatterns(rs).slice(0, 3),
      firstError: firstError?.id ?? null,
    };
  });
  const metrics: MetricWindow[] = [];
  for (const [series, rs] of groups) {
    for (const metric of ['ElapsedMs', 'Cpu', 'Elapsed']) {
      const vals = numbers(rs, metric);
      if (!vals.length) continue;
      const prev = numbers(before.filter((r) => r.group === series), metric);
      const a = avg(vals);
      const p = prev.length ? avg(prev) : null;
      metrics.push({
        series, metric, samples: vals.length, min: vals[0], avg: +a.toFixed(2), max: vals[vals.length - 1],
        last: Number(rs.filter((r) => typeof r.fields[metric] === 'number').pop()?.fields[metric]),
        previousAvg: p === null ? null : +p.toFixed(2), changePct: p ? +(((a - p) / p) * 100).toFixed(1) : null,
      });
    }
  }
  const userOf = (r: RecordView): { field: string; value: string } | null => {
    for (const field of ['CustomerId', 'UserName']) {
      const v = r.fields[field];
      if (v !== undefined && v !== null) return { field, value: String(v) };
    }
    return null;
  };
  const userCounts = new Map<string, UserCount>();
  const activity: ActivityItem[] = [];
  for (const r of inWin) {
    const u = userOf(r);
    if (u === null) continue;
    const key = `${u.field}\u0000${u.value}`;
    const c = userCounts.get(key) ?? userCounts.set(key, { ...u, count: 0 }).get(key)!;
    c.count++;
    if (activity.length < 50) activity.push({ id: r.id, ts: r.ts!, instance: r.group, user: u.value, action: r.message, level: r.level });
  }
  const users = [...userCounts.values()].sort((a, b) => b.count - a.count).slice(0, 12);
  const anomalies = mockAnomalies(filtered).filter((a) => a.to >= from && a.from <= to);
  return { from, to, total: inWin.length, levels, instances, metrics, users, activity, anomalies };
}
