import { T0, tinyDataset } from '../../../testing/helpers';
import { MockBackend } from './mock-backend';
import { MockDataset, MockRecord } from './mock-data';
import { checkOffset, firstChanged, MOCK_MAX_OFFSET_MS, mockSuggest, recordKey, resolveOffset, shiftSource } from './mock-skew';
import { at, must } from '../../../testing/must';

const r = (source: number, line: number, ts: number | null, fields: Record<string, unknown> = {}): MockRecord => ({
  id: 0,
  ts,
  level: 'Information',
  message: `s${source} l${line}`,
  template: null,
  exception: null,
  fields,
  source,
  line,
  markers: [],
});

function correlated(n: number, skew: number, noisyEvery = 0): MockDataset {
  const records: MockRecord[] = [];
  for (let i = 0; i < n; i++) {
    records.push(r(0, i + 1, T0 + i * 1000, { CorrelationId: `c${i}` }));
    const noise = noisyEvery && i % noisyEvery === 0 ? 600_000 : 0;
    records.push(r(1, i + 1, T0 + i * 1000 - skew + noise, { CorrelationId: `c${i}` }));
  }
  records.push(r(2, 1, T0, { CorrelationId: 'lonely' }), r(1, 999, null, { CorrelationId: 'c0' }));
  return {
    sources: [
      { id: 0, path: '/a.log', name: 'a.log', importer: 'json', skipped: 0 },
      { id: 1, path: '/b.log', name: 'b.log', importer: 'json', skipped: 0 },
      { id: 2, path: '/c.log', name: 'c.log', importer: 'json', skipped: 0 },
    ],
    unrecognized: [],
    records,
  };
}

describe('mock clock skew', () => {
  it('checks offsets and resolves manual and aligned requests like the engine', () => {
    expect(checkOffset(1.4)).toBe(1);
    expect(() => checkOffset(MOCK_MAX_OFFSET_MS + 1)).toThrow(/too large/);
    expect(() => checkOffset(Number.NaN)).toThrow(/too large/);
    const d = correlated(3, 2000);
    const shown = d.records.map((x, i) => ({ ...x, id: i }));
    expect(resolveOffset({ kind: 'manual', source: 1, offsetMs: 5 }, shown, d.sources)).toEqual({ source: 1, offsetMs: 5 });
    expect(() => resolveOffset({ kind: 'manual', source: 9, offsetMs: 5 }, shown, d.sources)).toThrow(/not open/);
    expect(resolveOffset({ kind: 'align', reference: 0, target: 1 }, shown, d.sources)).toEqual({ source: 1, offsetMs: 2000 });
    expect(() => resolveOffset({ kind: 'align', reference: 0, target: 99 }, shown, d.sources)).toThrow(/no longer exists/);
    expect(() => resolveOffset({ kind: 'align', reference: 0, target: shown.length - 1 }, shown, d.sources)).toThrow(/no timestamp/);
    expect(() => resolveOffset({ kind: 'align', reference: 0, target: 2 }, shown, d.sources)).toThrow(/same source/);
    // An already shifted target aligns from its original time.
    at(d.sources, 1).offsetMs = 500;
    expect(resolveOffset({ kind: 'align', reference: 0, target: 1 }, shown, d.sources).offsetMs).toBe(2500);
  });

  it('shifts raw records and finds the first changed position', () => {
    const d = correlated(2, 0);
    const before = d.records.map((x) => ({ ...x }));
    expect(shiftSource(d.records, d.sources, 1, 300)).toBe(0);
    expect(d.records.filter((x) => x.source === 1 && x.ts !== null).every((x) => (x.ts! - T0) % 1000 === 300)).toBe(true);
    expect(shiftSource(d.records, d.sources, 1, 300)).toBe(300);
    expect(shiftSource(d.records, d.sources, 7, 1)).toBe(0);
    expect(firstChanged(before, d.records)).toBe(1);
    expect(firstChanged(before, before)).toBeNull();
    expect(firstChanged(before, before.slice(1))).toBe(0);
    expect(recordKey(at(before, 0))).toBe('0:1');
  });

  it('suggests the median delta with a confidence, lower for noisy ids', () => {
    const clean = correlated(12, 2500);
    const s = mockSuggest(clean.records, clean.sources);
    expect(s.reference).toBe(0);
    expect(s.fields).toEqual(['CorrelationId', 'TraceId', 'RequestId']);
    expect(s.suggestions).toEqual([expect.objectContaining({ source: 1, suggestedMs: 2500, pairs: 12, agreeing: 12, confidence: 1, spreadMs: 0 })]);
    const noisy = correlated(12, 2500, 3);
    const n = at(mockSuggest(noisy.records, noisy.sources, { reference: 0 }).suggestions, 0);
    expect(n.confidence).toBeLessThan(1);
    expect(n.agreeing).toBe(8);
    const few = correlated(2, 100);
    expect(mockSuggest(few.records, few.sources).insufficient).toEqual([1]);
    expect(mockSuggest([], []).reference).toBeNull();
    expect(mockSuggest(clean.records, clean.sources, { fields: ['Nope'] }).reference).toBeNull();
    expect(mockSuggest(clean.records, clean.sources, { maxDeltaMs: 10 }).insufficient).toEqual([1]);
  });

  it('applies offsets through the mock backend keeping tracked records and showing skew', async () => {
    const b = new MockBackend({ latency: false, dataset: tinyDataset, pluginMarkerKinds: [] });
    const page = async () => (await b.queryPage({ query: { from: null, to: null, levels: [], text: '', regex: false, caseSensitive: false, filters: [], groupBy: [], groups: [], markers: [] }, offset: 0, limit: 100, descending: false })).rows;
    const rows = await page();
    const worker = rows.find((x) => x.message === 'Application starting — Worker')!;
    const change = await b.setSourceOffset({ kind: 'manual', source: 1, offsetMs: 100_000, track: [worker.id, 999] });
    expect(change).toMatchObject({ source: 1, offsetMs: 100_000, previousMs: 0 });
    expect(change.idsChangedFrom).toBeNull();
    expect(change.tracked[1]).toBeNull();
    const moved = at(await page(), must(at(change.tracked, 0)));
    expect(moved.message).toBe('Application starting — Worker');
    expect(moved.ts).toBe(worker.ts! + 100_000);
    expect((await b.workspaceSummary()).sources.find((s) => s.id === 1)?.offsetMs).toBe(100_000);
    const same = await b.setSourceOffset({ kind: 'manual', source: 1, offsetMs: 100_000 });
    expect(same.idsChangedFrom).toBeNull();
    await expect(b.setSourceOffset({ kind: 'manual', source: 1, offsetMs: 1e15 })).rejects.toMatchObject({ code: 'invalid', retryable: false, message: expect.stringMatching(/too large/) });
    expect((await b.suggestOffsets()).suggestions).toEqual([]);
  });
});
