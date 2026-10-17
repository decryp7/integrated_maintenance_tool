import {
  adjacentNeighbour,
  compassCells,
  DropPosition,
  edgeCells,
  hitGuide,
  isLayoutTarget,
  nearestSide,
  previewRect,
  Rect,
  resolveGuideDrop,
} from './dock-geometry';

const r = (x: number, y: number, width: number, height: number): Rect => ({ x, y, width, height });
const ALL = new Set(['center', 'left', 'right', 'top', 'bottom']);

describe('adjacentNeighbour', () => {
  const me = r(0, 0, 100, 100);

  it('reports the side of the neighbour the group sat on', () => {
    expect(adjacentNeighbour(me, [{ rect: r(100, 0, 100, 100), value: 'a' }])).toEqual({ value: 'a', direction: 'left' });
    expect(adjacentNeighbour(me, [{ rect: r(-100, 0, 100, 100), value: 'a' }])).toEqual({ value: 'a', direction: 'right' });
    expect(adjacentNeighbour(me, [{ rect: r(0, 100, 100, 100), value: 'a' }])).toEqual({ value: 'a', direction: 'above' });
    expect(adjacentNeighbour(me, [{ rect: r(0, -100, 100, 100), value: 'a' }])).toEqual({ value: 'a', direction: 'below' });
  });

  it('accepts a gap within the tolerance but not beyond it', () => {
    expect(adjacentNeighbour(me, [{ rect: r(105, 0, 50, 100), value: 'a' }])?.value).toBe('a');
    expect(adjacentNeighbour(me, [{ rect: r(107, 0, 50, 100), value: 'a' }])).toBeNull();
    expect(adjacentNeighbour(me, [{ rect: r(120, 0, 50, 100), value: 'a' }], 25)?.value).toBe('a');
  });

  it('returns null without neighbours, or when they only touch at a corner', () => {
    expect(adjacentNeighbour(me, [])).toBeNull();
    expect(adjacentNeighbour(me, [{ rect: r(100, 100, 50, 50), value: 'corner' }])).toBeNull();
  });

  it('prefers the neighbour sharing the longest edge', () => {
    const others = [
      { rect: r(100, 0, 100, 30), value: 'short' },
      { rect: r(-100, 10, 100, 80), value: 'long' },
    ];
    expect(adjacentNeighbour(me, others)).toEqual({ value: 'long', direction: 'right' });
  });
});

describe('nearestSide', () => {
  const root = r(0, 0, 1000, 800);

  it('picks the closest of left, right and bottom', () => {
    expect(nearestSide(r(0, 100, 200, 200), root)).toBe('left');
    expect(nearestSide(r(800, 100, 200, 200), root)).toBe('right');
    expect(nearestSide(r(400, 600, 200, 200), root)).toBe('bottom');
  });

  it('prefers the sides on ties', () => {
    expect(nearestSide(r(400, 0, 200, 200), root)).toBe('left');
    expect(nearestSide(r(800, 600, 200, 200), root)).toBe('right');
  });

  it('honours the root offset', () => {
    expect(nearestSide(r(1000, 0, 100, 100), r(1000, 0, 500, 500))).toBe('left');
  });
});

describe('guide cells', () => {
  it('lays out five cells centred in a large group', () => {
    const cells = compassCells(200, 200);
    expect(cells.map((c) => c.position)).toEqual(['center', 'left', 'right', 'top', 'bottom']);
    expect(cells[0].rect).toEqual(r(84, 84, 32, 32));
    expect(cells[1].rect).toEqual(r(48, 84, 32, 32));
    expect(cells[2].rect).toEqual(r(120, 84, 32, 32));
    expect(cells[3].rect).toEqual(r(84, 48, 32, 32));
    expect(cells[4].rect).toEqual(r(84, 120, 32, 32));
  });

  it('keeps only the centre in a small group and nothing in a tiny one', () => {
    expect(compassCells(60, 60)).toEqual([{ position: 'center', rect: r(14, 14, 32, 32) }]);
    expect(compassCells(200, 60).map((c) => c.position)).toEqual(['center']);
    expect(compassCells(20, 200)).toEqual([]);
    expect(compassCells(200, 31)).toEqual([]);
  });

  it('puts one edge cell at the middle of each edge', () => {
    const cells = edgeCells(200, 200);
    expect(cells.map((c) => [c.position, c.rect.x, c.rect.y])).toEqual([
      ['left', 10, 84],
      ['right', 158, 84],
      ['top', 84, 10],
      ['bottom', 84, 158],
    ]);
  });

  it('has no edge cells when the area is tiny', () => {
    expect(edgeCells(90, 200)).toEqual([]);
    expect(edgeCells(200, 90)).toEqual([]);
  });
});

