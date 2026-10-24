import { onMockRelease } from '../core/mock/mock-hold';
import { mockNotifications } from '../notifications/mock-notifications';
import { Unlisten } from '../windowing/window-events';
import { JobsBackend } from './jobs-backend';
import { JobInfo, JobKind, JobLink, JobOutcome, JobProgress, JobResult, JobsRecord, JobsSnapshot, JobState, RestoredJobs } from './jobs.types';

/** What a mock job's work sees. */
export interface MockJobCtx {
  readonly cancelled: () => boolean;
  progress(p: Partial<JobProgress>): void;
  message(m: string): void;
}

/** What a mock producer submits (the engine's `JobSpec`). */
export interface MockJobSpec {
  kind: JobKind;
  title: string;
  link?: JobLink | null;
  /** App-wide work (downloads); otherwise the window's. */
  appWide?: boolean;
  cancellable?: boolean;
  retryable?: boolean;
  /** The message title when it is done ("Explanation ready"). */
  doneTitle?: string;
}

/** A job's work: resolves with the caller's value and the result View opens. */
export type MockWork<T> = (ctx: MockJobCtx) => Promise<{ value: T; result?: JobResult | null }>;

export class MockJobCancelled extends Error {
  constructor() {
    super('cancelled');
    this.name = 'MockJobCancelled';
  }
}

/** The engine's rules, mimicked (see app/src-tauri/src/jobs/queue.rs). */
const LIMITS: Record<JobKind, number> = { ai: 1, research: 1, analysis: 3, import: 1, export: 2, download: 2, collection: 1, capture: 4, maintenance: 1 };
const QUEUE_CAP = 20;
/** Results the user asked for and waits to see: their job always tells when it is done (engine: queue.rs). */
const ATTENTION_RESULTS = new Set<string>(['narrative', 'proposal', 'file', 'folderSurvey', 'storage', 'relatedLogs']);
const HISTORY_CAP = 100;

export interface MockJobsOptions {
  /** A job shows after this long (engine: 2 s). */
  visibleMs: number;
  /** A job that ran this long asks for attention (engine: 10 s). */
  attentionMs: number;
  /** Kinds whose first job fails (to try Retry). */
  failOnce: Set<JobKind>;
  /** Kinds whose jobs, once their work is done, wait to finish until the test releases them (`onMockRelease`). */
  hold?: Set<JobKind>;
}

export function mockJobsOptionsFromSearch(search: string): MockJobsOptions {
  const p = new URLSearchParams(search);
  const num = (name: string, d: number) => {
    const v = p.get(name);
    return v !== null && /^\d{1,6}$/.test(v) ? Number(v) : d;
  };
  const kinds = (name: string) => new Set((p.get(name) ?? '').split(',').filter((k): k is JobKind => k in LIMITS));
  const failOnce = kinds('mockJobsFailOnce');
  const hold = kinds('mockJobsHold');
  return { visibleMs: num('mockJobsVisibleMs', 2000), attentionMs: num('mockJobsAttentionMs', 10_000), failOnce, ...(hold.size ? { hold } : {}) };
}

interface Entry {
  info: JobInfo;
  result: JobResult | null;
  start: (() => void) | null;
  cancel: () => void;
  rerun: (() => Promise<unknown>) | null;
  doneTitle?: string;
}

/**
 * The jobs service for the browser build: other mocks (AI, exports) run their slow work through `run`, so the
 * Jobs pane, toasts and the notification centre behave as in the app. It is not the reference for behaviour.
 */
export class MockJobs implements JobsBackend {
  private entries: Entry[] = [];
  private next = 1;
  private readonly handlers = new Set<(s: JobsSnapshot) => void>();
  /** Who waits for a job's end (a dialog following it). */
  private readonly waiters = new Map<number, ((o: JobOutcome) => void)[]>();
  private ticker: ReturnType<typeof setInterval> | null = null;

