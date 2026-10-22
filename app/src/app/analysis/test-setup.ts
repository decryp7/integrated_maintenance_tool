import { inject, Provider } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { AnalysisBackend } from '../core/analysis-backend';
import { Analyzed, Anomaly, Comparison, Coverage, ExceptionGroup, FieldStats, fullCoverage, MomentSummary, OutlierReport, Pattern, RequestSpan, TracesResult, RecordView } from '../core/api.types';
import { Logger } from '../core/logger';
import { ImtBackend } from '../core/backend';
import { MockBackend } from '../core/mock/mock-backend';
import { COMMAND_CONTEXT } from '../extensions/command-registry';
import { CommandContext } from '../extensions/tokens';
import { WorkspaceStore } from '../core/workspace.store';
import { MemoryLogger, settle } from '../../testing/helpers';
import { HISTORY_STORE, HistoryStore } from './history-store';
import { MockAnalysisBackend, recordsFrom } from './mock/mock-analysis-backend';

/** Shared setup for the analysis specs: demo workspace, mock analysis backend, in-memory history. */

export class MemoryHistory implements HistoryStore {
  saved: string[] = [];
  constructor(public initial: string[] = []) {}
  load() {
    return this.initial;
  }
  save(entries: string[]) {
    this.saved = entries;
  }
}

export interface Call {
  fn: string;
  args: unknown[];
}

/** A `CommandContext` that records what the views asked for. */
export class FakeContext implements CommandContext {
  readonly calls: Call[] = [];
  summary: CommandContext['summary'] = null;
  openPaths = async (...args: string[][]) => void this.calls.push({ fn: 'openPaths', args });
  setQuery = (...args: unknown[]) => void this.calls.push({ fn: 'setQuery', args });
  addFilter = (...args: unknown[]) => void this.calls.push({ fn: 'addFilter', args });
  selectRecord = (...args: unknown[]) => void this.calls.push({ fn: 'selectRecord', args });
  jumpTo = async (...args: number[]) => void this.calls.push({ fn: 'jumpTo', args });
  showView = (...args: unknown[]) => void this.calls.push({ fn: 'showView', args });
  suggestPerspective = (...args: unknown[]) => void this.calls.push({ fn: 'suggestPerspective', args });
  notifyError = (...args: unknown[]) => void this.calls.push({ fn: 'notifyError', args });
  last(fn: string): unknown[] | undefined {
    return [...this.calls].reverse().find((c) => c.fn === fn)?.args;
  }
}

export function provideAnalysisTesting(extra: { backend?: AnalysisBackend; history?: HistoryStore; context?: CommandContext } = {}): Provider[] {
  return [
    { provide: ImtBackend, useFactory: () => new MockBackend({ latency: false }) },
    { provide: Logger, useFactory: () => new MemoryLogger() },
    { provide: AnalysisBackend, useFactory: () => extra.backend ?? new MockAnalysisBackend(recordsFrom(inject(ImtBackend))) },
    { provide: HISTORY_STORE, useValue: extra.history ?? new MemoryHistory() },
    ...(extra.context ? [{ provide: COMMAND_CONTEXT, useValue: extra.context }] : []),
  ];
}

/** Loads the demo workspace into the store and waits for it. */
export async function loadWorkspace() {
  const store = TestBed.inject(WorkspaceStore);
  await settle(undefined, 6);
  return store;
}

export async function allRecords(): Promise<RecordView[]> {
  return recordsFrom(TestBed.inject(ImtBackend))();
}

