import { TestBed } from '@angular/core/testing';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { FakeContext } from '../analysis/test-setup';
import { AiLauncher } from '../ai/ai-launcher';
import { ChatStore } from '../ai/chat/chat.store';
import { FolderSetupStore } from '../ai/folder-setup/folder-setup.store';
import { FolderSetupLauncher } from '../ai/folder-setup/provide-folder-setup';
import { SettingsDialogService } from '../settings/settings-dialog';
import { StorageStore } from '../settings/storage/storage.store';
import { STORAGE_ID } from '../settings/storage/storage.types';
import { Logger } from '../core/logger';
import { WorkspaceStore } from '../core/workspace.store';
import { PANEL } from '../dock/dock.service';
import { COMMAND_CONTEXT } from '../extensions/command-registry';
import { IMT_ACTIVITY_ITEMS, IMT_COMMANDS, IMT_VIEWS } from '../extensions/tokens';
import { UI_STATE_CONTRIBUTIONS } from '../lifecycle/ui-state';
import { mockNotifications, resetMockNotifications } from '../notifications/mock-notifications';
import { MemoryLogger, settle } from '../../testing/helpers';
import { at, must } from '../../testing/must';
import { JobViewer } from './job-viewer';
import { JobsBackend, TauriJobsBackend } from './jobs-backend';
import { JobsStore } from './jobs.store';
import { EMPTY_JOBS, JOB_KINDS, JobInfo, JobKind, JobResult, JobsRecord, JobsSnapshot, RestoredJobs } from './jobs.types';
import { kindLabel, linkLabel, queuedLabel, stateLabel } from './jobs-text';
import { JobsPanel } from './jobs-panel';
import { MockJobCancelled, MockJobsOptions, MockJobs, mockJobs, mockJobsOptionsFromSearch, mockSteps, resetMockJobs } from './mock-jobs';
import { jobsActivityItems, jobsCommands, JobsPersistence, jobsViews, provideJobs } from './provide-jobs';

function jobInfo(p: Partial<JobInfo> & { id: number }): JobInfo {
  return {
    kind: 'export',
    title: `Job ${p.id}`,
    source: { window: 'main', link: null },
    state: 'done',
    progress: { fraction: null, done: null, total: null, etaMs: null },
    message: null,
    queuedAt: 0,
    startedAt: 0,
    finishedAt: 1,
    elapsedMs: 1,
    etaMs: null,
    cancellable: false,
    retryable: false,
    movable: false,
    position: null,
    resultType: null,
    error: null,
    unseen: false,
    retryOf: null,
    ...p,
  };
}

class FakeJobs extends JobsBackend {
  snapshot: JobsSnapshot = EMPTY_JOBS;
  handler: ((s: JobsSnapshot) => void) | null = null;
  off = vi.fn();
  list = vi.fn<JobsBackend['list']>(async () => this.snapshot);
  cancel = vi.fn<JobsBackend['cancel']>(async () => undefined);
  retry = vi.fn<JobsBackend['retry']>(async () => null);
  move = vi.fn<JobsBackend['move']>(async () => undefined);
  clearFinished = vi.fn<JobsBackend['clearFinished']>(async () => 0);
  markSeen = vi.fn<JobsBackend['markSeen']>(async () => true);
  result = vi.fn<JobsBackend['result']>(async () => null);
  reveal = vi.fn<JobsBackend['reveal']>(async () => '/p');
  wait = vi.fn<JobsBackend['wait']>(async (id) => ({ id, state: 'done', result: null, error: null, message: null }));
  unwait = vi.fn<JobsBackend['unwait']>(async () => undefined);
  record = vi.fn<JobsBackend['record']>(async (): Promise<JobsRecord> => ({ version: 1, jobs: [] }));
  restore = vi.fn<JobsBackend['restore']>(async (): Promise<RestoredJobs> => ({ snapshot: EMPTY_JOBS, warning: null }));
  onChange = vi.fn<JobsBackend['onChange']>(async (h) => {
    this.handler = h;
    return this.off;
  });
  push(s: JobsSnapshot) {
    must(this.handler, 'handler')(s);
  }
}

function setupStore(snapshot: JobsSnapshot = EMPTY_JOBS) {
  const fake = new FakeJobs();
  fake.snapshot = snapshot;
  TestBed.configureTestingModule({
    providers: [{ provide: JobsBackend, useValue: fake }, { provide: Logger, useFactory: () => new MemoryLogger() }],
  });
  return { fake, store: TestBed.inject(JobsStore) };
}

describe('TauriJobsBackend', () => {
  it('maps every method to its command and ignores malformed events', async () => {
    const calls: [string, unknown][] = [];
    const call = (async (cmd: string, args?: unknown) => {
      calls.push([cmd, args]);
      return cmd;
    }) as never;
    const handlers: ((p: unknown) => void)[] = [];
    const b = new TauriJobsBackend(call, async (event, h) => {
      expect(event).toBe('imt://jobs');
      handlers.push(h);
      return () => undefined;
    });
    const cases: [() => Promise<unknown>, string, unknown][] = [
      [() => b.list(), 'jobs_list', undefined],
      [() => b.cancel(3), 'jobs_cancel', { id: 3 }],
      [() => b.retry(4), 'jobs_retry', { id: 4 }],
      [() => b.move(5, true), 'jobs_move', { id: 5, up: true }],
      [() => b.move(5, false), 'jobs_move', { id: 5, up: false }],
      [() => b.clearFinished(), 'jobs_clear_finished', undefined],
      [() => b.markSeen(6), 'jobs_mark_seen', { id: 6 }],
      [() => b.result(7), 'jobs_result', { id: 7 }],
      [() => b.reveal(8), 'jobs_reveal', { id: 8 }],
      [() => b.wait(8), 'jobs_wait', { id: 8 }],
      [() => b.unwait(8), 'jobs_unwait', { id: 8 }],
      [() => b.record(), 'jobs_record', undefined],
      [() => b.restore({ version: 1 }), 'jobs_restore', { record: { version: 1 } }],
    ];
    for (const [run, cmd, args] of cases) {
      calls.length = 0;
      expect(await run()).toBe(cmd);
      expect(calls).toEqual([[cmd, args]]);
    }
    const seen: JobsSnapshot[] = [];
    await b.onChange((s) => seen.push(s));
    const h = must(handlers[0]);
    h({ version: 1, order: [], changed: [], paused: [] });
    for (const bad of [null, 'junk', 7, {}, { jobs: [], paused: [] }, { version: 2, order: 1, changed: [], paused: [] }]) h(bad);
    expect(seen).toEqual([{ jobs: [], paused: [] }]);
  });

  it('rebuilds the snapshot from versioned deltas and asks again after a gap', async () => {
    const job = (id: number, title: string) => ({ id, title, state: 'running' }) as never;
    const calls: string[] = [];
    const call = (async (cmd: string) => (calls.push(cmd), { jobs: [job(9, 'listed')], paused: [] })) as never;
    let h: (p: unknown) => void = () => undefined;
    const b = new TauriJobsBackend(call, async (_e, handler) => ((h = handler), () => undefined));
    const seen: JobsSnapshot[] = [];
    await b.onChange((s) => seen.push(s));
    h({ version: 1, order: [1, 2], changed: [job(1, 'a'), job(2, 'b')], paused: [] });
    h({ version: 2, order: [2, 1], changed: [job(1, 'a2')], paused: [] });
    h({ version: 3, order: [2], changed: [], paused: [] });
    expect(seen.map((s) => s.jobs.map((j) => j.title))).toEqual([['a', 'b'], ['b', 'a2'], ['b']]);
    // A missed version: the list is asked for again.
    h({ version: 5, order: [2], changed: [], paused: [] });
    await vi.waitFor(() => expect(seen.at(-1)?.jobs.map((j) => j.title)).toEqual(['listed']));
    expect(calls).toEqual(['jobs_list']);
    // The next version applies on top of the list; a job it does not explain asks again.
    h({ version: 6, order: [9, 2], changed: [job(2, 'b2')], paused: [] });
    expect(seen.at(-1)?.jobs.map((j) => j.title)).toEqual(['listed', 'b2']);
    h({ version: 7, order: [3], changed: [], paused: [] });
    await vi.waitFor(() => expect(calls).toEqual(['jobs_list', 'jobs_list']));
  });
});