  /** Kinds still held (`options.hold`), and the finishes waiting for the release. */
  private readonly holding: Set<JobKind>;
  private readonly heldFinishes: (() => void)[] = [];

  constructor(readonly options: MockJobsOptions = mockJobsOptionsFromSearch(typeof window === 'undefined' ? '' : window.location.search)) {
    this.holding = new Set(options.hold ?? []);
    if (this.holding.size) onMockRelease(() => this.release());
  }

  private release(): void {
    this.holding.clear();
    for (const finish of this.heldFinishes.splice(0)) finish();
  }

  /** `work`'s outcome, delivered once its kind is no longer held. */
  private afterHold<T>(kind: JobKind, work: Promise<T>): Promise<T> {
    return work.then((value) => (this.holding.has(kind) ? new Promise<T>((resolve) => this.heldFinishes.push(() => resolve(value))) : value));
  }

  private visible(e: Entry, now: number): boolean {
    const end = e.info.finishedAt ?? now;
    return end - e.info.queuedAt >= this.options.visibleMs || (e.info.state === 'failed' && e.info.kind !== 'analysis');
  }

  private snapshot(): JobsSnapshot {
    const now = Date.now();
    const active = this.entries.filter((e) => e.info.state === 'queued' || e.info.state === 'running');
    const finished = this.entries.filter((e) => !active.includes(e)).reverse();
    const view = (e: Entry): JobInfo => {
      const startedAt = e.info.startedAt;
      const elapsedMs = startedAt === null ? 0 : (e.info.finishedAt ?? now) - startedAt;
      const queued = active.filter((o) => o.info.kind === e.info.kind && o.info.state === 'queued');
      const position = e.info.state === 'queued' ? queued.indexOf(e) + 1 : null;
      const f = e.info.progress.fraction;
      const etaMs = e.info.state === 'running' && f !== null && f > 0.01 && f < 1 ? Math.round((elapsedMs * (1 - f)) / f) : null;
      return { ...e.info, elapsedMs, etaMs, position, movable: e.info.kind === 'ai' && e.info.state === 'queued', cancellable: e.info.cancellable && (e.info.state === 'queued' || e.info.state === 'running'), retryable: e.info.retryable && ['failed', 'cancelled', 'interrupted'].includes(e.info.state) };
    };
    const running = active.filter((e) => this.visible(e, now)).sort((a, b) => Number(a.info.state !== 'running') - Number(b.info.state !== 'running'));
    return { jobs: [...running, ...finished].map(view), paused: [] };
  }

  private emit(): void {
    const s = this.snapshot();
    for (const h of this.handlers) h(s);
    const active = this.entries.some((e) => e.info.state === 'queued' || e.info.state === 'running');
    if (active && !this.ticker) this.ticker = setInterval(() => this.emit(), 400);
    if (!active && this.ticker) {
      clearInterval(this.ticker);
      this.ticker = null;
    }
  }

  private pump(): void {
    for (const e of this.entries) {
      if (e.info.state !== 'queued' || !e.start) continue;
      const running = this.entries.filter((o) => o.info.kind === e.info.kind && o.info.state === 'running').length;
      if (running < LIMITS[e.info.kind]) {
        const start = e.start;
        e.start = null;
        start();
      }
    }
  }

  private finish(e: Entry, state: JobState, extra: Partial<JobInfo> = {}, result: JobResult | null = null): void {
    const finishedAt = Date.now();
    const ran = e.info.startedAt === null ? 0 : finishedAt - e.info.startedAt;
    const age = finishedAt - e.info.queuedAt;
    const waiters = this.waiters.get(e.info.id) ?? [];
    this.waiters.delete(e.info.id);
    // A dialog still waiting shows the outcome itself: no message, nothing unseen.
    const attended = waiters.length > 0;
    const attention =
      !attended &&
      ((state === 'done' && (e.info.kind === 'research' || ATTENTION_RESULTS.has(result?.type ?? '') || Math.max(ran, age) >= this.options.attentionMs)) ||
        (state === 'failed' && (e.info.kind !== 'analysis' || age >= this.options.visibleMs)));
    e.info = { ...e.info, ...extra, state, finishedAt, unseen: attention };
    e.result = result;
    if (state === 'done' && e.info.progress.fraction !== null) e.info.progress = { ...e.info.progress, fraction: 1 };
    const kept = age >= this.options.visibleMs || attention;
    if (!kept) this.entries = this.entries.filter((x) => x !== e);
    const finished = this.entries.filter((x) => x.info.finishedAt !== null);
    if (finished.length > HISTORY_CAP) this.entries = this.entries.filter((x) => x !== finished[0]);
    if (attention) this.notify(e, state);
    for (const w of waiters) w(this.outcome(e));
    this.pump();
    this.emit();
  }

