import { TestBed } from '@angular/core/testing';
import { FakeECharts } from '../../../testing/fake-echarts';
import { all, buttonByText, el, settle, text } from '../../../testing/helpers';
import { at, must } from '../../../testing/must';
import { ChartData, WorkspaceSummary } from '../../core/api.types';
import { WorkspaceStore } from '../../core/workspace.store';
import { DebriefBackend } from '../../core/backend';
import { ChartTheme, ChartTokens } from '../../metrics/chart-theme';
import { LifecycleService } from '../../lifecycle/lifecycle.service';
import { mockTools, provideToolsTesting } from '../mock/test-support';
import { confusable, contrastRatio, CVD_KINDS, deltaE, paletteProblems, worstCvdDelta } from './cvd';
import { MockMonitorCharts, mockMonitorChartsOptions, MockMonitorChartsMode, MOCK_PROCESSES, MOCK_RUN_PID_STEP } from './mock/mock-monitor-charts';
import { MONITOR_CHARTS } from './monitor-charts-backend';
import {
  assignSlots,
  endLabels,
  exactValue,
  humanValue,
  isAutoUnit,
  tooltipValue,
  hasMonitorSamples,
  MONITOR_METRICS,
  MONITOR_SOURCE_CONTEXT,
  monitorChartDef,
  monitorOption,
  monitorQuery,
  monitorSourceIds,
} from './monitor-charts-model';
import { MonitorChartsStore } from './monitor-charts.store';
import { MonitorLiveCharts } from './monitor-live-charts';
import { MonitorStore } from './monitor.store';

// eslint-disable-next-line @typescript-eslint/no-explicit-any -- ECharts options are deeply nested loose objects; tests probe them structurally.
type Opt = Record<string, any>;

/** Four distinct series colours for option tests (the theme's own tokens are checked in palette.spec.ts). */
const DARK = ['#1795fa', '#db551d', '#0a9068', '#b16ec0'];

const TOKENS: ChartTokens = {
  text: '#fff',
  muted: '#aaa',
  faint: '#888',
  grid: '#222',
  border: '#333',
  tooltipBg: '#111',
  accentSoft: 'rgba(0,0,255,.1)',
  accent: '#00f',
  series: DARK,
  other: '#5b6478',
  levels: {},
  markers: {},
  fontSans: 'sans',
  fontMono: 'mono',
};

const NOW = Date.UTC(2026, 9, 17, 12, 0, 0);

function chart(keys: string[], unit = '%'): ChartData {
  return {
    def: { ...monitorChartDef(at(MONITOR_METRICS, 0)), unit },
    from: 0,
    to: 60_000,
    bucketMs: 30_000,
    series: keys.map((key, i) => ({ key, points: [[0, i + 1.234], [30_000, null]] })),
    lifecycle: [],
  };
}

describe('cvd', () => {
  it('measures colour distance under simulated deficiencies like the palette validator', () => {
    expect(deltaE('#000000', '#000000')).toBe(0);
    expect(deltaE('#ffffff', '#000000')).toBeCloseTo(100, 0);
    // The validator reports this dark pair at ΔE 1.9 (protan).
    expect(must(deltaE('#9085e9', '#3987e5', 'protan'))).toBeCloseTo(1.9, 0);
    expect(deltaE('red', '#000000')).toBeNull();
    expect(worstCvdDelta('#000000', 'nope')).toBeNull();
    expect(confusable('#d95926', '#199e70')).toBe(false);
    expect(confusable('#9085e9', '#3987e5')).toBe(true);
    expect(confusable('bad', '#3987e5')).toBe(true);
    // Tritan only counts when asked for: blue vs a teal of similar lightness.
    expect(must(deltaE('#3987e5', '#199e70', 'tritan'))).toBeGreaterThan(0);
    expect(must(worstCvdDelta('#000000', '#ffffff', CVD_KINDS))).toBeGreaterThan(50);
  });

  it('measures WCAG contrast and reports every palette problem', () => {
    expect(must(contrastRatio('#000000', '#ffffff'))).toBeCloseTo(21, 5);
    expect(must(contrastRatio('#ffffff', '#000000'))).toBeCloseTo(21, 5);
    expect(contrastRatio('#fff', '#000000')).toBeNull();
    // The previous light palette: violet confusable with blue, the green too light for white.
    const old = paletteProblems(['#2a78d6', '#eb6834', '#1baf7a', '#6250d6'], ['#ffffff']);
    expect(old.some((p) => p.startsWith('#2a78d6 and #6250d6 are confusable'))).toBe(true);
    expect(old.some((p) => p.startsWith('#1baf7a on #ffffff has contrast'))).toBe(true);
    expect(paletteProblems(['#000000'], ['nope'])).toEqual(['#000000 on nope has contrast unreadable:1, below 3:1']);
    expect(paletteProblems(['#000000', '#ffffff'], [])).toEqual([]);
  });
});

