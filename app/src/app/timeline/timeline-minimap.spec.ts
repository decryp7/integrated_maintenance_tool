import { computed, signal } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { all, provideFakeDock, provideTinyBackend, settle } from '../../testing/helpers';
import { HomeBackend } from '../home/home-backend';
import { HomeSummary } from '../core/api.types';
import { AnalysisBackend } from '../core/analysis-backend';
import { DebriefBackend } from '../core/backend';
import { MockAnalysisBackend, recordsFrom } from '../mock-backends';
import { inject } from '@angular/core';
import { TimelineMinimap } from './timeline-minimap';
import { MINIMAP_BUCKETS, TimelineMinimapStore } from './timeline-minimap-store';
import { TIMELINE_SOURCE } from './timeline-source';
import { Range, RangePort, RangeZoom } from './zoom';

const EXTENT: Range = { from: 0, to: 10_000 };

function fakeSource() {
  const range = signal<Range | null>(null);
  const applied: (Range | null)[] = [];
  const port: RangePort = {
    extent: signal<Range | null>(EXTENT),
    view: computed(() => range() ?? EXTENT),
    hasRange: computed(() => range() !== null),
    canPan: computed(() => range() !== null),
    setRange: (r) => {
      applied.push(r);
      range.set(r);
    },
  };
  return { zoom: new RangeZoom(port), applied, range };
}

