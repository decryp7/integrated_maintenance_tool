import { describe, expect, it, vi } from 'vitest';
import { FlameGraph, FlameRect, FrameDto } from '../../core/api.types';
import { diffColor, FALLBACK_PALETTE, frameColor, inkFor, KIND_LABEL, KIND_SLOT, maxAbsDelta, mix, modulesByValue, PALETTE_TOKENS, resolvePalette, stickySlots } from './flame-colors';
import { depthAt, DrawOptions, drawFlame, fitLabel, FLAME_ROW_H, FlameCtx, rowY } from './flame-draw';
import { breadcrumbs, buildFlameModel, chainTo, extendFocus, FlameModel, flameKey, frameNamer, hitTest, topRects } from './flame-model';

const fr = (id: number, patch: Partial<FrameDto> = {}): FrameDto => ({ id, name: `Ns.Type.f${id}`, module: null, file: null, line: null, kind: 'managed', isFramework: false, ...patch });
const rc = (frame: number, depth: number, x: number, w: number, value = w * 100, patch: Partial<FlameRect> = {}): FlameRect => ({
  frame, parent: null, depth, x, w, value, selfValue: value / 2, matched: true, ...patch,
});
const under = (parent: number) => ({ parent });
const graph = (rects: FlameRect[], frames: FrameDto[] = [0, 1, 2, 3, 4, 5].map((i) => fr(i))): FlameGraph => ({
  rects, frames, focusFrames: [], focusValue: 100, grandTotal: 100, matchedValue: 100, matchedPct: 100, truncated: false, maxDepth: 3, metric: 'timeNs', measurement: 'sampled',
});

/**
 * 0: root [0,1]            depth 0
 *   1: a   [0,.6]          depth 1
 *     3: c [0,.2]          depth 2
 *     4: d [.2,.3]         depth 2 (bigger than c? .3 > .2)
 *   2: b   [.6,.4]         depth 1
 */
const sample = (): FlameModel =>
  buildFlameModel(graph([rc(0, 0, 0, 1, 100), rc(1, 1, 0, 0.6, 60, under(0)), rc(2, 1, 0.6, 0.4, 40, under(0)), rc(3, 2, 0, 0.2, 20, under(1)), rc(4, 2, 0.2, 0.3, 30, under(1))]));

describe('buildFlameModel', () => {
  it('links parents and children from the engine parent indexes, children largest first', () => {
    const m = sample();
    expect(m.parent).toEqual([-1, 0, 0, 1, 1]);
    expect(m.children[0]).toEqual([1, 2]);
    expect(m.children[1]).toEqual([4, 3]);
    expect(m.children[2]).toEqual([]);
    expect(m.byDepth).toEqual([[0], [1, 2], [3, 4]]);
    expect(m.maxDepth).toBe(2);
    expect(m.frames.get(3)?.id).toBe(3);
  });

  it('sorts a depth left to right regardless of input order', () => {
    const m = buildFlameModel(graph([rc(0, 0, 0, 1), rc(2, 1, 0.6, 0.4, 40, under(0)), rc(1, 1, 0, 0.6, 60, under(0))]));
    expect(m.byDepth[1]).toEqual([2, 1]);
    expect(m.parent).toEqual([-1, 0, 0]);
  });

  it('trusts the engine link, not the geometry: siblings that touch are not parents', () => {
    // Two adjacent same-depth frames whose edges meet: containment would guess wrong at the seam.
    const m = buildFlameModel(graph([rc(0, 0, 0, 0.5), rc(1, 0, 0.5, 0.5), rc(2, 1, 0.5, 0.25, 25, under(1))]));
    expect(m.parent).toEqual([-1, -1, 1]);
    expect(m.children[1]).toEqual([2]);
    expect(chainTo(m, 2)).toEqual([1, 2]);
  });

  it('ignores parent indexes outside the graph', () => {
    const m = buildFlameModel(graph([rc(0, 0, 0, 1), rc(1, 1, 0, 0.5, 50, under(7)), rc(2, 1, 0.5, 0.5, 50, under(-1))]));
    expect(m.parent).toEqual([-1, -1, -1]);
    expect(m.children[0]).toEqual([]);
  });

  it('handles empty graphs and gaps in depth', () => {
    const e = buildFlameModel(graph([]));
    expect(e.maxDepth).toBe(0);
    expect(e.rects).toEqual([]);
    const gap = buildFlameModel(graph([rc(0, 0, 0, 1), rc(1, 2, 0, 0.5)]));
    expect(gap.byDepth[1]).toEqual([]);
    expect(gap.parent).toEqual([-1, -1]);
    expect(gap.maxDepth).toBe(2);
  });
});

