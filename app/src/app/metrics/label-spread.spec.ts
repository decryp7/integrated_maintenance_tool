import { describe, expect, it } from 'vitest';
import { END_LABEL_GAP, spreadLabels } from './label-spread';

describe('spreadLabels', () => {
  it('leaves labels that are already apart', () => {
    expect(spreadLabels([10, 40, 80])).toEqual([0, 0, 0]);
  });

  it('separates labels that share a y by at least the gap, keeping their order', () => {
    const ys = [100, 100, 101, 130];
    const off = spreadLabels(ys);
    const placed = ys.map((y, i) => y + (off[i] ?? 0));
    const sorted = [...placed].sort((a, b) => a - b);
    for (let i = 1; i < sorted.length; i++) expect((sorted[i] ?? 0) - (sorted[i - 1] ?? 0)).toBeGreaterThanOrEqual(END_LABEL_GAP);
    expect(placed[0]).toBeLessThan(placed[1] ?? 0);
    expect(placed[1]).toBeLessThan(placed[2] ?? 0);
  });

  it('keeps the stack inside the plot bottom', () => {
    const ys = [196, 197, 198];
    const placed = ys.map((y, i) => y + (spreadLabels(ys, 13, 0, 200)[i] ?? 0));
    expect(Math.max(...placed)).toBeLessThanOrEqual(200);
    expect(placed[1]! - placed[0]!).toBeGreaterThanOrEqual(13);
    expect(placed[2]! - placed[1]!).toBeGreaterThanOrEqual(13);
  });

  it('handles none and one', () => {
    expect(spreadLabels([])).toEqual([]);
    expect(spreadLabels([5])).toEqual([0]);
  });
});
