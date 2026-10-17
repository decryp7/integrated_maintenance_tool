import { describe, expect, it } from 'vitest';
import { CaptureSummary, HeapSummary, MetricDto, ProfileSummary } from '../core/api.types';
import {
  captureKindLabel,
  combineMeasurements,
  formatBytes,
  formatDelta,
  formatMetric,
  formatNs,
  formatPct,
  formatPp,
  formatValue,
  isHeap,
  isProfile,
  measurementGlyph,
  measurementHelp,
  measurementLabel,
  measurementOf,
  metricLabel,
  shortName,
  unitOf,
} from './profiling-format';

const profile = (patch: Partial<ProfileSummary> = {}): ProfileSummary =>
  ({
    capture: 'profile',
    id: 1,
    name: 'p',
    kind: 'cpu',
    metrics: [
      { id: 'timeNs', measurement: 'sampled', unit: 'nanoseconds', label: 'CPU time' },
      { id: 'calls', measurement: 'exact', unit: 'count', label: 'Calls' },
    ],
    ...patch,
  }) as ProfileSummary;
const heap = { capture: 'heap', id: 2 } as HeapSummary;

describe('profiling-format', () => {
  it('unitOf prefers the profile metric over the default table', () => {
    expect(unitOf('allocBytes')).toBe('bytes');
    expect(unitOf('timeNs')).toBe('nanoseconds');
    expect(unitOf('samples')).toBe('count');
    expect(unitOf('waitNs')).toBe('nanoseconds');
    const m: MetricDto[] = [{ id: 'samples', measurement: 'sampled', unit: 'bytes', label: 'x' }];
    expect(unitOf('samples', m)).toBe('bytes');
    expect(unitOf('calls', m)).toBe('count');
  });

  it('formatNs covers every magnitude and non-finite input', () => {
    expect(formatNs(null)).toBe('—');
    expect(formatNs(undefined)).toBe('—');
    expect(formatNs(NaN)).toBe('—');
    expect(formatNs(Infinity)).toBe('—');
    expect(formatNs(850)).toBe('850 ns');
    expect(formatNs(12_500)).toBe('12.5 µs');
    expect(formatNs(3_200_000)).toBe('3.20 ms');
    expect(formatNs(25_000_000)).toBe('25.0 ms');
    expect(formatNs(1_250_000_000)).toBe('1.25 s');
    expect(formatNs(125_000_000_000)).toBe('2 min 5 s');
  });

  it('formatBytes covers every magnitude', () => {
    expect(formatBytes(null)).toBe('—');
    expect(formatBytes(undefined)).toBe('—');
    expect(formatBytes(NaN)).toBe('—');
    expect(formatBytes(512)).toBe('512 B');
    expect(formatBytes(4096)).toBe('4.0 KB');
    expect(formatBytes(1.5 * 1024 ** 2)).toBe('1.5 MB');
    expect(formatBytes(2 * 1024 ** 3)).toBe('2.00 GB');
  });

  it('formatValue / formatMetric dispatch on the unit', () => {
    expect(formatValue(null, 'count')).toBe('—');
    expect(formatValue(undefined, 'bytes')).toBe('—');
    expect(formatValue(1500, 'nanoseconds')).toBe('1.5 µs');
    expect(formatValue(2048, 'bytes')).toBe('2.0 KB');
    expect(formatValue(1234.6, 'count')).toBe('1,235');
    expect(formatMetric(2048, 'allocBytes')).toBe('2.0 KB');
    expect(formatMetric(5, 'samples', [{ id: 'samples', measurement: 'sampled', unit: 'nanoseconds', label: 's' }])).toBe('5 ns');
  });

  it('formatPct, formatPp and formatDelta handle signs and gaps', () => {
    expect(formatPct(null)).toBe('—');
    expect(formatPct(undefined)).toBe('—');
    expect(formatPct(NaN)).toBe('—');
    expect(formatPct(12.345)).toBe('12.3%');
    expect(formatPct(12.345, 2)).toBe('12.35%');
    expect(formatPp(12.34)).toBe('+12.3 pp');
    expect(formatPp(-4)).toBe('−4.0 pp');
    expect(formatPp(0)).toBe('0.0 pp');
    expect(formatPp(0.001)).toBe('0.0 pp');
    expect(formatPp(-0.001)).toBe('0.0 pp');
    expect(formatDelta(4096, 'bytes')).toBe('+4.0 KB');
    expect(formatDelta(-3, 'count')).toBe('−3');
    expect(formatDelta(0, 'count')).toBe('0');
    expect(formatDelta(-2048, 'bytes')).toBe('−2.0 KB');
  });

  it('measurement helpers distinguish sampled from exact', () => {
    expect(measurementGlyph('sampled')).toBe('≈');
    expect(measurementGlyph('exact')).toBe('=');
    expect(measurementLabel('sampled')).toBe('Sampled');
    expect(measurementLabel('exact')).toBe('Exact');
    expect(measurementHelp('sampled')).toContain('Sampled (≈)');
    expect(measurementHelp('exact')).toContain('Exact (=)');
  });

  it('combineMeasurements is sampled when any input is sampled', () => {
    expect(combineMeasurements('exact', 'exact')).toBe('exact');
    expect(combineMeasurements('exact', 'sampled')).toBe('sampled');
    expect(combineMeasurements(null, undefined, 'sampled')).toBe('sampled');
    expect(combineMeasurements()).toBe('exact');
    expect(combineMeasurements(null, 'exact')).toBe('exact');
  });

  it('measurementOf and metricLabel read the summary', () => {
    const p = profile();
    expect(measurementOf(p, 'timeNs')).toBe('sampled');
    expect(measurementOf(p, 'calls')).toBe('exact');
    expect(measurementOf(p, 'waitNs')).toBeNull();
    expect(measurementOf(null, 'timeNs')).toBeNull();
    expect(measurementOf(undefined, 'timeNs')).toBeNull();
    expect(metricLabel(p, 'timeNs')).toBe('CPU time');
    expect(metricLabel(p, 'waitNs')).toBe('waitNs');
    expect(metricLabel(null, 'samples')).toBe('samples');
  });

  it('captureKindLabel, isProfile and isHeap', () => {
    expect(captureKindLabel(heap)).toBe('Heap snapshot');
    expect(captureKindLabel(profile({ kind: 'cpu' }))).toBe('CPU');
    expect(captureKindLabel(profile({ kind: 'alloc' }))).toBe('Allocations');
    expect(captureKindLabel(profile({ kind: 'wall' }))).toBe('Wall clock');
    expect(captureKindLabel(profile({ kind: 'mixed' }))).toBe('Mixed');
    const c: CaptureSummary = profile();
    expect(isProfile(c)).toBe(true);
    expect(isHeap(c)).toBe(false);
    expect(isHeap(heap)).toBe(true);
    expect(isProfile(heap)).toBe(false);
    expect(isProfile(null)).toBe(false);
    expect(isHeap(undefined)).toBe(false);
  });

  it('shortName keeps the last two parts, or the native symbol', () => {
    expect(shortName('Shop.Orders.OrderService.Place')).toBe('OrderService.Place');
    expect(shortName('A.B')).toBe('A.B');
    expect(shortName('main')).toBe('main');
    expect(shortName('libc.so!malloc')).toBe('malloc');
  });
});