describe('chainTo / extendFocus / hitTest / topRects / breadcrumbs', () => {
  it('chainTo lists indices from depth 0', () => {
    const m = sample();
    expect(chainTo(m, 3)).toEqual([0, 1, 3]);
    expect(chainTo(m, 0)).toEqual([0]);
    expect(chainTo(m, -1)).toEqual([]);
  });

  it('extendFocus replaces the focused tip when already zoomed', () => {
    expect(extendFocus([], [1, 2])).toEqual([1, 2]);
    expect(extendFocus([9, 8], [8, 7])).toEqual([9, 8, 7]);
    const focus = [1];
    extendFocus(focus, [2]);
    expect(focus).toEqual([1]);
  });

  it('hitTest finds the rectangle at a point or null', () => {
    const m = sample();
    expect(hitTest(m, 0.1, 1)).toBe(1);
    expect(hitTest(m, 0.6, 1)).toBe(2); // right edge exclusive, left inclusive
    expect(hitTest(m, 0.25, 2)).toBe(4);
    expect(hitTest(m, 0.9, 2)).toBeNull();
    expect(hitTest(m, 0.1, 7)).toBeNull();
    expect(hitTest(m, 1, 0)).toBeNull();
  });

  it('topRects orders by value then shallower depth and limits', () => {
    const m = buildFlameModel(graph([rc(0, 0, 0, 1, 50), rc(1, 1, 0, 0.5, 50), rc(2, 1, 0.5, 0.5, 70)]));
    expect(topRects(m, 10)).toEqual([2, 0, 1]);
    expect(topRects(m, 1)).toEqual([2]);
  });

  it('frameNamer names ids from the answer (focus frames and rectangles), else frame #id', () => {
    const name = frameNamer({ frames: [fr(1)], focusFrames: [fr(7, { name: 'Zoomed' })] });
    expect([name(1), name(7), name(9)]).toEqual(['Ns.Type.f1', 'Zoomed', 'frame #9']);
    expect(frameNamer(null)(3)).toBe('frame #3');
  });

  it('breadcrumbs start with All and accumulate paths', () => {
    const names: Record<number, string> = { 5: 'five', 6: 'six' };
    expect(breadcrumbs([], (i) => names[i])).toEqual([{ label: 'All', path: [] }]);
    expect(breadcrumbs([5, 6], (i) => names[i])).toEqual([
      { label: 'All', path: [] },
      { label: 'five', path: [5] },
      { label: 'six', path: [5, 6] },
    ]);
  });
});

