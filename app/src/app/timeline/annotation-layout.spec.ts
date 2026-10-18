import { AnnotationDto } from '../core/api.types';
import { markerKind, query } from '../../testing/helpers';
import { annotationKinds, laneMatches, layoutAnnotations, MIN_BAND_PCT } from './annotation-layout';

const ann = (patch: Partial<AnnotationDto>): AnnotationDto => ({
  annotator: 'a',
  owner: 'p',
  kind: 'p/span',
  from: 100,
  to: 200,
  instance: null,
  on: null,
  label: 'x',
  severity: 'info',
  evidenceQuery: query(),
  ...patch,
});

const LANES = ['WEB01 · 1', 'WEB01 · 2', 'WEB02 · 3'];
const opts = (patch: Partial<Parameters<typeof layoutAnnotations>[1]> = {}) => ({
  laneKeys: LANES,
  view: { from: 0, to: 1000 },
  groupBy: ['MachineName', 'ProcessId'],
  hidden: new Set<string>(),
  ...patch,
});

describe('laneMatches', () => {
  it('compares the group-by part of a lane key, or says it cannot tell', () => {
    const on = { field: 'MachineName', value: 'WEB01' };
    expect(laneMatches('WEB01 · 1', ['MachineName', 'ProcessId'], on)).toBe(true);
    expect(laneMatches('WEB02 · 3', ['MachineName', 'ProcessId'], on)).toBe(false);
    expect(laneMatches('WEB01 · 1', ['ProcessId'], on)).toBeNull();
    expect(laneMatches('odd key', ['MachineName', 'ProcessId'], on)).toBeNull();
  });
});

describe('layoutAnnotations', () => {
  it('puts instance spans on their lane and machine-wide ones on every lane of the machine', () => {
    const l = layoutAnnotations(
      [ann({ instance: 'WEB02 · 3' }), ann({ on: { field: 'MachineName', value: 'WEB01' }, from: 500, to: 600 }), ann({ instance: 'gone' })],
      opts(),
    );
    expect([...l.lanes.keys()]).toEqual(['WEB02 · 3', 'WEB01 · 1', 'WEB01 · 2']);
    expect(l.lanes.get('WEB02 · 3')![0]).toMatchObject({ left: 10, width: 10, point: false });
    expect(l.lanes.get('WEB01 · 2')![0]).toMatchObject({ left: 50, width: 10 });
    expect(l.global).toEqual([]);
  });

  it('draws annotations for every instance, or on a field the lanes do not show, full height', () => {
    const l = layoutAnnotations([ann({}), ann({ on: { field: 'Computer', value: 'X' } })], opts());
    expect(l.global).toHaveLength(2);
    expect(l.lanes.size).toBe(0);
  });

  it('makes points glyphs, keeps short spans visible and skips hidden or out-of-view ones', () => {
    const l = layoutAnnotations(
      [
        ann({ instance: 'WEB01 · 1', from: 300, to: 300, kind: 'p/point' }),
        ann({ instance: 'WEB01 · 1', from: 1000, to: 1000.5 }),
        ann({ instance: 'WEB01 · 1', kind: 'p/hidden' }),
        ann({ instance: 'WEB01 · 1', from: 2000, to: 3000 }),
        ann({ instance: 'WEB01 · 1', from: -500, to: -100 }),
      ],
      opts({ hidden: new Set(['p/hidden']) }),
    );
    const items = l.lanes.get('WEB01 · 1')!;
    expect(items).toHaveLength(2);
    expect(items[0]).toMatchObject({ point: true, width: 0, left: 30 });
    expect(items[1]?.width).toBe(MIN_BAND_PCT);
    expect(items[1]?.left).toBe(100 - MIN_BAND_PCT);
    expect(new Set(items.map((i) => i.id)).size).toBe(2);
  });
});

describe('annotationKinds', () => {
  it('lists each kind once with its count, most severe severity and a label', () => {
    const kinds = annotationKinds(
      [ann({ kind: 'p/downtime', severity: 'warning' }), ann({ kind: 'p/downtime', severity: 'critical' }), ann({ kind: 'p/downtime', severity: 'info' }), ann({ kind: 'p/app-crash' })],
      new Map([['p/app-crash', markerKind({ kind: 'p/app-crash', label: 'Application crash' })]]),
    );
    expect(kinds).toEqual([
      { kind: 'p/app-crash', label: 'Application crash', owner: 'p', count: 1, severity: 'info' },
      { kind: 'p/downtime', label: 'downtime', owner: 'p', count: 3, severity: 'critical' },
    ]);
  });
});
