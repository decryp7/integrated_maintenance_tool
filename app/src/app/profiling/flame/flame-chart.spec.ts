import { ComponentFixture, TestBed } from '@angular/core/testing';
import { afterEach, beforeEach, vi } from 'vitest';
import { all, el, settle, text } from '../../../testing/helpers';
import { FlameGraph, FlameRect, FrameDto, MetricDto } from '../../core/api.types';
import { FlameChart } from './flame-chart';

const frame = (id: number, name: string, module: string | null, kind: FrameDto['kind']): FrameDto => ({ id, name, module, file: null, line: null, kind, isFramework: false });

const FRAMES: FrameDto[] = [
  frame(1, 'App.Main', 'App', 'managed'),
  frame(2, 'App.Work', 'App', 'managed'),
  frame(3, 'lib!Native', 'lib', 'native'),
  frame(4, 'App.Leaf', 'App', 'managed'),
  frame(5, 'kernel!Swap', null, 'kernel'),
];

/** A rectangle; below depth 0 its parent is the first rectangle (index 0) unless the patch says otherwise. */
const rect = (frameId: number, depth: number, x: number, w: number, value: number, patch: Partial<FlameRect> = {}): FlameRect => ({ frame: frameId, parent: depth === 0 ? null : 0, depth, x, w, value, selfValue: value / 2, matched: false, ...patch });

/** root(100) → [work(60) → leaf(30), native(40)]. */
function graph(patch: Partial<FlameGraph> = {}, rects?: FlameRect[]): FlameGraph {
  return {
    rects: rects ?? [rect(1, 0, 0, 1, 100), rect(2, 1, 0, 0.6, 60), rect(3, 1, 0.6, 0.4, 40), rect(4, 2, 0, 0.3, 30, { parent: 1 })],
    frames: FRAMES,
    focusFrames: [],
    focusValue: 100,
    grandTotal: 100,
    matchedValue: 0,
    matchedPct: 0,
    truncated: false,
    maxDepth: 2,
    metric: 'samples',
    measurement: 'sampled',
    ...patch,
  };
}

const METRICS: MetricDto[] = [{ id: 'samples', measurement: 'sampled', unit: 'count', label: 'Samples' }];

function diffGraph(): FlameGraph {
  return graph({}, [
    rect(1, 0, 0, 1, 100, { delta: 0, baselineValue: 100 }),
    rect(2, 1, 0, 0.6, 60, { delta: 0.1, baselineValue: 50 }),
    rect(3, 1, 0.6, 0.4, 40, { delta: -0.1, baselineValue: 50 }),
  ]);
}

interface Inputs {
  graph?: FlameGraph | null;
  orientation?: 'flame' | 'icicle';
  colorBy?: 'kind' | 'module';
  diff?: boolean;
}

function mount(inputs: Inputs = {}): ComponentFixture<FlameChart> {
  const fixture = TestBed.createComponent(FlameChart);
  fixture.componentRef.setInput('graph', inputs.graph === undefined ? graph() : inputs.graph);
  fixture.componentRef.setInput('metrics', METRICS);
  if (inputs.orientation) fixture.componentRef.setInput('orientation', inputs.orientation);
  if (inputs.colorBy) fixture.componentRef.setInput('colorBy', inputs.colorBy);
  if (inputs.diff) fixture.componentRef.setInput('diff', true);
  fixture.detectChanges();
  return fixture;
}

function stubBox(fixture: ComponentFixture<unknown>, width = 1000): HTMLElement {
  const surface = el(fixture, '.surface');
  surface.getBoundingClientRect = () => ({ left: 0, top: 0, width, height: 54, right: width, bottom: 54, x: 0, y: 0, toJSON: () => ({}) }) as DOMRect;
  return surface;
}

function key(fixture: ComponentFixture<unknown>, k: string): KeyboardEvent {
  const e = new KeyboardEvent('keydown', { key: k, bubbles: true, cancelable: true });
  el(fixture, '.surface').dispatchEvent(e);
  return e;
}

const announce = (f: ComponentFixture<unknown>) => el(f, '[data-testid=flame-announce]').textContent ?? '';

function pointer(target: HTMLElement, type: string, x: number, y: number): void {
  target.dispatchEvent(new MouseEvent(type, { clientX: x, clientY: y, bubbles: true }));
}

