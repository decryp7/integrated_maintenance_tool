import { TestBed } from '@angular/core/testing';
import { provideTinyBackendBare, tinyBackend } from '../../testing/helpers';
import { DebriefBackend } from '../core/backend';
import { createRunsBackend, hasRuns, provideRunsBackend, RunsBackend, TauriRunsBackend, UnsupportedRunsBackend } from './runs-backend';

const tauriKey = '__TAURI_INTERNALS__';

describe('RunsBackend', () => {
  afterEach(() => delete (window as unknown as Record<string, unknown>)[tauriKey]);

  it('maps the methods to their Tauri commands', async () => {
    const invoke = vi.fn(async () => 'ok');
    const b = new TauriRunsBackend(invoke as never);
    await b.queryRuns({ sort: 'duration', limit: 5 });
    expect(invoke).toHaveBeenLastCalledWith('query_runs', { request: { sort: 'duration', limit: 5 } });
    const draft = { kind: 'r', label: 'R', start: { type: 'markers' as const, kinds: ['start'] }, stop: { type: 'pattern' as const, pattern: 'x' }, per: 'instance' };
    await b.previewSpans({ draft });
    expect(invoke).toHaveBeenLastCalledWith('preview_spans', { request: { draft } });
  });

  it('uses the mock workspace in a browser, a stand-in when it cannot, and Tauri in the app', async () => {
    TestBed.configureTestingModule({ providers: [...provideTinyBackendBare(), provideRunsBackend()] });
    expect(TestBed.inject(RunsBackend)).toBe(TestBed.inject(DebriefBackend));
    expect(hasRuns(null)).toBe(false);
    expect(hasRuns({ queryRuns: () => 0 })).toBe(false);

    TestBed.resetTestingModule();
    TestBed.configureTestingModule({ providers: [...provideTinyBackendBare({} as DebriefBackend)] });
    const unsupported = TestBed.inject(RunsBackend);
    expect(unsupported).toBeInstanceOf(UnsupportedRunsBackend);
    expect((await unsupported.queryRuns({})).runs).toEqual([]);
    await expect(unsupported.previewSpans({ draft: { kind: 'r', label: '', start: { type: 'markers', kinds: [] }, stop: { type: 'markers', kinds: [] }, per: 'instance' } })).rejects.toMatch(/cannot preview/);

    (window as unknown as Record<string, unknown>)[tauriKey] = {};
    TestBed.resetTestingModule();
    TestBed.configureTestingModule({ providers: [...provideTinyBackendBare(tinyBackend())] });
    const tauri = TestBed.runInInjectionContext(() => createRunsBackend());
    expect(tauri).not.toBeInstanceOf(UnsupportedRunsBackend);
    expect(typeof tauri.queryRuns).toBe('function');
  });
});
