import { HIST_STACK, HIST_VB, histogramPaths } from './histogram';

/** [Verbose, Debug, Information, Warning, Error, Fatal] */
const b = (info: number, warn = 0, error = 0, fatal = 0): number[] => [0, 0, info, warn, error, fatal];

/** The y extents of a path's bars. */
function bars(d: string): { x: number; y: number; h: number }[] {
  return [...d.matchAll(/M([\d.]+) ([\d.]+)h[\d.]+v([\d.]+)/g)].map((m) => ({ x: Number(m[1]), y: Number(m[2]), h: Number(m[3]) }));
}

describe('histogramPaths', () => {
  it('scales to the busiest bucket and stacks the most severe level on the baseline', () => {
    const { paths, max } = histogramPaths([b(90, 10), b(0, 0, 5)]);
    expect(max).toBe(100);
    expect(paths.map((p) => p.level)).toEqual(['Error', 'Warning', 'Information']);
    const [warn] = bars(paths.find((p) => p.level === 'Warning')!.d);
    const [info] = bars(paths.find((p) => p.level === 'Information')!.d);
    // Warnings start at the baseline, info sits on top of them, the busiest bucket reaches the top padding.
    expect(warn!.y + warn!.h).toBeCloseTo(HIST_VB);
    expect(info!.y + info!.h).toBeCloseTo(warn!.y);
    expect(info!.y).toBeCloseTo(4);
    expect(paths[0]?.color).toBe('var(--lvl-error)');
    expect(paths.find((p) => p.level === 'Information')?.color).toBe('var(--hist-info)');
  });

  it('keeps a single error visible next to a tall bucket', () => {
    const { paths } = histogramPaths([b(10_000), b(0, 0, 1)]);
    const [err] = bars(paths.find((p) => p.level === 'Error')!.d);
    expect(err!.h).toBeGreaterThanOrEqual(3);
    expect(err!.x).toBeGreaterThan(1);
  });

  it('never draws above the top, even with minimum heights', () => {
    const { paths } = histogramPaths([b(0, 1, 1, 1)]);
    for (const p of paths) for (const bar of bars(p.d)) expect(bar.y).toBeGreaterThanOrEqual(0);
  });

  it('is empty without events', () => {
    expect(histogramPaths([])).toEqual({ paths: [], max: 0 });
    expect(histogramPaths([b(0)]).paths).toEqual([]);
    expect(HIST_STACK[0]).toBe('Fatal');
  });
});
