import { TestBed } from '@angular/core/testing';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { NativeCaptureStatus, NativeProcess } from '../../core/api.types';
import { COMMAND_CONTEXT } from '../../extensions/command-registry';
import { CapturesStore } from '../captures.store';
import { PROFILING_VIEW_IDS as V } from '../profiling-navigator';
import { setupProfiling } from '../testing/profiling-test-support';
import { NativeBackend } from './native-backend';
import { clampInt, isRunning, matchesProcess, NativeCaptureStore, NativeOpener, POLL_MS } from './native-capture.store';
import { MOCK_NATIVE_PROFILE_PATH, MockNativeBackend } from './mock-native-backend';

class FakeOpener {
  opened: string[] = [];
  fail = false;
  async open(path: string) {
    if (this.fail) throw new Error('cannot open');
    this.opened.push(path);
  }
}

function setup() {
  let now = 1_000_000;
  const backend = new MockNativeBackend(() => now);
  const opener = new FakeOpener();
  setupProfiling();
  TestBed.overrideProvider(NativeBackend, { useValue: backend });
  TestBed.overrideProvider(NativeOpener, { useValue: opener });
  const store = TestBed.inject(NativeCaptureStore);
  return { store, backend, opener, advance: (ms: number) => (now += ms) };
}

const proc = (patch: Partial<NativeProcess> = {}): NativeProcess => ({ pid: 1, name: 'A.exe', path: 'C:\\x\\A.exe', arch: 'x64', accessible: true, ...patch });

describe('native capture helpers', () => {
  it('match processes by name, pid and path; clamp numbers; know running captures', () => {
    expect(matchesProcess(proc(), '')).toBe(true);
    expect(matchesProcess(proc(), 'a.EXE')).toBe(true);
    expect(matchesProcess(proc(), '1')).toBe(true);
    expect(matchesProcess(proc(), '\\x\\')).toBe(true);
    expect(matchesProcess(proc({ path: null }), 'zzz')).toBe(false);
    expect(clampInt(5000, 1, 1000, 100)).toBe(1000);
    expect(clampInt(0.4, 1, 1000, 100)).toBe(1);
    expect(clampInt(Number.NaN, 1, 1000, 100)).toBe(100);
    expect(isRunning({ state: 'stopping' } as NativeCaptureStatus)).toBe(true);
    expect(isRunning({ state: 'failed' } as NativeCaptureStatus)).toBe(false);
  });
});

describe('NativeCaptureStore', () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it('loads the settings defaults once, lists processes and knows which need ETW', async () => {
    const { store } = setup();
    await store.refresh();
    expect(store.symbolPath()).toBe('C:\\symbols');
    expect(store.symbolServer()).toBe(false);
    expect(store.processes().length).toBe(3);
    expect(store.selectedPid()).toBe(3100);
    store.setSymbolPath('D:\\pdb');
    await store.refresh();
    expect(store.symbolPath(), 'the user’s edit survives a refresh').toBe('D:\\pdb');
    store.select(640);
    expect(store.needsEtw()).toBe(true);
    expect(store.canRecord()).toBe(false);
    store.setMode('etw');
    expect(store.needsEtw()).toBe(false);
    expect(store.canRecord()).toBe(true);
    store.setFilter('legacy');
    expect(store.visible().map((p) => p.pid)).toEqual([3200]);
  });

  it('sends each mode only its own options', async () => {
    const { store } = setup();
    await store.refresh();
    store.setRate(250);
    store.setDuration(null);
    expect(store.request(1)).toEqual({ pid: 1, durationSec: null, symbolPath: 'C:\\symbols', symbolServer: false, rateHz: 250 });
    store.setMode('etw');
    store.setHeap(true);
    store.setSymbolServer(true);
    store.setDuration(99999);
    expect(store.request(2)).toEqual({ pid: 2, durationSec: 3600, symbolPath: 'C:\\symbols', symbolServer: true, heap: true });
  });

  it('records, polls while running, opens the finished capture once, and polls ETW only after it was used', async () => {
    const { store, backend, opener, advance } = setup();
    await store.refresh();
    expect(backend.polled.every((m) => m.join() === 'sampling'), 'no administrator prompt before ETW is chosen').toBe(true);
    store.setDuration(2);
    const started = await store.start();
    expect(started?.state).toBe('recording');
    expect(store.canRecord(), 'one capture per process').toBe(false);
    advance(2500);
    await vi.advanceTimersByTimeAsync(POLL_MS);
    expect(store.captures()[0]?.state).toBe('done');
    expect(opener.opened).toEqual([MOCK_NATIVE_PROFILE_PATH]);
    await store.poll();
    expect(opener.opened.length, 'opened once').toBe(1);
    store.select(640);
    store.setMode('etw');
    await store.start();
    expect(store.etwUsed()).toBe(true);
    await store.poll();
    expect(backend.polled.at(-1)).toEqual(['sampling', 'etw']);
    const etw = store.captures().find((c) => c.mode === 'etw')!;
    await store.stop(etw);
    expect(store.captures().find((c) => c.id === etw.id)?.state).toBe('done');
  });

  it('cancels, reports denied starts with their code, and shows failures', async () => {
    const { store, backend, opener } = setup();
    await store.refresh();
    store.setDuration(null);
    const s = (await store.start())!;
    await store.cancel(s);
    expect(store.captures()[0]?.state).toBe('cancelled');
    backend.declineElevation = true;
    store.setMode('etw');
    await store.start();
    expect(store.errorCode()).toBe('denied');
    expect(store.error()).toContain('declined');
    store.dismissError();
    expect(store.error()).toBeNull();
    backend.failures.set('stop', 'stop failed');
    store.setMode('sampling');
    backend.declineElevation = false;
    const r = (await store.start())!;
    await store.stop(r);
    expect(store.error()).toBe('stop failed');
    backend.failures.set('processes', 'no list');
    await store.refresh();
    expect(store.error()).toBe('no list');
    opener.fail = true;
    backend.failures.clear();
    await store.stop(r);
    expect(store.error()).toBe('cannot open');
    backend.failures.set('captures', 'poll failed');
    await store.poll();
    expect(store.error()).toBe('poll failed');
    store.stopPolling();
  });
});

describe('NativeOpener', () => {
  it('opens the capture and shows its findings first', async () => {
    const { context } = setupProfiling();
    const captures = TestBed.inject(CapturesStore);
    const spy = vi.spyOn(captures, 'openPaths').mockResolvedValue([{ capture: 'profile', id: 1 } as never]);
    await TestBed.inject(NativeOpener).open('C:\\x.nettrace');
    expect(spy).toHaveBeenCalledWith(['C:\\x.nettrace']);
    expect(TestBed.inject(COMMAND_CONTEXT)).toBe(context);
    expect(context.last('showView')).toEqual([V.findings]);
    spy.mockResolvedValue([]);
    const before = context.calls.length;
    await TestBed.inject(NativeOpener).open('C:/y.nettrace');
    expect(context.calls.length, 'nothing opened: no view change').toBe(before);
  });
});
