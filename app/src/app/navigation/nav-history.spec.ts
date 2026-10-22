import { NAV_LIMIT, NAV_MERGE_MS, NavHistory, NavPlace, samePlace } from './nav-history';

const ev = (id: number): NavPlace => ({ kind: 'event', id });
const range = (from: number | null, to: number | null): NavPlace => ({ kind: 'range', from, to });
const view = (id: string): NavPlace => ({ kind: 'view', id });

describe('samePlace', () => {
  it('compares events by id, views by id and ranges by both ends', () => {
    expect(samePlace(ev(1), ev(1))).toBe(true);
    expect(samePlace(ev(1), ev(2))).toBe(false);
    expect(samePlace(view('a'), view('a'))).toBe(true);
    expect(samePlace(view('a'), view('b'))).toBe(false);
    expect(samePlace(range(1, 2), range(1, 2))).toBe(true);
    expect(samePlace(range(1, 2), range(1, 3))).toBe(false);
    expect(samePlace(range(0, 2), range(1, 2))).toBe(false);
    expect(samePlace(range(null, null), range(null, null))).toBe(true);
    expect(samePlace(range(null, 2), range(0, 2))).toBe(false);
  });

  it('never equates places of different kinds', () => {
    expect(samePlace(ev(1), view('1'))).toBe(false);
    expect(samePlace(range(null, null), ev(1))).toBe(false);
    expect(samePlace(view('a'), range(1, 2))).toBe(false);
  });
});

describe('NavHistory', () => {
  it('starts empty', () => {
    const h = new NavHistory();
    expect(h.current).toBeNull();
    expect(h.size).toBe(0);
    expect(h.canBack).toBe(false);
    expect(h.canForward).toBe(false);
    expect(h.back()).toBeNull();
    expect(h.forward()).toBeNull();
  });

  it('records places, ignoring the current one again', () => {
    const h = new NavHistory();
    expect(h.record(ev(1), 0)).toBe(true);
    expect(h.record(ev(1), 10)).toBe(false);
    expect(h.record(view('events'), 20)).toBe(true);
    expect(h.size).toBe(2);
    expect(h.current).toEqual(view('events'));
    expect(h.canBack).toBe(true);
    expect(h.canForward).toBe(false);
  });

  it('goes back and forward and stops at the ends', () => {
    const h = new NavHistory();
    h.record(ev(1), 0);
    h.record(ev(2), 1);
    h.record(ev(3), 2);
    expect(h.back()).toEqual(ev(2));
    expect(h.back()).toEqual(ev(1));
    expect(h.back()).toBeNull();
    expect(h.current).toEqual(ev(1));
    expect(h.canForward).toBe(true);
    expect(h.forward()).toEqual(ev(2));
    expect(h.forward()).toEqual(ev(3));
    expect(h.forward()).toBeNull();
    expect(h.current).toEqual(ev(3));
  });

  it('recording after going back drops the places ahead', () => {
    const h = new NavHistory();
    h.record(ev(1), 0);
    h.record(ev(2), 1);
    h.record(ev(3), 2);
    h.back();
    h.back();
    h.record(ev(9), 3);
    expect(h.size).toBe(2);
    expect(h.canForward).toBe(false);
    expect(h.back()).toEqual(ev(1));
    expect(h.forward()).toEqual(ev(9));
  });

  it('re-recording the place it went back to adds nothing (what the watcher does)', () => {
    const h = new NavHistory();
    h.record(ev(1), 0);
    h.record(ev(2), 1);
    const place = h.back()!;
    expect(h.record(place, 2)).toBe(false);
    expect(h.canForward).toBe(true);
  });

  it('merges range places recorded within the merge window into one', () => {
    const h = new NavHistory();
    h.record(ev(1), 0);
    h.record(range(0, 10), 100);
    h.record(range(0, 8), 100 + NAV_MERGE_MS);
    h.record(range(0, 5), 100 + NAV_MERGE_MS * 2);
    expect(h.size).toBe(2);
    expect(h.current).toEqual(range(0, 5));
    expect(h.back()).toEqual(ev(1));
  });

  it('keeps range places apart after the merge window', () => {
    const h = new NavHistory();
    h.record(range(0, 10), 0);
    h.record(range(0, 5), NAV_MERGE_MS + 1);
    expect(h.size).toBe(2);
    expect(h.back()).toEqual(range(0, 10));
  });

  it('does not merge a range into a non-range place, nor when not at the newest place', () => {
    const h = new NavHistory();
    h.record(range(0, 10), 0);
    h.record(ev(1), 1);
    h.record(range(0, 5), 2);
    expect(h.size).toBe(3);
    h.back();
    h.back();
    h.record(range(1, 2), 3);
    expect(h.size).toBe(2);
    expect(h.current).toEqual(range(1, 2));
  });

  it('keeps at most NAV_LIMIT places, dropping the oldest', () => {
    const h = new NavHistory();
    for (let i = 0; i < NAV_LIMIT + 10; i++) h.record(ev(i), i);
    expect(h.size).toBe(NAV_LIMIT);
    expect(h.current).toEqual(ev(NAV_LIMIT + 9));
    for (let i = 0; i < NAV_LIMIT - 1; i++) h.back();
    expect(h.current).toEqual(ev(10));
    expect(h.canBack).toBe(false);
  });

  it('clears everything', () => {
    const h = new NavHistory();
    h.record(ev(1), 0);
    h.record(ev(2), 1);
    h.clear();
    expect(h.size).toBe(0);
    expect(h.current).toBeNull();
    expect(h.canBack).toBe(false);
    expect(h.canForward).toBe(false);
    expect(h.record(ev(1), 2)).toBe(true);
  });
});