  private notify(e: Entry, state: JobState): void {
    const ok = state === 'done';
    const jobId = e.info.id;
    const primary = ok
      ? e.result?.type === 'proposal'
        ? { type: 'reviewProposals' as const, jobId }
        : e.result?.type === 'file'
          ? { type: 'showInFolder' as const, jobId }
          : { type: 'viewJob' as const, jobId }
      : e.info.retryable
        ? { type: 'retryJob' as const, jobId }
        : { type: 'showJobs' as const };
    mockNotifications().post({
      source: 'job',
      severity: ok ? 'success' : 'error',
      title: ok && e.doneTitle ? e.doneTitle : ok && e.info.kind === 'research' ? `Research finished: ${e.info.title.replace(/^Research: /, '')}` : `${ok ? 'Finished' : 'Failed'}: ${e.info.title}`,
      body: ok && e.doneTitle ? e.info.title : ok && e.info.message ? e.info.message : ok ? `Took ${Math.round(((e.info.finishedAt ?? 0) - (e.info.startedAt ?? 0)) / 1000)} s.` : (e.info.error ?? ''),
      primary,
      secondary: ok ? null : { type: 'showJobs' },
      cause: `job:${jobId}`,
      scope: e.info.source.window ? 'project' : 'app',
      link: { jobId },
    });
  }

  /** Runs `work` as a job (queued by kind) and resolves with its value; a cancelled job rejects with `MockJobCancelled`. */
  run<T>(spec: MockJobSpec, work: MockWork<T>): Promise<T> {
    return this.start(spec, work).done;
  }

  private outcome(e: Entry): JobOutcome {
    return { id: e.info.id, state: e.info.state, result: e.result, error: e.info.error, message: e.info.message };
  }

  /** The id the next job gets. */
  nextId(): number {
    return this.next;
  }

  /** Cancels the active jobs matching `pred`; returns how many. */
  cancelWhere(pred: (j: JobInfo) => boolean): number {
    const hits = this.entries.filter((e) => (e.info.state === 'queued' || e.info.state === 'running') && pred(e.info));
    for (const e of hits) e.cancel();
    this.emit();
    return hits.length;
  }