describe('MockJobs', () => {
  const gate = () => {
    let resolve!: (v?: { value: number; result?: JobResult | null }) => void;
    let reject!: (e: unknown) => void;
    const promise = new Promise<{ value: number; result?: JobResult | null }>((res, rej) => {
      resolve = (v = { value: 1 }) => res(v);
      reject = rej;
    });
    return { work: () => promise, resolve, reject };
  };
  const flush = () => vi.advanceTimersByTimeAsync(0);
  const opts = (o: Partial<MockJobsOptions> = {}): MockJobsOptions => ({ visibleMs: 0, attentionMs: 10_000, failOnce: new Set<JobKind>(), ...o });
  const quiet = (p: Promise<unknown>) => p.catch(() => undefined);
  const states = async (m: MockJobs) => (await m.list()).jobs.map((j) => `${j.id}:${j.state}`);

  beforeEach(() => {
    vi.useFakeTimers();
    resetMockNotifications();
    resetMockJobs();
  });
  afterEach(() => {
    vi.useRealTimers();
    resetMockNotifications();
    resetMockJobs();
  });

  it('parses its options from the page address', () => {
    expect(mockJobsOptionsFromSearch('')).toEqual({ visibleMs: 2000, attentionMs: 10_000, failOnce: new Set() });
    const o = mockJobsOptionsFromSearch('?mockJobsVisibleMs=5&mockJobsAttentionMs=7&mockJobsFailOnce=export,nope,ai');
    expect(o.visibleMs).toBe(5);
    expect(o.attentionMs).toBe(7);
    expect([...o.failOnce]).toEqual(['export', 'ai']);
    expect(mockJobsOptionsFromSearch('?mockJobsVisibleMs=abc&mockJobsAttentionMs=1234567').visibleMs).toBe(2000);
    expect(mockJobsOptionsFromSearch('?mockJobsAttentionMs=1234567').attentionMs).toBe(10_000);
  });

  it('shares one instance until reset', () => {
    const a = mockJobs();
    expect(mockJobs()).toBe(a);
    const b = new MockJobs(opts());
    resetMockJobs(b);
    expect(mockJobs()).toBe(b);
    resetMockJobs();
    expect(mockJobs()).not.toBe(b);
  });

  it('runs one AI job at a time and queues the next with its position', async () => {
    const m = new MockJobs(opts());
    const a = gate();
    const b = gate();
    const ra = m.start({ kind: 'ai', title: 'A' }, a.work);
    const rb = m.start({ kind: 'ai', title: 'B' }, b.work);
    const jobs = (await m.list()).jobs;
    const first = must(jobs.find((j) => j.id === ra.id));
    const second = must(jobs.find((j) => j.id === rb.id));
    expect(first.state).toBe('running');
    expect(second.state).toBe('queued');
    expect(second.position).toBe(1);
    expect(second.movable).toBe(true);
    expect(first.movable).toBe(false);
    a.resolve();
    await ra.done;
    await flush();
    expect(must((await m.list()).jobs.find((j) => j.id === rb.id)).state).toBe('running');
    b.resolve({ value: 2 });
    expect(await rb.done).toBe(2);
  });

  it('allows two exports at once and queues the third', async () => {
    const m = new MockJobs(opts());
    const g = [gate(), gate(), gate()];
    const runs = g.map((x, i) => m.start({ kind: 'export', title: `E${i}` }, x.work));
    expect((await m.list()).jobs.filter((j) => j.state === 'running')).toHaveLength(2);
    const third = must((await m.list()).jobs.find((j) => j.id === must(runs[2]).id));
    expect(third.state).toBe('queued');
    expect(third.movable).toBe(false);
    g.forEach((x) => x.resolve());
    await Promise.all(runs.map((r) => r.done));
  });

  it('refuses a 21st waiting job of a kind', async () => {
    const m = new MockJobs(opts());
    const running = gate();
    const first = m.start({ kind: 'ai', title: 'running' }, running.work);
    const waiting = Array.from({ length: 20 }, (_, i) => m.start({ kind: 'ai', title: `w${i}` }, gate().work));
    const refused = m.start({ kind: 'ai', title: 'too many' }, gate().work);
    expect(refused.id).toBe(0);
    await expect(refused.done).rejects.toThrow('20 ai jobs are already waiting');
    // Another kind has its own queue.
    const other = m.start({ kind: 'export', title: 'fine' }, gate().work);
    expect(other.id).toBeGreaterThan(0);
    for (const w of waiting) void quiet(w.done);
    running.resolve();
    await first.done;
  });

  it('cancels a queued job at once and a running one when its work stops', async () => {
    const m = new MockJobs(opts());
    const a = gate();
    const ra = m.start({ kind: 'ai', title: 'A' }, a.work);
    const rb = m.start({ kind: 'ai', title: 'B' }, gate().work);
    const caught = rb.done.catch((e: unknown) => e);
    await m.cancel(rb.id);
    expect(await caught).toBeInstanceOf(MockJobCancelled);
    expect(await states(m)).toContain(`${rb.id}:cancelled`);

    const running = ra.done.catch((e: unknown) => e);
    await m.cancel(ra.id);
    // Still running until its work notices.
    expect((await states(m)).some((s) => s === `${ra.id}:running`)).toBe(true);
    a.resolve();
    expect(await running).toBeInstanceOf(MockJobCancelled);
    expect(await states(m)).toContain(`${ra.id}:cancelled`);
  });

  it('refuses to cancel an unknown job or one that is not cancellable', async () => {
    const m = new MockJobs(opts());
    await expect(m.cancel(99)).rejects.toThrow('no longer listed');
    const g = gate();
    const r = m.start({ kind: 'maintenance', title: 'M', cancellable: false }, g.work);
    await expect(m.cancel(r.id)).rejects.toThrow('cannot be cancelled');
    g.resolve();
    await r.done;
  });

  it('cancelWhere cancels the matching active jobs', async () => {
    const m = new MockJobs(opts());
    const g = gate();
    const a = m.start({ kind: 'ai', title: 'A' }, g.work);
    const b = m.start({ kind: 'ai', title: 'B' }, gate().work);
    const c = m.start({ kind: 'export', title: 'C' }, gate().work);
    const caught = b.done.catch((e: unknown) => e);
    expect(m.cancelWhere((j) => j.kind === 'ai')).toBe(2);
    expect(await caught).toBeInstanceOf(MockJobCancelled);
    expect(m.cancelWhere((j) => j.kind === 'download')).toBe(0);
    g.resolve();
    await quiet(a.done);
    void quiet(c.done);
  });

  it('fails a job once for failOnce kinds, posts a notification with Retry, and retry creates a new job', async () => {
    const m = new MockJobs(opts({ failOnce: new Set<JobKind>(['export']) }));
    resetMockJobs(m);
    const work = vi.fn(async () => ({ value: 7 }));
    const first = m.start({ kind: 'export', title: 'Export log' }, work);
    await expect(first.done).rejects.toThrow('The disk is full');
    expect(work).not.toHaveBeenCalled();
    const failed = must((await m.list()).jobs.find((j) => j.id === first.id));
    expect(failed.state).toBe('failed');
    expect(failed.error).toContain('disk is full');
    expect(failed.unseen).toBe(true);
    expect(failed.retryable).toBe(true);

    const notices = (await mockNotifications().list()).notices;
    expect(notices).toHaveLength(1);
    const n = at(notices, 0);
    expect(n).toMatchObject({ source: 'job', severity: 'error', title: 'Failed: Export log', primary: { type: 'retryJob', jobId: first.id }, secondary: { type: 'showJobs' }, scope: 'project' });
    expect(n.link.jobId).toBe(first.id);

    const newId = await m.retry(first.id);
    expect(newId).toBe(first.id + 1);
    await flush();
    const jobs = (await m.list()).jobs;
    const retried = must(jobs.find((j) => j.id === newId));
    expect(retried.retryOf).toBe(first.id);
    expect(retried.state).toBe('done');
    expect(work).toHaveBeenCalledTimes(1);
    expect(must(jobs.find((j) => j.id === first.id)).unseen).toBe(false);
  });

  it('puts a failure without a retry into a Show in Jobs action and app-wide jobs into app scope', async () => {
    const m = new MockJobs(opts());
    const r = m.start({ kind: 'download', title: 'Model', appWide: true, retryable: false }, async () => {
      throw { message: 'offline' };
    });
    await expect(r.done).rejects.toEqual({ message: 'offline' });
    const n = at((await mockNotifications().list()).notices, 0);
    expect(n.primary).toEqual({ type: 'showJobs' });
    expect(n.scope).toBe('app');
    expect(n.body).toBe('offline');
    const str = m.start({ kind: 'download', title: 'Model 2', appWide: true }, async () => Promise.reject('plain'));
    await expect(str.done).rejects.toBe('plain');
    expect(must((await m.list()).jobs.find((j) => j.id === str.id)).error).toBe('plain');
  });

  it('treats a rejection with code cancelled as a cancellation', async () => {
    const m = new MockJobs(opts());
    const r = m.start({ kind: 'import', title: 'I' }, async () => Promise.reject({ code: 'cancelled', message: 'stop' }));
    await r.done.catch(() => undefined);
    expect(await states(m)).toEqual([`${r.id}:cancelled`]);
  });

  it('only retries failed, cancelled or interrupted jobs and unknown ids fail', async () => {
    const m = new MockJobs(opts());
    const r = m.start({ kind: 'export', title: 'E' }, async () => ({ value: 1 }));
    await r.done;
    await expect(m.retry(r.id)).rejects.toThrow('only failed, cancelled or interrupted');
    await expect(m.retry(404)).rejects.toThrow('no longer listed');
  });

  it('moves waiting AI jobs up and down and refuses everything else', async () => {
    const m = new MockJobs(opts());
    const run = m.start({ kind: 'ai', title: 'running' }, gate().work);
    const b = m.start({ kind: 'ai', title: 'B' }, gate().work);
    const c = m.start({ kind: 'ai', title: 'C' }, gate().work);
    const order = async () => (await m.list()).jobs.filter((j) => j.state === 'queued').map((j) => `${j.title}:${j.position}`);
    expect(await order()).toEqual(['B:1', 'C:2']);
    await m.move(c.id, true);
    expect(await order()).toEqual(['C:1', 'B:2']);
    await m.move(c.id, false);
    expect(await order()).toEqual(['B:1', 'C:2']);
    await expect(m.move(b.id, true)).rejects.toThrow('only waiting AI jobs');
    await expect(m.move(c.id, false)).rejects.toThrow('only waiting AI jobs');
    await expect(m.move(run.id, true)).rejects.toThrow('only waiting AI jobs');
    const e = [m.start({ kind: 'export', title: 'e1' }, gate().work), m.start({ kind: 'export', title: 'e2' }, gate().work), m.start({ kind: 'export', title: 'e3' }, gate().work)];
    await expect(m.move(must(e[2]).id, true)).rejects.toThrow('only waiting AI jobs');
    await expect(m.move(777, true)).rejects.toThrow('no longer listed');
    for (const r of [run, b, c, ...e]) void quiet(r.done);
  });

  it('hides jobs shorter than the visibility threshold and keeps no history for them', async () => {
    const m = new MockJobs(opts({ visibleMs: 500 }));
    const quick = gate();
    const q = m.start({ kind: 'export', title: 'quick' }, quick.work);
    expect((await m.list()).jobs).toEqual([]);
    await vi.advanceTimersByTimeAsync(100);
    quick.resolve();
    await q.done;
    expect((await m.list()).jobs).toEqual([]);
    expect((await mockNotifications().list()).notices).toEqual([]);

    const slow = gate();
    const s = m.start({ kind: 'export', title: 'slow' }, slow.work);
    expect((await m.list()).jobs).toEqual([]);
    await vi.advanceTimersByTimeAsync(600);
    expect((await states(m)).length).toBe(1);
    slow.resolve();
    await s.done;
    expect(await states(m)).toEqual([`${s.id}:done`]);
  });

  it('keeps a failed non-analysis job visible at once but not a quick failed analysis', async () => {
    const m = new MockJobs(opts({ visibleMs: 500 }));
    const a = m.start({ kind: 'analysis', title: 'a' }, async () => Promise.reject(new Error('x')));
    await a.done.catch(() => undefined);
    expect((await m.list()).jobs).toEqual([]);
    expect((await mockNotifications().list()).notices).toEqual([]);
    const e = m.start({ kind: 'export', title: 'e' }, async () => Promise.reject(new Error('y')));
    await e.done.catch(() => undefined);
    expect(await states(m)).toEqual([`${e.id}:failed`]);
  });

  it('asks for attention (a success notification) only after the attention threshold', async () => {
    const m = new MockJobs(opts({ attentionMs: 1000 }));
    const quick = gate();
    const q = m.start({ kind: 'export', title: 'quick' }, quick.work);
    await vi.advanceTimersByTimeAsync(100);
    quick.resolve();
    await q.done;
    expect(must((await m.list()).jobs.find((j) => j.id === q.id)).unseen).toBe(false);
    expect((await mockNotifications().list()).notices).toEqual([]);

    const long = gate();
    const l = m.start({ kind: 'export', title: 'long' }, long.work);
    await vi.advanceTimersByTimeAsync(1500);
    long.resolve({ value: 1, result: { type: 'workspace' } });
    await l.done;
    const job = must((await m.list()).jobs.find((j) => j.id === l.id));
    expect(job.unseen).toBe(true);
    expect(job.resultType).toBe('workspace');
    expect(job.progress.fraction).toBeNull();
    const n = at((await mockNotifications().list()).notices, 0);
    expect(n).toMatchObject({ severity: 'success', title: 'Finished: long', body: 'Took 2 s.', primary: { type: 'viewJob', jobId: l.id }, secondary: null });

    const proposal = gate();
    const p = m.start({ kind: 'ai', title: 'ai' }, proposal.work);
    await vi.advanceTimersByTimeAsync(1500);
    proposal.resolve({ value: 1, result: { type: 'proposal', proposal: {} } });
    await p.done;
    expect(must((await mockNotifications().list()).notices.find((x) => x.link.jobId === p.id)).primary).toEqual({ type: 'reviewProposals', jobId: p.id });
  });

  it('reports progress, eta, message and sets the fraction to 1 when done', async () => {
    const m = new MockJobs(opts());
    const g = gate();
    const r = m.start({ kind: 'import', title: 'I' }, async (ctx) => {
      ctx.progress({ fraction: 0.25, done: 1, total: 4 });
      ctx.message('reading');
      return g.work();
    });
    await vi.advanceTimersByTimeAsync(1000);
    const running = must((await m.list()).jobs.find((j) => j.id === r.id));
    expect(running.progress).toMatchObject({ fraction: 0.25, done: 1, total: 4 });
    expect(running.message).toBe('reading');
    expect(running.elapsedMs).toBe(1000);
    expect(running.etaMs).toBe(3000);
    g.resolve();
    await r.done;
    expect(must((await m.list()).jobs.find((j) => j.id === r.id)).progress.fraction).toBe(1);
  });

  it('emits snapshots to subscribers, with a ticker while jobs are active, until unsubscribed', async () => {
    const m = new MockJobs(opts());
    const seen: JobsSnapshot[] = [];
    const off = await m.onChange((s) => seen.push(s));
    const g = gate();
    const r = m.start({ kind: 'export', title: 'E' }, g.work);
    const before = seen.length;
    await vi.advanceTimersByTimeAsync(1000);
    expect(seen.length).toBeGreaterThan(before + 1);
    g.resolve();
    await r.done;
    const after = seen.length;
    await vi.advanceTimersByTimeAsync(2000);
    expect(seen.length).toBe(after);
    off();
    await m.clearFinished();
    expect(seen.length).toBe(after);
  });

  it('run resolves with the value; result, reveal, markSeen and clearFinished work on the history', async () => {
    const m = new MockJobs(opts({ attentionMs: 0 }));
    expect(await m.run({ kind: 'export', title: 'x' }, async () => ({ value: 'v', result: { type: 'file', path: '/tmp/x.csv' } }))).toBe('v');
    const job = at((await m.list()).jobs, 0);
    expect(job.unseen).toBe(true);
    expect(await m.reveal(job.id)).toBe('/tmp/x.csv');
    expect(at((await m.list()).jobs, 0).unseen).toBe(false);
    expect(await m.result(job.id)).toEqual({ type: 'file', path: '/tmp/x.csv' });
    expect(await m.markSeen(job.id)).toBe(true);
    expect(await m.markSeen(999)).toBe(false);
    await expect(m.result(999)).rejects.toThrow('no longer listed');
    await expect(m.reveal(999)).rejects.toThrow('no longer listed');

    await m.run({ kind: 'import', title: 'y' }, async () => ({ value: 1, result: { type: 'workspace' } }));
    const imp = must((await m.list()).jobs.find((j) => j.title === 'y'));
    await expect(m.reveal(imp.id)).rejects.toThrow('did not write a file');
    const g = gate();
    const running = m.start({ kind: 'import', title: 'running' }, g.work);
    expect(await m.clearFinished()).toBe(2);
    expect(await states(m)).toEqual([`${running.id}:running`]);
    g.resolve();
    await running.done;
  });

  it('wait resolves with the outcome and a waited job posts no message', async () => {
    const m = new MockJobs(opts({ attentionMs: 100_000 }));
    const g = gate();
    const job = m.start({ kind: 'ai', title: 'Ask' }, g.work);
    const waiting = m.wait(job.id);
    g.resolve();
    await job.done;
    const end = await waiting;
    expect([end.id, end.state, end.error]).toEqual([job.id, 'done', null]);
    expect((await m.list()).jobs.every((j) => !j.unseen)).toBe(true);
    expect((await mockNotifications().list()).notices).toEqual([]);
  });

  it('wait returns the whole result and the job message, also for a job that already ended', async () => {
    const m = new MockJobs(opts({ visibleMs: 0 }));
    const result: JobResult = { type: 'proposal', proposal: { kind: 'query', ql: 'a' } };
    const job = m.start({ kind: 'ai', title: 'Ask' }, async (ctx) => {
      ctx.message('Done thinking.');
      return { value: 1, result };
    });
    await job.done;
    expect(await m.wait(job.id)).toEqual({ id: job.id, state: 'done', result, error: null, message: 'Done thinking.' });
    // Nobody waited, so the user is told: the results the user asked for always notify.
    expect(at((await mockNotifications().list()).notices, 0).primary).toEqual({ type: 'reviewProposals', jobId: job.id });
  });

  it('wait reports a failed job and an unknown one as cancelled', async () => {
    const m = new MockJobs(opts());
    const job = m.start({ kind: 'export', title: 'E' }, async () => Promise.reject(new Error('disk full')));
    const waiting = m.wait(job.id);
    await job.done.catch(() => undefined);
    const end = await waiting;
    expect([end.state, end.error]).toEqual(['failed', 'disk full']);
    expect((await m.wait(9999)).state).toBe('cancelled');
  });

  it('unwait releases the waiters so the job tells the user when it ends', async () => {
    const m = new MockJobs(opts());
    const g = gate();
    const job = m.start({ kind: 'ai', title: 'Ask' }, g.work);
    const waiting = m.wait(job.id);
    await m.unwait(job.id);
    expect((await waiting).state).toBe('running');
    g.resolve();
    await job.done;
    await m.unwait(12345);
    expect((await mockNotifications().list()).notices).toHaveLength(0);
  });

  it('notifies a finished file job with Show in folder as the primary action and the job message as body', async () => {
    const m = new MockJobs(opts({ visibleMs: 100_000, attentionMs: 100_000 }));
    const job = m.start({ kind: 'export', title: 'Export a.csv' }, async (ctx) => {
      ctx.message('Exported 12 events.');
      return { value: 1, result: { type: 'file', path: '/out/a.csv' } };
    });
    await job.done;
    const notice = at((await mockNotifications().list()).notices, 0);
    expect(notice.primary).toEqual({ type: 'showInFolder', jobId: job.id });
    expect(notice.body).toBe('Exported 12 events.');
  });

  it('caps the finished history at 100 jobs', async () => {
    const m = new MockJobs(opts());
    for (let i = 0; i < 105; i++) await m.run({ kind: 'export', title: `j${i}` }, async () => ({ value: i }));
    const jobs = (await m.list()).jobs;
    expect(jobs).toHaveLength(100);
    expect(jobs.some((j) => j.title === 'j0')).toBe(false);
    expect(at(jobs, 0).title).toBe('j104');
  });

  it('records the window jobs and restores them, marking unfinished ones interrupted', async () => {
    const m = new MockJobs(opts());
    await m.run({ kind: 'export', title: 'mine' }, async () => ({ value: 1, result: { type: 'file', path: '/f' } }));
    await m.run({ kind: 'download', title: 'app', appWide: true }, async () => ({ value: 1 }));
    const record = await m.record();
    expect(record.version).toBe(1);
    expect(record.jobs).toHaveLength(1);
    expect((at(record.jobs, 0) as JobInfo).title).toBe('mine');

    const other = new MockJobs(opts());
    const restored = await other.restore(record);
    expect(restored.warning).toBeNull();
    const job = at(restored.snapshot.jobs, 0);
    expect(job.title).toBe('mine');
    expect(job.state).toBe('done');
    expect(await other.reveal(job.id)).toBe('/f');

    const interrupted = await new MockJobs(opts()).restore({ version: 1, jobs: [jobInfo({ id: 9, state: 'running', finishedAt: null }), jobInfo({ id: 10, state: 'queued', finishedAt: null, queuedAt: 5 })] });
    expect(interrupted.snapshot.jobs.map((j) => j.state)).toEqual(['interrupted', 'interrupted']);
    expect(interrupted.snapshot.jobs.every((j) => j.unseen && j.error === 'IMT closed before this job finished')).toBe(true);
  });

  it('keeps active and app-wide jobs on restore and reports a damaged record', async () => {
    const m = new MockJobs(opts());
    await m.run({ kind: 'export', title: 'old' }, async () => ({ value: 1 }));
    const g = gate();
    const running = m.start({ kind: 'import', title: 'running' }, g.work);
    const cleared = await m.restore(null);
    expect(cleared.warning).toBeNull();
    expect(cleared.snapshot.jobs.map((j) => j.title)).toEqual(['running']);

    await m.run({ kind: 'export', title: 'later' }, async () => ({ value: 1 }));
    for (const bad of [{ version: 2, jobs: [] }, { version: 1, jobs: 'x' }]) {
      const damaged = await m.restore(bad);
      expect(damaged.warning).toContain('damaged');
      expect(damaged.snapshot.jobs.map((j) => j.title)).toEqual(['running']);
    }
    g.resolve();
    await running.done;
  });

  it('restores an empty record into an empty list', async () => {
    const m = new MockJobs(opts());
    expect(await m.restore({ version: 1, jobs: [] })).toEqual({ snapshot: EMPTY_JOBS, warning: null });
  });

  it('mockSteps reports progress, stops when cancelled and skips zero waits', async () => {
    const progress: number[] = [];
    let cancelled = false;
    const ctx = { cancelled: () => cancelled, progress: (p: { fraction?: number | null }) => void progress.push(p.fraction ?? -1), message: () => undefined };
    await mockSteps(ctx, 0);
    expect(progress).toEqual([]);
    const p = mockSteps(ctx, 100, 4);
    await vi.advanceTimersByTimeAsync(100);
    await p;
    expect(progress).toEqual([0.25, 0.5, 0.75, 1]);
    cancelled = true;
    await expect(mockSteps(ctx, 100)).rejects.toBeInstanceOf(MockJobCancelled);
  });
});

