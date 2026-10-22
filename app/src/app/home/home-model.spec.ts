import { HomeSummary } from '../core/api.types';
import { ChartTokens } from '../metrics/chart-theme';
import type { Breadcrumb } from '../core/activity/types';
import { bucketAt, bucketRange, feedItems, historyOption, sparklinePath } from './home-model';

const TOKENS: ChartTokens = {
  text: '#fff',
  muted: '#aaa',
  faint: '#888',
  grid: '#222',
  border: '#333',
  tooltipBg: '#111',
  accentSoft: 'rgba(0,0,255,.1)',
  accent: '#00f',
  series: ['#1', '#2', '#3', '#4'],
  other: '#o',
  levels: { Verbose: '#v', Debug: '#d', Information: '#i', Warning: '#w', Error: '#e', Fatal: '#f' },
  markers: { start: '#s', stop: '#x' },
  fontSans: 'sans',
  fontMono: 'mono',
};

const labels = { level: (l: string) => `L:${l}`, marker: (k: string) => `M:${k}`, anomaly: 'Anomaly' };

function summary(patch: Partial<HomeSummary> = {}): HomeSummary {
  return {
    sources: 1,
    events: 30,
    errors: 3,
    warnings: 4,
    from: 1000,
    to: 4000,
    bucketMs: 1000,
    histogram: [
      [0, 0, 5, 1, 0, 0],
      [0, 0, 6, 2, 2, 0],
      [0, 0, 10, 1, 1, 0],
    ],
    errorTrend: [0, 2, 1],
    warningTrend: [1, 2, 1],
    markers: [
      { kind: 'start', ts: 1000, count: 1 },
      { kind: 'custom', ts: 3000, count: 3 },
    ],
    anomalyCount: 1,
    anomalies: [{ kind: 'errorSpike', from: 2000, to: 3000, message: 'Error spike' }],
    exceptionGroups: 0,
    exceptionEvents: 0,
    coverage: { matched: 30, analysed: 30, limits: [] },
    ...patch,
  } as HomeSummary;
}

describe('sparklinePath', () => {
  it('draws a line scaled to the largest value, and nothing for fewer than two points', () => {
    expect(sparklinePath([])).toBe('');
    expect(sparklinePath([3])).toBe('');
    const d = sparklinePath([0, 4, 2], 100, 24);
    expect(d.startsWith('M0.00 23.00L50.00 1.00L100.00 12.00')).toBe(true);
  });

  it('is flat on all zeros', () => {
    expect(sparklinePath([0, 0])).toBe('M0.00 23.00L100.00 23.00');
  });
});

describe('bucketRange and bucketAt', () => {
  it('maps a bucket to its time range and a time to its bucket', () => {
    const s = summary();
    expect(bucketRange(s, 0)).toEqual({ from: 1000, to: 2000 });
    expect(bucketRange(s, 2)).toEqual({ from: 3000, to: 4000 });
    expect(bucketAt(s, 1000)).toBe(0);
    expect(bucketAt(s, 2500)).toBe(1);
    expect(bucketAt(s, 99_000)).toBe(2);
  });

  it('is empty outside the history', () => {
    const s = summary();
    expect(bucketRange(s, -1)).toBeNull();
    expect(bucketRange(s, 3)).toBeNull();
    expect(bucketRange(s, 0.5)).toBeNull();
    expect(bucketRange(summary({ from: null }), 0)).toBeNull();
    expect(bucketRange(summary({ bucketMs: 0 }), 0)).toBeNull();
    expect(bucketAt(summary({ from: null }), 5)).toBe(-1);
  });
});

describe('historyOption', () => {
  it('stacks the levels present, most severe at the baseline, with markers and anomalies on the first series', () => {
    const option = historyOption(summary(), TOKENS, 'utc', labels) as unknown as {
      series: { name: string; stack: string; markLine?: { data: unknown[] }; markArea?: { data: unknown[] }; data: number[][] }[];
      xAxis: { min: number; max: number };
    };
    expect(option.series.map((x) => x.name)).toEqual(['L:Error', 'L:Warning', 'L:Information']);
    expect(new Set(option.series.map((x) => x.stack))).toEqual(new Set(['events']));
    expect(option.xAxis).toMatchObject({ min: 1000, max: 4000 });
    expect(option.series[0]?.markLine?.data).toHaveLength(2);
    expect(option.series[0]?.markArea?.data).toHaveLength(1);
    expect(option.series[1]?.markLine).toBeUndefined();
    // Bucket centres on the x axis.
    expect(option.series[0]?.data[0]).toEqual([1500, 0]);
  });

  it('has nothing to draw without timestamps or events', () => {
    expect(historyOption(summary({ from: null, to: null }), TOKENS, 'utc', labels)).toBeNull();
    expect(historyOption(summary({ histogram: [] }), TOKENS, 'utc', labels)).toBeNull();
  });
});

describe('feedItems', () => {
  const crumb = (operation: string, outcome: 'ok' | 'error' = 'ok', i = 0): Breadcrumb => ({ time: `2026-01-01T00:00:0${i}Z`, operation, trigger: 'menu', operationId: `op${i}`, outcome, window: 'main' });

  it('lists the latest user actions newest first, leaving out bookkeeping, and marks failures', () => {
    const crumbs = [crumb('file.openFiles', 'ok', 1), crumb('selection.change', 'ok', 2), crumb('timeRange.set', 'ok', 3), crumb('export.run', 'error', 4)];
    const items = feedItems(crumbs, (c) => `T:${c.operation}`);
    expect(items.map((i) => i.label)).toEqual(['T:export.run', 'T:file.openFiles']);
    expect(items.map((i) => i.failed)).toEqual([true, false]);
  });

  it('shows at most n entries', () => {
    const crumbs = Array.from({ length: 20 }, (_, i) => crumb('a.b', 'ok', i % 10));
    expect(feedItems(crumbs, (c) => c.operation)).toHaveLength(8);
    expect(feedItems(crumbs, (c) => c.operation, 3)).toHaveLength(3);
  });
});
