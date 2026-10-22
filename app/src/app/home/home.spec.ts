import { inject } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { buttonByText, FakeDock, provideFakeDock, provideTinyBackend, settle, text } from '../../testing/helpers';
import { FakeECharts } from '../../testing/fake-echarts';
import { AnalysisBackend } from '../core/analysis-backend';
import { Finding, fullCoverage, HomeSummary } from '../core/api.types';
import { ImtBackend } from '../core/backend';
import { MockAnalysisBackend, recordsFrom } from '../mock-backends';
import { LAYOUT_MODE_KEY, LayoutModeStore } from '../core/workspace/layout-mode';
import { WorkspaceStore } from '../core/workspace.store';
import { COMMAND_CONTEXT, CommandRegistry } from '../extensions/command-registry';
import { CommandContext } from '../extensions/tokens';
import { ViewRegistry } from '../extensions/view-registry';
import { ShareBackend } from '../share/share-backend';
import { HomeBackend, TauriHomeBackend } from './home-backend';
import { HOME_AUTO_OPEN_KEY, HomePrefs } from './home-prefs';
import { HomeAutoOpen, provideHome } from './home.plugin';
import { HomeStore, RefreshLimiter } from './home.store';
import { HomeView } from './home-view';

const crash: Finding = { kind: 'crash', severity: 'critical', title: 'Crash', detail: 'WEB01 stopped', action: { kind: 'zoom', from: 1000, to: 2000, recordId: 7 } };
const file: Finding = { kind: 'fileNotOpened', severity: 'info', title: 'File not opened', detail: 'x.reg: no importer', action: { kind: 'setupFormat', path: '/logs/x.reg' } };

class FakeCtx implements CommandContext {
  calls: string[] = [];
  summary = null;
  openPaths = async () => undefined;
  setQuery = (p: object) => void this.calls.push(`query:${JSON.stringify(p)}`);
  addFilter = () => undefined;
  selectRecord = () => undefined;
  jumpTo = async () => undefined;
  showView = (id: string) => void this.calls.push(`show:${id}`);
  notifyError = (m: string) => void this.calls.push(`error:${m}`);
}

describe('HomePrefs', () => {
  afterEach(() => localStorage.removeItem(HOME_AUTO_OPEN_KEY));

  it('defaults to opening, remembers the choice and survives broken storage', () => {
    expect(TestBed.inject(HomePrefs).autoOpen()).toBe(true);
    TestBed.inject(HomePrefs).setAutoOpen(false);
    expect(localStorage.getItem(HOME_AUTO_OPEN_KEY)).toBe('false');
    TestBed.resetTestingModule();
    expect(TestBed.inject(HomePrefs).autoOpen()).toBe(false);
    TestBed.resetTestingModule();
    const get = vi.spyOn(Storage.prototype, 'getItem').mockImplementation(() => {
      throw new Error('denied');
    });
    const set = vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => {
      throw new Error('denied');
    });
    const prefs = TestBed.inject(HomePrefs);
    expect(prefs.autoOpen()).toBe(true);
    prefs.setAutoOpen(false);
    expect(prefs.autoOpen()).toBe(false);
    get.mockRestore();
    set.mockRestore();
  });
});

describe('RefreshLimiter', () => {
  afterEach(() => vi.useRealTimers());

  it('runs the first request at once and folds the rest into one trailing run', () => {
    vi.useFakeTimers();
    let now = 0;
    const task = vi.fn();
    const limiter = new RefreshLimiter(task, 1000, () => now);
    limiter.request();
    expect(task).toHaveBeenCalledTimes(1);
    now = 100;
    limiter.request();
    limiter.request();
    expect(task).toHaveBeenCalledTimes(1);
    now = 1000;
    vi.advanceTimersByTime(900);
    expect(task).toHaveBeenCalledTimes(2);
    now = 3000;
    limiter.request();
    expect(task).toHaveBeenCalledTimes(3);
  });

  it('cancels a pending run', () => {
    vi.useFakeTimers();
    const task = vi.fn();
    const limiter = new RefreshLimiter(task, 1000, () => 0);
    limiter.request();
    limiter.request();
    limiter.cancel();
    vi.advanceTimersByTime(5000);
    expect(task).toHaveBeenCalledTimes(1);
  });
});

