import { TestBed } from '@angular/core/testing';
import { RecordView } from '../../core/api.types';
import { AnalysisBackend, TauriAnalysisBackend } from '../../core/analysis-backend';
import { QlQuery } from '../../core/api.types';
import { EMPTY_QUERY } from '../../core/workspace.store';
import { DEMO_START } from '../../core/mock/mock-data';
import { allRecords, provideAnalysisTesting } from '../test-setup';
import { filterRecords, mockCompare, mockFieldStats, mockMoment, mockRequests, mockTraces } from './mock-insights';

const q = (patch: Partial<QlQuery> = {}): QlQuery => ({ ...EMPTY_QUERY, ...patch });
const HOUR = 3_600_000;

describe('MockAnalysisBackend (demo data)', () => {
  let backend: AnalysisBackend;
  let all: RecordView[];

  beforeEach(async () => {
    TestBed.configureTestingModule({ providers: provideAnalysisTesting() });
    backend = TestBed.inject(AnalysisBackend);
    all = await allRecords();
  });

  it('runs and checks queries', async () => {
    const events = await backend.runQuery(q({ ql: 'level >= error' }));
    expect('result' in events && events.result.kind === 'events' && events.result.ids.length).toBeGreaterThan(0);
    const table = await backend.runQuery(q({ ql: '| stats count() by level' }));
    expect('result' in table && table.result.kind).toBe('table');
    expect(await backend.runQuery(q({ ql: '| nope' }))).toEqual({ error: expect.objectContaining({ message: "unknown stage 'nope'" }) });
    expect(await backend.runQuery(q())).toMatchObject({ result: { kind: 'events' } });
    expect(await backend.checkQuery('level >= error')).toBeNull();
    expect(await backend.checkQuery('| nope')).not.toBeNull();
    const c = await backend.completeQuery('lev', 3);
    expect(c.map((x) => x.label)).toContain('level');
  });

  it('applies the structured query before ql', async () => {
    const out = await backend.runQuery(q({ levels: ['Fatal'], ql: '| stats count()' }));
    expect('result' in out && out.result.kind === 'table' && out.result.rows[0][0]).toBe(all.filter((r) => r.level === 'Fatal').length);
    expect(filterRecords(all, q({ text: 'timed out', from: DEMO_START, to: DEMO_START + HOUR * 4 })).length).toBeGreaterThan(0);
    expect(filterRecords(all, q({ ql: 'level = fatal' })).every((r) => r.level === 'Fatal')).toBe(true);
    expect(filterRecords(all, q({ ql: '| stats count()' })).length).toBe(all.length);
    const host = all.find((r) => r.group)?.group ?? '';
    expect(filterRecords(all, q({ groups: [host] })).every((r) => r.group === host)).toBe(true);
    expect(filterRecords(all, q({ from: Number.MAX_SAFE_INTEGER - 1 }))).toEqual([]);
    expect(filterRecords(all, q({ to: 0 }))).toEqual([]);
    const ops = ['eq', 'ne', 'contains', 'notContains', 'exists', 'missing', 'gt', 'lt', 'matches'] as const;
    for (const op of ops) expect(() => filterRecords(all, q({ filters: [{ field: 'ElapsedMs', op, value: '3000' }] }))).not.toThrow();
    expect(filterRecords(all, q({ filters: [{ field: 'ElapsedMs', op: 'exists' }] })).every((r) => r.fields['ElapsedMs'] !== undefined)).toBe(true);
    expect(filterRecords(all, q({ filters: [{ field: 'ElapsedMs', op: 'gt', value: '3000' }] })).every((r) => Number(r.fields['ElapsedMs']) > 3000)).toBe(true);
  });

  it('resolves a live window against the newest record, like the engine', async () => {
    const newest = Math.max(...all.map((r) => r.ts ?? -Infinity));
    const lastHour = all.filter((r) => r.ts !== null && r.ts >= newest - HOUR);
    expect(lastHour.length).toBeGreaterThan(0);
    expect(lastHour.length).toBeLessThan(all.length);
    const windowed = q({ window: { kind: 'live', lastMs: HOUR }, ql: '| stats count()' });
    const out = await backend.runQuery(windowed);
    expect('result' in out && out.result.kind === 'table' && out.result.rows[0][0]).toBe(lastHour.length);
    // Every analysis sees the same window.
    const fieldStats = await backend.fieldStats(q({ window: { kind: 'live', lastMs: HOUR } }), 'level', 10);
    const unwindowed = await backend.fieldStats(q(), 'level', 10);
    expect(fieldStats.count).toBe(lastHour.length);
    expect(unwindowed.count).toBe(all.length);
    // A later `from` is kept.
    const later = await backend.runQuery(q({ window: { kind: 'live', lastMs: HOUR }, from: newest - 60_000, ql: '| stats count()' }));
    expect('result' in later && later.result.kind === 'table' && later.result.rows[0][0]).toBe(all.filter((r) => r.ts !== null && r.ts >= newest - 60_000).length);
    const moment = await backend.queryMoment(q({ window: { kind: 'live', lastMs: HOUR } }), newest - HOUR, newest);
    expect(moment).toBeTruthy();
  });

  it('mines patterns, exceptions and anomalies', async () => {
    const patterns = await backend.queryPatterns(q());
    expect(patterns.length).toBeGreaterThan(3);
    expect(patterns[0].count).toBeGreaterThanOrEqual(patterns[1].count);
    expect(patterns.reduce((s, p) => s + p.count, 0)).toBe(all.length);
    expect(patterns[0].levels).toHaveLength(6);
    expect(patterns.some((p) => p.source === 'Mined')).toBe(true);

    const groups = await backend.queryExceptions(q());
    expect(groups.length).toBeGreaterThan(0);
    expect(groups[0]).toMatchObject({ typeName: expect.stringContaining('Exception'), count: expect.any(Number) });
    expect(groups[0].instances[0]).toEqual([expect.any(String), expect.any(Number)]);

    const anomalies = await backend.queryAnomalies(q());
    expect(anomalies.length).toBeGreaterThan(0);
    expect(anomalies.map((a) => a.kind)).toContain('crash');
    expect(anomalies[0].score).toBeGreaterThanOrEqual(anomalies[anomalies.length - 1].score);
    expect(anomalies.every((a) => a.sampleIds.length > 0)).toBe(true);
  });

  it('compares two windows', async () => {
    const mid = DEMO_START + 100 * 60_000;
    const c = await backend.compareWindows(q({ from: mid - 30 * 60_000, to: mid }), q({ from: mid, to: mid + 30 * 60_000 }));
    expect(c.a.count).toBeGreaterThan(0);
    expect(c.b.count).toBeGreaterThan(0);
    expect(c.summary[0]).toContain('Window B has');
    expect(c.a.levels[0]).toMatchObject({ level: expect.any(String), count: expect.any(Number) });
    const incident = await backend.compareWindows(q({ from: DEMO_START, to: DEMO_START + 20 * 60_000 }), q({ from: DEMO_START + 95 * 60_000, to: DEMO_START + 115 * 60_000 }));
    expect(incident.changedFields.length + incident.numericChanges.length + incident.newValues.length).toBeGreaterThan(0);
    expect(mockCompare([], []).summary[0]).toContain('0 events');
    expect(mockCompare([], []).a.durationMs).toBe(0);
  });

  it('computes field statistics', async () => {
    const numeric = await backend.fieldStats(q(), 'ElapsedMs', 10);
    expect(numeric.numeric?.histogram).toHaveLength(10);
    expect(numeric.numeric?.histogram.reduce((s, b) => s + b.count, 0)).toBe(numeric.count);
    expect(numeric.numeric!.p99).toBeGreaterThanOrEqual(numeric.numeric!.p50);
    const text = await backend.fieldStats(q(), 'MachineName', 5);
    expect(text.numeric).toBeNull();
    expect(text.top[0].count).toBeGreaterThanOrEqual(text.top[text.top.length - 1].count);
    const none = await backend.fieldStats(q(), 'NoSuchField', 5);
    expect(none).toMatchObject({ count: 0, distinct: 0, numeric: null, top: [] });
    const flat = mockFieldStats([{ ...all[0], fields: { n: 5 } }, { ...all[1], fields: { n: 5 } }], 'n', 4);
    expect(flat.numeric?.histogram.reduce((s, b) => s + b.count, 0)).toBe(2);
  });

  it('correlates, traces and requests', async () => {
    const cid = all.find((r) => typeof r.fields['CorrelationId'] === 'string')!.fields['CorrelationId'] as string;
    const ids = await backend.correlate('CorrelationId', cid);
    expect(ids.length).toBeGreaterThan(0);
    expect(await backend.correlate('CorrelationId', 'nope')).toEqual([]);

    const list = await backend.queryTraces(q(), '');
    expect(list.traces.length).toBeGreaterThan(0);
    expect(list.waterfall).toEqual([]);
    expect(list.traces[0].durationMs).toBeGreaterThanOrEqual(list.traces[list.traces.length - 1].durationMs);
    const multi = list.traces.find((t) => t.spans > 1) ?? list.traces[0];
    const one = await backend.queryTraces(q(), multi.traceId);
    expect(one.waterfall.length).toBe(multi.spans);
    expect(one.waterfall[0]).toMatchObject({ depth: 0, parent: null, startOffsetMs: 0 });
    expect((await backend.queryTraces(q(), 'unknown')).waterfall).toEqual([]);

    const requests = await backend.queryRequests(q(), 'RequestId|CorrelationId');
    expect(requests.length).toBeGreaterThan(0);
    expect(requests[0].last - requests[0].first).toBeGreaterThanOrEqual(requests[requests.length - 1].last - requests[requests.length - 1].first);
    expect(await backend.queryRequests(q(), 'NoSuch')).toEqual([]);
    expect(mockRequests(all.map((r) => ({ ...r, ts: null })), 'CorrelationId')).toEqual([]);
    expect(mockTraces([], '')).toEqual({ traces: [], waterfall: [] });
  });

  it('summarises a moment', async () => {
    const from = DEMO_START + 100 * 60_000;
    const m = await backend.queryMoment(q(), from, from + 15 * 60_000);
    expect(m.total).toBeGreaterThan(0);
    expect(Object.values(m.levels).reduce((s, n) => s + (n ?? 0), 0)).toBe(m.total);
    expect(m.instances.length).toBeGreaterThan(0);
    expect(m.instances[0].patterns.length).toBeGreaterThan(0);
    expect(m.instances.some((i) => i.firstError !== null)).toBe(true);
    expect(m.metrics.length).toBeGreaterThan(0);
    expect(m.metrics.some((x) => x.previousAvg !== null && x.changePct !== null)).toBe(true);
    expect(m.users.length).toBeGreaterThan(0);
    expect(m.activity[0]).toMatchObject({ user: expect.any(String), action: expect.any(String) });
    expect(m.anomalies.every((a) => a.to >= m.from && a.from <= m.to)).toBe(true);
    const quiet = mockMoment(all, q(), 0, 1);
    expect(quiet).toMatchObject({ total: 0, instances: [], metrics: [], users: [], activity: [] });
  });
});

