import { MockBackend } from './mock-backend';
import { MockRecord } from './mock-data';
import { DEFAULT_RUN_RULE, mockDurationMs, mockPreviewSpans, mockQueryRuns, mockRunAnnotations, RunContext, spanRules } from './mock-runs';
import { markerKind, tinyBackend } from '../../../testing/helpers';
import { at } from '../../../testing/must';

let id = 0;
function rec(ts: number, lane: string, message: string, markers: string[] = [], fields: Record<string, unknown> = {}): MockRecord {
  return { id: id++, ts, level: 'Information', message, template: null, exception: null, fields: { Lane: lane, ...fields }, source: 0, line: id, markers };
}

const ctx = (records: MockRecord[]): RunContext => ({ records, laneOf: (r) => String(r.fields['Lane']), groupBy: ['Lane'] });
const statuses = (records: MockRecord[], rule = DEFAULT_RUN_RULE) => at(mockRunAnnotations([rule], ctx(records)), 0).map((a) => [a.instance, a.run?.status, a.from, a.to]);

describe('mock runs', () => {
  it('pairs per lane like the engine: closed, open, no stop, no start, consecutive starts and stops', () => {
    const records = [
      rec(0, 'A', 'stopped', ['stop']),
      rec(1, 'A', 'work'),
      rec(2, 'A', 'starting', ['start']),
      rec(3, 'A', 'started', ['start']),
      rec(4, 'B', 'started', ['start']),
      rec(5, 'A', 'work'),
      rec(6, 'A', 'started', ['start']),
      rec(7, 'A', 'stopping', ['stop']),
      rec(8, 'A', 'stopped', ['stop']),
      rec(9, 'B', 'work'),
      { ...rec(10, 'B', 'untimed'), ts: null },
    ];
    expect(statuses(records)).toEqual([
      ['A', 'noStart', 0, 0],
      ['A', 'noStop', 2, 5],
      ['A', 'closed', 6, 8],
      ['B', 'open', 4, 9],
    ]);
    // Without restarts the extra start belongs to the running run.
    expect(statuses(records.slice(2, 9), { ...DEFAULT_RUN_RULE, restart_on_start: false }).filter((r) => r[0] === 'A')).toEqual([['A', 'closed', 2, 8]]);
  });

  it('pairs per field value, flags long runs, and skips switched-off rules', () => {
    const records = [rec(0, 'A', 's', ['start'], { Pid: 1 }), rec(1, 'A', 's', ['start'], { Pid: 2 }), rec(5, 'A', 't', ['stop'], { Pid: 1 }), rec(9, 'A', 't', ['stop'], { Pid: 2 })];
    const rule = { ...DEFAULT_RUN_RULE, per: 'field:Pid', max_duration: '5ms' };
    const runs = at(mockRunAnnotations([rule], ctx(records)), 0);
    expect(runs.map((a) => [a.run?.group?.value, a.run?.tooLong, a.severity])).toEqual([
      ['1', false, 'info'],
      ['2', true, 'warning'],
    ]);
    expect(runs[0]?.evidenceQuery.filters).toEqual([{ field: 'Pid', op: 'eq', value: '1' }]);
    expect(mockRunAnnotations([{ ...rule, enabled: false }], ctx(records))).toEqual([[]]);
  });

  it('parses durations and falls back to the default rule', () => {
    expect([mockDurationMs('12h'), mockDurationMs('90m'), mockDurationMs('500'), mockDurationMs('2d'), mockDurationMs('soon'), mockDurationMs('0')]).toEqual([43_200_000, 5_400_000, 500, 172_800_000, null, null]);
    expect(spanRules({})).toEqual([DEFAULT_RUN_RULE]);
    expect(spanRules({ span: [] })).toEqual([]);
  });

  it('sorts, filters, pages and summarises like query_runs', () => {
    const records = [rec(0, 'A', 's', ['start']), rec(10, 'A', 't', ['stop']), rec(2, 'B', 's', ['start']), rec(3, 'B', 't', ['stop']), rec(20, 'B', 's', ['start']), rec(25, 'B', 'w')];
    records.sort((a, b) => (a.ts ?? 0) - (b.ts ?? 0));
    const rules = [DEFAULT_RUN_RULE, { kind: 'x', start: 'nope', stop: 'stop', enabled: false }];
    const per = mockRunAnnotations(rules, ctx(records));
    const kinds = [markerKind({ kind: 'start' }), markerKind({ kind: 'stop' })];
    const all = mockQueryRuns(rules, per, {}, kinds);
    expect(all.total).toBe(3);
    expect(all.summaries.map((s) => [s.kind, s.runs, s.closed, s.open, s.lanes, s.maxMs, s.avgMs, s.enabled])).toEqual([
      ['run', 3, 2, 1, 2, 10, 5, true],
      ['x', 0, 0, 0, 0, 0, 0, false],
    ]);
    expect(all.summaries[1]?.missingMarkers).toEqual(['nope']);
    const dur = (r: ReturnType<typeof mockQueryRuns>) => r.runs.map((a) => a.run?.durationMs);
    expect(dur(mockQueryRuns(rules, per, { sort: 'duration', descending: true }, kinds))).toEqual([10, 5, 1]);
    expect(dur(mockQueryRuns(rules, per, { sort: 'status' }, kinds))).toEqual([5, 10, 1]);
    expect(dur(mockQueryRuns(rules, per, { sort: 'stop' }, kinds))).toEqual([1, 10, 5]);
    expect(dur(mockQueryRuns(rules, per, { sort: 'lane' }, kinds))).toEqual([10, 1, 5]);
    expect(dur(mockQueryRuns(rules, per, { statuses: ['open'], instances: ['B'], from: 15, to: 30 }, kinds))).toEqual([5]);
    expect(mockQueryRuns(rules, per, { kind: 'other' }, kinds).runs).toEqual([]);
    const page = mockQueryRuns(rules, per, { offset: 1, limit: 1 }, kinds);
    expect([page.offset, page.runs.length, page.total]).toEqual([1, 1, 3]);
  });

  it('previews drafts with field errors, warnings and a plan', () => {
    const records = [rec(0, 'A', 'worker up'), rec(5, 'A', 'worker down'), rec(6, 'A', 'worker up'), rec(7, 'A', 'w')];
    const kinds = [markerKind({ kind: 'start' }), markerKind({ kind: 'stop' })];
    const p = mockPreviewSpans({ kind: 'w', label: 'Worker', start: { type: 'pattern', pattern: 'up' }, stop: { type: 'pattern', pattern: 'DOWN' }, per: 'instance', maxDuration: '1ms', restartOnStart: false, color: '#abc' }, { marker: [{ kind: 'w-start' }] }, ctx(records), kinds);
    expect(p.errors).toEqual([]);
    expect(p.warnings.join()).toContain('w-start');
    expect(p.summary?.runs).toBe(2);
    expect(p.anomalies).toHaveLength(1);
    expect(p.plan?.span).toEqual({ kind: 'w', label: 'Worker', start: 'w-start', stop: 'w-stop', per: 'instance', color: '#abc', max_duration: '1ms', restart_on_start: false });
    expect(p.plan?.spans.map((s) => s.kind)).toEqual(['run', 'w']);
    expect(p.plan?.markers.map((m) => m['kind'])).toEqual(['w-start', 'w-stop']);
    const replaced = mockPreviewSpans({ kind: 'run', label: '', start: { type: 'markers', kinds: ['start'] }, stop: { type: 'markers', kinds: ['stop'] }, per: 'instance' }, {}, ctx(records), kinds);
    expect(replaced.warnings.join()).toContain('replaces');
    expect(replaced.plan?.spans).toHaveLength(1);

    const bad = mockPreviewSpans({ kind: 'a b', label: '', start: { type: 'markers', kinds: [] }, stop: { type: 'pattern', pattern: '(' }, per: 'machine', maxDuration: 'soon', color: 'red' }, {}, ctx(records), kinds);
    expect(bad.errors.map((e) => e.field)).toEqual(['kind', 'start', 'stop', 'per', 'color', 'maxDuration']);
    expect(bad.plan).toBeNull();
    const missing = mockPreviewSpans({ kind: 'a', label: '', start: { type: 'markers', kinds: ['nope'] }, stop: { type: 'pattern', pattern: ' ' }, per: 'field:X' }, {}, ctx(records), kinds);
    expect(missing.errors.map((e) => e.field)).toEqual(['start', 'stop']);
  });

  it('is served by the mock backend, also inside the annotations', async () => {
    const b: MockBackend = tinyBackend();
    await b.workspaceSummary();
    const reply = await b.queryRuns({});
    expect(reply.total).toBeGreaterThan(0);
    expect(reply.rules).toEqual([DEFAULT_RUN_RULE]);
    const anns = await b.annotations({});
    expect(anns.filter((a) => a.run).length).toBe(reply.total);
    expect((await b.annotations({ from: 0, to: 1 })).filter((a) => a.run)).toEqual([]);
    const p = await b.previewSpans({ draft: { kind: 'r', label: 'R', start: { type: 'markers', kinds: ['start'] }, stop: { type: 'markers', kinds: ['stop'] }, per: 'instance' } });
    expect(p.summary?.runs).toBe(reply.total);
    const failing = new MockBackend({ latency: false, fail: { queryRuns: 1, previewSpans: 1 } });
    await expect(failing.queryRuns({})).rejects.toBeTruthy();
    await expect(failing.previewSpans({ draft: { kind: 'r', label: '', start: { type: 'markers', kinds: [] }, stop: { type: 'markers', kinds: [] }, per: 'instance' } })).rejects.toBeTruthy();
  });
});