describe('flameKey', () => {
  const m = sample();
  const move = (index: number) => ({ type: 'move', index });

  it('Escape and Backspace zoom out even without a current rectangle or any rectangles', () => {
    for (const k of ['Escape', 'Backspace']) {
      expect(flameKey(m, null, k, 'flame')).toEqual({ type: 'zoomOut' });
      expect(flameKey(m, 3, k, 'icicle')).toEqual({ type: 'zoomOut' });
      expect(flameKey(buildFlameModel(graph([])), null, k, 'flame')).toEqual({ type: 'zoomOut' });
    }
  });

  it('returns null for every other key when there are no rectangles', () => {
    expect(flameKey(buildFlameModel(graph([])), null, 'ArrowUp', 'flame')).toBeNull();
    expect(flameKey(buildFlameModel(graph([])), 0, 'Enter', 'flame')).toBeNull();
  });

  it('with no (or a stale) current rectangle, navigation keys land on the first top-level frame', () => {
    for (const k of ['ArrowUp', 'ArrowDown', 'ArrowLeft', 'ArrowRight', 'Home']) {
      expect(flameKey(m, null, k, 'flame')).toEqual(move(0));
      expect(flameKey(m, 99, k, 'icicle')).toEqual(move(0));
    }
    expect(flameKey(m, null, 'Enter', 'flame')).toBeNull();
    expect(flameKey(m, null, 'x', 'flame')).toBeNull();
  });

  it('flame: ArrowUp goes to the largest child, ArrowDown to the parent', () => {
    expect(flameKey(m, 0, 'ArrowUp', 'flame')).toEqual(move(1));
    expect(flameKey(m, 1, 'ArrowUp', 'flame')).toEqual(move(4));
    expect(flameKey(m, 3, 'ArrowUp', 'flame')).toBeNull(); // leaf
    expect(flameKey(m, 3, 'ArrowDown', 'flame')).toEqual(move(1));
    expect(flameKey(m, 0, 'ArrowDown', 'flame')).toBeNull(); // root
  });

  it('icicle reverses the vertical arrows', () => {
    expect(flameKey(m, 0, 'ArrowDown', 'icicle')).toEqual(move(1));
    expect(flameKey(m, 3, 'ArrowDown', 'icicle')).toBeNull();
    expect(flameKey(m, 3, 'ArrowUp', 'icicle')).toEqual(move(1));
    expect(flameKey(m, 0, 'ArrowUp', 'icicle')).toBeNull();
  });

  it('Left/Right move to the neighbour at the same depth and stop at the edges', () => {
    expect(flameKey(m, 1, 'ArrowRight', 'flame')).toEqual(move(2));
    expect(flameKey(m, 2, 'ArrowRight', 'flame')).toBeNull();
    expect(flameKey(m, 2, 'ArrowLeft', 'flame')).toEqual(move(1));
    expect(flameKey(m, 1, 'ArrowLeft', 'flame')).toBeNull();
    expect(flameKey(m, 4, 'ArrowLeft', 'icicle')).toEqual(move(3));
  });

  it('Home goes to the first top-level frame', () => {
    expect(flameKey(m, 4, 'Home', 'flame')).toEqual(move(0));
  });

  it('Enter and space zoom into the current rectangle', () => {
    expect(flameKey(m, 4, 'Enter', 'flame')).toEqual({ type: 'zoom', index: 4 });
    expect(flameKey(m, 2, ' ', 'icicle')).toEqual({ type: 'zoom', index: 2 });
  });

  it('ignores unknown keys', () => {
    expect(flameKey(m, 1, 'q', 'flame')).toBeNull();
  });

  it('falls back to index 0 when depth 0 is empty', () => {
    const noRoot = buildFlameModel(graph([rc(0, 1, 0, 1)]));
    expect(flameKey(noRoot, null, 'Home', 'flame')).toEqual(move(0));
  });
});

