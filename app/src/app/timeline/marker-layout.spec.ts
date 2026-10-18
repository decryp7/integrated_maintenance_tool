import { badgeWidth, defaultMaxShapes, LaidMarker, layoutLane, layoutTargets, MarkerTarget, MarkerVm, placeBadges, TARGET_PX } from './marker-layout';
import { at } from '../../testing/must';

const mk = (kind: string, left: number, count = 1, id = Math.round(left * 100)): MarkerVm => ({
  cluster: { kind, bucket: 0, ts: left, id, count, message: '' },
  left,
  color: null,
  label: kind,
});

const PRIORITY = ['fatal', 'start', 'stop', 'exception', 'error', 'warning'];
const COMPACT = new Set(['warning']);

describe('marker layout', () => {
  it('sizes badges by digit count and scales the busy threshold with width', () => {
    expect(badgeWidth(7)).toBeCloseTo(9.6);
    expect(badgeWidth(1234)).toBeCloseTo(26.4);
    expect(defaultMaxShapes(100)).toBe(24);
    expect(defaultMaxShapes(1800)).toBe(100);
  });

  it('shows a badge only when it clears neighbouring icons', () => {
    // 1000px plot: 10% = 100px. Neighbour 1% (10px) to the right blocks the badge.
    const a = mk('warning', 10, 5);
    const b = mk('error', 11, 1);
    expect(placeBadges([a, b], 1000).has(a)).toBe(false);
    const c = mk('error', 14, 1);
    expect(placeBadges([a, c], 1000).has(a)).toBe(true);
  });

  it('gives contested space to the higher count and drops low counts first', () => {
    const low = mk('warning', 10, 3);
    const high = mk('error', 10.9, 250);
    const shown = placeBadges([low, high], 1000);
    expect(shown.has(high)).toBe(true);
    expect(shown.has(low)).toBe(false);
  });

  it('breaks ties by position and priority deterministically', () => {
    const a = mk('warning', 10, 4);
    const b = mk('warning', 40, 4);
    expect([...placeBadges([b, a], 1000)].map((m) => m.left)).toEqual([10, 40]);
    const layout = layoutLane([mk('error', 30), mk('error', 20)], { plotWidth: 1000, priority: PRIORITY, compactKinds: COMPACT });
    expect(layout.shapes.map((m) => m.left)).toEqual([20, 30]);
  });

  it('never lets badges overlap each other or run off the plot', () => {
    const a = mk('warning', 50, 12);
    const b = mk('error', 51.3, 9);
    const shown = placeBadges([a, b], 1000);
    expect(shown.size).toBe(1);
    expect(placeBadges([mk('warning', 100, 99)], 1000).size).toBe(0);
    expect(placeBadges([mk('warning', 20, 1)], 1000).size).toBe(0);
  });

  it('merges close same-kind markers and orders paint by priority', () => {
    const layout = layoutLane([mk('fatal', 50), mk('warning', 50.2, 2), mk('warning', 50.4, 3), mk('custom', 10)], {
      plotWidth: 1000,
      priority: PRIORITY,
      compactKinds: COMPACT,
    });
    expect(layout.shapes.map((m) => m.cluster.kind)).toEqual(['custom', 'warning', 'fatal']);
    expect(layout.shapes.find((m) => m.cluster.kind === 'warning')!.cluster.count).toBe(5);
    expect(layout.ticks).toEqual([]);
    expect(layout.tickCount).toBe(0);
  });

  it('collapses warnings into ticks when a lane is busy, keeping other kinds as shapes', () => {
    const markers = [...Array.from({ length: 30 }, (_, i) => mk('warning', i * 3, 2)), mk('error', 1), mk('fatal', 80), mk('start', 0)];
    const layout = layoutLane(markers, { plotWidth: 1000, priority: PRIORITY, compactKinds: COMPACT, maxShapes: 20 });
    expect(layout.shapes.map((m) => m.cluster.kind).sort()).toEqual(['error', 'fatal', 'start']);
    expect(layout.ticks).toHaveLength(30);
    expect(layout.ticks[1]).toBeGreaterThan(at(layout.ticks, 0));
    expect(layout.tickCount).toBe(60);
  });

  it('keeps all shapes when under the threshold and tolerates a zero width', () => {
    const layout = layoutLane([mk('warning', 1), mk('warning', 90)], { plotWidth: 0, priority: PRIORITY, compactKinds: COMPACT });
    expect(layout.shapes.length).toBeGreaterThan(0);
    expect(layout.shapes.every((m) => m.showCount === false)).toBe(true);
  });
});

