import { invoke } from './activity/ipc';
import {
  Analyzed,
  Anomaly,
  Comparison,
  Completion,
  ExceptionGroup,
  FieldStats,
  MomentSummary,
  Pattern,
  QlError,
  QlQuery,
  SearchInputKind,
  QueryOutcome,
  QueryProgress,
  QueryStreamEvent,
  RequestSpan,
  StreamedQueryOutcome,
  TracesResult,
} from './api.types';
import { ChannelFactory, tauriChannel } from './tauri-channel';
import { cancelJob } from '../jobs/job-cancel';

/** A streamed run: its job once known, and whether a cancel was asked for. */
interface StreamRun {
  job: number | null;
  cancelled: boolean;
}

/**
 * Everything the analysis views ask the engine. Injected as a class token (see
 * `provideAnalysis()` for the Tauri / mock selection). Every method is a pure query.
 */
export abstract class AnalysisBackend {
  /** Runs `query.ql`. Query-language mistakes come back as `{ error }`, not as a rejection. */
  abstract runQuery(query: QlQuery): Promise<QueryOutcome>;
  abstract checkQuery(text: string): Promise<QlError | null>;
  /** Is the search field's text a message search or a query? `fields`: the workspace's field names. */
  abstract classifySearch(text: string, fields: readonly string[]): Promise<SearchInputKind>;
  /** `cursor` is a UTF-8 byte offset into `text`. */
  abstract completeQuery(text: string, cursor: number): Promise<Completion[]>;
  // The analyses resolve with their result and how much of the matching data they looked at (`Analyzed`).
  abstract queryPatterns(query: QlQuery): Promise<Analyzed<Pattern[]>>;
  abstract queryExceptions(query: QlQuery): Promise<Analyzed<ExceptionGroup[]>>;
  abstract queryAnomalies(query: QlQuery): Promise<Analyzed<Anomaly[]>>;
  abstract compareWindows(a: QlQuery, b: QlQuery): Promise<Analyzed<Comparison>>;
  abstract fieldStats(query: QlQuery, field: string, buckets: number): Promise<FieldStats>;
  /** Ids of the events whose `field` equals `value`. */
  abstract correlate(field: string, value: string): Promise<Analyzed<number[]>>;
  /** The trace list, plus the waterfall of `traceId` when it is not empty, with the coverage. */
  abstract queryTraces(query: QlQuery, traceId: string): Promise<TracesResult>;
  abstract queryRequests(query: QlQuery, idField: string): Promise<Analyzed<RequestSpan[]>>;
  abstract queryMoment(query: QlQuery, from: number, to: number): Promise<Analyzed<MomentSummary>>;

  /**
   * Like `runQuery`, reporting progress and cancellable with `cancelQuery(queryId)`; a cancelled run resolves to
   * `{ cancelled: true }`. A backend without cancellation runs `runQuery` to completion (the default).
   */
  runQueryStreaming(query: QlQuery, _queryId: string, _onProgress: (p: QueryProgress) => void): Promise<StreamedQueryOutcome> {
    return this.runQuery(query);
  }
  /** Asks the run `queryId` to stop; false when it is not running (any more). */
  cancelQuery(_queryId: string): Promise<boolean> {
    return Promise.resolve(false);
  }
}

/** Talks to the Rust core through Tauri commands (see `api.types.ts` for the contract). */
export class TauriAnalysisBackend extends AnalysisBackend {
  /** `call` is Tauri's `invoke` and `channel` makes IPC channels; tests pass fakes. */
  constructor(
    private readonly call: typeof invoke = invoke,
    private readonly channel: ChannelFactory = tauriChannel,
  ) {
    super();
  }

  /**
   * The run is a job: its id is the channel's first event, and `cancelQuery` cancels that job (Jobs are
   * the one cancel authority). A cancel that comes before the id is applied when the id arrives.
   */
  override async runQueryStreaming(query: QlQuery, queryId: string, onProgress: (p: QueryProgress) => void) {
    const run: StreamRun = { job: null, cancelled: false };
    this.runs.set(queryId, run);
    const onEvent = (e: QueryStreamEvent) => {
      if (e.kind === 'started') {
        run.job = e.jobId;
        if (run.cancelled) void cancelJob(this.call, e.jobId).catch(() => undefined);
      } else {
        const { kind: _kind, ...progress } = e;
        onProgress(progress);
      }
    };
    try {
      return await this.call<StreamedQueryOutcome>('run_query_stream', { request: { query }, onProgress: this.channel(onEvent) });
    } finally {
      this.runs.delete(queryId);
    }
  }
  override async cancelQuery(queryId: string) {
    const run = this.runs.get(queryId);
    if (!run) return false;
    run.cancelled = true;
    if (run.job !== null) await cancelJob(this.call, run.job);
    return true;
  }
  /** The streamed runs in flight, by the UI's query id. */
  private readonly runs = new Map<string, StreamRun>();

  runQuery(query: QlQuery) {
    return this.call<QueryOutcome>('run_query', { request: { query } });
  }
  checkQuery(text: string) {
    return this.call<QlError | null>('check_query', { text });
  }
  classifySearch(text: string, fields: readonly string[]) {
    return this.call<SearchInputKind>('classify_search', { text, fields });
  }
  completeQuery(text: string, cursor: number) {
    return this.call<Completion[]>('complete_query', { request: { text, cursor } });
  }
  queryPatterns(query: QlQuery) {
    return this.call<Analyzed<Pattern[]>>('query_patterns', { request: { query } });
  }
  queryExceptions(query: QlQuery) {
    return this.call<Analyzed<ExceptionGroup[]>>('query_exceptions', { request: { query } });
  }
  queryAnomalies(query: QlQuery) {
    return this.call<Analyzed<Anomaly[]>>('query_anomalies', { request: { query } });
  }
  compareWindows(a: QlQuery, b: QlQuery) {
    return this.call<Analyzed<Comparison>>('compare_windows', { request: { a, b } });
  }
  fieldStats(query: QlQuery, field: string, buckets: number) {
    return this.call<FieldStats>('field_stats', { request: { query, field, buckets } });
  }
  correlate(field: string, value: string) {
    return this.call<Analyzed<number[]>>('correlate', { request: { field, value } });
  }
  queryTraces(query: QlQuery, traceId: string) {
    return this.call<TracesResult>('query_traces', { request: { query, traceId } });
  }
  queryRequests(query: QlQuery, idField: string) {
    return this.call<Analyzed<RequestSpan[]>>('query_requests', { request: { query, idField } });
  }
  queryMoment(query: QlQuery, from: number, to: number) {
    return this.call<Analyzed<MomentSummary>>('query_moment', { request: { query, from, to } });
  }
}
