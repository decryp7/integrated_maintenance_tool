import { DestroyRef } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { Logger } from '../core/logger';
import { NotificationsBackend } from '../notifications/notifications-backend';
import { NotificationsStore } from '../notifications/notifications.store';
import { MemoryLogger } from '../../testing/helpers';
import { at } from '../../testing/must';
import { JobFollow } from './job-follow';
import { JobsBackend } from './jobs-backend';
import { JobOutcome, JobResult } from './jobs.types';

/** A destroy ref the test can fire, like a dialog closing. */
function owner() {
  const callbacks = new Set<() => void>();
  const ref = {
    onDestroy: (cb: () => void) => {
      callbacks.add(cb);
      return () => callbacks.delete(cb);
    },
  } as unknown as DestroyRef;
  return { ref, destroy: () => [...callbacks].forEach((cb) => cb()), pending: () => callbacks.size };
}

const ticks = async () => {
  for (let i = 0; i < 4; i++) await Promise.resolve();
};

function setup() {
  let end!: (o: JobOutcome) => void;
  const jobs = {
    wait: vi.fn(() => new Promise<JobOutcome>((r) => (end = r))),
    unwait: vi.fn(async (_id: number) => undefined),
  };
  const notices = { list: vi.fn(async () => ({ notices: [], unread: 0 })), onChange: vi.fn(async () => () => undefined), onOpen: vi.fn(async () => () => undefined) };
  TestBed.configureTestingModule({
    providers: [
      { provide: JobsBackend, useValue: jobs },
      { provide: NotificationsBackend, useValue: notices },
      { provide: Logger, useFactory: () => new MemoryLogger() },
    ],
  });
  const outcome = (state: JobOutcome['state'], extra: Partial<JobOutcome> = {}): JobOutcome => ({ id: 7, state, result: null, error: null, message: null, ...extra });
  return { follow: TestBed.inject(JobFollow), toasts: TestBed.inject(NotificationsStore), jobs, end: (o: JobOutcome) => end(o), outcome };
}

describe('JobFollow', () => {
  afterEach(() => TestBed.resetTestingModule());

  it('started shows an info toast "running in Jobs" with Show in Jobs', () => {
    const { follow, toasts } = setup();
    follow.started('Exporting report.html — running in Jobs');
    const t = at(toasts.toasts(), 0);
    expect([t.severity, t.title, t.primary]).toEqual(['info', 'Exporting report.html — running in Jobs', { type: 'showJobs' }]);
  });

  it('notStarted shows an error toast with the reason', () => {
    const { follow, toasts } = setup();
    follow.notStarted('The AI request did not start', '20 ai jobs are already waiting');
    const t = at(toasts.toasts(), 0);
    expect([t.severity, t.title, t.body, t.primary]).toEqual(['error', 'The AI request did not start', '20 ai jobs are already waiting', null]);
  });

  it('follow resolves with the outcome and stops watching the owner', async () => {
    const { follow, jobs, end, outcome } = setup();
    const o = owner();
    const result = follow.follow(7, o.ref);
    expect(jobs.wait).toHaveBeenCalledWith(7);
    expect(o.pending()).toBe(1);
    const done = outcome('done', { message: 'ok' });
    end(done);
    expect(await result).toBe(done);
    expect(o.pending()).toBe(0);
    o.destroy();
    expect(jobs.unwait).not.toHaveBeenCalled();
  });

  it('follow gives null and releases the wait when the owner is destroyed first (the job goes on)', async () => {
    const { follow, jobs, end, outcome } = setup();
    const o = owner();
    const result = follow.follow(7, o.ref);
    o.destroy();
    expect(jobs.unwait).toHaveBeenCalledWith(7);
    end(outcome('interrupted'));
    expect(await result).toBeNull();
    // A failing unwait is swallowed.
    jobs.unwait.mockRejectedValueOnce(new Error('gone'));
    const again = follow.follow(8, o.ref);
    o.destroy();
    end(outcome('done'));
    expect(await again).toBeNull();
  });

  it('run starts the job, reports its id at once and resolves with its result', async () => {
    const { follow, end, outcome } = setup();
    const o = owner();
    const proposal: JobResult = { type: 'proposal', proposal: { kind: 'query', ql: 'a' } };
    const start = vi.fn(async () => 7);
    const started = vi.fn();
    const run = follow.run(start, o.ref, started);
    await ticks();
    // The id is known before the job ends: the dialog shows the wait line from here on.
    expect(started).toHaveBeenCalledWith(7);
    end(outcome('done', { result: proposal }));
    expect(await run).toEqual(proposal);
  });

  it('run resolves with null for a cancelled job or a closed owner', async () => {
    const { follow, end, outcome, jobs } = setup();
    const o = owner();
    let run = follow.run(async () => 7, o.ref);
    await ticks();
    end(outcome('cancelled'));
    expect(await run).toBeNull();

    run = follow.run(async () => 7, o.ref);
    await ticks();
    o.destroy();
    end(outcome('done'));
    expect(await run).toBeNull();
    expect(jobs.unwait).toHaveBeenCalled();
  });

  it('run rejects with the reason of a failed job', async () => {
    const { follow, end, outcome } = setup();
    const o = owner();
    let run = follow.run(async () => 7, o.ref);
    const failed = expect(run).rejects.toThrow('the model ran out of memory');
    await ticks();
    end(outcome('failed', { error: 'the model ran out of memory' }));
    await failed;

    run = follow.run(async () => 7, o.ref);
    const generic = expect(run).rejects.toThrow('The job failed.');
    await ticks();
    end(outcome('failed'));
    await generic;
  });

  it('run rejects at once when the job did not start, without waiting', async () => {
    const { follow, jobs } = setup();
    await expect(follow.run(async () => Promise.reject(new Error('20 ai jobs are already waiting')), owner().ref)).rejects.toThrow('20 ai jobs');
    expect(jobs.wait).not.toHaveBeenCalled();
  });
});
