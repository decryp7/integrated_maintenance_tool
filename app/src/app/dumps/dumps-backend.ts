import { inject, Provider } from '@angular/core';
import { jobValue } from '../jobs/job-value';
import { invoke as tauriInvoke } from '../core/activity/ipc';
import {
  DumpDocSummary,
  DumpFinding,
  DumpFindingsRequest,
  DumpMarker,
  DumpOverview,
  DumpSniffResult,
  ModulesPage,
  ModulesRequest,
  StackDto,
  StackRequest,
  ThreadsPage,
  ThreadsRequest,
  UmdhDiffPage,
  UmdhDiffRequest,
  UmdhStackDto,
  UmdhStackRequest,
  UmdhTreeNode,
  UmdhTreeRequest,
} from '../core/api.types';
import { reportBackendFailures } from '../core/failure-reporter';
import { Logger, withFailureLogging } from '../core/logger';
import { browserMocks } from '../core/mock-registry';
import { isTauri } from '../core/platform';
import { listenHere, PayloadListenFn, Unlisten } from '../windowing/window-events';

/** Emitted to the window when the symbols of one of its dumps changed (PDBs arrived or the download ended): `{ id }`. */
export const DUMP_SYMBOLS_EVENT = 'debrief://dump-symbols';

/**
 * The dump engine as the UI sees it (commands in `app/src-tauri/src/dumps.rs`, ADR 0023). The engine reads,
 * analyses and words everything (findings, culprit frames, growth, the AI summary); views only present it.
 * Queries have no side effects; only `open` and `close` change state. Every method rejects with a `CmdError`.
 */
export abstract class DumpsBackend {
  /** What each path is (dump, UMDH log or neither), without opening it: dropped files are routed with it. */
  abstract sniff(paths: string[]): Promise<DumpSniffResult[]>;
  /** Opens a dump or UMDH log; the engine recognises it by content. */
  abstract open(path: string): Promise<DumpDocSummary>;
  abstract list(): Promise<DumpDocSummary[]>;
  abstract close(id: number): Promise<boolean>;
  abstract overview(id: number): Promise<DumpOverview>;
  abstract threads(request: ThreadsRequest): Promise<ThreadsPage>;
  abstract stack(request: StackRequest): Promise<StackDto>;
  abstract modules(request: ModulesRequest): Promise<ModulesPage>;
  /** Findings of one dump, or of UMDH logs of one process. */
  abstract findings(request: DumpFindingsRequest): Promise<DumpFinding[]>;
  abstract umdhDiff(request: UmdhDiffRequest): Promise<UmdhDiffPage>;
  abstract umdhStack(request: UmdhStackRequest): Promise<UmdhStackDto>;
  abstract umdhTree(request: UmdhTreeRequest): Promise<UmdhTreeNode[]>;
  /** The open documents with a time, for the log timeline. */
  abstract markers(): Promise<DumpMarker[]>;
  /** The engine's compact, redacted question for "Explain with AI". */
  abstract explain(request: DumpFindingsRequest): Promise<string>;
  /** Told with the document's id when its symbols changed (downloaded PDBs arrived): views read the dump again. */
  abstract onSymbolsChanged(handler: (id: number) => void): Promise<Unlisten>;
}

/** Talks to the Rust shell; `invoke` is Tauri's (tests pass a fake). */
export class TauriDumpsBackend extends DumpsBackend {
  constructor(
    private readonly invoke: typeof tauriInvoke = tauriInvoke,
    private readonly listen: PayloadListenFn = listenHere,
  ) {
    super();
  }

  sniff(paths: string[]) {
    return this.invoke<DumpSniffResult[]>('dumps_sniff', { paths });
  }
  /** The command starts an import job and returns its id; the opened document is the job's value. */
  async open(path: string) {
    const job = await this.invoke<number>('dumps_open', { path });
    return jobValue<DumpDocSummary>(this.invoke, job);
  }
  list() {
    return this.invoke<DumpDocSummary[]>('dumps_list');
  }
  close(id: number) {
    return this.invoke<boolean>('dumps_close', { id });
  }
  overview(id: number) {
    return this.invoke<DumpOverview>('dumps_overview', { id });
  }
  threads(request: ThreadsRequest) {
    return this.invoke<ThreadsPage>('dumps_threads', { request });
  }
  stack(request: StackRequest) {
    return this.invoke<StackDto>('dumps_stack', { request });
  }
  modules(request: ModulesRequest) {
    return this.invoke<ModulesPage>('dumps_modules', { request });
  }
  findings(request: DumpFindingsRequest) {
    return this.invoke<DumpFinding[]>('dumps_findings', { request });
  }
  umdhDiff(request: UmdhDiffRequest) {
    return this.invoke<UmdhDiffPage>('dumps_umdh_diff', { request });
  }
  umdhStack(request: UmdhStackRequest) {
    return this.invoke<UmdhStackDto>('dumps_umdh_stack', { request });
  }
  umdhTree(request: UmdhTreeRequest) {
    return this.invoke<UmdhTreeNode[]>('dumps_umdh_tree', { request });
  }
  markers() {
    return this.invoke<DumpMarker[]>('dumps_markers');
  }
  explain(request: DumpFindingsRequest) {
    return this.invoke<string>('dumps_explain', { request });
  }
  onSymbolsChanged(handler: (id: number) => void) {
    return this.listen(DUMP_SYMBOLS_EVENT, (p) => {
      const id = (p as { id?: unknown } | null)?.id;
      if (typeof id === 'number') handler(id);
    });
  }
}

/** The real backend inside Tauri, the in-memory mock in a browser (dev server, e2e). */
export function createDumpsBackend(): DumpsBackend {
  const real = isTauri() ? new TauriDumpsBackend() : new (browserMocks().MockDumpsBackend)();
  return withFailureLogging<DumpsBackend>(real, inject(Logger), { onFailure: reportBackendFailures() });
}

export function provideDumpsBackend(): Provider[] {
  return [{ provide: DumpsBackend, useFactory: createDumpsBackend }];
}
