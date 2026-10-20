import { TestBed } from '@angular/core/testing';
import { ViewRequests } from '../core/view-requests';
import { WorkspaceStore } from '../core/workspace.store';
import { COMMAND_CONTEXT } from '../extensions/command-registry';
import { all, el, FakeDock, provideFakeDock, provideTinyBackend, settle, text, tinyBackend, typeInto } from '../../testing/helpers';
import { at } from '../../testing/must';
import { RunsView } from './runs-view';
import { RunsBackend } from './runs-backend';
import { RUNS_COMMANDS } from './runs.plugin';

async function mount() {
  const backend = tinyBackend();
  const dock = new FakeDock();
  const shown: string[] = [];
  TestBed.configureTestingModule({ providers: [provideTinyBackend(backend), provideFakeDock(dock), { provide: COMMAND_CONTEXT, useValue: { showView: (id: string) => shown.push(id) } }] });
  const ws = TestBed.inject(WorkspaceStore);
  await ws.refreshSummary();
  const fixture = TestBed.createComponent(RunsView);
  await settle(fixture, 10);
  return { fixture, ws, dock, shown };
}

describe('RunsView', () => {
  it('lists every run per lane with start, stop, duration and status, and a summary per kind', async () => {
    const { fixture } = await mount();
    const rows = all(fixture, 'tbody tr');
    expect(rows.length).toBe(3);
    expect(rows.map((r) => r.getAttribute('data-status'))).toEqual(['closed', 'closed', 'open']);
    expect(text(fixture)).toContain('Still running');
    const card = el(fixture, '.card[data-kind="run"]');
    expect(card.textContent).toContain('Running');
    expect(card.textContent).toContain('3 runs');
    expect(el(fixture, 'th[aria-sort="ascending"]').textContent).toContain('Start');
  });

  it('sorts by a column header and filters by status', async () => {
    const { fixture } = await mount();
    el<HTMLButtonElement>(fixture, 'button[data-sort="duration"]').click();
    await settle(fixture, 8);
    expect(el(fixture, 'th[aria-sort="descending"]').textContent).toContain('Duration');
    const select = at(all<HTMLSelectElement>(fixture, 'select'), 1);
    typeInto(select, 'open', 'change');
    await settle(fixture, 8);
    expect(all(fixture, 'tbody tr').map((r) => r.getAttribute('data-status'))).toEqual(['open']);
    typeInto(at(all<HTMLSelectElement>(fixture, 'select'), 0), 'run', 'change');
    typeInto(select, 'bogus', 'change');
    await settle(fixture, 8);
    expect(all(fixture, 'tbody tr').length).toBe(3);
  });

  it('jumps to a run, opens the editor and says why it is empty or failed', async () => {
    const { fixture, ws, shown } = await mount();
    const lane = at(all<HTMLButtonElement>(fixture, 'tbody .link'), 0);
    expect(lane.textContent).toContain('show the run from');
    lane.click();
    await settle(fixture, 6);
    expect(ws.query().groups.length).toBe(1);
    // The annotations ask for the events grid; the dock policy shows it.
    expect(TestBed.inject(ViewRequests).request()?.id).toBe('events');
    el<HTMLButtonElement>(fixture, '.head .btn').click();
    expect(shown).toEqual(['markers']);

    const backend = TestBed.inject(RunsBackend);
    vi.spyOn(backend, 'queryRuns').mockRejectedValueOnce('engine down');
    el<HTMLButtonElement>(fixture, 'button[data-sort="lane"]').click();
    await settle(fixture, 8);
    expect(el(fixture, '[role="alert"]').textContent).toContain('engine down');
    el<HTMLButtonElement>(fixture, '[role="alert"] .btn').click();
    await settle(fixture, 8);
    expect(fixture.nativeElement.querySelector('[role="alert"]')).toBeNull();

    await ws.clearWorkspace();
    await settle(fixture, 8);
    expect(text(fixture)).toContain('Open log files');
  });

  it('registers palette commands for the view and the editor', () => {
    const shown: string[] = [];
    for (const c of RUNS_COMMANDS) void c.run({ showView: (id: string) => shown.push(id) } as never);
    expect(shown).toEqual(['runs', 'markers']);
  });
});
