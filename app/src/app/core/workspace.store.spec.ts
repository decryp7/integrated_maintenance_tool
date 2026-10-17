import { TestBed } from '@angular/core/testing';
import { provideTinyBackend, settle, SEC, T0, testLogger, tinyBackend } from '../../testing/helpers';
import { ImtBackend } from './backend';
import { MockBackend } from './mock/mock-backend';
import { Platform } from './platform';
import { EMPTY_QUERY, loadPrefs, savePrefs, WorkspaceStore } from './workspace.store';

const fakePlatform = {
  failDialog: false,
  dropHandler: null as ((paths: string[]) => void) | null,
  hoverHandler: null as ((over: boolean) => void) | null,
  isTauri: () => false,
  async pickPaths(directory: boolean) {
    if (this.failDialog) throw 'no dialog';
    return [directory ? '/demo/dir' : '/demo/file.log'];
  },
  async onFilesDropped(handler: (paths: string[]) => void, hover: (over: boolean) => void) {
    this.dropHandler = handler;
    this.hoverHandler = hover;
    return () => undefined;
  },
  pickProfileToOpen: async () => null,
  pickProfileToSave: async () => null,
};

function setup(backend: ImtBackend = tinyBackend()) {
  TestBed.configureTestingModule({ providers: [provideTinyBackend(backend), { provide: Platform, useValue: fakePlatform }] });
  return TestBed.inject(WorkspaceStore);
}

