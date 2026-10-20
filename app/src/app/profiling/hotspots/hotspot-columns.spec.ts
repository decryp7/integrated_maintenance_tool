import { describe, expect, it } from 'vitest';
import { HotspotRow, Measurement, MetricDto, MetricId, MetricUnit, ProfileSummary } from '../../core/api.types';
import { GROUP_LABELS, hotspotColumns, sameSort } from './hotspot-columns';

const UNITS: Record<MetricId, MetricUnit> = {
  samples: 'count', timeNs: 'nanoseconds', calls: 'count', allocCount: 'count', allocBytes: 'bytes', exceptions: 'count', waitNs: 'nanoseconds', maxCallNs: 'nanoseconds', liveBytes: 'bytes',
};
const metric = (id: MetricId, measurement: Measurement): MetricDto => ({ id, measurement, unit: UNITS[id], label: id });
const summary = (metrics: MetricDto[]): ProfileSummary => ({ capture: 'profile', id: 1, metrics, defaultMetric: metrics[0]?.id ?? 'samples' }) as unknown as ProfileSummary;
const row = (patch: Partial<HotspotRow> = {}): HotspotRow => ({
  key: 'k', name: 'Foo.Bar', module: null, frame: null, thread: null,
  self: { timeNs: 2_000_000, calls: 4, maxCallNs: 1500, samples: 7, allocCount: 3, allocBytes: 2048, exceptions: 1, waitNs: 5000 },
  total: { timeNs: 3_000_000, samples: 9 },
  selfPct: 12.34, totalPct: 50, avgPerCallNs: 750_000, maxPerCallNs: null, ...patch,
});
const ids = (cols: { id: string }[]) => cols.map((c) => c.id);
const col = (cols: ReturnType<typeof hotspotColumns>, id: string) => cols.find((c) => c.id === id)!;

