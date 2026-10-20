import { TestBed } from '@angular/core/testing';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { DotnetCaptureStatus, DotnetProcess } from '../../core/api.types';
import { setupProfiling } from '../testing/profiling-test-support';
import { DotnetBackend } from './dotnet-backend';
import { DotnetCaptureStore, DotnetOpener, isRunning, matchesProcess, POLL_MS } from './dotnet-capture.store';
import { MOCK_HEAP_PATH, MOCK_PROFILE_PATH, MockDotnetBackend } from './mock-dotnet-backend';

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
  const backend = new MockDotnetBackend(() => now);
  const opener = new FakeOpener();
  setupProfiling();
  TestBed.overrideProvider(DotnetBackend, { useValue: backend });
  TestBed.overrideProvider(DotnetOpener, { useValue: opener });
  const store = TestBed.inject(DotnetCaptureStore);
  return { store, backend, opener, advance: (ms: number) => (now += ms) };
}

const proc = (patch: Partial<DotnetProcess> = {}): DotnetProcess => ({ pid: 1, name: 'A.exe', commandLine: 'A.exe --serve', runtimeVersion: null, arch: 'x64', runtime: 'core', needsElevation: false, ...patch });

describe('dotnet capture helpers', () => {
  it('match processes by name, pid and command line', () => {
    expect(matchesProcess(proc(), '')).toBe(true);
    expect(matchesProcess(proc(), 'a.EXE')).toBe(true);
    expect(matchesProcess(proc(), '1')).toBe(true);
    expect(matchesProcess(proc(), '--serve')).toBe(true);
    expect(matchesProcess(proc({ commandLine: null }), 'zzz')).toBe(false);
    expect(isRunning({ state: 'stopping' } as DotnetCaptureStatus)).toBe(true);
    expect(isRunning({ state: 'done' } as DotnetCaptureStatus)).toBe(false);
  });
});

describe('DotnetCaptureStore', () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it('lists processes, selects the first and filters', async () => {
    const { store } = setup();
    await store.refresh();
    expect(store.processes().length).toBe(3);
    expect(store.selectedPid()).toBe(4242);
    store.setFilter('billing');
    expect(store.visible().map((p) => p.pid)).toEqual([5150]);
    store.select(5150);
    expect(store.selected()?.needsElevation).toBe(true);
    store.setFilter('');
    await store.refresh();
    expect(store.selectedPid(), 'keeps a selection that still exists').toBe(5150);
  });

  it('records until the duration ends, then opens the capture once', async () => {
    const { store, opener, advance } = setup();
    await store.refresh();
    store.setDuration(2);
    const s = await store.start();
    expect(s?.state).toBe('recording');
    expect(store.canRecord(), 'one capture per process').toBe(false);
    expect(store.running().length).toBe(1);
    advance(1000);
    await vi.advanceTimersByTimeAsync(POLL_MS);
    expect(store.captures()[0]?.bytes).toBeGreaterThan(0);
    advance(1500);
    await vi.advanceTimersByTimeAsync(POLL_MS);
    expect(store.captures()[0]?.state).toBe('done');
    expect(opener.opened).toEqual([MOCK_PROFILE_PATH]);
    await store.poll();
    expect(opener.opened.length, 'opened once').toBe(1);
    expect(store.running().length).toBe(0);
  });

  it('stops, cancels, and heap snapshots end by themselves', async () => {
    const { store, opener, advance } = setup();
    await store.refresh();
    const a = await store.start();
    await store.stop(a!.id);
    expect(store.captures()[0]?.state).toBe('done');
    expect(opener.opened).toEqual([MOCK_PROFILE_PATH]);
    store.select(6001);
    store.setKind('heap');
    const h = await store.start();
    expect(h?.kind).toBe('heap');
    advance(2000);
    await vi.advanceTimersByTimeAsync(POLL_MS);
    expect(opener.opened).toContain(MOCK_HEAP_PATH);
    store.select(4242);
    store.setKind('cpu');
    const c = await store.start();
    await store.cancel(c!.id);
    expect(store.captures().find((x) => x.id === c!.id)?.state).toBe('cancelled');
  });

  it('shows missing rights as a failed capture that asks for elevation', async () => {
    const { store } = setup();
    await store.refresh();
    store.select(5150);
    const s = await store.start();
    expect(s?.state).toBe('failed');
    expect(s?.needsElevation).toBe(true);
    expect(s?.error).toContain('administrator');
  });

  it('reports backend and opener failures as errors and clamps durations', async () => {
    const { store, backend, opener, advance } = setup();
    backend.failures.set('processes', 'no rights to list');
    await store.refresh();
    expect(store.error()).toBe('no rights to list');
    store.dismissError();
    expect(store.error()).toBeNull();
    backend.failures.clear();
    await store.refresh();
    backend.failures.set('start', 'boom');
    expect(await store.start()).toBeNull();
    expect(store.error()).toBe('boom');
    backend.failures.clear();
    opener.fail = true;
    store.setDuration(1);
    await store.start();
    advance(1500);
    await store.poll();
    expect(store.error()).toBe('cannot open');
    backend.failures.set('stop', 'gone');
    await store.stop('dotnet-1');
    expect(store.error()).toBe('gone');
    backend.failures.set('captures', 'down');
    await store.poll();
    expect(store.error()).toBe('down');
    store.setDuration(99999);
    expect(store.durationSec()).toBe(3600);
    store.setDuration(0.2);
    expect(store.durationSec()).toBe(1);
    store.setDuration(Number.NaN);
    expect(store.durationSec()).toBeNull();
    store.stopPolling();
  });

  it('does not reopen captures that finished before the window looked', async () => {
    const { store, backend, opener } = setup();
    const s = await backend.start({ pid: 4242, kind: 'cpu', durationSec: null });
    await backend.stop(s.id);
    await store.refresh();
    expect(opener.opened).toEqual([]);
    expect(store.captures().length).toBe(1);
  });
});
