import { inject, Injectable } from '@angular/core';
import { invoke } from '../core/activity/ipc';
import {
  MaintenanceAction,
  QueryStreamEvent,
  SegmentsPage,
  SegmentsRequest,
  SqlIds,
  SqlOutcome,
  SqlPage,
  SqlPageRequest,
  SqlSchema,
  StoreFields,
  StoreIndex,
  StoreMemory,
  StoreOverview,
} from '../core/api.types';
import { ImtBackend } from '../core/backend';
import { reportBackendFailures } from '../core/failure-reporter';
import { Logger, withFailureLogging } from '../core/logger';
import { browserMocks } from '../core/mock-registry';
import { isTauri } from '../core/platform';
import { ChannelFactory, tauriChannel } from '../core/tauri-channel';
import { cancelJob } from '../jobs/job-cancel';

/**
 * The Store pane's commands (ADR 0030): read-only reports about the window's store, its maintenance as jobs, and the
 * read-only SQL console. The engine computes and checks everything; the UI shows it. Injected as a class token: the
 * Tauri commands in the app, the mock in a plain browser. Every method rejects with a `CmdError`.
 */
@Injectable({ providedIn: 'root', useFactory: createStorePaneBackend })
export abstract class StorePaneBackend {
  abstract overview(): Promise<StoreOverview>;
  abstract listSegments(request: SegmentsRequest): Promise<SegmentsPage>;
  abstract listFields(): Promise<StoreFields>;
  abstract describeIndex(): Promise<StoreIndex>;
  abstract describeMemory(): Promise<StoreMemory>;
  /** Starts the action as a job; resolves with its id. */
  abstract maintain(action: MaintenanceAction): Promise<number>;
  abstract describeSqlSchema(): Promise<SqlSchema>;
  /** Runs a console query as a job; `runId` names it for `cancelSql`. */
  abstract querySql(sql: string, explain: boolean, runId: string): Promise<SqlOutcome>;
  /** Stops the run `runId` (its job); false when it is not running. */
  abstract cancelSql(runId: string): Promise<boolean>;
  abstract fetchSqlPage(request: SqlPageRequest): Promise<SqlPage>;
  /** The event ids of the kept result `result` (for Open all in Events). */
  abstract fetchSqlIds(result: number): Promise<SqlIds>;
  /** Starts writing all rows of `sql` as CSV to `path` as an export job; resolves with its id. */
  abstract exportSql(sql: string, path: string, overwrite: boolean): Promise<number>;
}

/** A run in flight: its job (once the engine said it) and whether a stop came first. */
interface Run {
  job: number | null;
  cancelled: boolean;
}

/** Talks to `store_pane::*` (see `api.types.ts`). */
export class TauriStorePaneBackend extends StorePaneBackend {
  private readonly runs = new Map<string, Run>();

  /** `call` is Tauri's `invoke` and `channel` makes IPC channels; tests pass fakes. */
  constructor(
    private readonly call: typeof invoke = invoke,
    private readonly channel: ChannelFactory = tauriChannel,
  ) {
    super();
  }
  overview() {
    return this.call<StoreOverview>('store_overview');
  }
  listSegments(request: SegmentsRequest) {
    return this.call<SegmentsPage>('store_segments', { request });
  }
  listFields() {
    return this.call<StoreFields>('store_fields');
  }
  describeIndex() {
    return this.call<StoreIndex>('store_index');
  }
  describeMemory() {
    return this.call<StoreMemory>('store_memory');
  }
  maintain(action: MaintenanceAction) {
    return this.call<number>('store_maintenance', { action });
  }
  describeSqlSchema() {
    return this.call<SqlSchema>('sql_schema');
  }
  async querySql(sql: string, explain: boolean, runId: string) {
    const run: Run = { job: null, cancelled: false };
    this.runs.set(runId, run);
    const onEvent = (e: QueryStreamEvent) => {
      if (e.kind !== 'started') return;
      run.job = e.jobId;
      if (run.cancelled) void cancelJob(this.call, e.jobId).catch(() => undefined);
    };
    try {
      return await this.call<SqlOutcome>('sql_run', { sql, explain, onEvent: this.channel(onEvent) });
    } finally {
      this.runs.delete(runId);
    }
  }
  async cancelSql(runId: string) {
    const run = this.runs.get(runId);
    if (!run) return false;
    run.cancelled = true;
    if (run.job !== null) await cancelJob(this.call, run.job);
    return true;
  }
  fetchSqlPage(request: SqlPageRequest) {
    return this.call<SqlPage>('sql_page', { request });
  }
  fetchSqlIds(result: number) {
    return this.call<SqlIds>('sql_ids', { request: { result } });
  }
  exportSql(sql: string, path: string, overwrite: boolean) {
    return this.call<number>('sql_export', { sql, path, overwrite });
  }
}

export function createStorePaneBackend(): StorePaneBackend {
  const backend: StorePaneBackend = isTauri() ? new TauriStorePaneBackend() : new (browserMocks().MockStorePaneBackend)(inject(ImtBackend));
  return withFailureLogging<StorePaneBackend>(backend, inject(Logger), { onFailure: reportBackendFailures() });
}
