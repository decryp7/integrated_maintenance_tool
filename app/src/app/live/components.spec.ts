import { TestBed } from '@angular/core/testing';
import { all, buttonByText, el, FakeDock, provideFakeDock, provideTinyBackend, settle, T0, text, tinyDataset } from '../../testing/helpers';
import { alert } from '../../testing/live-helpers';
import { MAX_DETECT_EVERY_MS, SourceInfo } from '../core/api.types';
import { MockBackend } from '../core/mock/mock-backend';
import { Platform } from '../core/platform';
import { WorkspaceStore } from '../core/workspace.store';
import { CommandRegistry } from '../extensions/command-registry';
import { UiStateRegistry } from '../lifecycle/ui-state';
import { problemText, Sidebar } from '../shell/sidebar';
import { DialogService, DIALOG_REF } from '../ui/dialog.service';
import { DEFAULT_LIVE_OPTIONS, LiveTail } from '../workspace/live-tail';
import { AlertsStore } from './alerts.store';
import { LiveFeed } from './live-feed';
import { LiveFollow } from './live-follow.store';
import { parseDetectEvery, sanitizeLiveOptions } from './live-ui-state';
import { LiveWindowControls } from './live-window-controls';
import { NewEventsPill } from './new-events-pill';
import { OpenLiveDialog } from './open-live-dialog';
import { ALERTS_VIEW, alertsTitle, LIVE_VIEWS, provideLive, TAIL_STATUS_VIEW } from './provide-live';
import { groupSources } from './source-groups';
import { TailStatusStore } from './tail-status.store';
import { at } from '../../testing/must';

const src = (id: number, path: string, group: string, to: number | null, records = 1): SourceInfo => ({
  id,
  path,
  name: path.split('/').pop()!,
  group,
  importer: 'text',
  fileKind: 'text',
  records,
  dropped: 0,
  skipped: 0,
  error: null,
  from: null,
  to,
});

/** The tiny dataset with web02's app.log split by profile routes into two logical sources (ADR 0020). */
function splitDataset() {
  const d = tinyDataset();
  const path = '/logs/web02/app.log';
  d.sources.push(
    { id: 7, path, name: 'billing@srv1', importer: 'text', skipped: 0, routedFrom: 'web02/app.log', problems: [{ kind: 'unmatchedLine', count: 3, sampleLines: [12, 40, 41] }] },
    { id: 8, path, name: 'auth@srv2', importer: 'text', skipped: 0, routedFrom: 'web02/app.log' },
  );
  d.records.push({ ...at(d.records, 8), source: 7, line: 1 }, { ...at(d.records, 8), source: 8, line: 2 });
  return d;
}

/** The tiny dataset plus rolled files of web02's app.log. */
function rolledDataset() {
  const d = tinyDataset();
  d.sources.push({ id: 7, path: '/logs/web02/app.log.1', name: 'app.log.1', importer: 'text', skipped: 0 }, { id: 8, path: '/logs/web02/app.log.2', name: 'app.log.2', importer: 'text', skipped: 0 });
  d.records.push({ ...at(d.records, 8), source: 7, ts: T0 - 60_000, line: 1 }, { ...at(d.records, 8), source: 8, ts: T0 - 120_000, line: 1 });
  return d;
}

function makePlatform() {
  return { isTauri: () => false, pickPaths: vi.fn(async (dir: boolean) => [dir ? '/live/dir' : '/live/file.log']), onFilesDropped: async () => () => undefined };
}

function setup(backend = new MockBackend({ latency: false, dataset: tinyDataset, liveMs: 100_000 }), extra: unknown[] = []) {
  const dock = new FakeDock();
  const platform = makePlatform();
  TestBed.configureTestingModule({ providers: [provideTinyBackend(backend), provideFakeDock(dock), { provide: Platform, useValue: platform }, ...(extra as never[])] });
  const store = TestBed.inject(WorkspaceStore);
  return { backend, store, dock, platform, live: TestBed.inject(LiveTail) };
}

