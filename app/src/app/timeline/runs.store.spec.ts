import { TestBed } from '@angular/core/testing';
import { RunsReply } from '../core/api.types';
import { WorkspaceStore } from '../core/workspace.store';
import { DockService } from '../dock/dock.service';
import { FakeDock, provideFakeDock, provideTinyBackend, settle, tinyBackend } from '../../testing/helpers';
import { at, must } from '../../testing/must';
import { TimelineAnnotations } from './annotations.store';
import { RunsBackend } from './runs-backend';
import { RUNS_PAGE, RunsStore } from './runs.store';

async function setup() {
  const backend = tinyBackend();
  const dock = new FakeDock();
  TestBed.configureTestingModule({ providers: [provideTinyBackend(backend), provideFakeDock(dock)] });
  const ws = TestBed.inject(WorkspaceStore);
  await ws.refreshSummary();
  ws.setBuckets(100);
  const runs = TestBed.inject(RunsStore);
  await settle(undefined, 8);
  return { backend, ws, runs, dock };
}

describe('RunsStore', () => {
  it('loads runs and summaries, and reloads when a parameter changes', async () => {
    const { runs, backend } = await setup();
    expect(runs.loaded()).toBe(true);
    expect(runs.rows().length).toBe(3);
    expect(at(runs.summaries(), 0).kind).toBe('run');
    expect(runs.more()).toBe(0);
    const spy = vi.spyOn(backend, 'queryRuns');
    runs.sortBy('duration');
    await settle(undefined, 6);
    expect([runs.sort(), runs.descending()]).toEqual(['duration', true]);
    expect(spy).toHaveBeenLastCalledWith(expect.objectContaining({ sort: 'duration', descending: true, limit: RUNS_PAGE }));
    runs.sortBy('duration');
    expect(runs.descending()).toBe(false);
    runs.sortBy('lane');
    expect(runs.descending()).toBe(false);
    runs.status.set('open');
    runs.inView.set(true);
    runs.showMore();
    await settle(undefined, 6);
    expect(spy).toHaveBeenLastCalledWith(expect.objectContaining({ statuses: ['open'], limit: 2 * RUNS_PAGE, from: expect.any(Number) }));
    expect(runs.rows().every((a) => a.run?.status === 'open')).toBe(true);
    for (let i = 0; i < 10; i++) runs.showMore();
    expect(runs.limit()).toBe(1000);
  });

  it('shows failures with retry and ignores superseded answers', async () => {
    const { runs } = await setup();
    const backend = TestBed.inject(RunsBackend);
    let release!: (r: RunsReply) => void;
    vi.spyOn(backend, 'queryRuns')
      .mockImplementationOnce(() => new Promise<RunsReply>((res) => (release = res)))
      .mockImplementationOnce(() => Promise.reject('engine down'));
    runs.kind.set('run');
    await settle(undefined, 2);
    runs.kind.set(null);
    await settle(undefined, 6);
    expect(runs.error()).toContain('engine down');
    release({ summaries: [], rules: [], runs: [], total: 0, offset: 0 });
    await settle(undefined, 4);
    expect(runs.error()).toContain('engine down');
    runs.retry();
    await settle(undefined, 6);
    expect(runs.error()).toBeNull();
    expect(runs.rows().length).toBe(3);
  });

  it('is empty without data', async () => {
    const { runs, ws } = await setup();
    await ws.clearWorkspace();
    await settle(undefined, 8);
    expect(runs.rows()).toEqual([]);
  });
});

describe('TimelineAnnotations runs', () => {
  it('keeps runs apart from plugin annotations, toggles them and filters to a run', async () => {
    const { ws, dock } = await setup();
    const ann = TestBed.inject(TimelineAnnotations);
    await settle(undefined, 8);
    expect(ann.runs().length).toBe(3);
    expect(ann.plugin().every((a) => !a.run)).toBe(true);
    expect(ann.kinds().some((k) => k.kind === 'run')).toBe(false);
    expect(ann.runKinds()).toEqual([{ kind: 'run', label: 'Running', count: 3, anomalies: 0 }]);
    ann.toggleRuns();
    expect(ann.runsShown()).toBe(false);
    ann.toggleRunKind('run');
    expect(ann.isRunKindShown('run')).toBe(false);
    ann.toggleRunKind('run');
    expect(ann.isRunKindShown('run')).toBe(true);

    const run = must(ann.runs().find((a) => a.run?.status === 'closed'));
    ann.showRun(run);
    await settle(undefined, 6);
    expect(ws.query().groups).toEqual([run.instance]);
    expect(ws.query().from).toBe(run.from);
    expect(dock.calls).toContain('show:events');
    expect(ws.anchor()?.id).toBe(run.run?.start?.id);
    expect(TestBed.inject(DockService)).toBe(dock);
  });
});
