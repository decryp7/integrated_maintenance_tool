import { inject } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { describe, expect, it, vi } from 'vitest';
import { MOCK_CAPTURE_PATHS } from '../core/mock/mock-profiling-data';
import { UI_STATE_CONTRIBUTIONS, UiStateRegistry } from '../lifecycle/ui-state';
import { CapturesStore } from './captures.store';
import { ProfileViewState } from './profile-view-state.store';
import { ProfilingBackend } from './profiling-backend';
import { parseProfilingUiState, PROFILING_STATE_KEY, PROFILING_STATE_VERSION, ProfilingPersistence, ProfilingUiState } from './profiling-persistence';
import { openDemoCaptures, provideProfilingTesting, setupProfiling } from './testing/profiling-test-support';
import { at } from '../../testing/must';

const [ORDERS, REGRESSION, HEAP1, HEAP2] = MOCK_CAPTURE_PATHS.map((p) => p) as [string, string, string, string];

function setup() {
  setupProfiling();
  return { persistence: TestBed.inject(ProfilingPersistence), captures: TestBed.inject(CapturesStore), state: TestBed.inject(ProfileViewState), backend: TestBed.inject(ProfilingBackend) };
}

const saved = (patch: Partial<ProfilingUiState> = {}): ProfilingUiState => ({
  version: PROFILING_STATE_VERSION,
  captures: [],
  profile: null,
  heap: null,
  baseline: null,
  justMyCode: false,
  frameworkPatterns: null,
  useFrameFlags: true,
  recursion: 'off',
  metric: null,
  ...patch,
});

describe('parseProfilingUiState', () => {
  it('is null for anything that is not an object', () => {
    for (const v of [null, undefined, 3, 'x', [1]]) expect(parseProfilingUiState(v)).toBeNull();
  });

  it('fills every missing field with its default (older files)', () => {
    expect(parseProfilingUiState({})).toEqual(saved());
  });

  it('replaces malformed fields with defaults and keeps the valid ones', () => {
    const s = parseProfilingUiState({
      version: 'x',
      captures: ['/a.nettrace', 7, '', '/a.nettrace', '/b.folded'],
      profile: 5,
      heap: '/h.heapsnapshot',
      baseline: '',
      justMyCode: 'yes',
      frameworkPatterns: ['System.*', 3],
      useFrameFlags: 0,
      recursion: 'sideways',
      metric: 'bogus',
    });
    expect(s).toEqual(saved({ captures: ['/a.nettrace', '/b.folded'], heap: '/h.heapsnapshot', frameworkPatterns: ['System.*'] }));
  });

  it('reads the known fields of a newer version and bounds the captures', () => {
    const many = Array.from({ length: 40 }, (_, i) => `/c${i}.folded`);
    const s = parseProfilingUiState({ version: 9, captures: many, recursion: 'all', metric: 'allocBytes', justMyCode: true, useFrameFlags: false, future: { x: 1 } })!;
    expect(s.version).toBe(9);
    expect(s.captures).toHaveLength(32);
    expect([s.recursion, s.metric, s.justMyCode, s.useFrameFlags]).toEqual(['all', 'allocBytes', true, false]);
  });
});

