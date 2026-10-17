import { invoke } from './activity/ipc';
import {
  Anomaly,
  Comparison,
  Completion,
  ExceptionGroup,
  FieldStats,
  MomentSummary,
  Pattern,
  QlError,
  QlQuery,
  QueryOutcome,
  RequestSpan,
  TracesResult,
} from './api.types';

/**
 * Everything the analysis views ask the engine. Injected as a class token (see
 * `provideAnalysis()` for the Tauri / mock selection). Every method is a pure query.
 */
export abstract class AnalysisBackend {
  /** Runs `query.ql`. Query-language mistakes come back as `{ error }`, not as a rejection. */
  abstract runQuery(query: QlQuery): Promise<QueryOutcome>;
  abstract checkQuery(text: string): Promise<QlError | null>;
  /** `cursor` is a UTF-8 byte offset into `text`. */
  abstract completeQuery(text: string, cursor: number): Promise<Completion[]>;
  abstract queryPatterns(query: QlQuery): Promise<Pattern[]>;
  abstract queryExceptions(query: QlQuery): Promise<ExceptionGroup[]>;
  abstract queryAnomalies(query: QlQuery): Promise<Anomaly[]>;
  abstract compareWindows(a: QlQuery, b: QlQuery): Promise<Comparison>;
  abstract fieldStats(query: QlQuery, field: string, buckets: number): Promise<FieldStats>;
  /** Ids of the events whose `field` equals `value`. */
  abstract correlate(field: string, value: string): Promise<number[]>;
  /** The trace list, plus the waterfall of `traceId` when it is not empty. */
  abstract queryTraces(query: QlQuery, traceId: string): Promise<TracesResult>;
  abstract queryRequests(query: QlQuery, idField: string): Promise<RequestSpan[]>;
  abstract queryMoment(query: QlQuery, from: number, to: number): Promise<MomentSummary>;
}

/** Talks to the Rust core through Tauri commands (see `api.types.ts` for the contract). */
export class TauriAnalysisBackend extends AnalysisBackend {
  /** `call` is Tauri's `invoke`; tests pass a fake. */
  constructor(private readonly call: typeof invoke = invoke) {
    super();
  }

  runQuery(query: QlQuery) {
    return this.call<QueryOutcome>('run_query', { request: { query } });
  }
  checkQuery(text: string) {
    return this.call<QlError | null>('check_query', { text });
  }
  completeQuery(text: string, cursor: number) {
    return this.call<Completion[]>('complete_query', { request: { text, cursor } });
  }
  queryPatterns(query: QlQuery) {
    return this.call<Pattern[]>('query_patterns', { request: { query } });
  }
  queryExceptions(query: QlQuery) {
    return this.call<ExceptionGroup[]>('query_exceptions', { request: { query } });
  }
  queryAnomalies(query: QlQuery) {
    return this.call<Anomaly[]>('query_anomalies', { request: { query } });
  }
  compareWindows(a: QlQuery, b: QlQuery) {
    return this.call<Comparison>('compare_windows', { request: { a, b } });
  }
  fieldStats(query: QlQuery, field: string, buckets: number) {
    return this.call<FieldStats>('field_stats', { request: { query, field, buckets } });
  }
  correlate(field: string, value: string) {
    return this.call<number[]>('correlate', { request: { field, value } });
  }
  queryTraces(query: QlQuery, traceId: string) {
    return this.call<TracesResult>('query_traces', { request: { query, traceId } });
  }
  queryRequests(query: QlQuery, idField: string) {
    return this.call<RequestSpan[]>('query_requests', { request: { query, idField } });
  }
  queryMoment(query: QlQuery, from: number, to: number) {
    return this.call<MomentSummary>('query_moment', { request: { query, from, to } });
  }
}