describe('TauriHomeBackend', () => {
  it('calls home_summary with the request', async () => {
    const call = vi.fn(async () => ({ events: 1 }));
    const backend = new TauriHomeBackend(call as never);
    const request = { query: {} as never, buckets: 60 };
    await backend.querySummary(request);
    expect(call).toHaveBeenCalledWith('home_summary', { request });
  });
});

describe('HomeView', () => {
  const charts = new FakeECharts();

  function setup(findings: Finding[] | (() => Promise<Finding[]>)) {
    charts.reset();
    const answer = typeof findings === 'function' ? findings : async () => findings;
    const overview = vi.fn(async () => ({ result: await answer(), coverage: fullCoverage(3) }));
    const ctx = new FakeCtx();
    TestBed.configureTestingModule({ providers: [provideTinyBackend(), provideFakeDock(), charts.provider, { provide: AnalysisBackend, useFactory: () => new MockAnalysisBackend(recordsFrom(inject(ImtBackend))) }, { provide: ShareBackend, useValue: { overview } }, { provide: COMMAND_CONTEXT, useValue: ctx }] });
    const fixture = TestBed.createComponent(HomeView);
    return { fixture, overview, ctx };
  }

  afterEach(() => localStorage.removeItem(HOME_AUTO_OPEN_KEY));

  it('shows the tiles, the history chart and the findings with their actions', async () => {
    const { fixture, overview, ctx } = setup([crash, file]);
    await settle(fixture, 8);
    expect(overview).toHaveBeenCalledWith(expect.objectContaining({ text: '', filters: [] }));
    const t = text(fixture);
    expect(t).toContain('Events');
    expect(t).toContain('Exception groups');
    expect(t).toContain('1 file not opened');
    expect(t).toContain('Events over time');
    expect(charts.instances).toHaveLength(1);
    expect(t).toContain('Crash');
    expect(buttonByText(fixture, 'Jump to time')).toBeTruthy();
    expect(t).not.toContain('Nothing alarming found');
    // A click on the chart sets the range of the bucket under it; a drag sets the dragged range.
    const chart = charts.instances[0]!;
    const from = TestBed.inject(HomeStore).summary()!.from!;
    chart.convertFromPixel.mockReturnValue([from + 1]);
    chart.zrHandlers['click']?.({ offsetX: 1, offsetY: 1 });
    expect(ctx.calls.at(-1)).toMatch(/^query:\{"from":\d+,"to":\d+\}$/);
    chart.handlers['brushEnd']?.({ areas: [{ coordRange: [10, 20] }] });
    expect(ctx.calls.at(-1)).toBe('query:{"from":10,"to":20}');
  });

  it('renders only the visible findings however many the engine returns', async () => {
    const many = Array.from({ length: 5000 }, (_, i): Finding => ({ kind: 'rateChange', severity: 'info', title: `Rate change ${i}`, detail: `detail ${i}`, action: null }));
    const { fixture } = setup(many);
    await settle(fixture, 8);
    const rendered = fixture.nativeElement.querySelectorAll('.finding').length;
    expect(rendered).toBeGreaterThan(0);
    expect(rendered).toBeLessThan(100);
  });

  it('says when nothing alarming was found and refreshes', async () => {
    const { fixture, overview } = setup([]);
    await settle(fixture, 8);
    expect(text(fixture)).toContain('Nothing alarming found');
    const calls = overview.mock.calls.length;
    buttonByText(fixture, 'Refresh').click();
    await settle(fixture, 8);
    expect(overview.mock.calls.length).toBe(calls + 1);
  });

  it('shows an inline error with Retry for a failing health check, and a hint before anything is opened', async () => {
    let fail = true;
    const { fixture } = setup(async () => (fail ? Promise.reject(new Error('engine busy')) : [crash]));
    await settle(fixture, 8);
    expect(text(fixture)).toContain('engine busy');
    // The other tiles are unaffected by the failure.
    expect(text(fixture)).toContain('Events');
    fail = false;
    buttonByText(fixture, 'Retry').click();
    await settle(fixture, 8);
    expect(text(fixture)).toContain('Crash');
    const store = TestBed.inject(WorkspaceStore);
    store.applySummary({ ...store.summary()!, sources: [], unrecognized: [] });
    await settle(fixture, 8);
    expect(text(fixture)).toContain('Open files or a folder to see the dashboard here.');
  });

  it('runs the quick actions through the command registry, and Open last project needs a recent project', async () => {
    const { fixture } = setup([]);
    await settle(fixture, 8);
    const run = vi.spyOn(TestBed.inject(CommandRegistry), 'run');
    buttonByText(fixture, 'Open files…').click();
    expect(run).toHaveBeenCalledWith('file.openFiles', 'button');
    const last = fixture.nativeElement.querySelector('[data-testid=home-open-last]') as HTMLButtonElement;
    expect(last.disabled).toBe(true);
  });

  it('toggles opening after imports', async () => {
    const { fixture } = setup([]);
    await settle(fixture, 8);
    const box = fixture.nativeElement.querySelector('input[type=checkbox]') as HTMLInputElement;
    expect(box.checked).toBe(true);
    box.click();
    expect(TestBed.inject(HomePrefs).autoOpen()).toBe(false);
  });

  it('reads its numbers from the Home backend', async () => {
    const { fixture } = setup([]);
    const spy = vi.spyOn(TestBed.inject(HomeBackend), 'querySummary');
    await settle(fixture, 8);
    expect(spy).toHaveBeenCalledWith(expect.objectContaining({ buckets: 60 }));
  });
});