describe('ProfilingPersistence', () => {
  it('captures the open captures and the view settings by path', async () => {
    const { persistence, captures, state } = setup();
    expect(persistence.capture()).toEqual(saved({ frameworkPatterns: state.frameworkPatterns() }));
    await openDemoCaptures();
    const profiles = captures.profiles();
    const orders = at(profiles, 0);
    const regression = at(profiles, 1);
    state.setProfile(regression.id);
    state.setBaseline(orders.id);
    state.setHeap(at(captures.heaps(), 1).id);
    state.setJustMyCode(true);
    state.setRecursion('direct');
    state.setMetric('samples');
    expect(persistence.capture()).toEqual(
      saved({ captures: [...MOCK_CAPTURE_PATHS], profile: REGRESSION, heap: HEAP2, baseline: ORDERS, justMyCode: true, frameworkPatterns: state.frameworkPatterns(), recursion: 'direct', metric: 'samples' }),
    );
  });

  it('round-trips: a restored state reopens the captures and reapplies the settings', async () => {
    const { persistence, captures, state } = setup();
    await openDemoCaptures();
    state.setProfile(at(captures.profiles(), 1).id);
    state.setJustMyCode(true);
    state.setFrameworkPatterns(['Shop.*']);
    state.setUseFrameFlags(false);
    state.setRecursion('all');
    const before = JSON.parse(JSON.stringify(persistence.capture()));
    await persistence.restore(null);
    expect(captures.captures()).toEqual([]);
    expect(state.justMyCode()).toBe(false);
    await persistence.restore(parseProfilingUiState(before));
    expect(captures.captures().map((c) => c.path)).toEqual(MOCK_CAPTURE_PATHS);
    expect(persistence.capture()).toEqual(before);
    expect(captures.error()).toBeNull();
  });

  it('reports the state being restored until the restore finished, so a just-opened project stays clean', async () => {
    const { persistence, captures } = setup();
    await openDemoCaptures();
    const before = JSON.parse(JSON.stringify(persistence.capture()));
    const reset = persistence.restore(null);
    expect(persistence.capture().captures).toEqual([]);
    await reset;
    const reopening = persistence.restore(parseProfilingUiState(before));
    expect(persistence.capture(), 'while reopening').toEqual(before);
    expect(captures.captures()).toEqual([]);
    await reopening;
    expect(persistence.capture(), 'after reopening').toEqual(before);
  });

  it('keeps captures already open from the same file and closes the others', async () => {
    const { persistence, captures, backend } = setup();
    await captures.openPaths([ORDERS, HEAP1]);
    const keep = captures.captures().find((c) => c.path === ORDERS)!.id;
    const open = vi.spyOn(backend, 'open');
    await persistence.restore(saved({ captures: [ORDERS, REGRESSION], profile: ORDERS }));
    expect(open.mock.calls.map((c) => c[0])).toEqual([REGRESSION]);
    expect(captures.captures().map((c) => c.path)).toEqual([ORDERS, REGRESSION]);
    expect(captures.captures().find((c) => c.path === ORDERS)!.id).toBe(keep);
  });

  it('degrades when saved captures no longer exist: a notice, not an error', async () => {
    const { persistence, captures, state } = setup();
    await persistence.restore(saved({ captures: ['/gone/missing-orders.nettrace', HEAP1], profile: '/gone/missing-orders.nettrace', heap: HEAP1, baseline: '/gone/missing-orders.nettrace' }));
    expect(captures.error()).toBeNull();
    expect(captures.notice()).toContain('1 capture was not found: missing-orders.nettrace');
    expect(captures.heaps()).toHaveLength(1);
    expect(state.heapId()).toBe(captures.heaps()[0]?.id);
    expect(state.profileId()).toBeNull();
    expect(state.baselineId()).toBeNull();
  });

  it('restores through the UI state registry, and resets when the project has no profiling state', async () => {
    const { providers } = provideProfilingTesting();
    TestBed.configureTestingModule({ providers: [...providers, { provide: UI_STATE_CONTRIBUTIONS, multi: true, useFactory: () => [inject(ProfilingPersistence).contribution] }] });
    const registry = TestBed.inject(UiStateRegistry);
    const persistence = TestBed.inject(ProfilingPersistence);
    const captures = TestBed.inject(CapturesStore);
    registry.restore({ version: 1, [PROFILING_STATE_KEY]: { version: 1, captures: [ORDERS] } });
    await persistence.pending;
    expect(captures.captures().map((c) => c.path)).toEqual([ORDERS]);
    expect(registry.serialize()[PROFILING_STATE_KEY]).toMatchObject({ version: PROFILING_STATE_VERSION, captures: [ORDERS], profile: ORDERS });
    // A corrupt entry counts as none: everything closes.
    registry.restore({ version: 1, [PROFILING_STATE_KEY]: 'garbage' });
    await persistence.pending;
    expect(captures.captures()).toEqual([]);
    await captures.openPaths([HEAP1]);
    // An older project without the key: reset.
    registry.restore({ version: 1 });
    await persistence.pending;
    expect(captures.captures()).toEqual([]);
  });
});
