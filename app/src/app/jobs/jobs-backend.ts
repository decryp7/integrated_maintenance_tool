import { inject, Injectable, Injector } from '@angular/core';
import { invoke } from '../core/activity/ipc';
import { reportBackendFailures } from '../core/failure-reporter';
import { Logger, withFailureLogging } from '../core/logger';
import { isTauri } from '../core/platform';
import { listenHere, PayloadListenFn, Unlisten } from '../windowing/window-events';
import { JobInfo, JobOutcome, JobResult, JobsDelta, JobsRecord, JobsSnapshot, RestoredJobs } from './jobs.types';

export const JOBS_EVENT = 'imt://jobs';

/**
 * The window's jobs (contract: `jobs.types.ts`). Injected as a class token: the Tauri commands in the desktop
 * app, the mock in the browser build (dev and UI tests).
 */
@Injectable({ providedIn: 'root', useFactory: () => createJobsBackend() })
export abstract class JobsBackend {
  abstract list(): Promise<JobsSnapshot>;
  abstract cancel(id: number): Promise<void>;
  /** Runs a failed, cancelled or interrupted job again; the new job, or null when the window starts it. */
  abstract retry(id: number): Promise<number | null>;
  /** Moves a queued AI job one place up or down. */
  abstract move(id: number, up: boolean): Promise<void>;
  abstract clearFinished(): Promise<number>;
  abstract markSeen(id: number): Promise<boolean>;
  /** What View opens (marks the job seen). */
  abstract result(id: number): Promise<JobResult | null>;
  /** Shows an export's file in Finder or Explorer; resolves with its path. */
  abstract reveal(id: number): Promise<string>;
  /** Waits for the job to end (a dialog following it): its state and whole result. A job someone waits for
   * posts no message when it ends; the waiter shows the outcome. */
  abstract wait(id: number): Promise<JobOutcome>;
  /** Stops waiting (the dialog closed): the job tells the user when it is done. */
  abstract unwait(id: number): Promise<void>;
  abstract record(): Promise<JobsRecord>;
  abstract restore(record: unknown): Promise<RestoredJobs>;
  abstract onChange(handler: (s: JobsSnapshot) => void): Promise<Unlisten>;
}


const isDelta = (p: unknown): p is JobsDelta =>
  !!p && typeof p === 'object' && typeof (p as JobsDelta).version === 'number' && Array.isArray((p as JobsDelta).order) && Array.isArray((p as JobsDelta).changed) && Array.isArray((p as JobsDelta).paused);

/** Talks to the Rust shell; `call` is Tauri's `invoke`, tests pass fakes. */
export class TauriJobsBackend implements JobsBackend {
  constructor(
    private readonly call: typeof invoke = invoke,
    private readonly listen: PayloadListenFn = listenHere,
  ) {}

  list() {
    return this.call<JobsSnapshot>('jobs_list');
  }
  cancel(id: number) {
    return this.call<void>('jobs_cancel', { id });
  }
  retry(id: number) {
    return this.call<number | null>('jobs_retry', { id });
  }
  move(id: number, up: boolean) {
    return this.call<void>('jobs_move', { id, up });
  }
  clearFinished() {
    return this.call<number>('jobs_clear_finished');
  }
  markSeen(id: number) {
    return this.call<boolean>('jobs_mark_seen', { id });
  }
  result(id: number) {
    return this.call<JobResult | null>('jobs_result', { id });
  }
  reveal(id: number) {
    return this.call<string>('jobs_reveal', { id });
  }
  wait(id: number) {
    return this.call<JobOutcome>('jobs_wait', { id });
  }
  unwait(id: number) {
    return this.call<void>('jobs_unwait', { id });
  }
  record() {
    return this.call<JobsRecord>('jobs_record');
  }
  restore(record: unknown) {
    return this.call<RestoredJobs>('jobs_restore', { record });
  }
  /**
   * The event carries what changed since the last one (`JobsDelta`); this keeps the window's snapshot and
   * hands the whole of it to `handler`. A missed version (or a job the delta does not explain) asks for
   * the list again.
   */
  onChange(handler: (s: JobsSnapshot) => void) {
    let known: { version: number; jobs: Map<number, JobInfo> } | null = null;
    // The list is at least as new as `version`: the deltas after it apply on top of it.
    const resync = (version: number) =>
      void this.list()
        .then((s) => {
          known = { version, jobs: new Map(s.jobs.map((j) => [j.id, j])) };
          handler(s);
        })
        .catch(() => undefined);
    return this.listen(JOBS_EVENT, (p) => {
      if (!isDelta(p)) return;
      const base = known && p.version === known.version + 1 ? known.jobs : p.version === 1 ? new Map<number, JobInfo>() : null;
      if (!base) return resync(p.version);
      const jobs = new Map(base);
      for (const j of p.changed) jobs.set(j.id, j);
      const ordered = p.order.map((id) => jobs.get(id));
      if (ordered.some((j) => !j)) return resync(p.version);
      known = { version: p.version, jobs: new Map(ordered.map((j) => [j!.id, j!])) };
      handler({ jobs: ordered as JobInfo[], paused: p.paused });
    });
  }
}

let shared: { injector: Injector; backend: JobsBackend } | null = null;

/** The adapter for this host behind the activity and failure proxy. Call in an injection context. */
export function createJobsBackend(): JobsBackend {
  const injector = inject(Injector);
  if (shared?.injector !== injector) {
    const target: JobsBackend = isTauri() ? new TauriJobsBackend() : lazyMock();
    shared = { injector, backend: withFailureLogging<JobsBackend>(target, inject(Logger), { onFailure: reportBackendFailures() }) };
  }
  return shared.backend;
}

/** Defers loading the mock until the first call. */
function lazyMock(): JobsBackend {
  let loaded: Promise<JobsBackend> | null = null;
  const get = () => (loaded ??= import('./mock-jobs').then((m) => m.mockJobs()));
  return new Proxy({} as JobsBackend, {
    get(_t, name) {
      if (typeof name !== 'string' || name === 'then' || name.startsWith('ng')) return undefined;
      return (...args: unknown[]) =>
        get().then((b) => {
          const fn = (b as unknown as Record<string, (...a: unknown[]) => Promise<unknown>>)[name];
          if (!fn) throw new TypeError(`${name} is not a function`);
          return fn.apply(b, args);
        });
    },
  });
}
