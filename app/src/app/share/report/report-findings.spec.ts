import { TestBed } from '@angular/core/testing';
import { Finding } from '../../core/api.types';
import { loadWorkspace } from '../../analysis/test-setup';
import { IMT_RECORD_MENU_ITEMS, RecordActions } from '../../context/record-actions';
import { settle } from '../../../testing/helpers';
import { MockShareBackend } from '../mock/mock-share-backend';
import { ShareBackend } from '../share-backend';
import { setupVisuals } from '../visuals/visual-testing';
import { ReportActions } from './report-actions';
import { reportMenuItems } from './report-menu';
import { blocksOf } from './report-model';
import { ReportCommands } from './report.commands';
import { ReportStore } from './report.store';

async function setup() {
  setupVisuals(undefined, [{ provide: IMT_RECORD_MENU_ITEMS, multi: true, useFactory: reportMenuItems }]);
  const store = await loadWorkspace();
  await settle(undefined, 6);
  return { store, reports: TestBed.inject(ReportStore), actions: TestBed.inject(ReportActions), share: TestBed.inject(ShareBackend) as MockShareBackend };
}

describe('health-check findings and events in reports', () => {
  it('adds the health check findings, capped, and reports empty results and failures', async () => {
    const { store, reports, actions, share } = await setup();
    const finding = (i: number): Finding => ({ severity: i ? 'warning' : 'critical', title: `f${i}`, detail: 'd', action: null });
    vi.spyOn(share, 'overview')
      .mockResolvedValueOnce(Array.from({ length: 12 }, (_, i) => finding(i)))
      .mockResolvedValueOnce([])
      .mockRejectedValueOnce('engine down');
    expect(await actions.addFindings()).toBe(true);
    const blocks = blocksOf(reports.active()!);
    expect(blocks).toHaveLength(10);
    expect(blocks[0]).toMatchObject({ type: 'finding', severity: 'critical', title: 'f0' });
    expect(store.notice()).toBe('Added 10 health-check findings to the report');
    expect(await actions.addFindings()).toBe(false);
    expect(store.notice()).toBe('The health check found nothing to report');
    expect(await actions.addFindings()).toBe(false);
    expect(store.error()).toContain('engine down');
  });

  it('the mock health check derives findings from anomalies, most severe first', async () => {
    const { share, store } = await setup();
    const found = await share.overview(store.effectiveQuery());
    expect(found.length).toBeGreaterThan(0);
    const rank = { critical: 0, warning: 1, info: 2 };
    expect(found.map((f) => rank[f.severity])).toEqual([...found.map((f) => rank[f.severity])].sort());
    expect(found[0].action?.kind).toBe('zoom');
  });

  it('adds events from the record context menu (one, or the multi-selection)', async () => {
    const { store, reports } = await setup();
    const rows = (await (TestBed.inject(ShareBackend) as MockShareBackend)['imt'].queryPage({ query: store.query(), offset: 0, limit: 3, descending: false, anchor: null })).rows;
    const items = TestBed.inject(RecordActions).itemsFor(rows[0]);
    const add = items.find((i) => i.label === 'Add to report')!;
    add.run();
    expect(blocksOf(reports.active()!)[0]).toMatchObject({ type: 'events', ids: [rows[0].id] });
    const multi = TestBed.inject(RecordActions).itemsFor(rows[0], { records: rows });
    multi.find((i) => i.label === 'Add 3 events to report')!.run();
    expect(blocksOf(reports.active()!)[1]).toMatchObject({ type: 'events', ids: rows.map((r) => r.id) });
    expect(TestBed.inject(ReportActions).addEvents([])).toBe(false);
  });

  it('offers the findings command while there are events', async () => {
    await setup();
    const cmd = TestBed.inject(ReportCommands).commands.find((c) => c.id === 'report.addFindings')!;
    expect(cmd.when!({} as never)).toBe(true);
    const shown: string[] = [];
    await cmd.run({ showView: (id: string) => shown.push(id) } as never);
    expect(shown).toEqual(['report']);
  });
});