describe('hotspotColumns', () => {
  it('always has name and percentage columns, and only the metrics the profile has', () => {
    const cols = hotspotColumns(summary([metric('samples', 'sampled')]), 'samples', 'Function');
    expect(ids(cols)).toEqual(['name', 'selfPct', 'totalPct', 'total-samples']);
    expect(cols[0]).toMatchObject({ label: 'Function', sort: { metric: null }, measurement: null, numeric: false });
    expect(cols[0]?.help).toBe('Sort by function name');
    expect(cols[0]?.value(row())).toBe('Foo.Bar');
  });

  it('offers every column for a full instrumented profile', () => {
    const all = (Object.keys(UNITS) as MetricId[]).map((id) => metric(id, 'exact'));
    const cols = hotspotColumns(summary(all), 'timeNs', 'Type');
    expect(ids(cols)).toEqual(['name', 'self-timeNs', 'total-timeNs', 'selfPct', 'totalPct', 'total-samples', 'self-calls', 'avg', 'self-maxCallNs', 'self-allocCount', 'self-allocBytes', 'self-exceptions', 'self-waitNs']);
  });

  it('labels each column sampled or exact from the profile MetricDto', () => {
    const cols = hotspotColumns(
      summary([metric('timeNs', 'sampled'), metric('samples', 'sampled'), metric('calls', 'exact'), metric('allocCount', 'exact'), metric('waitNs', 'sampled')]),
      'timeNs',
      'Function',
    );
    expect(col(cols, 'self-timeNs').measurement).toBe('sampled');
    expect(col(cols, 'total-timeNs').measurement).toBe('sampled');
    expect(col(cols, 'total-samples').measurement).toBe('sampled');
    expect(col(cols, 'self-calls').measurement).toBe('exact');
    expect(col(cols, 'self-allocCount').measurement).toBe('exact');
    expect(col(cols, 'self-waitNs').measurement).toBe('sampled');
    // percentages follow the table's metric
    expect(col(cols, 'selfPct').measurement).toBe('sampled');
    expect(col(cols, 'totalPct').measurement).toBe('sampled');
    const exactTable = hotspotColumns(summary([metric('calls', 'exact'), metric('timeNs', 'sampled')]), 'calls', 'Function');
    expect(col(exactTable, 'selfPct').measurement).toBe('exact');
    expect(col(exactTable, 'totalPct').measurement).toBe('exact');
  });

  it('flips the label when the same metric is exact in another profile', () => {
    const exact = hotspotColumns(summary([metric('timeNs', 'exact')]), 'timeNs', 'Function');
    expect(col(exact, 'self-timeNs').measurement).toBe('exact');
  });

  it('treats a table metric the profile lacks as sampled', () => {
    const cols = hotspotColumns(summary([metric('timeNs', 'exact')]), 'samples', 'Function');
    expect(col(cols, 'selfPct').measurement).toBe('sampled');
  });

  it('avg per call is sampled when either input is sampled, exact when both exact, and sorts by the engine key', () => {
    const both = (c: Measurement, t: Measurement) => col(hotspotColumns(summary([metric('calls', c), metric('timeNs', t)]), 'timeNs', 'F'), 'avg');
    expect(both('exact', 'sampled').measurement).toBe('sampled');
    expect(both('sampled', 'exact').measurement).toBe('sampled');
    expect(both('sampled', 'sampled').measurement).toBe('sampled');
    expect(both('exact', 'exact').measurement).toBe('exact');
    expect(both('exact', 'exact').sort).toEqual({ metric: 'timeNs', of: 'avgPerCall' });
    expect(both('exact', 'exact').value(row())).toBe('750.0 µs');
    expect(both('exact', 'exact').value(row({ avgPerCallNs: null }))).toBe('—');
  });

  it('has no avg column without both calls and time', () => {
    expect(ids(hotspotColumns(summary([metric('calls', 'exact')]), 'calls', 'F'))).not.toContain('avg');
    expect(ids(hotspotColumns(summary([metric('timeNs', 'exact')]), 'timeNs', 'F'))).not.toContain('avg');
  });

  it('sorts by the engine metric and value-of', () => {
    const cols = hotspotColumns(summary([metric('timeNs', 'sampled'), metric('calls', 'exact')]), 'timeNs', 'F');
    expect(col(cols, 'self-timeNs').sort).toEqual({ metric: 'timeNs', of: 'self' });
    expect(col(cols, 'total-timeNs').sort).toEqual({ metric: 'timeNs', of: 'total' });
    expect(col(cols, 'selfPct').sort).toEqual({ metric: 'timeNs', of: 'self' });
    expect(col(cols, 'totalPct').sort).toEqual({ metric: 'timeNs', of: 'total' });
    expect(col(cols, 'selfPct').pct).toBe('self');
    expect(col(cols, 'totalPct').pct).toBe('total');
  });

  it('formats values by unit', () => {
    const all = (Object.keys(UNITS) as MetricId[]).map((id) => metric(id, 'exact'));
    const cols = hotspotColumns(summary(all), 'timeNs', 'F');
    const r = row();
    expect(col(cols, 'self-timeNs').value(r)).toBe('2.00 ms');
    expect(col(cols, 'total-timeNs').value(r)).toBe('3.00 ms');
    expect(col(cols, 'selfPct').value(r)).toBe('12.3%');
    expect(col(cols, 'totalPct').value(r)).toBe('50.0%');
    expect(col(cols, 'total-samples').value(r)).toBe('9');
    expect(col(cols, 'self-calls').value(r)).toBe('4');
    expect(col(cols, 'self-allocBytes').value(r)).toBe('2.0 KB');
    expect(col(cols, 'self-exceptions').value(r)).toBe('1');
    expect(col(cols, 'self-waitNs').value(r)).toBe('5.0 µs');
    expect(col(cols, 'total-samples').value(row({ total: {} }))).toBe('—');
  });

  it('max per call uses the row value and falls back to self maxCallNs', () => {
    const cols = hotspotColumns(summary([metric('maxCallNs', 'exact')]), 'maxCallNs', 'F');
    const max = col(cols, 'self-maxCallNs');
    expect(max.value(row({ maxPerCallNs: 9000 }))).toBe('9.0 µs');
    expect(max.value(row({ maxPerCallNs: null }))).toBe('1.5 µs');
    expect(max.value(row({ maxPerCallNs: null, self: {} }))).toBe('—');
    expect(max.measurement).toBe('exact');
  });

  it('GROUP_LABELS names every grouping', () => {
    expect(Object.keys(GROUP_LABELS)).toEqual(['function', 'type', 'namespace', 'module', 'thread']);
  });
});

describe('sameSort', () => {
  it('is false when either side is missing', () => {
    expect(sameSort(null, { metric: null })).toBe(false);
    expect(sameSort({ metric: null }, undefined)).toBe(false);
  });
  it('compares metric and value-of (default self)', () => {
    expect(sameSort({ metric: null }, { metric: null })).toBe(true);
    expect(sameSort({ metric: 'timeNs', of: 'self' }, { metric: 'timeNs' })).toBe(true);
    expect(sameSort({ metric: 'timeNs', of: 'self' }, { metric: 'timeNs', of: 'total' })).toBe(false);
    expect(sameSort({ metric: 'timeNs' }, { metric: 'calls' })).toBe(false);
    expect(sameSort({ metric: null }, { metric: 'calls' })).toBe(false);
    expect(sameSort({ metric: 'calls' }, { metric: null })).toBe(false);
    expect(sameSort({} as never, { metric: null })).toBe(true);
    // name sort ignores `of`
    expect(sameSort({ metric: null, of: 'total' }, { metric: null })).toBe(true);
  });
});
