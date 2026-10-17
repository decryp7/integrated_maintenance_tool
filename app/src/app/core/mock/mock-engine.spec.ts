import { parse } from 'smol-toml';
import { query, SEC, T0, tinyDataset } from '../../../testing/helpers';
import { ProfileModel } from '../profile.model';
import { compileConditions, MockEngine, safeRegex } from './mock-engine';
import { MOCK_PROFILE_TOML } from './mock-profile';

function engine(profile: ProfileModel = parse(MOCK_PROFILE_TOML) as ProfileModel): MockEngine {
  const ds = tinyDataset();
  const e = new MockEngine();
  e.rebuild(ds.records, ds.sources, profile);
  return e;
}

describe('MockEngine', () => {
  describe('query language (Query.ql)', () => {
    it('filters page, facet, timeline and chart like the other content filters', () => {
      const e = engine();
      const all = e.page({ query: query(), offset: 0, limit: 100, descending: false });
      const errs = e.page({ query: { ...query(), ql: 'level >= error' }, offset: 0, limit: 100, descending: false });
      expect(errs.total).toBeGreaterThan(0);
      expect(errs.total).toBeLessThan(all.total);
      expect(errs.rows.every((r) => r.level === 'Error' || r.level === 'Fatal')).toBe(true);
      expect(e.facet({ query: { ...query(), ql: 'level >= error' }, field: '@level', limit: 10 }).values.map((v) => v.value).sort()).toEqual(
        [...new Set(errs.rows.map((r) => r.level))].sort(),
      );
      const tl = e.timeline({ query: { ...query(), ql: 'level >= error' }, buckets: 10, alwaysShowLifecycle: false });
      expect(tl.lanes.reduce((n, l) => n + l.matched, 0)).toBe(errs.rows.filter((r) => r.ts !== null).length);
      const chart = e.chart({ query: { ...query(), ql: 'level >= error' }, chartId: 'events-by-level', def: null, buckets: 10 });
      expect(chart.series.map((s) => s.key).every((k) => k === 'Error' || k === 'Fatal')).toBe(true);
    });

    it('reads the source name and ignores a blank query', () => {
      const e = engine();
      const name = e.sources[0].name;
      const bySource = e.page({ query: { ...query(), ql: `@source = "${name}"` }, offset: 0, limit: 100, descending: false });
      expect(bySource.total).toBeGreaterThan(0);
      expect(bySource.rows.every((r) => r.sourceName === name)).toBe(true);
      const total = e.page({ query: query(), offset: 0, limit: 1, descending: false }).total;
      expect(e.page({ query: { ...query(), ql: '  ' }, offset: 0, limit: 1, descending: false }).total).toBe(total);
    });

    it('fails like the engine when the query does not parse', () => {
      expect(() => engine().page({ query: { ...query(), ql: 'level >= | nope' }, offset: 0, limit: 1, descending: false })).toThrow(/^query: /);
    });
  });

  describe('rebuild', () => {
    it('sorts by time with untimed records last and assigns ids by position', () => {
      const e = engine();
      expect(e.records.map((r) => r.id)).toEqual(e.records.map((_, i) => i));
      expect(e.records.at(-1)!.ts).toBeNull();
      for (let i = 1; i < e.records.length - 1; i++) expect(e.records[i].ts!).toBeGreaterThanOrEqual(e.records[i - 1].ts!);
    });

    it('applies extractors with unit conversion', () => {
      const e = engine();
      const stats = e.records.filter((r) => r.fields['SourceContext'] === 'Diagnostics.ProcessMonitor');
      expect(stats.map((r) => r.fields['cpu'])).toEqual([10.5, 20]);
      expect(stats.map((r) => r.fields['ws'])).toEqual([100, 1536]);
      expect(stats.map((r) => r.fields['threads'])).toEqual([20, 30]);
      expect(e.derived.has('cpu')).toBe(true);
    });

    it('assigns markers in profile order', () => {
      const e = engine();
      const fatal = e.records.find((r) => r.level === 'Fatal')!;
      expect(fatal.markers[0]).toBe('fatal');
      expect(fatal.markers).toContain('exception');
      expect(e.records.find((r) => r.message.startsWith('Application starting — Svc 1.0'))!.markers).toEqual(['start']);
      expect(e.records.find((r) => r.message.startsWith('Application is shutting'))!.markers).toEqual(['stop']);
      expect(e.markerKinds.map((k) => k.kind)).toEqual(['fatal', 'start', 'stop', 'exception', 'error', 'warning']);
      expect(e.markerKinds.find((k) => k.kind === 'start')!.lifecycle).toBe(true);
    });

    it('maps profile charts to chart definitions', () => {
      const e = engine();
      expect(e.charts.map((c) => c.id)).toEqual(['events-by-level', 'cpu', 'working-set']);
      expect(e.charts[1]).toMatchObject({ y: 'avg(cpu)', splitBy: ['MachineName', 'ProcessId'], type: 'line', unit: '%' });
    });

    it('picks distinguishing instance candidates and falls back to the source', () => {
      expect(engine().defaultGroupBy).toEqual(['MachineName', 'ProcessId']);
      expect(engine({ instance: { candidates: ['Nope'] } }).defaultGroupBy).toEqual(['@source']);
      expect(engine({ instance: { key: ['Application'] } }).defaultGroupBy).toEqual(['Application']);
    });

    it('skips extractors with invalid regexes and fills defaults', () => {
      const e = engine({ extract: [{ name: 'bad', regex: '(' }], marker: [{ kind: 'k' }], chart: [{ id: 'c', y: 'count()' }] });
      expect(e.derived.size).toBe(0);
      expect(e.markerKinds[0]).toEqual({ kind: 'k', owner: 'profile', role: null, label: 'k', lifecycle: false, color: null, icon: null, badge: null, shape: null, description: null, when: [] });
      expect(e.charts[0]).toMatchObject({ title: 'c', bucket: 'auto', splitBy: [], type: 'line', unit: '' });
    });
  });

  it('reads pseudo-fields', () => {
    const e = engine();
    const r = e.records.find((x) => x.exception)!;
    expect(e.fieldText(r, '@level')).toBe(r.level);
    expect(e.fieldText(r, '@message')).toBe(r.message);
    expect(e.fieldText(r, '@exception')).toBe(r.exception);
    expect(e.fieldText(r, '@template')).toBe(r.template);
    expect(e.fieldText(r, '@source')).toBe('app.clef');
    expect(e.fieldText(r, '@line')).toBe(String(r.line));
    expect(e.fieldText(r, 'Missing')).toBeNull();
    expect(e.fieldText(r, 'ProcessId')).toBe('100');
  });

  it('summarises fields', () => {
    const fields = engine().fields();
    const pid = fields.find((f) => f.name === 'ProcessId')!;
    expect(pid).toMatchObject({ count: 11, distinct: 3, numeric: true, derived: false });
    expect(fields.find((f) => f.name === 'MachineName')!.numeric).toBe(false);
    expect(fields.find((f) => f.name === 'cpu')!.derived).toBe(true);
  });

  describe('filtering', () => {
    const e = engine();
    const count = (patch: Parameters<typeof query>[0]) => e.matching(query(patch)).length;

    it('matches everything by default and caches', () => {
      const q = query();
      expect(e.matching(q)).toBe(e.matching(q));
      expect(count({})).toBe(11);
    });

    it('filters by level, text, regex and case', () => {
      expect(count({ levels: ['Error', 'Fatal'] })).toBe(2);
      expect(count({ text: 'TIMEOUTEXCEPTION' })).toBe(2);
      expect(count({ text: 'TIMEOUTEXCEPTION', caseSensitive: true })).toBe(0);
      expect(count({ text: '^Application (starting|is)', regex: true })).toBe(4);
      expect(count({ text: '(', regex: true })).toBe(0);
      expect(count({ text: 'Application starting', regex: false, caseSensitive: true })).toBe(3);
      expect(count({ text: '{Version}' })).toBe(1);
    });

    it('filters by time range, excluding untimed records', () => {
      expect(count({ from: T0, to: T0 + 60 * SEC })).toBe(3);
      expect(count({ from: T0 + 450 * SEC })).toBe(1);
      expect(count({ to: T0 + 10 * SEC })).toBe(1);
    });

    it('filters by fields with every operator', () => {
      expect(count({ filters: [{ field: 'MachineName', op: 'eq', value: 'WEB02' }] })).toBe(4);
      expect(count({ filters: [{ field: 'MachineName', op: 'ne', value: 'WEB02' }] })).toBe(7);
      expect(count({ filters: [{ field: '@message', op: 'contains', value: 'PROCESS' }] })).toBe(2);
      expect(count({ filters: [{ field: '@message', op: 'notContains', value: 'process' }] })).toBe(9);
      expect(count({ filters: [{ field: '@message', op: 'matches', value: '^Cache' }] })).toBe(1);
      expect(count({ filters: [{ field: 'CorrelationId', op: 'exists' }] })).toBe(2);
      expect(count({ filters: [{ field: 'CorrelationId', op: 'missing' }] })).toBe(9);
      expect(count({ filters: [{ field: 'ElapsedMs', op: 'gt', value: '1000' }] })).toBe(2);
      expect(count({ filters: [{ field: 'ElapsedMs', op: 'lt', value: '1000' }] })).toBe(1);
    });

    it('filters by instance group and markers', () => {
      expect(count({ groups: ['WEB01 · 100'] })).toBe(5);
      expect(count({ groupBy: ['MachineName'], groups: ['WEB02'] })).toBe(4);
      expect(count({ markers: ['start', 'stop'] })).toBe(5);
    });
  });

  describe('paging', () => {
    const e = engine();

    it('returns pages with group keys and source names', () => {
      const p = e.page({ query: query(), offset: 2, limit: 3, descending: false, anchor: null });
      expect(p.total).toBe(11);
      expect(p.offset).toBe(2);
      expect(p.rows.map((r) => r.id)).toEqual([2, 3, 4]);
      expect(p.rows[0].group).toBe('WEB01 · 100');
      expect(p.rows[0].sourceName).toBe('app.clef');
      expect(p.anchorIndex).toBeNull();
    });

    it('pages in descending order', () => {
      const p = e.page({ query: query(), offset: 0, limit: 2, descending: true, anchor: null });
      expect(p.rows.map((r) => r.id)).toEqual([10, 9]);
    });

    it('centres the page on an anchor', () => {
      const p = e.page({ query: query(), offset: 0, limit: 4, descending: false, anchor: 6 });
      expect(p.anchorIndex).toBe(6);
      expect(p.offset).toBe(4);
      const d = e.page({ query: query(), offset: 0, limit: 1, descending: true, anchor: 6 });
      expect(d.anchorIndex).toBe(4);
      expect(d.rows[0].id).toBe(6);
    });

    it('reports a filtered-out anchor as null and clamps offsets', () => {
      const p = e.page({ query: query({ levels: ['Fatal'] }), offset: 50, limit: 5, descending: false, anchor: 0 });
      expect(p.anchorIndex).toBeNull();
      expect(p.offset).toBe(0);
      expect(p.rows).toHaveLength(1);
    });
  });

  describe('timeline', () => {
    const e = engine();

    it('builds lanes, histogram and markers', () => {
      const t = e.timeline({ query: query(), buckets: 10, alwaysShowLifecycle: true });
      expect(t.from).toBe(T0);
      expect(t.to).toBe(T0 + 500 * SEC);
      expect(t.bucketMs).toBe(50 * SEC);
      expect(t.histogram).toHaveLength(10);
      expect(t.histogram.flat().reduce((a, b) => a + b, 0)).toBe(10);
      expect(t.lanes.map((l) => l.key)).toEqual(['WEB01 · 100', 'WEB02 · 300', 'WEB01 · 200']);
      const web01 = t.lanes[0];
      expect(web01).toMatchObject({ first: T0, last: T0 + 240 * SEC, total: 5, matched: 5 });
      expect(web01.markers.map((m) => m.kind).sort()).toEqual(['exception', 'fatal', 'start', 'warning']);
      expect(t.truncatedLanes).toBe(0);
    });

    it('keeps lifecycle markers for non-matching events only when asked', () => {
      const q = query({ levels: ['Error'] });
      const withLc = e.timeline({ query: q, buckets: 10, alwaysShowLifecycle: true });
      expect(withLc.lanes[0].matched).toBe(1);
      expect(withLc.lanes[0].markers.map((m) => m.kind).sort()).toEqual(['exception', 'start']);
      const without = e.timeline({ query: q, buckets: 10, alwaysShowLifecycle: false });
      expect(without.lanes[0].markers.map((m) => m.kind)).toEqual(['exception']);
    });

    it('clusters markers of the same kind in a bucket', () => {
      const t = e.timeline({ query: query({ groupBy: ['Application'] }), buckets: 10, alwaysShowLifecycle: true });
      const starts = t.lanes[0].markers.filter((m) => m.kind === 'start');
      expect(starts.find((m) => m.bucket === 0)!.count).toBe(2);
    });

    it('honours an explicit range and clamps buckets', () => {
      const t = e.timeline({ query: query({ from: T0, to: T0 + 100 * SEC }), buckets: 3, alwaysShowLifecycle: false });
      expect(t.histogram).toHaveLength(10);
      expect(t.to).toBe(T0 + 100 * SEC);
    });

    it('returns an empty timeline when nothing has a timestamp', () => {
      const t = e.timeline({ query: query({ filters: [{ field: 'MachineName', op: 'eq', value: 'none' }] }), buckets: 10, alwaysShowLifecycle: true });
      expect(t.lanes).toEqual([]);
      expect(t.histogram).toEqual([]);
    });
  });

  it('computes facets over the filtered result', () => {
    const f = engine().facet({ query: query({ levels: ['Information'] }), field: 'MachineName', limit: 1 });
    expect(f.values).toEqual([{ value: 'WEB01', count: 4 }]);
    expect(f.distinct).toBe(2);
    expect(f.total).toBe(7);
  });

  describe('charts', () => {
    const e = engine();

    it('aggregates by bucket and split', () => {
      const d = e.chart({ query: query(), chartId: 'cpu', buckets: 10 });
      expect(d.bucketMs).toBe(60 * SEC);
      expect(d.series.map((s) => s.key).sort()).toEqual(['WEB01 · 100', 'WEB01 · 200']);
      const s100 = d.series.find((s) => s.key === 'WEB01 · 100')!;
      expect(s100.points[1]).toEqual([T0 + 60 * SEC, 10.5]);
      expect(s100.points[0][1]).toBeNull();
      expect(d.lifecycle.map((l) => l.kind)).toEqual(['start', 'start', 'fatal', 'start', 'stop']);
    });

    it('supports every aggregate and ad-hoc definitions', () => {
      const def = (y: string, splitBy: string[] = []) => ({ id: 'x', title: 'x', y, bucket: '10m', splitBy, type: 'bar' as const, unit: '' });
      const value = (y: string) => e.chart({ query: query(), def: def(y), buckets: 10 }).series[0].points[0][1];
      expect(value('count()')).toBe(10);
      expect(value('sum(ElapsedMs)')).toBe(6502);
      expect(value('min(ElapsedMs)')).toBe(2);
      expect(value('max(ElapsedMs)')).toBe(5000);
      expect(value('last(ElapsedMs)')).toBe(2);
      expect(value('avg(ElapsedMs)')).toBeCloseTo(6502 / 3);
      const empty = e.chart({ query: query(), def: { ...def('count()', ['MachineName']), bucket: '1m' }, buckets: 10 });
      expect(empty.series[0].points.some(([, v]) => v === 0)).toBe(true);
    });

    it('rejects unknown charts and bad expressions', () => {
      expect(() => e.chart({ query: query(), chartId: 'nope', buckets: 10 })).toThrow(/Unknown chart/);
      expect(() => e.chart({ query: query(), def: { id: 'x', title: '', y: 'median(x)', bucket: 'auto', splitBy: [], type: 'line', unit: '' }, buckets: 10 })).toThrow(/Invalid y/);
    });

    it('returns empty data when nothing matches and caps tiny buckets', () => {
      const none = e.chart({ query: query({ filters: [{ field: 'MachineName', op: 'eq', value: 'none' }] }), chartId: 'cpu', buckets: 10 });
      expect(none.series).toEqual([]);
      const fine = e.chart({ query: query(), def: { id: 'x', title: '', y: 'count()', bucket: '1ms', splitBy: [], type: 'line', unit: '' }, buckets: 10 });
      expect(fine.series[0].points.length).toBeLessThanOrEqual(5000);
    });
  });

  describe('extractor preview', () => {
    const e = engine();
    const rule = (parse(MOCK_PROFILE_TOML) as ProfileModel).extract![0];

    it('reports matches with capture spans and converted values', () => {
      const p = e.previewExtractor(rule, query(), 1);
      expect(p).toMatchObject({ candidates: 2, matches: 2, failed: 0, error: null });
      expect(p.samples).toHaveLength(1);
      const cpu = p.samples[0].spans.find((s) => s.name === 'cpu')!;
      expect(p.samples[0].text.slice(cpu.start, cpu.end)).toBe('10.5');
      expect(p.samples[0].values).toMatchObject({ cpu: 10.5, ws: 100, threads: 20 });
    });

    it('counts failures and reports invalid patterns', () => {
      const p = e.previewExtractor({ ...rule, regex: 'Threads (?P<threads>\\d+) never', when: [] }, query(), 5);
      expect(p.failed).toBe(p.candidates);
      expect(e.previewExtractor({ ...rule, regex: '(' }, query(), 5).error).toMatch(/Invalid/);
      expect(e.previewExtractor({ ...rule, from: 'Missing' }, query(), 5).candidates).toBe(0);
    });
  });
});

