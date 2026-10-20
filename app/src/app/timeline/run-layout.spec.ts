import { AnnotationDto, RunInfo, RunStatus } from '../core/api.types';
import { anomalyOf, layoutRuns, MAX_RUN_ROWS, MIN_RUN_PCT, runKinds, runLabel, runRange, runRowCount } from './run-layout';
import { runAnomalyText, runDuration, runStatusLabel } from './run-text';

function run(status: RunStatus, from: number, to: number, extra: Partial<AnnotationDto> = {}, info: Partial<RunInfo> = {}): AnnotationDto {
  return {
    annotator: 'profile.spans',
    owner: 'profile',
    kind: 'run',
    from,
    to,
    instance: 'A',
    on: null,
    label: 'Running',
    severity: status === 'closed' || status === 'open' ? 'info' : 'warning',
    evidenceQuery: { from, to, levels: [], text: '', regex: false, caseSensitive: false, filters: [], groupBy: [], groups: ['A'], markers: [] },
    run: { span: 'run', spanLabel: 'Running', status, start: null, stop: null, durationMs: to - from, tooLong: false, group: null, color: null, ...info },
    ...extra,
  };
}

const view = { from: 0, to: 1000 };
const opts = { laneKeys: ['A', 'B'], view, hidden: new Set<string>(), shown: true };

describe('layoutRuns', () => {
  it('puts each run on its lane as a band in percent of the view', () => {
    const lanes = layoutRuns([run('closed', 300, 400), run('closed', 100, 200), run('open', 500, 900, { instance: 'B' })], opts);
    const a = lanes.get('A') ?? [];
    expect(a.map((v) => [v.left, v.width])).toEqual([
      [10, 10],
      [30, 10],
    ]);
    expect(lanes.get('B')?.[0]?.status).toBe('open');
  });

  it('clips runs at the view and marks their open ends', () => {
    const [v] = layoutRuns([run('closed', -500, 2000)], opts).get('A') ?? [];
    expect(v).toMatchObject({ left: 0, width: 100, clippedLeft: true, clippedRight: true });
    const [inside] = layoutRuns([run('closed', 100, 200)], opts).get('A') ?? [];
    expect(inside).toMatchObject({ clippedLeft: false, clippedRight: false });
  });

  it('keeps tiny runs visible and draws stops without start as points with a glyph', () => {
    const [tiny] = layoutRuns([run('closed', 999, 1000)], opts).get('A') ?? [];
    expect(tiny?.width).toBe(MIN_RUN_PCT);
    expect(tiny?.left).toBe(100 - MIN_RUN_PCT);
    const [point] = layoutRuns([run('noStart', 500, 500)], opts).get('A') ?? [];
    expect(point).toMatchObject({ point: true, width: 0, left: 50, anomaly: 'noStart' });
  });

  it('leaves out runs outside the view, on unknown lanes, of hidden kinds, plain annotations, or when runs are off', () => {
    const anns = [run('closed', 2000, 3000), run('closed', 1, 2, { instance: 'Z' }), run('closed', 1, 2, { instance: null }), run('closed', 1, 2, { kind: 'other' }), { ...run('closed', 1, 2), run: undefined }];
    expect(layoutRuns(anns, { ...opts, hidden: new Set(['other']) }).size).toBe(0);
    expect(layoutRuns([run('closed', 1, 2)], { ...opts, shown: false }).size).toBe(0);
  });

  it('names anomalies, most telling first', () => {
    expect(anomalyOf(run('noStop', 0, 1).run!)).toBe('noStop');
    expect(anomalyOf(run('closed', 0, 1, {}, { tooLong: true }).run!)).toBe('tooLong');
    expect(anomalyOf(run('closed', 0, 1).run!)).toBeNull();
  });
});

