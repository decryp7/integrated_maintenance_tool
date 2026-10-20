import { describe, expect, it } from 'vitest';
import { Evidence, FrameDto, HeapObjectRow, HeapSummary, ProfileSpan } from '../core/api.types';
import { evidenceSummary } from './findings/findings-view';
import { byTaken } from './heap/heap-diff';
import { flattenDominators } from './heap/heap-dominators';
import { rowWindow } from './hotspots/hotspots-view';
import { labelWidth, layoutSpans } from './integration/profile-span-track';
import { brushToRange, EVENT_GLYPHS, lanePath } from './threads/thread-timeline-view';
import { at } from '../../testing/must';

describe('layoutSpans / labelWidth', () => {
  const span = (id: number, fromMs: number, toMs: number, patch: Partial<ProfileSpan> = {}): ProfileSpan => ({ id, name: `cap${id}`, capture: 'profile', kind: 'cpu', fromMs, toMs, ...patch });

  it('labelWidth grows with the name and is capped at 220px', () => {
    expect(labelWidth('')).toBe(12);
    expect(labelWidth('abcd')).toBe(38);
    expect(labelWidth('x'.repeat(100))).toBe(220);
  });

  it('places spans as percentages of the visible range, unclipped when fully inside', () => {
    const a = at(layoutSpans([span(1, 250, 750)], { from: 0, to: 1000 }), 0);
    expect(a).toMatchObject({ left: 25, width: 50, range: null, row: 0 });
  });

  it('drops spans outside the range but keeps ones touching its edges', () => {
    const out = layoutSpans([span(1, -100, -1), span(2, 1001, 2000), span(3, -50, 0), span(4, 1000, 1200)], { from: 0, to: 1000 });
    expect(out.map((v) => v.span.id)).toEqual([3, 4]);
  });

  it('clips profile spans to the view and reports the visible part in ns since the capture start', () => {
    const v = at(layoutSpans([span(1, 0, 1000)], { from: 200, to: 600 }), 0);
    expect(v.left).toBe(0);
    expect(v.width).toBe(100);
    expect(v.range).toEqual({ fromNs: 200_000_000, toNs: 600_000_000 });
    const only = at(layoutSpans([span(1, 0, 300)], { from: 200, to: 600 }), 0);
    expect(only.range).toEqual({ fromNs: 200_000_000, toNs: 300_000_000 });
    const tail = at(layoutSpans([span(1, 500, 900)], { from: 200, to: 600 }), 0);
    expect(tail.range).toEqual({ fromNs: 0, toNs: 100_000_000 });
  });

  it('never reports a range for heap snapshots, even when they touch the edge', () => {
    const h = at(layoutSpans([span(1, 100, 100, { capture: 'heap', kind: null })], { from: 100, to: 200 }), 0);
    expect(h.range).toBeNull();
    expect(h.width).toBe(0);
    const h2 = at(layoutSpans([span(1, 0, 300, { capture: 'heap', kind: null })], { from: 100, to: 200 }), 0);
    expect(h2.range).toBeNull();
  });

  it('orders by start then id and stacks overlapping labels on new rows', () => {
    const out = layoutSpans([span(3, 10, 20), span(2, 0, 100), span(1, 0, 100), span(4, 900, 950)], { from: 0, to: 1000 });
    expect(out.map((v) => v.span.id)).toEqual([1, 2, 3, 4]);
    // 1 and 2 start at x=0, 3 at x=10: all overlap on labels; 4 starts far right and reuses row 0
    expect(out.map((v) => v.row)).toEqual([0, 1, 2, 0]);
  });

  it('reuses a row once the previous label has ended', () => {
    const out = layoutSpans([span(1, 0, 10), span(2, 500, 510)], { from: 0, to: 1000 });
    expect(out.map((v) => v.row)).toEqual([0, 0]);
  });

  it('handles an empty or degenerate view without dividing by zero', () => {
    expect(layoutSpans([], { from: 0, to: 10 })).toEqual([]);
    const v = at(layoutSpans([span(1, 5, 5)], { from: 5, to: 5 }), 0);
    expect(Number.isFinite(v.left) && Number.isFinite(v.width)).toBe(true);
  });

  it('respects the plot width when stacking labels', () => {
    const out = layoutSpans([span(1, 0, 10), span(2, 100, 110)], { from: 0, to: 1000 }, 100);
    // x = 10px for the second; first label ends at 0 + labelWidth + 4 > 10
    expect(out.map((v) => v.row)).toEqual([0, 1]);
  });
});

describe('lanePath', () => {
  it('is empty without buckets or a maximum', () => {
    expect(lanePath([], 5)).toBe('');
    expect(lanePath([1, 2], 0)).toBe('');
  });
  it('draws one bar per non-empty bucket in a 1000 x 100 box', () => {
    expect(lanePath([0, 50, 100, 0], 100)).toBe('M250.00 100V50.00H500.00V100ZM500.00 100V0.00H750.00V100Z');
  });
  it('gives tiny non-zero buckets a minimum 4px bar and skips negatives', () => {
    expect(lanePath([1, -3], 1000)).toBe('M0.00 100V96.00H500.00V100Z');
  });
});

describe('brushToRange', () => {
  it('ignores tiny brushes and zero widths', () => {
    expect(brushToRange(10, 12, 100, 0, 1000)).toBeNull();
    expect(brushToRange(10, 50, 0, 0, 1000)).toBeNull();
    expect(brushToRange(10, 50, -5, 0, 1000)).toBeNull();
  });
  it('maps pixels to ns in either drag direction, floor/ceil rounded', () => {
    expect(brushToRange(10, 50, 100, 0, 1000)).toEqual({ fromNs: 100, toNs: 500 });
    expect(brushToRange(50, 10, 100, 0, 1000)).toEqual({ fromNs: 100, toNs: 500 });
    expect(brushToRange(0, 33, 100, 0, 1001)).toEqual({ fromNs: 0, toNs: 331 });
    expect(brushToRange(10, 50, 100, 1000, 2000)).toEqual({ fromNs: 1100, toNs: 1500 });
  });
  it('clamps to the shown range', () => {
    expect(brushToRange(-50, 500, 100, 0, 1000)).toEqual({ fromNs: 0, toNs: 1000 });
  });
});