describe('marker hit targets (WCAG 2.5.8)', () => {
  const laid = (kind: string, left: number): LaidMarker => ({ ...mk(kind, left), showCount: false });
  const centres = (t: MarkerTarget) => t.members.map((g) => t.left + g.offset);

  /** Every target is at least 24px wide and none overlaps the next. */
  function expectValid(targets: MarkerTarget[]) {
    for (const [i, t] of targets.entries()) {
      expect(t.width).toBeGreaterThanOrEqual(TARGET_PX);
      expect(t.left).toBeGreaterThanOrEqual(0);
      const next = targets[i + 1];
      if (next) expect(t.left + t.width).toBeLessThanOrEqual(next.left);
    }
  }

  it('gives a lone marker a 24px target centred on its time', () => {
    const [t, ...rest] = layoutTargets([laid('error', 50)], 1000);
    expect(rest).toEqual([]);
    expect(t).toMatchObject({ left: 488, width: 24 });
    expect(centres(t!)).toEqual([500]);
    expect(t!.primary.cluster.kind).toBe('error');
  });

  it('keeps markers 24px or more apart as separate targets', () => {
    const targets = layoutTargets([laid('error', 10), laid('warning', 12.4), laid('fatal', 80)], 1000);
    expect(targets.map((t) => t.primary.cluster.kind)).toEqual(['error', 'warning', 'fatal']);
    expectValid(targets);
  });

  it('shares one target between closer markers, keeps each glyph at its time and activates the most important', () => {
    // Paint order (least important first): warning, error, fatal.
    const shapes = [laid('warning', 50.5), laid('error', 50.1), laid('fatal', 50)];
    const targets = layoutTargets(shapes, 1000);
    expect(targets).toHaveLength(1);
    const t = at(targets, 0);
    expect(t.primary.cluster.kind).toBe('fatal');
    expect(t.members.map((g) => g.marker.cluster.kind)).toEqual(['warning', 'error', 'fatal']);
    expect(centres(t).map(Math.round)).toEqual([505, 501, 500]);
    expect(t.left).toBe(488);
    expect(t.width).toBeCloseTo(29);
    expectValid(targets);
  });

  it('activates the earliest marker of the most important kind', () => {
    const t = at(layoutTargets([laid('error', 50.8), laid('error', 50)], 1000), 0);
    expect(t.primary.cluster.ts).toBe(50);
  });

  it('clamps targets to the plot start and never overlaps them, however dense the lane', () => {
    const shapes = Array.from({ length: 120 }, (_, i) => laid(i % 2 ? 'error' : 'exception', i * 0.7));
    const targets = layoutTargets(shapes, 1000);
    expect(at(targets, 0).left).toBe(0);
    expect(targets.reduce((n, t) => n + t.members.length, 0)).toBe(120);
    expectValid(targets);
    // A target never spans two targets' width, so a pointer can still tell regions apart.
    expect(Math.max(...targets.map((t) => t.width))).toBeLessThan(TARGET_PX * 2);
  });

  it('is part of the lane layout and empty for an empty lane', () => {
    const layout = layoutLane([mk('fatal', 50), mk('error', 90)], { plotWidth: 1000, priority: PRIORITY, compactKinds: COMPACT });
    expect(layout.targets.map((t) => t.primary.cluster.kind)).toEqual(['fatal', 'error']);
    expect(layoutTargets([], 1000)).toEqual([]);
  });
});
