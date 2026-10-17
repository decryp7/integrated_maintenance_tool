import {
  clamp,
  DOM_DELTA,
  firstRowAtScroll,
  makeGeometry,
  MAX_SPACER_PX,
  navigate,
  pageRows,
  rowAtTop,
  rowY,
  scrollToTop,
  topForRow,
  topToScroll,
  visibleWindow,
  wheelToPixels,
} from './scroll-map';

const ROW = 26;
const VIEW = 600;
const HUGE = 50_000_000;

describe('makeGeometry', () => {
  it('is the identity (native scrolling) while the content fits under the cap', () => {
    const g = makeGeometry(1000, ROW, VIEW);
    expect(g).toMatchObject({ virtualH: 26_000, physicalH: 26_000, maxScroll: 25_400, maxTop: 25_400, scaled: false });
  });

  it('caps the spacer and scales beyond it', () => {
    const g = makeGeometry(HUGE, ROW, VIEW);
    expect(g.virtualH).toBe(1_300_000_000);
    expect(g.physicalH).toBe(MAX_SPACER_PX);
    expect(g.maxScroll).toBe(MAX_SPACER_PX - VIEW);
    expect(g.maxTop).toBe(1_300_000_000 - VIEW);
    expect(g.scaled).toBe(true);
  });

  it('switches to scaling exactly past the cap', () => {
    const atCap = Math.floor(MAX_SPACER_PX / ROW);
    expect(makeGeometry(atCap, ROW, VIEW).scaled).toBe(false);
    expect(makeGeometry(atCap + 1, ROW, VIEW).scaled).toBe(true);
  });

  it('handles empty and tiny lists', () => {
    expect(makeGeometry(0, ROW, VIEW)).toMatchObject({ virtualH: 0, physicalH: 0, maxScroll: 0, maxTop: 0, scaled: false });
    expect(makeGeometry(3, ROW, VIEW)).toMatchObject({ virtualH: 78, maxScroll: 0, maxTop: 0, scaled: false });
    expect(makeGeometry(23, ROW, VIEW)).toMatchObject({ maxScroll: 0 });
    expect(makeGeometry(24, ROW, VIEW).maxScroll).toBe(24);
  });

  it('survives nonsense inputs', () => {
    expect(makeGeometry(NaN, ROW, VIEW).total).toBe(0);
    expect(makeGeometry(-5, ROW, VIEW).total).toBe(0);
    expect(makeGeometry(10.9, ROW, VIEW).total).toBe(10);
    expect(makeGeometry(10, 0, VIEW).rowH).toBe(1);
    expect(makeGeometry(10, NaN, VIEW).rowH).toBe(1);
    expect(makeGeometry(10, ROW, -3).viewportH).toBe(0);
    expect(makeGeometry(10, ROW, NaN).viewportH).toBe(0);
    expect(makeGeometry(10, ROW, VIEW, NaN).physicalH).toBe(260);
    expect(makeGeometry(10, ROW, VIEW, -1).physicalH).toBeGreaterThan(0);
  });

  it('never makes the scroll range negative, even with a cap below the viewport', () => {
    const g = makeGeometry(1000, ROW, VIEW, 100);
    expect(g.maxScroll).toBe(0);
    expect(g.physicalH).toBe(VIEW);
    expect(g.scaled).toBe(true);
    expect(scrollToTop(g, 5)).toBe(0);
    expect(topToScroll(g, 5)).toBe(0);
  });
});

describe('clamp', () => {
  it('clamps and maps NaN to the lower bound', () => {
    expect(clamp(5, 0, 10)).toBe(5);
    expect(clamp(-1, 0, 10)).toBe(0);
    expect(clamp(11, 0, 10)).toBe(10);
    expect(clamp(NaN, 2, 10)).toBe(2);
    expect(clamp(Infinity, 0, 10)).toBe(10);
    expect(clamp(-Infinity, 0, 10)).toBe(0);
  });
});

