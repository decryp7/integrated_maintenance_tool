import { Component, signal } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { FakeECharts } from '../../testing/fake-echarts';
import type { EChartsCoreOption } from 'echarts/core';
import { all, buttonByText, el, FakeDock, provideFakeDock, provideTinyBackend, settle, text, tinyBackend, typeInto } from '../../testing/helpers';
import { ChartData, ChartDef } from '../core/api.types';
import { WorkspaceStore } from '../core/workspace.store';
import { buildOption, ChartCard, LIVE_UPDATE_MS, liveOption } from './chart-card';
import { ImtBackend } from '../core/backend';
import { LiveTail } from '../workspace/live-tail';
import { ChartTheme, ChartTokens } from './chart-theme';
import { EChart } from './echart';
import { MetricsView } from './metrics-view';
import { TimeCursor } from '../analysis/time-cursor';
import { at, must } from '../../testing/must';

// Through DI, not vi.mock('echarts/core'): with shared module graphs (isolate: false) a module mock only applies
// when no earlier spec in the worker has loaded echarts, which made these tests flaky.
const charts = new FakeECharts();
beforeEach(() => {
  charts.reset();
  TestBed.configureTestingModule({ providers: [charts.provider] });
});

const TOKENS: ChartTokens = {
  text: '#fff',
  muted: '#aaa',
  faint: '#888',
  grid: '#222',
  border: '#333',
  tooltipBg: '#111',
  accentSoft: 'rgba(0,0,255,.1)',
  accent: '#00f',
  series: ['#1', '#2', '#3', '#4'],
  other: '#o',
  levels: { Verbose: '#v', Debug: '#d', Information: '#i', Warning: '#w', Error: '#e', Fatal: '#f' },
  markers: { start: '#s', stop: '#x', fatal: '#f' },
  fontSans: 'sans',
  fontMono: 'mono',
};

const def = (patch: Partial<ChartDef> = {}): ChartDef => ({ id: 'c', title: 'C', y: 'avg(cpu)', bucket: '1m', splitBy: ['MachineName'], type: 'line', unit: '%', ...patch });
const data = (keys: string[], patch: Partial<ChartDef> = {}): ChartData => ({
  def: def(patch),
  from: 0,
  to: 120_000,
  bucketMs: 60_000,
  series: keys.map((key, i) => ({ key, points: [[0, i + 1], [60_000, null]] })),
  lifecycle: [
    { kind: 'fatal', ts: 30_000, group: 'A', id: 1 },
    { kind: 'custom', ts: 40_000, group: 'B', id: 2 },
  ],
});

// eslint-disable-next-line @typescript-eslint/no-explicit-any -- ECharts options are deeply nested loose objects; tests probe them structurally.
type Opt = Record<string, any>;

describe('buildOption', () => {
  it('shifts overlapping end labels and wraps the legend with headroom', () => {
    const few = buildOption(data(['a', 'b']), TOKENS, 'utc', new Map()) as Opt;
    expect(few['series'][0]['labelLayout']).toEqual({ moveOverlap: 'shiftY' });
    expect(few['legend'].type).toBe('plain');
    const many = buildOption(data(['a', 'b', 'c', 'd', 'e', 'f', 'g', 'h', 'i']), TOKENS, 'utc', new Map()) as Opt;
    expect(many['legend'].type).toBe('plain');
    expect(many['grid'].top).toBeGreaterThan(few['grid'].top);
  });

  it('colours up to four series and labels line ends', () => {
    const o = buildOption(data(['a', 'b']), TOKENS, 'utc', new Map([['fatal', 'Fatal']])) as Opt;
    expect(o['series'].map((s: Opt) => s['color'])).toEqual(['#1', '#2']);
    expect(o['series'][0]['endLabel'].show).toBe(true);
    expect(o['series'][0]['markLine'].data).toHaveLength(2);
    expect(o['series'][0]['markLine'].data[0]).toMatchObject({ xAxis: 30_000, name: 'Fatal · A', lineStyle: { color: '#f' } });
    expect(o['series'][0]['markLine'].data[1].lineStyle.color).toBe('#aaa');
    expect(o['series'][1]['markLine']).toBeUndefined();
    expect(o['xAxis']).toMatchObject({ type: 'time', min: 0, max: 120_000 });
    expect(o['grid'].right).toBe(130);
    expect(o['tooltip'].valueFormatter(12.345)).toBe('12.3 %');
    expect(o['tooltip'].valueFormatter(null)).toBe('—');
    expect(o['xAxis'].axisLabel.formatter(Date.UTC(2026, 9, 14, 10, 41))).toBe('10:41');
    expect(o['xAxis'].axisPointer.label.formatter({ value: Date.UTC(2026, 9, 14, 10, 41) })).toBe('14 Oct 10:41:00.000');
    expect(o['yAxis'].axisLabel.formatter(2500)).toBe('2500');
  });

  it('folds extra line series into thin neutral "Other" lines', () => {
    const o = buildOption(data(['a', 'b', 'c', 'd', 'e', 'f']), TOKENS, 'utc', new Map()) as Opt;
    const s = o['series'] as Opt[];
    expect(s.map((x) => x['name'])).toEqual(['a', 'b', 'c', 'd', 'Other', 'Other']);
    expect(at(s, 4)['color']).toBe('#o');
    expect(at(s, 4)['lineStyle'].width).toBe(1);
    expect(at(s, 4)['endLabel']).toBeUndefined();
  });

  it('sums extra bar series into one "Other" series', () => {
    const o = buildOption(data(['a', 'b', 'c', 'd', 'e', 'f'], { type: 'stacked', unit: '' }), TOKENS, 'utc', new Map()) as Opt;
    const s = o['series'] as Opt[];
    expect(s.map((x) => x['name'])).toEqual(['a', 'b', 'c', 'd', 'Other (2)']);
    expect(at(s, 4)['data']).toEqual([
      [0, 11],
      [60_000, null],
    ]);
    expect(at(s, 0)['stack']).toBe('total');
    expect(at(s, 0)['type']).toBe('bar');
    expect(o['grid'].right).toBe(20);
    expect(o['tooltip'].valueFormatter(3)).toBe('3');
  });

  it('uses level colours for split-by-level charts in level order', () => {
    const o = buildOption(data(['Error', 'Information', 'Nope'], { splitBy: ['@level'], type: 'bar' }), TOKENS, 'local', new Map()) as Opt;
    expect(o['series'].map((s: Opt) => [s['name'], s['color']])).toEqual([
      ['Information', '#i'],
      ['Error', '#e'],
    ]);
    expect(o['series'][0]['itemStyle'].borderRadius).toEqual([2, 2, 0, 0]);
  });
});

