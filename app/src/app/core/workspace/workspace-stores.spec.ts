import { TestBed } from '@angular/core/testing';
import { provideTinyBackendBare, settle, testLogger, tinyBackend } from '../../../testing/helpers';
import { Platform } from '../platform';
import { WorkspaceSummary } from '../api.types';
import { ImtBackend } from '../backend';
import { provideWorkspace } from './provide-workspace';
import { QueryStore } from './query.store';
import { SelectionStore } from './selection.store';
import { WorkspaceStatusStore } from './status.store';
import { ThemeService } from './theme.service';
import { TimelineStore } from './timeline-data.store';
import { UiPrefsStore } from './ui-prefs.store';
import { sameSources, WorkspaceDataStore } from './workspace-data.store';

function bare(backend: ImtBackend = tinyBackend()) {
  TestBed.configureTestingModule({ providers: provideTinyBackendBare(backend) });
  return { data: TestBed.inject(WorkspaceDataStore), status: TestBed.inject(WorkspaceStatusStore), backend };
}

describe('workspace stores', () => {
  beforeEach(() => {
    localStorage.clear();
    document.documentElement.removeAttribute('data-theme');
  });
  afterEach(() => document.documentElement.removeAttribute('data-theme'));

  it('have no side effects of their own besides loading the summary: nothing persists or touches the document without provideWorkspace', async () => {
    const backend = tinyBackend();
    const summary = vi.spyOn(backend, 'workspaceSummary');
    const { data } = bare(backend);
    const prefs = TestBed.inject(UiPrefsStore);
    prefs.setScheme('dark');
    prefs.setTimezone('local');
    await settle();
    expect(summary).toHaveBeenCalledTimes(1);
    expect(data.summary()?.total).toBe(11);
    expect(localStorage.getItem('imt.prefs.v1')).toBeNull();
    expect(document.documentElement.hasAttribute('data-theme')).toBe(false);
  });

  it('provideWorkspace starts the theme and persistence; the summary loads once, when first used', async () => {
    const backend = tinyBackend();
    const summary = vi.spyOn(backend, 'workspaceSummary');
    TestBed.configureTestingModule({ providers: [provideWorkspace(), ...provideTinyBackendBare(backend)] });
    const data = TestBed.inject(WorkspaceDataStore);
    const prefs = TestBed.inject(UiPrefsStore);
    await settle();
    expect(summary).toHaveBeenCalledTimes(1);
    expect(data.summary()?.total).toBe(11);
    prefs.setScheme('light');
    await settle();
    expect(document.documentElement.getAttribute('data-scheme')).toBe('light');
    expect(document.documentElement.getAttribute('data-theme')).toBe('calm');
    expect(JSON.parse(localStorage.getItem('imt.prefs.v1')!)).toMatchObject({ scheme: 'light', theme: null });
    // Starting the theme again adds no second effect.
    TestBed.inject(ThemeService).start();
    prefs.setScheme('system');
    await settle();
    expect(['light', 'dark']).toContain(document.documentElement.getAttribute('data-scheme'));
  });

  it('keeps working when the window cannot listen for file drops, and logs why', async () => {
    const platform = { onFilesDropped: () => Promise.reject(new Error('no webview')), isTauri: () => true };
    TestBed.configureTestingModule({ providers: [provideWorkspace(), ...provideTinyBackendBare(), { provide: Platform, useValue: platform }] });
    const data = TestBed.inject(WorkspaceDataStore);
    await settle();
    expect(data.summary()?.total).toBe(11);
    expect(testLogger().entries.some((e) => e.message === 'File drops are not available' && String(e.props?.['Error']).includes('no webview'))).toBe(true);
  });

  it('tells appended events from changed sources in its generations', async () => {
    const { data, backend } = bare();
    await settle();
    expect(data.sourcesVersion()).toBe(1);
    expect(data.appendVersion()).toBe(0);
    expect(data.dataVersion()).toBe(1);
    // Live tail: same sources, more events.
    await data.refreshSummary();
    expect(data.sourcesVersion()).toBe(1);
    expect(data.appendVersion()).toBe(1);
    expect(data.dataVersion()).toBe(2);
    // The host of a pop-out opened another file: the sources changed.
    const current = data.summary()!;
    vi.spyOn(backend, 'workspaceSummary').mockResolvedValueOnce({ ...current, sources: current.sources.slice(1) });
    await data.refreshSummary();
    expect(data.sourcesVersion()).toBe(2);
    expect(data.appendVersion()).toBe(1);
    expect(data.dataVersion()).toBe(3);
  });

  it('sameSources compares source ids in order and the profile', () => {
    const base = { sources: [{ id: 1 }, { id: 2 }], profileName: 'p' } as unknown as WorkspaceSummary;
    expect(sameSources(null, base)).toBe(false);
    expect(sameSources(base, { ...base })).toBe(true);
    expect(sameSources(base, { ...base, profileName: 'q' })).toBe(false);
    expect(sameSources(base, { ...base, sources: [base.sources[1]!, base.sources[0]!] })).toBe(false);
    expect(sameSources(base, { ...base, sources: base.sources.slice(1) })).toBe(false);
  });

  it('marks opens and re-imports as imports and other commands as operations, with typed busy state', async () => {
    const backend = tinyBackend();
    const { data, status } = bare(backend);
    const kinds: (string | undefined)[] = [];
    const record = <T>(p: Promise<T>) => {
      kinds.push(status.busy()?.kind);
      return p;
    };
    await settle();
    await record(data.init());
    await record(data.openPaths(['/a.log']));
    await record(data.reload());
    await record(data.removeSource(data.summary()!.sources[0]!.id));
    await record(data.clearWorkspace());
    expect(kinds).toEqual(['op', 'import', 'import', 'op', 'op']);
    expect(status.busy()).toBeNull();
  });

  it('clears the selection and the error when a new summary applies', async () => {
    const { data, status } = bare();
    await settle();
    const selection = TestBed.inject(SelectionStore);
    selection.select({ id: 1 } as never);
    status.setError('old');
    data.applySummary(data.summary()!);
    expect(selection.selected()).toBeNull();
    expect(status.error()).toBeNull();
  });

  it('SelectionStore issues a fresh anchor per jump and reports misses', () => {
    TestBed.configureTestingModule({});
    const selection = TestBed.inject(SelectionStore);
    selection.jumpTo(4);
    const first = selection.anchor();
    selection.jumpTo(4);
    expect(selection.anchor()?.nonce).toBe((first?.nonce ?? 0) + 1);
    selection.anchorResolved(null);
    expect(selection.anchorMissed()).toBe(true);
    selection.select(null);
    expect(selection.anchorMissed()).toBe(false);
  });

  it('TimelineStore fetches for the effective query and keeps only the latest answer', async () => {
    const { data } = bare();
    const timeline = TestBed.inject(TimelineStore);
    const query = TestBed.inject(QueryStore);
    expect(data).toBeTruthy();
    await settle();
    expect(timeline.timeline()?.lanes).toHaveLength(3);
    expect(timeline.instanceCount()).toBe(3);
    const spy = vi.spyOn(TestBed.inject(ImtBackend), 'queryTimeline');
    query.toggleLevel('Error');
    await settle();
    expect(spy).toHaveBeenCalledTimes(1);
    expect(spy.mock.calls[0]?.[0].query.levels).toEqual(['Error']);
    expect(timeline.loading()).toBe(false);
    expect(timeline.error()).toBeNull();
  });
});
