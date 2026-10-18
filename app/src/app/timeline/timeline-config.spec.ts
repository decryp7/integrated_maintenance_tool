import { query } from '../../testing/helpers';
import {
  defaultConfig,
  defaultTitle,
  instanceQuery,
  isTimelineInstance,
  laneGroupBy,
  MAX_TIMELINES,
  nextTimelineId,
  sanitizeConfig,
  sanitizeConfigs,
  TimelineConfig,
} from './timeline-config';

const cfg = (patch: Partial<TimelineConfig> = {}): TimelineConfig => ({ ...defaultConfig('timeline#2'), ...patch });

describe('timeline ids and titles', () => {
  it('recognises additional timeline panels', () => {
    expect(isTimelineInstance('timeline#2')).toBe(true);
    expect(isTimelineInstance('timeline')).toBe(false);
    expect(isTimelineInstance('timeline#')).toBe(false);
    expect(isTimelineInstance('events#2')).toBe(false);
    expect(isTimelineInstance(null)).toBe(false);
    expect(isTimelineInstance(undefined)).toBe(false);
  });

  it('numbers new timelines from 2, filling gaps', () => {
    expect(nextTimelineId([])).toBe('timeline#2');
    expect(nextTimelineId(['timeline', 'timeline#2', 'timeline#4', 'events'])).toBe('timeline#3');
    expect(nextTimelineId(['timeline#2', 'timeline#3', 'timeline#x'])).toBe('timeline#4');
  });

  it('derives a default title from the id', () => {
    expect(defaultTitle('timeline#7')).toBe('Timeline 7');
    expect(defaultTitle('timeline#abc')).toBe('Timeline');
    expect(defaultConfig('timeline#3')).toEqual({
      id: 'timeline#3',
      title: 'Timeline 3',
      scope: 'global',
      ql: '',
      lanes: 'instance',
      laneField: '',
      markers: null,
      groups: [],
      sync: true,
    });
  });
});

describe('sanitizeConfig', () => {
  it('keeps valid settings', () => {
    const c = cfg({ title: 'Errors WEB01', scope: 'own', ql: 'level >= error', lanes: 'field', laneField: 'MachineName', markers: ['fatal'], groups: ['a'], sync: false });
    expect(sanitizeConfig(c)).toEqual(c);
  });

  it('falls back to defaults for malformed values', () => {
    expect(sanitizeConfig({ id: 'timeline#2', title: '  ', scope: 'x', ql: 5, lanes: 'nope', laneField: 1, markers: 'fatal', groups: [1, 'a', 'a', ''], sync: 'yes' })).toEqual({
      ...defaultConfig('timeline#2'),
      markers: null,
      groups: ['a'],
    });
    expect(sanitizeConfig({ id: 'timeline#2', markers: [3, 'error'] })?.markers).toEqual(['error']);
    expect(sanitizeConfig({ id: 'timeline#2', title: 'x'.repeat(200), ql: 'y'.repeat(5000) })).toMatchObject({ title: 'x'.repeat(80), ql: 'y'.repeat(4000) });
  });

  it('rejects anything that is not an additional timeline', () => {
    for (const raw of [null, 'x', [], {}, { id: 'timeline' }, { id: 'events#2' }, { id: 'timeline#' + 'x'.repeat(80) }]) expect(sanitizeConfig(raw)).toBeNull();
  });

  it('parses lists, dropping duplicates and keeping at most the limit', () => {
    expect(sanitizeConfigs('nope')).toEqual([]);
    expect(sanitizeConfigs([{ id: 'timeline#2' }, { id: 'timeline#2', title: 'dup' }, 7, { id: 'timeline#3' }]).map((c) => c.id)).toEqual(['timeline#2', 'timeline#3']);
    const many = Array.from({ length: MAX_TIMELINES + 5 }, (_, i) => ({ id: `timeline#${i + 2}` }));
    expect(sanitizeConfigs(many)).toHaveLength(MAX_TIMELINES);
  });
});

describe('laneGroupBy', () => {
  const ws = ['MachineName', 'ProcessId'];
  it('maps lane modes to group-by fields', () => {
    expect(laneGroupBy({ lanes: 'instance', laneField: '' }, ws)).toEqual(ws);
    expect(laneGroupBy({ lanes: 'source', laneField: '' }, ws)).toEqual(['@source']);
    expect(laneGroupBy({ lanes: 'level', laneField: '' }, ws)).toEqual(['@level']);
    expect(laneGroupBy({ lanes: 'field', laneField: 'MachineName' }, ws)).toEqual(['MachineName']);
    // A field mode without a field yet groups like the workspace.
    expect(laneGroupBy({ lanes: 'field', laneField: '' }, ws)).toEqual(ws);
  });
});

describe('instanceQuery', () => {
  const ws = query({ from: 10, to: 20, levels: ['Error'], text: 'boom', ql: 'MachineName = WEB01', groupBy: [], groups: ['WEB01 · 100'], markers: ['fatal'] });
  const groupBy = ['MachineName', 'ProcessId'];

  it('inherits the workspace filter, range, instance filter and markers by default', () => {
    expect(instanceQuery(ws, groupBy, cfg(), null)).toEqual({ ...ws, groupBy, window: null });
  });

  it('uses only its own query-language expression with an own scope', () => {
    const q = instanceQuery(ws, groupBy, cfg({ scope: 'own', ql: '  level >= error  ' }), null);
    expect(q).toMatchObject({ ql: 'level >= error', levels: [], text: '', filters: [], groups: [], markers: [], from: 10, to: 20 });
  });

  it('groups lanes its own way and drops the instance filter that no longer applies', () => {
    const q = instanceQuery(ws, groupBy, cfg({ lanes: 'level' }), null);
    expect(q.groupBy).toEqual(['@level']);
    expect(q.groups).toEqual([]);
    expect(instanceQuery(ws, groupBy, cfg({ lanes: 'level', groups: ['Error'] }), null).groups).toEqual(['Error']);
  });

  it('uses its own marker selection', () => {
    expect(instanceQuery(ws, groupBy, cfg({ markers: ['error'] }), null).markers).toEqual(['error']);
    expect(instanceQuery(ws, groupBy, cfg({ scope: 'own', markers: ['warning'] }), null).markers).toEqual(['warning']);
  });

  it('follows the workspace range and live window while synced, its own range otherwise', () => {
    const live = query({ window: { kind: 'live', lastMs: 60_000 } });
    expect(instanceQuery(live, groupBy, cfg(), null)).toMatchObject({ from: null, to: null, window: { kind: 'live', lastMs: 60_000 } });
    expect(instanceQuery(live, groupBy, cfg({ sync: false }), { from: 1, to: 5 })).toMatchObject({ from: 1, to: 5, window: null });
    expect(instanceQuery(ws, groupBy, cfg({ sync: false }), null)).toMatchObject({ from: null, to: null, window: null });
  });
});