describe('monitor charts model', () => {
  const summary = (paths: string[], fields: WorkspaceSummary['fields'] = []) =>
    ({ sources: paths.map((path, id) => ({ id, path, name: path.split(/[\\/]/).pop() })), fields }) as unknown as WorkspaceSummary;

  it('scopes the query to the monitor output and a live window the engine resolves', () => {
    const q = monitorQuery([3, 7], 15);
    expect(q.filters).toEqual([{ field: 'SourceContext', op: 'eq', value: MONITOR_SOURCE_CONTEXT }]);
    // Scoped by source id, not by the (repeating) file name: no instance grouping involved.
    expect(q.sources).toEqual([3, 7]);
    expect(q).toMatchObject({ groupBy: [], groups: [] });
    expect(q.window).toEqual({ kind: 'live', lastMs: 900_000 });
    expect(monitorQuery([], 5).sources).toEqual([]);
    expect(monitorChartDef(at(MONITOR_METRICS, 0))).toMatchObject({ y: 'avg(CpuPercent)', splitBy: ['ProcessName', 'ProcessId'], type: 'line', unit: '%' });
  });

  it('finds the sources inside the output folder by id and notices opened monitor output', () => {
    const s = summary(['C:\\out\\run\\process-monitor.clef', '/other/x.log', '/out/run2/process-monitor.clef', '/out/run']);
    expect(monitorSourceIds(s, 'c:/out/run/')).toEqual([0]);
    expect(monitorSourceIds(s, 'C:\\OUT\\run')).toEqual([0]);
    // The same file name in another run's folder is another source id, never mixed in.
    expect(monitorSourceIds(s, '/out/run2')).toEqual([2]);
    expect(monitorSourceIds(s, '/out/run')).toEqual([3]);
    expect(monitorSourceIds(s, null)).toEqual([]);
    expect(monitorSourceIds(null, '/out')).toEqual([]);
    expect(hasMonitorSamples(null)).toBe(false);
    expect(hasMonitorSamples(summary([], [{ name: 'CpuPercent', count: 2, distinct: 2, numeric: true, derived: false }]))).toBe(true);
    expect(hasMonitorSamples(summary([], [{ name: 'CpuPercent', count: 0, distinct: 0, numeric: true, derived: false }]))).toBe(false);
  });

  it('keeps palette slots with the process, assigns new ones in order and stops at the slot count', () => {
    const first = assignSlots({}, [chart(['b', 'a']), null], 4);
    expect(first).toEqual({ b: 0, a: 1 });
    expect(assignSlots(first, [chart(['c', 'a', 'Other', 'd', 'e'])], 4)).toEqual({ b: 0, a: 1, c: 2, d: 3 });
    expect(assignSlots({ a: 0 }, [chart(['x', 'y'])], 1)).toEqual({ a: 0 });
  });

  it('labels line ends with the process name unless two instances share it', () => {
    const labels = endLabels(['svc.exe · 1', 'svc.exe · 2', 'db.exe · 3', 'Other']);
    expect([...labels.values()]).toEqual(['svc.exe · 1', 'svc.exe · 2', 'db.exe', 'Other']);
  });

  it('formats exact tooltip values with the unit', () => {
    expect(exactValue(12345.678, 'MB')).toBe('12,345.68 MB');
    expect(exactValue(3, '')).toBe('3');
    expect(exactValue(null, '%')).toBe('—');
    expect(exactValue(Number.NaN, '%')).toBe('—');
  });

  it('humanizes values by unit for axes and tooltips', () => {
    expect(humanValue(512, 'MB')).toBe('512 MB');
    expect(humanValue(1536, 'MB')).toBe('1.5 GB');
    expect(humanValue(2048, 'B')).toBe('2 KB');
    expect(humanValue(5 * 1024 ** 2, 'B/s')).toBe('5 MB/s');
    expect(humanValue(0, 'B/s')).toBe('0 bytes/s');
    expect(humanValue(42.46, '%')).toBe('42.5%');
    expect(humanValue(1500, '/s')).toBe('1500/s');
    expect(humanValue(25_000, '')).toBe('25k');
    expect(humanValue(null, 'MB')).toBe('—');
    expect(tooltipValue(1536, 'MB')).toBe('1.5 GB (1,536 MB)');
    expect(tooltipValue(7, '%')).toBe('7%');
    expect(['MB', 'B', 'B/s', '%'].every(isAutoUnit)).toBe(true);
    expect(['/s', ''].some(isAutoUnit)).toBe(false);
  });

  it('builds a small multiple: colours follow the slot, end labels shift apart, the shared legend toggles', () => {
    const o = monitorOption({ data: chart(['a', 'b', 'c']), tokens: TOKENS, tz: 'utc', slots: { c: 0, a: 3 }, hidden: new Set(['a']) }) as Opt;
    const series = o['series'] as Opt[];
    expect(series.map((s) => s['name'])).toEqual(['c', 'a', 'b']);
    expect(at(series, 0)['color']).toBe(DARK[0]);
    expect(at(series, 1)['color']).toBe(DARK[3]);
    // The palette is CVD-safe for every pair: lines stay solid.
    expect(series.every((s) => s['lineStyle']?.['type'] === undefined)).toBe(true);
    expect(at(series, 0)['endLabel']['show']).toBe(true);
    expect(at(series, 0)['endLabel']['formatter']).toBe('c');
    expect(at(series, 0)['labelLayout']).toEqual({ moveOverlap: 'shiftY', hideOverlap: true });
    expect(o['legend']['show']).toBe(false);
    expect(o['legend']['selected']).toEqual({ c: true, a: false, b: true });
    expect(o['tooltip']['trigger']).toBe('axis');
    expect(o['tooltip']['valueFormatter'](1.234)).toBe('1.2%');
    expect(o['grid']['top']).toBe(12);
    expect(o['yAxis']['axisLabel']['formatter'](42.46)).toBe('42.5%');
  });
});