describe('runKinds and runRange', () => {
  it('counts runs and anomalies per span kind, by label', () => {
    const kinds = runKinds([run('closed', 0, 1), run('noStop', 0, 1), run('closed', 0, 1, { kind: 'batch' }, { spanLabel: 'Batch' }), { ...run('closed', 0, 1), run: undefined }]);
    expect(kinds).toEqual([
      { kind: 'batch', label: 'Batch', count: 1, anomalies: 0 },
      { kind: 'run', label: 'Running', count: 2, anomalies: 1 },
    ]);
  });

  it('never filters to an empty range', () => {
    expect(runRange(run('noStart', 5, 5))).toEqual({ from: 5, to: 6 });
    const noEvidence = run('closed', 10, 20);
    noEvidence.evidenceQuery = { ...noEvidence.evidenceQuery, from: null, to: null };
    expect(runRange(noEvidence)).toEqual({ from: 10, to: 20 });
  });
});

describe('run texts', () => {
  it('labels every status and anomaly', () => {
    expect(['closed', 'open', 'noStop', 'noStart'].map((s) => runStatusLabel(s as RunStatus))).toEqual(['Stopped', 'Still running', 'No stop seen', 'Stop without start']);
    expect(runAnomalyText(run('noStart', 0, 0).run!)).toContain('without a start');
    expect(runAnomalyText(run('noStop', 0, 1).run!)).toContain('restart');
    expect(runAnomalyText(run('closed', 0, 1, {}, { tooLong: true }).run!)).toContain('maximum');
    expect(runAnomalyText(run('closed', 0, 1).run!)).toBeNull();
    expect(runDuration(run('noStart', 0, 0).run!)).toBe('–');
    expect(runDuration(run('closed', 0, 90_000).run!)).not.toBe('–');
  });
});

describe('run sub-tracks', () => {
  const view = (n: string, from: number, to: number) => run('closed', from, to, { kind: 'view' }, { span: 'view', spanLabel: 'View', group: { field: 'View', value: n } });

  it('stack overlapping runs (views opened and closed at different times) on labelled rows without overlap', () => {
    const anns = [view('Settings', 0, 500), view('Editor', 100, 200), view('Help', 150, 600), view('About', 250, 300)];
    const lane = layoutRuns(anns, { ...opts, plotWidth: 1000 }).get('A') ?? [];
    expect(lane.map((v) => [v.label, v.row])).toEqual([
      ['Settings', 0],
      ['Editor', 1],
      ['Help', 2],
      ['About', 1],
    ]);
    expect(runRowCount(lane)).toBe(3);
    // Every band on a row is clear of the others on it.
    for (const a of lane) for (const b of lane) if (a !== b && a.row === b.row) expect(a.left + a.width <= b.left || b.left + b.width <= a.left).toBe(true);
  });

  it('keep runs that follow each other on one row, back to back too, and points on a row of their own when they collide', () => {
    const lane = layoutRuns([run('closed', 0, 100), run('closed', 200, 300), run('noStart', 300, 300)], { ...opts, plotWidth: 1000 }).get('A') ?? [];
    expect(lane.map((v) => v.row)).toEqual([0, 0, 1]);
    // A restart: the next run starts where the previous one ended without a stop.
    expect((layoutRuns([run('noStop', 0, 100), run('closed', 100, 200)], { ...opts, plotWidth: 1000 }).get('A') ?? []).map((v) => v.row)).toEqual([0, 0]);
  });

  it('cap the rows per lane', () => {
    const anns = Array.from({ length: MAX_RUN_ROWS + 3 }, (_, i) => view(`v${i}`, 0, 900 + i));
    expect(runRowCount(layoutRuns(anns, opts).get('A'))).toBe(MAX_RUN_ROWS);
  });

  it('show a label only where it fits, and name the band by its paired value, else the rule', () => {
    const lane = layoutRuns([run('closed', 0, 300), run('closed', 400, 420), run('noStart', 600, 600)], { ...opts, plotWidth: 1000 }).get('A') ?? [];
    expect(lane.map((v) => v.labelFits)).toEqual([true, false, false]);
    expect(lane[0]?.label).toBe('Running');
    const r = run('closed', 0, 1);
    expect(runLabel(r, { ...r.run!, group: { field: 'Job', value: 'nightly' } })).toBe('nightly');
    expect(runLabel({ ...r, label: 'Ann' }, { ...r.run!, spanLabel: '' })).toBe('Ann');
    expect(runRowCount(undefined)).toBe(0);
    expect(runRowCount([])).toBe(0);
  });
});
