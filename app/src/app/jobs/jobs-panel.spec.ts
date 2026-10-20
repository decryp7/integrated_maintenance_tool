import { ComponentFixture, TestBed } from '@angular/core/testing';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { AiBackend } from '../ai/ai-backend';
import { FakeContext } from '../analysis/test-setup';
import { Logger } from '../core/logger';
import { COMMAND_CONTEXT } from '../extensions/command-registry';
import { NotificationsBackend } from '../notifications/notifications-backend';
import { Notice, NoticesEvent, NoticesSnapshot } from '../notifications/notifications.types';
import { all, el, MemoryLogger, settle, text } from '../../testing/helpers';
import { at, must } from '../../testing/must';
import { JobViewer } from './job-viewer';
import { JobsBackend } from './jobs-backend';
import { EMPTY_JOBS, JobInfo, JobsSnapshot } from './jobs.types';
import { JobsPanel } from './jobs-panel';
import { StatusJobs } from './status-jobs';

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
  record = vi.fn<JobsBackend['record']>(async () => ({ version: 1, jobs: [] }));
  restore = vi.fn<JobsBackend['restore']>(async () => ({ snapshot: EMPTY_JOBS, warning: null }));
  onChange = vi.fn<JobsBackend['onChange']>(async (h) => {
    this.handler = h;
    return () => undefined;
  });
  push(s: JobsSnapshot) {
    must(this.handler, 'handler')(s);
  }
}

class FakeNotices extends NotificationsBackend {
  snapshot: NoticesSnapshot = { notices: [], unread: 0 };
  list = vi.fn<NotificationsBackend['list']>(async () => this.snapshot);
  markRead = vi.fn<NotificationsBackend['markRead']>(async () => true);
  markAllRead = vi.fn<NotificationsBackend['markAllRead']>(async () => 0);
  dismiss = vi.fn<NotificationsBackend['dismiss']>(async () => true);
  record = vi.fn<NotificationsBackend['record']>(async () => ({ version: 1, notices: [] }));
  restore = vi.fn<NotificationsBackend['restore']>(async () => ({ snapshot: this.snapshot, warning: null }));
  onChange = vi.fn<NotificationsBackend['onChange']>(async (_h: (e: NoticesEvent) => void) => () => undefined);
  onOpen = vi.fn<NotificationsBackend['onOpen']>(async () => () => undefined);
}

function setup(snapshot: JobsSnapshot = EMPTY_JOBS) {
  const fake = new FakeJobs();
  fake.snapshot = snapshot;
  const ai = { reset: vi.fn(async () => ({})) };
  const viewer = { view: vi.fn(async (_j: unknown): Promise<string | null> => null) };
  const ctx = new FakeContext();
  TestBed.configureTestingModule({
    providers: [
      { provide: JobsBackend, useValue: fake },
      { provide: AiBackend, useValue: ai },
      { provide: JobViewer, useValue: viewer },
      { provide: COMMAND_CONTEXT, useValue: ctx },
      { provide: Logger, useFactory: () => new MemoryLogger() },
    ],
  });
  const f = TestBed.createComponent(JobsPanel);
  return { f, fake, ai, viewer, ctx, root: f.nativeElement as HTMLElement };
}

async function render(f: ComponentFixture<unknown>) {
  f.detectChanges();
  await settle(f);
  f.detectChanges();
}

const sample = (): JobsSnapshot => ({
  jobs: [
    jobInfo({ id: 1, kind: 'ai', title: 'Explain event 7', state: 'running', cancellable: true, elapsedMs: 5000, etaMs: 3000, progress: { fraction: 0.4, done: 2, total: 5, etaMs: 3000 }, message: 'reading' }),
    jobInfo({ id: 2, kind: 'ai', title: 'Summarize', state: 'queued', position: 1, movable: true, cancellable: true }),
    jobInfo({ id: 3, kind: 'ai', title: 'Root causes', state: 'queued', position: 2, movable: true, cancellable: true }),
    jobInfo({ id: 4, kind: 'export', title: 'Export CSV', state: 'done', resultType: 'file', unseen: true }),
    jobInfo({ id: 5, kind: 'import', title: 'Import logs', state: 'failed', retryable: true, error: 'file is locked' }),
  ],
  paused: [],
});