describe('source groups', () => {
  it('collapses rolling files of one log into a group, newest first, keeping the order', () => {
    const items = groupSources([
      src(1, '/l/a.log', '/l/a.log', 50),
      src(2, '/l/b.log', '/l/b.log', 10, 5),
      src(3, '/l/a.log.1', '/l/a.log', 20, 7),
      src(4, '/l/c.log', '', null),
      src(5, '/l/a.log.2', '/l/a.log', 50, 2),
      src(6, '/l/d.log', 'd.log', null),
      src(9, '/l/d.log.1', 'd.log', null),
    ]);
    expect(items.map((i) => (i.kind === 'file' ? i.source.id : i.group.key))).toEqual(['/l/a.log', 2, 4, 'd.log']);
    const first = at(items, 0);
    const g = first.kind === 'group' ? first.group : null;
    expect(g!.label).toBe('a.log');
    expect(g!.sources.map((s) => s.id)).toEqual([5, 1, 3]);
    expect(g!.newest.id).toBe(5);
    expect(g!.records).toBe(10);
    const last = at(items, 3);
    const d = last.kind === 'group' ? last.group : null;
    expect(d!.newest.id).toBe(9);
    expect(groupSources([])).toEqual([]);
  });

  it('nests the logical sources of a split file under it, the own source of the file first', () => {
    const routed = (id: number, name: string, records: number): SourceInfo => ({ ...src(id, '/l/shared.log', `l/shared.log › ${name}`, null, records), name, routedFrom: 'l/shared.log' });
    const own = { ...src(3, '/l/shared.log', 'l/shared.log', null, 4), name: 'l/shared.log' };
    const items = groupSources([routed(1, 'web@a', 2), src(2, '/l/b.log', '/l/b.log', 10), own, routed(4, 'db@b', 1)]);
    expect(items.map((i) => (i.kind === 'file' ? i.source.id : i.group.key))).toEqual(['split:/l/shared.log', 2]);
    const first = at(items, 0);
    const g = first.kind === 'group' ? first.group : null;
    expect(g!.split).toBe(true);
    expect(g!.label).toBe('shared.log');
    expect(g!.sources.map((s) => s.id)).toEqual([3, 4, 1]);
    expect(g!.newest.id).toBe(3);
    expect(g!.records).toBe(7);
    // Without an own source, the label still names the file.
    const only = groupSources([routed(1, 'web@a', 2)]);
    const o = at(only, 0);
    expect(o.kind === 'group' && o.group.label).toBe('shared.log');
    expect(o.kind === 'group' && o.group.newest.id).toBe(1);
  });

  it('describes import problems in words', () => {
    expect(problemText({ kind: 'unmatchedLine', count: 3, sampleLines: [12] })).toContain('first on line 12');
    expect(problemText({ kind: 'unmatchedLine', count: 3, sampleLines: [] })).toBe('3 lines look like events but match no format (kept with the event above)');
    expect(problemText({ kind: 'routeLimit', count: 2, sampleLines: [1] })).toContain('logical-source limit');
    expect(problemText({ kind: 'lineTooLong', count: 1, sampleLines: [1] })).toContain('lineTooLong');
  });

  it('the sidebar shows a split file as a row of its logical sources', async () => {
    setup(new MockBackend({ latency: false, dataset: splitDataset }));
    const fixture = TestBed.createComponent(Sidebar);
    await settle(fixture);
    const row = el<HTMLButtonElement>(fixture, '[data-testid=source-group]');
    expect(row.textContent).toContain('app.log');
    expect(el(fixture, '[data-testid=split-badge]').textContent).toContain('3 sources');
    expect(text(fixture)).not.toContain('newest:');
    expect(row.parentElement!.getAttribute('title')).toContain('Split into 3 sources by profile routes');
    row.click();
    await settle(fixture);
    const members = all(fixture, '.src.member');
    expect(members.map((m) => m.querySelector('.name')!.textContent!.trim())).toEqual(['app.log', 'auth@srv2', 'billing@srv1']);
    const billing = at(members, 2).getAttribute('title')!;
    expect(billing).toContain('Split from web02/app.log by a profile route');
    expect(billing).toContain('3 lines look like events but match no format (kept with the event above), first on line 12');
  });

  it('the sidebar shows a group row that expands, with state badges explained in words', async () => {
    const { backend } = setup(new MockBackend({ latency: false, dataset: rolledDataset }));
    const fixture = TestBed.createComponent(Sidebar);
    await settle(fixture);
    const row = el<HTMLButtonElement>(fixture, '[data-testid=source-group]');
    expect(row.getAttribute('aria-expanded')).toBe('false');
    expect(row.textContent).toContain('app.log');
    expect(row.textContent).toContain('3 files');
    expect(text(fixture)).toContain('newest: app.log');
    row.click();
    await settle(fixture);
    expect(row.getAttribute('aria-expanded')).toBe('true');
    expect(all(fixture, '.src.member').length).toBe(3);
    const real = await backend.tailStatus();
    vi.spyOn(backend, 'tailStatus').mockResolvedValue({
      ...real,
      files: real.files.map((f) => (f.sourceId === 7 ? { ...f, state: { kind: 'rotated' } } : f.sourceId === 8 ? { ...f, state: { kind: 'error', message: 'locked' } } : f)),
    });
    await TestBed.inject(TailStatusStore).refresh();
    await settle(fixture);
    const badges = all(fixture, '.state');
    expect(badges.map((b) => b.textContent?.trim())).toEqual(['Rotated', 'Error']);
    expect(at(badges, 0).getAttribute('aria-label')).toContain('Renamed by log rotation');
    expect(at(badges, 1).getAttribute('title')).toContain('locked');
    row.click();
    await settle(fixture);
    expect(all(fixture, '.src.member').length).toBe(0);
    const open = vi.spyOn(TestBed.inject(DialogService), 'open');
    all<HTMLButtonElement>(fixture, 'button').find((b) => b.getAttribute('aria-label') === 'Open for live tail')!.click();
    await vi.waitFor(() => expect(open).toHaveBeenCalled());
  });
});

