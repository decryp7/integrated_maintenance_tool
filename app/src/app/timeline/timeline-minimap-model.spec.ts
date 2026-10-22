import { centreOn, dragRange, fractionToTs, MIN_WINDOW_FRACTION, stepRange, windowRect } from './timeline-minimap-model';

const extent = { from: 1000, to: 11_000 };

describe('windowRect', () => {
  it('places the visible range within the extent as fractions', () => {
    expect(windowRect({ from: 3000, to: 6000 }, extent)).toEqual({ left: 0.2, width: 0.3 });
    expect(windowRect(extent, extent)).toEqual({ left: 0, width: 1 });
  });

  it('keeps a tiny range grabbable and inside the strip', () => {
    const tiny = windowRect({ from: 5000, to: 5001 }, extent);
    expect(tiny?.width).toBe(MIN_WINDOW_FRACTION);
    const atEnd = windowRect({ from: 11_000, to: 11_001 }, extent);
    expect(atEnd && atEnd.left + atEnd.width).toBeLessThanOrEqual(1);
  });

  it('clamps a range that sticks out and has nothing for an empty extent', () => {
    expect(windowRect({ from: -5000, to: 3000 }, extent)).toEqual({ left: 0, width: 0.2 });
    expect(windowRect({ from: 0, to: 1 }, { from: 5, to: 5 })).toBeNull();
  });
});

describe('gestures', () => {
  it('maps a fraction of the strip to a time, clamped to the extent', () => {
    expect(fractionToTs(0.5, extent)).toBe(6000);
    expect(fractionToTs(-1, extent)).toBe(1000);
    expect(fractionToTs(9, extent)).toBe(11_000);
  });

  it('centres a same-size window on a click and keeps it inside', () => {
    expect(centreOn(6000, { from: 1000, to: 3000 }, extent)).toEqual({ from: 5000, to: 7000 });
    expect(centreOn(1100, { from: 1000, to: 3000 }, extent)).toEqual({ from: 1000, to: 3000 });
    expect(centreOn(10_900, { from: 1000, to: 3000 }, extent)).toEqual({ from: 9000, to: 11_000 });
  });

  it('moves the whole window, or one edge, never past the extent or a minimum span', () => {
    const start = { from: 4000, to: 6000 };
    expect(dragRange('move', start, 1000, extent)).toEqual({ from: 5000, to: 7000 });
    expect(dragRange('move', start, 99_999, extent)).toEqual({ from: 9000, to: 11_000 });
    expect(dragRange('left', start, -1500, extent)).toEqual({ from: 2500, to: 6000 });
    expect(dragRange('left', start, -99_999, extent)).toEqual({ from: 1000, to: 6000 });
    expect(dragRange('left', start, 99_999, extent, 10)).toEqual({ from: 5990, to: 6000 });
    expect(dragRange('right', start, 1500, extent)).toEqual({ from: 4000, to: 7500 });
    expect(dragRange('right', start, 99_999, extent)).toEqual({ from: 4000, to: 11_000 });
    expect(dragRange('right', start, -99_999, extent, 10)).toEqual({ from: 4000, to: 4010 });
  });

  it('steps by a share of the window, stopping at the ends', () => {
    expect(stepRange({ from: 4000, to: 6000 }, extent, 1)).toEqual({ from: 4200, to: 6200 });
    expect(stepRange({ from: 4000, to: 6000 }, extent, -1, 0.5)).toEqual({ from: 3000, to: 5000 });
    expect(stepRange({ from: 1000, to: 3000 }, extent, -1)).toEqual({ from: 1000, to: 3000 });
  });
});
