import { Type } from '@angular/core';
import { ComponentFixture, TestBed } from '@angular/core/testing';
import { all, buttonByText, el, settle, text } from '../../testing/helpers';
import { AnalysisBackend } from '../core/analysis-backend';
import { ChartTheme } from '../metrics/chart-theme';
import { AnalysisTargets } from './analysis-targets';
import { barGeometry, DEFAULT_ID_FIELD, TracesView, waterfallTotal } from './traces-view';
import { FieldStatsView, HISTOGRAM_BUCKETS, histogramOption } from './field-stats-view';
import { FakeContext, FIX, loadWorkspace, provideAnalysisTesting, stubBackend } from './test-setup';

const charts = vi.hoisted(() => {
  const instances: { setOption: ReturnType<typeof vi.fn> }[] = [];
  return {
    instances,
    init: vi.fn(() => {
      const c = { setOption: vi.fn(), dispatchAction: vi.fn(), resize: vi.fn(), dispose: vi.fn(), on: vi.fn() };
      instances.push(c);
      return c;
    }),
  };
});
vi.mock('echarts/core', () => ({ init: charts.init, use: vi.fn() }));

async function mount<T>(view: Type<T>, backend: AnalysisBackend = stubBackend(), ctx = new FakeContext()) {
  TestBed.configureTestingModule({ providers: provideAnalysisTesting({ backend, context: ctx }) });
  const store = await loadWorkspace();
  const fixture = TestBed.createComponent(view);
  fixture.detectChanges();
  await settle(fixture, 6);
  return { fixture: fixture as ComponentFixture<T>, ctx, store, targets: TestBed.inject(AnalysisTargets) };
}

const select = (fixture: ComponentFixture<unknown>, value: string) => {
  const s = el<HTMLSelectElement>(fixture, 'select');
  s.value = value;
  s.dispatchEvent(new Event('change', { bubbles: true }));
};

describe('FieldStatsView', () => {
  it('asks for a field, listing the workspace fields', async () => {
    const { fixture } = await mount(FieldStatsView);
    expect(text(fixture)).toContain('Choose a field');
    expect(all(fixture, 'option').length).toBeGreaterThan(5);
  });

  it('draws a histogram with percentiles and top values', async () => {
    const stats = vi.fn(async () => FIX.stats);
    const { fixture, targets } = await mount(FieldStatsView, stubBackend({ fieldStats: stats }));
    select(fixture, 'ElapsedMs');
    await settle(fixture, 6);
    expect(targets.field()).toBe('ElapsedMs');
    expect(stats).toHaveBeenCalledWith(expect.anything(), 'ElapsedMs', HISTOGRAM_BUCKETS);
    expect(text(fixture)).toContain('90 events with the field · 10 without · 40 distinct');
    expect(el(fixture, '[role=img]').getAttribute('aria-label')).toBe('Histogram of ElapsedMs from 1 to 100');
    expect(charts.instances.length).toBeGreaterThan(0);
    const pcts = all(fixture, '.pcts div').map((d) => d.textContent?.replace(/\s+/g, ' ').trim());
    expect(pcts).toEqual(expect.arrayContaining(['min1', 'p5035.0', 'p9590.0', 'max100']));
    expect(all(fixture, '.top .val').map((v) => v.textContent)).toEqual(['42', '7']);
  });

  it('filters by a clicked top value', async () => {
    const { fixture, ctx, targets } = await mount(FieldStatsView);
    targets.setField('Status');
    await settle(fixture, 6);
    all<HTMLButtonElement>(fixture, '.top')[0].click();
    expect(ctx.last('addFilter')).toEqual([{ field: 'Status', op: 'eq', value: '42' }]);
    targets.setField(null);
    await settle(fixture, 4);
    expect(text(fixture)).toContain('Choose a field');
  });

  it('handles non-numeric fields without values and failures', async () => {
    const { fixture, targets } = await mount(FieldStatsView, stubBackend({ fieldStats: async () => FIX.textStats }));
    targets.setField('Host');
    await settle(fixture, 6);
    expect(all(fixture, 'imt-echart')).toHaveLength(0);
    expect(text(fixture)).toContain('No values.');
    select(fixture, '');
    expect(targets.field()).toBeNull();

    TestBed.resetTestingModule();
    const failing = await mount(FieldStatsView, stubBackend({ fieldStats: () => Promise.reject('stats failed') }));
    failing.targets.setField('X');
    await settle(failing.fixture, 6);
    expect(el(failing.fixture, '[role=alert]').textContent).toContain('stats failed');
  });

  it('builds a bar chart option from the engine histogram', () => {
    TestBed.configureTestingModule({ providers: provideAnalysisTesting() });
    const tokens = TestBed.inject(ChartTheme).tokens();
    const opt = histogramOption(FIX.stats.numeric!, tokens) as { xAxis: { data: string[] }; series: { type: string; data: number[] }[] };
    expect(opt.xAxis.data).toHaveLength(2);
    expect(opt.series[0]).toMatchObject({ type: 'bar', data: [60, 30] });
  });
});