describe('JobsStore', () => {
  afterEach(() => TestBed.resetTestingModule());

  const mixed = (): JobsSnapshot => ({
    jobs: [
      jobInfo({ id: 1, kind: 'ai', state: 'running' }),
      jobInfo({ id: 2, kind: 'ai', state: 'queued', position: 1, source: { window: 'main', link: { type: 'chat', sessionId: 'c1' } } }),
      jobInfo({ id: 3, kind: 'export', state: 'queued', position: 1, source: { window: 'main', link: { type: 'chat', sessionId: 'c2' } } }),
      jobInfo({ id: 4, kind: 'export', state: 'done' }),
      jobInfo({ id: 5, kind: 'import', state: 'failed' }),
    ],
    paused: [{ kind: 'ai', reason: 'quarantined' }],
  });

  it('loads on init, listens for changes and applies them', async () => {
    const { fake, store } = setupStore(mixed());
    expect(store.loaded()).toBe(false);
    await settle();
    expect(fake.list).toHaveBeenCalledTimes(1);
    expect(store.loaded()).toBe(true);
    expect(store.jobs()).toHaveLength(5);
    fake.push({ jobs: [jobInfo({ id: 9 })], paused: [] });
    expect(store.jobs().map((j) => j.id)).toEqual([9]);
    expect(store.paused()).toEqual([]);
  });

  it('splits running, queued and finished and counts active jobs of every kind', async () => {
    const { store } = setupStore(mixed());
    await settle();
    expect(store.running().map((j) => j.id)).toEqual([1]);
    expect(store.queued().map((j) => j.id)).toEqual([2, 3]);
    expect(store.finished().map((j) => j.id)).toEqual([4, 5]);
    expect(store.activeCount()).toBe(3);
    expect(store.paused()).toEqual([{ kind: 'ai', reason: 'quarantined' }]);
  });

  it('filters by kind, toggles, dedupes and keeps the badge unfiltered', async () => {
    const { store } = setupStore(mixed());
    await settle();
    store.toggleKind('export');
    expect(store.jobs().map((j) => j.id)).toEqual([3, 4]);
    expect(store.activeCount()).toBe(3);
    store.toggleKind('ai');
    expect(store.kinds()).toEqual(['export', 'ai']);
    expect(store.jobs()).toHaveLength(4);
    store.toggleKind('export');
    expect(store.kinds()).toEqual(['ai']);
    store.setKinds(['import', 'import', 'export']);
    expect(store.kinds()).toEqual(['import', 'export']);
    store.setKinds([]);
    expect(store.jobs()).toHaveLength(5);
  });

  it('finds the queued turn of a chat only', async () => {
    const { fake, store } = setupStore(mixed());
    await settle();
    expect(store.queuedTurn('c1')?.id).toBe(2);
    expect(store.queuedTurn('c2')?.id).toBe(3);
    expect(store.queuedTurn('nope')).toBeNull();
    fake.push({ jobs: [jobInfo({ id: 2, state: 'running', source: { window: 'main', link: { type: 'chat', sessionId: 'c1' } } })], paused: [] });
    expect(store.queuedTurn('c1')).toBeNull();
  });

  it('forwards the commands to the backend', async () => {
    const { fake, store } = setupStore();
    await settle();
    await store.cancel(1);
    await store.retry(2);
    await store.move(3, true);
    await store.clearFinished();
    await store.markSeen(4);
    expect(fake.cancel).toHaveBeenCalledWith(1);
    expect(fake.retry).toHaveBeenCalledWith(2);
    expect(fake.move).toHaveBeenCalledWith(3, true);
    expect(fake.clearFinished).toHaveBeenCalled();
    expect(fake.markSeen).toHaveBeenCalledWith(4);
    expect(store.error()).toBeNull();
  });

  it('keeps the last error, retries the failed request and clears the error on success or dismissal', async () => {
    const { fake, store } = setupStore();
    await settle();
    await store.retryRequest();
    expect(fake.cancel).not.toHaveBeenCalled();
    fake.cancel.mockRejectedValueOnce(new Error('engine down'));
    await store.cancel(1);
    expect(store.error()).toBe('engine down');
    await store.retryRequest();
    expect(fake.cancel).toHaveBeenCalledTimes(2);
    expect(store.error()).toBeNull();

    fake.move.mockRejectedValueOnce('as text');
    await store.move(1, false);
    expect(store.error()).toBe('as text');
    store.dismissError();
    expect(store.error()).toBeNull();
    fake.move.mockRejectedValueOnce({ message: 'from object' });
    await store.move(1, false);
    expect(store.error()).toBe('from object');
    fake.move.mockRejectedValueOnce({ weird: true });
    await store.move(1, false);
    expect(store.error()).toBe('The jobs service did not answer.');
  });

  it('shows a load failure and leaves the list as it was', async () => {
    const fake = new FakeJobs();
    fake.list.mockRejectedValueOnce(new Error('no list'));
    TestBed.configureTestingModule({ providers: [{ provide: JobsBackend, useValue: fake }, { provide: Logger, useFactory: () => new MemoryLogger() }] });
    const store = TestBed.inject(JobsStore);
    await settle();
    expect(store.error()).toBe('no list');
    expect(store.loaded()).toBe(false);
    await store.retryRequest();
    expect(fake.list).toHaveBeenCalledTimes(2);
    expect(store.error()).toBeNull();
  });

  it('stops listening when destroyed, also when the subscription arrives late', async () => {
    const early = setupStore();
    await settle();
    TestBed.resetTestingModule();
    expect(early.fake.off).toHaveBeenCalledTimes(1);

    const fake = new FakeJobs();
    let release!: () => void;
    fake.onChange.mockImplementation(
      () =>
        new Promise((resolve) => {
          release = () => resolve(fake.off);
        }),
    );
    TestBed.configureTestingModule({ providers: [{ provide: JobsBackend, useValue: fake }, { provide: Logger, useFactory: () => new MemoryLogger() }] });
    TestBed.inject(JobsStore);
    await settle();
    TestBed.resetTestingModule();
    expect(fake.off).not.toHaveBeenCalled();
    release();
    await settle();
    expect(fake.off).toHaveBeenCalledTimes(1);
  });

  it('survives a failing subscription', async () => {
    const fake = new FakeJobs();
    fake.onChange.mockRejectedValue(new Error('no events'));
    TestBed.configureTestingModule({ providers: [{ provide: JobsBackend, useValue: fake }, { provide: Logger, useFactory: () => new MemoryLogger() }] });
    const store = TestBed.inject(JobsStore);
    await settle();
    expect(store.loaded()).toBe(true);
  });
});