describe('JobsPanel', () => {
  afterEach(() => TestBed.resetTestingModule());

  it('shows an empty state when there are no jobs', async () => {
    const { f, root } = setup();
    await render(f);
    expect(el(f, '[data-testid=jobs-empty]').textContent).toContain('No jobs');
    expect(root.querySelector('h3')).toBeNull();
    expect((el(f, '[data-testid=jobs-clear]') as HTMLButtonElement).disabled).toBe(true);
  });

  it('groups jobs into Running, Queued and Finished with progress and queue positions', async () => {
    const { f, root } = setup(sample());
    await render(f);
    expect(all(f, 'h3').map((h) => h.textContent?.replace(/\s+/g, ' ').trim())).toEqual(['Running 1', 'Queued 2', 'Finished 2']);
    expect(root.querySelector('[data-testid=jobs-empty]')).toBeNull();

    const bar = el<HTMLProgressElement>(f, '[data-testid=job-1] progress');
    expect(bar.value).toBe(0.4);
    expect(bar.max).toBe(1);
    expect(bar.getAttribute('aria-label')).toBe('Explain event 7 progress');
    const meta = el(f, '[data-testid=job-1] .meta').textContent ?? '';
    expect(meta).toContain('elapsed');
    expect(meta).toContain('left');
    expect(meta).toContain('2 / 5');
    expect(meta).toContain('reading');

    expect(el(f, '[data-testid=job-state-1]').textContent).toBe('Running');
    expect(el(f, '[data-testid=job-state-2]').textContent).toBe('Queued (position 1)');
    expect(el(f, '[data-testid=job-state-3]').textContent).toBe('Queued (position 2)');
    expect(root.querySelector('[data-testid=job-2] progress')).toBeNull();
    expect(root.querySelector('[data-testid=job-2] .meta')?.textContent).not.toContain('elapsed');
    expect(el(f, '[data-testid=job-state-5]').textContent).toBe('Failed');
    expect(el(f, '[data-testid=job-5] .error').textContent).toBe('file is locked');
    expect(el(f, '[data-testid=job-4]').classList.contains('unseen')).toBe(true);
    expect(el(f, '[data-testid=job-2]').getAttribute('aria-label')).toBe('AI: Summarize, Queued (position 1)');
  });

  it('shows an indeterminate bar while the fraction is unknown', async () => {
    const { f } = setup({ jobs: [jobInfo({ id: 1, state: 'running', cancellable: true })], paused: [] });
    await render(f);
    const bar = el<HTMLProgressElement>(f, '[data-testid=job-1] progress');
    expect(bar.hasAttribute('value')).toBe(false);
  });

  it('cancels, retries and views through the store and the viewer', async () => {
    const { f, fake, viewer } = setup(sample());
    await render(f);
    el<HTMLButtonElement>(f, '[data-testid=job-cancel-1]').click();
    expect(fake.cancel).toHaveBeenCalledWith(1);
    expect(el(f, '[data-testid=job-cancel-1]').getAttribute('aria-label')).toBe('Cancel Explain event 7');
    el<HTMLButtonElement>(f, '[data-testid=job-retry-5]').click();
    expect(fake.retry).toHaveBeenCalledWith(5);
    expect(el(f, '[data-testid=job-retry-5]').getAttribute('aria-label')).toBe('Retry Import logs');
    // Job 4 is done with a file result: View is offered.
    el<HTMLButtonElement>(f, '[data-testid=job-view-4]').click();
    await settle(f);
    expect(viewer.view).toHaveBeenCalledWith(expect.objectContaining({ id: 4, title: 'Export CSV', resultType: 'file' }));
    expect(el(f, '[data-testid=job-view-4]').getAttribute('aria-label')).toBe('View the result of Export CSV');
  });

  it('offers View only for finished jobs that have a result, and shows why a view failed', async () => {
    const { f, viewer, root } = setup({ jobs: [jobInfo({ id: 1, resultType: null }), jobInfo({ id: 2, resultType: 'workspace' }), jobInfo({ id: 3, state: 'failed', resultType: 'workspace' })], paused: [] });
    await render(f);
    expect(root.querySelector('[data-testid=job-view-1]')).toBeNull();
    expect(root.querySelector('[data-testid=job-view-3]')).toBeNull();
    viewer.view.mockResolvedValueOnce('The result could not be opened.');
    el<HTMLButtonElement>(f, '[data-testid=job-view-2]').click();
    await settle(f);
    f.detectChanges();
    expect(el(f, 'p.err[role=alert]').textContent).toContain('could not be opened');
    el<HTMLButtonElement>(f, '[data-testid=job-view-2]').click();
    await settle(f);
    f.detectChanges();
    expect(root.querySelector('p.err')).toBeNull();
  });

  it('moves waiting AI jobs; the first cannot move up', async () => {
    const { f, fake } = setup(sample());
    await render(f);
    const first = all<HTMLButtonElement>(f, '[data-testid=job-2] .icon-btn');
    const second = all<HTMLButtonElement>(f, '[data-testid=job-3] .icon-btn');
    expect(first.map((b) => b.getAttribute('aria-label'))).toEqual(['Move Summarize up', 'Move Summarize down']);
    expect(at(first, 0).disabled).toBe(true);
    expect(at(first, 1).disabled).toBe(false);
    at(first, 1).click();
    expect(fake.move).toHaveBeenLastCalledWith(2, false);
    at(second, 0).click();
    expect(fake.move).toHaveBeenLastCalledWith(3, true);
    expect(all(f, '[data-testid=job-1] .icon-btn')).toHaveLength(0);
  });

  it('filters by kind with pressed chips and combines kinds', async () => {
    const { f } = setup(sample());
    await render(f);
    const chip = (k: string) => el<HTMLButtonElement>(f, `[data-testid=jobs-kind-${k}]`);
    expect(all(f, '.chip').map((c) => c.textContent)).toEqual(['AI', 'Research', 'Analysis', 'Import', 'Export', 'Download', 'Collection', 'Capture', 'Maintenance']);
    expect(chip('ai').getAttribute('aria-pressed')).toBe('false');
    chip('export').click();
    f.detectChanges();
    expect(chip('export').getAttribute('aria-pressed')).toBe('true');
    expect(all(f, '[data-testid^=job-state-]')).toHaveLength(1);
    chip('import').click();
    f.detectChanges();
    expect(all(f, '[data-testid^=job-state-]')).toHaveLength(2);
    chip('export').click();
    chip('import').click();
    f.detectChanges();
    expect(all(f, '[data-testid^=job-state-]')).toHaveLength(5);
    expect(chip('import').getAttribute('aria-pressed')).toBe('false');
  });

  it('clears finished jobs', async () => {
    const { f, fake } = setup(sample());
    await render(f);
    const clear = el<HTMLButtonElement>(f, '[data-testid=jobs-clear]');
    expect(clear.disabled).toBe(false);
    clear.click();
    expect(fake.clearFinished).toHaveBeenCalled();
  });

  it('warns when the AI queue is paused and resets the AI', async () => {
    const { f, ai, fake } = setup({ jobs: [], paused: [{ kind: 'ai', reason: 'the model crashed' }] });
    await render(f);
    expect(el(f, '[data-testid=jobs-paused]').textContent).toContain('AI requests are paused: the model crashed');
    el<HTMLButtonElement>(f, '[data-testid=jobs-paused] button').click();
    await settle(f);
    expect(ai.reset).toHaveBeenCalledTimes(1);
    f.detectChanges();
    expect(f.nativeElement.querySelector('p.err')).toBeNull();

    ai.reset.mockRejectedValueOnce(new Error('still broken'));
    el<HTMLButtonElement>(f, '[data-testid=jobs-paused] button').click();
    await settle(f);
    f.detectChanges();
    expect(el(f, 'p.err').textContent).toContain('still broken');

    ai.reset.mockRejectedValueOnce('as text');
    el<HTMLButtonElement>(f, '[data-testid=jobs-paused] button').click();
    await settle(f);
    f.detectChanges();
    expect(el(f, 'p.err').textContent).toContain('as text');

    fake.push({ jobs: [], paused: [{ kind: 'export', reason: 'disk full' }] });
    f.detectChanges();
    expect(el(f, '[data-testid=jobs-paused]').textContent).toContain('Export jobs are paused: disk full');
    expect(all(f, '[data-testid=jobs-paused] button')).toHaveLength(0);
  });

  it('shows a request failure inline with Retry and Dismiss', async () => {
    const { f, fake } = setup(sample());
    await render(f);
    fake.cancel.mockRejectedValueOnce(new Error('the engine is busy'));
    el<HTMLButtonElement>(f, '[data-testid=job-cancel-1]').click();
    await settle(f);
    f.detectChanges();
    const alert = el(f, 'div.err[role=alert]');
    expect(alert.textContent).toContain('the engine is busy');
    at(all<HTMLButtonElement>(f, 'div.err button'), 0).click();
    await settle(f);
    f.detectChanges();
    expect(fake.cancel).toHaveBeenCalledTimes(2);
    expect(f.nativeElement.querySelector('div.err')).toBeNull();

    fake.cancel.mockRejectedValueOnce('again');
    el<HTMLButtonElement>(f, '[data-testid=job-cancel-1]').click();
    await settle(f);
    f.detectChanges();
    el<HTMLButtonElement>(f, 'div.err button[aria-label=Dismiss]').click();
    f.detectChanges();
    expect(f.nativeElement.querySelector('div.err')).toBeNull();
  });

  it('announces a state change to screen readers, but not the first list', async () => {
    const { f, fake } = setup({ jobs: [jobInfo({ id: 1, title: 'Export CSV', state: 'running' })], paused: [] });
    await render(f);
    expect(el(f, '[data-testid=jobs-live]').textContent).toBe('');
    expect(el(f, '[data-testid=jobs-live]').getAttribute('aria-live')).toBe('polite');
    fake.push({ jobs: [jobInfo({ id: 1, title: 'Export CSV', state: 'running', elapsedMs: 9 })], paused: [] });
    await render(f);
    expect(el(f, '[data-testid=jobs-live]').textContent).toBe('');
    fake.push({ jobs: [jobInfo({ id: 1, title: 'Export CSV', state: 'done' }), jobInfo({ id: 2, title: 'New one', state: 'queued', position: 1 })], paused: [] });
    await render(f);
    expect(el(f, '[data-testid=jobs-live]').textContent).toBe('Export CSV: Done');
  });

  it('moves between jobs with the arrow keys', async () => {
    const { f } = setup({ jobs: [jobInfo({ id: 1, retryable: true, state: 'failed' }), jobInfo({ id: 2, retryable: true, state: 'failed' }), jobInfo({ id: 3, retryable: true, state: 'failed' })], paused: [] });
    await render(f);
    const retry = (id: number) => el<HTMLButtonElement>(f, `[data-testid=job-retry-${id}]`);
    const press = (target: HTMLElement, key: string) => {
      const e = new KeyboardEvent('keydown', { key, bubbles: true, cancelable: true });
      target.dispatchEvent(e);
      return e;
    };
    retry(1).focus();
    expect(document.activeElement).toBe(retry(1));
    const down = press(retry(1), 'ArrowDown');
    expect(down.defaultPrevented).toBe(true);
    expect(document.activeElement).toBe(retry(2));
    press(retry(2), 'ArrowDown');
    expect(document.activeElement).toBe(retry(3));
    const end = press(retry(3), 'ArrowDown');
    expect(end.defaultPrevented).toBe(false);
    expect(document.activeElement).toBe(retry(3));
    press(retry(3), 'ArrowUp');
    expect(document.activeElement).toBe(retry(2));
    press(retry(2), 'ArrowUp');
    const top = press(retry(1), 'ArrowUp');
    expect(top.defaultPrevented).toBe(false);
    expect(document.activeElement).toBe(retry(1));
    const other = press(retry(1), 'Tab');
    expect(other.defaultPrevented).toBe(false);
  });

  it('opens what started a job', async () => {
    const link = (l: JobInfo['source']['link'], id: number) => jobInfo({ id, title: `T${id}`, source: { window: 'main', link: l } });
    const { f, ctx, viewer } = setup({
      jobs: [link({ type: 'event', recordId: 42 }, 1), link({ type: 'query', ql: 'level >= error' }, 2), link({ type: 'range', from: 10, to: 20 }, 3), link({ type: 'chat', sessionId: 's' }, 4), link({ type: 'path', path: 'C:\\logs\\app.log' }, 5), link({ type: 'model', id: 'qwen' }, 6)],
      paused: [],
    });
    await render(f);
    const source = (id: number) => el<HTMLButtonElement>(f, `[data-testid=job-${id}] .link`);
    expect(source(1).textContent).toBe('event 42');
    expect(source(1).getAttribute('aria-label')).toBe('Started from event 42');
    source(1).click();
    expect(ctx.last('jumpTo')).toEqual([42]);
    source(2).click();
    expect(ctx.last('setQuery')).toEqual([{ ql: 'level >= error' }]);
    source(3).click();
    expect(ctx.last('setQuery')).toEqual([{ from: 10, to: 20 }]);
    source(4).click();
    await settle(f);
    expect(viewer.view).toHaveBeenCalledWith({ id: 4, title: 'T4', resultType: 'chat' });
    ctx.calls.length = 0;
    expect(source(5).textContent).toBe('app.log');
    source(5).click();
    source(6).click();
    expect(ctx.calls).toEqual([]);
    expect(all(f, '[data-testid=job-4] .link')).toHaveLength(1);
  });
});