describe('scrollToTop / topToScroll', () => {
  const g = makeGeometry(HUGE, ROW, VIEW);

  it('maps the ends exactly', () => {
    expect(scrollToTop(g, 0)).toBe(0);
    expect(scrollToTop(g, g.maxScroll)).toBe(g.maxTop);
    expect(topToScroll(g, 0)).toBe(0);
    expect(topToScroll(g, g.maxTop)).toBe(g.maxScroll);
  });

  it('maps linearly in between', () => {
    expect(scrollToTop(g, g.maxScroll / 2)).toBeCloseTo(g.maxTop / 2, 3);
    expect(topToScroll(g, g.maxTop / 4)).toBeCloseTo(g.maxScroll / 4, 3);
  });

  it('clamps out-of-range and invalid input', () => {
    expect(scrollToTop(g, -50)).toBe(0);
    expect(scrollToTop(g, g.maxScroll + 1e9)).toBe(g.maxTop);
    expect(scrollToTop(g, NaN)).toBe(0);
    expect(topToScroll(g, -1)).toBe(0);
    expect(topToScroll(g, 1e15)).toBe(g.maxScroll);
    expect(topToScroll(g, NaN)).toBe(0);
  });

  it('round-trips within one scrollbar pixel everywhere', () => {
    for (let s = 0; s <= g.maxScroll; s += Math.floor(g.maxScroll / 997)) {
      expect(Math.abs(topToScroll(g, scrollToTop(g, s)) - s)).toBeLessThan(1e-3);
    }
  });

  it('is monotonic', () => {
    let prev = -1;
    for (let s = 0; s <= g.maxScroll; s += Math.floor(g.maxScroll / 1000)) {
      const t = scrollToTop(g, s);
      expect(t).toBeGreaterThanOrEqual(prev);
      prev = t;
    }
  });

  it('is the identity when not scaled', () => {
    const small = makeGeometry(1000, ROW, VIEW);
    expect(scrollToTop(small, 1234)).toBe(1234);
    expect(topToScroll(small, 1234)).toBe(1234);
    expect(scrollToTop(small, 1e9)).toBe(small.maxScroll);
  });

  it('is zero for lists that do not scroll', () => {
    const none = makeGeometry(5, ROW, VIEW);
    expect(scrollToTop(none, 100)).toBe(0);
    expect(topToScroll(none, 100)).toBe(0);
    const empty = makeGeometry(0, ROW, VIEW);
    expect(scrollToTop(empty, 100)).toBe(0);
  });
});

describe('firstRowAtScroll (round(scrollTop / maxScroll * (total - visibleRows)))', () => {
  const g = makeGeometry(HUGE, ROW, VIEW);
  const visibleRows = VIEW / ROW;

  it('is 0 at the top and total - visibleRows at the bottom', () => {
    expect(firstRowAtScroll(g, 0)).toBe(0);
    expect(firstRowAtScroll(g, g.maxScroll)).toBe(Math.round(HUGE - visibleRows));
  });

  it('is proportional in between', () => {
    expect(firstRowAtScroll(g, g.maxScroll / 2)).toBe(Math.round((HUGE - visibleRows) / 2));
    expect(firstRowAtScroll(g, g.maxScroll * 0.8)).toBe(Math.round((HUGE - visibleRows) * 0.8));
  });

  it('agrees with the pixel mapping', () => {
    for (const s of [0, 1, 12_345, 5_000_000, g.maxScroll - 1, g.maxScroll]) {
      expect(Math.abs(firstRowAtScroll(g, s) - scrollToTop(g, s) / ROW)).toBeLessThanOrEqual(0.5);
    }
  });

  it('clamps, and is 0 for lists that do not scroll', () => {
    expect(firstRowAtScroll(g, -5)).toBe(0);
    expect(firstRowAtScroll(g, 1e12)).toBe(firstRowAtScroll(g, g.maxScroll));
    expect(firstRowAtScroll(makeGeometry(0, ROW, VIEW), 10)).toBe(0);
    expect(firstRowAtScroll(makeGeometry(5, ROW, VIEW), 10)).toBe(0);
  });

  it('moves one row per row of scrolling when not scaled', () => {
    const small = makeGeometry(1000, ROW, VIEW);
    expect(firstRowAtScroll(small, ROW * 10)).toBe(10);
  });
});

