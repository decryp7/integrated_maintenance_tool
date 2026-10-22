import { ComponentFixture, TestBed } from '@angular/core/testing';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { FakeContext } from '../analysis/test-setup';
import { Logger } from '../core/logger';
import { WorkspaceStore } from '../core/workspace.store';
import { COMMAND_CONTEXT } from '../extensions/command-registry';
import { all, buttonByText, el, MemoryLogger, settle } from '../../testing/helpers';
import { at, must } from '../../testing/must';
import { JobsBackend } from '../jobs/jobs-backend';
import { JobsStore } from '../jobs/jobs.store';
import { JobInfo } from '../jobs/jobs.types';
import { NoticeActions } from './notice-actions';
import { NotificationToasts } from './notification-toasts';
import { NotificationsBackend } from './notifications-backend';
import { NotificationsPanel } from './notifications-panel';
import { NotificationsStore, TOAST_MS } from './notifications.store';
import { EMPTY_NOTICES, Notice, NoticeAction, NoticesEvent, NoticesSnapshot } from './notifications.types';

function notice(p: Partial<Notice> & { id: number }): Notice {
  return {
    source: 'job',
    severity: 'info',
    title: `Notice ${p.id}`,
    body: '',
    at: Date.UTC(2026, 0, 2, 3, 4, 5) + p.id,
    firstAt: 0,
    count: 1,
    read: false,
    primary: null,
    secondary: null,
    tertiary: null,
    link: { window: 'main', jobId: null, recordId: null, pluginId: null, path: null },
    dedupeKey: null,
    scope: 'project',
    ...p,
  };
}

const event = (notices: Notice[], posted: Notice | null = null): NoticesEvent => ({ notices, unread: notices.filter((n) => !n.read).length, posted });

class FakeNotices extends NotificationsBackend {
  snapshot: NoticesSnapshot = EMPTY_NOTICES;
  changed: ((e: NoticesEvent) => void) | null = null;
  opened: ((id: number) => void) | null = null;
  list = vi.fn<NotificationsBackend['list']>(async () => this.snapshot);
  markRead = vi.fn<NotificationsBackend['markRead']>(async () => true);
  markAllRead = vi.fn<NotificationsBackend['markAllRead']>(async () => 0);
  dismiss = vi.fn<NotificationsBackend['dismiss']>(async () => true);
  record = vi.fn<NotificationsBackend['record']>(async () => ({ version: 1, notices: [] }));
  restore = vi.fn<NotificationsBackend['restore']>(async () => ({ snapshot: this.snapshot, warning: null }));
  onChange = vi.fn<NotificationsBackend['onChange']>(async (h) => {
    this.changed = h;
    return () => undefined;
  });
  onOpen = vi.fn<NotificationsBackend['onOpen']>(async (h) => {
    this.opened = h;
    return () => undefined;
  });
  push(e: NoticesEvent) {
    must(this.changed, 'handler')(e);
  }
}

function providers(fake: FakeNotices, ctx: FakeContext, actions: { run: (n: Notice, a: NoticeAction) => Promise<string | null> }) {
  return [
    { provide: NotificationsBackend, useValue: fake },
    { provide: COMMAND_CONTEXT, useValue: ctx },
    { provide: NoticeActions, useValue: actions },
    // The Jobs list is not known until a test applies one.
    { provide: JobsBackend, useValue: { list: () => new Promise(() => undefined), onChange: async () => () => undefined } },
    { provide: WorkspaceStore, useValue: { timezone: () => 'utc' } },
    { provide: Logger, useFactory: () => new MemoryLogger() },
  ];
}

function makeActions() {
  return { run: vi.fn(async (_n: Notice, _a: NoticeAction): Promise<string | null> => null) };
}

async function render(f: ComponentFixture<unknown>) {
  f.detectChanges();
  await settle(f);
  f.detectChanges();
}

