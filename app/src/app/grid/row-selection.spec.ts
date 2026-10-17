import { contains, count, EMPTY_SELECTION, extendTo, indices, normalize, shift, single, toggle, withFocus } from './row-selection';

describe('row selection', () => {
  it('normalizes ranges: sorts, merges overlapping and adjacent, drops empty ones', () => {
    expect(
      normalize([
        { start: 5, end: 6 },
        { start: 0, end: 1 },
        { start: 2, end: 3 },
        { start: 9, end: 8 },
        { start: 6, end: 7 },
      ]),
    ).toEqual([
      { start: 0, end: 3 },
      { start: 5, end: 7 },
    ]);
  });

  it('a single selection holds one row; a negative index selects nothing', () => {
    const s = single(4);
    expect(contains(s, 4)).toBe(true);
    expect(contains(s, 5)).toBe(false);
    expect(count(s)).toBe(1);
    expect(s.focus).toBe(4);
    expect(count(single(-1))).toBe(0);
    expect(count(EMPTY_SELECTION)).toBe(0);
  });

  it('toggle adds and removes rows, splitting ranges', () => {
    let s = toggle(single(2), 3);
    expect(indices(s, 10)).toEqual([2, 3]);
    s = toggle(s, 7);
    expect(indices(s, 10)).toEqual([2, 3, 7]);
    s = extendTo(single(0), 4);
    s = toggle(s, 2);
    expect(indices(s, 10)).toEqual([0, 1, 3, 4]);
    expect(s.anchor).toBe(2);
    s = toggle(toggle(single(1), 1), 1);
    expect(indices(s, 10)).toEqual([1]);
    expect(count(toggle(single(1), 1))).toBe(0);
  });

  it('extendTo selects from the anchor and replaces the previous shift range', () => {
    let s = extendTo(single(5), 8);
    expect(indices(s, 10)).toEqual([5, 6, 7, 8]);
    expect(s.focus).toBe(8);
    s = extendTo(s, 3);
    expect(indices(s, 10)).toEqual([3, 4, 5]);
    expect(s.anchor).toBe(5);
    expect(indices(extendTo(EMPTY_SELECTION, 2), 10)).toEqual([2]);
  });

  it('extendTo keeps rows toggled before the anchor was set', () => {
    const s = extendTo(toggle(single(0), 5), 6);
    expect(indices(s, 10)).toEqual([0, 5, 6]);
  });

  it('withFocus moves the focus only', () => {
    const s = withFocus(extendTo(single(1), 3), 2);
    expect(s.focus).toBe(2);
    expect(indices(s, 10)).toEqual([1, 2, 3]);
  });

  it('shift moves every index; an empty selection stays empty', () => {
    const s = shift(extendTo(toggle(single(0), 4), 5), 10);
    expect(indices(s, 10)).toEqual([10, 14, 15]);
    expect(s.anchor).toBe(14);
    expect(s.focus).toBe(15);
    expect(extendTo(s, 13).ranges).toEqual([
      { start: 10, end: 10 },
      { start: 13, end: 14 },
    ]);
    expect(shift(EMPTY_SELECTION, 3)).toBe(EMPTY_SELECTION);
  });

  it('indices stops at the limit and counts huge ranges without enumerating them', () => {
    const s = extendTo(single(0), 9_999_999);
    expect(count(s)).toBe(10_000_000);
    expect(indices(s, 3)).toEqual([0, 1, 2]);
  });
});