describe('EVENT_GLYPHS', () => {
  it('has a distinct shape and a label for every event kind', () => {
    expect(Object.keys(EVENT_GLYPHS).sort()).toEqual(['exception', 'gc', 'jit', 'lockWait', 'marker', 'wait']);
    const paths = Object.values(EVENT_GLYPHS).map((g) => g.path);
    expect(new Set(paths).size).toBe(paths.length);
    for (const g of Object.values(EVENT_GLYPHS)) expect(g.label.length).toBeGreaterThan(0);
  });
});

describe('evidenceSummary', () => {
  const frame: FrameDto = { id: 1, name: 'Shop.Repo.Save', module: null, file: null, line: null, kind: 'managed', isFramework: false };
  const ev = (patch: Partial<Evidence> = {}): Evidence => ({ view: 'callTree', profileId: 1, baselineId: null, frame: null, path: [], metric: 'timeNs', range: null, threads: null, eventKind: null, ...patch });

  it('names every view', () => {
    const labels = (['hotspots', 'callTree', 'flame', 'butterfly', 'timeline', 'diff'] as const).map((view) => evidenceSummary(ev({ view })));
    expect(labels).toEqual(['hotspots', 'call tree', 'flame graph', 'callers and callees', 'thread timeline', 'comparison']);
  });
  it('prefers the frame over the path', () => {
    expect(evidenceSummary(ev({ frame, path: [1, 2] }))).toBe('call tree · Shop.Repo.Save');
    expect(evidenceSummary(ev({ path: [1, 2, 3] }))).toBe('call tree · path of 3 frames');
  });
  it('adds range, thread count (singular and plural) and event kind', () => {
    expect(evidenceSummary(ev({ range: { fromNs: 1500, toNs: 2_000_000 }, threads: [1], eventKind: 'gc' }))).toBe('call tree · 1.5 µs – 2.00 ms · 1 thread · gc events');
    expect(evidenceSummary(ev({ threads: [1, 2] }))).toBe('call tree · 2 threads');
    expect(evidenceSummary(ev({ threads: [] }))).toBe('call tree');
  });
});

describe('byTaken', () => {
  const heap = (id: number, takenMs: number | null): HeapSummary => ({ capture: 'heap', id, name: `h${id}`, path: null, format: 'x', takenMs, typeCount: 0, objectCount: 0, totalBytes: 0, hasGraph: true, notes: [] });
  it('sorts oldest first, ties and unknown times by id, without mutating', () => {
    const input = [heap(3, 300), heap(2, 100), heap(5, null), heap(4, 100), heap(1, 0)];
    expect(byTaken(input).map((h) => h.id)).toEqual([1, 5, 2, 4, 3]);
    expect(input.map((h) => h.id)).toEqual([3, 2, 5, 4, 1]);
    expect(byTaken([])).toEqual([]);
  });
});

describe('flattenDominators', () => {
  const row = (object: number, childCount: number): HeapObjectRow => ({ object, typeName: `T${object}`, shallow: 1, retained: 2, childCount });
  const level = (rows: HeapObjectRow[], total = rows.length) => ({ rows, total, loading: false, error: null });

  it('is empty until the top level is loaded', () => {
    expect(flattenDominators(new Map(), new Set())).toEqual([]);
  });
  it('lists the top level collapsed with position and set size', () => {
    const out = flattenDominators(new Map([[null, level([row(1, 2), row(2, 0)], 10)]]), new Set([2]));
    expect(out.map((r) => [r.row.object, r.level, r.pos, r.size, r.expanded])).toEqual([
      [1, 1, 1, 10, false],
      [2, 1, 2, 10, false], // expanded set but no children
    ]);
  });
  it('descends into expanded nodes whose level is loaded', () => {
    const levels = new Map([
      [null, level([row(1, 1), row(2, 3)])],
      [1, level([row(11, 0)])],
    ]);
    const out = flattenDominators(levels, new Set([1, 2]));
    expect(out.map((r) => [r.row.object, r.level, r.expanded])).toEqual([
      [1, 1, true],
      [11, 2, false],
      [2, 1, true], // expanded but its children are not loaded yet
    ]);
  });
});

describe('rowWindow', () => {
  it('renders the visible rows plus overscan with spacer heights', () => {
    expect(rowWindow(1000, 0, 260)).toEqual({ first: 0, last: 18, before: 0, after: 982 * 26 });
    expect(rowWindow(1000, 2600, 260)).toEqual({ first: 92, last: 118, before: 92 * 26, after: 882 * 26 });
  });
  it('clamps to the total and never goes negative', () => {
    expect(rowWindow(10, 0, 1000)).toEqual({ first: 0, last: 10, before: 0, after: 0 });
    // Scrolled far past the end: both ends are clamped to the total.
    expect(rowWindow(10, 100_000, 100)).toEqual({ first: 10, last: 10, before: 10 * 26, after: 0 });
    expect(rowWindow(0, 0, 100)).toEqual({ first: 0, last: 0, before: 0, after: 0 });
  });
  it('honours custom row height and overscan', () => {
    expect(rowWindow(100, 50, 20, 10, 0)).toEqual({ first: 5, last: 7, before: 50, after: 930 });
  });
});