describe('FlameChart', () => {
  beforeEach(() => vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockReturnValue(null));
  afterEach(() => vi.restoreAllMocks());

  it('lists the frame-kind legend and the measurement tag', () => {
    const f = mount();
    const legend = el(f, '.legend').textContent!;
    for (const k of ['Managed', 'Native', 'Kernel', 'Unknown']) expect(legend).toContain(k);
    expect(el(f, 'imt-measurement .m').getAttribute('data-measurement')).toBe('sampled');
    expect(text(f)).toContain('(Sampled, estimate)');
  });

  it('lists the modules by value plus "Other modules" when coloured by module', () => {
    const f = mount({ colorBy: 'module' });
    const labels = all(f, '.legend .lg').map((x) => x.textContent!.trim());
    expect(labels).toEqual(['App', 'lib', 'Other modules']);
  });

  it('shows a diverging legend with the scale in a differential chart', () => {
    const f = mount({ graph: diffGraph(), diff: true });
    const legend = text(f);
    expect(legend).toContain('less than the baseline (−10.0 pp)');
    expect(legend).toContain('more than the baseline (+10.0 pp)');
    expect(legend).toContain('no change');
    expect(el(f, '.ramp').getAttribute('style')).toContain('linear-gradient');
    expect(text(f)).toContain('Change');
    expect(all(f, '.frames tbody tr')[1].textContent).toContain('+10.0 pp');
  });

  it('describes the chart, and mentions truncation', () => {
    const f = mount();
    expect(el(f, '.surface').getAttribute('aria-label')).toBe('Flame graph. 4 frames, 3 levels');
    f.componentRef.setInput('graph', graph({ truncated: true }));
    f.detectChanges();
    expect(el(f, '.surface').getAttribute('aria-label')).toContain('truncated: more frames than the limit, zoom in for detail');
    f.componentRef.setInput('graph', null);
    f.detectChanges();
    expect(el(f, '.surface').getAttribute('aria-label')).toContain('No data.');
    expect(all(f, '.frames tbody tr')).toHaveLength(0);
  });

  it('gives each instance its own help id and explains the keys per orientation', () => {
    const a = mount();
    const b = mount({ orientation: 'icicle' });
    const idA = el(a, '.surface').getAttribute('aria-describedby')!;
    const idB = el(b, '.surface').getAttribute('aria-describedby')!;
    expect(idA).not.toBe(idB);
    expect(el(a, `#${idA}`).textContent).toContain('Up to a callee, Down to the caller');
    expect(el(b, `#${idB}`).textContent).toContain('Down to a callee, Up to the caller');
  });

  it('navigates with Home, arrows and announces the frame', () => {
    const f = mount();
    const selected: FrameDto[] = [];
    f.componentInstance.selected.subscribe((x) => selected.push(x));
    expect(announce(f)).toBe('');
    expect(key(f, 'Home').defaultPrevented).toBe(true);
    f.detectChanges();
    expect(announce(f)).toContain('App.Main, depth 0, total 100, 100.0%');
    key(f, 'ArrowUp'); // flame: up = callee (largest child)
    f.detectChanges();
    expect(announce(f)).toContain('App.Work, depth 1');
    key(f, 'ArrowRight');
    f.detectChanges();
    expect(announce(f)).toContain('lib!Native');
    key(f, 'ArrowRight'); // no more neighbours
    f.detectChanges();
    expect(announce(f)).toContain('lib!Native');
    key(f, 'ArrowLeft');
    key(f, 'ArrowUp');
    f.detectChanges();
    expect(announce(f)).toContain('App.Leaf, depth 2');
    key(f, 'ArrowUp'); // no deeper
    key(f, 'ArrowDown'); // back to the caller
    f.detectChanges();
    expect(announce(f)).toContain('App.Work');
    expect(selected.map((s) => s.name)).toEqual(['App.Main', 'App.Work', 'lib!Native', 'App.Work', 'App.Leaf', 'App.Work']);
  });

  it('swaps the vertical arrows in the icicle orientation', () => {
    const f = mount({ orientation: 'icicle' });
    key(f, 'Home');
    key(f, 'ArrowUp'); // up is the caller: none at the root
    f.detectChanges();
    expect(announce(f)).toContain('App.Main');
    key(f, 'ArrowDown');
    f.detectChanges();
    expect(announce(f)).toContain('App.Work');
    key(f, 'ArrowUp');
    f.detectChanges();
    expect(announce(f)).toContain('App.Main');
  });

  it('enters a zoom with the chain of frame ids, and escapes to zoom out', () => {
    const f = mount();
    const zooms: number[][] = [];
    let outs = 0;
    f.componentInstance.zoom.subscribe((z) => zooms.push(z));
    f.componentInstance.zoomOut.subscribe(() => outs++);
    key(f, 'Home');
    key(f, 'ArrowUp');
    key(f, 'ArrowUp');
    key(f, 'Enter');
    expect(zooms).toEqual([[1, 2, 4]]);
    key(f, 'Escape');
    expect(outs).toBe(1);
  });

  it('ignores unknown keys and keys with no frame to act on', () => {
    const f = mount();
    expect(key(f, 'x').defaultPrevented).toBe(false);
    expect(key(f, 'Enter').defaultPrevented).toBe(false);
    f.componentRef.setInput('graph', graph({}, []));
    f.detectChanges();
    expect(key(f, 'Home').defaultPrevented).toBe(false);
    expect(key(f, 'Escape').defaultPrevented).toBe(true);
  });

  it('shows a tooltip under the pointer and hides it on leave', () => {
    const f = mount();
    const surface = stubBox(f);
    pointer(surface, 'pointermove', 100, 20); // depth 1, x 0.1 → App.Work
    f.detectChanges();
    const tip = el(f, '[role=tooltip]');
    expect(tip.textContent).toContain('App.Work');
    expect(tip.textContent).toContain('App · Managed');
    expect(tip.textContent).toContain('Total 60 (60.0% of all)');
    expect(tip.textContent).toContain('Self 30');
    expect(tip.textContent).toContain('Sampled values · click to zoom');
    pointer(surface, 'pointermove', 100, 0); // depth 2 but x 0.1 → leaf
    f.detectChanges();
    expect(el(f, '[role=tooltip]').textContent).toContain('App.Leaf');
    pointer(surface, 'pointermove', 900, 0); // nothing at depth 2 there
    f.detectChanges();
    expect(f.nativeElement.querySelector('[role=tooltip]')).toBeNull();
    pointer(surface, 'pointermove', 100, 20);
    f.detectChanges();
    pointer(surface, 'pointerleave', 0, 0);
    f.detectChanges();
    expect(f.nativeElement.querySelector('[role=tooltip]')).toBeNull();
  });

  it('shows baseline and change in a differential tooltip, and "the zoomed frame" when focused', () => {
    const f = mount({ graph: graph({ focusValue: 50 }, diffGraph().rects), diff: true });
    const surface = stubBox(f);
    pointer(surface, 'pointermove', 100, 2);
    f.detectChanges();
    const tip = el(f, '[role=tooltip]').textContent!;
    expect(tip).toContain('Baseline 50 · change +10.0 pp');
    expect(tip).toContain('of the zoomed frame');
  });

  it('names a frame the graph does not describe', () => {
    const f = mount({ graph: graph({}, [rect(99, 0, 0, 1, 10)]) });
    const surface = stubBox(f);
    pointer(surface, 'pointermove', 10, 0);
    f.detectChanges();
    const tip = el(f, '[role=tooltip]').textContent!;
    expect(tip).toContain('frame #99');
    key(f, 'Home');
    f.detectChanges();
    expect(announce(f)).toContain('frame #99');
  });

  it('hit-tests rows in the icicle orientation', () => {
    const f = mount({ orientation: 'icicle' });
    stubBox(f);
    expect(f.componentInstance.hitAt(100, 20)).toBe(1);
    expect(f.componentInstance.hitAt(100, 2)).toBe(0);
    expect(f.componentInstance.hitAt(100, 500)).toBeNull();
  });

  it('zooms on click and ignores clicks on empty space', () => {
    const f = mount();
    const surface = stubBox(f);
    const zooms: number[][] = [];
    const selected: FrameDto[] = [];
    f.componentInstance.zoom.subscribe((z) => zooms.push(z));
    f.componentInstance.selected.subscribe((s) => selected.push(s));
    pointer(surface, 'click', 700, 20);
    expect(zooms).toEqual([[1, 3]]);
    expect(selected[0].name).toBe('lib!Native');
    pointer(surface, 'click', 900, 0);
    expect(zooms).toHaveLength(1);
    f.componentInstance.zoomAt(99);
    expect(zooms).toHaveLength(1);
  });

  it('falls back to the measured width when the box has none', () => {
    const f = mount();
    expect(f.componentInstance.hitAt(100, 20)).toBe(1);
  });

  it('offers the frames in view as buttons that zoom, with match tags when searching', () => {
    const rects = [rect(1, 0, 0, 1, 100, { matched: true }), rect(2, 1, 0, 0.6, 60), rect(3, 1, 0.6, 0.4, 40, { matched: true })];
    const f = mount({ graph: graph({}, rects) });
    expect(text(f)).toContain('Frames in view (3)');
    const zooms: number[][] = [];
    f.componentInstance.zoom.subscribe((z) => zooms.push(z));
    const rows = all(f, '.frames tbody tr');
    expect(rows.map((r) => r.querySelector('button')!.textContent)).toEqual(['App.Main', 'App.Work', 'lib!Native']);
    expect(rows.map((r) => r.querySelector('.tag') !== null)).toEqual([true, false, true]);
    el<HTMLButtonElement>(f, 'button[aria-label="Zoom to lib!Native"]').click();
    expect(zooms).toEqual([[1, 3]]);
  });

  it('caps the accessible list and says so', () => {
    const rects = [rect(1, 0, 0, 1, 1000), ...Array.from({ length: 120 }, (_, i) => rect(2, 1, i / 120, 1 / 120, 10 + i))];
    const f = mount({ graph: graph({ maxDepth: 1 }, rects) });
    expect(text(f)).toContain('Frames in view (121, largest 100 listed)');
    expect(all(f, '.frames tbody tr')).toHaveLength(100);
  });

  it('highlights the keyboard cursor row in the list', () => {
    const f = mount();
    key(f, 'Home');
    f.detectChanges();
    expect(all(f, '.frames tbody tr.sel')).toHaveLength(1);
  });

  it('exposes the drawing width', () => {
    const f = mount();
    expect(f.componentInstance.widthPx()).toBe(800);
  });

  describe('drawing', () => {
    function stubContext() {
      const ctx = {
        clearRect: vi.fn(),
        fillRect: vi.fn(),
        strokeRect: vi.fn(),
        fillText: vi.fn(),
        measureText: vi.fn((t: string) => ({ width: t.length * 6 })),
        save: vi.fn(),
        restore: vi.fn(),
        beginPath: vi.fn(),
        rect: vi.fn(),
        clip: vi.fn(),
        setTransform: vi.fn(),
        fillStyle: '',
        strokeStyle: '',
        globalAlpha: 1,
        lineWidth: 1,
        font: '',
        textBaseline: 'alphabetic',
      };
      vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockReturnValue(ctx as unknown as CanvasRenderingContext2D);
      return ctx;
    }

    it('fills a rectangle per frame and draws labels that fit', async () => {
      const ctx = stubContext();
      const f = mount();
      await settle(f);
      expect(ctx.fillRect.mock.calls.length).toBeGreaterThanOrEqual(4);
      expect(ctx.fillRect.mock.calls.length % 4).toBe(0); // one fill per frame per pass
      expect(ctx.setTransform).toHaveBeenCalled();
      const labels = ctx.fillText.mock.calls.map((c) => c[0]);
      expect(labels).toContain('App.Main');
      expect(labels).toContain('App.Work');
      expect(ctx.strokeRect).not.toHaveBeenCalled();
    });

    it('outlines the cursor and the hovered frame, and the matches while searching', async () => {
      const ctx = stubContext();
      const rects = [rect(1, 0, 0, 1, 100), rect(2, 1, 0, 0.6, 60, { matched: true }), rect(3, 1, 0.6, 0.4, 40)];
      const f = mount({ graph: graph({}, rects) });
      await settle(f);
      expect(ctx.strokeRect).toHaveBeenCalled(); // the matched frame is outlined
      ctx.strokeRect.mockClear();
      key(f, 'Home');
      pointer(stubBox(f), 'pointermove', 700, 20);
      await settle(f);
      expect(ctx.strokeRect.mock.calls.length).toBeGreaterThanOrEqual(3); // match + cursor + hover
    });

    it('draws a differential chart', async () => {
      const ctx = stubContext();
      const f = mount({ graph: diffGraph(), diff: true });
      await settle(f);
      expect(ctx.fillRect.mock.calls.length).toBeGreaterThanOrEqual(3);
      expect(ctx.fillRect.mock.calls.length % 3).toBe(0);
    });

    it('survives a canvas without a 2d context', async () => {
      vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockReturnValue(null);
      const f = mount();
      await settle(f);
      expect(el(f, 'canvas').style.width).toBe('800px');
    });

    it('scrolls a moving cursor into view', async () => {
      stubContext();
      const f = mount();
      const scroller = el(f, '.scroll');
      Object.defineProperty(scroller, 'clientHeight', { value: 18, configurable: true });
      scroller.scrollTop = 500;
      key(f, 'Home');
      expect(scroller.scrollTop).toBeLessThan(500);
    });
  });
});