describe('jobs-text', () => {
  it('names every kind and state', () => {
    expect(JOB_KINDS.map(kindLabel)).toEqual(['AI', 'Research', 'Analysis', 'Import', 'Export', 'Download', 'Collection', 'Maintenance']);
    expect((['queued', 'running', 'done', 'failed', 'cancelled', 'interrupted'] as const).map(stateLabel)).toEqual(['Queued', 'Running', 'Done', 'Failed', 'Cancelled', 'Interrupted']);
  });

  it('describes what started a job', () => {
    expect(linkLabel(null)).toBeNull();
    expect(linkLabel({ type: 'event', recordId: 42 })).toBe('event 42');
    expect(linkLabel({ type: 'range', from: 1, to: 2 })).toBe('a time range');
    expect(linkLabel({ type: 'query', ql: 'level >= error' })).toBe('query level >= error');
    expect(linkLabel({ type: 'path', path: 'C:\\logs\\app.log' })).toBe('app.log');
    expect(linkLabel({ type: 'path', path: '/var/log/app.log' })).toBe('app.log');
    expect(linkLabel({ type: 'path', path: '/var/log/' })).toBe('/var/log/');
    expect(linkLabel({ type: 'chat', sessionId: 'c' })).toBe('a chat');
    expect(linkLabel({ type: 'model', id: 'qwen' })).toBe('qwen');
  });

  it('words the queue position', () => {
    expect(queuedLabel(2)).toBe('Queued (position 2)');
    expect(queuedLabel(null)).toBe('Queued');
  });
});

