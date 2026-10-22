import { ANALYSIS_TAB_CAP, analysisTabToClose, viewArea } from './view-area';

describe('viewArea', () => {
  it('is a document for a view that is not registered', () => {
    expect(viewArea(undefined)).toBe('document');
    expect(viewArea(null)).toBe('document');
  });

  it('uses the area the view declares, whatever its kind and position', () => {
    expect(viewArea({ id: 'x', defaultPosition: 'bottom', area: 'side' })).toBe('side');
    expect(viewArea({ id: 'inspector', defaultPosition: 'right', area: 'analysis' })).toBe('analysis');
    expect(viewArea({ id: 'x', area: 'status' })).toBe('status');
  });

  it('derives side tools, status tools, analysis and documents from kind and position', () => {
    expect(viewArea({ id: 'inspector', defaultPosition: 'right' })).toBe('side');
    expect(viewArea({ id: 'problems', defaultPosition: 'bottom' })).toBe('status');
    expect(viewArea({ id: 'notes' })).toBe('status');
    expect(viewArea({ id: 'plugin-view', defaultPosition: 'right' })).toBe('side');
    expect(viewArea({ id: 'anomalies', defaultPosition: 'bottom' })).toBe('analysis');
    expect(viewArea({ id: 'my-doc', defaultPosition: 'bottom' })).toBe('analysis');
    expect(viewArea({ id: 'timeline', defaultPosition: 'center' })).toBe('document');
    expect(viewArea({ id: 'timeline' })).toBe('document');
    expect(viewArea({ id: 'events', defaultPosition: 'right' })).toBe('document');
  });
});

describe('analysisTabToClose', () => {
  const tabs = (n: number) => Array.from({ length: n }, (_, i) => `t${i}`);

  it('is null while the panel has room', () => {
    expect(analysisTabToClose(tabs(ANALYSIS_TAB_CAP - 1), 't0', new Set(), [])).toBeNull();
    expect(analysisTabToClose([], null, new Set(), [])).toBeNull();
  });

  it('closes the least recently used tab that is neither active nor pinned', () => {
    const all = tabs(ANALYSIS_TAB_CAP);
    // Most recent first: t0 is the newest, t7 the oldest.
    const mru = [...all];
    expect(analysisTabToClose(all, 't0', new Set(), mru)).toBe('t7');
    expect(analysisTabToClose(all, 't7', new Set(), mru)).toBe('t6');
    expect(analysisTabToClose(all, 't0', new Set(['t7', 't6']), mru)).toBe('t5');
  });

  it('treats tabs missing from the history as the oldest', () => {
    const all = tabs(ANALYSIS_TAB_CAP);
    expect(analysisTabToClose(all, 't0', new Set(), ['t7', 't6', 't5', 't4', 't3', 't2', 't1'])).toBe('t1');
    expect(analysisTabToClose(all, 't1', new Set(), all.slice(2))).toBe('t0');
  });

  it('is null when every tab is active or pinned', () => {
    const all = tabs(ANALYSIS_TAB_CAP);
    expect(analysisTabToClose(all, 't0', new Set(all.slice(1)), all)).toBeNull();
  });

  it('honours a custom cap', () => {
    expect(analysisTabToClose(['a', 'b', 'c'], 'a', new Set(), ['a', 'b', 'c'], 3)).toBe('c');
    expect(analysisTabToClose(['a', 'b'], 'a', new Set(), ['a', 'b'], 3)).toBeNull();
  });
});
