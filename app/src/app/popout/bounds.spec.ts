import { areaAt, centreIn, clampToWorkAreas, dragGrab, fitInto, intersection, MIN_POPOUT, placeAtPointer, placeNextToHost, sizeForPanel, WorkArea } from './bounds';

const wa = (x: number, name: string | null, primary = false, width = 1000): WorkArea => ({ name, primary, x, y: 0, width, height: 800 });
const A = wa(0, 'a');
const B = wa(1000, 'b', true);

describe('intersection', () => {
  it('returns the overlapping rectangle', () => {
    expect(intersection({ x: 0, y: 0, width: 100, height: 100 }, { x: 50, y: 60, width: 100, height: 100 })).toEqual({ x: 50, y: 60, width: 50, height: 40 });
  });
  it('has zero size when disjoint', () => {
    const r = intersection({ x: 0, y: 0, width: 10, height: 10 }, { x: 50, y: 50, width: 10, height: 10 });
    expect(r.width).toBe(0);
    expect(r.height).toBe(0);
  });
});

describe('fitInto', () => {
  const area = { x: 100, y: 100, width: 1000, height: 800 };
  it('leaves a rect that fits unchanged', () => {
    expect(fitInto({ x: 200, y: 200, width: 400, height: 300 }, area)).toEqual({ x: 200, y: 200, width: 400, height: 300 });
  });
  it('enforces the minimum size', () => {
    expect(fitInto({ x: 200, y: 200, width: 10, height: 10 }, area)).toEqual({ x: 200, y: 200, ...MIN_POPOUT });
  });
  it('shrinks to the area when larger', () => {
    expect(fitInto({ x: 0, y: 0, width: 5000, height: 5000 }, area)).toEqual(area);
  });
  it('moves in from each edge', () => {
    expect(fitInto({ x: 0, y: 0, width: 400, height: 300 }, area)).toMatchObject({ x: 100, y: 100 });
    expect(fitInto({ x: 5000, y: 5000, width: 400, height: 300 }, area)).toMatchObject({ x: 700, y: 600 });
  });
  it('rounds fractional values', () => {
    expect(fitInto({ x: 100.4, y: 100.6, width: 400.2, height: 300.7 }, area)).toEqual({ x: 100, y: 101, width: 400, height: 301 });
  });
});

describe('clampToWorkAreas', () => {
  const b = { x: 900, y: 100, width: 400, height: 300 };
  it('returns a copy unchanged without monitor information', () => {
    const r = clampToWorkAreas(b, []);
    expect(r).toEqual(b);
    expect(r).not.toBe(b);
  });
  it('fits to the named monitor when present', () => {
    expect(clampToWorkAreas({ x: 0, y: 0, width: 400, height: 300 }, [A, B], 'b')).toEqual({ x: 1000, y: 0, width: 400, height: 300 });
  });
  it('centres on the primary when the named monitor is gone', () => {
    expect(clampToWorkAreas(b, [A, B], 'gone')).toEqual({ x: 1300, y: 250, width: 400, height: 300 });
  });
  it('centres on the primary when nothing overlaps', () => {
    expect(clampToWorkAreas({ x: 5000, y: 5000, width: 400, height: 300 }, [A, B])).toEqual({ x: 1300, y: 250, width: 400, height: 300 });
  });
  it('picks the monitor with the larger overlap', () => {
    expect(clampToWorkAreas(b, [A, B])).toEqual({ x: 1000, y: 100, width: 400, height: 300 });
    expect(clampToWorkAreas({ x: 700, y: 100, width: 400, height: 300 }, [A, B])).toEqual({ x: 600, y: 100, width: 400, height: 300 });
  });
  it('falls back to the first area when none is primary', () => {
    expect(clampToWorkAreas({ x: 5000, y: 5000, width: 400, height: 300 }, [A, wa(1000, 'b')])).toEqual({ x: 300, y: 250, width: 400, height: 300 });
  });
});