const sample = (): Notice[] => [
  notice({ id: 5, source: 'ai', title: 'AI paused', body: 'The model crashed', severity: 'warning', primary: { type: 'resetAi' }, secondary: { type: 'showJobs' } }),
  notice({ id: 4, source: 'job', title: 'Finished: Export', read: true, primary: { type: 'viewJob', jobId: 1 } }),
  notice({ id: 3, source: 'ai', title: 'Earlier AI note', read: true }),
  notice({ id: 2, source: 'update', title: 'Update ready', count: 3, severity: 'success' }),
];

describe('NotificationsPanel', () => {
  afterEach(() => TestBed.resetTestingModule());

  function setup(notices: Notice[] = sample()) {
    const fake = new FakeNotices();
    fake.snapshot = { notices, unread: notices.filter((n) => !n.read).length };
    const ctx = new FakeContext();
    const actions = makeActions();
    TestBed.configureTestingModule({ providers: providers(fake, ctx, actions) });
    const f = TestBed.createComponent(NotificationsPanel);
    return { f, fake, ctx, actions, store: TestBed.inject(NotificationsStore), root: f.nativeElement as HTMLElement };
  }

  it('shows an empty state and disables Mark all read when there is nothing', async () => {
    const { f } = setup([]);
    await render(f);
    expect(el(f, '[data-testid=notices-empty]').textContent).toContain('Nothing needs your attention');
    expect(el<HTMLButtonElement>(f, '[data-testid=notices-mark-all]').disabled).toBe(true);
    expect(f.nativeElement.querySelector('h3')).toBeNull();
  });

  it('groups messages by source under headings with counts, time, body and read state', async () => {
    const { f, root } = setup();
    await render(f);
    expect(all(f, 'h3').map((h) => h.textContent)).toEqual(['AI', 'Jobs', 'Updates']);
    expect(all(f, '#notices-h-ai + ul li')).toHaveLength(2);
    const ai = el(f, '[data-testid=notice-5]');
    expect(ai.classList.contains('unread')).toBe(true);
    expect(ai.getAttribute('data-severity')).toBe('warning');
    expect(ai.getAttribute('aria-label')).toBe('AI paused, unread');
    expect(ai.querySelector('.body')?.textContent).toBe('The model crashed');
    expect(ai.querySelector('.body')?.classList).toContain('imt-selectable');
    expect(ai.querySelector('.time')?.textContent).toContain('2 Jan');
    expect(el(f, '[data-testid=notice-4]').getAttribute('aria-label')).toBe('Finished: Export, read');
    expect(el(f, '[data-testid=notice-4]').querySelector('.body')).toBeNull();
    expect(el(f, '[data-testid=notice-2] .title').textContent).toBe('Update ready (×3)');
    expect(root.querySelector('[data-testid=notices-empty]')).toBeNull();
    expect(el<HTMLButtonElement>(f, '[data-testid=notices-mark-all]').disabled).toBe(false);
  });

  it('filters with an All / Unread segmented control and a Source filter with counts (K5)', async () => {
    const { f } = setup();
    await render(f);
    expect(all(f, '.chip')).toHaveLength(0);
    const unread = el<HTMLButtonElement>(f, '[data-testid=notices-unread-only]');
    const allBtn = el<HTMLButtonElement>(f, '[data-testid=notices-view-all]');
    expect(allBtn.getAttribute('aria-pressed')).toBe('true');
    unread.click();
    f.detectChanges();
    expect(unread.getAttribute('aria-pressed')).toBe('true');
    expect(all(f, 'li.notice').map((e) => e.getAttribute('data-testid'))).toEqual(['notice-5', 'notice-2']);
    allBtn.click();
    f.detectChanges();
    el<HTMLButtonElement>(f, '[data-testid=kind-filter]').click();
    f.detectChanges();
    const labels = Array.from(document.querySelectorAll('[data-testid=kind-filter-list] [role=option]')).map((l) => l.textContent?.replace(/\s+/g, ' ').trim());
    expect(labels.some((l) => l?.includes('Updates1'))).toBe(true);
    expect(labels).toHaveLength(9);
    document.querySelector<HTMLElement>('[data-testid=kind-update]')!.click();
    f.detectChanges();
    expect(all(f, 'h3').map((h) => h.textContent)).toEqual(['Updates']);
    expect(el(f, '[data-testid=kind-filter]').textContent).toContain('Source: 1');
    document.querySelector<HTMLButtonElement>('[data-testid=kind-filter-clear]')!.click();
    f.detectChanges();
    expect(all(f, 'h3')).toHaveLength(3);
  });

  it('shows the empty state when the filters hide everything', async () => {
    const { f, store } = setup([notice({ id: 1, read: true })]);
    await render(f);
    store.setRead('unread');
    f.detectChanges();
    expect(el(f, '[data-testid=notices-empty]')).toBeTruthy();
  });

  it('runs the primary and secondary action through NoticeActions', async () => {
    const { f, actions } = setup();
    await render(f);
    const row = el(f, '[data-testid=notice-5]');
    const buttons = [...row.querySelectorAll('button')].map((b) => b.textContent?.trim());
    expect(buttons).toEqual(['Reset AI', 'Show in Jobs', 'Mark read', 'Dismiss']);
    el<HTMLButtonElement>(f, '[data-testid=notice-primary-5]').click();
    await settle(f);
    expect(actions.run).toHaveBeenLastCalledWith(expect.objectContaining({ id: 5 }), { type: 'resetAi' });
    at([...row.querySelectorAll('button')], 1).click();
    await settle(f);
    expect(actions.run).toHaveBeenLastCalledWith(expect.objectContaining({ id: 5 }), { type: 'showJobs' });
  });

  it('offers a third action when the message has one', async () => {
    const key = 'runtime:vulkan@b11700';
    const { f, actions } = setup([
      notice({ id: 9, source: 'ai', title: 'AI runtime update', primary: { type: 'updateAi', key }, secondary: { type: 'laterAiUpdate', key }, tertiary: { type: 'skipAiUpdate', key } }),
    ]);
    await render(f);
    const buttons = [...el(f, '[data-testid=notice-9]').querySelectorAll('button')].map((b) => b.textContent?.trim());
    expect(buttons).toEqual(['Update', 'Later', 'Skip this version', 'Mark read', 'Dismiss']);
    el<HTMLButtonElement>(f, '[data-testid=notice-tertiary-9]').click();
    await settle(f);
    expect(actions.run).toHaveBeenLastCalledWith(expect.objectContaining({ id: 9 }), { type: 'skipAiUpdate', key });
  });

  it('shows an action failure inline and clears it after a success', async () => {
    const { f, actions } = setup();
    await render(f);
    actions.run.mockResolvedValueOnce('The AI could not be reset.');
    el<HTMLButtonElement>(f, '[data-testid=notice-primary-5]').click();
    await settle(f);
    f.detectChanges();
    expect(el(f, 'p.err[role=alert]').textContent).toContain('The AI could not be reset.');
    el<HTMLButtonElement>(f, '[data-testid=notice-primary-5]').click();
    await settle(f);
    f.detectChanges();
    expect(f.nativeElement.querySelector('p.err')).toBeNull();
  });

  it('marks read, dismisses and marks all read through the engine', async () => {
    const { f, fake } = setup();
    await render(f);
    // Read messages offer no Mark read.
    expect(buttonByText(f, 'Mark read')).toBeTruthy();
    expect([...el(f, '[data-testid=notice-4]').querySelectorAll('button')].map((b) => b.textContent?.trim())).toEqual(['View', 'Dismiss']);
    const mark = at(all<HTMLButtonElement>(f, '[data-testid=notice-5] button'), 2);
    expect(mark.getAttribute('aria-label')).toBe('Mark read: AI paused');
    mark.click();
    expect(fake.markRead).toHaveBeenCalledWith(5);
    const dismiss = el<HTMLButtonElement>(f, '[data-testid=notice-dismiss-4]');
    expect(dismiss.getAttribute('aria-label')).toBe('Dismiss Finished: Export');
    dismiss.click();
    expect(fake.dismiss).toHaveBeenCalledWith(4);
    el<HTMLButtonElement>(f, '[data-testid=notices-mark-all]').click();
    expect(fake.markAllRead).toHaveBeenCalled();
  });

  it('highlights a message and focuses its first button when it gets the focus', async () => {
    const { f, store, fake } = setup();
    await render(f);
    const row = el(f, '[data-testid=notice-2]');
    const scroll = vi.fn();
    (row as unknown as { scrollIntoView: () => void }).scrollIntoView = scroll;
    store.setFocus(2);
    f.detectChanges();
    await settle(f);
    f.detectChanges();
    expect(row.classList.contains('focus')).toBe(true);
    expect(scroll).toHaveBeenCalledWith({ block: 'nearest' });
    expect(document.activeElement).toBe(row.querySelector('button'));
    // An OS notification click does the same for another message.
    must(fake.opened)(5);
    await settle(f);
    f.detectChanges();
    expect(el(f, '[data-testid=notice-5]').classList.contains('focus')).toBe(true);
    expect(row.classList.contains('focus')).toBe(false);
    // A message that is not listed is ignored.
    store.setFocus(404);
    await settle(f);
  });

  it('announces a new unread message but not the initial list or read messages', async () => {
    const { f, fake } = setup();
    await render(f);
    const live = el(f, '[data-testid=notices-live]');
    expect(live.getAttribute('aria-live')).toBe('polite');
    expect(live.textContent).toBe('');
    const base = sample();
    fake.push(event([notice({ id: 6, read: true, title: 'Quiet' }), ...base]));
    await render(f);
    expect(live.textContent).toBe('');
    fake.push(event([notice({ id: 7, title: 'Disk almost full', count: 2 }), notice({ id: 6, read: true, title: 'Quiet' }), ...base]));
    await render(f);
    expect(live.textContent).toBe('Disk almost full (×2)');
  });

  it('shows a request failure inline with Retry', async () => {
    const { f, fake } = setup();
    await render(f);
    fake.markAllRead.mockRejectedValueOnce(new Error('engine busy'));
    el<HTMLButtonElement>(f, '[data-testid=notices-mark-all]').click();
    await settle(f);
    f.detectChanges();
    expect(el(f, 'div.err[role=alert]').textContent).toContain('engine busy');
    buttonByText(f, 'Retry').click();
    await settle(f);
    f.detectChanges();
    expect(fake.markAllRead).toHaveBeenCalledTimes(2);
    expect(f.nativeElement.querySelector('div.err')).toBeNull();
  });
  it('hides the job actions of a message whose job was cleared from Jobs and says the outcome is all that is kept', async () => {
    const done = notice({ id: 9, source: 'job', title: 'Finished: Export', body: 'Bundle written', primary: { type: 'viewJob', jobId: 1 } });
    const failed = notice({ id: 8, source: 'job', title: 'Failed: Import', body: 'disk full', primary: { type: 'retryJob', jobId: 2 }, secondary: { type: 'showJobs' } });
    const { f } = setup([done, failed]);
    const jobs = TestBed.inject(JobsStore);
    await render(f);
    // Until the Jobs list is known every action stays offered.
    expect(f.nativeElement.querySelector('[data-testid=notice-primary-9]')).not.toBeNull();
    expect(f.nativeElement.querySelector('[data-testid=notice-cleared-9]')).toBeNull();
    jobs.apply({ jobs: [{ id: 2 } as JobInfo], paused: [] });
    await render(f);
    expect(f.nativeElement.querySelector('[data-testid=notice-primary-9]')).toBeNull();
    expect(el(f, '[data-testid=notice-cleared-9]').textContent).toContain('cleared from Jobs');
    expect(el(f, '[data-testid=notice-9] .body').textContent).toBe('Bundle written');
    expect(el(f, '[data-testid=notice-primary-8]').textContent).toBe('Retry');
    expect(f.nativeElement.querySelector('[data-testid=notice-cleared-8]')).toBeNull();
    // Job 2 cleared too: Retry goes, "Show in Jobs" (no job of its own) stays.
    jobs.apply({ jobs: [], paused: [] });
    await render(f);
    expect(f.nativeElement.querySelector('[data-testid=notice-primary-8]')).toBeNull();
    expect(el(f, '[data-testid=notice-cleared-8]')).toBeTruthy();
    expect(buttonByText(f, 'Show in Jobs')).toBeTruthy();
  });
});

