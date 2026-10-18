import { TestBed } from '@angular/core/testing';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { AiBackend } from '../ai/ai-backend';
import { FakeContext } from '../analysis/test-setup';
import { LifecycleBackend } from '../core/tools-backend';
import { Logger } from '../core/logger';
import { COMMAND_CONTEXT } from '../extensions/command-registry';
import { IMT_ACTIVITY_ITEMS, IMT_COMMANDS, IMT_VIEWS } from '../extensions/tokens';
import { JobViewer } from '../jobs/job-viewer';
import { JobsBackend } from '../jobs/jobs-backend';
import { JobsStore } from '../jobs/jobs.store';
import { EMPTY_JOBS, JobInfo } from '../jobs/jobs.types';
import { FormatWizard } from '../format/format-wizard';
import { UI_STATE_CONTRIBUTIONS } from '../lifecycle/ui-state';
import { LiveAlerts } from '../live/live-alerts';
import { AlertsStore } from '../live/alerts.store';
import { UpdatesStore } from '../updates/updates.store';
import { DialogService } from '../ui/dialog.service';
import { DockService } from '../dock/dock.service';
import { FakeDock, MemoryLogger, settle } from '../../testing/helpers';
import { at, must } from '../../testing/must';
import { actionLabel, NoticeActions } from './notice-actions';
import { MockNotifications, mockNotifications, resetMockNotifications } from './mock-notifications';
import { NotificationsBackend, TauriNotificationsBackend } from './notifications-backend';
import { MAX_TOASTS, NotificationsStore, TOAST_MS } from './notifications.store';
import { EMPTY_NOTICES, Notice, NoticeAction, NoticesEvent, NoticesRecord, NoticesSnapshot, NOTICE_SOURCES } from './notifications.types';
import { sourceLabel, titleWithCount } from './notifications-text';
import { NotificationsPanel } from './notifications-panel';
import { notificationsActivityItems, notificationsCommands, NotificationsPersistence, notificationsViews, provideNotifications, startNotifications } from './provide-notifications';

