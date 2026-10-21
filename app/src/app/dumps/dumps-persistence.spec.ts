import { TestBed } from '@angular/core/testing';
import { describe, expect, it } from 'vitest';
import { DumpsPersistence, DUMPS_STATE_KEY, DUMPS_STATE_VERSION, parseDumpsUiState } from './dumps-persistence';
import { DumpsStore } from './dumps.store';
import { MOCK_DUMP_PATHS, MOCK_UMDH_PATHS } from './mock/mock-dumps-backend';
import { setupDumps } from './testing/dumps-test-support';

describe('parseDumpsUiState', () => {
  it('reads what it can and defaults the rest', () => {
    expect(parseDumpsUiState(null)).toBeNull();
    expect(parseDumpsUiState([1])).toBeNull();
    expect(parseDumpsUiState('x')).toBeNull();
    expect(parseDumpsUiState({})).toEqual({ version: DUMPS_STATE_VERSION, paths: [], dump: null, umdhTarget: null, umdhBase: null });
    const p = parseDumpsUiState({ version: 9, paths: ['/a', '/a', 3, '', '/b'], dump: '/a', umdhTarget: 7, umdhBase: '/b' });
    expect(p).toEqual({ version: 9, paths: ['/a', '/b'], dump: '/a', umdhTarget: null, umdhBase: '/b' });
    expect(parseDumpsUiState({ paths: Array.from({ length: 40 }, (_, i) => `/f${i}`) })!.paths).toHaveLength(32);
  });
});

describe('DumpsPersistence', () => {
  it('captures the open documents and restores them by path', async () => {
    setupDumps();
    const store = TestBed.inject(DumpsStore);
    const p = TestBed.inject(DumpsPersistence);
    expect(p.contribution.key).toBe(DUMPS_STATE_KEY);
    await store.openPaths([MOCK_DUMP_PATHS[0]!, ...MOCK_UMDH_PATHS]);
    const saved = p.contribution.capture() as ReturnType<DumpsPersistence['capture']>;
    expect(saved.paths).toHaveLength(4);
    expect(saved.dump).toBe(MOCK_DUMP_PATHS[0]);
    expect(saved.umdhTarget).toBe(MOCK_UMDH_PATHS[2]);
    expect(saved.umdhBase).toBe(MOCK_UMDH_PATHS[0]);

    // A project with only two of them: the others close, the comparison is restored.
    const restoring = p.restore({ ...saved, paths: [MOCK_UMDH_PATHS[0]!, MOCK_UMDH_PATHS[1]!, '/missing/a.dmp'], dump: null, umdhTarget: MOCK_UMDH_PATHS[1]!, umdhBase: MOCK_UMDH_PATHS[0]! });
    expect((p.capture() as { paths: string[] }).paths).toHaveLength(3);
    await restoring;
    expect(store.docs().map((d) => d.path)).toEqual([MOCK_UMDH_PATHS[0], MOCK_UMDH_PATHS[1]]);
    expect(store.notice()).toContain('not found: a.dmp');
    expect(p.capture().umdhTarget).toBe(MOCK_UMDH_PATHS[1]);

    p.contribution.reset!();
    await p.pending;
    expect(store.docs()).toEqual([]);
    p.contribution.restore({ paths: [MOCK_DUMP_PATHS[0]] });
    await p.pending;
    expect(store.dump()?.path).toBe(MOCK_DUMP_PATHS[0]);
    p.contribution.restore('garbage');
    await p.pending;
    expect(store.docs()).toEqual([]);
  });
});
