import { DRAG_THRESHOLD, hitDockTarget, moveTab, popoutDragReduce, PopoutDrag, outsideViewport, pressTab, screenToClient, SETTLE_MS, targetPreview } from './dock-target';
import { at } from '../../testing/must';

const root = { x: 0, y: 0, width: 800, height: 600 };
const groups = [
  { id: 'g1', rect: { x: 100, y: 100, width: 400, height: 300 } },
  { id: 'tiny', rect: { x: 600, y: 100, width: 50, height: 50 } },
];

describe('hitDockTarget', () => {
  it('hits the compass of the group under the point first, then the edge guides', () => {
    expect(hitDockTarget({ x: 300, y: 250 }, root, groups)).toEqual({ kind: 'group', groupId: 'g1', position: 'center' });
    expect(hitDockTarget({ x: 336, y: 250 }, root, groups)).toEqual({ kind: 'group', groupId: 'g1', position: 'right' });
    expect(hitDockTarget({ x: 300, y: 214 }, root, groups)).toEqual({ kind: 'group', groupId: 'g1', position: 'top' });
    expect(hitDockTarget({ x: 26, y: 300 }, root, groups)).toEqual({ kind: 'edge', position: 'left' });
    expect(hitDockTarget({ x: 400, y: 574 }, root, groups)).toEqual({ kind: 'edge', position: 'bottom' });
  });

  it('is null off the guides and outside the dock', () => {
    expect(hitDockTarget({ x: 120, y: 120 }, root, groups)).toBeNull();
    expect(hitDockTarget({ x: 700, y: 500 }, root, groups)).toBeNull();
    expect(hitDockTarget({ x: 900, y: 300 }, root, groups)).toBeNull();
    // A group too small for a compass: only its centre (here none fits either).
    expect(hitDockTarget({ x: 625, y: 125 }, root, groups)).toEqual({ kind: 'group', groupId: 'tiny', position: 'center' });
  });

  it('previews the drop: a quarter of the dock for an edge, half (or all) of the group for its compass', () => {
    expect(targetPreview({ kind: 'edge', position: 'left' }, root, groups)).toEqual({ x: 0, y: 0, width: 200, height: 600 });
    expect(targetPreview({ kind: 'group', groupId: 'g1', position: 'center' }, root, groups)).toEqual(at(groups, 0).rect);
    expect(targetPreview({ kind: 'group', groupId: 'g1', position: 'bottom' }, root, groups)).toEqual({ x: 100, y: 250, width: 400, height: 150 });
    expect(targetPreview({ kind: 'group', groupId: 'gone', position: 'left' }, root, groups)).toBeNull();
  });

  it('converts screen points to client points', () => {
    expect(screenToClient({ x: 150, y: 80 }, { x: 100, y: 50 })).toEqual({ x: 50, y: 30 });
  });
});

describe('popoutDragReduce (a pop-out dragged over its host)', () => {
  const t = { kind: 'edge' as const, position: 'left' as const };
  const move = (state: PopoutDrag | null, now: number, target = t as typeof t | null, point = { x: 1, y: 1 } as { x: number; y: number } | null, label = 'p-main-1', settle = true) =>
    popoutDragReduce(state, { type: 'move', label, viewId: 'inspector', point, target, now, settle });

  it('never docks while merely hovering a guide when the release can be observed', () => {
    const s = move(null, 0, t, { x: 1, y: 1 }, 'p-main-1', false).state;
    expect(popoutDragReduce(s, { type: 'tick', now: 60_000 })).toEqual({ state: s, dock: null });
    expect(popoutDragReduce(s, { type: 'end', label: 'p-main-1', now: 60_001 }).dock).toEqual({ label: 'p-main-1', viewId: 'inspector', target: t });
  });

  it('docks after the pointer rests on a guide for the settle time where the release cannot be observed', () => {
    let s = move(null, 0).state;
    s = move(s, 100).state;
    expect(s!.since).toBe(0);
    expect(popoutDragReduce(s, { type: 'tick', now: SETTLE_MS - 1 }).dock).toBeNull();
    const done = popoutDragReduce(s, { type: 'tick', now: SETTLE_MS });
    expect(done).toEqual({ state: null, dock: { label: 'p-main-1', viewId: 'inspector', target: t } });
  });

  it('restarts the settle time when the target changes; no target means no dock', () => {
    let s = move(null, 0).state;
    s = move(s, 250, { kind: 'edge', position: 'right' } as never).state;
    expect(s!.since).toBe(250);
    expect(popoutDragReduce(s, { type: 'tick', now: 400 }).dock).toBeNull();
    s = move(s, 500, null).state;
    expect(popoutDragReduce(s, { type: 'tick', now: 5000 }).dock).toBeNull();
    // Outside the host: no target even if one was given.
    expect(move(null, 0, t, null).state!.target).toBeNull();
    expect(popoutDragReduce(null, { type: 'tick', now: 1 })).toEqual({ state: null, dock: null });
  });

  it('docks on release over a guide; a release elsewhere or of another window does nothing', () => {
    const s = move(null, 0).state;
    expect(popoutDragReduce(s, { type: 'end', label: 'p-main-9', now: 1 }).state).toBe(s);
    expect(popoutDragReduce(s, { type: 'end', label: 'p-main-1', now: 1 }).dock).toEqual({ label: 'p-main-1', viewId: 'inspector', target: t });
    const none = move(null, 0, null).state;
    expect(popoutDragReduce(none, { type: 'end', label: 'p-main-1', now: 1 })).toEqual({ state: null, dock: null });
    expect(popoutDragReduce(null, { type: 'end', label: 'p-main-1', now: 1 }).dock).toBeNull();
    expect(popoutDragReduce(s, { type: 'cancel' })).toEqual({ state: null, dock: null });
  });

  it('ignores a second window while the first is over the host', () => {
    const s = move(null, 0).state;
    expect(move(s, 10, t, { x: 2, y: 2 }, 'p-main-2').state).toBe(s);
  });
});

describe('tab drag out (pointer)', () => {
  const vp = { width: 1000, height: 800 };
  it('is a drag only past the threshold, and knows when the pointer left the window', () => {
    const d = pressTab('metrics', { x: 10, y: 10 }, { x: 110, y: 110 });
    expect(moveTab(d, { x: 10 + DRAG_THRESHOLD - 1, y: 10 }, { x: 0, y: 0 }).dragging).toBe(false);
    const dragged = moveTab(d, { x: 10 + DRAG_THRESHOLD, y: 10 }, { x: 0, y: 0 });
    expect(dragged.dragging).toBe(true);
    expect(outsideViewport({ x: -40, y: 10 }, vp)).toBe(true);
    expect(outsideViewport({ x: 500, y: 400 }, vp)).toBe(false);
    expect(outsideViewport({ x: 500, y: 801 }, vp)).toBe(true);
    expect(outsideViewport({ x: 1001, y: 400 }, vp)).toBe(true);
    expect(outsideViewport({ x: 500, y: -1 }, vp)).toBe(true);
    expect(outsideViewport({ x: -5, y: 400 }, vp, 10)).toBe(false);
    expect(outsideViewport({ x: 0, y: 0 }, vp)).toBe(false);
  });
});