describe('WorkspaceStore', () => {
  beforeEach(() => localStorage.clear());
  afterEach(() => document.documentElement.removeAttribute('data-theme'));

  it('loads the summary and the timeline on init', async () => {
    const store = setup();
    expect(store.busy()).toBe('Loading workspace…');
    await settle();
    expect(store.busy()).toBeNull();
    expect(store.summary()?.total).toBe(11);
    expect(store.dataVersion()).toBe(1);
    expect(store.effectiveGroupBy()).toEqual(['MachineName', 'ProcessId']);
    expect(store.timeline()?.lanes).toHaveLength(3);
    expect(store.timelineLoading()).toBe(false);
    expect(store.instanceCount()).toBe(3);
    expect(store.fieldNames()).toContain('cpu');
    expect(store.numericFields()).toContain('cpu');
    expect(store.markerKindMap().get('start')?.lifecycle).toBe(true);
  });

  it('debounces search text into the query', async () => {
    vi.useFakeTimers();
    try {
      const store = setup();
      store.setTextInput('time');
      store.setTextInput('timeout');
      expect(store.textInput()).toBe('timeout');
      expect(store.query().text).toBe('');
      vi.advanceTimersByTime(260);
      expect(store.query().text).toBe('timeout');
    } finally {
      vi.useRealTimers();
    }
  });

  it('updates query flags, levels, filters and groups', async () => {
    const store = setup();
    await settle();
    store.setRegex(true);
    store.setCaseSensitive(true);
    store.toggleLevel('Error');
    store.toggleLevel('Fatal');
    store.toggleLevel('Error');
    expect(store.query()).toMatchObject({ regex: true, caseSensitive: true, levels: ['Fatal'] });

    store.addFilter({ field: 'a', op: 'eq', value: '1' });
    store.addFilter({ field: 'a', op: 'eq', value: '1' });
    store.addFilter({ field: 'b', op: 'exists' });
    expect(store.query().filters).toHaveLength(2);
    store.removeFilter(0);
    expect(store.query().filters).toEqual([{ field: 'b', op: 'exists' }]);

    store.toggleGroup('WEB01 · 100');
    store.toggleGroup('WEB02 · 300');
    store.toggleGroup('WEB01 · 100');
    expect(store.query().groups).toEqual(['WEB02 · 300']);
    store.clearGroups();
    expect(store.query().groups).toEqual([]);

    store.toggleGroup('x');
    store.setGroupBy(['Application']);
    expect(store.query().groups).toEqual([]);
    expect(store.effectiveGroupBy()).toEqual(['Application']);
    expect(store.effectiveQuery().groupBy).toEqual(['Application']);

    store.setRange(T0, T0 + 10 * SEC);
    expect(store.hasRange()).toBe(true);
    store.resetRange();
    expect(store.hasRange()).toBe(false);

    store.setMarkedOnly(true);
    expect(store.markedOnly()).toBe(true);
    expect(store.query().markers).toEqual(['fatal', 'start', 'stop', 'exception', 'error', 'warning']);
    store.setMarkedOnly(false);
    expect(store.markedOnly()).toBe(false);

    store.setTextInput('abc');
    store.clearQuery();
    expect(store.query()).toEqual(EMPTY_QUERY);
    expect(store.textInput()).toBe('');
  });

  it('refetches the timeline when the query changes and ignores stale responses', async () => {
    const backend = tinyBackend();
    const spy = vi.spyOn(backend, 'queryTimeline');
    const store = setup(backend);
    await settle();
    const before = spy.mock.calls.length;
    store.toggleLevel('Fatal');
    store.toggleLevel('Error');
    await settle();
    expect(spy.mock.calls.length).toBeGreaterThan(before);
    expect(store.timeline()?.lanes[0].matched).toBe(2);
  });

  it('surfaces timeline errors', async () => {
    const backend = tinyBackend();
    vi.spyOn(backend, 'queryTimeline').mockRejectedValue('boom');
    const store = setup(backend);
    await settle();
    expect(store.error()).toBe('boom');
    expect(store.timelineLoading()).toBe(false);
  });

  it('opens, removes, clears and reloads sources', async () => {
    const store = setup();
    await settle();
    await store.openPaths(['/x/new.log']);
    expect(store.summary()?.sources).toHaveLength(3);
    expect(store.dataVersion()).toBe(2);
    await store.pickAndOpen(false);
    expect(store.summary()?.sources).toHaveLength(4);
    await store.removeSource(store.summary()!.sources[3].id);
    expect(store.summary()?.sources).toHaveLength(3);
    await store.clearWorkspace();
    expect(store.summary()?.total).toBe(0);
    await store.reload();
    expect(store.summary()?.total).toBe(11);
    const opened = testLogger().entries.find((e) => e.message === 'Workspace opened')!;
    expect(opened).toMatchObject({ level: 'info', category: 'ui.workspace', props: { Operation: 'opened', Sources: 3, Unrecognized: 1 } });
    expect(JSON.stringify(testLogger().entries)).not.toContain('/x/new.log');
    expect(testLogger().entries.map((e) => e.props?.['Operation'])).toEqual(['loaded', 'opened', 'opened', 'source removed', 'cleared', 'reloaded']);
  });

  it('records errors from failed operations', async () => {
    const backend = tinyBackend();
    const store = setup(backend);
    await settle();
    vi.spyOn(backend, 'openPaths').mockRejectedValueOnce(new Error('denied')).mockRejectedValueOnce({ code: 5 });
    await store.openPaths(['/x']);
    expect(store.error()).toBe('denied');
    expect(store.busy()).toBeNull();
    await store.pickAndOpen(true);
    expect(store.error()).toBe('{"code":5}');
    store.dismissError();
    expect(store.error()).toBeNull();
    store.setError('x');
    expect(store.error()).toBe('x');
  });

  it('reports dialog failures', async () => {
    const store = setup();
    await settle();
    fakePlatform.failDialog = true;
    await store.pickAndOpen(false);
    fakePlatform.failDialog = false;
    expect(store.error()).toBe('no dialog');
  });

  it('opens dropped files and shows the drop overlay', async () => {
    const store = setup();
    await settle();
    fakePlatform.hoverHandler!(true);
    expect(store.dropHover()).toBe(true);
    fakePlatform.dropHandler!(['/dropped.log']);
    await settle();
    expect(store.summary()?.sources.some((s) => s.path === '/dropped.log')).toBe(true);
  });

  it('tracks selection, anchors and totals', async () => {
    const store = setup();
    await settle();
    const page = await TestBed.inject(ImtBackend).queryPage({ query: EMPTY_QUERY, offset: 0, limit: 1, descending: false });
    store.select(page.rows[0]);
    expect(store.selected()?.id).toBe(0);
    store.setTab('metrics');
    store.jumpTo(4);
    expect(store.activeTab()).toBe('timeline');
    const first = store.anchor()!;
    store.jumpTo(4);
    expect(store.anchor()!.nonce).toBe(first.nonce + 1);
    store.anchorResolved(null);
    expect(store.anchorMissed()).toBe(true);
    store.anchorResolved(page.rows[0]);
    expect(store.anchorMissed()).toBe(false);
    store.setMatchTotal(5);
    expect(store.matchTotal()).toBe(5);
    store.select(null);
    expect(store.selected()).toBeNull();
  });

  it('only changes bucket count meaningfully', async () => {
    const store = setup();
    store.setBuckets(402);
    expect(store.buckets()).toBe(400);
    store.setBuckets(300);
    expect(store.buckets()).toBe(300);
  });

  it('handles view preferences and persists them', async () => {
    const store = setup();
    await settle();
    store.setTimezone('local');
    store.setAlwaysShowLifecycle(false);
    store.toggleSidebar();
    store.toggleInspector();
    store.toggleSort();
    store.addColumn('cpu');
    store.addColumn('cpu');
    store.addColumn('ws');
    store.removeColumn('cpu');
    store.setTimelineHeight(321.6);
    store.setTheme('light');
    store.setDropHover(true);
    store.focusSearch();
    await settle();
    expect(store.sidebarOpen()).toBe(false);
    expect(store.inspectorOpen()).toBe(false);
    expect(store.descending()).toBe(true);
    expect(store.extraColumns()).toEqual(['ws']);
    expect(store.timelineHeight()).toBe(322);
    expect(store.dropHover()).toBe(true);
    expect(store.focusSearchNonce()).toBe(1);
    expect(document.documentElement.getAttribute('data-theme')).toBe('light');
    expect(JSON.parse(localStorage.getItem('imt.prefs.v1')!)).toMatchObject({ timezone: 'local', theme: 'light', extraColumns: ['ws'], timelineHeight: 322, alwaysShowLifecycle: false });
    store.setTheme('system');
    await settle();
    expect(document.documentElement.hasAttribute('data-theme')).toBe(false);
  });

  it('reads and writes preferences defensively', () => {
    localStorage.setItem('imt.prefs.v1', JSON.stringify({ timezone: 'local', extraColumns: ['x'] }));
    expect(loadPrefs()).toEqual({ timezone: 'local', extraColumns: ['x'] });
    localStorage.setItem('imt.prefs.v1', '{not json');
    expect(loadPrefs()).toEqual({});
    localStorage.removeItem('imt.prefs.v1');
    expect(loadPrefs()).toEqual({});
    const setItem = vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => {
      throw new Error('quota');
    });
    expect(() => savePrefs({ timezone: 'utc', theme: 'dark', timelineHeight: 1, extraColumns: [], alwaysShowLifecycle: true })).not.toThrow();
    setItem.mockRestore();
  });
});
