import { inject, Injectable, Injector } from '@angular/core';
import { invoke } from '../core/activity/ipc';
import { HomeRequest, HomeSummary } from '../core/api.types';
import { AnalysisBackend } from '../core/analysis-backend';
import { ImtBackend } from '../core/backend';
import { reportBackendFailures } from '../core/failure-reporter';
import { Logger, withFailureLogging } from '../core/logger';
import { browserMocks } from '../core/mock-registry';
import { isTauri } from '../core/platform';

/**
 * The Home dashboard's engine query (`home_summary`): counts, the history by level with its markers and anomalies,
 * and the exception groups, computed by the engine in one bounded, side-effect-free request. Injected as a class
 * token: the Tauri command in the app, the mock in a plain browser. Rejects with a `CmdError`.
 */
@Injectable({ providedIn: 'root', useFactory: createHomeBackend })
export abstract class HomeBackend {
  abstract querySummary(request: HomeRequest): Promise<HomeSummary>;
}

/** Talks to `workspace_commands::home_summary`. */
export class TauriHomeBackend extends HomeBackend {
  /** `call` is Tauri's `invoke`; tests pass a fake. */
  constructor(private readonly call: typeof invoke = invoke) {
    super();
  }
  querySummary(request: HomeRequest) {
    return this.call<HomeSummary>('home_summary', { request });
  }
}

export function createHomeBackend(): HomeBackend {
  const injector = inject(Injector);
  const backend: HomeBackend = isTauri() ? new TauriHomeBackend() : new (browserMocks().MockHomeBackend)(inject(ImtBackend), () => injector.get(AnalysisBackend));
  return withFailureLogging<HomeBackend>(backend, inject(Logger), { onFailure: reportBackendFailures() });
}