@Component({ selector: 'imt-host', imports: [EChart], template: `<imt-echart [options]="opts()" (brushed)="got = $event" />` })
class Host {
  readonly opts = signal<EChartsCoreOption>({ series: [] });
  got: [number, number] | null = null;
}

describe('EChart', () => {
  beforeEach(() => (charts.instances.length = 0));

  it('initialises, updates, reports brushes and disposes', async () => {
    const fixture = TestBed.createComponent(Host);
    await settle(fixture);
    const c = at(charts.instances, 0);
    expect(c.setOption).toHaveBeenCalledWith({ series: [] }, { notMerge: true });
    expect(c.dispatchAction).toHaveBeenCalledWith(expect.objectContaining({ type: 'takeGlobalCursor' }));
    fixture.componentInstance.opts.set({ series: [{}] });
    await settle(fixture);
    expect(c.setOption).toHaveBeenLastCalledWith({ series: [{}] }, { notMerge: true });
    must(c.handlers['brushEnd'])({ areas: [{ coordRange: [10.4, 20.2] }] });
    expect(fixture.componentInstance.got).toEqual([10, 21]);
    fixture.componentInstance.got = null;
    must(c.handlers['brushEnd'])({ areas: [] });
    must(c.handlers['brushEnd'])({ areas: [{ coordRange: [5, 5] }] });
    expect(fixture.componentInstance.got).toBeNull();
    expect(c.resize).toHaveBeenCalled();
    fixture.destroy();
    expect(c.dispose).toHaveBeenCalled();
  });
});

describe('ChartTheme', () => {
  it('reads tokens and refreshes on theme changes', async () => {
    const theme = TestBed.inject(ChartTheme);
    expect(theme.tokens().series).toHaveLength(4);
    const before = theme.tokens();
    document.documentElement.setAttribute('data-theme', 'light');
    await new Promise((r) => setTimeout(r, 40));
    expect(theme.tokens()).not.toBe(before);
    document.documentElement.removeAttribute('data-theme');
    TestBed.resetTestingModule();
  });
});