describe('HomeStore', () => {
  const charts = new FakeECharts();

  function setup(querySummary: HomeBackend['querySummary']) {
    charts.reset();
    TestBed.configureTestingModule({
      providers: [
        provideTinyBackend(),
        provideFakeDock(),
        charts.provider,
        { provide: AnalysisBackend, useFactory: () => new MockAnalysisBackend(recordsFrom(inject(ImtBackend))) },
        { provide: HomeBackend, useValue: { querySummary } },
        { provide: ShareBackend, useValue: { overview: async () => ({ result: [], coverage: fullCoverage(0) }) } },
        { provide: COMMAND_CONTEXT, useValue: new FakeCtx() },
      ],
    });
    const fixture = TestBed.createComponent(HomeView);
    return { fixture, store: TestBed.inject(HomeStore) };
  }

  const answer = (analyses: boolean): HomeSummary =>
    ({ sources: 1, events: 10, errors: 0, warnings: 0, from: null, to: null, bucketMs: 0, histogram: [], errorTrend: [], warningTrend: [], markers: [], anomalyCount: analyses ? 7 : 0, anomalies: [], exceptionGroups: 0, exceptionEvents: 0, analysed: analyses, coverage: fullCoverage(0) }) as HomeSummary;

  it('asks without the analyses first so the counts appear before the slow part, then with them', async () => {
    const calls: boolean[] = [];
    let release: () => void = () => undefined;
    const gate = new Promise<void>((r) => (release = r));
    const { fixture, store } = setup(async (request) => {
      calls.push(request.analyses ?? true);
      if (request.analyses) await gate;
      return answer(request.analyses ?? true);
    });
    await settle(fixture, 6);
    expect(calls.sort()).toEqual([false, true]);
    // The counts are on screen while the analyses still run; their tiles say so.
    expect(store.summary()?.analysed).toBe(false);
    expect(store.analysing()).toBe(true);
    expect(text(fixture)).toContain('10');
    expect(text(fixture)).toContain('…');
    release();
    await settle(fixture, 6);
    expect(store.summary()?.analysed).toBe(true);
    expect(store.analysing()).toBe(false);
    expect(text(fixture)).toContain('7');
  });

  it('keeps the quick summary when only the analyses fail, and shows an error when nothing arrived', async () => {
    const { fixture, store } = setup(async (request) => {
      if (request.analyses) throw new Error('analysis failed');
      return answer(false);
    });
    await settle(fixture, 6);
    expect(store.summary()?.events).toBe(10);
    expect(store.summaryError()).toBeNull();
    TestBed.resetTestingModule();
    const failing = setup(async () => {
      throw new Error('engine down');
    });
    await settle(failing.fixture, 6);
    expect(failing.store.summaryError()).toContain('engine down');
    expect(text(failing.fixture)).toContain('engine down');
  });
});