describe('MockMonitorCharts', () => {
  const mock = (mode: MockMonitorChartsMode = 'data') => new MockMonitorCharts({ mode, now: () => NOW, intervalMs: 2000, dotnet: () => dotnet });
  let dotnet = false;
  const request = (field: string, lastMs = 300_000) => ({
    query: { ...monitorQuery([], 5), window: { kind: 'live' as const, lastMs } },
    def: { ...monitorChartDef(at(MONITOR_METRICS, 0)), y: `avg(${field})` },
    buckets: 150,
    chartId: null,
  });

  it('reads its mode from the URL', () => {
    expect(mockMonitorChartsOptions('?mock-monitor-charts=empty').mode).toBe('empty');
    expect(mockMonitorChartsOptions('?mock-monitor-charts=fail').mode).toBe('fail');
    expect(mockMonitorChartsOptions('').mode).toBe('data');
    expect(mockMonitorChartsOptions('').now()).toBeGreaterThan(0);
  });

  it('answers avg(field) per process over the live window', async () => {
    const m = mock();
    for (const f of ['CpuPercent', 'WorkingSetMb', 'PrivateMb', 'Handles', 'Threads', 'ReadBytesPerSec', 'WriteBytesPerSec']) {
      const d = await m.queryChart(request(f));
      expect(d.series.map((s) => s.key)).toEqual(MOCK_PROCESSES.map((p) => p.key));
      expect(d.to).toBe(NOW);
      expect(d.from).toBe(NOW - 300_000);
      expect(d.bucketMs).toBe(2000);
    }
    expect((await m.queryChart(request('Nope'))).series).toEqual([]);
    expect((await m.queryChart(request('CpuPercent', 3_600_000))).bucketMs).toBe(24_000);
    const noWindow = { ...request('CpuPercent'), query: { ...monitorQuery([], 5), window: null } };
    expect((await m.queryChart(noWindow)).from).toBe(NOW - 300_000);
    expect(m.requests).toHaveLength(10);
    await expect(m.queryChart({ ...request('x'), def: null })).rejects.toThrow('needs chartId');
    await expect(m.queryChart({ ...request('x'), def: { ...monitorChartDef(at(MONITOR_METRICS, 0)), y: 'count()' } })).rejects.toThrow('unsupported');
  });

  it('has .NET runtime samples only for .NET processes of a run that asked for them', async () => {
    const m = mock();
    dotnet = false;
    expect((await m.queryChart(request('DotnetHeapBytes'))).series).toEqual([]);
    dotnet = true;
    for (const f of ['DotnetHeapBytes', 'DotnetTimeInGcPercent', 'DotnetExceptionsPerSec', 'DotnetContentionPerSec']) {
      const d = await m.queryChart(request(f));
      expect(d.series.map((s) => s.key)).toEqual(['OrderService.exe · 4120', 'Worker.exe · 5232']);
      expect(d.series.every((s) => s.points.every(([, v]) => v !== null && v >= 0))).toBe(true);
    }
    expect((await m.queryChart(request('CpuPercent'))).series).toHaveLength(3);
    dotnet = false;
    expect(mockMonitorChartsOptions('').dotnet()).toBe(false);
  });

  it('answers only the requested sources, one run per source id', async () => {
    const m = mock();
    const keys = async (sources: number[]) => (await m.queryChart({ ...request('CpuPercent'), query: { ...monitorQuery(sources, 5) } })).series.map((s) => s.key);
    const first = MOCK_PROCESSES.map((p) => p.key);
    expect(await keys([9])).toEqual(first);
    const second = MOCK_PROCESSES.map((p) => `${p.name} · ${p.pid + MOCK_RUN_PID_STEP}`);
    expect(await keys([4])).toEqual(second);
    expect(await keys([9, 4, 9])).toEqual([...first, ...second]);
    expect(await keys([])).toEqual(first);
  });

  it('can be empty, or fail one panel load', async () => {
    expect((await mock('empty').queryChart(request('CpuPercent'))).series).toEqual([]);
    const failing = mock('fail');
    for (let i = 0; i < MONITOR_METRICS.length; i++) await expect(failing.queryChart(request('CpuPercent'))).rejects.toThrow('locked');
    expect((await failing.queryChart(request('CpuPercent'))).series.length).toBe(3);
  });
});