describe('ChartCard', () => {
  beforeEach(() => (charts.instances.length = 0));

  async function render(d: ChartDef, fromProfile = true, backend = tinyBackend()) {
    TestBed.configureTestingModule({ providers: [provideTinyBackend(backend), charts.provider] });
    const fixture = TestBed.createComponent(ChartCard);
    fixture.componentRef.setInput('def', d);
    fixture.componentRef.setInput('fromProfile', fromProfile);
    await settle(fixture);
    return { fixture, store: TestBed.inject(WorkspaceStore) };
  }

  it('queries profile charts and renders them', async () => {
    const { fixture } = await render(def({ id: 'cpu', y: 'avg(cpu)', splitBy: ['MachineName', 'ProcessId'] }));
    expect(text(fixture)).toContain('avg(cpu) (%) · per 1 min · split by MachineName, ProcessId');
    expect(charts.instances).toHaveLength(1);
    expect(text(fixture)).toContain('Fatal');
  });

  it('shows empty and error states', async () => {
    const empty = await render(def({ id: 'x', y: 'avg(nothing)', splitBy: [] }), false);
    expect(text(empty.fixture)).toContain('No numeric values for avg(nothing)');
    TestBed.resetTestingModule();
    const failing = await render(def({ id: 'missing' }));
    expect(el(failing.fixture, '.err').textContent).toMatch(/Unknown chart/);
  });

  it('zooms the shared range from a brush and can be removed', async () => {
    const { fixture, store } = await render(def({ id: 'x', y: 'count()', splitBy: [], bucket: 'auto' }), false);
    fixture.componentRef.setInput('removable', true);
    let removed = false;
    fixture.componentInstance.remove.subscribe(() => (removed = true));
    await settle(fixture);
    const t0 = store.summary()!.from!;
    must(at(charts.instances, 0).handlers['brushEnd'])({ areas: [{ coordRange: [t0 + 1000, t0 + 60_000] }] });
    expect(store.query().from).toBe(t0 + 1000);
    expect(TestBed.inject(TimeCursor).selection()).toEqual({ from: t0 + 1000, to: t0 + 60_000 });
    el<HTMLButtonElement>(fixture, '[aria-label="Remove chart C"]').click();
    expect(removed).toBe(true);
  });
});

describe('ChartCard time cursor', () => {
  beforeEach(() => (charts.instances.length = 0));

  it('publishes the hovered time of the plot box and draws the shared cursor line', async () => {
    TestBed.configureTestingModule({ providers: [provideTinyBackend()] });
    const fixture = TestBed.createComponent(ChartCard);
    fixture.componentRef.setInput('def', def({ id: 'x', y: 'count()', splitBy: [], bucket: 'auto', type: 'bar' }));
    fixture.componentRef.setInput('fromProfile', false);
    await settle(fixture);
    const cursor = TestBed.inject(TimeCursor);
    expect(fixture.nativeElement.querySelector('.hover-line')).toBeNull();

    const plot = el<HTMLElement>(fixture, '.plot');
    plot.getBoundingClientRect = () => ({ left: 0, width: 56 + 20 + 400 }) as DOMRect;
    plot.dispatchEvent(new PointerEvent('pointermove', { clientX: 56 + 200, bubbles: true }));
    const hovered = cursor.hovered()!;
    expect(hovered).not.toBeNull();
    await settle(fixture);
    const line = el<HTMLElement>(fixture, '.hover-line');
    expect(line.style.left).toContain('56px');
    expect(line.style.left).toContain('0.5');
    plot.dispatchEvent(new PointerEvent('pointermove', { clientX: 10, bubbles: true }));
    expect(cursor.hovered()).toBeNull();

    // A time hovered elsewhere (e.g. the timeline) draws the line; one outside the axis hides it.
    cursor.hover(hovered);
    await settle(fixture);
    expect(fixture.nativeElement.querySelector('.hover-line')).not.toBeNull();
    cursor.hover(0);
    await settle(fixture);
    expect(fixture.nativeElement.querySelector('.hover-line')).toBeNull();
  });
});

describe('MetricsView', () => {
  let dock: FakeDock;
  async function render() {
    dock = new FakeDock();
    TestBed.configureTestingModule({ providers: [provideTinyBackend(), provideFakeDock(dock)] });
    const fixture = TestBed.createComponent(MetricsView);
    await settle(fixture);
    return { fixture, store: TestBed.inject(WorkspaceStore) };
  }

  it('renders profile charts and the range', async () => {
    const { fixture, store } = await render();
    expect(all(fixture, 'imt-chart-card')).toHaveLength(3);
    expect(text(fixture)).toContain('3 charts');
    expect(text(fixture)).toContain('UTC');
    store.setRange(store.summary()!.from!, store.summary()!.from! + 60_000);
    await settle(fixture);
    buttonByText(fixture, /Reset zoom/).click();
    expect(store.hasRange()).toBe(false);
    buttonByText(fixture, 'Edit charts in profile').click();
    expect(dock.calls).toContain('show:profile');
  });

  it('builds ad-hoc charts', async () => {
    const { fixture } = await render();
    buttonByText(fixture, '+ Add chart').click();
    await settle(fixture);
    const form = el<HTMLFormElement>(fixture, '.builder');
    const [title, unit] = [at(form.querySelectorAll('input'), 0), at(form.querySelectorAll('input'), 1)];
    const selects = form.querySelectorAll('select');
    typeInto(title, 'My CPU');
    typeInto(unit, '%');
    typeInto(selects[1] as unknown as HTMLInputElement, 'cpu', 'change');
    typeInto(selects[2] as unknown as HTMLInputElement, '1m', 'change');
    const split = selects[3] as HTMLSelectElement;
    at(split.options, 0).selected = true;
    split.dispatchEvent(new Event('change'));
    typeInto(selects[4] as unknown as HTMLInputElement, 'bar', 'change');
    form.dispatchEvent(new Event('submit'));
    await settle(fixture);
    expect(all(fixture, 'imt-chart-card')).toHaveLength(4);
    expect(text(fixture)).toContain('My CPU');

    buttonByText(fixture, '+ Add chart').click();
    await settle(fixture);
    typeInto(el<HTMLFormElement>(fixture, '.builder').querySelector('select') as unknown as HTMLInputElement, 'count', 'change');
    await settle(fixture);
    el<HTMLFormElement>(fixture, '.builder').dispatchEvent(new Event('submit'));
    await settle(fixture);
    expect(text(fixture)).toContain('count()');
    buttonByText(fixture, '+ Add chart').click();
    await settle(fixture);
    buttonByText(fixture, 'Cancel').click();
    await settle(fixture);

    for (const b of all<HTMLButtonElement>(fixture, '[aria-label^="Remove chart"]')) b.click();
    await settle(fixture);
    expect(all(fixture, 'imt-chart-card')).toHaveLength(3);
  });

  it('shows an empty state without charts', async () => {
    const { fixture, store } = await render();
    await TestBed.inject(WorkspaceStore).applySummary({ ...store.summary()!, charts: [] });
    await settle(fixture);
    expect(text(fixture)).toContain('No charts defined');
  });
});

