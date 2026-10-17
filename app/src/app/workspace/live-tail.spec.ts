import { TestBed } from '@angular/core/testing';
import { provideTinyBackend, settle, testLogger, tinyBackend, tinyDataset } from '../../testing/helpers';
import { ImtBackend } from '../core/backend';
import { MockBackend } from '../core/mock/mock-backend';
import { WorkspaceStore } from '../core/workspace.store';
import { LiveTail } from './live-tail';

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
    expect(setLive).toHaveBeenLastCalledWith(true);
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
    const row = (await TestBed.inject(ImtBackend).queryPage({ query: store.effectiveQuery(), offset: 1, limit: 1, descending: false })).rows[0];
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

  it('coalesces changes that arrive during a refresh into one more refresh', async () => {
    const { backend, live } = await setup(new MockBackend({ latency: false, dataset: tinyDataset, liveMs: 100000 }));
    let release!: () => void;
    const real = backend.workspaceSummary.bind(backend);
    const summary = vi.spyOn(backend, 'workspaceSummary').mockImplementationOnce(async () => {
      await new Promise<void>((r) => (release = r));
      return real();
    });
    let notify!: () => void;
    vi.spyOn(backend, 'onWorkspaceChanged').mockImplementation(async (h) => ((notify = h), () => undefined));
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
    let notify!: () => void;
    vi.spyOn(backend, 'onWorkspaceChanged').mockImplementation(async (h) => ((notify = h), () => undefined));
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
    expect(setLive).toHaveBeenCalledWith(true);
    const calls = setLive.mock.calls.length;
    store.applySummary(store.summary()!);
    await settle();
    expect(setLive.mock.calls.length).toBe(calls);
    vi.spyOn(backend, 'setLive').mockRejectedValueOnce('watch failed');
    await store.removeSource(store.summary()!.sources[0].id);
    await settle();
    expect(live.enabled()).toBe(false);
    expect(live.error()).toBe('watch failed');
    void tinyBackend;
  });
});