describe('NotificationToasts', () => {
  afterEach(() => {
    vi.useRealTimers();
    TestBed.resetTestingModule();
  });

  function setup() {
    const fake = new FakeNotices();
    const ctx = new FakeContext();
    const actions = makeActions();
    TestBed.configureTestingModule({ providers: providers(fake, ctx, actions) });
    const f = TestBed.createComponent(NotificationToasts);
    return { f, fake, ctx, actions, store: TestBed.inject(NotificationsStore) };
  }

  it('holds a toast under the pointer or focus and releases it once both have left', async () => {
    const { f, fake, store } = setup();
    await render(f);
    const n = notice({ id: 1, primary: { type: 'showJobs' } });
    fake.push(event([n], n));
    f.detectChanges();
    const toast = el(f, '[data-testid=notice-toast]');
    const hold = vi.spyOn(store, 'holdToast');
    const release = vi.spyOn(store, 'releaseToast');
    toast.dispatchEvent(new MouseEvent('mouseenter'));
    expect(hold).toHaveBeenCalledWith(1);
    el<HTMLButtonElement>(f, '[data-testid=notice-toast-primary]').focus();
    toast.dispatchEvent(new MouseEvent('mouseleave'));
    expect(release).not.toHaveBeenCalled(); // focus is still inside
    el<HTMLButtonElement>(f, '[data-testid=notice-toast-primary]').blur();
    toast.dispatchEvent(new FocusEvent('focusout', { relatedTarget: document.body }));
    expect(release).toHaveBeenCalledWith(1);
  });

  it('renders nothing until a message is posted, then previews it with its action', async () => {
    const { f, fake } = setup();
    await render(f);
    expect(all(f, '[data-testid=notice-toast]')).toHaveLength(0);
    expect(el(f, '[role=region]').getAttribute('aria-live')).toBe('polite');
    const n = notice({ id: 1, title: 'Failed: Export', body: 'disk full', severity: 'error', count: 2, primary: { type: 'retryJob', jobId: 4 } });
    fake.push(event([n], n));
    f.detectChanges();
    const toast = el(f, '[data-testid=notice-toast]');
    expect(toast.getAttribute('data-severity')).toBe('error');
    expect(toast.querySelector('.title')?.textContent).toBe('Failed: Export (×2)');
    expect(toast.querySelector('.body')?.textContent).toBe('disk full');
    expect(el(f, '[data-testid=notice-toast-primary]').textContent).toBe('Retry');
    expect(el(f, '[data-testid=notice-toast-show]').textContent).toBe('Show in Notifications');
  });

  it('a toast of the UI own (running in Jobs) has its action but no Show in Notifications', async () => {
    const { f, store, actions } = setup();
    await render(f);
    store.showLocal({ severity: 'info', title: 'Exporting to a.csv — running in Jobs', primary: { type: 'showJobs' } });
    f.detectChanges();
    expect(el(f, '[data-testid=notice-toast] .title').textContent).toBe('Exporting to a.csv — running in Jobs');
    expect(f.nativeElement.querySelector('[data-testid=notice-toast-show]')).toBeNull();
    el<HTMLButtonElement>(f, '[data-testid=notice-toast-primary]').click();
    await settle(f);
    expect(actions.run).toHaveBeenCalledWith(expect.objectContaining({ id: -1 }), { type: 'showJobs' });
  });

  it('leaves out the body and the primary button when the message has none', async () => {
    const { f, fake } = setup();
    await render(f);
    const n = notice({ id: 1 });
    fake.push(event([n], n));
    f.detectChanges();
    expect(f.nativeElement.querySelector('.body')).toBeNull();
    expect(f.nativeElement.querySelector('[data-testid=notice-toast-primary]')).toBeNull();
  });

  it('runs the primary action and closes the toast on success, but keeps it with the message on failure', async () => {
    const { f, fake, actions } = setup();
    await render(f);
    const a = notice({ id: 1, primary: { type: 'resetAi' } });
    const b = notice({ id: 2, primary: { type: 'showJobs' } });
    fake.push(event([a], a));
    fake.push(event([b, a], b));
    f.detectChanges();
    actions.run.mockResolvedValueOnce('did not work');
    at(all<HTMLButtonElement>(f, '[data-testid=notice-toast-primary]'), 0).click();
    await settle(f);
    f.detectChanges();
    expect(actions.run).toHaveBeenLastCalledWith(expect.objectContaining({ id: 1 }), { type: 'resetAi' });
    expect(all(f, '[data-testid=notice-toast]')).toHaveLength(2);
    expect(f.nativeElement.textContent).toContain('did not work');
    at(all<HTMLButtonElement>(f, '[data-testid=notice-toast-primary]'), 0).click();
    await settle(f);
    f.detectChanges();
    expect(all(f, '[data-testid=notice-toast]')).toHaveLength(1);
    expect(f.nativeElement.textContent).not.toContain('did not work');
  });

  it('Show in Notifications sets the focus, opens the view and closes the toast', async () => {
    const { f, fake, ctx, store } = setup();
    await render(f);
    const n = notice({ id: 9 });
    fake.push(event([n], n));
    f.detectChanges();
    el<HTMLButtonElement>(f, '[data-testid=notice-toast-show]').click();
    f.detectChanges();
    expect(store.focus()).toBe(9);
    expect(ctx.last('showView')).toEqual(['notifications']);
    expect(all(f, '[data-testid=notice-toast]')).toHaveLength(0);
  });

  it('dismisses with the close button, which leaves the message in the list', async () => {
    const { f, fake, store } = setup();
    await render(f);
    const n = notice({ id: 3 });
    fake.push(event([n], n));
    f.detectChanges();
    el<HTMLButtonElement>(f, 'button[aria-label="Dismiss notification"]').click();
    f.detectChanges();
    expect(all(f, '[data-testid=notice-toast]')).toHaveLength(0);
    expect(store.notices()).toHaveLength(1);
    expect(fake.dismiss).not.toHaveBeenCalled();
  });

  it('holds a toast while the pointer is on it and removes it after the timeout otherwise', async () => {
    const { f, fake } = setup();
    await render(f);
    vi.useFakeTimers();
    const a = notice({ id: 1 });
    const b = notice({ id: 2 });
    fake.push(event([a], a));
    fake.push(event([b, a], b));
    f.detectChanges();
    const toasts = all(f, '[data-testid=notice-toast]');
    at(toasts, 0).dispatchEvent(new MouseEvent('mouseenter'));
    vi.advanceTimersByTime(TOAST_MS);
    f.detectChanges();
    expect(all(f, '[data-testid=notice-toast]')).toHaveLength(1);
    at(all(f, '[data-testid=notice-toast]'), 0).dispatchEvent(new FocusEvent('focusin'));
    vi.advanceTimersByTime(TOAST_MS * 3);
    f.detectChanges();
    expect(all(f, '[data-testid=notice-toast]')).toHaveLength(1);
  });
});
