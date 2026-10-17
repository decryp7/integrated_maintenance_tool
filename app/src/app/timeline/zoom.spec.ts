import { TestBed } from '@angular/core/testing';
import { provideTinyBackend, settle, SEC, T0 } from '../../testing/helpers';
import { WorkspaceStore } from '../core/workspace.store';
import { clampRange, coversExtent, pan, RangeHistory, span, TimelineZoom, wheelFactor, xToTs, zoomAt } from './zoom';

const EXT = { from: 0, to: 1000 };

describe('zoom math', () => {
  it('clamps ranges into the extent and enforces a minimum span', () => {
    expect(clampRange({ from: -50, to: 50 }, EXT)).toEqual({ from: 0, to: 100 });
    expect(clampRange({ from: 950, to: 1050 }, EXT)).toEqual({ from: 900, to: 1000 });
    expect(clampRange({ from: -10, to: 5000 }, EXT)).toEqual({ from: 0, to: 1000 });
    expect(clampRange({ from: 10, to: 10 }, EXT)).toEqual({ from: 10, to: 11 });
    expect(clampRange({ from: 5, to: 5 }, { from: 5, to: 5 })).toEqual({ from: 5, to: 6 });
  });

  it('detects full coverage', () => {
    expect(coversExtent({ from: 0, to: 1000 }, EXT)).toBe(true);
    expect(coversExtent({ from: 1, to: 1000 }, EXT)).toBe(false);
  });

  it('zooms around the anchor keeping its screen position', () => {
    const r = zoomAt({ from: 0, to: 1000 }, 250, 0.5, EXT);
    expect(r).toEqual({ from: 125, to: 625 });
    expect((250 - r.from) / span(r)).toBeCloseTo(0.25);
    expect(zoomAt({ from: 100, to: 200 }, 150, 4, EXT)).toEqual({ from: 0, to: 400 });
    expect(zoomAt({ from: 0, to: 1000 }, 500, 3, EXT)).toEqual(EXT);
    expect(span(zoomAt({ from: 10, to: 12 }, 11, 0.0001, EXT))).toBe(1);
    expect(zoomAt({ from: 10, to: 10 }, 10, 2, EXT).from).toBeGreaterThanOrEqual(0);
  });

  it('pans without leaving the extent', () => {
    expect(pan({ from: 100, to: 200 }, 50, EXT)).toEqual({ from: 150, to: 250 });
    expect(pan({ from: 100, to: 200 }, -500, EXT)).toEqual({ from: 0, to: 100 });
    expect(pan({ from: 800, to: 900 }, 500, EXT)).toEqual({ from: 900, to: 1000 });
  });

  it('maps pixels to time and wheel deltas to factors', () => {
    expect(xToTs(50, 100, { from: 0, to: 1000 })).toBe(500);
    expect(xToTs(50, 0, { from: 10, to: 1000 })).toBe(10);
    expect(wheelFactor(0)).toBe(1);
    expect(wheelFactor(-100)).toBeLessThan(1);
    expect(wheelFactor(100)).toBeGreaterThan(1);
    expect(wheelFactor(1, 1)).toBeCloseTo(wheelFactor(16));
    expect(wheelFactor(1, 2)).toBeCloseTo(wheelFactor(400));
  });
});

describe('RangeHistory', () => {
  it('walks back and forward and truncates on new pushes', () => {
    const h = new RangeHistory();
    expect(h.current).toBeNull();
    expect(h.canBack).toBe(false);
    h.push({ from: 1, to: 2 });
    h.push({ from: 1, to: 2 });
    h.push({ from: 3, to: 4 });
    expect(h.back()).toEqual({ from: 1, to: 2 });
    expect(h.back()).toBeNull();
    expect(h.back()).toBeNull();
    expect(h.canForward).toBe(true);
    expect(h.forward()).toEqual({ from: 1, to: 2 });
    h.push(null);
    expect(h.canForward).toBe(false);
    expect(h.forward()).toBeNull();
    h.push(null);
    expect(h.back()).toEqual({ from: 1, to: 2 });
  });

  it('keeps at most 100 entries', () => {
    const h = new RangeHistory();
    for (let i = 0; i < 150; i++) h.push({ from: i, to: i + 1 });
    let n = 0;
    while (h.canBack) {
      h.back();
      n++;
    }
    expect(n).toBe(99);
  });
});

