import { Interval, packRows } from './sub-tracks';

const iv = (start: number, end: number): Interval => ({ start, end });

/** No two intervals on one row overlap (with the gap). */
function disjoint(items: Interval[], rows: number[], gap = 0): boolean {
  return items.every((a, i) => items.every((b, j) => i === j || rows[i] !== rows[j] || a.end + gap <= b.start || b.end + gap <= a.start));
}

/** The most intervals open at one moment: the fewest rows any packing can use. */
function depth(items: Interval[]): number {
  const edges = items.flatMap((i) => [[i.start, 1] as const, [i.end, -1] as const]).sort((a, b) => a[0] - b[0] || a[1] - b[1]);
  let open = 0;
  let max = 0;
  for (const [, d] of edges) max = Math.max(max, (open += d));
  return max;
}

describe('packRows', () => {
  it('keeps intervals that do not overlap on one row', () => {
    expect(packRows([iv(0, 10), iv(20, 30), iv(40, 50)])).toEqual({ rows: [0, 0, 0], count: 1, overflow: 0 });
  });

  it('stacks overlapping intervals (views opened and closed at different times) on rows of their own', () => {
    // A: 0–50, B: 10–20, C: 15–60, D: 25–30 (B is closed by then, so D reuses its row).
    const items = [iv(0, 50), iv(10, 20), iv(15, 60), iv(25, 30)];
    const p = packRows(items);
    expect(p.rows).toEqual([0, 1, 2, 1]);
    expect(p.count).toBe(3);
    expect(disjoint(items, p.rows)).toBe(true);
  });

  it('works in start order whatever the input order, and answers in input order', () => {
    const p = packRows([iv(25, 30), iv(15, 60), iv(0, 50), iv(10, 20)]);
    expect(p.rows).toEqual([1, 2, 0, 1]);
  });

  it('keeps a gap between neighbours on a row', () => {
    expect(packRows([iv(0, 10), iv(11, 20)], { gap: 4 }).rows).toEqual([0, 1]);
    expect(packRows([iv(0, 10), iv(14, 20)], { gap: 4 }).rows).toEqual([0, 0]);
    expect(packRows([iv(0, 10), iv(10, 20)]).rows).toEqual([0, 0]);
  });

  it('treats points and reversed intervals as points', () => {
    const p = packRows([iv(5, 5), iv(5, 5), iv(9, 2)]);
    expect(p.count).toBe(1);
    expect(packRows([iv(5, 5), iv(5, 5)], { gap: 1 }).count).toBe(2);
  });

  it('caps the rows: the rest share the row that frees up first and are counted', () => {
    const items = [iv(0, 100), iv(0, 50), iv(0, 80), iv(10, 20)];
    const p = packRows(items, { maxRows: 2 });
    expect(p.count).toBe(2);
    expect(p.overflow).toBe(2);
    expect(p.rows).toEqual([0, 0, 1, 1]);
    expect(packRows(items, { maxRows: 0 }).count).toBe(1);
  });

  it('is empty for no intervals', () => {
    expect(packRows([])).toEqual({ rows: [], count: 0, overflow: 0 });
  });

  it('uses the fewest rows possible and never overlaps (random intervals)', () => {
    let seed = 7;
    const rnd = () => ((seed = (seed * 16807) % 2147483647) / 2147483647) * 1000;
    for (let round = 0; round < 50; round++) {
      const items = Array.from({ length: 40 }, () => {
        const a = rnd();
        return iv(a, a + rnd() / 8);
      });
      const p = packRows(items);
      expect(disjoint(items, p.rows)).toBe(true);
      expect(p.count).toBe(depth(items));
    }
  });
});