describe('TimelineMinimap', () => {
  const queries: unknown[] = [];

  function setup() {
    const src = fakeSource();
    const hist = Array.from({ length: 10 }, (_, i) => [0, 0, 5 + i, 0, i === 3 ? 4 : 0, 0]);
    const querySummary = vi.fn(async (request: unknown) => {
      queries.push(request);
      return { histogram: hist } as unknown as HomeSummary;
    });
    TestBed.configureTestingModule({
      providers: [
        provideTinyBackend(),
        provideFakeDock(),
        { provide: AnalysisBackend, useFactory: () => new MockAnalysisBackend(recordsFrom(inject(DebriefBackend))) },
        { provide: HomeBackend, useValue: { querySummary } },
        { provide: TIMELINE_SOURCE, useValue: { zoom: () => src.zoom } },
      ],
    });
    const fixture = TestBed.createComponent(TimelineMinimap);
    // The strip is 1000 px wide in the test: 10 ms per px over the 10 s extent.
    vi.spyOn(fixture.nativeElement as HTMLElement, 'getBoundingClientRect').mockReturnValue({ left: 0, width: 1000, top: 0, right: 1000, bottom: 26, height: 26, x: 0, y: 0, toJSON: () => ({}) });
    return { fixture, src, querySummary, el: fixture.nativeElement as HTMLElement };
  }

  const pointer = (type: string, x: number, init: PointerEventInit = {}) => new PointerEvent(type, { clientX: x, pointerId: 1, button: 0, bubbles: true, ...init });

  beforeEach(() => (queries.length = 0));

  it('draws the whole extent by level and a window for the visible range', async () => {
    const { fixture, src, querySummary, el } = setup();
    await settle(fixture, 6);
    expect(querySummary).toHaveBeenCalledWith(expect.objectContaining({ buckets: MINIMAP_BUCKETS, analyses: false }));
    expect(all(fixture, 'svg.mini path').length).toBeGreaterThan(0);
    const win = el.querySelector<HTMLElement>('[data-testid=minimap-window]')!;
    expect(win.style.left).toBe('0%');
    expect(win.style.width).toBe('100%');
    src.zoom.apply({ from: 2000, to: 4000 });
    await settle(fixture, 4);
    expect(win.style.left).toBe('20%');
    expect(win.style.width).toBe('20%');
    expect(win.getAttribute('aria-valuetext')).toContain('to');
    expect(el.querySelector('[role=group]')?.getAttribute('aria-label')).toBe('Timeline overview');
  });

  it('dragging the window pans on release, and an edge zooms', async () => {
    const { fixture, src, el } = setup();
    src.zoom.apply({ from: 2000, to: 4000 });
    await settle(fixture, 4);
    const win = el.querySelector<HTMLElement>('[data-testid=minimap-window]')!;
    src.applied.length = 0;
    win.dispatchEvent(pointer('pointerdown', 300));
    win.dispatchEvent(pointer('pointermove', 400));
    // Shown at once, applied only on release.
    await settle(fixture, 2);
    expect(win.style.left).toBe('30%');
    expect(src.applied).toEqual([]);
    win.dispatchEvent(pointer('pointerup', 400));
    expect(src.applied).toEqual([{ from: 3000, to: 5000 }]);
    const edge = el.querySelector<HTMLElement>('[data-testid=minimap-edge-right]')!;
    edge.dispatchEvent(pointer('pointerdown', 500));
    edge.dispatchEvent(pointer('pointermove', 600));
    edge.dispatchEvent(pointer('pointerup', 600));
    expect(src.applied.at(-1)).toEqual({ from: 3000, to: 6000 });
    const left = el.querySelector<HTMLElement>('[data-testid=minimap-edge-left]')!;
    left.dispatchEvent(pointer('pointerdown', 300));
    left.dispatchEvent(pointer('pointermove', 200));
    left.dispatchEvent(pointer('pointercancel', 200));
    expect(src.range()).toEqual({ from: 3000, to: 6000 });
  });

  it('a press on the strip centres the window, a double-click fits, and a click without movement does nothing', async () => {
    const { fixture, src, el } = setup();
    src.zoom.apply({ from: 2000, to: 4000 });
    await settle(fixture, 4);
    src.applied.length = 0;
    const strip = el.querySelector<HTMLElement>('[data-testid=timeline-minimap]')!;
    strip.dispatchEvent(pointer('pointerdown', 700));
    expect(src.applied).toEqual([{ from: 6000, to: 8000 }]);
    const win = el.querySelector<HTMLElement>('[data-testid=minimap-window]')!;
    src.applied.length = 0;
    win.dispatchEvent(pointer('pointerdown', 700));
    win.dispatchEvent(pointer('pointerup', 700));
    expect(src.applied).toEqual([]);
    strip.dispatchEvent(new MouseEvent('dblclick', { bubbles: true }));
    expect(src.applied).toEqual([null]);
    // Other buttons do nothing.
    strip.dispatchEvent(pointer('pointerdown', 100, { button: 2 }));
    expect(src.applied).toEqual([null]);
  });

  it('is a slider: arrow keys pan, Shift pans further, Home and End go to the ends', async () => {
    const { fixture, src, el } = setup();
    src.zoom.apply({ from: 2000, to: 4000 });
    await settle(fixture, 4);
    const win = el.querySelector<HTMLElement>('[data-testid=minimap-window]')!;
    const press = (key: string, init: KeyboardEventInit = {}) => win.dispatchEvent(new KeyboardEvent('keydown', { key, bubbles: true, cancelable: true, ...init }));
    press('ArrowRight');
    expect(src.range()).toEqual({ from: 2200, to: 4200 });
    press('ArrowLeft', { shiftKey: true });
    expect(src.range()).toEqual({ from: 1200, to: 3200 });
    press('End');
    expect(src.range()).toEqual({ from: 8000, to: 10_000 });
    press('Home');
    expect(src.range()).toEqual({ from: 0, to: 2000 });
    const before = src.range();
    press('x');
    expect(src.range()).toBe(before);
  });
});

describe('TimelineMinimapStore', () => {
  it('asks only while a timeline is visible, and resets without sources', async () => {
    const querySummary = vi.fn(async () => ({ histogram: [[1, 0, 0, 0, 0, 0]] }) as unknown as HomeSummary);
    TestBed.configureTestingModule({
      providers: [
        provideTinyBackend(),
        provideFakeDock(),
        { provide: AnalysisBackend, useFactory: () => new MockAnalysisBackend(recordsFrom(inject(DebriefBackend))) },
        { provide: HomeBackend, useValue: { querySummary } },
      ],
    });
    const store = TestBed.inject(TimelineMinimapStore);
    await settle(undefined, 4);
    expect(querySummary).not.toHaveBeenCalled();
    expect(store.histogram()).toEqual([]);
    const visible = signal(true);
    store.gate.add(visible, { onDestroy: () => () => undefined });
    await settle(undefined, 4);
    expect(querySummary).toHaveBeenCalledTimes(1);
    expect(store.histogram()).toHaveLength(1);
  });
});