describe('provideHome', () => {
  afterEach(() => {
    localStorage.removeItem(HOME_AUTO_OPEN_KEY);
    localStorage.removeItem(LAYOUT_MODE_KEY);
  });

  it('does not open the view after an import in the Focused layout (the default)', async () => {
    const dock = new FakeDock();
    TestBed.configureTestingModule({ providers: [provideTinyBackend(), provideFakeDock(dock), provideHome()] });
    TestBed.inject(HomeAutoOpen);
    const store = TestBed.inject(WorkspaceStore);
    await settle();
    await store.openPaths(['/x.log']);
    await settle();
    expect(dock.calls).toEqual([]);
  });

  it('registers the view and command, and in the Full layout opens the view after each import unless turned off', async () => {
    const dock = new FakeDock();
    TestBed.configureTestingModule({ providers: [provideTinyBackend(), provideFakeDock(dock), provideHome()] });
    TestBed.inject(LayoutModeStore).set('full');
    TestBed.inject(HomeAutoOpen);
    const store = TestBed.inject(WorkspaceStore);
    await settle();
    const view = TestBed.inject(ViewRegistry).find('home');
    expect(view?.defaultPosition).toBe('right');
    expect(await (await import('../extensions/view-registry')).isViewAvailable(view!, store.summary())).toBe(true);
    expect(await view!.component()).toBe(HomeView);
    expect(dock.calls).toEqual([]);
    await store.openPaths(['/x.log']);
    await settle();
    expect(dock.calls).toEqual(['show:home']);
    TestBed.inject(HomePrefs).setAutoOpen(false);
    await store.openPaths(['/y.log']);
    await settle();
    expect(dock.calls).toEqual(['show:home']);
    expect(await TestBed.inject(CommandRegistry).run('view.home')).toBe(true);
    expect(dock.calls).toEqual(['show:home', 'show:home']);
  });
});

describe('HomeView tiles link to the views instead of repeating their lists', () => {
  it('opens Anomalies and Exceptions, and filters the events by level', async () => {
    const charts = new FakeECharts();
    const ctx = new FakeCtx();
    TestBed.configureTestingModule({
      providers: [provideTinyBackend(), provideFakeDock(), charts.provider, { provide: AnalysisBackend, useFactory: () => new MockAnalysisBackend(recordsFrom(inject(ImtBackend))) }, { provide: ShareBackend, useValue: { overview: async () => ({ result: [], coverage: fullCoverage(0) }) } }, { provide: COMMAND_CONTEXT, useValue: ctx }],
    });
    const fixture = TestBed.createComponent(HomeView);
    await settle(fixture, 8);
    const click = (id: string) => (fixture.nativeElement.querySelector(`[data-testid=${id}]`) as HTMLElement).click();
    click('home-tile-anomalies');
    click('home-tile-exceptions');
    click('home-tile-warnings');
    expect(ctx.calls).toEqual(['show:anomalies', 'show:exceptions', 'query:{"levels":["Warning"]}', 'show:events']);
  });
});
