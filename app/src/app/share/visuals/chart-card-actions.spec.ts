import { TestBed } from '@angular/core/testing';
import { FakeECharts } from '../../../testing/fake-echarts';
import { el, settle } from '../../../testing/helpers';
import { ChartDef } from '../../core/api.types';
import { ChartCard } from '../../metrics/chart-card';
import { loadWorkspace } from '../../analysis/test-setup';
import { blocksOf } from '../report/report-model';
import { ReportStore } from '../report/report.store';
import { setupVisuals } from './visual-testing';

const def: ChartDef = { id: 'events-by-level', title: 'Events per interval by level', y: 'count()', bucket: 'auto', splitBy: ['@level'], type: 'stacked', unit: 'events' };

describe('chart card share actions', () => {
  it('exports the chart as PNG and SVG and adds it to the report', async () => {
    const charts = new FakeECharts();
    const v = setupVisuals(undefined, [charts.provider]);
    await loadWorkspace();
    const f = TestBed.createComponent(ChartCard);
    f.componentRef.setInput('def', def);
    await settle(f, 6);
    el<HTMLButtonElement>(f, `[aria-label="Export chart ${def.title} as PNG"]`).click();
    el<HTMLButtonElement>(f, `[aria-label="Export chart ${def.title} as SVG"]`).click();
    await settle(f, 6);
    expect(v.share.written.map((w) => w.path).sort()).toEqual(['/out/events-per-interval-by-level.png', '/out/events-per-interval-by-level.svg']);
    el<HTMLButtonElement>(f, `[aria-label="Add chart ${def.title} to report"]`).click();
    await settle(f, 6);
    expect(blocksOf(TestBed.inject(ReportStore).active()!)[0]).toMatchObject({ type: 'chart', title: def.title, spec: def });
  });
});