describe('flame colours', () => {
  const p = FALLBACK_PALETTE;

  it('resolvePalette reads tokens, trims, and falls back when empty', () => {
    const seen: string[] = [];
    const full = resolvePalette((t) => {
      seen.push(t);
      return ` ${t}-v `;
    });
    expect(full.slots).toEqual(['--s1-v', '--s2-v', '--s3-v', '--s4-v']);
    expect(full.other).toBe('--s-other-v');
    expect(full.more).toBe('--lvl-error-v');
    expect(full.less).toBe('--s1-v');
    expect(full.neutral).toBe('--s-other-v');
    expect(full.outline).toBe('--text-v');
    expect(full.inkDark).toBe(p.inkDark);
    expect(full.inkLight).toBe(p.inkLight);
    expect(seen).toContain(PALETTE_TOKENS.outline);
    expect(resolvePalette(() => '  ')).toEqual(p);
  });

  it('exposes kind slots and labels', () => {
    expect(KIND_SLOT).toEqual({ managed: 0, native: 1, kernel: 3, unknown: null });
    expect(KIND_LABEL.kernel).toBe('Kernel');
  });

  describe('stickySlots', () => {
    it('keeps existing assignments and gives free slots in value order', () => {
      const out = stickySlots(new Map([['b', 1]]), ['a', 'b', 'c']);
      expect([...out]).toEqual([['b', 1], ['a', 0], ['c', 2]]);
    });
    it('does not mutate the previous map', () => {
      const prev = new Map([['x', 0]]);
      stickySlots(prev, ['y']);
      expect(prev.size).toBe(1);
    });
    it('folds overflow into other by leaving extra modules unassigned', () => {
      const out = stickySlots(new Map(), ['a', 'b', 'c', 'd', 'e', 'f']);
      expect([...out.keys()]).toEqual(['a', 'b', 'c', 'd']);
      expect(out.has('e')).toBe(false);
    });
    it('respects a custom slot count and keeps stale assignments', () => {
      const out = stickySlots(new Map([['old', 0]]), ['a', 'b'], 2);
      expect([...out]).toEqual([['old', 0], ['a', 1]]);
    });
  });

  it('modulesByValue sums self value per module, ties by name, ignoring module-less frames', () => {
    const rects = [rc(0, 0, 0, 1, 10, { selfValue: 5 }), rc(1, 1, 0, 0.5, 10, { selfValue: 5 }), rc(2, 1, 0.5, 0.5, 10, { selfValue: 8 }), rc(3, 1, 0.9, 0.1, 1, { selfValue: 1 }), rc(9, 1, 0.9, 0.1, 1, { selfValue: 100 })];
    const frames = [fr(0, { module: 'zeta' }), fr(1, { module: 'alpha' }), fr(2, { module: 'beta' }), fr(3, { module: null })];
    // alpha 5, zeta 5, beta 8; frame 9 is unknown
    expect(modulesByValue(buildFlameModel(graph(rects, frames)))).toEqual(['beta', 'alpha', 'zeta']);
  });

  describe('frameColor', () => {
    const slots = new Map([['m', 2]]);
    it('uses other for a missing frame', () => expect(frameColor(undefined, 'kind', slots, p)).toBe(p.other));
    it('colours by kind', () => {
      expect(frameColor(fr(1, { kind: 'managed' }), 'kind', slots, p)).toBe(p.slots[0]);
      expect(frameColor(fr(1, { kind: 'native' }), 'kind', slots, p)).toBe(p.slots[1]);
      expect(frameColor(fr(1, { kind: 'kernel' }), 'kind', slots, p)).toBe(p.slots[3]);
      expect(frameColor(fr(1, { kind: 'unknown' }), 'kind', slots, p)).toBe(p.other);
    });
    it('colours by module slot, with other for unassigned or module-less frames', () => {
      expect(frameColor(fr(1, { module: 'm' }), 'module', slots, p)).toBe(p.slots[2]);
      expect(frameColor(fr(1, { module: 'x' }), 'module', slots, p)).toBe(p.other);
      expect(frameColor(fr(1, { module: null }), 'module', slots, p)).toBe(p.other);
    });
  });

  describe('mix', () => {
    it('interpolates hex colours and clamps t', () => {
      expect(mix('#000000', '#ffffff', 0.5)).toBe('#808080');
      expect(mix('#000000', '#ffffff', 0)).toBe('#000000');
      expect(mix('#000000', '#ffffff', 2)).toBe('#ffffff');
      expect(mix('#000000', '#ffffff', -1)).toBe('#000000');
      expect(mix('000000', '#FF0000', 1)).toBe('#ff0000');
    });
    it('returns b at t >= 0.5 and a otherwise for non-hex inputs', () => {
      expect(mix('red', '#ffffff', 0.5)).toBe('#ffffff');
      expect(mix('red', '#ffffff', 0.49)).toBe('red');
      expect(mix('#ffffff', 'blue', 0.9)).toBe('blue');
      expect(mix('#fff', '#000000', 0.1)).toBe('#fff'); // short hex unsupported
    });
  });

  describe('diffColor', () => {
    it('is neutral without a delta, scale, or a negligible change', () => {
      expect(diffColor(undefined, 0.5, p)).toBe(p.neutral);
      expect(diffColor(0.2, 0, p)).toBe(p.neutral);
      expect(diffColor(0.00001, 0.5, p)).toBe(p.neutral);
      expect(diffColor(-0.00001, 0.5, p)).toBe(p.neutral);
    });
    it('goes red for growth and blue for shrinkage, saturating at maxAbs', () => {
      expect(diffColor(0.5, 0.5, p)).toBe(p.more);
      expect(diffColor(5, 0.5, p)).toBe(p.more);
      expect(diffColor(-0.5, 0.5, p)).toBe(p.less);
      expect(diffColor(0.25, 0.5, p)).toBe(mix(p.neutral, p.more, 0.5));
      expect(diffColor(-0.25, 0.5, p)).toBe(mix(p.neutral, p.less, 0.5));
    });
  });

  describe('inkFor', () => {
    it('uses dark ink on light fills and light ink on dark fills', () => {
      expect(inkFor('#ffffff', p)).toBe(p.inkDark);
      expect(inkFor('#0b0d12', p)).toBe(p.inkLight);
      expect(inkFor('#f2545b', p)).toBe(p.inkDark);
    });
    it('exercises both linearisation branches', () => {
      expect(inkFor('#030303', p)).toBe(p.inkLight); // channel <= 0.03928 * 255
    });
    it('uses light ink for non-hex backgrounds', () => {
      expect(inkFor('rgb(1,2,3)', p)).toBe(p.inkLight);
    });
  });

  it('maxAbsDelta is the largest absolute delta (0 when none)', () => {
    expect(maxAbsDelta(buildFlameModel(graph([rc(0, 0, 0, 1, 1, { delta: 0.1 }), rc(1, 1, 0, 1, 1, { delta: -0.4 }), rc(2, 1, 0, 1, 1)])))).toBe(0.4);
    expect(maxAbsDelta(buildFlameModel(graph([])))).toBe(0);
  });
});

