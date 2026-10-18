import { TestBed } from '@angular/core/testing';
import { describe, expect, it, vi } from 'vitest';
import { FrameDto } from '../core/api.types';
import { MockProfilingBackend } from '../core/mock/mock-profiling';
import { MOCK_CAPTURE_PATHS } from '../core/mock/mock-profiling-data';
import { CapturePicker, CapturesStore, FrameNames } from './captures.store';
import { ProfileViewState } from './profile-view-state';
import { ProfilingBackend } from './profiling-backend';
import { openDemoCaptures, setupProfiling } from './testing/profiling-test-support';
import { at } from '../../testing/must';

const ORDERS = '/demo/profiles/orders.speedscope.json';
const REGRESSION = '/demo/profiles/orders-regression.speedscope.json';
const HEAP1 = '/demo/profiles/heap-1.heapsnapshot';
const BAD = '/demo/notes.txt';

function failing<K extends keyof MockProfilingBackend>(patch: Partial<Record<K, unknown>>): MockProfilingBackend {
  return Object.assign(new MockProfilingBackend(), patch);
}

describe('CapturesStore', () => {
  const store = () => TestBed.inject(CapturesStore);
  const state = () => TestBed.inject(ProfileViewState);

  it('refresh lists the open captures and makes the first of each kind active', async () => {
    setupProfiling({ options: { preopen: [HEAP1, ORDERS] } });
    await store().refresh();
    expect(store().captures()).toHaveLength(2);
    expect(store().profiles()).toHaveLength(1);
    expect(store().heaps()).toHaveLength(1);
    expect(state().profileId()).toBe(store().profiles()[0]?.id);
    expect(state().heapId()).toBe(store().heaps()[0]?.id);
    expect(store().loading()).toBe(false);
    expect(store().error()).toBeNull();
  });

  it('refresh reports a failure and clears loading', async () => {
    setupProfiling({ backend: failing({ list: vi.fn().mockRejectedValue('engine down') }) });
    await store().refresh();
    expect(store().error()).toContain('engine down');
    expect(store().loading()).toBe(false);
  });

  it('openPaths opens everything, activates the first profile and heap and bumps capturesChanged', async () => {
    setupProfiling();
    const opened = await store().openPaths(MOCK_CAPTURE_PATHS);
    expect(opened).toHaveLength(5);
    expect(store().captures()).toHaveLength(5);
    expect(state().profileId()).toBe(opened[0]?.id);
    expect(state().heapId()).toBe(opened[2]?.id);
    expect(store().notice()).toBe('Opened 5 captures.');
    expect(store().error()).toBeNull();
    expect(state().capturesVersion()).toBe(1);
    expect(store().byId(at(opened, 1).id)?.name).toContain('regression');
  });

  it('openPaths with no paths does nothing; a single open says "capture"', async () => {
    setupProfiling();
    expect(await store().openPaths([])).toEqual([]);
    expect(store().notice()).toBeNull();
    await store().openPaths([ORDERS]);
    expect(store().notice()).toBe('Opened 1 capture.');
  });

  it('openPaths collects per-file failures while the rest open', async () => {
    setupProfiling();
    const opened = await store().openPaths([BAD, ORDERS, '/demo/other.bin']);
    expect(opened).toHaveLength(1);
    expect(store().failedPaths()).toEqual([BAD, '/demo/other.bin']);
    expect(store().error()).toContain('notes.txt');
    expect(store().error()).toContain('other.bin');
    expect(store().error()?.split('\n')).toHaveLength(2);
    expect(store().notice()).toBe('Opened 1 capture.');
    expect(store().loading()).toBe(false);
  });

  it('openPaths with nothing opened has no notice and does not bump capturesChanged', async () => {
    setupProfiling();
    await store().openPaths([BAD]);
    expect(store().notice()).toBeNull();
    expect(state().capturesVersion()).toBe(0);
  });

  it('openPaths keeps the opened captures when the follow-up list fails', async () => {
    const backend = new MockProfilingBackend();
    setupProfiling({ backend: Object.assign(backend, { list: vi.fn().mockRejectedValue('list broke') }) });
    const opened = await store().openPaths([ORDERS]);
    expect(opened).toHaveLength(1);
    expect(store().error()).toContain('list broke');
    expect(state().profileId()).toBe(opened[0]?.id);
  });

  it('pickAndOpen opens what the picker returns', async () => {
    const d = setupProfiling();
    d.picker.paths = [ORDERS];
    expect(await store().pickAndOpen()).toHaveLength(1);
    expect(store().profiles()).toHaveLength(1);
  });

  it('pickAndOpen reports a picker failure', async () => {
    setupProfiling();
    vi.spyOn(TestBed.inject(CapturePicker), 'pick').mockRejectedValue(new Error('dialog crashed'));
    expect(await store().pickAndOpen()).toEqual([]);
    expect(store().error()).toContain('dialog crashed');
  });

  it('openDropped claims and opens what the engine sniffs as captures, and leaves the rest', async () => {
    setupProfiling();
    const open = vi.spyOn(TestBed.inject(ProfilingBackend), 'open');
    const { claimed, opened } = await store().openDropped([BAD, ORDERS, HEAP1]);
    expect(claimed).toEqual([ORDERS, HEAP1]);
    expect(opened.map((c) => c.capture)).toEqual(['profile', 'heap']);
    expect(open.mock.calls.map((c) => c[0]), 'files that are not captures are never opened here').toEqual([ORDERS, HEAP1]);
    expect(store().error()).toBeNull();
    expect(store().failedPaths()).toEqual([]);
    expect(state().capturesVersion()).toBe(1);
  });

  it('openDropped with nothing recognised opens nothing and changes nothing', async () => {
    setupProfiling();
    expect(await store().openDropped([BAD])).toEqual({ claimed: [], opened: [] });
    expect(await store().openDropped([])).toEqual({ claimed: [], opened: [] });
    expect(store().captures()).toEqual([]);
    expect(state().capturesVersion()).toBe(0);
  });

  it('restorePaths reports missing files in the notice, other failures as errors', async () => {
    setupProfiling();
    const opened = await store().restorePaths([ORDERS, '/gone/missing-1.nettrace', BAD]);
    expect(opened).toHaveLength(1);
    expect(store().notice()).toBe('Opened 1 capture. 1 capture was not found: missing-1.nettrace.');
    expect(store().error()).toContain('notes.txt');
    expect(store().failedPaths()).toEqual([BAD]);
    // An ordinary open treats a missing file as an error (with Retry).
    await store().openPaths(['/gone/missing-2.nettrace']);
    expect(store().error()).toContain('missing-2.nettrace');
  });

  it('closeAll closes every capture, even when one is already gone', async () => {
    setupProfiling();
    await store().openPaths([ORDERS, HEAP1]);
    const backend = TestBed.inject(ProfilingBackend);
    vi.spyOn(backend, 'close').mockRejectedValueOnce(new Error('gone'));
    const before = state().capturesVersion();
    await store().closeAll();
    expect(store().captures()).toEqual([]);
    expect(state().profileId()).toBeNull();
    expect(state().capturesVersion()).toBe(before + 1);
  });

  it('retry re-opens only the failed paths', async () => {
    setupProfiling();
    await store().openPaths([ORDERS, BAD]);
    expect(store().failedPaths()).toEqual([BAD]);
    const again = await store().retry();
    expect(again).toEqual([]);
    expect(store().failedPaths()).toEqual([BAD]);
    expect(store().profiles()).toHaveLength(1);
  });

  it('retry succeeds when the failure was transient', async () => {
    setupProfiling({ options: { fail: { open: 1 } } });
    await store().openPaths([ORDERS]);
    expect(store().failedPaths()).toEqual([ORDERS]);
    expect(store().captures()).toEqual([]);
    const opened = (await store().retry()) as unknown[];
    expect(opened).toHaveLength(1);
    expect(store().failedPaths()).toEqual([]);
    expect(store().error()).toBeNull();
  });

  it('retry without failed paths refreshes', async () => {
    setupProfiling({ options: { preopen: [ORDERS] } });
    await store().retry();
    expect(store().profiles()).toHaveLength(1);
  });

  it('close removes the capture, falls back the active ones and clears a closed baseline', async () => {
    setupProfiling();
    await openDemoCaptures();
    const profiles = store().profiles();
    const orders = at(profiles, 0);
    const regression = at(profiles, 1);
    const heaps = store().heaps();
    state().setBaseline(regression.id);
    const version = state().capturesVersion();
    await store().close(regression.id);
    expect(state().baselineId()).toBeNull();
    expect(state().profileId()).toBe(orders.id);
    await store().close(orders.id);
    expect(state().profileId()).toBeNull();
    await store().close(at(heaps, 0).id);
    expect(state().heapId()).toBe(heaps[1]?.id);
    expect(store().byId(at(heaps, 0).id)).toBeNull();
    expect(state().capturesVersion()).toBe(version + 3);
  });

  it('close keeps a baseline that is still open', async () => {
    setupProfiling();
    await openDemoCaptures();
    await store().openPaths([ORDERS]);
    const profiles = store().profiles();
    const orders = at(profiles, 0);
    const regression = at(profiles, 1);
    const third = at(profiles, 2);
    state().setProfile(orders.id);
    state().setBaseline(third.id);
    await store().close(regression.id);
    expect(state().baselineId()).toBe(third.id);
    expect(state().profileId()).toBe(orders.id);
  });

  it('close failure sets the error and keeps the capture', async () => {
    setupProfiling({ backend: failing({ close: vi.fn().mockRejectedValue('locked') }) });
    await store().openPaths([ORDERS]);
    await store().close(at(store().captures(), 0).id);
    expect(store().error()).toContain('locked');
    expect(store().captures()).toHaveLength(1);
  });

  it('dismiss clears error, notice and failed paths', async () => {
    setupProfiling();
    await store().openPaths([ORDERS, BAD]);
    store().dismiss();
    expect(store().error()).toBeNull();
    expect(store().notice()).toBeNull();
    expect(store().failedPaths()).toEqual([]);
  });

  it('byId resolves null and unknown ids to null', async () => {
    setupProfiling();
    await store().openPaths([REGRESSION]);
    expect(store().byId(null)).toBeNull();
    expect(store().byId(999)).toBeNull();
    expect(store().byId(at(store().captures(), 0).id)).not.toBeNull();
  });
});