describe('centreIn', () => {
  it('centres a rect', () => {
    expect(centreIn({ x: 0, y: 0, width: 400, height: 300 }, { x: 100, y: 0, width: 1000, height: 800 })).toEqual({ x: 400, y: 250, width: 400, height: 300 });
  });
  it('shrinks a rect larger than the area', () => {
    expect(centreIn({ x: 0, y: 0, width: 5000, height: 5000 }, A)).toEqual({ x: 0, y: 0, width: 1000, height: 800 });
  });
});

describe('areaAt', () => {
  it('is null without areas', () => {
    expect(areaAt({ x: 1, y: 1 }, [])).toBeNull();
  });
  it('finds the containing area', () => {
    expect(areaAt({ x: 10, y: 10 }, [A, B])).toBe(A);
    expect(areaAt({ x: 1000, y: 10 }, [A, B])).toBe(B);
  });
  it('falls back to the primary, or the first area', () => {
    expect(areaAt({ x: -50, y: -50 }, [A, B])).toBe(B);
    expect(areaAt({ x: -50, y: -50 }, [A, wa(1000, 'c')])).toBe(A);
  });
});

describe('sizeForPanel', () => {
  it('uses defaults for null', () => {
    expect(sizeForPanel(null)).toEqual({ width: 560, height: 420 });
  });
  it('adds window chrome to the height', () => {
    expect(sizeForPanel({ width: 600, height: 500 })).toEqual({ width: 600, height: 540 });
  });
  it('applies the minimums and rounds', () => {
    expect(sizeForPanel({ width: 100, height: 100 })).toEqual({ ...MIN_POPOUT });
    expect(sizeForPanel({ width: 400.4, height: 300.2 })).toEqual({ width: 400, height: 340 });
  });
});

describe('placeNextToHost', () => {
  const area = { x: 0, y: 0, width: 1920, height: 1080, name: null, primary: true };
  const size = { width: 400, height: 300 };
  it('goes right of the host when it fits', () => {
    expect(placeNextToHost({ x: 100, y: 100, width: 800, height: 600 }, size, [area])).toEqual({ x: 908, y: 140, ...size });
  });
  it('goes left when right does not fit', () => {
    expect(placeNextToHost({ x: 1200, y: 100, width: 700, height: 600 }, size, [area])).toEqual({ x: 792, y: 140, ...size });
  });
  it('overlaps the host when neither side fits', () => {
    const small = { x: 0, y: 0, width: 1000, height: 800, name: null, primary: true };
    expect(placeNextToHost({ x: 500, y: 100, width: 500, height: 600 }, { width: 600, height: 300 }, [small])).toEqual({ x: 376, y: 164, width: 600, height: 300 });
  });
  it('uses a default spot without a host', () => {
    expect(placeNextToHost(null, size, [])).toEqual({ x: 80, y: 80, ...size });
    expect(placeNextToHost(null, size, [area])).toEqual({ x: 80, y: 80, ...size });
  });
  it('places right of the host without monitor information', () => {
    expect(placeNextToHost({ x: 100, y: 100, width: 800, height: 600 }, size, [])).toEqual({ x: 908, y: 140, ...size });
  });
});

describe('placeAtPointer', () => {
  const area = { x: 0, y: 0, width: 1920, height: 1080, name: null, primary: true };
  const size = { width: 400, height: 300 };
  it('puts the title bar under the pointer', () => {
    expect(placeAtPointer({ x: 500, y: 400 }, size, [area])).toEqual({ x: 380, y: 388, ...size });
  });
  it('keeps the window on the monitor', () => {
    expect(placeAtPointer({ x: 1900, y: 1070 }, size, [area])).toEqual({ x: 1520, y: 780, ...size });
  });
  it('is not clamped without monitors', () => {
    expect(placeAtPointer({ x: 500, y: 400 }, size, [])).toEqual({ x: 380, y: 388, ...size });
  });
  it('holds the window where the drag ghost is held: in its title bar, near its left (half way on a narrow one)', () => {
    expect(dragGrab(size)).toEqual({ x: 120, y: 12 });
    expect(dragGrab({ width: 200, height: 100 })).toEqual({ x: 100, y: 12 });
  });
});