/** Fixed engine answers for view tests (exact assertions); the mock backend covers the plausible-data path. */
export const FIX = {
  anomalies: [
    { kind: 'errorSpike', instance: 'WEB01 · 3312', from: 1000, to: 5000, score: 6.5, message: '12 errors in 5 min', sampleIds: [7, 8] },
    { kind: 'crash', instance: null, from: 9000, to: 9000, score: 10, message: 'Process terminated', sampleIds: [] },
  ] as Anomaly[],
  outliers: {
    subset: '@level >= error',
    subsetTotal: 60,
    restTotal: 540,
    fieldsChecked: 4,
    outliers: [
      { field: 'Host', value: 'B', subsetCount: 60, restCount: 240, lift: 4.2, significance: 38.5, pValue: 1e-9, query: 'Host = "B" and (@level >= error)', sampleIds: [5, 6] },
      { field: 'Build', value: '3.1', subsetCount: 30, restCount: 10, lift: 9, significance: 12.1, pValue: 0.0005, query: 'Build = "3.1" and (@level >= error)', sampleIds: [] },
    ],
  } as OutlierReport,
  patterns: [
    { id: 1, template: 'Order {OrderId} placed', count: 120, levels: [0, 0, 100, 15, 5, 0], firstTs: 1000, lastTs: 9000, sampleIds: [1], source: 'Serilog', trend: [0, 5, 20, 40, 30, 15, 10], trendFrom: 1000, trendBucketMs: 1200, placeholders: [] },
    { id: 2, template: 'Took <*> ms', count: 30, levels: [0, 0, 30, 0, 0, 0], firstTs: null, lastTs: null, sampleIds: [], source: 'Mined', trend: [], trendFrom: null, trendBucketMs: 0, placeholders: [{ index: 0, total: 30, exact: false, values: [{ value: '12', count: 9 }, { value: '15', count: 4 }] }] },
  ] as Pattern[],
  exceptions: [
    { fingerprint: 'fp1', typeName: 'System.TimeoutException', message: 'The operation timed out', topFrame: 'Foo.Bar()', count: 5, instances: [['WEB01', 3], ['WEB02', 2]], firstTs: 1000, lastTs: 4000, sampleIds: [11, 12] },
    { fingerprint: 'fp2', typeName: 'System.Exception', message: 'x', topFrame: null, count: 1, instances: [], firstTs: null, lastTs: null, sampleIds: [] },
  ] as ExceptionGroup[],
  comparison: {
    a: { count: 100, from: 0, to: 60000, durationMs: 60000, perMinute: 100, levels: [{ level: 'Information', count: 90, perMinute: 90 }, { level: 'Error', count: 10, perMinute: 10 }] },
    b: { count: 300, from: 60000, to: 120000, durationMs: 60000, perMinute: 300, levels: [{ level: 'Error', count: 120, perMinute: 120 }] },
    changedFields: [{ field: 'StatusCode', distance: 0.42, top: [{ value: '504', countA: 1, countB: 80, shareA: 0.01, shareB: 0.4 }, { value: '200', countA: 99, countB: 120, shareA: 0.99, shareB: 0.6 }] }],
    numericChanges: [{ field: 'ElapsedMs', a: { count: 5, median: 100, p95: 200 }, b: { count: 6, median: 250, p95: 150 }, medianChangePct: 150, p95ChangePct: -25 }],
    newValues: [{ field: 'Dest', values: [{ value: 'web03', count: 4 }] }],
    goneValues: [{ field: 'Dest', values: [{ value: 'web01', count: 9 }] }],
    summary: ['Errors increased 12x', 'New destination web03'],
  } as Comparison,
  stats: {
    count: 90, missing: 10, distinct: 40,
    numeric: { min: 1, max: 100, mean: 40, stddev: 12, p50: 35, p90: 80, p95: 90, p99: 99, histogram: [{ from: 1, to: 50, count: 60 }, { from: 50, to: 100, count: 30 }] },
    top: [{ value: '42', count: 20 }, { value: '7', count: 5 }],
  } as FieldStats,
  textStats: { count: 5, missing: 0, distinct: 2, numeric: null, top: [] } as FieldStats,
  traces: {
    traces: [
      { traceId: 'aaaaaaaaaaaaaaaa1111', spans: 3, errors: 1, start: 1000, durationMs: 1500, root: 'GET /orders' },
      { traceId: 'bbbb', spans: 1, errors: 0, start: 3000, durationMs: 20, root: 'GET /health' },
    ],
    waterfall: [],
    coverage: fullCoverage(3000),
  } as TracesResult,
  waterfall: {
    traces: [],
    waterfall: [
      { spanId: 's1', parent: null, depth: 0, name: 'GET /orders', startOffsetMs: 0, durationMs: 1500, errors: 0 },
      { spanId: 's2', parent: 's1', depth: 1, name: 'SELECT orders', startOffsetMs: 100, durationMs: 1200, errors: 2 },
    ],
    coverage: fullCoverage(3000),
  } as TracesResult,
  requests: [
    { id: 'req-0001-abcdefghijkl', first: 1000, last: 3500, events: 12, errors: 1, instance: 'WEB01' },
    { id: 'req-2', first: 2000, last: 2100, events: 2, errors: 0, instance: 'WEB02' },
  ] as RequestSpan[],
  moment: {
    from: 1000, to: 61000, total: 3000,
    levels: { Information: 2900, Warning: 70, Error: 30 },
    instances: [
      { key: 'WEB01 · 3312', events: 2000, warnings: 50, errors: 25, patterns: [FIXED_PATTERN()], firstError: 77 },
      { key: 'WORKER01', events: 1000, warnings: 20, errors: 0, patterns: [], firstError: null },
    ],
    metrics: [
      { series: 'WEB01 · 3312', metric: 'Cpu', samples: 5, min: 10, avg: 55.5, max: 99, last: 80, previousAvg: 20, changePct: 177.5 },
      { series: 'WORKER01', metric: 'Threads', samples: 2, min: 5, avg: 5, max: 5, last: 5, previousAvg: 8, changePct: -37.5 },
      { series: 'WORKER01', metric: 'Mem', samples: 1, min: 1, avg: 1, max: 1, last: 1, previousAvg: null, changePct: null },
    ],
    users: [{ field: 'UserName', value: 'ann', count: 9 }, { field: 'UserName', value: 'bob', count: 2 }],
    activity: [{ id: 5, ts: 2000, instance: 'WEB01 · 3312', user: 'ann', action: 'Order placed', level: 'Information' }, { id: 6, ts: 3000, instance: 'WEB01 · 3312', user: null, action: 'Anonymous', level: 'Warning' }],
    anomalies: [{ kind: 'errorSpike', instance: 'WEB01 · 3312', from: 2000, to: 4000, score: 3, message: 'spike', sampleIds: [9] }, { kind: 'silence', instance: null, from: 1, to: 2, score: 1, message: 'quiet', sampleIds: [] }],
  } as MomentSummary,
};