describe('CapturePicker', () => {
  it('returns the demo captures in a browser', async () => {
    TestBed.configureTestingModule({});
    expect(await TestBed.inject(CapturePicker).pick()).toEqual(MOCK_CAPTURE_PATHS);
  });
});

describe('FrameNames', () => {
  const frame = (id: number, name: string): FrameDto => ({ id, name, module: null, file: null, line: null, kind: 'managed', isFramework: false });

  it('remembers frames per profile, ignores nulls and repeats, and bumps the version only for new names', () => {
    const names = TestBed.inject(FrameNames);
    expect(names.version()).toBe(0);
    names.remember(1, [frame(3, 'A.B'), null]);
    expect(names.version()).toBe(1);
    names.remember(1, [frame(3, 'Renamed')]);
    expect(names.version()).toBe(1);
    names.remember(2, [frame(3, 'Other')]);
    expect(names.version()).toBe(2);
    names.remember(1, []);
    expect(names.version()).toBe(2);
    expect(names.get(1, 3)?.name).toBe('A.B');
    expect(names.get(2, 3)?.name).toBe('Other');
    expect(names.get(1, 4)).toBeNull();
  });

  it('names fall back to the frame id', () => {
    const names = TestBed.inject(FrameNames);
    names.remember(1, [frame(3, 'A.B')]);
    expect(names.name(1, 3)).toBe('A.B');
    expect(names.name(1, 9)).toBe('frame #9');
  });
});