function notice(p: Partial<Notice> & { id: number }): Notice {
  return {
    source: 'job',
    severity: 'info',
    title: `Notice ${p.id}`,
    body: '',
    at: p.id * 1000,
    firstAt: p.id * 1000,
    count: 1,
    read: false,
    primary: null,
    secondary: null,
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
  off = vi.fn();
  list = vi.fn<NotificationsBackend['list']>(async () => this.snapshot);
  markRead = vi.fn<NotificationsBackend['markRead']>(async () => true);
  markAllRead = vi.fn<NotificationsBackend['markAllRead']>(async () => 0);
  dismiss = vi.fn<NotificationsBackend['dismiss']>(async () => true);
  record = vi.fn<NotificationsBackend['record']>(async (): Promise<NoticesRecord> => ({ version: 1, notices: [] }));
  restore = vi.fn<NotificationsBackend['restore']>(async () => ({ snapshot: this.snapshot, warning: null }));
  onChange = vi.fn<NotificationsBackend['onChange']>(async (h) => {
    this.changed = h;
    return this.off;
  });
  onOpen = vi.fn<NotificationsBackend['onOpen']>(async (h) => {
    this.opened = h;
    return this.off;
  });
  push(e: NoticesEvent) {
    must(this.changed, 'handler')(e);
  }
}

function setupStore(snapshot: NoticesSnapshot = EMPTY_NOTICES) {
  const fake = new FakeNotices();
  fake.snapshot = snapshot;
  TestBed.configureTestingModule({ providers: [{ provide: NotificationsBackend, useValue: fake }, { provide: Logger, useFactory: () => new MemoryLogger() }] });
  return { fake, store: TestBed.inject(NotificationsStore) };
}

describe('TauriNotificationsBackend', () => {
  it('maps every method to its command and ignores malformed events', async () => {
    const calls: [string, unknown][] = [];
    const call = (async (cmd: string, args?: unknown) => {
      calls.push([cmd, args]);
      return cmd;
    }) as never;
    const handlers = new Map<string, (p: unknown) => void>();
    const b = new TauriNotificationsBackend(call, async (name, h) => {
      handlers.set(name, h);
      return () => undefined;
    });
    const cases: [() => Promise<unknown>, string, unknown][] = [
      [() => b.list(), 'notifications_list', undefined],
      [() => b.markRead(3), 'notifications_mark_read', { id: 3 }],
      [() => b.markAllRead(), 'notifications_mark_all_read', undefined],
      [() => b.dismiss(4), 'notifications_dismiss', { id: 4 }],
      [() => b.record(), 'notifications_record', undefined],
      [() => b.restore({ version: 1 }), 'notifications_restore', { record: { version: 1 } }],
    ];
    for (const [run, cmd, args] of cases) {
      calls.length = 0;
      expect(await run()).toBe(cmd);
      expect(calls).toEqual([[cmd, args]]);
    }

    const seen: NoticesEvent[] = [];
    await b.onChange((e) => seen.push(e));
    const change = must(handlers.get('imt://notifications'));
    change({ notices: [], unread: 0 });
    change({ notices: [], unread: 1, posted: notice({ id: 1 }) });
    for (const bad of [null, 'x', 4, {}, { notices: [] }, { notices: 'x', unread: 0 }, { notices: [], unread: '0' }]) change(bad);
    expect(seen).toHaveLength(2);
    expect(at(seen, 0).posted).toBeNull();
    expect(at(seen, 1).posted?.id).toBe(1);

    const ids: number[] = [];
    await b.onOpen((id) => ids.push(id));
    const open = must(handlers.get('imt://notifications-open'));
    open(5);
    open('5');
    open(null);
    expect(ids).toEqual([5]);
  });
});

describe('MockNotifications', () => {
  beforeEach(() => resetMockNotifications());
  afterEach(() => {
    vi.useRealTimers();
    resetMockNotifications();
  });

  it('posts messages, newest first, with defaults', async () => {
    const m = new MockNotifications();
    const events: NoticesEvent[] = [];
    await m.onChange((e) => events.push(e));
    const a = m.post({ source: 'plugin', severity: 'warning', title: 'A' });
    const b = m.post({ source: 'job', severity: 'error', title: 'B', body: 'why', primary: { type: 'showJobs' }, link: { jobId: 4 }, scope: 'app' });
    expect(a).toMatchObject({ id: 1, body: '', count: 1, read: false, primary: null, secondary: null, scope: 'project', dedupeKey: null });
    expect(a.link).toEqual({ window: 'main', jobId: null, recordId: null, pluginId: null, path: null });
    expect(b.link.jobId).toBe(4);
    const snap = await m.list();
    expect(snap.notices.map((n) => n.title)).toEqual(['B', 'A']);
    expect(snap.unread).toBe(2);
    expect(events.map((e) => e.posted?.title)).toEqual(['A', 'B']);
  });

  it('merges messages with the same dedupe key into one with a count and makes it unread again', async () => {
    vi.useFakeTimers();
    vi.setSystemTime(1000);
    const m = new MockNotifications();
    const first = m.post({ source: 'liveAlert', severity: 'warning', title: 'Spike', dedupeKey: 'k', body: 'one' });
    await m.markRead(first.id);
    vi.setSystemTime(5000);
    const second = m.post({ source: 'liveAlert', severity: 'error', title: 'Spike again', dedupeKey: 'k', body: 'two', primary: { type: 'showAlert', alertId: 'a' } });
    expect(second.id).toBe(first.id);
    expect(second).toMatchObject({ count: 2, read: false, title: 'Spike again', body: 'two', severity: 'error', at: 5000, firstAt: 1000, primary: { type: 'showAlert', alertId: 'a' } });
    const { notices, unread } = await m.list();
    expect(notices).toHaveLength(1);
    expect(unread).toBe(1);
    m.post({ source: 'liveAlert', severity: 'info', title: 'Other', dedupeKey: 'other' });
    expect((await m.list()).notices).toHaveLength(2);
  });

  it('keeps at most 200 messages and drops read ones first', async () => {
    const m = new MockNotifications();
    for (let i = 0; i < 200; i++) m.post({ source: 'job', severity: 'info', title: `n${i}` });
    await m.markRead(5);
    m.post({ source: 'job', severity: 'info', title: 'extra' });
    let ids = (await m.list()).notices.map((n) => n.id);
    expect(ids).toHaveLength(200);
    expect(ids).not.toContain(5);
    expect(ids).toContain(1);
    // Nothing read left: the oldest goes.
    m.post({ source: 'job', severity: 'info', title: 'extra 2' });
    ids = (await m.list()).notices.map((n) => n.id);
    expect(ids).toHaveLength(200);
    expect(ids).not.toContain(1);
    expect(ids).toContain(2);
  });

  it('marks read, marks all read and dismisses, reporting what changed', async () => {
    const m = new MockNotifications();
    const a = m.post({ source: 'job', severity: 'info', title: 'A' });
    const b = m.post({ source: 'job', severity: 'info', title: 'B' });
    m.post({ source: 'job', severity: 'info', title: 'C' });
    const events: NoticesEvent[] = [];
    await m.onChange((e) => events.push(e));
    expect(await m.markRead(a.id)).toBe(true);
    expect(await m.markRead(999)).toBe(false);
    expect((await m.list()).unread).toBe(2);
    expect(await m.markAllRead()).toBe(2);
    expect(await m.markAllRead()).toBe(0);
    expect(await m.dismiss(b.id)).toBe(true);
    expect(await m.dismiss(b.id)).toBe(false);
    expect((await m.list()).notices.map((n) => n.title)).toEqual(['C', 'A']);
    expect(events.every((e) => e.posted === null)).toBe(true);
  });

  it('records only the project messages and restores them next to the app ones', async () => {
    const m = new MockNotifications();
    m.post({ source: 'job', severity: 'info', title: 'project' });
    m.post({ source: 'update', severity: 'info', title: 'app', scope: 'app' });
    const record = await m.record();
    expect(record.version).toBe(1);
    expect(record.notices).toHaveLength(1);

    const other = new MockNotifications();
    other.post({ source: 'update', severity: 'info', title: 'app', scope: 'app' });
    const restored = await other.restore(record);
    expect(restored.warning).toBeNull();
    expect(restored.snapshot.notices.map((n) => n.title).sort()).toEqual(['app', 'project']);
    expect(new Set(restored.snapshot.notices.map((n) => n.id)).size).toBe(2);
    // A restore with nothing saved drops the project messages.
    const cleared = await other.restore(null);
    expect(cleared.snapshot.notices.map((n) => n.title)).toEqual(['app']);
  });

  it('reports a damaged record and keeps the app messages', async () => {
    const m = new MockNotifications();
    m.post({ source: 'job', severity: 'info', title: 'project' });
    m.post({ source: 'update', severity: 'info', title: 'app', scope: 'app' });
    for (const bad of [{ version: 9, notices: [] }, { version: 1, notices: 'x' }]) {
      const r = await m.restore(bad);
      expect(r.warning).toContain('damaged');
      expect(r.snapshot.notices.map((n) => n.title)).toEqual(['app']);
    }
  });

  it('tells open handlers which message an OS notification opened, until unsubscribed', async () => {
    const m = new MockNotifications();
    const ids: number[] = [];
    const off = await m.onOpen((id) => ids.push(id));
    m.open(3);
    off();
    m.open(4);
    expect(ids).toEqual([3]);
    const events: NoticesEvent[] = [];
    const offChange = await m.onChange((e) => events.push(e));
    offChange();
    m.post({ source: 'job', severity: 'info', title: 'x' });
    expect(events).toEqual([]);
  });

  it('shares one centre per page until reset', () => {
    const a = mockNotifications();
    expect(mockNotifications()).toBe(a);
    expect((window as unknown as { __imtMockNotifications?: unknown }).__imtMockNotifications).toBe(a);
    resetMockNotifications();
    expect(mockNotifications()).not.toBe(a);
  });
});

describe('NotificationsStore', () => {
  afterEach(() => {
    vi.useRealTimers();
    TestBed.resetTestingModule();
  });

  it('loads on init and follows changes', async () => {
    const { fake, store } = setupStore({ notices: [notice({ id: 1 })], unread: 1 });
    expect(store.loaded()).toBe(false);
    await settle();
    expect(store.loaded()).toBe(true);
    expect(store.notices().map((n) => n.id)).toEqual([1]);
    expect(store.unread()).toBe(1);
    fake.push(event([notice({ id: 2 }), notice({ id: 1, read: true })]));
    expect(store.notices().map((n) => n.id)).toEqual([2, 1]);
    expect(store.unread()).toBe(1);
  });

  it('previews a newly posted message as a toast, but not a read one or a live alert', async () => {
    const { fake, store } = setupStore();
    await settle();
    const a = notice({ id: 1 });
    fake.push(event([a], a));
    expect(store.toasts().map((t) => t.id)).toEqual([1]);
    const read = notice({ id: 2, read: true });
    fake.push(event([a, read], read));
    expect(store.toasts().map((t) => t.id)).toEqual([1]);
    const alert = notice({ id: 3, source: 'liveAlert' });
    fake.push(event([a, read, alert], alert));
    expect(store.toasts().map((t) => t.id)).toEqual([1]);
    fake.push(event([a, read, alert], null));
    expect(store.toasts().map((t) => t.id)).toEqual([1]);
  });

  it('shows at most three toasts, newest last, and a repeated message replaces its toast', async () => {
    const { fake, store } = setupStore();
    await settle();
    expect(MAX_TOASTS).toBe(3);
    const all = [1, 2, 3, 4].map((id) => notice({ id }));
    for (const n of all) fake.push(event(all, n));
    expect(store.toasts().map((t) => t.id)).toEqual([2, 3, 4]);
    const again = notice({ id: 3, count: 2 });
    fake.push(event(all, again));
    expect(store.toasts().map((t) => t.id)).toEqual([2, 4, 3]);
    expect(at(store.toasts(), 2).count).toBe(2);
  });

  it('dismisses a toast by itself after TOAST_MS', async () => {
    const { fake, store } = setupStore();
    await settle();
    vi.useFakeTimers();
    const a = notice({ id: 1 });
    fake.push(event([a], a));
    vi.advanceTimersByTime(TOAST_MS - 1);
    expect(store.toasts()).toHaveLength(1);
    vi.advanceTimersByTime(1);
    expect(store.toasts()).toHaveLength(0);
    expect(store.notices()).toHaveLength(1);
  });

  it('restarts the timer when a message is posted again and keeps a held toast open', async () => {
    const { fake, store } = setupStore();
    await settle();
    vi.useFakeTimers();
    const a = notice({ id: 1 });
    fake.push(event([a], a));
    vi.advanceTimersByTime(TOAST_MS - 1000);
    fake.push(event([a], a));
    vi.advanceTimersByTime(TOAST_MS - 1000);
    expect(store.toasts()).toHaveLength(1);
    vi.advanceTimersByTime(1000);
    expect(store.toasts()).toHaveLength(0);

    fake.push(event([a], a));
    store.holdToast(1);
    vi.advanceTimersByTime(TOAST_MS * 5);
    expect(store.toasts()).toHaveLength(1);
    store.dismissToast(1);
    expect(store.toasts()).toHaveLength(0);
    store.holdToast(99);
  });

  it('dismissing a message removes its toast and asks the engine', async () => {
    const { fake, store } = setupStore();
    await settle();
    const a = notice({ id: 1 });
    fake.push(event([a], a));
    await store.dismiss(1);
    expect(store.toasts()).toEqual([]);
    expect(fake.dismiss).toHaveBeenCalledWith(1);
  });

  it('filters by source and unread, and groups by source in order of appearance', async () => {
    const { fake, store } = setupStore();
    await settle();
    fake.push(
      event([
        notice({ id: 5, source: 'ai' }),
        notice({ id: 4, source: 'job', read: true }),
        notice({ id: 3, source: 'ai', read: true }),
        notice({ id: 2, source: 'update' }),
        notice({ id: 1, source: 'job' }),
      ]),
    );
    expect(store.groups().map((g) => [g.source, g.notices.map((n) => n.id)])).toEqual([
      ['ai', [5, 3]],
      ['job', [4, 1]],
      ['update', [2]],
    ]);
    store.toggleSource('job');
    expect(store.notices().map((n) => n.id)).toEqual([4, 1]);
    store.toggleSource('ai');
    expect(store.sources()).toEqual(['job', 'ai']);
    store.toggleSource('job');
    expect(store.sources()).toEqual(['ai']);
    store.setRead('unread');
    expect(store.notices().map((n) => n.id)).toEqual([5]);
    expect(store.unread()).toBe(3);
    store.clearSources();
    expect(store.notices().map((n) => n.id)).toEqual([5, 2, 1]);
    store.setRead('all');
    expect(store.notices()).toHaveLength(5);
  });

  it('focuses a message when an OS notification opens it and on request', async () => {
    const { fake, store } = setupStore();
    await settle();
    must(fake.opened)(7);
    expect(store.focus()).toBe(7);
    store.setFocus(null);
    expect(store.focus()).toBeNull();
    store.setFocus(2);
    expect(store.focus()).toBe(2);
  });

  it('forwards mark read / mark all read to the engine', async () => {
    const { fake, store } = setupStore();
    await settle();
    await store.markRead(3);
    await store.markAllRead();
    expect(fake.markRead).toHaveBeenCalledWith(3);
    expect(fake.markAllRead).toHaveBeenCalled();
  });

  it('keeps the last error and retries the same request', async () => {
    const { fake, store } = setupStore();
    await settle();
    await store.retryRequest();
    expect(fake.markRead).not.toHaveBeenCalled();
    fake.markRead.mockRejectedValueOnce(new Error('engine busy'));
    await store.markRead(1);
    expect(store.error()).toBe('engine busy');
    await store.retryRequest();
    expect(fake.markRead).toHaveBeenCalledTimes(2);
    expect(store.error()).toBeNull();
    fake.markAllRead.mockRejectedValueOnce('text');
    await store.markAllRead();
    expect(store.error()).toBe('text');
    store.dismissError();
    expect(store.error()).toBeNull();
    fake.markAllRead.mockRejectedValueOnce({});
    await store.markAllRead();
    expect(store.error()).toBe('The notification centre did not answer.');
  });

  it('stops listening when destroyed, also when subscriptions arrive late', async () => {
    const early = setupStore();
    await settle();
    TestBed.resetTestingModule();
    expect(early.fake.off).toHaveBeenCalledTimes(2);

    const fake = new FakeNotices();
    const release: (() => void)[] = [];
    const late = () => new Promise<() => void>((resolve) => release.push(() => resolve(fake.off)));
    fake.onChange.mockImplementation(late);
    fake.onOpen.mockImplementation(late);
    TestBed.configureTestingModule({ providers: [{ provide: NotificationsBackend, useValue: fake }, { provide: Logger, useFactory: () => new MemoryLogger() }] });
    TestBed.inject(NotificationsStore);
    await settle();
    TestBed.resetTestingModule();
    expect(fake.off).not.toHaveBeenCalled();
    release.forEach((r) => r());
    await settle();
    expect(fake.off).toHaveBeenCalledTimes(2);
  });

  it('survives failing subscriptions', async () => {
    const fake = new FakeNotices();
    fake.onChange.mockRejectedValue(new Error('x'));
    fake.onOpen.mockRejectedValue(new Error('y'));
    TestBed.configureTestingModule({ providers: [{ provide: NotificationsBackend, useValue: fake }, { provide: Logger, useFactory: () => new MemoryLogger() }] });
    const store = TestBed.inject(NotificationsStore);
    await settle();
    expect(store.loaded()).toBe(true);
  });
});

describe('notifications-text', () => {
  it('names every source', () => {
    expect(NOTICE_SOURCES.map(sourceLabel)).toEqual(['Jobs', 'Live alerts', 'Imports', 'Plugins', 'AI', 'Updates', 'Session']);
  });

  it('adds the count only when a message repeated', () => {
    expect(titleWithCount({ title: 'Error spike', count: 1 })).toBe('Error spike');
    expect(titleWithCount({ title: 'Error spike', count: 0 })).toBe('Error spike');
    expect(titleWithCount({ title: 'Error spike on WEB02', count: 4 })).toBe('Error spike on WEB02 (×4)');
  });
});

describe('NoticeActions', () => {
  afterEach(() => TestBed.resetTestingModule());

  function setup() {
    const ctx = new FakeContext();
    const jobsBackend = { retry: vi.fn(async () => 1 as number | null), list: vi.fn(async () => EMPTY_JOBS), onChange: vi.fn(async () => () => undefined) };
    const notices = new FakeNotices();
    const lifecycle = { pluginReset: vi.fn(async () => undefined) };
    const dialogs = { open: vi.fn() };
    const ai = { reset: vi.fn(async () => ({})) };
    const updates = { download: vi.fn(async () => undefined) };
    const alertEntries = [{ alert: { id: 'a1', title: 'Spike' } }];
    const alerts = { alerts: () => alertEntries };
    const live = { show: vi.fn() };
    TestBed.configureTestingModule({
      providers: [
        { provide: NotificationsBackend, useValue: notices },
        { provide: JobsBackend, useValue: jobsBackend },
        { provide: COMMAND_CONTEXT, useValue: ctx },
        { provide: LifecycleBackend, useValue: lifecycle },
        { provide: DialogService, useValue: dialogs },
        { provide: AiBackend, useValue: ai },
        { provide: UpdatesStore, useValue: updates },
        { provide: AlertsStore, useValue: alerts },
        { provide: LiveAlerts, useValue: live },
        { provide: Logger, useFactory: () => new MemoryLogger() },
      ],
    });
    const actions = TestBed.inject(NoticeActions);
    const run = (action: NoticeAction, n = notice({ id: 11 })) => actions.run(n, action);
    return { actions, ctx, jobsBackend, notices, lifecycle, dialogs, ai, updates, alertEntries, live, run, alerts };
  }

  it('shows a job result with its title and result type through the JobViewer', async () => {
    const { run } = setup();
    const view = vi.spyOn(TestBed.inject(JobViewer), 'view').mockResolvedValue(null);
    TestBed.inject(JobsStore).apply({ jobs: [{ id: 7, title: 'Export CSV', resultType: 'file' } as JobInfo], paused: [] });
    expect(await run({ type: 'viewJob', jobId: 7 })).toBeNull();
    expect(view).toHaveBeenLastCalledWith({ id: 7, title: 'Export CSV', resultType: 'file' });
    expect(await run({ type: 'reviewProposals', jobId: 7 })).toBeNull();
    expect(view).toHaveBeenCalledTimes(2);
    await run({ type: 'viewJob', jobId: 99 });
    expect(view).toHaveBeenLastCalledWith({ id: 99, title: '', resultType: null });
  });

  it('returns the viewer message when the result cannot be shown', async () => {
    const { run } = setup();
    vi.spyOn(TestBed.inject(JobViewer), 'view').mockResolvedValue('gone');
    expect(await run({ type: 'viewJob', jobId: 1 })).toBe('gone');
  });

  it('retries a job and shows the Jobs pane', async () => {
    const { run, jobsBackend, ctx } = setup();
    expect(await run({ type: 'retryJob', jobId: 5 })).toBeNull();
    expect(jobsBackend.retry).toHaveBeenCalledWith(5);
    expect(ctx.last('showView')).toEqual(['jobs']);
  });

  it('shows the Jobs pane', async () => {
    const { run, ctx } = setup();
    expect(await run({ type: 'showJobs' })).toBeNull();
    expect(ctx.last('showView')).toEqual(['jobs']);
  });

  it('resets a plugin, resets the AI and installs an update with the existing features', async () => {
    const { run, lifecycle, ai, updates } = setup();
    expect(await run({ type: 'resetPlugin', pluginId: 'acme' })).toBeNull();
    expect(lifecycle.pluginReset).toHaveBeenCalledWith('acme');
    expect(await run({ type: 'resetAi' })).toBeNull();
    expect(ai.reset).toHaveBeenCalled();
    expect(await run({ type: 'installUpdate' })).toBeNull();
    expect(updates.download).toHaveBeenCalled();
  });

  it('opens the format wizard for a file', async () => {
    const { run, dialogs } = setup();
    expect(await run({ type: 'setUpFormat', path: 'C:\\logs\\app.log' })).toBeNull();
    expect(dialogs.open).toHaveBeenCalledWith(FormatWizard, expect.objectContaining({ title: 'Set up format', inputs: { path: 'C:\\logs\\app.log', name: 'app.log' }, width: 760 }));
    await run({ type: 'setUpFormat', path: '/var/log/syslog' });
    expect(at(dialogs.open.mock.calls, 1)[1]).toMatchObject({ inputs: { name: 'syslog' } });
  });

  it('shows a live alert, or the alerts view when it is gone', async () => {
    const { run, live, ctx, alertEntries } = setup();
    expect(await run({ type: 'showAlert', alertId: 'a1' })).toBeNull();
    expect(live.show).toHaveBeenCalledWith(at(alertEntries, 0).alert);
    expect(ctx.calls).toEqual([]);
    expect(await run({ type: 'showAlert', alertId: 'zzz' })).toBeNull();
    expect(live.show).toHaveBeenCalledTimes(1);
    expect(ctx.last('showView')).toEqual(['live-alerts']);
  });

  it('marks the message read whatever the action does', async () => {
    const { run, notices } = setup();
    await run({ type: 'showJobs' }, notice({ id: 21 }));
    expect(notices.markRead).toHaveBeenCalledWith(21);
  });

  it('returns a message when the action fails', async () => {
    const { run, jobsBackend, lifecycle, ai } = setup();
    jobsBackend.retry.mockRejectedValueOnce(new Error('cannot retry'));
    expect(await run({ type: 'retryJob', jobId: 1 })).toBe('cannot retry');
    lifecycle.pluginReset.mockRejectedValueOnce('plain text');
    expect(await run({ type: 'resetPlugin', pluginId: 'x' })).toBe('plain text');
    ai.reset.mockRejectedValueOnce({ odd: true });
    expect(await run({ type: 'resetAi' })).toBe('That did not work.');
  });

  it('labels every action', () => {
    const labels: [NoticeAction, string][] = [
      [{ type: 'viewJob', jobId: 1 }, 'View'],
      [{ type: 'retryJob', jobId: 1 }, 'Retry'],
      [{ type: 'reviewProposals', jobId: 1 }, 'Review proposals'],
      [{ type: 'resetPlugin', pluginId: 'p' }, 'Reset plugin'],
      [{ type: 'setUpFormat', path: 'p' }, 'Set up format'],
      [{ type: 'resetAi' }, 'Reset AI'],
      [{ type: 'installUpdate' }, 'Install update'],
      [{ type: 'showAlert', alertId: 'a' }, 'Show'],
      [{ type: 'showJobs' }, 'Show in Jobs'],
    ];
    for (const [a, label] of labels) expect(actionLabel(a)).toBe(label);
  });
});

describe('NotificationsPersistence', () => {
  afterEach(() => TestBed.resetTestingModule());

  const saved: NoticesRecord = { version: 1, notices: [{ id: 1 }] };

  it('keeps the engine record for saving, or nothing when there are no messages', async () => {
    const { fake } = setupStore();
    const p = TestBed.inject(NotificationsPersistence);
    await settle();
    expect(p.contribution.key).toBe('notifications');
    expect(p.contribution.tracksChanges).toBe(false);
    expect(p.contribution.capture()).toBeNull();
    fake.record.mockResolvedValueOnce(saved);
    await p.sync();
    expect(p.contribution.capture()).toEqual(saved);
    await p.sync();
    expect(p.record()).toBeNull();
  });

  it('syncs when a project message appears, repeats or is read, but not for app messages', async () => {
    const { fake, store } = setupStore();
    TestBed.inject(NotificationsPersistence);
    await settle();
    const base = fake.record.mock.calls.length;
    store.apply(event([notice({ id: 1 })]));
    await settle();
    expect(fake.record).toHaveBeenCalledTimes(base + 1);
    store.apply(event([notice({ id: 1 }), notice({ id: 2, scope: 'app' })]));
    await settle();
    expect(fake.record).toHaveBeenCalledTimes(base + 1);
    store.apply(event([notice({ id: 1, count: 2 })]));
    await settle();
    expect(fake.record).toHaveBeenCalledTimes(base + 2);
    store.apply(event([notice({ id: 1, count: 2, read: true })]));
    await settle();
    expect(fake.record).toHaveBeenCalledTimes(base + 3);
  });

  it('restores through the engine, applies the list without a toast and reports a warning', async () => {
    const { fake, store } = setupStore();
    const p = TestBed.inject(NotificationsPersistence);
    await settle();
    fake.restore.mockResolvedValueOnce({ snapshot: { notices: [notice({ id: 3 })], unread: 1 }, warning: 'damaged' });
    p.contribution.restore(saved);
    await settle();
    expect(fake.restore).toHaveBeenCalledWith(saved);
    expect(p.warning()).toBe('damaged');
    expect(store.notices().map((n) => n.id)).toEqual([3]);
    expect(store.toasts()).toEqual([]);
    must(p.contribution.reset)();
    await settle();
    expect(fake.restore).toHaveBeenLastCalledWith(null);
    await p.restore(undefined);
    expect(fake.restore).toHaveBeenLastCalledWith(null);
  });

  it('logs and carries on when the engine fails', async () => {
    const { fake } = setupStore();
    const p = TestBed.inject(NotificationsPersistence);
    await settle();
    const log = TestBed.inject(Logger) as MemoryLogger;
    fake.record.mockRejectedValueOnce(new Error('no record'));
    await p.sync();
    fake.restore.mockRejectedValueOnce('no restore');
    await p.restore({});
    const warns = log.entries.filter((e) => e.level === 'warn');
    expect(warns.map((w) => w.message)).toEqual(['Notifications could not be read for saving', 'Saved notifications not restored']);
    expect(warns.map((w) => w.props?.['Error'])).toEqual(['no record', 'no restore']);
  });
});

describe('provide-notifications', () => {
  afterEach(() => {
    document.querySelectorAll('imt-notification-toasts').forEach((e) => e.remove());
    TestBed.resetTestingModule();
  });

  it('describes the Notifications view', async () => {
    const [view] = notificationsViews();
    expect(view).toMatchObject({ id: 'notifications', title: 'Notifications', icon: 'bell', category: 'core', defaultPosition: 'right' });
    expect(await must(view).component()).toBe(NotificationsPanel);
  });

  it('offers show and mark-all-read commands, the latter only with unread messages', async () => {
    const { fake, store } = setupStore();
    await settle();
    const [show, mark] = notificationsCommands(store);
    const ctx = new FakeContext();
    must(show).run(ctx);
    expect(ctx.last('showView')).toEqual(['notifications']);
    expect(must(mark).when?.(ctx)).toBe(false);
    store.apply(event([notice({ id: 1 })]));
    expect(must(mark).when?.(ctx)).toBe(true);
    must(mark).run(ctx);
    expect(fake.markAllRead).toHaveBeenCalled();
  });

  it('badges the bell button with the unread count', async () => {
    const { store } = setupStore();
    await settle();
    const item = must(notificationsActivityItems(store)[0]);
    expect(item).toMatchObject({ id: 'notifications', icon: 'bell', section: 'bottom', run: { view: 'notifications' } });
    expect(must(item.badge)(new FakeContext())).toBeNull();
    store.apply(event([notice({ id: 1 }), notice({ id: 2 }), notice({ id: 3, read: true })]));
    expect(must(item.badge)(new FakeContext())).toBe(2);
  });

  it('mounts the toasts and shows Notifications when a message gets the focus', async () => {
    const dock = new FakeDock();
    const fake = new FakeNotices();
    TestBed.configureTestingModule({
      providers: [
        { provide: NotificationsBackend, useValue: fake },
        { provide: DockService, useValue: dock },
        { provide: COMMAND_CONTEXT, useValue: new FakeContext() },
        { provide: NoticeActions, useValue: {} },
        { provide: Logger, useFactory: () => new MemoryLogger() },
      ],
    });
    TestBed.runInInjectionContext(() => startNotifications());
    expect(document.querySelectorAll('imt-notification-toasts')).toHaveLength(1);
    TestBed.tick();
    expect(dock.calls).toEqual([]);
    TestBed.inject(NotificationsStore).setFocus(4);
    TestBed.tick();
    expect(dock.calls).toEqual(['show:notifications']);
  });

  it('registers the view, commands, button and saved state with one call', async () => {
    const fake = new FakeNotices();
    TestBed.configureTestingModule({
      providers: [{ provide: NotificationsBackend, useValue: fake }, { provide: DockService, useValue: new FakeDock() }, { provide: COMMAND_CONTEXT, useValue: new FakeContext() }, { provide: NoticeActions, useValue: {} }, { provide: Logger, useFactory: () => new MemoryLogger() }, provideNotifications()],
    });
    expect(TestBed.inject(IMT_VIEWS).flat().map((v) => v.id)).toContain('notifications');
    expect(TestBed.inject(IMT_COMMANDS).flat().map((c) => c.id)).toEqual(expect.arrayContaining(['notifications.show', 'notifications.markAllRead']));
    expect(TestBed.inject(IMT_ACTIVITY_ITEMS).flat().map((i) => i.id)).toContain('notifications');
    expect(TestBed.inject(UI_STATE_CONTRIBUTIONS).flat().map((c) => c.key)).toContain('notifications');
    await settle();
    expect(fake.list).toHaveBeenCalled();
  });
});
