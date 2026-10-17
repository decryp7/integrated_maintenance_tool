import { TestBed } from '@angular/core/testing';
import { provideTinyBackend, settle, tinyDataset } from '../../testing/helpers';
import { MockBackend } from '../core/mock/mock-backend';
import { WorkspaceStore } from '../core/workspace.store';
import { TimelineZoom } from '../timeline/zoom';
import { LiveTail } from '../workspace/live-tail';
import { clampMinutes, CUSTOM_MINUTES, LiveFollow, sanitizeFollow, windowMsOf } from './live-follow.store';

describe('LiveFollow', () => {
  async function setup() {
    const backend = new MockBackend({ latency: false, dataset: tinyDataset, liveMs: 100_000 });
    TestBed.configureTestingModule({ providers: [provideTinyBackend(backend)] });
    const store = TestBed.inject(WorkspaceStore);
    const live = TestBed.inject(LiveTail);
    const follow = TestBed.inject(LiveFollow);
    const zoom = TestBed.inject(TimelineZoom);
    await settle();
    return { backend, store, live, follow, zoom };
  }

  it('follows by default with no window ("all"): turning live on leaves the query untouched', async () => {
    const { store, live, follow } = await setup();
    const before = store.query();
    await live.set(true);
    await settle();
    expect(follow.active()).toBe(true);
    expect(follow.holding()).toBe(false);
    expect(store.query()).toBe(before);
    expect('window' in store.query()).toBe(false);
    await live.set(false);
  });

  it('sends a sliding window for the engine to resolve, and drops it when live stops', async () => {
    const { store, live, follow } = await setup();
    follow.setPreset('5m');
    await settle();
    expect(store.query().window).toBeUndefined();
    await live.set(true);
    await settle();
    expect(store.query().window).toEqual({ kind: 'live', lastMs: 300_000 });
    expect(store.query().from).toBeNull();
    follow.setCustomMinutes(2);
    await settle();
    expect(follow.preset()).toBe('custom');
    expect(store.query().window).toEqual({ kind: 'live', lastMs: 120_000 });
    follow.setPreset('all');
    await settle();
    expect(store.query().window).toBeUndefined();
    follow.setPreset('1m');
    await settle();
    // Clearing the filters keeps following.
    store.clearQuery();
    await settle();
    expect(store.query().window).toEqual({ kind: 'live', lastMs: 60_000 });
    await live.set(false);
    await settle();
    expect(store.query().window).toBeUndefined();
  });

  it('pauses on a manual zoom and resumes on request', async () => {
    const { store, live, follow, zoom } = await setup();
    follow.setPreset('15m');
    await live.set(true);
    await settle();
    zoom.zoomIn();
    await settle();
    expect(follow.paused()).toBe(true);
    expect(follow.active()).toBe(false);
    expect(follow.holding()).toBe(true);
    expect(store.query().window).toBeUndefined();
    expect(store.query().from).not.toBeNull();
    const nonce = follow.resumeNonce();
    follow.resume();
    await settle();
    expect(follow.paused()).toBe(false);
    expect(follow.resumeNonce()).toBe(nonce + 1);
    expect(store.query().from).toBeNull();
    expect(store.query().to).toBeNull();
    expect(store.query().window).toEqual({ kind: 'live', lastMs: 900_000 });
    // A range from anywhere else (a bookmark, an alert's evidence) pauses too.
    store.setRange(1, 2);
    await settle();
    expect(follow.paused()).toBe(true);
    follow.pause();
    expect(follow.paused()).toBe(true);
    await live.set(false);
  });

  it('turning Follow off holds; on again clears a fixed range', async () => {
    const { store, live, follow } = await setup();
    await live.set(true);
    follow.setFollow(false);
    await settle();
    expect(follow.holding()).toBe(true);
    store.setRange(5, 10);
    await settle();
    expect(follow.paused()).toBe(false);
    follow.setFollow(true);
    await settle();
    expect(store.query().from).toBeNull();
    expect(follow.active()).toBe(true);
    await live.set(false);
    expect(follow.holding()).toBe(false);
  });

  it('restores saved settings defensively and resets to the defaults', async () => {
    const { follow } = await setup();
    follow.restore({ follow: false, preset: '60m', customMinutes: 99999999 });
    expect([follow.follow(), follow.preset(), follow.customMinutes()]).toEqual([false, '60m', CUSTOM_MINUTES.max]);
    follow.restore('nonsense');
    expect([follow.follow(), follow.preset(), follow.customMinutes()]).toEqual([true, 'all', 30]);
    follow.restore({ preset: 'bogus', follow: 'yes', customMinutes: 'x' });
    expect(follow.preset()).toBe('all');
    follow.setPreset('5m');
    follow.reset();
    expect(follow.preset()).toBe('all');
  });

  it('computes window lengths and clamps custom minutes', () => {
    expect(windowMsOf('1m', 0)).toBe(60_000);
    expect(windowMsOf('60m', 0)).toBe(3_600_000);
    expect(windowMsOf('all', 5)).toBeNull();
    expect(windowMsOf('custom', 0.5)).toBe(60_000);
    expect(windowMsOf('custom', 10)).toBe(600_000);
    expect(clampMinutes(Number.NaN)).toBe(30);
    expect(sanitizeFollow(null)).toEqual({ follow: true, preset: 'all', customMinutes: 30 });
  });
});