@Component({ selector: 'imt-merge-host', imports: [EChart], template: `<imt-echart [options]="opts()" [merge]="merge()" />` })
class MergeHost {
  readonly opts = signal<EChartsCoreOption>({ series: [] });
  readonly merge = signal(false);
}

describe('live charts', () => {
  beforeEach(() => (charts.instances.length = 0));
  afterEach(() => delete (globalThis as Record<string, unknown>)['IntersectionObserver']);

  it('merge mode updates the chart in place (notMerge false, series replaced by name)', async () => {
    const fixture = TestBed.createComponent(MergeHost);
    await settle(fixture);
    const c = at(charts.instances, 0);
    fixture.componentInstance.merge.set(true);
    fixture.componentInstance.opts.set({ series: [{ name: 'a' }] });
    await settle(fixture);
    expect(c.setOption).toHaveBeenLastCalledWith({ series: [{ name: 'a' }] }, { notMerge: false, replaceMerge: ['series'], lazyUpdate: true });
  });

  it('live options glide briefly unless motion is reduced', () => {
    const o = { animation: false, series: [] };
    expect(liveOption(o, true)).toBe(o);
    expect(liveOption(o, false)).toMatchObject({ animation: true, animationDuration: 0, animationDurationUpdate: LIVE_UPDATE_MS });
    expect(liveOption(o)).toBeTruthy();
  });

  it('a card scrolled out of view skips live refreshes until it is visible again, without a spinner', async () => {
    let observed: ((entries: { isIntersecting: boolean }[]) => void) | null = null;
    const disconnect = vi.fn();
    (globalThis as Record<string, unknown>)['IntersectionObserver'] = class {
      constructor(cb: (entries: { isIntersecting: boolean }[]) => void) {
        observed = cb;
      }
      observe() {}
      disconnect = disconnect;
    };
    const backend = tinyBackend();
    TestBed.configureTestingModule({ providers: [provideTinyBackend(backend)] });
    const store = TestBed.inject(WorkspaceStore);
    const live = TestBed.inject(LiveTail);
    const fixture = TestBed.createComponent(ChartCard);
    fixture.componentRef.setInput('def', def({ id: 'cpu', y: 'avg(cpu)' }));
    await settle(fixture);
    expect(observed).not.toBeNull();
    const query = vi.spyOn(TestBed.inject(ImtBackend), 'queryChart');
    await live.set(true);
    await settle(fixture);
    // Live: option animates updates and the chart merges.
    expect(charts.instances.at(-1)!.setOption.mock.calls.at(-1)![0]).toMatchObject({ animation: true });
    observed!([{ isIntersecting: false }]);
    await live.refresh();
    await settle(fixture);
    expect(query).not.toHaveBeenCalled();
    observed!([{ isIntersecting: true }]);
    await settle(fixture);
    expect(query).toHaveBeenCalledTimes(1);
    expect(el(fixture, '.head').querySelector('.spinner')).toBeNull();
    // Not a live change (a profile save): fetched even out of view.
    observed!([{ isIntersecting: false }]);
    store.applySummary(store.summary()!);
    await settle(fixture);
    expect(query).toHaveBeenCalledTimes(2);
    await live.set(false);
    fixture.destroy();
    expect(disconnect).toHaveBeenCalled();
  });
});