describe('live window controls and the new events pill', () => {
  it('shows nothing while live tail is off, then Follow, the window, custom minutes and the paused chip', async () => {
    const { live, store } = setup();
    const follow = TestBed.inject(LiveFollow);
    const fixture = TestBed.createComponent(LiveWindowControls);
    await settle(fixture);
    expect(text(fixture).trim()).toBe('');
    await live.set(true);
    await settle(fixture);
    const toggle = el<HTMLButtonElement>(fixture, '[data-testid=follow-toggle]');
    expect(toggle.getAttribute('aria-pressed')).toBe('true');
    const select = el<HTMLSelectElement>(fixture, 'select');
    select.value = 'custom';
    select.dispatchEvent(new Event('change'));
    await settle(fixture);
    const minutes = el<HTMLInputElement>(fixture, 'input[aria-label="Custom window in minutes"]');
    minutes.value = '7';
    minutes.dispatchEvent(new Event('change'));
    minutes.value = 'x';
    minutes.dispatchEvent(new Event('change'));
    await settle(fixture);
    expect(follow.customMinutes()).toBe(7);
    expect(store.query().window).toEqual({ kind: 'live', lastMs: 420_000 });
    store.setRange(1, 2);
    await settle(fixture);
    const chip = el<HTMLButtonElement>(fixture, '[data-testid=resume-following]');
    expect(chip.textContent).toContain('Paused — Resume following');
    chip.click();
    await settle(fixture);
    expect(follow.paused()).toBe(false);
    el<HTMLButtonElement>(fixture, '[data-testid=follow-toggle]').click();
    await settle(fixture);
    expect(follow.follow()).toBe(false);
    await live.set(false);
  });

  it('the pill appears with the count of held events and reveals them', async () => {
    setup();
    const feed = TestBed.inject(LiveFeed);
    const fixture = TestBed.createComponent(NewEventsPill);
    await settle(fixture);
    expect(all(fixture, 'button').length).toBe(0);
    feed.newEvents.set(1);
    await settle(fixture);
    expect(text(fixture)).toContain('1 new event — show');
    feed.newEvents.set(1200);
    await settle(fixture);
    expect(text(fixture)).toContain('1,200 new events');
    el<HTMLButtonElement>(fixture, 'button').click();
    expect(feed.newEvents()).toBe(0);
  });
});

describe('open for live tail', () => {
  it('starts at the end by default, or imports the history when unticked', async () => {
    const close = vi.fn();
    const { live, platform } = setup(undefined, [{ provide: DIALOG_REF, useValue: { close, closed: Promise.resolve() } }]);
    const openForLive = vi.spyOn(live, 'openForLive').mockResolvedValue();
    const fixture = TestBed.createComponent(OpenLiveDialog);
    await settle(fixture);
    expect(text(fixture)).toContain('Live: start at end (only new lines)');
    buttonByText(fixture, 'Open folder…').click();
    await vi.waitFor(() => expect(openForLive).toHaveBeenCalledWith(['/live/dir'], true));
    expect(close).toHaveBeenCalledWith(true);
    el<HTMLInputElement>(fixture, '[data-testid=start-at-end]').click();
    await settle(fixture);
    buttonByText(fixture, 'Open files…').click();
    await vi.waitFor(() => expect(openForLive).toHaveBeenCalledWith(['/live/file.log'], false));
    platform.pickPaths.mockResolvedValueOnce([]);
    buttonByText(fixture, 'Open files…').click();
    await settle(fixture);
    expect(openForLive).toHaveBeenCalledTimes(2);
    platform.pickPaths.mockRejectedValueOnce(new Error('dialog failed'));
    buttonByText(fixture, 'Open files…').click();
    await settle(fixture);
    expect(text(fixture)).toContain('dialog failed');
    buttonByText(fixture, 'Cancel').click();
    expect(close).toHaveBeenLastCalledWith();
  });
});