const charts = new FakeECharts();
const PROCESS_METRICS = MONITOR_METRICS.filter((m) => m.group === 'process').length;

async function setup(mode: MockMonitorChartsMode = 'data') {
  charts.reset();
  const backend = new MockMonitorCharts({ mode, now: () => NOW, intervalMs: 2000, dotnet: () => mockTools().monitorConfig?.dotnet === true });
  TestBed.configureTestingModule({ providers: [...provideToolsTesting(), charts.provider, { provide: MONITOR_CHARTS, useValue: backend }] });
  const theme = TestBed.inject(ChartTheme);
  theme.tokens.set(TOKENS);
  await TestBed.inject(LifecycleService).init();
  const fixture = TestBed.createComponent(MonitorLiveCharts);
  await settle(fixture);
  const monitor = TestBed.inject(MonitorStore);
  const store = fixture.debugElement.injector.get(MonitorChartsStore);
  const start = async (dotnet = false) => {
    monitor.setTarget(0, { value: 'svc*' });
    monitor.patchDraft({ dotnet });
    await monitor.start();
    await settle(fixture);
  };
  return { fixture, backend, monitor, store, start, workspace: TestBed.inject(WorkspaceStore) };
}

describe('MonitorLiveCharts', () => {
  it('is hidden until the monitor records, then charts every metric per process', async () => {
    const { fixture, backend, start, store, workspace } = await setup();
    workspace.setTimezone('utc'); // the asserted wall-clock text must not depend on the host's zone
    expect(fixture.nativeElement.querySelector('[data-testid=monitor-live]')).toBeNull();
    expect(backend.requests).toHaveLength(0);

    await start();
    expect(el(fixture, '[data-testid=monitor-live]')).toBeTruthy();
    // Without .NET samples only the process charts show.
    expect(all(fixture, 'figure.card')).toHaveLength(PROCESS_METRICS);
    expect(charts.instances.length).toBe(PROCESS_METRICS);
    expect(fixture.nativeElement.querySelector('[data-group=dotnet]')).toBeNull();
    expect(fixture.nativeElement.querySelector('.group-title')).toBeNull();
    // Every request is scoped to the run's output source and the default 5 minute window.
    const last = backend.requests.slice(-MONITOR_METRICS.length);
    expect(last.map((r) => r.def?.y)).toEqual(MONITOR_METRICS.map((m) => `avg(${m.field})`));
    expect(at(last, 0).query.sources).toEqual(store.sources());
    expect(at(last, 0).query.groups).toEqual([]);
    expect(store.sources().length).toBe(1);
    expect(at(last, 0).query.window).toEqual({ kind: 'live', lastMs: 300_000 });
    expect(text(fixture)).toContain('OrderService.exe · 4120');
    expect(el(fixture, '[data-testid=monitor-live-until]').textContent).toContain('17 Oct 12:00:00');
    const option = at(charts.instances, 0).setOption.mock.calls.at(-1)?.[0] as Opt;
    expect((option['series'] as Opt[]).map((s) => s['name'])).toEqual(MOCK_PROCESSES.map((p) => p.key));
  });

  it('switches the window, pauses and follows', async () => {
    const { fixture, backend, start, store, workspace } = await setup();
    await start();
    buttonByText(fixture, '60 min').click();
    await settle(fixture);
    expect(store.windowMin()).toBe(60);
    expect(at(backend.requests, -1).query.window).toEqual({ kind: 'live', lastMs: 3_600_000 });
    expect(buttonByText(fixture, '60 min').getAttribute('aria-pressed')).toBe('true');

    const follow = el<HTMLButtonElement>(fixture, '[data-testid=monitor-follow]');
    expect(follow.getAttribute('aria-pressed')).toBe('true');
    follow.click();
    await settle(fixture);
    expect(follow.textContent).toContain('Paused');
    const before = backend.requests.length;
    const s = must(workspace.summary());
    workspace.applySummary({ ...s });
    await settle(fixture);
    expect(backend.requests.length).toBe(before);

    follow.click();
    await settle(fixture);
    expect(follow.textContent).toContain('Following');
    expect(backend.requests.length).toBe(before + MONITOR_METRICS.length);
    workspace.applySummary({ ...s });
    await settle(fixture);
    expect(backend.requests.length).toBe(before + 2 * MONITOR_METRICS.length);
  });

  it('toggles a process in every chart from the shared legend', async () => {
    const { fixture, start } = await setup();
    await start();
    const key = buttonByText(fixture, /Worker\.exe/);
    expect(key.getAttribute('aria-pressed')).toBe('true');
    key.click();
    await settle(fixture);
    expect(key.getAttribute('aria-pressed')).toBe('false');
    for (const c of charts.instances) {
      const o = c.setOption.mock.calls.at(-1)?.[0] as Opt;
      expect(o['legend']['selected']['Worker.exe · 5232']).toBe(false);
    }
    key.click();
    await settle(fixture);
    expect(((at(charts.instances, 0).setOption.mock.calls.at(-1)?.[0] as Opt)['legend']['selected'] as Opt)['Worker.exe · 5232']).toBe(true);
  });

  it('charts only the current run when an earlier run is still open, with fresh colour slots', async () => {
    const { fixture, backend, monitor, start, store } = await setup();
    await start();
    const first = store.sources();
    await monitor.stop();
    await start();
    expect(store.sources()).toHaveLength(1);
    expect(store.sources()).not.toEqual(first);
    expect(at(backend.requests, -1).query.sources).toEqual(store.sources());
    const second = MOCK_PROCESSES.map((p) => `${p.name} · ${p.pid + MOCK_RUN_PID_STEP}`);
    expect(store.legend().map((l) => l.key)).toEqual(second);
    expect(store.legend().map((l) => l.slot)).toEqual([0, 1, 2]);
    expect(text(fixture)).not.toContain('OrderService.exe · 4120');
  });

  it('shows an empty state before the first samples', async () => {
    const { fixture, start } = await setup('empty');
    await start();
    expect(el(fixture, '[data-testid=monitor-live-empty]').textContent).toContain('No samples in the last 5 minutes');
  });

  it('shows a failed load inline and retries', async () => {
    const { fixture, start, backend, workspace } = await setup('fail');
    await start();
    expect(el(fixture, '[data-testid=monitor-live-error]').textContent).toContain('locked by another process');
    // New data does not retry behind the user's back.
    const before = backend.requests.length;
    workspace.applySummary({ ...must(workspace.summary()) });
    await settle(fixture);
    expect(backend.requests.length).toBe(before);
    buttonByText(fixture, 'Retry').click();
    await settle(fixture);
    expect(fixture.nativeElement.querySelector('[data-testid=monitor-live-error]')).toBeNull();
    expect(all(fixture, 'figure.card')).toHaveLength(PROCESS_METRICS);
  });

  it('shows the loading state, a per-chart gap, the Other entry, and keeps the newest load', async () => {
    const { fixture, backend, start, store } = await setup();
    let release: () => void = () => undefined;
    const gate = new Promise<void>((r) => (release = r));
    const real = backend.queryChart.bind(backend);
    const spy = vi.spyOn(backend, 'queryChart').mockImplementation(async (r) => {
      await gate;
      const d = await real(r);
      if (r.def?.y === 'avg(Handles)') return { ...d, series: [] };
      return r.def?.y === 'avg(CpuPercent)' ? { ...d, series: [...d.series, ...['p4', 'p5'].map((key) => ({ key, points: d.series[0]?.points ?? [] }))] } : d;
    });
    await start();
    expect(el(fixture, '[data-testid=monitor-live-loading]').textContent).toContain('Loading');
    release();
    await settle(fixture);
    expect(text(fixture)).toContain('Not recorded for these processes');
    expect(store.legend().map((l) => l.key)).toEqual([...MOCK_PROCESSES.map((p) => p.key), 'p4', 'Other']);
    expect(all(fixture, '.legend line').at(-1)?.getAttribute('stroke')).toBe(TOKENS.other);
    expect(all(fixture, '.legend line').every((l) => !l.hasAttribute('stroke-dasharray'))).toBe(true);
    spy.mockRestore();

    // A slow older load does not overwrite a newer one.
    let slow: () => void = () => undefined;
    const slowGate = new Promise<void>((r) => (slow = r));
    vi.spyOn(backend, 'queryChart').mockImplementationOnce(async () => {
      await slowGate;
      throw new Error('stale');
    });
    const older = store.load();
    await store.load();
    slow();
    await older;
    expect(store.error()).toBeNull();
  });

  it('adds a .NET runtime group when the run sampled runtime counters', async () => {
    const { fixture, start } = await setup();
    await start(true);
    const group = el(fixture, '[data-group=dotnet]');
    expect(group.getAttribute('aria-label')).toBe('.NET runtime');
    expect([...group.querySelectorAll('figure')].map((f) => f.getAttribute('data-metric'))).toEqual(['dotnet-heap', 'dotnet-gc-time', 'dotnet-exceptions', 'dotnet-contention']);
    expect(all(fixture, '.group-title').map((h) => h.textContent)).toEqual(['Process', '.NET runtime']);
    expect(charts.instances.length).toBe(MONITOR_METRICS.length);
    const heap = at(charts.instances, -4).setOption.mock.calls.at(-1)?.[0] as Opt;
    expect((heap['series'] as Opt[]).map((s) => s['name'])).toEqual(['OrderService.exe · 4120', 'Worker.exe · 5232']);
    expect(heap['tooltip']['valueFormatter'](1048576)).toBe('1 MB (1,048,576 B)');
  });

  it('appears for an opened monitor output without a run', async () => {
    const { fixture, workspace } = await setup();
    const s = must(workspace.summary());
    workspace.applySummary({ ...s, fields: [...s.fields, { name: 'CpuPercent', count: 4, distinct: 4, numeric: true, derived: false }] });
    await settle(fixture);
    expect(el(fixture, '[data-testid=monitor-live]')).toBeTruthy();
  });
});

describe('MONITOR_CHARTS', () => {
  afterEach(() => delete (window as unknown as Record<string, unknown>)['__TAURI_INTERNALS__']);

  it('asks the engine inside Tauri and the mock in a plain browser', async () => {
    TestBed.configureTestingModule({ providers: provideToolsTesting() });
    expect(TestBed.inject(MONITOR_CHARTS)).toBeInstanceOf(MockMonitorCharts);
    TestBed.resetTestingModule();

    (window as unknown as Record<string, unknown>)['__TAURI_INTERNALS__'] = {};
    TestBed.configureTestingModule({ providers: provideToolsTesting() });
    const engine = TestBed.inject(DebriefBackend);
    const spy = vi.spyOn(engine, 'queryChart');
    const request = { query: monitorQuery([], 5), def: monitorChartDef(at(MONITOR_METRICS, 0)), chartId: null, buckets: 10 };
    const port = TestBed.inject(MONITOR_CHARTS);
    expect(port).not.toBeInstanceOf(MockMonitorCharts);
    await port.queryChart(request);
    expect(spy).toHaveBeenCalledWith(request);
  });
});
