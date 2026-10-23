import { ComponentFixture, TestBed } from '@angular/core/testing';
import { all, el, settle, text } from '../../testing/helpers';
import { at } from '../../testing/must';
import { AnalysisBackend } from '../core/analysis-backend';
import { formatLift, formatP, peakBucket, sparkBars, templateValueQl } from './analysis-format';
import { mockOutliers } from './mock/mock-insights';
import { OutliersView } from './outliers-view';
import { allRecords, FakeContext, FIX, loadWorkspace, provideAnalysisTesting, stubBackend } from './test-setup';

async function mount(backend: AnalysisBackend = stubBackend(), ctx = new FakeContext()) {
  TestBed.configureTestingModule({ providers: provideAnalysisTesting({ backend, context: ctx }) });
  await loadWorkspace();
  const fixture: ComponentFixture<OutliersView> = TestBed.createComponent(OutliersView);
  fixture.detectChanges();
  await settle(fixture, 6);
  return { fixture, ctx };
}

describe('OutliersView', () => {
  it('lists the engine outliers with counts, lift and p-value', async () => {
    const { fixture } = await mount();
    const rows = all<HTMLButtonElement>(fixture, '.row');
    expect(rows).toHaveLength(2);
    expect(rows[0]?.textContent).toContain('Host');
    expect(rows[0]?.textContent).toContain('60 vs 240');
    expect(rows[0]?.textContent).toContain('4.2×');
    expect(rows[0]?.textContent).toContain('p < 0.001');
    expect(text(fixture)).toContain('60 selected events against 540 others');
  });

  it('filters the workspace to an outlier and opens its evidence', async () => {
    const { fixture, ctx } = await mount();
    at(all<HTMLButtonElement>(fixture, '.row'), 0).click();
    expect(ctx.last('setQuery')).toEqual([{ ql: 'Host = "B" and (@level >= error)' }]);
    const evidence = all<HTMLButtonElement>(fixture, '.ev');
    expect(evidence.map((b) => b.textContent?.trim())).toEqual(['#5', '#6']);
    at(evidence, 0).click();
    await settle(fixture);
    expect(ctx.last('jumpTo')).toEqual([5]);
  });

  it('asks the engine again with the typed subset', async () => {
    const calls: string[] = [];
    const backend = stubBackend({
      queryOutliers: (async (_q: unknown, subset: string) => {
        calls.push(subset);
        return FIX.outliers;
      }) as never,
    });
    const { fixture } = await mount(backend);
    const input = el<HTMLInputElement>(fixture, 'input');
    input.value = 'Host = "B"';
    input.dispatchEvent(new Event('input'));
    el<HTMLButtonElement>(fixture, '.bar .btn').click();
    await settle(fixture, 6);
    expect(calls).toEqual(['', 'Host = "B"']);
  });

  it('says so when nothing stands out, and shows engine errors with Retry', async () => {
    const empty = await mount(stubBackend({ queryOutliers: (async () => ({ ...FIX.outliers, outliers: [] })) as never }));
    expect(text(empty.fixture)).toContain('No value stands out');
    TestBed.resetTestingModule();
    const failed = await mount(stubBackend({ queryOutliers: (() => Promise.reject(new Error('use a filter without | stages'))) as never }));
    expect(el(failed.fixture, '[role=alert]').textContent).toContain('use a filter without | stages');
  });
});

describe('outlier formatting and the mock engine', () => {
  it('formats lift and p-values', () => {
    expect(formatLift(4.24)).toBe('4.2×');
    expect(formatLift(250.4)).toBe('250×');
    expect(formatLift(Infinity)).toBe('—');
    expect(formatP(0.0004)).toBe('p < 0.001');
    expect(formatP(0.0123)).toBe('p = 0.012');
    expect(formatP(NaN)).toBe('—');
  });

  it('finds values concentrated in the errors of the demo data and respects the field list', async () => {
    TestBed.configureTestingModule({ providers: provideAnalysisTesting() });
    await loadWorkspace();
    const records = await allRecords();
    const report = mockOutliers(records, []);
    expect(report.subsetTotal).toBeGreaterThan(0);
    expect(report.subsetTotal + report.restTotal).toBe(records.length);
    for (const o of report.outliers) {
      expect(o.lift).toBeGreaterThanOrEqual(1.5);
      expect(o.subsetCount).toBeGreaterThanOrEqual(3);
    }
    const sorted = [...report.outliers].sort((a, b) => b.significance - a.significance);
    expect(report.outliers.map((o) => o.significance)).toEqual(sorted.map((o) => o.significance));
    const only = mockOutliers(records, ['@source']);
    expect(only.outliers.every((o) => o.field === '@source')).toBe(true);
    const none = mockOutliers([], []);
    expect(none.outliers).toEqual([]);
  });
});

describe('pattern trend and value helpers', () => {
  it('scales sparkline bars to the busiest bucket and keeps single events visible', () => {
    expect(sparkBars([], 72, 18)).toEqual([]);
    expect(sparkBars([0, 0], 72, 18)).toEqual([]);
    const bars = sparkBars([0, 1, 100, 50], 40, 20);
    expect(bars.map((b) => b.height)).toEqual([1, 20, 10]);
    expect(bars.map((b) => b.y)).toEqual([19, 0, 10]);
    expect(bars[0]?.x).toBe(10);
    expect(bars.every((b) => b.width >= 1)).toBe(true);
  });

  it('finds the busiest bucket', () => {
    expect(peakBucket([])).toBe(-1);
    expect(peakBucket([0, 0])).toBe(-1);
    expect(peakBucket([1, 7, 3, 7])).toBe(1);
  });

  it('builds the filter for a template with one of its values', () => {
    expect(templateValueQl('Took <*> ms', true, '12')).toBe('(@message ~ "Took .+ ms") and @message contains "12"');
    expect(templateValueQl('Order {Id}', false, 'a"b')).toBe('(@template = "Order {Id}") and @message contains "a\\"b"');
  });
});
