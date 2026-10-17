import { TestBed } from '@angular/core/testing';
import { describe, expect, it, vi } from 'vitest';
import { Evidence, EvidenceView, FrameDto, HotspotRow } from '../core/api.types';
import { ProfileViewState } from './profile-view-state';
import { ActiveProfile, PROFILING_VIEW_IDS as V, ProfilingNavigator } from './profiling-navigator';
import { CapturesStore } from './captures.store';
import { ProfilingBackend } from './profiling-backend';
import { openDemoCaptures, setupProfiling } from './testing/profiling-test-support';

const frame: FrameDto = { id: 4, name: 'Shop.Orders.Pricing.PriceCalculator.Calculate', module: 'Shop.Orders', file: null, line: null, kind: 'managed', isFramework: false };

const row = (self: number, f: FrameDto | null = frame): HotspotRow => ({
  key: 'k', name: 'n', module: null, frame: f, thread: null, self: { timeNs: self }, total: { timeNs: 10 }, selfPct: 0, totalPct: 0, avgPerCallNs: null, maxPerCallNs: null,
});

const evidence = (view: EvidenceView, patch: Partial<Evidence> = {}): Evidence => ({
  view, profileId: 2, baselineId: 1, frame, path: [0, 4], metric: 'timeNs', range: { fromNs: 1, toNs: 9 }, threads: [1], eventKind: null, ...patch,
});

describe('ActiveProfile', () => {
  it('is empty until a capture is open', () => {
    setupProfiling();
    const active = TestBed.inject(ActiveProfile);
    expect(active.profile()).toBeNull();
    expect(active.heap()).toBeNull();
    expect(active.metric()).toBeNull();
  });

  it('derives the active profile and heap from the captures and the view state', async () => {
    setupProfiling();
    await openDemoCaptures();
    const active = TestBed.inject(ActiveProfile);
    const state = TestBed.inject(ProfileViewState);
    const captures = TestBed.inject(CapturesStore);
    expect(active.profile()?.id).toBe(captures.profiles()[0].id);
    expect(active.heap()?.id).toBe(captures.heaps()[0].id);
    state.setProfile(captures.profiles()[1].id);
    state.setHeap(captures.heaps()[2].id);
    expect(active.profile()?.name).toContain('regression');
    expect(active.heap()?.id).toBe(captures.heaps()[2].id);
    // A heap id never resolves as the active profile and vice versa.
    state.setProfile(captures.heaps()[0].id);
    state.setHeap(captures.profiles()[0].id);
    expect(active.profile()).toBeNull();
    expect(active.heap()).toBeNull();
  });

  it('uses the chosen metric when the profile has it, else the profile default', async () => {
    setupProfiling();
    await openDemoCaptures();
    const active = TestBed.inject(ActiveProfile);
    const state = TestBed.inject(ProfileViewState);
    const [orders, regression] = TestBed.inject(CapturesStore).profiles();
    expect(active.metric()).toBe(orders.defaultMetric);
    state.setMetric('allocBytes');
    expect(active.metric()).toBe('allocBytes');
    state.setProfile(regression.id);
    state.setMetric('allocBytes');
    expect(regression.metrics.some((m) => m.id === 'allocBytes')).toBe(false);
    expect(active.metric()).toBe(regression.defaultMetric);
    state.setMetric('samples');
    expect(active.metric()).toBe('samples');
  });
});