describe('JobViewer', () => {
  afterEach(() => TestBed.resetTestingModule());

  function setup() {
    const ctx = new FakeContext();
    const ai = { showNarrative: vi.fn(async () => undefined), showProposal: vi.fn(async () => undefined), openModels: vi.fn(async () => undefined) };
    const chat = { select: vi.fn(async () => undefined) };
    const folder = { open: vi.fn(), show: vi.fn() };
    const folderStore = { addRefinement: vi.fn(), showSurvey: vi.fn() };
    const storage = { showResult: vi.fn() };
    const settings = { open: vi.fn() };
    const ws = { patchQuery: vi.fn() };
    const fake = new FakeJobs();
    TestBed.configureTestingModule({
      providers: [
        { provide: JobsBackend, useValue: fake },
        { provide: COMMAND_CONTEXT, useValue: ctx },
        { provide: AiLauncher, useValue: ai },
        { provide: ChatStore, useValue: chat },
        { provide: FolderSetupLauncher, useValue: folder },
        { provide: FolderSetupStore, useValue: folderStore },
        { provide: StorageStore, useValue: storage },
        { provide: SettingsDialogService, useValue: settings },
        { provide: WorkspaceStore, useValue: ws },
      ],
    });
    const view = (result: JobResult | null, resultType: JobInfo['resultType'] = null, title = 'T') => {
      fake.result.mockResolvedValueOnce(result);
      return TestBed.inject(JobViewer).view({ id: 5, title, resultType });
    };
    return { ctx, ai, chat, folder, folderStore, storage, settings, ws, fake, view };
  }

  it('reveals an export file (Show in folder) without asking for a result', async () => {
    const { fake, view, ctx } = setup();
    expect(await view({ type: 'file', path: '/x' }, 'file')).toBeNull();
    expect(fake.reveal).toHaveBeenCalledWith(5);
    expect(fake.result).not.toHaveBeenCalled();
    expect(ctx.calls).toEqual([]);
  });

  it('opens the Jobs pane when the engine has no result any more', async () => {
    const { ctx, view } = setup();
    expect(await view(null)).toBeNull();
    expect(ctx.last('showView')).toEqual(['jobs']);
  });

  it('shows a narrative and a proposal with the job title', async () => {
    const { ai, view } = setup();
    const narrative = { text: 'x', citations: [], evidence: { recordIds: [], queries: [] }, usage: { inputTokens: 0, outputTokens: 0 } };
    await view({ type: 'narrative', narrative }, 'narrative', 'Explain');
    expect(ai.showNarrative).toHaveBeenCalledWith(narrative, 'Explain');
    const proposal = { kind: 'query', ql: 'a' };
    await view({ type: 'proposal', proposal }, 'proposal', 'Write');
    expect(ai.showProposal).toHaveBeenCalledWith(proposal, 'Write');
  });

  it('adds a folder set-up proposal to the wizard and reopens it; a folder set-up view starts a survey', async () => {
    const { ai, folder, folderStore, view, ctx } = setup();
    const proposal = { kind: 'folderSetup', proposals: [], skipped: [], notes: [] };
    await view({ type: 'proposal', proposal });
    expect(folderStore.addRefinement).toHaveBeenCalledWith(proposal);
    expect(folder.show).toHaveBeenCalledTimes(1);
    expect(folder.open).not.toHaveBeenCalled();
    await view({ type: 'view', view: 'folderSetup', ql: null });
    expect(folder.open).toHaveBeenCalledTimes(1);
    expect(ai.showProposal).not.toHaveBeenCalled();
    expect(ctx.calls).toEqual([]);
  });

  it('shows a finished folder survey in the wizard', async () => {
    const { folder, folderStore, view } = setup();
    const outcome = { cancelled: true };
    expect(await view({ type: 'folderSurvey', outcome }, 'folderSurvey', 'Survey')).toBeNull();
    expect(folderStore.showSurvey).toHaveBeenCalledWith(outcome);
    expect(folder.show).toHaveBeenCalledTimes(1);
  });

  it('shows what a finished storage clear freed in Settings at Storage', async () => {
    const { storage, settings, view } = setup();
    const result = { freedBytes: 3, categories: [], cancelled: false };
    expect(await view({ type: 'storage', result }, 'storage', 'Clear')).toBeNull();
    expect(storage.showResult).toHaveBeenCalledWith(result);
    expect(settings.open).toHaveBeenCalledWith(STORAGE_ID);
  });

  it('selects the chat of a turn', async () => {
    const { chat, ctx, view } = setup();
    await view({ type: 'chat', sessionId: 'c9' });
    expect(ctx.last('showView')).toEqual(['ai-chat']);
    expect(chat.select).toHaveBeenCalledWith('c9');
  });

  it('opens analysis views with their query and falls back to the Jobs pane for unknown views', async () => {
    const { ctx, ws, view } = setup();
    for (const v of ['patterns', 'exceptions', 'anomalies', 'compare', 'moment', 'query']) {
      await view({ type: 'view', view: v, ql: null });
      expect(ctx.last('showView')).toEqual([v]);
    }
    expect(ws.patchQuery).not.toHaveBeenCalled();
    await view({ type: 'view', view: 'patterns', ql: 'level = error' });
    expect(ws.patchQuery).toHaveBeenCalledWith({ ql: 'level = error' });
    await view({ type: 'view', view: 'mystery', ql: '' });
    expect(ws.patchQuery).toHaveBeenCalledWith({ ql: '' });
    expect(ctx.last('showView')).toEqual(['jobs']);
  });

  it('shows the events after an import and the model manager after a download', async () => {
    const { ctx, ai, view } = setup();
    await view({ type: 'workspace' });
    expect(ctx.last('showView')).toEqual([PANEL.events]);
    await view({ type: 'model', fileName: 'q.gguf' });
    expect(ai.openModels).toHaveBeenCalled();
    ctx.calls.length = 0;
    expect(await view({ type: 'file', path: '/x' }, 'workspace')).toBeNull();
    expect(ctx.calls).toEqual([]);
  });

  it('returns a message when the result cannot be opened', async () => {
    const { fake, view } = setup();
    fake.result.mockRejectedValueOnce(new Error('gone'));
    expect(await TestBed.inject(JobViewer).view({ id: 1, title: 't', resultType: 'workspace' })).toBe('gone');
    fake.result.mockRejectedValueOnce('text');
    expect(await TestBed.inject(JobViewer).view({ id: 1, title: 't', resultType: 'workspace' })).toBe('text');
    fake.result.mockRejectedValueOnce({ odd: 1 });
    expect(await TestBed.inject(JobViewer).view({ id: 1, title: 't', resultType: 'workspace' })).toBe('The result could not be opened.');
    fake.reveal.mockRejectedValueOnce(new Error('no file'));
    expect(await TestBed.inject(JobViewer).view({ id: 1, title: 't', resultType: 'file' })).toBe('no file');
    expect(await view(null)).toBeNull();
  });
});

