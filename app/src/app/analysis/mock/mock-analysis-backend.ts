import { ImtBackend } from '../../core/backend';
import { EMPTY_QUERY } from '../../core/workspace.store';
import { RecordView } from '../../core/api.types';
import { AnalysisBackend } from '../../core/analysis-backend';
import { Analyzed, fullCoverage, QlQuery, QueryOutcome, QueryProgress, StreamedQueryOutcome } from '../../core/api.types';
import {
  filterRecords,
  mockAnomalies,
  mockCompare,
  mockCorrelate,
  mockExceptions,
  mockFieldStats,
  mockMoment,
  mockPatterns,
  mockRequests,
  mockTraces,
} from './mock-insights';
import { checkMockQl, MockQlError, mockCompletions, runMockQl } from '../../core/mock/mock-ql';
import { mockUnknownField } from './mock-unknown-field';
import { mockResolveWindow } from '../../core/mock/mock-live';

/** Timestamp of the newest record (the "now" of live windows, as the engine's default data clock). */
function newestTs(records: readonly RecordView[]): number | null {
  let newest: number | null = null;
  for (const r of records) if (r.ts !== null && (newest === null || r.ts > newest)) newest = r.ts;
  return newest;
}

/** The demo data is small: an analysis looks at every one of the `matched` events. */
function analysed<T>(result: T, matched: number): Analyzed<T> {
  return { result, coverage: fullCoverage(matched) };
}

/** Loads every record of the (mock) workspace through the regular backend, so ids match the grid. */
export function recordsFrom(backend: ImtBackend): () => Promise<RecordView[]> {
  return async () => (await backend.queryPage({ query: EMPTY_QUERY, offset: 0, limit: 1_000_000, descending: false })).rows;
}

/**
 * Plausible analysis results computed from the demo dataset, for browser development and UI tests.
 * It is never the reference for behaviour: the Rust engine is.
 */
export class MockAnalysisBackend extends AnalysisBackend {
  constructor(private readonly records: () => Promise<RecordView[]>) {
    super();
  }

  /** The records and the query with its sliding window (`window: live`) resolved into `from`, like the engine. */
  private async resolved(query: QlQuery): Promise<{ all: RecordView[]; query: QlQuery }> {
    const all = await this.records();
    return { all, query: mockResolveWindow(query, newestTs(all)) };
  }

  private async matched(query: QlQuery): Promise<RecordView[]> {
    const r = await this.resolved(query);
    return filterRecords(r.all, r.query);
  }

  async runQuery(query: QlQuery): Promise<QueryOutcome> {
    const r = await this.resolved(query);
    const all = r.all;
    const { ql, ...base } = r.query;
    try {
      return { result: runMockQl(ql ?? '', filterRecords(all, base)) };
    } catch (e) {
      if (e instanceof MockQlError) return { error: e.toQlError() };
      throw e;
    }
  }
  /** Yields once first, so a `cancelQuery` right after the start takes effect, like the engine's batch checks. */
  override async runQueryStreaming(query: QlQuery, queryId: string, onProgress: (p: QueryProgress) => void): Promise<StreamedQueryOutcome> {
    this.running.set(queryId, false);
    try {
      onProgress({ stage: 'matching', done: 0, total: 1 });
      await Promise.resolve();
      if (this.running.get(queryId)) return { cancelled: true };
      const out = await this.runQuery(query);
      onProgress({ stage: 'evaluating', done: 1, total: 1 });
      return out;
    } finally {
      this.running.delete(queryId);
    }
  }
  override async cancelQuery(queryId: string): Promise<boolean> {
    if (!this.running.has(queryId)) return false;
    this.running.set(queryId, true);
    return true;
  }
  /** Runs in flight, by id: whether a cancel was asked. */
  private readonly running = new Map<string, boolean>();

  async checkQuery(text: string) {
    const syntax = checkMockQl(text);
    if (syntax) return syntax;
    const all = await this.records();
    return mockUnknownField(text, [...new Set(all.slice(0, 500).flatMap((r) => Object.keys(r.fields)))]);
  }
  async completeQuery(text: string, cursor: number) {
    const all = await this.records();
    const names = [...new Set(all.slice(0, 500).flatMap((r) => Object.keys(r.fields)))].sort();
    return mockCompletions(text, cursor, names);
  }
  async queryPatterns(query: QlQuery) {
    const m = await this.matched(query);
    return analysed(mockPatterns(m), m.length);
  }
  async queryExceptions(query: QlQuery) {
    const m = await this.matched(query);
    return analysed(mockExceptions(m), m.length);
  }
  async queryAnomalies(query: QlQuery) {
    const m = await this.matched(query);
    return analysed(mockAnomalies(m), m.length);
  }
  async compareWindows(a: QlQuery, b: QlQuery) {
    const [ma, mb] = [await this.matched(a), await this.matched(b)];
    return analysed(mockCompare(ma, mb), ma.length + mb.length);
  }
  async fieldStats(query: QlQuery, field: string, buckets: number) {
    return mockFieldStats(await this.matched(query), field, buckets);
  }
  async correlate(field: string, value: string) {
    const all = await this.records();
    return analysed(mockCorrelate(all, field, value), all.length);
  }
  async queryTraces(query: QlQuery, traceId: string) {
    const m = await this.matched(query);
    return { ...mockTraces(m, traceId), coverage: fullCoverage(m.length) };
  }
  async queryRequests(query: QlQuery, idField: string) {
    const m = await this.matched(query);
    return analysed(mockRequests(m, idField), m.length);
  }
  async queryMoment(query: QlQuery, from: number, to: number) {
    const r = await this.resolved(query);
    return analysed(mockMoment(r.all, r.query, from, to), filterRecords(r.all, { ...r.query, from, to }).length);
  }
}