describe('conditions and conversions', () => {
  const get = (v: Record<string, string | null>) => (f: string) => v[f] ?? null;

  it('evaluates in and unknown operators', () => {
    expect(compileConditions([{ field: 'a', op: 'in', value: 'x, y' }])(get({ a: 'y' }))).toBe(true);
    expect(compileConditions([{ field: 'a', op: 'in', value: 'x' }])(get({}))).toBe(false);
    expect(compileConditions([{ field: 'a', op: 'bogus' as never }])(get({}))).toBe(true);
    expect(compileConditions([])(get({}))).toBe(true);
    expect(compileConditions([{ field: 'a', op: 'matches', value: '(' }])(get({ a: 'x' }))).toBe(false);
  });

  it('converts every extracted field type', () => {
    const ds = tinyDataset();
    ds.records = [{ ...ds.records[0], message: 'v=1.5 on=yes d=00:00:01.5 n=7 b=2 KB s=hi t=3' }];
    const e = new MockEngine();
    e.rebuild(ds.records, ds.sources, {
      extract: [
        {
          name: 'all',
          regex: 'v=(?P<v>\\S+) on=(?P<on>\\S+) d=(?P<d>\\S+) n=(?P<n>\\d+) b=(?P<b>\\d+) KB s=(?P<s>\\S+) t=(?P<t>\\d+)',
          fields: {
            v: { type: 'number' },
            on: { type: 'bool' },
            d: { type: 'duration', to: 's' },
            n: { type: 'integer' },
            b: { type: 'bytes', unit: 'KB' },
            s: { type: 'string' },
            t: { type: 'duration', unit: 's' },
            missing: { type: 'number' },
          },
        },
      ],
    });
    expect(e.records[0].fields).toMatchObject({ v: 1.5, on: true, d: 1.5, n: 7, b: 2048, s: 'hi', t: 3000 });
    expect(e.records[0].fields['missing']).toBeUndefined();
  });

  it('builds safe regexes', () => {
    expect(safeRegex('(')).toBeNull();
    expect(safeRegex('(?i)abc')!.test('ABC')).toBe(true);
  });
});