  /** Like `run`, with the new job's id at once. */
  start<T>(spec: MockJobSpec, work: MockWork<T>): { id: number; done: Promise<T> } {
    const waiting = this.entries.filter((e) => e.info.kind === spec.kind && e.info.state === 'queued').length;
    if (waiting >= QUEUE_CAP) return { id: 0, done: Promise.reject(new Error(`${QUEUE_CAP} ${spec.kind} jobs are already waiting; wait for some to finish or cancel one in the Jobs pane`)) };
    const id = this.next++;
    let cancelled = false;
    const info: JobInfo = {
      id,
      kind: spec.kind,
      title: spec.title,
      source: { window: spec.appWide ? null : 'main', link: spec.link ?? null },
      state: 'queued',
      progress: { fraction: null, done: null, total: null, etaMs: null, unit: 'items' },
      message: null,
      queuedAt: Date.now(),
      startedAt: null,
      finishedAt: null,
      elapsedMs: 0,
      etaMs: null,
      cancellable: spec.cancellable ?? true,
      retryable: spec.retryable ?? true,
      movable: false,
      position: null,
      resultType: null,
      error: null,
      unseen: false,
      retryOf: null,
    };
    const done = new Promise<T>((resolve, reject) => {
      const entry: Entry = {
        info,
        result: null,
        start: null,
        rerun: null,
        doneTitle: spec.doneTitle,
        cancel: () => {
          cancelled = true;
          if (entry.info.state === 'queued') {
            this.finish(entry, 'cancelled');
            reject(new MockJobCancelled());
          }
        },
      };
      entry.rerun = () => this.run(spec, work).catch(() => undefined);
      entry.start = () => {
        entry.info = { ...entry.info, state: 'running', startedAt: Date.now() };
        this.emit();
        const ctx: MockJobCtx = {
          cancelled: () => cancelled,
          progress: (p) => {
            entry.info = { ...entry.info, progress: { ...entry.info.progress, ...p } };
          },
          message: (m) => {
            entry.info = { ...entry.info, message: m };
          },
        };
        const failNow = this.options.failOnce.delete(spec.kind);
        const job = failNow ? Promise.reject(new Error('The disk is full (mock failure).')) : this.afterHold(spec.kind, work(ctx));
        job.then(
          ({ value, result }) => {
            if (cancelled) {
              this.finish(entry, 'cancelled');
              reject(new MockJobCancelled());
              return;
            }
            this.finish(entry, 'done', { resultType: result?.type ?? null }, result ?? null);
            resolve(value);
          },
          (err: unknown) => {
            const message = err instanceof Error ? err.message : typeof err === 'object' && err && 'message' in err ? String((err as { message: unknown }).message) : String(err);
            const isCancel = cancelled || err instanceof MockJobCancelled || (typeof err === 'object' && !!err && (err as { code?: unknown }).code === 'cancelled');
            this.finish(entry, isCancel ? 'cancelled' : 'failed', isCancel ? {} : { error: message });
            reject(err);
          },
        );
      };
      this.entries.push(entry);
      this.emit();
      this.pump();
    });
    return { id, done };
  }

  private find(id: number): Entry {
    const e = this.entries.find((x) => x.info.id === id);
    if (!e) throw new Error('this job is no longer listed');
    return e;
  }

  async list() {
    return this.snapshot();
  }

  async cancel(id: number) {
    const e = this.find(id);
    if (!e.info.cancellable) throw new Error('this job cannot be cancelled');
    e.cancel();
    this.emit();
  }

  async retry(id: number) {
    const e = this.find(id);
    if (!e.rerun || !['failed', 'cancelled', 'interrupted'].includes(e.info.state)) throw new Error('only failed, cancelled or interrupted jobs that know their request can be retried');
    e.info = { ...e.info, unseen: false };
    const before = this.next;
    void e.rerun();
    const created = this.entries.find((x) => x.info.id === before);
    if (created) created.info = { ...created.info, retryOf: id };
    this.emit();
    return created ? created.info.id : null;
  }

  async move(id: number, up: boolean) {
    const e = this.find(id);
    if (e.info.kind !== 'ai' || e.info.state !== 'queued') throw new Error('only waiting AI jobs can be moved');
    const i = this.entries.indexOf(e);
    const same = (x: Entry) => x.info.kind === 'ai' && x.info.state === 'queued';
    const others = this.entries.map((x, j) => ({ x, j })).filter(({ x, j }) => same(x) && (up ? j < i : j > i));
    const target = up ? others.at(-1) : others[0];
    if (!target) throw new Error('only waiting AI jobs can be moved');
    const other = target.x;
    this.entries[target.j] = e;
    this.entries[i] = other;
    this.emit();
  }

  async clearFinished() {
    const before = this.entries.length;
    this.entries = this.entries.filter((e) => e.info.state === 'queued' || e.info.state === 'running');
    this.emit();
    return before - this.entries.length;
  }