describe('StatusJobs', () => {
  afterEach(() => TestBed.resetTestingModule());

  function setupStatus(jobs: JobInfo[], unread: number) {
    const fake = new FakeJobs();
    fake.snapshot = { jobs, paused: [] };
    const notices = new FakeNotices();
    notices.snapshot = { notices: [] as Notice[], unread };
    const ctx = new FakeContext();
    TestBed.configureTestingModule({
      providers: [
        { provide: JobsBackend, useValue: fake },
        { provide: NotificationsBackend, useValue: notices },
        { provide: COMMAND_CONTEXT, useValue: ctx },
        { provide: Logger, useFactory: () => new MemoryLogger() },
      ],
    });
    const f = TestBed.createComponent(StatusJobs);
    return { f, fake, ctx };
  }

  it('hides the job count when nothing runs and shows the bell without a count', async () => {
    const { f } = setupStatus([jobInfo({ id: 1 })], 0);
    await render(f);
    expect(f.nativeElement.querySelector('[data-testid=status-jobs]')).toBeNull();
    const bell = el(f, '[data-testid=status-notifications]');
    expect(bell.getAttribute('aria-label')).toBe('Notifications');
    expect(bell.textContent?.trim()).toBe('');
    expect(bell.classList.contains('on')).toBe(false);
  });

  it('shows the running and waiting job count and opens the Jobs pane', async () => {
    const { f, ctx, fake } = setupStatus([jobInfo({ id: 1, state: 'running' }), jobInfo({ id: 2, state: 'queued' }), jobInfo({ id: 3 })], 0);
    await render(f);
    const pill = el<HTMLButtonElement>(f, '[data-testid=status-jobs]');
    expect(pill.textContent?.trim()).toBe('2');
    expect(pill.getAttribute('aria-label')).toBe('2 jobs running or waiting');
    pill.click();
    expect(ctx.last('showView')).toEqual(['jobs']);
    fake.push({ jobs: [jobInfo({ id: 1, state: 'running' })], paused: [] });
    f.detectChanges();
    expect(el(f, '[data-testid=status-jobs]').getAttribute('aria-label')).toBe('1 job running or waiting');
    fake.push({ jobs: [], paused: [] });
    f.detectChanges();
    expect(f.nativeElement.querySelector('[data-testid=status-jobs]')).toBeNull();
  });

  it('shows the unread notification count on the bell and opens Notifications', async () => {
    const { f, ctx } = setupStatus([], 3);
    await render(f);
    const bell = el<HTMLButtonElement>(f, '[data-testid=status-notifications]');
    expect(bell.textContent?.trim()).toBe('3');
    expect(bell.getAttribute('aria-label')).toBe('3 unread notifications');
    expect(bell.classList.contains('on')).toBe(true);
    bell.click();
    expect(ctx.last('showView')).toEqual(['notifications']);
    expect(text(f)).toContain('3');
  });

  it('words a single unread notification in the singular', async () => {
    const { f } = setupStatus([], 1);
    await render(f);
    expect(el(f, '[data-testid=status-notifications]').getAttribute('aria-label')).toBe('1 unread notification');
  });
});
