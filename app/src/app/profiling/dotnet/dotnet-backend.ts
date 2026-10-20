import { inject, Provider } from '@angular/core';
import { DotnetCaptureRequest, DotnetCaptureStatus, DotnetProcess, EtlConverted, EtlProcess } from '../../core/api.types';
import { isTauri } from '../../core/platform';
import { ToolCatalogBackend } from '../../core/tools-backend';
import { JobsBackend } from '../../jobs/jobs-backend';
import { reportBackendFailures } from '../../core/failure-reporter';
import { Logger, withFailureLogging } from '../../core/logger';

/** The plugin tool that records .NET processes and converts `.etl` traces (crates/imt-plugin-dotnet). */
export const DOTNET_TOOL = 'dotnet-profiler';

/**
 * .NET profiling as the UI sees it: the `dotnet-profiler` tool of the .NET profiling plugin, reached through the
 * generic `tool_invoke`. Running captures are tool tasks, so they also appear in the Jobs pane.
 */
export abstract class DotnetBackend {
  /** .NET processes that can be recorded. */
  abstract processes(): Promise<DotnetProcess[]>;
  abstract start(request: DotnetCaptureRequest): Promise<DotnetCaptureStatus>;
  /** Every capture, newest first (polled while one records). */
  abstract captures(): Promise<DotnetCaptureStatus[]>;
  /** Stops and keeps the capture (method names are collected first). */
  abstract stop(id: string): Promise<DotnetCaptureStatus>;
  /** Stops at once and deletes the file (through the capture's job: Jobs are the one cancel authority). */
  abstract cancel(id: string): Promise<DotnetCaptureStatus>;
  /** The processes in an `.etl` trace (Windows only). */
  abstract etlProcesses(path: string): Promise<EtlProcess[]>;
  /** Converts one process of an `.etl` trace to a NetTrace file. */
  abstract etlConvert(path: string, pid: number): Promise<EtlConverted>;
}

/** The real backend: the plugin tool; cancelling goes through the capture's job. */
export class ToolDotnetBackend extends DotnetBackend {
  constructor(
    private readonly tools: ToolCatalogBackend,
    private readonly jobs: JobsBackend,
  ) {
    super();
  }

  private call<T>(op: string, args: unknown = {}): Promise<T> {
    return this.tools.toolInvoke<T>(DOTNET_TOOL, op, args);
  }

  processes() {
    return this.call<DotnetProcess[]>('processes');
  }
  start(request: DotnetCaptureRequest) {
    return this.call<DotnetCaptureStatus>('start', request);
  }
  async captures() {
    const r = await this.tools.toolPoll<{ captures: DotnetCaptureStatus[] }>(DOTNET_TOOL, 'status', {});
    return r.captures;
  }
  stop(id: string) {
    return this.call<DotnetCaptureStatus>('stop', { id });
  }
  async cancel(id: string) {
    const job = await this.tools.toolTaskJob(id);
    if (job === null) throw new Error($localize`:@@profiling.dotnet.noJob:This capture is not running any more.`);
    await this.jobs.cancel(job);
    const status = (await this.captures()).find((c) => c.id === id);
    if (!status) throw new Error($localize`:@@profiling.dotnet.gone:The capture is no longer listed.`);
    return status;
  }
  etlProcesses(path: string) {
    return this.call<EtlProcess[]>('etl-processes', { path });
  }
  etlConvert(path: string, pid: number) {
    return this.call<EtlConverted>('etl-convert', { path, pid });
  }
}

/* v8 ignore start: the browser demo loads the mock lazily, so it stays out of the shipped bundle */
class LazyMockDotnetBackend extends DotnetBackend {
  private readonly impl = import('./mock-dotnet-backend').then((m) => new m.MockDotnetBackend());
  processes() {
    return this.impl.then((b) => b.processes());
  }
  start(r: DotnetCaptureRequest) {
    return this.impl.then((b) => b.start(r));
  }
  captures() {
    return this.impl.then((b) => b.captures());
  }
  stop(id: string) {
    return this.impl.then((b) => b.stop(id));
  }
  cancel(id: string) {
    return this.impl.then((b) => b.cancel(id));
  }
  etlProcesses(path: string) {
    return this.impl.then((b) => b.etlProcesses(path));
  }
  etlConvert(path: string, pid: number) {
    return this.impl.then((b) => b.etlConvert(path, pid));
  }
}
/* v8 ignore stop */

/** The plugin tool inside Tauri, the mock in a plain browser (ng serve, e2e). */
export function provideDotnetBackend(): Provider[] {
  const factory = () => {
    const real = isTauri() ? new ToolDotnetBackend(inject(ToolCatalogBackend), inject(JobsBackend)) : new LazyMockDotnetBackend();
    return withFailureLogging<DotnetBackend>(real, inject(Logger), { onFailure: reportBackendFailures() });
  };
  return [{ provide: DotnetBackend, useFactory: factory }];
}
