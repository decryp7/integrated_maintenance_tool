import { inject, Injectable, Provider } from '@angular/core';
import { invoke } from '../core/activity/ipc';
import { RunsReply, RunsRequest, SpanPreview, SpanPreviewRequest } from '../core/api.types';
import { ImtBackend } from '../core/backend';
import { reportBackendFailures } from '../core/failure-reporter';
import { Logger, withFailureLogging } from '../core/logger';
import { isTauri } from '../core/platform';

/** Runs between lifecycle markers: the Runs view and the Runs editor's preview. Injected as a class token (root default: `createRunsBackend`). */
@Injectable({ providedIn: 'root', useFactory: () => createRunsBackend() })
export abstract class RunsBackend {
  /** A sorted page of runs and every span kind's summary over the selection. A pure query. */
  abstract queryRuns(request: RunsRequest): Promise<RunsReply>;
  /** Field errors, counts, first runs and anomalies of a candidate rule, and what saving it writes. A pure query. */
  abstract previewSpans(request: SpanPreviewRequest): Promise<SpanPreview>;
}

/** Talks to the Rust core through Tauri commands (see `api.types.ts` for the contract). */
export class TauriRunsBackend extends RunsBackend {
  /** `call` is Tauri's `invoke`; tests pass a fake. */
  constructor(private readonly call: typeof invoke = invoke) {
    super();
  }

  queryRuns(request: RunsRequest) {
    return this.call<RunsReply>('query_runs', { request });
  }
  previewSpans(request: SpanPreviewRequest) {
    return this.call<SpanPreview>('preview_spans', { request });
  }
}

/** True when a backend object also implements the runs commands (the mock backend does). */
export function hasRuns(backend: unknown): backend is RunsBackend {
  const b = backend as Partial<RunsBackend> | null;
  return !!b && typeof b.queryRuns === 'function' && typeof b.previewSpans === 'function';
}

/** A backend without runs: nothing to list, and a preview that says why. */
export class UnsupportedRunsBackend extends RunsBackend {
  queryRuns(): Promise<RunsReply> {
    return Promise.resolve({ summaries: [], rules: [], runs: [], total: 0, offset: 0 });
  }
  previewSpans(): Promise<SpanPreview> {
    return Promise.reject($localize`:@@runs.backend.unsupported:this backend cannot preview runs`);
  }
}

/** The Rust backend inside Tauri; in a plain browser the in-memory mock workspace (which logs its own failures). */
export function createRunsBackend(): RunsBackend {
  if (!isTauri()) {
    const workspace = inject(ImtBackend);
    return hasRuns(workspace) ? workspace : new UnsupportedRunsBackend();
  }
  return withFailureLogging<RunsBackend>(new TauriRunsBackend(), inject(Logger), { onFailure: reportBackendFailures() });
}

export function provideRunsBackend(): Provider {
  return { provide: RunsBackend, useFactory: createRunsBackend };
}
