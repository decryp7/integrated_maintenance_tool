import { AnnotationDto, RunInfo, RunStatus } from '../core/api.types';
import { anomalyOf, layoutRuns, MIN_RUN_PCT, runKinds, runRange } from './run-layout';
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
