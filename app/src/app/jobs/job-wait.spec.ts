import { Component, signal } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { Logger } from '../core/logger';
import { all, el, MemoryLogger, settle } from '../../testing/helpers';
import { JobWait } from './job-wait';
import { JobsBackend } from './jobs-backend';
import { JobsStore } from './jobs.store';
import { EMPTY_JOBS, JobInfo } from './jobs.types';

function jobInfo(p: Partial<JobInfo> & { id: number }): JobInfo {
  return {
    kind: 'ai',
    title: `Job ${p.id}`,
    source: { window: 'main', link: null },
    state: 'running',
    progress: { fraction: null, done: null, total: null, etaMs: null },
    message: null,
    queuedAt: 0,
    startedAt: 0,
    finishedAt: null,
    elapsedMs: 1,
    etaMs: null,
    cancellable: true,
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

@Component({ imports: [JobWait], template: `<imt-job-wait [jobId]="id()" busyText="The local model proposes…" />` })
class Host {
  readonly id = signal(5);
}

function setup() {
  const backend = {
    cancel: vi.fn(async (_id: number) => undefined),
    list: vi.fn(async () => EMPTY_JOBS),
    onChange: vi.fn(async () => () => undefined),
  };
  TestBed.configureTestingModule({
    providers: [
      { provide: JobsBackend, useValue: backend },
      { provide: Logger, useFactory: () => new MemoryLogger() },
    ],
  });
  const store = TestBed.inject(JobsStore);
  const f = TestBed.createComponent(Host);
  f.detectChanges();
  return { f, backend, store, line: () => el(f, '.line').textContent?.trim() };
}

describe('JobWait', () => {
  afterEach(() => TestBed.resetTestingModule());

  it('shows the busy text while the job reports nothing, and that the dialog may be closed', () => {
    const { f, line } = setup();
    expect(line()).toBe('The local model proposes…');
    expect(el(f, '[data-testid=job-wait]').getAttribute('role')).toBe('status');
    expect(f.nativeElement.textContent).toContain('You can close this dialog');
  });

  it('shows the queue position while queued and the job message while it runs', () => {
    const { f, store, line } = setup();
    store.apply({ jobs: [jobInfo({ id: 5, state: 'queued', position: 2 })], paused: [] });
    f.detectChanges();
    expect(line()).toBe('Queued (position 2)');
    store.apply({ jobs: [jobInfo({ id: 5, message: 'Reading 40 of 80 lines' })], paused: [] });
    f.detectChanges();
    expect(line()).toBe('Reading 40 of 80 lines');
    store.apply({ jobs: [jobInfo({ id: 9, message: 'another job' })], paused: [] });
    f.detectChanges();
    expect(line()).toBe('The local model proposes…');
  });

  it('Cancel cancels the job and disables itself', async () => {
    const { f, backend } = setup();
    el<HTMLButtonElement>(f, '[data-testid=job-wait-cancel]').click();
    expect(backend.cancel).toHaveBeenCalledWith(5);
    f.detectChanges();
    expect(el<HTMLButtonElement>(f, '[data-testid=job-wait-cancel]').disabled).toBe(true);
    await settle(f);
    expect(all(f, '[role=alert]')).toHaveLength(0);
  });

  it('a refused Cancel shows why and can be tried again', async () => {
    const { f, backend } = setup();
    backend.cancel.mockRejectedValueOnce(new Error('this job cannot be cancelled'));
    el<HTMLButtonElement>(f, '[data-testid=job-wait-cancel]').click();
    await settle(f);
    expect(el(f, '[role=alert]').textContent).toContain('this job cannot be cancelled');
    expect(el<HTMLButtonElement>(f, '[data-testid=job-wait-cancel]').disabled).toBe(false);
    backend.cancel.mockRejectedValueOnce('plain');
    el<HTMLButtonElement>(f, '[data-testid=job-wait-cancel]').click();
    await settle(f);
    expect(el(f, '[role=alert]').textContent).toContain('plain');
  });
});