describe('JobsPersistence', () => {
  afterEach(() => TestBed.resetTestingModule());

  const saved: JobsRecord = { version: 1, jobs: [{ id: 1 }] };

  it('keeps the engine record for saving, or nothing when there are no jobs', async () => {
    const { fake } = setupStore();
    const p = TestBed.inject(JobsPersistence);
    await settle();
    expect(p.contribution.key).toBe('jobs');
    expect(p.contribution.tracksChanges).toBe(false);
    expect(p.contribution.capture()).toBeNull();
    fake.record.mockResolvedValueOnce(saved);
    await p.sync();
    expect(p.contribution.capture()).toEqual(saved);
    await p.sync();
    expect(p.record()).toBeNull();
  });

  it('syncs when the set of finished jobs (or their unseen state) changes, not for running jobs', async () => {
    const { fake, store } = setupStore();
    TestBed.inject(JobsPersistence);
    await settle();
    const baseline = fake.record.mock.calls.length;
    store.apply({ jobs: [jobInfo({ id: 1 })], paused: [] });
    await settle();
    expect(fake.record).toHaveBeenCalledTimes(baseline + 1);
    store.apply({ jobs: [jobInfo({ id: 2, state: 'running' }), jobInfo({ id: 1 })], paused: [] });
    await settle();
    expect(fake.record).toHaveBeenCalledTimes(baseline + 1);
    store.apply({ jobs: [jobInfo({ id: 2, state: 'running' }), jobInfo({ id: 1, unseen: true })], paused: [] });
    await settle();
    expect(fake.record).toHaveBeenCalledTimes(baseline + 2);
  });

  it('restores a saved record through the engine, applies the snapshot and reports a warning', async () => {
    const { fake, store } = setupStore();
    const p = TestBed.inject(JobsPersistence);
    await settle();
    fake.restore.mockResolvedValueOnce({ snapshot: { jobs: [jobInfo({ id: 3 })], paused: [] }, warning: 'damaged' });
    fake.record.mockResolvedValue(saved);
    p.contribution.restore(saved);
    await settle();
    expect(fake.restore).toHaveBeenCalledWith(saved);
    expect(p.warning()).toBe('damaged');
    expect(store.jobs().map((j) => j.id)).toEqual([3]);
    expect(p.record()).toEqual(saved);
    must(p.contribution.reset)();
    await settle();
    expect(fake.restore).toHaveBeenLastCalledWith(null);
    await p.restore(undefined);
    expect(fake.restore).toHaveBeenLastCalledWith(null);
  });

  it('logs and carries on when the engine fails', async () => {
    const { fake } = setupStore();
    const p = TestBed.inject(JobsPersistence);
    await settle();
    const log = TestBed.inject(Logger) as MemoryLogger;
    fake.record.mockRejectedValueOnce(new Error('no record'));
    await p.sync();
    fake.restore.mockRejectedValueOnce('no restore');
    await p.restore({});
    const warns = log.entries.filter((e) => e.level === 'warn');
    expect(warns.map((w) => w.message)).toEqual(['Job history could not be read for saving', 'Saved jobs not restored']);
    expect(warns.map((w) => w.props?.['Error'])).toEqual(['no record', 'no restore']);
  });
});