describe('rowAtTop', () => {
  const g = makeGeometry(HUGE, ROW, VIEW);
  it('finds the row containing an offset', () => {
    expect(rowAtTop(g, 0)).toBe(0);
    expect(rowAtTop(g, ROW - 0.5)).toBe(0);
    expect(rowAtTop(g, ROW)).toBe(1);
    expect(rowAtTop(g, g.maxTop)).toBe(Math.floor(g.maxTop / ROW));
    expect(rowAtTop(g, 1e15)).toBe(Math.floor(g.maxTop / ROW));
    expect(rowAtTop(g, NaN)).toBe(0);
    expect(rowAtTop(makeGeometry(0, ROW, VIEW), 100)).toBe(0);
  });
});

describe('visibleWindow / rowY', () => {
  const g = makeGeometry(HUGE, ROW, VIEW);

  it('covers the viewport plus overscan', () => {
    const w = visibleWindow(g, 26 * 1000 + 10, 8);
    expect(w.first).toBe(992);
    expect(w.last).toBe(Math.ceil((26 * 1000 + 10 + VIEW) / ROW) + 8);
  });

  it('does not run past either end', () => {
    expect(visibleWindow(g, 0, 8).first).toBe(0);
    expect(visibleWindow(g, g.maxTop, 8).last).toBe(HUGE);
    expect(visibleWindow(makeGeometry(0, ROW, VIEW), 0, 8)).toEqual({ first: 0, last: 0 });
    expect(visibleWindow(makeGeometry(3, ROW, VIEW), 0, 8)).toEqual({ first: 0, last: 3 });
  });

  it('treats a bad overscan or offset as zero', () => {
    expect(visibleWindow(g, NaN, NaN)).toEqual({ first: 0, last: Math.ceil(VIEW / ROW) });
    expect(visibleWindow(g, 0, -4).first).toBe(0);
  });

  it('shows the last row at the bottom of the viewport at the end of the content', () => {
    const w = visibleWindow(g, g.maxTop, 0);
    expect(w.last).toBe(HUGE);
    expect(rowY(g, HUGE - 1, g.maxTop) + ROW).toBeCloseTo(VIEW, 6);
  });

  it('positions rows relative to the offset', () => {
    expect(rowY(g, 10, 100)).toBe(160);
    expect(rowY(g, 0, 0)).toBe(0);
  });
});

describe('topForRow', () => {
  const g = makeGeometry(HUGE, ROW, VIEW);
  const idx = 40_000_123;

  it('center lands the row in the middle of the viewport, exactly', () => {
    const top = topForRow(g, idx, 'center');
    expect(rowY(g, idx, top)).toBeCloseTo((VIEW - ROW) / 2, 6);
  });

  it('start puts the row on top', () => {
    expect(topForRow(g, idx, 'start')).toBe(idx * ROW);
  });

  it('nearest does not move a visible row, and scrolls the minimum otherwise', () => {
    const cur = 1000 * ROW;
    expect(topForRow(g, 1005, 'nearest', cur)).toBe(cur);
    expect(topForRow(g, 999, 'nearest', cur)).toBe(999 * ROW);
    expect(topForRow(g, 1000 + 22, 'nearest', cur)).toBe(cur);
    const below = topForRow(g, 1000 + 40, 'nearest', cur);
    expect(rowY(g, 1040, below) + ROW).toBe(VIEW);
  });

  it('clamps at both ends so the first and last rows land exactly', () => {
    expect(topForRow(g, 0, 'center')).toBe(0);
    expect(topForRow(g, 0, 'nearest', 5000)).toBe(0);
    expect(topForRow(g, HUGE - 1, 'center')).toBe(g.maxTop);
    expect(topForRow(g, HUGE - 1, 'nearest', 0)).toBe(g.maxTop);
    expect(topForRow(g, HUGE - 1, 'start')).toBe(g.maxTop);
    expect(topForRow(g, HUGE + 5, 'start')).toBe(g.maxTop);
    expect(topForRow(g, -5, 'start')).toBe(0);
    expect(topForRow(g, NaN, 'start')).toBe(0);
  });

  it('is 0 for an empty list', () => {
    expect(topForRow(makeGeometry(0, ROW, VIEW), 3, 'center')).toBe(0);
  });

  it('a row it reveals is inside the visible window', () => {
    for (const row of [0, 1, 17, 1_000_000, idx, HUGE - 2, HUGE - 1]) {
      const top = topForRow(g, row, 'center');
      const w = visibleWindow(g, top, 0);
      expect(row).toBeGreaterThanOrEqual(w.first);
      expect(row).toBeLessThan(w.last);
    }
  });

  it('survives the physical round trip within a scrollbar pixel (row precision comes from the virtual offset)', () => {
    const top = topForRow(g, idx, 'center');
    const back = scrollToTop(g, Math.round(topToScroll(g, top)));
    // One physical pixel covers this many rows: the reason programmatic positions are kept virtually.
    expect(Math.abs(back - top) / ROW).toBeLessThan(HUGE / MAX_SPACER_PX + 1);
  });
});

