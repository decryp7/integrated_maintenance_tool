import { TestBed } from '@angular/core/testing';
import { query } from '../../testing/helpers';
import { ImtBackend, provideBackend, TauriBackend } from './backend';
import { MockBackend } from './mock/mock-backend';

const invoke = vi.hoisted(() => vi.fn(async (..._args: unknown[]) => 'ok'));

describe('TauriBackend', () => {
  beforeEach(() => invoke.mockClear());

  it('maps every method to its Tauri command', async () => {
    const b = new TauriBackend(invoke as never);
    const q = query();
    const calls: [() => Promise<unknown>, string, unknown?][] = [
      [() => b.workspaceSummary(), 'workspace_summary'],
      [() => b.openPaths(['/a']), 'open_paths', { paths: ['/a'] }],
      [() => b.removeSource(3), 'remove_source', { sourceId: 3 }],
      [() => b.clearWorkspace(), 'clear_workspace'],
      [() => b.reloadWorkspace(), 'reload_workspace'],
      [() => b.queryPage({ query: q, offset: 0, limit: 1, descending: false }), 'query_page', { request: { query: q, offset: 0, limit: 1, descending: false } }],
      [() => b.queryTimeline({ query: q, buckets: 5, alwaysShowLifecycle: true }), 'query_timeline', { request: { query: q, buckets: 5, alwaysShowLifecycle: true } }],
      [() => b.queryFacet({ query: q, field: 'f', limit: 3 }), 'query_facet', { request: { query: q, field: 'f', limit: 3 } }],
      [() => b.queryChart({ query: q, chartId: 'c', buckets: 4 }), 'query_chart', { request: { query: q, chartId: 'c', buckets: 4 } }],
      [() => b.getProfile(), 'get_profile'],
      [() => b.validateProfile('x'), 'validate_profile', { toml: 'x' }],
      [() => b.saveProfile('x'), 'save_profile', { toml: 'x' }],
      [() => b.previewExtractor({ toml: 'x', extractor: 'e', query: q, limit: 2 }), 'preview_extractor', { request: { toml: 'x', extractor: 'e', query: q, limit: 2 } }],
      [() => b.importProfile('/p.toml'), 'import_profile', { path: '/p.toml' }],
      [() => b.exportProfile('/p.toml', 'x'), 'export_profile', { path: '/p.toml', toml: 'x' }],
      [() => b.exportEvents({ query: q, format: 'csv', path: '/e.csv' }), 'export_events', { request: { query: q, format: 'csv', path: '/e.csv' } }],
    ];
    for (const [call, name, args] of calls) {
      invoke.mockClear();
      await call();
      expect(invoke).toHaveBeenCalledTimes(1);
      expect(invoke.mock.calls[0][0]).toBe(name);
      if (args) expect(invoke.mock.calls[0][1]).toEqual(args);
    }
  });
});

describe('provideBackend', () => {
  afterEach(() => {
    delete (window as unknown as Record<string, unknown>)['__TAURI_INTERNALS__'];
    TestBed.resetTestingModule();
  });

  it('uses the mock backend in a plain browser', () => {
    TestBed.configureTestingModule({ providers: [provideBackend()] });
    expect(TestBed.inject(ImtBackend)).toBeInstanceOf(MockBackend);
  });

  it('uses Tauri inside the desktop shell', () => {
    (window as unknown as Record<string, unknown>)['__TAURI_INTERNALS__'] = {};
    TestBed.configureTestingModule({ providers: [provideBackend()] });
    expect(TestBed.inject(ImtBackend)).toBeInstanceOf(TauriBackend);
  });
});