  async markSeen(id: number) {
    const e = this.entries.find((x) => x.info.id === id);
    if (e) e.info = { ...e.info, unseen: false };
    this.emit();
    return !!e;
  }

  async result(id: number) {
    const e = this.find(id);
    e.info = { ...e.info, unseen: false };
    this.emit();
    return e.result;
  }

  async reveal(id: number) {
    const e = this.find(id);
    if (e.result?.type !== 'file') throw new Error('this job did not write a file');
    e.info = { ...e.info, unseen: false };
    this.emit();
    return e.result.path;
  }

  async wait(id: number): Promise<JobOutcome> {
    const e = this.entries.find((x) => x.info.id === id);
    if (!e) return { id, state: 'cancelled', result: null, error: null, message: null };
    if (e.info.finishedAt !== null) return this.outcome(e);
    return new Promise<JobOutcome>((resolve) => this.waiters.set(id, [...(this.waiters.get(id) ?? []), resolve]));
  }

  async unwait(id: number): Promise<void> {
    const waiting = this.waiters.get(id) ?? [];
    this.waiters.delete(id);
    const e = this.entries.find((x) => x.info.id === id);
    for (const w of waiting) w(e ? this.outcome(e) : { id, state: 'interrupted', result: null, error: null, message: null });
  }

  async record(): Promise<JobsRecord> {
    return { version: 1, jobs: this.entries.filter((e) => e.info.finishedAt !== null && e.info.source.window).map((e) => ({ ...e.info, result: e.result })) };
  }

  async restore(record: unknown): Promise<RestoredJobs> {
    const r = record as JobsRecord | null;
    const active = this.entries.filter((e) => e.info.finishedAt === null || !e.info.source.window);
    if (r && (r.version !== 1 || !Array.isArray(r.jobs))) {
      this.entries = active;
      this.emit();
      return { snapshot: this.snapshot(), warning: 'the saved job history is damaged and was not loaded' };
    }
    const restored = ((r?.jobs ?? []) as (JobInfo & { result?: JobResult | null })[]).map((j): Entry => {
      const unfinished = j.state === 'queued' || j.state === 'running';
      const info: JobInfo = { ...j, id: this.next++, state: unfinished ? 'interrupted' : j.state, finishedAt: j.finishedAt ?? j.queuedAt, error: unfinished ? 'Debrief closed before this job finished' : j.error, unseen: unfinished || j.unseen };
      return { info, result: j.result ?? null, start: null, cancel: () => undefined, rerun: null };
    });
    this.entries = [...restored, ...active];
    this.emit();
    return { snapshot: this.snapshot(), warning: null };
  }

  async onChange(handler: (s: JobsSnapshot) => void): Promise<Unlisten> {
    this.handlers.add(handler);
    return () => this.handlers.delete(handler);
  }

  /** Stops the progress ticker and forgets listeners and waiters (an instance replaced by `resetMockJobs`). */
  dispose(): void {
    if (this.ticker) clearInterval(this.ticker);
    this.ticker = null;
    this.handlers.clear();
    this.waiters.clear();
  }
}

let instance: MockJobs | null = null;

/** The one mock jobs service of this page (shared by the mock backends and the UI). */
export function mockJobs(): MockJobs {
  instance ??= new MockJobs();
  return instance;
}

/** Replaces the shared instance (tests); the old one stops ticking, so its jobs cannot reach the next test. */
export function resetMockJobs(next: MockJobs | null = null): void {
  if (instance !== next) instance?.dispose();
  instance = next;
}

const sleep = (ms: number) => new Promise<void>((r) => setTimeout(r, ms));

/** Waits `ms` in small steps, reporting progress and stopping early when cancelled. */
export async function mockSteps(ctx: MockJobCtx, ms: number, steps = 10): Promise<void> {
  if (ms <= 0) return;
  for (let i = 0; i < steps; i++) {
    if (ctx.cancelled()) throw new MockJobCancelled();
    await sleep(ms / steps);
    ctx.progress({ fraction: (i + 1) / steps });
  }
}