describe('ProfilingNavigator', () => {
  function setup() {
    const doubles = setupProfiling();
    return { nav: TestBed.inject(ProfilingNavigator), state: TestBed.inject(ProfileViewState), ctx: doubles.context };
  }
  const shown = (ctx: ReturnType<typeof setup>['ctx']) => ctx.calls.filter((c) => c.fn === 'showView').map((c) => c.args[0]);

  it('has unique view ids', () => {
    const ids = Object.values(V);
    expect(new Set(ids).size).toBe(ids.length);
  });

  it('show forwards to the command context', () => {
    const { nav, ctx } = setup();
    nav.show('x');
    expect(shown(ctx)).toEqual(['x']);
  });

  it.each<[EvidenceView, string]>([
    ['hotspots', V.hotspots],
    ['callTree', V.tree],
    ['flame', V.flame],
    ['butterfly', V.butterfly],
    ['timeline', V.threads],
    ['diff', V.compare],
  ])('openEvidence %s shows %s and applies the evidence', (view, id) => {
    const { nav, state, ctx } = setup();
    nav.openEvidence(evidence(view));
    expect(shown(ctx)).toEqual([id]);
    expect(state.profileId()).toBe(2);
    expect(state.baselineId()).toBe(1);
    expect(state.range()).toEqual({ fromNs: 1, toNs: 9 });
    expect(state.threads()).toEqual([1]);
    expect(state.selectedFrame()).toEqual(frame);
  });

  it('openOnRange selects the profile and range and shows the hotspots', () => {
    const { nav, state, ctx } = setup();
    nav.openOnRange(3, { fromNs: 5, toNs: 2 });
    expect(state.profileId()).toBe(3);
    expect(state.range()).toEqual({ fromNs: 2, toNs: 5 });
    expect(shown(ctx)).toEqual([V.hotspots]);
    nav.openOnRange(3, null);
    expect(state.range()).toBeNull();
  });

  it('openHeap activates the snapshot and shows the memory view', () => {
    const { nav, state, ctx } = setup();
    nav.openHeap(8);
    expect(state.heapId()).toBe(8);
    expect(shown(ctx)).toEqual([V.heap]);
  });

  it('butterfly selects the frame and shows callers and callees', () => {
    const { nav, state, ctx } = setup();
    nav.butterfly(frame);
    expect(state.selectedFrame()).toEqual(frame);
    expect(shown(ctx)).toEqual([V.butterfly]);
  });

  it('showInTree reveals the exact node: the hottest top-down path the engine found', async () => {
    const { nav, state, ctx } = setup();
    await openDemoCaptures();
    const framePaths = vi.spyOn(TestBed.inject(ProfilingBackend), 'framePaths');
    await nav.showInTree(row(5), 'timeNs');
    expect(framePaths).toHaveBeenCalledWith({ profileId: state.profileId(), view: state.view(), frame: frame.id, metric: 'timeNs', limit: 1 });
    const best = (await framePaths.mock.results[0].value).paths[0].path;
    expect(best.at(-1)).toBe(frame.id);
    expect(state.selectedFrame()).toEqual(frame);
    expect(state.reveal()).toEqual({ path: best, direction: 'topDown', nonce: 1 });
    expect(shown(ctx)).toEqual([V.tree]);
  });

  it('showInTree still opens the tree when there is no path (no profile, not in the view, or a failure)', async () => {
    const { nav, state, ctx } = setup();
    await nav.showInTree(row(5), 'timeNs');
    expect(state.reveal()).toBeNull();
    await openDemoCaptures();
    vi.spyOn(TestBed.inject(ProfilingBackend), 'framePaths').mockRejectedValueOnce(new Error('down')).mockResolvedValueOnce({ paths: [], totalPaths: 0, metric: 'timeNs' });
    await nav.showInTree(row(5), 'timeNs');
    await nav.showInTree(row(5), 'timeNs');
    expect(state.reveal()).toBeNull();
    expect(shown(ctx)).toEqual([V.tree, V.tree, V.tree]);
  });

  it('showInTree does nothing for a row without a frame (thread grouping)', async () => {
    const { nav, state, ctx } = setup();
    await nav.showInTree(row(5, null), 'timeNs');
    expect(state.selectedFrame()).toBeNull();
    expect(ctx.calls).toEqual([]);
  });

  it('focusInFlame zooms the flame graph to the hottest path, or highlights when there is none', async () => {
    const { nav, state, ctx } = setup();
    await openDemoCaptures();
    state.setFlameSearch('old');
    await nav.focusInFlame(frame, 'timeNs');
    expect(state.flameFocus().at(-1)).toBe(frame.id);
    expect(state.flameSearch()).toBe('');
    expect(state.selectedFrame()).toEqual(frame);
    expect(shown(ctx)).toEqual([V.flame]);
    vi.spyOn(TestBed.inject(ProfilingBackend), 'framePaths').mockResolvedValueOnce({ paths: [], totalPaths: 0, metric: 'timeNs' });
    await nav.focusInFlame(frame, null);
    expect(state.flameFocus()).toEqual([]);
    expect(state.flameSearch()).toBe(frame.name);
  });

  it('showInFlame searches for the function and clears the focus', () => {
    const { nav, state, ctx } = setup();
    state.setFlameFocus([1, 2]);
    nav.showInFlame(frame);
    expect(state.selectedFrame()).toEqual(frame);
    expect(state.flameFocus()).toEqual([]);
    expect(state.flameSearch()).toBe(frame.name);
    expect(shown(ctx)).toEqual([V.flame]);
  });
});
