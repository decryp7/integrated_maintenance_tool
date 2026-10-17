import { Component, signal } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import type { EChartsCoreOption } from 'echarts/core';
import { all, buttonByText, el, provideTinyBackend, settle, text, tinyBackend, typeInto } from '../../testing/helpers';
import { ChartData, ChartDef } from '../core/api.types';
import { WorkspaceStore } from '../core/workspace.store';
import { buildOption, ChartCard } from './chart-card';
import { ChartTheme, ChartTokens } from './chart-theme';
import { EChart } from './echart';
import { MetricsView } from './metrics-view';

const charts = vi.hoisted(() => {
  const instances: { handlers: Record<string, (e: unknown) => void>; setOption: ReturnType<typeof vi.fn>; dispatchAction: ReturnType<typeof vi.fn>; resize: ReturnType<typeof vi.fn>; dispose: ReturnType<typeof vi.fn> }[] = [];
  return {
    instances,
    init: vi.fn(() => {
      const c = {
        handlers: {} as Record<string, (e: unknown) => void>,
        setOption: vi.fn(),
        dispatchAction: vi.fn(),
        resize: vi.fn(),
        dispose: vi.fn(),
        on(name: string, h: (e: unknown) => void) {
          c.handlers[name] = h;
        },
      };
      instances.push(c);
      return c;
    }),
  };
});
vi.mock('echarts/core', () => ({ init: charts.init, use: vi.fn() }));

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

type Opt = Record<string, any>;

describe('buildOption', () => {
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
    expect(s[4]['color']).toBe('#o');
    expect(s[4]['lineStyle'].width).toBe(1);
    expect(s[4]['endLabel']).toBeUndefined();
  });

  it('sums extra bar series into one "Other" series', () => {
    const o = buildOption(data(['a', 'b', 'c', 'd', 'e', 'f'], { type: 'stacked', unit: '' }), TOKENS, 'utc', new Map()) as Opt;
    const s = o['series'] as Opt[];
    expect(s.map((x) => x['name'])).toEqual(['a', 'b', 'c', 'd', 'Other (2)']);
    expect(s[4]['data']).toEqual([
      [0, 11],
      [60_000, null],
    ]);
    expect(s[0]['stack']).toBe('total');
    expect(s[0]['type']).toBe('bar');
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
    const c = charts.instances[0];
    expect(c.setOption).toHaveBeenCalledWith({ series: [] }, { notMerge: true });
    expect(c.dispatchAction).toHaveBeenCalledWith(expect.objectContaining({ type: 'takeGlobalCursor' }));
    fixture.componentInstance.opts.set({ series: [{}] });
    await settle(fixture);
    expect(c.setOption).toHaveBeenLastCalledWith({ series: [{}] }, { notMerge: true });
    c.handlers['brushEnd']({ areas: [{ coordRange: [10.4, 20.2] }] });
    expect(fixture.componentInstance.got).toEqual([10, 21]);
    fixture.componentInstance.got = null;
    c.handlers['brushEnd']({ areas: [] });
    c.handlers['brushEnd']({ areas: [{ coordRange: [5, 5] }] });
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
    TestBed.configureTestingModule({ providers: [provideTinyBackend(backend)] });
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
    charts.instances[0].handlers['brushEnd']({ areas: [{ coordRange: [t0 + 1000, t0 + 60_000] }] });
    expect(store.query().from).toBe(t0 + 1000);
    el<HTMLButtonElement>(fixture, '[aria-label="Remove chart C"]').click();
    expect(removed).toBe(true);
  });
});

describe('MetricsView', () => {
  async function render() {
    TestBed.configureTestingModule({ providers: [provideTinyBackend()] });
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
    expect(store.activeTab()).toBe('profile');
  });

  it('builds ad-hoc charts', async () => {
    const { fixture } = await render();
    buttonByText(fixture, '+ Add chart').click();
    await settle(fixture);
    const form = el<HTMLFormElement>(fixture, '.builder');
    const [title, unit] = [form.querySelectorAll('input')[0], form.querySelectorAll('input')[1]];
    const selects = form.querySelectorAll('select');
    typeInto(title, 'My CPU');
    typeInto(unit, '%');
    typeInto(selects[1] as unknown as HTMLInputElement, 'cpu', 'change');
    typeInto(selects[2] as unknown as HTMLInputElement, '1m', 'change');
    const split = selects[3] as HTMLSelectElement;
    split.options[0].selected = true;
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
