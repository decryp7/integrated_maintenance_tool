import { inject, Provider } from '@angular/core';
import { EtlConverted, EtlProcess, NativeCaptureMode, NativeCaptureRequest, NativeCaptureStatus, NativeDefaults, NativeProcess } from '../../core/api.types';
import { reportBackendFailures } from '../../core/failure-reporter';
import { Logger, withFailureLogging } from '../../core/logger';
import { isTauri } from '../../core/platform';
import { ToolCatalogBackend } from '../../core/tools-backend';
import { JobsBackend } from '../../jobs/jobs-backend';

/** The tool of the native plugin's normal process: listing, suspend-and-sample, `.etl` (crates/debrief-plugin-native). */
export const NATIVE_TOOL = 'native-profiler';
/** The tool of its elevated process: ETW captures (the first call starts it through the administrator prompt). */
export const NATIVE_ETW_TOOL = 'native-profiler.etw';

/** The tool that records captures of `mode`. */
export function toolOf(mode: NativeCaptureMode): string {
  return mode === 'etw' ? NATIVE_ETW_TOOL : NATIVE_TOOL;
}

/**
 * Native (C/C++) profiling as the UI sees it: the two tools of the native profiling plugin, reached through the
 * generic `tool_invoke`. Running captures are tool tasks, so they also appear in the Jobs pane.
 */
export abstract class NativeBackend {
  /** The settings' symbol options and the sampling bounds. */
  abstract defaults(): Promise<NativeDefaults>;
  abstract processes(): Promise<NativeProcess[]>;
  abstract start(mode: NativeCaptureMode, request: NativeCaptureRequest): Promise<NativeCaptureStatus>;
  /** The captures of `modes`, newest first (polled while one records). ETW is asked only once it was used. */
  abstract captures(modes: readonly NativeCaptureMode[]): Promise<NativeCaptureStatus[]>;
  /** Stops and keeps the capture (symbols are resolved first). */
  abstract stop(mode: NativeCaptureMode, id: string): Promise<NativeCaptureStatus>;
  /** Stops at once and deletes the file (through the capture's job: Jobs are the one cancel authority). */
  abstract cancel(mode: NativeCaptureMode, id: string): Promise<NativeCaptureStatus>;
  abstract etlProcesses(path: string): Promise<EtlProcess[]>;
  abstract etlConvert(path: string, pid: number): Promise<EtlConverted>;
}

/** The real backend: the plugin tools; cancelling goes through the capture's job. */
export class ToolNativeBackend extends NativeBackend {
  constructor(
    private readonly tools: ToolCatalogBackend,
    private readonly jobs: JobsBackend,
  ) {
    super();
  }

  defaults() {
    return this.tools.toolInvoke<NativeDefaults>(NATIVE_TOOL, 'defaults', {});
  }
  processes() {
    return this.tools.toolInvoke<NativeProcess[]>(NATIVE_TOOL, 'processes', {});
  }
  start(mode: NativeCaptureMode, request: NativeCaptureRequest) {
    return this.tools.toolInvoke<NativeCaptureStatus>(toolOf(mode), 'start', request);
  }
  async captures(modes: readonly NativeCaptureMode[]) {
    const lists = await Promise.all(modes.map((m) => this.tools.toolPoll<{ captures: NativeCaptureStatus[] }>(toolOf(m), 'status', {})));
    return lists.flatMap((l) => l.captures);
  }
  stop(mode: NativeCaptureMode, id: string) {
    return this.tools.toolInvoke<NativeCaptureStatus>(toolOf(mode), 'stop', { id });
  }
  async cancel(mode: NativeCaptureMode, id: string) {
    const job = await this.tools.toolTaskJob(id);
    if (job === null) throw new Error($localize`:@@profiling.native.noJob:This capture is not running any more.`);
    await this.jobs.cancel(job);
    const status = (await this.captures([mode])).find((c) => c.id === id);
    if (!status) throw new Error($localize`:@@profiling.native.gone:The capture is no longer listed.`);
    return status;
  }
  etlProcesses(path: string) {
    return this.tools.toolInvoke<EtlProcess[]>(NATIVE_TOOL, 'etl-processes', { path });
  }
  etlConvert(path: string, pid: number) {
    return this.tools.toolInvoke<EtlConverted>(NATIVE_TOOL, 'etl-convert', { path, pid });
  }
}

/* v8 ignore start: the browser demo loads the mock lazily, so it stays out of the shipped bundle */
class LazyMockNativeBackend extends NativeBackend {
  private readonly impl = import('./mock-native-backend').then((m) => new m.MockNativeBackend());
  defaults() {
    return this.impl.then((b) => b.defaults());
  }
  processes() {
    return this.impl.then((b) => b.processes());
  }
  start(mode: NativeCaptureMode, r: NativeCaptureRequest) {
    return this.impl.then((b) => b.start(mode, r));
  }
  captures(modes: readonly NativeCaptureMode[]) {
    return this.impl.then((b) => b.captures(modes));
  }
  stop(mode: NativeCaptureMode, id: string) {
    return this.impl.then((b) => b.stop(mode, id));
  }
  cancel(mode: NativeCaptureMode, id: string) {
    return this.impl.then((b) => b.cancel(mode, id));
  }
  etlProcesses(path: string) {
    return this.impl.then((b) => b.etlProcesses(path));
  }
  etlConvert(path: string, pid: number) {
    return this.impl.then((b) => b.etlConvert(path, pid));
  }
}
/* v8 ignore stop */

/** The plugin tools inside Tauri, the mock in a plain browser (ng serve, e2e). */
export function provideNativeBackend(): Provider[] {
  const factory = () => {
    const real = isTauri() ? new ToolNativeBackend(inject(ToolCatalogBackend), inject(JobsBackend)) : new LazyMockNativeBackend();
    return withFailureLogging<NativeBackend>(real, inject(Logger), { onFailure: reportBackendFailures() });
  };
  return [{ provide: NativeBackend, useFactory: factory }];
}