function FIXED_PATTERN(): Pattern {
  return { id: 9, template: 'Failed to capture payment', count: 25, levels: [0, 0, 0, 0, 25, 0], firstTs: 1, lastTs: 2, sampleIds: [], source: 'Serilog', trend: [], trendFrom: null, trendBucketMs: 0, placeholders: [] };
}

/** The coverage the stub reports when a test gives none: everything analysed. */
export const STUB_COVERAGE: Coverage = fullCoverage(3000);

/** The analyses that resolve with `Analyzed` (the stub wraps plain results of overrides in it). */
const ANALYZED = new Set<string>(['queryPatterns', 'queryExceptions', 'queryAnomalies', 'queryOutliers', 'compareWindows', 'correlate', 'queryRequests', 'queryMoment']);

function isAnalyzed(v: unknown): v is Analyzed<unknown> {
  return !!v && typeof v === 'object' && 'result' in v && 'coverage' in v;
}

/** An override's answer in the reply's shape: plain results get the stub coverage (tests may pass `Analyzed` themselves). */
function shaped(key: string, v: unknown): unknown {
  if (ANALYZED.has(key)) return isAnalyzed(v) ? v : { result: v, coverage: STUB_COVERAGE };
  if (key === 'queryTraces' && v && typeof v === 'object' && !('coverage' in v)) return { ...v, coverage: STUB_COVERAGE };
  return v;
}

/**
 * A backend answering from `FIX`; override or spy on individual methods in tests. Overrides of the analyses may
 * return the plain result (wrapped with `STUB_COVERAGE`) or an `Analyzed` reply.
 */
export function stubBackend(overrides: Partial<Record<keyof AnalysisBackend, (...a: never[]) => unknown>> = {}): AnalysisBackend {
  const base = {
    runQuery: async () => ({ result: { kind: 'events', ids: [1, 2], fields: null } }),
    runQueryStreaming(this: AnalysisBackend, query: never) {
      return this.runQuery(query);
    },
    cancelQuery: async () => false,
    checkQuery: async () => null,
    // eslint-disable-next-line no-control-regex
    classifySearch: async (text: string) => (/[=<>~|]|exists\(/.test(text) ? 'query' : 'text'),
    completeQuery: async () => [],
    queryPatterns: async () => FIX.patterns,
    queryExceptions: async () => FIX.exceptions,
    queryAnomalies: async () => FIX.anomalies,
    queryOutliers: async () => FIX.outliers,
    compareWindows: async () => FIX.comparison,
    fieldStats: async () => FIX.stats,
    correlate: async (_f: string, v: string) => (v.startsWith('req') ? [1, 2, 3] : []),
    queryTraces: async (_q: unknown, id: string) => (id ? FIX.waterfall : FIX.traces),
    queryRequests: async () => FIX.requests,
    queryMoment: async () => FIX.moment,
  };
  const merged: Record<string, unknown> = { ...base, ...overrides };
  for (const key of [...ANALYZED, 'queryTraces']) {
    const fn = merged[key] as (...a: unknown[]) => unknown;
    merged[key] = (...a: unknown[]) => {
      const out = fn(...a);
      return out instanceof Promise ? out.then((v) => shaped(key, v)) : shaped(key, out);
    };
  }
  return merged as unknown as AnalysisBackend;
}
