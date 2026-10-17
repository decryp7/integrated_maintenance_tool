import { TestBed } from '@angular/core/testing';
import { describe, expect, it, vi } from 'vitest';
import { Logger } from '../core/logger';
import { MockProfilingBackend } from '../core/mock/mock-profiling';
import { MemoryLogger } from '../../testing/helpers';
import { createProfilingBackend, provideProfilingBackend, ProfilingBackend, TauriProfilingBackend } from './profiling-backend';
import { ProfilingFailure } from './profiling-errors';

type Invoke = ConstructorParameters<typeof TauriProfilingBackend>[0];

function tauri() {
  const call = vi.fn(async (..._args: unknown[]) => ({ ok: true }));
  return { call, backend: new TauriProfilingBackend(call as unknown as Invoke) };
}

describe('TauriProfilingBackend', () => {
  it('sends path, id and no-argument commands with the right shapes', async () => {
    const { call, backend } = tauri();
    await backend.open('/a/b.cpuprofile');
    await backend.list();
    await backend.close(7);
    await backend.spans();
    await backend.sniff(['/a.log', '/b.nettrace']);
    expect(call.mock.calls).toEqual([
      ['profiling_open', { path: '/a/b.cpuprofile' }],
      ['profiling_list'],
      ['profiling_close', { id: 7 }],
      ['profiling_spans'],
      ['profiling_sniff', { paths: ['/a.log', '/b.nettrace'] }],
    ]);
  });

  it('wraps every query request in { request }', async () => {
    const { call, backend } = tauri();
    const r = { profileId: 1 };
    const calls: Array<[keyof TauriProfilingBackend, string, unknown]> = [
      ['hotspots', 'profiling_hotspots', { profileId: 1, offset: 0 }],
      ['tree', 'profiling_tree', { profileId: 1, path: [1, 2] }],
      ['hotPath', 'profiling_hot_path', r],
      ['framePaths', 'profiling_frame_paths', { profileId: 1, frame: 3 }],
      ['butterfly', 'profiling_butterfly', { profileId: 1, frame: 3 }],
      ['flame', 'profiling_flame', r],
      ['diffFlame', 'profiling_diff_flame', { profileId: 2, baselineId: 1 }],
      ['functionDiff', 'profiling_function_diff', { profileId: 2, baselineId: 1 }],
      ['threads', 'profiling_threads', r],
      ['findings', 'profiling_findings', r],
      ['moment', 'profiling_moment', { profileId: 1, atNs: 5 }],
      ['heapTypes', 'profiling_heap_types', { heapId: 1 }],
      ['heapDiff', 'profiling_heap_diff', { heapId: 2, baselineId: 1 }],
      ['heapGrowth', 'profiling_heap_growth', { heapIds: [1, 2] }],
      ['heapDominators', 'profiling_heap_dominators', { heapId: 1 }],
      ['heapInstances', 'profiling_heap_instances', { heapId: 1, type: 'Order' }],
      ['heapPaths', 'profiling_heap_paths', { heapId: 1, object: 4 }],
    ];
    for (const [method, command, request] of calls) {
      call.mockClear();
      await (backend[method] as (r: unknown) => Promise<unknown>).call(backend, request);
      expect(call).toHaveBeenCalledTimes(1);
      expect(call).toHaveBeenCalledWith(command, { request });
    }
  });

  it('returns what the shell answered and turns rejections into typed failures', async () => {
    const call = vi
      .fn()
      .mockResolvedValueOnce({ id: 3 })
      .mockRejectedValueOnce({ code: 'notFound', message: 'cannot open /y: No such file' })
      .mockRejectedValueOnce('invalid args: profileId');
    const backend = new TauriProfilingBackend(call as unknown as Invoke);
    expect(await backend.open('/x')).toEqual({ id: 3 });
    const missing = await backend.open('/y').catch((e: unknown) => e);
    expect(missing).toBeInstanceOf(ProfilingFailure);
    expect(missing).toMatchObject({ code: 'notFound', message: 'cannot open /y: No such file' });
    // Tauri refusing the arguments rejects with a plain string: still a typed failure.
    await expect(backend.hotspots({ profileId: 1 })).rejects.toMatchObject({ code: 'invalid', message: 'invalid args: profileId' });
  });
});

describe('createProfilingBackend', () => {
  it('returns the mock outside Tauri, with failures logged and still thrown', async () => {
    TestBed.configureTestingModule({ providers: [{ provide: Logger, useFactory: () => new MemoryLogger() }] });
    const backend = TestBed.runInInjectionContext(() => createProfilingBackend());
    const opened = await backend.open('/demo/profiles/orders.speedscope.json');
    expect(opened.capture).toBe('profile');
    expect(await backend.list()).toHaveLength(1);
    await expect(backend.open('/demo/readme.txt')).rejects.toMatchObject({ code: 'notACapture', message: expect.stringContaining('readme.txt') });
    const log = TestBed.inject(Logger) as MemoryLogger;
    expect(log.entries).toHaveLength(1);
    expect(log.entries[0]).toMatchObject({ level: 'warn', category: 'ui.backend', props: { Command: 'open' } });
    // Sniffing never opens and never fails for files that are not captures.
    expect(await backend.sniff(['/demo/readme.txt', '/demo/heap-1.heapsnapshot'])).toEqual([
      { path: '/demo/readme.txt', kind: 'none' },
      { path: '/demo/heap-1.heapsnapshot', kind: 'heap' },
    ]);
  });

  it('wraps the Tauri backend inside Tauri', () => {
    const w = window as unknown as Record<string, unknown>;
    w['__TAURI_INTERNALS__'] = {};
    try {
      TestBed.configureTestingModule({ providers: [{ provide: Logger, useFactory: () => new MemoryLogger() }] });
      const backend = TestBed.runInInjectionContext(() => createProfilingBackend());
      expect(backend).toBeInstanceOf(TauriProfilingBackend);
    } finally {
      delete w['__TAURI_INTERNALS__'];
    }
  });

  it('is the ProfilingBackend provider the app uses', async () => {
    TestBed.configureTestingModule({ providers: [{ provide: Logger, useFactory: () => new MemoryLogger() }, provideProfilingBackend()] });
    const backend = TestBed.inject(ProfilingBackend);
    expect(backend).not.toBeInstanceOf(TauriProfilingBackend);
    expect(backend).toBeInstanceOf(MockProfilingBackend);
    expect(await backend.list()).toEqual([]);
  });
});