describe('provideLive', () => {
  afterEach(() => document.querySelectorAll('imt-alert-toasts').forEach((e) => e.remove()));

  it('registers the views, commands and saved state, mounts the toasts and keeps the alerts tab count', async () => {
    const backend = new MockBackend({ latency: false, dataset: tinyDataset, liveMs: 100_000 });
    const sub = vi.spyOn(backend, 'onLiveAlert');
    const setTitle = vi.fn();
    (FakeDock.prototype as unknown as { setTitle: typeof setTitle }).setTitle = setTitle;
    const { dock, live } = setup(backend, [provideLive()]);
    TestBed.inject(CommandRegistry);
    await settle();
    expect(document.querySelector('imt-alert-toasts')).not.toBeNull();
    expect(sub).toHaveBeenCalled();
    expect(LIVE_VIEWS.map((v) => v.id)).toEqual([ALERTS_VIEW, TAIL_STATUS_VIEW]);
    for (const v of LIVE_VIEWS) expect(await v.component()).toBeTruthy();

    // The tab count follows the alerts while the panel is open.
    const alerts = TestBed.inject(AlertsStore);
    alerts.receive(alert());
    await settle();
    expect(setTitle).not.toHaveBeenCalled();
    dock.openIds.set(new Set([ALERTS_VIEW]));
    await settle();
    expect(setTitle).toHaveBeenLastCalledWith(ALERTS_VIEW, 'Alerts (1)');
    expect(alertsTitle(0)).toBe('Alerts');
    expect(alertsTitle(5000)).toBe('Alerts (999+)');

    // Commands.
    const commands = TestBed.inject(CommandRegistry);
    const ids = commands.all().map((c) => c.id);
    for (const id of ['live.toggle', 'live.open', 'live.resume', 'live.alerts', 'live.status', 'live.rules']) expect(ids).toContain(id);
    const open = vi.spyOn(TestBed.inject(DialogService), 'open');
    await commands.run('live.open');
    await commands.run('live.rules');
    expect(open).toHaveBeenCalledTimes(2);
    await commands.run('live.toggle');
    expect(live.enabled()).toBe(true);
    TestBed.inject(LiveFollow).pause();
    await commands.run('live.resume');
    expect(TestBed.inject(LiveFollow).paused()).toBe(false);
    await commands.run('live.alerts');
    await commands.run('live.status');
    expect(dock.calls).toEqual(expect.arrayContaining([`show:${ALERTS_VIEW}`, `show:${TAIL_STATUS_VIEW}`]));

    // Saved state: follow settings and the live options.
    const registry = TestBed.inject(UiStateRegistry);
    const saved = registry.serialize()['live'] as { follow: unknown; options: unknown };
    expect(saved).toEqual({ follow: { follow: true, preset: 'all', customMinutes: 30 }, options: DEFAULT_LIVE_OPTIONS });
    registry.restore({ live: { follow: { follow: false, preset: '5m', customMinutes: 3 }, options: { watch: 'polling', pollIntervalMs: 900 } } });
    await settle();
    expect(TestBed.inject(LiveFollow).preset()).toBe('5m');
    expect(live.options()).toMatchObject({ watch: 'polling', pollIntervalMs: 900 });
    registry.restore({});
    await settle();
    expect(TestBed.inject(LiveFollow).preset()).toBe('all');
    expect(live.options()).toEqual(DEFAULT_LIVE_OPTIONS);
    registry.restore({ live: null });
    expect(live.options()).toEqual(DEFAULT_LIVE_OPTIONS);
    await live.set(false);
    delete (FakeDock.prototype as unknown as { setTitle?: unknown }).setTitle;
  });

  it('sanitizes saved live options', () => {
    expect(sanitizeLiveOptions(null)).toEqual(DEFAULT_LIVE_OPTIONS);
    expect(sanitizeLiveOptions({ watch: 'events', pollIntervalMs: -1, deletedFiles: 'drop', carryFlushMs: 0 })).toEqual({
      watch: 'events',
      pollIntervalMs: 2000,
      deletedFiles: 'drop',
      carryFlushMs: 0,
      detectEveryMs: 1000,
    });
    expect(sanitizeLiveOptions({ watch: 'nope', carryFlushMs: -5 }).carryFlushMs).toBe(500);
    // The detector interval is kept within the engine's range.
    expect(sanitizeLiveOptions({ detectEveryMs: 0 }).detectEveryMs).toBe(0);
    expect(sanitizeLiveOptions({ detectEveryMs: 1e12 }).detectEveryMs).toBe(MAX_DETECT_EVERY_MS);
    expect(sanitizeLiveOptions({ detectEveryMs: -1 }).detectEveryMs).toBe(1000);
    expect(sanitizeLiveOptions({ detectEveryMs: 'x' }).detectEveryMs).toBe(1000);
    expect(parseDetectEvery(' 250 ')).toBe(250);
    expect(parseDetectEvery('12.6')).toBe(13);
    expect(parseDetectEvery('1e3')).toBeNull();
    expect(parseDetectEvery('')).toBeNull();
    expect(parseDetectEvery(Number.NaN)).toBeNull();
  });
});