describe('hitGuide', () => {
  const cells = compassCells(200, 200);

  it('finds the cell under a point, edges included', () => {
    expect(hitGuide(cells, { x: 100, y: 100 })).toBe('center');
    expect(hitGuide(cells, { x: 48, y: 84 })).toBe('left');
    expect(hitGuide(cells, { x: 152, y: 116 })).toBe('right');
  });

  it('is null in the gaps and outside', () => {
    expect(hitGuide(cells, { x: 82, y: 100 })).toBeNull();
    expect(hitGuide(cells, { x: 0, y: 0 })).toBeNull();
    expect(hitGuide([], { x: 1, y: 1 })).toBeNull();
  });
});

describe('resolveGuideDrop', () => {
  const box = { width: 200, height: 200, zones: ALL };
  const group: [number, number, DropPosition][] = [
    [100, 100, 'center'],
    [60, 100, 'left'],
    [130, 100, 'right'],
    [100, 60, 'top'],
    [100, 130, 'bottom'],
  ];

  it.each(group)('over a group at (%i,%i) docks %s', (x, y, position) => {
    expect(resolveGuideDrop({ ...box, x, y }, false)).toEqual({ position });
  });

  it('is null off the guides', () => {
    expect(resolveGuideDrop({ ...box, x: 5, y: 5 }, false)).toBeNull();
  });

  it('is null when the target does not accept the zone', () => {
    expect(resolveGuideDrop({ ...box, x: 100, y: 100, zones: new Set(['left']) }, false)).toBeNull();
    expect(resolveGuideDrop({ ...box, x: 60, y: 100, zones: new Set(['left']) }, false)).toEqual({ position: 'left' });
  });

  it('uses only the edge cells for the layout root', () => {
    expect(resolveGuideDrop({ ...box, x: 20, y: 100 }, true)).toEqual({ position: 'left' });
    expect(resolveGuideDrop({ ...box, x: 100, y: 20 }, true)).toEqual({ position: 'top' });
    expect(resolveGuideDrop({ ...box, x: 100, y: 170 }, true)).toEqual({ position: 'bottom' });
    expect(resolveGuideDrop({ ...box, x: 170, y: 100 }, true)).toEqual({ position: 'right' });
    expect(resolveGuideDrop({ ...box, x: 100, y: 100 }, true)).toBeNull();
    expect(resolveGuideDrop({ ...box, x: 20, y: 100, zones: new Set(['right']) }, true)).toBeNull();
  });

  it('routes an edge guide under a group to the layout edge (edge: true), given where the pointer is in the dock', () => {
    const dock = { width: 800, height: 600 };
    expect(resolveGuideDrop({ ...box, x: 5, y: 100 }, false, { x: 26, y: 300 }, dock)).toEqual({ position: 'left', edge: true });
    expect(resolveGuideDrop({ ...box, x: 5, y: 100 }, false, { x: 400, y: 300 }, dock)).toBeNull();
    // The group's own compass wins.
    expect(resolveGuideDrop({ ...box, x: 100, y: 100 }, false, { x: 26, y: 300 }, dock)).toEqual({ position: 'center' });
  });

  it('does not offer edge guides to a group, nor compass guides to the layout', () => {
    expect(resolveGuideDrop({ ...box, x: 20, y: 100 }, false)).toBeNull();
    expect(resolveGuideDrop({ ...box, x: 60, y: 100 }, true)).toBeNull();
  });
});

describe('isLayoutTarget', () => {
  it('matches a target the size of the root within the tolerance', () => {
    const root = { width: 1000, height: 800 };
    expect(isLayoutTarget({ width: 1000, height: 800 }, root)).toBe(true);
    expect(isLayoutTarget({ width: 1002, height: 798 }, root)).toBe(true);
    expect(isLayoutTarget({ width: 1003, height: 800 }, root)).toBe(false);
    expect(isLayoutTarget({ width: 500, height: 800 }, root)).toBe(false);
    expect(isLayoutTarget({ width: 1003, height: 800 }, root, 5)).toBe(true);
  });

  it('is false without a root', () => {
    expect(isLayoutTarget({ width: 1, height: 1 }, null)).toBe(false);
  });
});

describe('previewRect', () => {
  const box = r(10, 20, 200, 100);

  it('gives the rectangle each position would occupy', () => {
    expect(previewRect('center', box)).toEqual(box);
    expect(previewRect('left', box)).toEqual(r(10, 20, 100, 100));
    expect(previewRect('right', box)).toEqual(r(110, 20, 100, 100));
    expect(previewRect('top', box)).toEqual(r(10, 20, 200, 50));
    expect(previewRect('bottom', box)).toEqual(r(10, 70, 200, 50));
  });

  it('honours the fraction and does not alias the box', () => {
    expect(previewRect('left', box, 0.25)).toEqual(r(10, 20, 50, 100));
    expect(previewRect('right', box, 0.25)).toEqual(r(160, 20, 50, 100));
    expect(previewRect('center', box)).not.toBe(box);
  });
});
