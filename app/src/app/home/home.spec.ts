import { inject } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { buttonByText, FakeDock, provideFakeDock, provideTinyBackend, settle, text } from '../../testing/helpers';
import { FakeECharts } from '../../testing/fake-echarts';
import { AnalysisBackend } from '../core/analysis-backend';
import { Finding, fullCoverage, HomeSummary } from '../core/api.types';
import { DebriefBackend } from '../core/backend';
import { MockAnalysisBackend, recordsFrom } from '../mock-backends';
import { WorkspaceStore } from '../core/workspace.store';
import { COMMAND_CONTEXT, CommandRegistry } from '../extensions/command-registry';
import { CommandContext } from '../extensions/tokens';
import { ViewRegistry } from '../extensions/view-registry';
import { ShareBackend } from '../share/share-backend';
import { HealthFindingsStore } from './health/health-findings.store';
import { HealthHeadline } from './health/health-headline';
import { HomeBackend, TauriHomeBackend } from './home-backend';
import { HOME_AUTO_OPEN_KEY, HomePrefs } from './home-prefs';
import { ProjectBackend } from '../core/tools-backend';
import { ProjectService } from '../projects/project.service';
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
    prefs.setAutoOpen(true);
    expect(prefs.autoOpen()).toBe(true);
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
    TestBed.configureTestingModule({ providers: [provideTinyBackend(), provideFakeDock(), charts.provider, { provide: AnalysisBackend, useFactory: () => new MockAnalysisBackend(recordsFrom(inject(DebriefBackend))) }, { provide: ShareBackend, useValue: { overview } }, { provide: COMMAND_CONTEXT, useValue: ctx }] });
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
    expect(t).toContain('Needs attention');
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

  it('keeps Recent activity collapsed so the first screen fits, and puts attention beside the health check', async () => {
    const { fixture } = setup([crash, file]);
    await settle(fixture, 8);
    const feed = fixture.nativeElement.querySelector('details.feed') as HTMLDetailsElement;
    expect(feed.open).toBe(false);
    expect(feed.textContent).toContain('Recent activity');
    const pair = fixture.nativeElement.querySelector('.pair') as HTMLElement;
    expect(pair.querySelector('.attention')).toBeTruthy();
    expect(pair.querySelector('.health')).toBeTruthy();
  });

  it('shows a long unrecognised path shortened, with the full path as tooltip, and keeps a long finding title and detail reachable', async () => {
    const path = `C:\\ProgramData\\${'VeryLongVendorName\\'.repeat(8)}CompanyA.Graphic.Builder.Service.Diagnostics.2026-10-23.dmp`;
    const long: Finding = { kind: 'rateChange', severity: 'warning', title: 'x'.repeat(200), detail: `No importer reads ${path}`, action: null };
    const { fixture } = setup([long]);
    await settle(fixture, 8);
    const store = TestBed.inject(WorkspaceStore);
    store.applySummary({ ...store.summary()!, unrecognized: [{ path, name: 'CompanyA.Graphic.Builder.Service.Diagnostics.2026-10-23.dmp', reason: 'No importer' }] });
    await settle(fixture, 4);
    const row = fixture.nativeElement.querySelector('[data-testid=home-attention] .callout [data-testid=path-text]') as HTMLElement;
    expect(row.getAttribute('title')).toBe(path);
    expect(fixture.nativeElement.querySelector('[data-testid=home-attention] .callout button[aria-label="Copy the full path"]')).toBeTruthy();
    const finding = fixture.nativeElement.querySelector('.finding') as HTMLElement;
    expect(finding.querySelector('.title')!.getAttribute('title')).toBe('x'.repeat(200));
    expect(finding.querySelector('.detail')!.getAttribute('title')).toContain(path);
  });

  it('stacks the actions below the text of a finding when the Health column is narrow and gives the row room for them', async () => {
    const width = vi.spyOn(HTMLElement.prototype, 'clientWidth', 'get');
    const original = globalThis.ResizeObserver;
    globalThis.ResizeObserver = class {
      observe(): void {}
      unobserve(): void {}
      disconnect(): void {}
    } as unknown as typeof ResizeObserver;
    try {
      for (const [px, narrow, row] of [[900, false, '56px'], [320, true, '116px']] as const) {
        TestBed.resetTestingModule();
        width.mockReturnValue(px);
        const { fixture } = setup([crash]);
        await settle(fixture, 8);
        const host = fixture.nativeElement as HTMLElement;
        expect((host.querySelector('section.health') as HTMLElement).classList.contains('narrow')).toBe(narrow);
        expect((host.querySelector('.finding') as HTMLElement).style.height).toBe(row);
      }
    } finally {
      globalThis.ResizeObserver = original;
      width.mockRestore();
    }
  });

  it('asks the health check once when the timeline headline and Home both show it', async () => {
    const { fixture, overview } = setup([crash, file]);
    const headline = TestBed.createComponent(HealthHeadline);
    await settle(headline, 8);
    await settle(fixture, 8);
    expect(overview).toHaveBeenCalledTimes(1);
    expect(TestBed.inject(HomeStore).health.result()).toHaveLength(2);
    expect(text(headline)).toContain('crash');
    // Refresh still asks the engine again (once, for both).
    (fixture.nativeElement.querySelector('[data-testid=home-refresh]') as HTMLButtonElement).click();
    await settle(fixture, 8);
    expect(overview).toHaveBeenCalledTimes(2);
    expect(TestBed.inject(HomeStore).health).toBe(TestBed.inject(HealthFindingsStore).findings);
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
    (fixture.nativeElement.querySelector('[data-testid=home-refresh]') as HTMLButtonElement).click();
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
    // Without a recent project the shortcut is not offered at all (no disabled ghost button).
    expect(fixture.nativeElement.querySelector('[data-testid=home-open-last]')).toBeNull();
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
        { provide: AnalysisBackend, useFactory: () => new MockAnalysisBackend(recordsFrom(inject(DebriefBackend))) },
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
  });

  it('opens the view after an import by default, and after a project is opened (not a restored session)', async () => {
    const dock = new FakeDock();
    TestBed.configureTestingModule({ providers: [provideTinyBackend(), provideFakeDock(dock), { provide: ProjectBackend, useValue: {} }, provideHome()] });
    TestBed.inject(HomeAutoOpen);
    const store = TestBed.inject(WorkspaceStore);
    const projects = TestBed.inject(ProjectService);
    await settle();
    await store.openPaths(['/x.log']);
    await settle();
    expect(dock.calls).toEqual(['show:home']);
    projects.openedCount.update((n) => n + 1);
    await settle();
    expect(dock.calls).toEqual(['show:home', 'show:home']);
    TestBed.inject(HomePrefs).setAutoOpen(false);
    projects.openedCount.update((n) => n + 1);
    await settle();
    expect(dock.calls).toEqual(['show:home', 'show:home']);
  });

  it('registers the view (a document) and command, and opens the view after each import unless turned off', async () => {
    const dock = new FakeDock();
    TestBed.configureTestingModule({ providers: [provideTinyBackend(), provideFakeDock(dock), { provide: ProjectBackend, useValue: {} }, provideHome()] });
    TestBed.inject(HomeAutoOpen);
    const store = TestBed.inject(WorkspaceStore);
    await settle();
    const view = TestBed.inject(ViewRegistry).find('home');
    expect(view?.defaultPosition).toBe('center');
    expect(view?.layout?.area).toBe('document');
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
      providers: [provideTinyBackend(), provideFakeDock(), charts.provider, { provide: AnalysisBackend, useFactory: () => new MockAnalysisBackend(recordsFrom(inject(DebriefBackend))) }, { provide: ShareBackend, useValue: { overview: async () => ({ result: [], coverage: fullCoverage(0) }) } }, { provide: COMMAND_CONTEXT, useValue: ctx }],
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