describe('TimelineZoom', () => {
  async function setup() {
    TestBed.configureTestingModule({ providers: [provideTinyBackend()] });
    const store = TestBed.inject(WorkspaceStore);
    const zoom = TestBed.inject(TimelineZoom);
    await settle();
    return { store, zoom };
  }

  it('derives extent and view from the summary and query', async () => {
    const { store, zoom } = await setup();
    expect(zoom.extent()).toEqual({ from: T0, to: T0 + 500 * SEC });
    expect(zoom.view()).toEqual(zoom.extent());
    store.setRange(T0 + 10 * SEC, null);
    expect(zoom.view()).toEqual({ from: T0 + 10 * SEC, to: T0 + 500 * SEC });
  });

  it('with a live window, the view is what the engine resolved, and it can be panned', async () => {
    const { store, zoom } = await setup();
    expect(zoom.canPan()).toBe(false);
    store.patchQuery({ window: { kind: 'live', lastMs: 100 * SEC } });
    await vi.waitFor(() => expect(store.timeline()!.from).toBe(T0 + 400 * SEC));
    expect(zoom.view()).toEqual({ from: T0 + 400 * SEC, to: T0 + 500 * SEC });
    expect(zoom.canPan()).toBe(true);
    zoom.panBy(-10 * SEC, false);
    expect(store.query().from).toBe(T0 + 390 * SEC);
  });

  it('has no view before data loads', () => {
    TestBed.configureTestingModule({ providers: [provideTinyBackend()] });
    const zoom = TestBed.inject(TimelineZoom);
    expect(zoom.extent()).toBeNull();
    expect(zoom.view()).toBeNull();
    zoom.zoomIn();
    zoom.wheelZoom(0.5, 0);
    zoom.panBy(10);
    expect(TestBed.inject(WorkspaceStore).hasRange()).toBe(false);
  });

  it('applies, clamps and fits ranges with history', async () => {
    const { store, zoom } = await setup();
    zoom.apply({ from: T0 - 100 * SEC, to: T0 + 100 * SEC });
    expect(store.query().from).toBe(T0);
    expect(store.query().to).toBe(T0 + 200 * SEC);
    expect(zoom.isZoomed()).toBe(true);
    expect(zoom.canBack()).toBe(true);
    zoom.apply({ from: T0 - SEC, to: T0 + 9999 * SEC });
    expect(store.hasRange()).toBe(false);
    zoom.back();
    expect(store.query().from).toBe(T0);
    expect(zoom.canForward()).toBe(true);
    zoom.forward();
    expect(store.hasRange()).toBe(false);
    zoom.zoomIn();
    expect(store.query().to! - store.query().from!).toBe(250 * SEC);
    zoom.zoomOut();
    expect(store.hasRange()).toBe(false);
    zoom.zoomIn(T0);
    expect(store.query().from).toBe(T0);
    zoom.fit();
    expect(store.hasRange()).toBe(false);
  });

  it('debounces wheel zoom and pans only when zoomed', async () => {
    const { store, zoom } = await setup();
    vi.useFakeTimers();
    try {
      zoom.panBy(10 * SEC);
      vi.advanceTimersByTime(200);
      expect(store.hasRange()).toBe(false);
      zoom.wheelZoom(0.5, T0);
      zoom.wheelZoom(0.5, T0);
      expect(zoom.pendingRange()).toEqual({ from: T0, to: T0 + 125 * SEC });
      expect(store.hasRange()).toBe(false);
      vi.advanceTimersByTime(200);
      expect(store.query().to).toBe(T0 + 125 * SEC);
      expect(zoom.pendingRange()).toBeNull();
      zoom.panBy(10 * SEC);
      vi.advanceTimersByTime(200);
      expect(store.query().from).toBe(T0 + 10 * SEC);
      zoom.panBy(-5 * SEC, false);
      expect(store.query().from).toBe(T0 + 5 * SEC);
    } finally {
      vi.useRealTimers();
    }
  });
});