describe('flame drawing', () => {
  const fakeCtx = () => {
    const calls: [string, ...unknown[]][] = [];
    const rec = (name: string) =>
      vi.fn((...args: unknown[]) => {
        calls.push([name, ...args]);
      });
    const ctx = {
      clearRect: rec('clearRect'), fillRect: rec('fillRect'), strokeRect: rec('strokeRect'), fillText: rec('fillText'),
      save: rec('save'), restore: rec('restore'), beginPath: rec('beginPath'), rect: rec('rect'), clip: rec('clip'),
      measureText: vi.fn((t: string) => ({ width: t.length * 6 })),
      fillStyle: '', strokeStyle: '', globalAlpha: 0, lineWidth: 0, font: '', textBaseline: 'alphabetic',
    };
    return { ctx: ctx as unknown as FlameCtx, calls, raw: ctx };
  };
  const opts = (patch: Partial<DrawOptions> = {}): DrawOptions => ({
    width: 1000, height: 100, orientation: 'icicle', color: () => '#123456', palette: FALLBACK_PALETTE, dimUnmatched: false, current: null, hovered: null, font: '12px x', ...patch,
  });

  it('rowY and depthAt are inverse in both orientations', () => {
    expect(rowY(2, 5, 'icicle')).toBe(2 * FLAME_ROW_H);
    expect(rowY(2, 5, 'flame')).toBe(3 * FLAME_ROW_H);
    expect(rowY(1, 3, 'flame', 10)).toBe(20);
    expect(depthAt(2 * FLAME_ROW_H + 1, 5, 'icicle')).toBe(2);
    expect(depthAt(3 * FLAME_ROW_H + 1, 5, 'flame')).toBe(2);
    expect(depthAt(-1, 5, 'icicle')).toBe(-1);
    expect(depthAt(25, 5, 'flame', 10)).toBe(3);
  });

  it('fitLabel returns the text, a truncation with an ellipsis, or nothing', () => {
    const { ctx } = fakeCtx();
    expect(fitLabel(ctx, 'abcdef', 36)).toBe('abcdef');
    expect(fitLabel(ctx, 'abcdefgh', 36)).toBe('abcde…');
    expect(fitLabel(ctx, 'abcdefgh', 12)).toBe('a…');
    expect(fitLabel(ctx, 'abcdefgh', 3)).toBe('');
  });

  it('clears, sets font/baseline, fills each rectangle and labels wide ones only', () => {
    const { ctx, calls, raw } = fakeCtx();
    const m = buildFlameModel(graph([rc(0, 0, 0, 1), rc(1, 1, 0, 0.5), rc(2, 1, 0.5, 0.01)], [fr(0, { name: 'Root' }), fr(1, { name: 'A.B.Child' }), fr(2, { name: 'Tiny' })]));
    drawFlame(ctx, m, opts());
    expect(calls[0]).toEqual(['clearRect', 0, 0, 1000, 100]);
    expect(raw.font).toBe('12px x');
    expect(raw.textBaseline).toBe('middle');
    const fills = calls.filter((c) => c[0] === 'fillRect');
    expect(fills).toHaveLength(3);
    expect(fills[0]).toEqual(['fillRect', 0, 0, 999, FLAME_ROW_H - 1]);
    expect(fills[1]).toEqual(['fillRect', 0, FLAME_ROW_H, 499, FLAME_ROW_H - 1]);
    const texts = calls.filter((c) => c[0] === 'fillText').map((c) => c[1]);
    expect(texts).toEqual(['Root', 'B.Child']); // 10px-wide rectangle gets no label
    expect(calls.filter((c) => c[0] === 'clip')).toHaveLength(2);
    expect(raw.globalAlpha).toBe(1);
    expect(calls.some((c) => c[0] === 'strokeRect')).toBe(false);
  });

  it('draws flame orientation with the root at the bottom', () => {
    const { ctx, calls } = fakeCtx();
    const m = buildFlameModel(graph([rc(0, 0, 0, 1), rc(1, 1, 0, 0.5)]));
    drawFlame(ctx, m, opts({ orientation: 'flame' }));
    const fills = calls.filter((c) => c[0] === 'fillRect');
    expect(fills[0][2]).toBe(FLAME_ROW_H);
    expect(fills[1][2]).toBe(0);
  });

  it('uses the frame id as a label for unknown frames and skips labels that cannot fit', () => {
    const { ctx, calls } = fakeCtx();
    drawFlame(ctx, buildFlameModel(graph([rc(42, 0, 0, 1), rc(43, 1, 0, 0.03)], [])), opts());
    // 30px wide: measure of "#43" = 18 <= 22 fits; 1000px root "#42"
    expect(calls.filter((c) => c[0] === 'fillText').map((c) => c[1])).toEqual(['#42', '#43']);
    const { ctx: c2, calls: calls2, raw } = fakeCtx();
    raw.measureText.mockImplementation(() => ({ width: 1000 }));
    drawFlame(c2, buildFlameModel(graph([rc(0, 0, 0, 1)])), opts({ width: 30 }));
    expect(calls2.filter((c) => c[0] === 'fillRect')).toHaveLength(1);
    expect(calls2.some((c) => c[0] === 'fillText')).toBe(false);
  });

  it('skips rectangles with no visible width and clips to the canvas', () => {
    const { ctx, calls } = fakeCtx();
    const m = buildFlameModel(graph([rc(0, 0, -0.5, 0.4), rc(1, 0, 1.2, 0.3), rc(2, 0, 0.9, 0.5)]));
    drawFlame(ctx, m, opts());
    const fills = calls.filter((c) => c[0] === 'fillRect');
    expect(fills).toHaveLength(1); // only the one overlapping the right edge, clipped
    expect(fills[0][1]).toBe(900);
    expect(fills[0][3]).toBe(99); // (1000 - 900) - gap
  });

  it('draws a minimum 0.5px width for slivers', () => {
    const { ctx, calls } = fakeCtx();
    drawFlame(ctx, buildFlameModel(graph([rc(0, 0, 0, 0.0005)])), opts());
    expect(calls.find((c) => c[0] === 'fillRect')?.[3]).toBe(0.5);
  });

  it('dims unmatched rectangles when searching and outlines the matched ones', () => {
    const { ctx, calls, raw } = fakeCtx();
    const alphas: number[] = [];
    Object.defineProperty(raw, 'globalAlpha', { get: () => alphas[alphas.length - 1], set: (v: number) => alphas.push(v) });
    const m = buildFlameModel(graph([rc(0, 0, 0, 1, 10, { matched: false }), rc(1, 1, 0, 0.5, 10, { matched: true }), rc(2, 1, 0.5, 0.001, 10, { matched: true })]));
    drawFlame(ctx, m, opts({ dimUnmatched: true }));
    expect(alphas).toEqual([0.3, 1, 1, 1]); // per-rect, then reset
    const outlines = calls.filter((c) => c[0] === 'strokeRect');
    expect(outlines).toHaveLength(1); // the sliver (< 2px) is not outlined, the unmatched is not outlined
    expect(outlines[0][2]).toBe(FLAME_ROW_H + 0.5);
  });

  it('does not dim when not searching', () => {
    const { ctx, raw } = fakeCtx();
    const alphas: number[] = [];
    Object.defineProperty(raw, 'globalAlpha', { get: () => alphas[alphas.length - 1], set: (v: number) => alphas.push(v) });
    drawFlame(ctx, buildFlameModel(graph([rc(0, 0, 0, 1, 10, { matched: false })])), opts());
    expect(alphas).toEqual([1, 1]);
  });

  it('outlines the hovered (1px) and current (2px) rectangles', () => {
    const { ctx, calls, raw } = fakeCtx();
    const widths: number[] = [];
    Object.defineProperty(raw, 'lineWidth', { get: () => widths[widths.length - 1], set: (v: number) => widths.push(v) });
    const m = buildFlameModel(graph([rc(0, 0, 0, 1), rc(1, 1, 0, 0.5)]));
    drawFlame(ctx, m, opts({ hovered: 0, current: 1 }));
    const strokes = calls.filter((c) => c[0] === 'strokeRect');
    expect(strokes).toHaveLength(2);
    expect(strokes[0]).toEqual(['strokeRect', 0.5, 0.5, 998, FLAME_ROW_H - 2]);
    expect(widths).toEqual([1, 2]);
    expect(raw.strokeStyle).toBe(FALLBACK_PALETTE.outline);
  });

  it('ignores an outline index outside the model', () => {
    const { ctx, calls } = fakeCtx();
    drawFlame(ctx, buildFlameModel(graph([rc(0, 0, 0, 1)])), opts({ hovered: 5, current: -1 }));
    expect(calls.some((c) => c[0] === 'strokeRect')).toBe(false);
  });

  it('asks the colour callback per rectangle index', () => {
    const { ctx, raw } = fakeCtx();
    const color = vi.fn((i: number) => (i === 0 ? '#ffffff' : '#000000'));
    drawFlame(ctx, buildFlameModel(graph([rc(0, 0, 0, 1), rc(1, 1, 0, 1)])), opts({ color }));
    expect(color).toHaveBeenCalledTimes(2);
    expect(raw.fillStyle).toBeDefined();
  });
});