describe('provide-jobs', () => {
  afterEach(() => TestBed.resetTestingModule());

  it('describes the Jobs view', async () => {
    const [view] = jobsViews();
    expect(view).toMatchObject({ id: 'jobs', title: 'Jobs', icon: 'jobs', category: 'core', defaultPosition: 'bottom' });
    expect(await must(view).component()).toBe(JobsPanel);
  });

  it('offers a show command and a clear command that is available only with finished jobs', async () => {
    const { fake, store } = setupStore();
    await settle();
    const [show, clear] = jobsCommands(store);
    const ctx = new FakeContext();
    must(show).run(ctx);
    expect(ctx.last('showView')).toEqual(['jobs']);
    expect(must(clear).when?.(ctx)).toBe(false);
    store.apply({ jobs: [jobInfo({ id: 1, state: 'running' })], paused: [] });
    expect(must(clear).when?.(ctx)).toBe(false);
    store.apply({ jobs: [jobInfo({ id: 1, state: 'failed' })], paused: [] });
    expect(must(clear).when?.(ctx)).toBe(true);
    must(clear).run(ctx);
    expect(fake.clearFinished).toHaveBeenCalled();
  });

  it('shows the active job count as the button badge and pulses while jobs run', async () => {
    const { store } = setupStore();
    await settle();
    const [item] = jobsActivityItems(store);
    const it0 = must(item);
    expect(it0).toMatchObject({ id: 'jobs', section: 'bottom', run: { view: 'jobs' } });
    expect(must(it0.badge)(new FakeContext())).toBeNull();
    expect(must(it0.pulse)(new FakeContext())).toBe(false);
    store.apply({ jobs: [jobInfo({ id: 1, state: 'queued' }), jobInfo({ id: 2, state: 'running' }), jobInfo({ id: 3 })], paused: [] });
    expect(must(it0.badge)(new FakeContext())).toBe(2);
    expect(must(it0.pulse)(new FakeContext())).toBe(true);
  });

  it('registers the view, commands, button and saved state with one call', async () => {
    const fake = new FakeJobs();
    TestBed.configureTestingModule({ providers: [{ provide: JobsBackend, useValue: fake }, { provide: Logger, useFactory: () => new MemoryLogger() }, provideJobs()] });
    expect(TestBed.inject(IMT_VIEWS).flat().map((v) => v.id)).toContain('jobs');
    expect(TestBed.inject(IMT_COMMANDS).flat().map((c) => c.id)).toEqual(expect.arrayContaining(['jobs.show', 'jobs.clearFinished']));
    expect(TestBed.inject(IMT_ACTIVITY_ITEMS).flat().map((i) => i.id)).toContain('jobs');
    expect(TestBed.inject(UI_STATE_CONTRIBUTIONS).flat().map((c) => c.key)).toContain('jobs');
    await settle();
    expect(fake.list).toHaveBeenCalled();
  });
});