describe('TauriAnalysisBackend', () => {
  it('maps every method to its command and request shape', async () => {
    const calls: [string, unknown][] = [];
    const t = new TauriAnalysisBackend((async (cmd: string, args?: unknown) => (calls.push([cmd, args]), cmd === 'check_query' ? null : [])) as never);
    const query = q({ ql: 'x' });
    await t.runQuery(query);
    await t.checkQuery('x');
    await t.completeQuery('x', 1);
    await t.queryPatterns(query);
    await t.queryExceptions(query);
    await t.queryAnomalies(query);
    await t.compareWindows(query, query);
    await t.fieldStats(query, 'f', 7);
    await t.correlate('f', 'v');
    await t.queryTraces(query, 't');
    await t.queryRequests(query, 'id');
    await t.queryMoment(query, 1, 2);
    expect(calls).toEqual([
      ['run_query', { request: { query } }],
      ['check_query', { text: 'x' }],
      ['complete_query', { request: { text: 'x', cursor: 1 } }],
      ['query_patterns', { request: { query } }],
      ['query_exceptions', { request: { query } }],
      ['query_anomalies', { request: { query } }],
      ['compare_windows', { request: { a: query, b: query } }],
      ['field_stats', { request: { query, field: 'f', buckets: 7 } }],
      ['correlate', { request: { field: 'f', value: 'v' } }],
      ['query_traces', { request: { query, traceId: 't' } }],
      ['query_requests', { request: { query, idField: 'id' } }],
      ['query_moment', { request: { query, from: 1, to: 2 } }],
    ]);
  });
});