describe('TracesView', () => {
  it('lists traces and shows a waterfall for the selected one', async () => {
    const { fixture } = await mount(TracesView);
    const rows = all<HTMLButtonElement>(fixture, '.tlist .row');
    expect(rows).toHaveLength(2);
    expect(rows[0].textContent).toContain('aaaaaaaaaaaa…');
    expect(rows[0].textContent).toContain('GET /orders');
    expect(rows[0].textContent).toContain('1.5 s');
    expect(rows[0].textContent).toContain('3 spans');
    expect(rows[0].textContent).toContain('1 err');
    expect(rows[1].textContent).not.toContain('err');
    expect(text(fixture)).toContain('Select a trace');

    rows[0].click();
    await settle(fixture, 6);
    expect(rows[0].getAttribute('aria-pressed')).toBe('true');
    const wf = all(fixture, '.wrow');
    expect(wf).toHaveLength(2);
    expect(wf[1].querySelector('.wbar')?.classList.contains('bad')).toBe(true);
    expect(wf[0].querySelector('.wbar')?.classList.contains('bad')).toBe(false);
    expect(wf[1].querySelector('[role=img]')?.getAttribute('aria-label')).toBe('SELECT orders: starts at 100 ms, lasts 1.2 s, 2 errors');
    expect(wf[0].querySelector('[role=img]')?.getAttribute('aria-label')).not.toContain('errors');
    expect((wf[1].querySelector('.wname') as HTMLElement).style.paddingLeft).toBe('14px');
    expect(text(fixture)).toContain('Total 1.5 s');
  });

  it('reports a trace without spans and failures', async () => {
    const { fixture } = await mount(TracesView, stubBackend({ queryTraces: async (_q: never, id: string) => (id ? { traces: [], waterfall: [] } : FIX.traces) }));
    all<HTMLButtonElement>(fixture, '.tlist .row')[1].click();
    await settle(fixture, 6);
    expect(text(fixture)).toContain('This trace has no spans');

    TestBed.resetTestingModule();
    const failing = await mount(TracesView, stubBackend({ queryTraces: () => Promise.reject('no traces') }));
    expect(el(failing.fixture, '[role=alert]').textContent).toContain('no traces');

    TestBed.resetTestingModule();
    const none = await mount(TracesView, stubBackend({ queryTraces: async () => ({ traces: [], waterfall: [] }) }));
    expect(text(none.fixture)).toContain('No traces');
  });

  it('lists requests and filters by id through correlate', async () => {
    const requests = vi.fn(async () => FIX.requests);
    const { fixture, ctx } = await mount(TracesView, stubBackend({ queryRequests: requests }));
    buttonByText(fixture, 'Requests').click();
    await settle(fixture, 6);
    expect(requests).toHaveBeenCalledWith(expect.anything(), DEFAULT_ID_FIELD);
    const rows = all<HTMLButtonElement>(fixture, '[aria-label=Requests] .row');
    expect(rows).toHaveLength(2);
    expect(rows[0].textContent).toContain('req-0001-abc…');
    expect(rows[0].textContent).toContain('2.5 s');
    expect(rows[0].textContent).toContain('12 events');
    expect(rows[0].textContent).toContain('1 errors');
    expect(rows[0].textContent).toContain('WEB01');

    rows[0].click();
    await settle(fixture, 6);
    expect(ctx.last('addFilter')).toEqual([{ field: 'RequestId', op: 'eq', value: 'req-0001-abcdefghijkl' }]);

    const input = el<HTMLInputElement>(fixture, 'input[aria-label="Request id field"]');
    input.value = 'CorrelationId';
    input.dispatchEvent(new Event('change'));
    await settle(fixture, 6);
    expect(requests).toHaveBeenLastCalledWith(expect.anything(), 'CorrelationId');
    input.value = ' ';
    input.dispatchEvent(new Event('change'));
    await settle(fixture, 6);
    expect(requests).toHaveBeenLastCalledWith(expect.anything(), DEFAULT_ID_FIELD);

    buttonByText(fixture, 'Refresh').click();
    await settle(fixture, 6);
    buttonByText(fixture, 'Traces').click();
    await settle(fixture, 6);
    buttonByText(fixture, 'Refresh').click();
    await settle(fixture, 6);
  });

  it('says so when no event carries the id', async () => {
    const { fixture, ctx } = await mount(TracesView, stubBackend({ queryRequests: async () => [{ ...FIX.requests[1], id: 'zzz' }] }));
    buttonByText(fixture, 'Requests').click();
    await settle(fixture, 6);
    all<HTMLButtonElement>(fixture, '[aria-label=Requests] .row')[0].click();
    await settle(fixture, 6);
    expect(ctx.last('notifyError')).toEqual(['No events carry the id zzz.']);
    expect(ctx.calls.some((c) => c.fn === 'addFilter')).toBe(false);

    TestBed.resetTestingModule();
    const empty = await mount(TracesView, stubBackend({ queryRequests: async () => [] }));
    buttonByText(empty.fixture, 'Requests').click();
    await settle(empty.fixture, 6);
    expect(text(empty.fixture)).toContain('No requests found');
  });

  it('computes waterfall geometry', () => {
    const row = { spanId: 's', parent: null, depth: 0, name: 'n', startOffsetMs: 250, durationMs: 500, errors: 0 };
    expect(barGeometry(row, 1000)).toEqual({ left: 25, width: 50 });
    expect(barGeometry(row, 0)).toEqual({ left: 0, width: 100 });
    expect(barGeometry({ ...row, startOffsetMs: 990, durationMs: 100 }, 1000).width).toBeCloseTo(1);
    expect(barGeometry({ ...row, startOffsetMs: -5, durationMs: 0 }, 1000)).toEqual({ left: 0, width: 0.5 });
    expect(waterfallTotal([])).toBe(0);
    expect(waterfallTotal(FIX.waterfall.waterfall)).toBe(1500);
  });
});