describe('pageRows / wheelToPixels', () => {
  const g = makeGeometry(HUGE, ROW, VIEW);

  it('pages by a screen minus a row, at least one', () => {
    expect(pageRows(g)).toBe(22);
    expect(pageRows(makeGeometry(100, ROW, 10))).toBe(1);
    expect(pageRows(makeGeometry(100, ROW, 0))).toBe(1);
  });

  it('converts every wheel mode to virtual pixels', () => {
    expect(wheelToPixels(53, DOM_DELTA.pixel, g)).toBe(53);
    expect(wheelToPixels(-2, DOM_DELTA.line, g)).toBe(-2 * ROW * 3);
    expect(wheelToPixels(1, DOM_DELTA.page, g)).toBe(VIEW - ROW);
    expect(wheelToPixels(1, DOM_DELTA.page, makeGeometry(10, ROW, 5))).toBe(ROW);
    expect(wheelToPixels(NaN, DOM_DELTA.pixel, g)).toBe(0);
    expect(wheelToPixels(10, 99, g)).toBe(10);
  });
});

describe('navigate', () => {
  const g = makeGeometry(HUGE, ROW, VIEW);
  const last = HUGE - 1;

  it('steps by rows and pages', () => {
    expect(navigate(g, 'ArrowDown', 10)).toBe(11);
    expect(navigate(g, 'ArrowUp', 10)).toBe(9);
    expect(navigate(g, 'PageDown', 10)).toBe(32);
    expect(navigate(g, 'PageUp', 100)).toBe(78);
  });

  it('goes to the ends', () => {
    expect(navigate(g, 'Home', 12345)).toBe(0);
    expect(navigate(g, 'End', 12345)).toBe(last);
  });

  it('stops at the ends', () => {
    expect(navigate(g, 'ArrowUp', 0)).toBe(0);
    expect(navigate(g, 'PageUp', 3)).toBe(0);
    expect(navigate(g, 'ArrowDown', last)).toBe(last);
    expect(navigate(g, 'PageDown', last - 2)).toBe(last);
  });

  it('starts at the first row when nothing is selected', () => {
    expect(navigate(g, 'ArrowDown', -1)).toBe(0);
    expect(navigate(g, 'ArrowUp', -1)).toBe(0);
    expect(navigate(g, 'PageDown', -1)).toBe(21);
    expect(navigate(g, 'PageUp', -1)).toBe(0);
  });

  it('pulls a stale selection back into range', () => {
    expect(navigate(makeGeometry(10, ROW, VIEW), 'ArrowDown', 500)).toBe(9);
  });

  it('ignores other keys and empty lists', () => {
    expect(navigate(g, 'a', 3)).toBeNull();
    expect(navigate(makeGeometry(0, ROW, VIEW), 'Home', -1)).toBeNull();
  });
});
