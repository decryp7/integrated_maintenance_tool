import { TestBed } from '@angular/core/testing';
import { provideTinyBackend, settle, tinyDataset } from '../../testing/helpers';
import { report } from '../../testing/live-helpers';
import { TailReport } from '../core/api.types';
import { MockBackend } from '../core/mock/mock-backend';
import { WorkspaceStore } from '../core/workspace.store';
import { LiveTail } from '../workspace/live-tail';
import { FACET_INTERVAL_MS, LiveFeed } from './live-feed';
import { LiveFollow } from './live-follow.store';
import { TailStatusStore } from './tail-status.store';

describe('LiveFeed', () => {
  async function setup() {
    const backend = new MockBackend({ latency: false, dataset: tinyDataset, liveMs: 100_000 });
    TestBed.configureTestingModule({ providers: [provideTinyBackend(backend)] });
    const store = TestBed.inject(WorkspaceStore);
    const live = TestBed.inject(LiveTail);
    const feed = TestBed.inject(LiveFeed);
    const follow = TestBed.inject(LiveFollow);
    const status = TestBed.inject(TailStatusStore);
    let notify!: (r: TailReport | null) => void;
    vi.spyOn(backend, 'onWorkspaceChanged').mockImplementation(async (h) => ((notify = h), () => undefined));
    await settle();
    const summary = vi.spyOn(backend, 'workspaceSummary');
    const tailStatus = vi.spyOn(backend, 'tailStatus');
    await live.set(true);
    await settle();
    return { backend, store, live, feed, follow, status, summary, tailStatus, notify: (r: TailReport | null) => notify(r) };
  }

  afterEach(() => vi.useRealTimers());

  it('applies per-file state changes of a report to the tail status at once', async () => {
    const { notify, status, live, tailStatus } = await setup();
    await vi.waitFor(() => expect(status.status()).not.toBeNull());
    const id = status.status()!.files[0].sourceId;
    // The polled status (the fallback) does not answer: the badge comes from the report alone.
    tailStatus.mockImplementation(() => new Promise(() => undefined));
    const apply = vi.spyOn(status, 'applyFileStates');
    notify(report({ fileStates: [{ sourceId: id, state: { kind: 'error', message: 'locked' } }] }));
    await vi.waitFor(() => expect(status.notable().get(id)).toEqual({ kind: 'error', message: 'locked' }));
    expect(apply).toHaveBeenCalledWith([{ sourceId: id, state: { kind: 'error', message: 'locked' } }]);
    await live.set(false);
  });

  it('coalesces a burst of change events into a few refreshes (at most ~4 per second)', async () => {
    const { feed, summary, notify, live, store } = await setup();
    const version = store.dataVersion();
    summary.mockClear();
    const start = Date.now();
    // 300 events within about a second.
    for (let i = 0; i < 300; i++) {
      notify(report({ appended: 1, newestTs: 1_000 + i }));
      if (i % 50 === 49) await new Promise((r) => setTimeout(r, 150));
    }
    await vi.waitFor(() => expect(feed.newestTs()).toBe(1_299), { timeout: 3000 });
    await new Promise((r) => setTimeout(r, 300));
    const seconds = Math.max(1, (Date.now() - start) / 1000);
    expect(feed.refreshes()).toBeLessThanOrEqual(Math.ceil(seconds * 4) + 1);
    expect(feed.refreshes()).toBeGreaterThanOrEqual(1);
    expect(summary.mock.calls.length).toBe(feed.refreshes());
    expect(live.isLiveRefresh()).toBe(true);
    expect(store.dataVersion()).toBeGreaterThan(version);
    await live.set(false);
  });

  it('counts new events instead of reloading the grid while not following, and reveals them on request', async () => {
    const { feed, follow, notify, live } = await setup();
    follow.setFollow(false);
    await settle();
    notify(report({ appended: 3 }));
    await vi.waitFor(() => expect(feed.newEvents()).toBe(3));
    expect(feed.holdGrid()).toBe(true);
    notify(report({ appended: 2 }));
    await vi.waitFor(() => expect(feed.newEvents()).toBe(5));
    // Ids changed: the grid must reload even while holding.
    notify(report({ appended: 1, idsChangedFrom: 0 }));
    await vi.waitFor(() => expect(feed.holdGrid()).toBe(false));
    expect(feed.newEvents()).toBe(5);
    const nonce = feed.revealNonce();
    feed.showNewEvents();
    expect(feed.newEvents()).toBe(0);
    expect(feed.revealNonce()).toBe(nonce + 1);
    // Resume / Follow on reveals what was held back too.
    notify(report({ appended: 4 }));
    await vi.waitFor(() => expect(feed.newEvents()).toBe(4));
    follow.setFollow(true);
    await settle();
    expect(feed.newEvents()).toBe(0);
    expect(feed.revealNonce()).toBe(nonce + 2);
    await live.set(false);
  });

  it('refreshes nothing but the status for an empty report, and everything for an event without payload', async () => {
    const { feed, summary, tailStatus, notify, live } = await setup();
    summary.mockClear();
    tailStatus.mockClear();
    notify(report());
    await vi.waitFor(() => expect(feed.refreshes()).toBe(1));
    expect(summary).not.toHaveBeenCalled();
    await new Promise((r) => setTimeout(r, 300));
    notify(null);
    await vi.waitFor(() => expect(summary).toHaveBeenCalledTimes(1));
    expect(tailStatus).toHaveBeenCalled();
    await live.set(false);
  });

  it('throttles facet refreshes and refreshes the status at once when files rotate', async () => {
    const { feed, tailStatus, notify, live } = await setup();
    notify(report({ appended: 1 }));
    await vi.waitFor(() => expect(feed.facetVersion()).toBe(1));
    tailStatus.mockClear();
    await new Promise((r) => setTimeout(r, 300));
    notify(report({ appended: 1, rotated: [{ sourceId: 0, from: 'a', to: 'a.1' }], added: [9] }));
    await vi.waitFor(() => expect(tailStatus).toHaveBeenCalled());
    expect(feed.facetVersion()).toBe(1);
    await vi.waitFor(() => expect(feed.facetVersion()).toBe(2), { timeout: FACET_INTERVAL_MS + 1000 });
    await live.set(false);
  });

  it('stops and forgets pending work when live tail turns off', async () => {
    const { feed, follow, notify, live, status } = await setup();
    follow.setFollow(false);
    notify(report({ appended: 2 }));
    await vi.waitFor(() => expect(feed.newEvents()).toBe(2));
    const refresh = vi.spyOn(status, 'refresh');
    await live.set(false);
    await settle();
    expect(feed.newEvents()).toBe(0);
    expect(feed.holdGrid()).toBe(false);
    expect(refresh).toHaveBeenCalled();
    const before = feed.refreshes();
    notify(report({ appended: 2 }));
    await new Promise((r) => setTimeout(r, 300));
    expect(feed.refreshes()).toBe(before);
  });
});
