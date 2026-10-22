import { TestBed } from '@angular/core/testing';
import { provideTinyBackend, settle, testLogger, tinyBackend, tinyDataset } from '../../testing/helpers';
import { LiveState } from '../core/api.types';
import { DebriefBackend } from '../core/backend';
import { MockBackend } from '../core/mock/mock-backend';
import { WorkspaceStore } from '../core/workspace.store';
import { DEFAULT_LIVE_OPTIONS, LiveTail } from './live-tail';
import { at } from '../../testing/must';

describe('LiveTail', () => {
  async function setup(backend = new MockBackend({ latency: false, dataset: tinyDataset, liveMs: 15 })) {
    TestBed.configureTestingModule({ providers: [provideTinyBackend(backend)] });
    const store = TestBed.inject(WorkspaceStore);
    const live = TestBed.inject(LiveTail);
    await settle();
    return { backend, store, live };
  }

  it('starts off, turns on and off through the backend, and logs it', async () => {
    const { backend, live } = await setup();
    const setLive = vi.spyOn(backend, 'setLive');
    expect(live.enabled()).toBe(false);
    await live.toggle();
    expect(live.enabled()).toBe(true);
    expect(setLive).toHaveBeenLastCalledWith(true, DEFAULT_LIVE_OPTIONS);
    await live.set(true);
    expect(setLive).toHaveBeenCalledTimes(1);
    await live.set(false);
    expect(live.enabled()).toBe(false);
    expect(setLive).toHaveBeenLastCalledWith(false);
    expect(testLogger().entries.map((e) => e.message).filter((m) => m.startsWith('Live'))).toEqual(['Live tail on', 'Live tail off']);
  });

  it('refreshes the summary on every change, keeps the selection and the query, and marks the refresh as live', async () => {
    const { store, live } = await setup();
    store.toggleLevel('Information');
    const row = at((await TestBed.inject(DebriefBackend).queryPage({ query: store.effectiveQuery(), offset: 1, limit: 1, descending: false })).rows, 0);
    store.select(row);
    const version = store.dataVersion();
    expect(live.isLiveRefresh()).toBe(false);
    await live.set(true);
    await vi.waitFor(() => expect(store.summary()!.total).toBeGreaterThan(11));
    expect(store.dataVersion()).toBeGreaterThan(version);
    expect(store.selected()).toBe(row);
    expect(store.query().levels).toEqual(['Information']);
    expect(live.isLiveRefresh()).toBe(true);
    await live.set(false);
    store.applySummary(store.summary()!);
    expect(live.isLiveRefresh()).toBe(false);
  });

  it('takes over the engine live state on start (e.g. after a webview reload)', async () => {
    const backend = new MockBackend({ latency: false, dataset: tinyDataset, liveMs: 20 });
    await backend.setLive(true, { watch: 'polling', pollIntervalMs: 700, detectEveryMs: 250 });
    const setLive = vi.spyOn(backend, 'setLive');
    const subscribe = vi.spyOn(backend, 'onWorkspaceChanged');
    const { live, store } = await setup(backend);
    await vi.waitFor(() => expect(live.enabled()).toBe(true));
    expect(live.options()).toMatchObject({ watch: 'polling', pollIntervalMs: 700, detectEveryMs: 250 });
    expect(subscribe).toHaveBeenCalledTimes(1);
    expect(testLogger().entries.some((e) => e.message === 'Live tail on (restored)')).toBe(true);
    // Changes are followed without starting the watchers again.
    const version = store.dataVersion();
    await vi.waitFor(() => expect(store.dataVersion()).toBeGreaterThan(version), { timeout: 5000 });
    expect(setLive.mock.calls.filter(([on]) => on).length).toBeLessThanOrEqual(1);
    await live.set(false);
    expect(setLive).toHaveBeenLastCalledWith(false);
  });

  it('takes over the options of an engine that is off, and keeps defaults when live_state fails', async () => {
    const backend = new MockBackend({ latency: false, dataset: tinyDataset, liveMs: 100_000 });
    await backend.setLive(true, { detectEveryMs: 5000 });
    await backend.setLive(false);
    const { live } = await setup(backend);
    expect(live.enabled()).toBe(false);
    expect(live.options().detectEveryMs).toBe(5000);

    TestBed.resetTestingModule();
    const failing = new MockBackend({ latency: false, dataset: tinyDataset });
    vi.spyOn(failing, 'liveState').mockRejectedValueOnce('no state');
    const second = await setup(failing);
    expect(second.live.enabled()).toBe(false);
    expect(second.live.options()).toEqual(DEFAULT_LIVE_OPTIONS);
    expect(testLogger().entries.some((e) => e.level === 'warn' && e.message === 'Live tail state unavailable')).toBe(true);
  });

  it('a user change made while live_state is pending wins over the restored state', async () => {
    const backend = new MockBackend({ latency: false, dataset: tinyDataset, liveMs: 100_000 });
    let answer!: (s: LiveState) => void;
    vi.spyOn(backend, 'liveState').mockImplementationOnce(() => new Promise((r) => (answer = r)));
    TestBed.configureTestingModule({ providers: [provideTinyBackend(backend)] });
    const live = TestBed.inject(LiveTail);
    await live.setOptions({ ...DEFAULT_LIVE_OPTIONS, detectEveryMs: 42 });
    answer({ enabled: true, options: { ...DEFAULT_LIVE_OPTIONS, detectEveryMs: 9 } });
    await settle();
    expect(live.enabled()).toBe(false);
    expect(live.options().detectEveryMs).toBe(42);
  });

  it('reports a failed subscription while restoring', async () => {
    const backend = new MockBackend({ latency: false, dataset: tinyDataset, liveMs: 100_000 });
    await backend.setLive(true);
    vi.spyOn(backend, 'onWorkspaceChanged').mockRejectedValueOnce('no events');
    const { live } = await setup(backend);
    await vi.waitFor(() => expect(live.error()).toBe('no events'));
    expect(live.enabled()).toBe(false);
    await backend.setLive(false);
  });

  it('coalesces changes that arrive during a refresh into one more refresh', async () => {
    const { backend, live } = await setup(new MockBackend({ latency: false, dataset: tinyDataset, liveMs: 100000 }));
    let release!: () => void;
    const real = backend.workspaceSummary.bind(backend);
    const summary = vi.spyOn(backend, 'workspaceSummary').mockImplementationOnce(async () => {
      await new Promise<void>((r) => (release = r));
      return real();
    });
    let notify!: (report?: null) => void;
    vi.spyOn(backend, 'onWorkspaceChanged').mockImplementation(async (h) => ((notify = () => h(null)), () => undefined));
    await live.set(true);
    notify();
    notify();
    notify();
    await settle();
    expect(summary).toHaveBeenCalledTimes(1);
    release();
    await vi.waitFor(() => expect(summary).toHaveBeenCalledTimes(2));
    await live.set(false);
  });

  it('reports a refresh that fails and keeps going', async () => {
    const { backend, live } = await setup();
    let notify!: (report?: null) => void;
    vi.spyOn(backend, 'onWorkspaceChanged').mockImplementation(async (h) => ((notify = () => h(null)), () => undefined));
    await live.set(true);
    vi.spyOn(backend, 'workspaceSummary').mockRejectedValueOnce('summary failed');
    notify();
    await vi.waitFor(() => expect(live.error()).toBe('summary failed'));
    expect(live.enabled()).toBe(true);
    expect(testLogger().entries.some((e) => e.level === 'warn' && e.message === 'Live refresh failed')).toBe(true);
    notify();
    await vi.waitFor(() => expect(TestBed.inject(WorkspaceStore).summary()).not.toBeNull());
    await live.set(false);
  });

  it('refreshes through WorkspaceStore.refreshSummary for events with or without a payload', async () => {
    const { backend, store, live } = await setup(new MockBackend({ latency: false, dataset: tinyDataset, liveMs: 100000 }));
    let notify!: (payload?: unknown) => void;
    vi.spyOn(backend, 'onWorkspaceChanged').mockImplementation(async (h) => ((notify = h as (payload?: unknown) => void), () => undefined));
    const refresh = vi.spyOn(store, 'refreshSummary');
    const init = vi.spyOn(store, 'init');
    await live.set(true);
    notify({ appended: 3, sourcesChanged: 1, rotated: false, truncated: false });
    await vi.waitFor(() => expect(refresh).toHaveBeenCalledTimes(1));
    await settle();
    notify();
    await vi.waitFor(() => expect(refresh).toHaveBeenCalledTimes(2));
    notify({ appended: 0, sourcesChanged: 1, rotated: true, truncated: true });
    await vi.waitFor(() => expect(refresh).toHaveBeenCalledTimes(3));
    expect(init).not.toHaveBeenCalled();
    expect(live.isLiveRefresh()).toBe(true);
    await live.set(false);
  });

  it('is available when there are files to follow, and while on', async () => {
    const { store, live } = await setup();
    expect(live.available()).toBe(true);
    await live.set(true);
    await store.clearWorkspace();
    expect(live.available()).toBe(true);
    await live.set(false);
    expect(live.available()).toBe(false);
  });

  it('start() turns live tail on for a feature, or rejects with the reason', async () => {
    const { backend, live } = await setup();
    vi.spyOn(backend, 'setLive').mockRejectedValueOnce('too many files');
    await expect(live.start()).rejects.toThrow('too many files');
    expect(live.enabled()).toBe(false);
    await live.start();
    expect(live.enabled()).toBe(true);
    await live.set(false);
    vi.spyOn(live, 'set').mockResolvedValueOnce();
    await expect(live.start()).rejects.toThrow('Live tail could not start');
  });

  it('stays off and says why when the watcher cannot start', async () => {
    const { backend, live } = await setup();
    vi.spyOn(backend, 'setLive').mockRejectedValueOnce('no permission');
    await live.toggle();
    expect(live.enabled()).toBe(false);
    expect(live.error()).toBe('no permission');
    await live.toggle();
    expect(live.enabled()).toBe(true);
    expect(live.error()).toBeNull();
    await live.set(false);
  });

  it('asks the engine to watch files opened while live is on', async () => {
    const { backend, store, live } = await setup();
    await live.set(true);
    const setLive = vi.spyOn(backend, 'setLive');
    await store.openPaths(['/more/new.log']);
    await settle();
    expect(setLive).toHaveBeenCalledWith(true, DEFAULT_LIVE_OPTIONS);
    const calls = setLive.mock.calls.length;
    store.applySummary(store.summary()!);
    await settle();
    expect(setLive.mock.calls.length).toBe(calls);
    vi.spyOn(backend, 'setLive').mockRejectedValueOnce('watch failed');
    await store.removeSource(at(store.summary()!.sources, 0).id);
    await settle();
    expect(live.enabled()).toBe(false);
    expect(live.error()).toBe('watch failed');
    void tinyBackend;
  });

  it('routes reports to a connected sink instead of refreshing, until disconnected', async () => {
    const { backend, store, live } = await setup(new MockBackend({ latency: false, dataset: tinyDataset, liveMs: 100000 }));
    let notify!: (report: null) => void;
    vi.spyOn(backend, 'onWorkspaceChanged').mockImplementation(async (h) => ((notify = h), () => undefined));
    const refresh = vi.spyOn(store, 'refreshSummary');
    const sink = vi.fn();
    const disconnect = live.connect(sink);
    await live.set(true);
    notify(null);
    expect(sink).toHaveBeenCalledWith(null);
    expect(refresh).not.toHaveBeenCalled();
    live.connect(vi.fn())();
    disconnect();
    disconnect();
    notify(null);
    await vi.waitFor(() => expect(refresh).toHaveBeenCalledTimes(1));
    expect(sink).toHaveBeenCalledTimes(1);
    await live.set(false);
  });

  it('applies new options at once while on, and only remembers them while off', async () => {
    const { backend, live } = await setup();
    const setLive = vi.spyOn(backend, 'setLive');
    const polling = { ...DEFAULT_LIVE_OPTIONS, watch: 'polling' as const };
    await live.setOptions(polling);
    expect(setLive).not.toHaveBeenCalled();
    expect(live.options()).toEqual(polling);
    await live.set(true);
    expect(setLive).toHaveBeenLastCalledWith(true, polling);
    await live.setOptions({ ...polling, pollIntervalMs: 500 });
    expect(setLive).toHaveBeenLastCalledWith(true, { ...polling, pollIntervalMs: 500 });
    const calls = setLive.mock.calls.length;
    await live.setOptions({ ...polling, pollIntervalMs: 500 });
    expect(setLive.mock.calls.length).toBe(calls);
    setLive.mockRejectedValueOnce('bad options');
    await live.setOptions(DEFAULT_LIVE_OPTIONS);
    expect(live.error()).toBe('bad options');
    expect(live.enabled()).toBe(true);
    await live.set(false);
  });

  it('opens files for live tail, from their end or with their history, and turns live on', async () => {
    const { backend, store, live } = await setup();
    const openLive = vi.spyOn(backend, 'openPathsLive');
    const open = vi.spyOn(backend, 'openPaths');
    await live.openForLive([], true);
    expect(openLive).not.toHaveBeenCalled();
    await live.openForLive(['/var/log/new.log'], true);
    expect(openLive).toHaveBeenCalledWith(['/var/log/new.log'], true);
    expect(store.summary()!.sources.some((s) => s.path === '/var/log/new.log' && s.records === 0)).toBe(true);
    expect(live.enabled()).toBe(true);
    await live.set(false);
    await live.openForLive(['/var/log/other.log'], false);
    expect(open).toHaveBeenCalledWith(['/var/log/other.log']);
    expect(live.enabled()).toBe(true);
    await live.set(false);
    openLive.mockRejectedValueOnce('denied');
    await live.openForLive(['/x.log'], true);
    expect(store.error()).toBe('denied');
    expect(live.enabled()).toBe(false);
  });
});
