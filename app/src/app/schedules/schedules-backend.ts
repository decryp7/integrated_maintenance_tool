import { inject, Injectable } from '@angular/core';
import { invoke } from '../core/activity/ipc';
import { AgentConfigDoc, AgentConfigFile, AgentIssue, AgentSchema, RunManifest, RunsListing } from '../core/agent.types';
import { reportBackendFailures } from '../core/failure-reporter';
import { Logger, withFailureLogging } from '../core/logger';
import { isTauri } from '../core/platform';
import { MockSchedulesBackend } from './mock/mock-schedules-backend';

/** The real backend inside Tauri, the in-memory mock in a plain browser; failures are logged and reported. */
export function createSchedulesBackend(): SchedulesBackend {
  const real = isTauri() ? new TauriSchedulesBackend() : new MockSchedulesBackend();
  return withFailureLogging<SchedulesBackend>(real, inject(Logger), { onFailure: reportBackendFailures() });
}

/**
 * The scheduled-collection agent's config (`imt-agent.toml`) and its runs. `path` is the config
 * file; omitted, the default one in IMT's config folder. Validation is the agent library's.
 */
@Injectable({ providedIn: 'root', useFactory: createSchedulesBackend })
export abstract class SchedulesBackend {
  abstract schema(): Promise<AgentSchema>;
  /** The config, or a starter document when the file does not exist yet. */
  abstract load(path?: string): Promise<AgentConfigDoc>;
  abstract validate(config: AgentConfigFile, path?: string): Promise<AgentIssue[]>;
  /**
   * Saves a valid config; an invalid one is not written and its issues are returned. An existing file is
   * replaced only with `overwrite` (else the engine's FILE_EXISTS error); its comments are kept.
   */
  abstract save(config: AgentConfigFile, path?: string, overwrite?: boolean): Promise<AgentIssue[]>;
  abstract runs(path?: string): Promise<RunsListing>;
  abstract manifest(runId: string, path?: string): Promise<RunManifest>;
  /** Opens the run's folder in a new window; resolves to the window label. */
  abstract openRun(runId: string, path?: string): Promise<string>;
}

/** Talks to the Rust shell; `call` is injectable for tests. */
export class TauriSchedulesBackend extends SchedulesBackend {
  constructor(private readonly call: typeof invoke = invoke) {
    super();
  }

  schema() {
    return this.call<AgentSchema>('agent_schema');
  }
  load(path?: string) {
    return this.call<AgentConfigDoc>('agent_config_load', { path: path ?? null });
  }
  validate(config: AgentConfigFile, path?: string) {
    return this.call<AgentIssue[]>('agent_config_validate', { path: path ?? null, config });
  }
  save(config: AgentConfigFile, path?: string, overwrite = false) {
    return this.call<AgentIssue[]>('agent_config_save', { path: path ?? null, config, ...(overwrite ? { overwrite } : {}) });
  }
  runs(path?: string) {
    return this.call<RunsListing>('agent_runs', { path: path ?? null });
  }
  manifest(runId: string, path?: string) {
    return this.call<RunManifest>('agent_run_manifest', { path: path ?? null, runId });
  }
  openRun(runId: string, path?: string) {
    return this.call<string>('agent_open_run', { path: path ?? null, runId });
  }
}
