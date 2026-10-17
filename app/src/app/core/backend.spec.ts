import { TestBed } from '@angular/core/testing';
import { query } from '../../testing/helpers';
import { ImtBackend, provideBackend, TauriBackend, WORKSPACE_CHANGED_EVENT } from './backend';
import { FailureReporter } from './failure-reporter';
import { Logger } from './logger';
import { MockBackend } from './mock/mock-backend';
import { MemoryLogger } from '../../testing/helpers';

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
      [() => b.sourceContext({ recordId: 3, before: 10, after: 10 }), 'source_context', { request: { recordId: 3, before: 10, after: 10 } }],
      [() => b.readFileHead('/a.log', 100), 'read_file_head', { path: '/a.log', maxBytes: 100 }],
      [() => b.previewParse({ toml: 'x', sample: 's', fileName: 'a.log', limit: 5 }), 'preview_parse', { request: { toml: 'x', sample: 's', fileName: 'a.log', limit: 5 } }],
      [() => b.setLive(true), 'set_live', { enabled: true }],
      [() => b.inferRegex(['a 1', 'a 2']), 'infer_regex', { lines: ['a 1', 'a 2'] }],
      [() => b.explainRegex('a\\d+'), 'explain_regex', { pattern: 'a\\d+' }],
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

describe('TauriBackend events', () => {
  it('subscribes to the workspace-changed event through the injected listener', async () => {
    const off = vi.fn();
    const subscribe = vi.fn(async (_event: string, _handler: () => void) => off);
    const b = new TauriBackend(invoke as never, subscribe);
    const handler = vi.fn();
    expect(await b.onWorkspaceChanged(handler)).toBe(off);
    expect(subscribe).toHaveBeenCalledWith(WORKSPACE_CHANGED_EVENT, handler);
    expect(WORKSPACE_CHANGED_EVENT).toBe('imt://workspace-changed');
  });

  it('uses the Tauri event API by default', async () => {
    const unlisten = vi.fn();
    let callback: (() => void) | undefined;
    vi.doMock('@tauri-apps/api/event', () => ({ listen: vi.fn(async (_e: string, cb: () => void) => ((callback = cb), unlisten)) }));
    const b = new TauriBackend(invoke as never);
    const handler = vi.fn();
    const off = await b.onWorkspaceChanged(handler);
    callback!();
    expect(handler).toHaveBeenCalledTimes(1);
    expect(off).toBe(unlisten);
    vi.doUnmock('@tauri-apps/api/event');
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

  it('reads the mock switches from the URL', async () => {
    window.history.replaceState({}, '', '/?mockTotal=1234');
    try {
      TestBed.configureTestingModule({ providers: [provideBackend()] });
      expect((await TestBed.inject(ImtBackend).workspaceSummary()).total).toBe(1234);
    } finally {
      window.history.replaceState({}, '', '/');
    }
  });

  it('uses Tauri inside the desktop shell', () => {
    (window as unknown as Record<string, unknown>)['__TAURI_INTERNALS__'] = {};
    TestBed.configureTestingModule({ providers: [provideBackend()] });
    expect(TestBed.inject(ImtBackend)).toBeInstanceOf(TauriBackend);
  });

  it('logs a failed command and reports it to the FailureReporter, then rethrows', async () => {
    window.history.replaceState({}, '', '/?mockFail=queryFacet:1');
    try {
      const backendFailed = vi.fn();
      const logger = new MemoryLogger();
      TestBed.configureTestingModule({
        providers: [provideBackend(), { provide: Logger, useValue: logger }, { provide: FailureReporter, useValue: { backendFailed, panelFailed: vi.fn() } }],
      });
      const backend = TestBed.inject(ImtBackend);
      await expect(backend.queryFacet({ query: query(), field: 'f', limit: 1 })).rejects.toBe('Simulated queryFacet failure');
      expect(backendFailed).toHaveBeenCalledWith('queryFacet', 'Simulated queryFacet failure');
      expect(logger.entries).toEqual([expect.objectContaining({ category: 'ui.backend', props: expect.objectContaining({ Command: 'queryFacet' }) })]);
    } finally {
      window.history.replaceState({}, '', '/');
    }
  });

  it('only logs failures when no FailureReporter is provided', async () => {
    window.history.replaceState({}, '', '/?mockFail=queryFacet:1');
    try {
      TestBed.configureTestingModule({ providers: [provideBackend()] });
      await expect(TestBed.inject(ImtBackend).queryFacet({ query: query(), field: 'f', limit: 1 })).rejects.toBe('Simulated queryFacet failure');
    } finally {
      window.history.replaceState({}, '', '/');
    }
  });
});
