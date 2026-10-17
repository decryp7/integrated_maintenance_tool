import { TestBed } from '@angular/core/testing';
import { all, el, provideTinyBackend, settle, text, tinyBackend } from '../../testing/helpers';
import { Lane } from '../core/api.types';
import { WorkspaceStore } from '../core/workspace.store';
import { MarkerVm, mergeMarkers } from './marker-layout';
import { stableLaneOrder, TimelinePanel } from './timeline-panel';
import { TimelineZoom } from './zoom';

const mk = (kind: string, left: number, count = 1, id = left): MarkerVm => ({
  cluster: { kind, bucket: Math.round(left), ts: left, id, count, message: '' },
  left,
  color: null,
  label: kind,
});

const lane = (key: string, first: number): Lane => ({ key, first, last: first + 1, total: 1, matched: 1, density: [], markers: [] });

describe('timeline geometry', () => {
  it('merges same-kind markers that are closer than the gap', () => {
    const out = mergeMarkers([mk('warning', 10, 2, 1), mk('warning', 10.5, 3, 2), mk('error', 10.2), mk('warning', 20)], 1);
    const warnings = out.filter((m) => m.cluster.kind === 'warning');
    expect(warnings.map((m) => [m.left, m.cluster.count, m.cluster.id])).toEqual([
      [10, 5, 1],
      [20, 1, 20],
    ]);
    expect(out.filter((m) => m.cluster.kind === 'error')).toHaveLength(1);
    expect(mergeMarkers([], 1)).toEqual([]);
  });

  it('keeps lane order stable while zoomed', () => {
    const order = new Map<string, number>();
    const full = [lane('a', 1), lane('b', 2), lane('c', 3)];
    expect(stableLaneOrder(full, order, true).map((l) => l.key)).toEqual(['a', 'b', 'c']);
    const zoomed = [lane('c', 0), lane('new', -1), lane('a', 5)];
    expect(stableLaneOrder(zoomed, order, false).map((l) => l.key)).toEqual(['a', 'c', 'new']);
  });
});

describe('TimelinePanel', () => {
  async function render() {
    TestBed.configureTestingModule({ providers: [provideTinyBackend()] });
    const fixture = TestBed.createComponent(TimelinePanel);
    const store = TestBed.inject(WorkspaceStore);
    const zoom = TestBed.inject(TimelineZoom);
    await settle(fixture);
    return { fixture, store, zoom };
  }

  const wrap = (f: Awaited<ReturnType<typeof render>>['fixture']) => el(f, '.wrap');
  const pointer = (type: string, clientX: number, extra: PointerEventInit = {}) => new PointerEvent(type, { bubbles: true, clientX, button: 0, pointerId: 1, ...extra });

  it('renders lanes, histogram, ticks and the legend', async () => {
    const { fixture } = await render();
    const labels = all(fixture, '.lane-label .key').map((e) => e.textContent?.trim());
    expect(labels).toEqual(['WEB01 · 100', 'WEB02 · 300', 'WEB01 · 200']);
    expect(all(fixture, '.hist path').length).toBeGreaterThan(0);
    expect(all(fixture, '.tick').length).toBeGreaterThan(0);
    expect(text(fixture)).toContain('Fatal');
    expect(text(fixture)).toContain('3 instances');
    expect(all(fixture, '.fatal-line')).toHaveLength(1);
  });

  it('jumps to an event when a marker is clicked and shows a tooltip on hover', async () => {
    const { fixture, store } = await render();
    const marker = all<HTMLButtonElement>(fixture, '.marker').find((m) => m.getAttribute('aria-label')?.startsWith('Fatal'))!;
    marker.dispatchEvent(new PointerEvent('pointerenter', { clientX: 10, clientY: 10 }));
    await settle(fixture);
    expect(el(fixture, '.tip').textContent).toContain('Host terminated unexpectedly');
    marker.dispatchEvent(new PointerEvent('pointerleave'));
    marker.click();
    await settle(fixture);
    expect(store.anchor()?.id).toBeDefined();
    expect(fixture.nativeElement.querySelector('.tip')).toBeNull();
  });

  it('filters to an instance from its lane label', async () => {
    const { fixture, store } = await render();
    all<HTMLButtonElement>(fixture, '.lane-label')[1].click();
    await settle(fixture);
    expect(store.query().groups).toEqual(['WEB02 · 300']);
    expect(all(fixture, '.lane-label.filtered')).toHaveLength(1);
  });

  it('brush-zooms with a drag and ignores tiny drags', async () => {
    const { fixture, store } = await render();
    const w = wrap(fixture);
    w.dispatchEvent(pointer('pointerdown', 260));
    w.dispatchEvent(pointer('pointermove', 262));
    w.dispatchEvent(pointer('pointerup', 262));
    expect(store.hasRange()).toBe(false);
    w.dispatchEvent(pointer('pointerdown', 250 + 100));
    w.dispatchEvent(pointer('pointermove', 250 + 300));
    await settle(fixture);
    expect(fixture.nativeElement.querySelector('.brush')).not.toBeNull();
    w.dispatchEvent(pointer('pointerup', 250 + 300));
    await settle(fixture);
    const t = store.timeline()!;
    expect(store.hasRange()).toBe(true);
    expect(store.query().from).toBeGreaterThanOrEqual(t.from);
    w.dispatchEvent(pointer('pointerdown', 10));
    w.dispatchEvent(pointer('pointerdown', 260, { button: 2 }));
    expect(fixture.nativeElement.querySelector('.brush')).toBeNull();
  });

  it('shows a cursor time while hovering the plot', async () => {
    const { fixture } = await render();
    wrap(fixture).dispatchEvent(pointer('pointermove', 450));
    await settle(fixture);
    expect(el(fixture, '.cursor-label').textContent).toMatch(/14 Oct/);
    wrap(fixture).dispatchEvent(pointer('pointermove', 5));
    await settle(fixture);
    expect(fixture.nativeElement.querySelector('.cursor')).toBeNull();
    wrap(fixture).dispatchEvent(pointer('pointerleave', 5));
  });

  it('zooms with ctrl+wheel, pans with shift+wheel, fits on double-click', async () => {
    const { fixture, store, zoom } = await render();
    const w = wrap(fixture);
    const plain = new WheelEvent('wheel', { deltaY: 10, cancelable: true, bubbles: true });
    w.dispatchEvent(plain);
    expect(plain.defaultPrevented).toBe(false);
    const ctrl = new WheelEvent('wheel', { deltaY: -200, ctrlKey: true, clientX: 650, cancelable: true, bubbles: true });
    w.dispatchEvent(ctrl);
    expect(ctrl.defaultPrevented).toBe(true);
    expect(zoom.pendingRange()).not.toBeNull();
    await new Promise((r) => setTimeout(r, 200));
    await settle(fixture);
    expect(store.hasRange()).toBe(true);
    const from = store.query().from!;
    const shift = new WheelEvent('wheel', { deltaY: 100, shiftKey: true, cancelable: true, bubbles: true });
    w.dispatchEvent(shift);
    w.dispatchEvent(new WheelEvent('wheel', { deltaX: 50, deltaY: 1, cancelable: true, bubbles: true }));
    await new Promise((r) => setTimeout(r, 200));
    await settle(fixture);
    expect(shift.defaultPrevented).toBe(true);
    expect(store.query().from).toBeGreaterThan(from);
    w.dispatchEvent(new MouseEvent('dblclick', { bubbles: true }));
    await settle(fixture);
    expect(store.hasRange()).toBe(false);
  });

  it('pans with the middle button and with space+drag', async () => {
    const { fixture, store, zoom } = await render();
    zoom.zoomIn();
    await settle(fixture);
    const from = store.query().from!;
    const w = wrap(fixture);
    w.dispatchEvent(pointer('pointerdown', 600, { button: 1 }));
    w.dispatchEvent(pointer('pointerup', 500));
    await settle(fixture);
    expect(store.query().from).toBeGreaterThan(from);

    const after = store.query().from!;
    document.dispatchEvent(new KeyboardEvent('keydown', { code: 'Space', key: ' ' }));
    w.dispatchEvent(pointer('pointerdown', 500));
    await settle(fixture);
    expect(el(fixture, '.wrap').classList.contains('panning')).toBe(true);
    w.dispatchEvent(pointer('pointerup', 600));
    document.dispatchEvent(new KeyboardEvent('keyup', { code: 'Space', key: ' ' }));
    await settle(fixture);
    expect(store.query().from).toBeLessThan(after);
    w.dispatchEvent(pointer('pointerdown', 500));
    w.dispatchEvent(pointer('pointerup', 501));
  });

  it('navigates history with buttons and Alt+arrows', async () => {
    const { fixture, store, zoom } = await render();
    const [back, fwd, zin, zout] = all<HTMLButtonElement>(fixture, '.zoom .icon-btn');
    expect(back.disabled).toBe(true);
    zin.click();
    await settle(fixture);
    expect(store.hasRange()).toBe(true);
    document.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowLeft', altKey: true }));
    await settle(fixture);
    expect(store.hasRange()).toBe(false);
    expect(fwd.disabled).toBe(false);
    document.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowRight', altKey: true }));
    await settle(fixture);
    expect(store.hasRange()).toBe(true);
    zout.click();
    await settle(fixture);
    expect(store.hasRange()).toBe(false);
    zoom.zoomIn();
    await settle(fixture);
    el<HTMLButtonElement>(fixture, '.zoom .btn').click();
    expect(store.hasRange()).toBe(false);
    const typing = document.createElement('input');
    document.body.appendChild(typing);
    zoom.zoomIn();
    typing.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowLeft', altKey: true, bubbles: true }));
    expect(store.hasRange()).toBe(true);
    typing.remove();
  });

  it('collapses warnings of a busy lane into a tick strip and keeps counts in the tooltip', async () => {
    const backend = tinyBackend();
    const real = backend.queryTimeline.bind(backend);
    vi.spyOn(backend, 'queryTimeline').mockImplementation(async (req) => {
      const t = await real(req);
      const lane = t.lanes[0];
      const span = t.to - t.from;
      lane.markers = Array.from({ length: 60 }, (_, i) => ({ kind: 'warning', bucket: i, ts: t.from + (span * i) / 60, id: 2, count: 1 + (i % 3), message: 'w' }));
      lane.markers.push({ kind: 'fatal', bucket: 59, ts: t.to, id: 5, count: 1, message: 'Host terminated' });
      return t;
    });
    TestBed.configureTestingModule({ providers: [provideTinyBackend(backend)] });
    const fixture = TestBed.createComponent(TimelinePanel);
    await settle(fixture);
    const firstLane = all(fixture, '.lane')[0];
    const strip = firstLane.querySelector('svg.ticks')!;
    expect(strip.getAttribute('aria-label')).toBe('120 warnings (zoom in for detail)');
    expect(firstLane.querySelectorAll('.marker')).toHaveLength(1);
    const fatal = firstLane.querySelector<HTMLButtonElement>('.marker')!;
    fatal.dispatchEvent(new PointerEvent('pointerenter', { clientX: 1, clientY: 1 }));
    await settle(fixture);
    expect(el(fixture, '.tip').textContent).toContain('1 event');
  });

  it('measures the plot, asks for matching buckets and reports truncated lanes', async () => {
    const backend = tinyBackend();
    const real = backend.queryTimeline.bind(backend);
    vi.spyOn(backend, 'queryTimeline').mockImplementation(async (req) => ({ ...(await real(req)), truncatedLanes: 4 }));
    const rect = vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockImplementation(function (this: HTMLElement) {
      const axis = this.classList.contains('axis');
      return { left: axis ? 300 : 50, width: axis ? 1000 : 1300, top: 0, height: 10, right: 0, bottom: 0, x: 0, y: 0, toJSON: () => ({}) } as DOMRect;
    });
    TestBed.configureTestingModule({ providers: [provideTinyBackend(backend)] });
    const fixture = TestBed.createComponent(TimelinePanel);
    const store = TestBed.inject(WorkspaceStore);
    await settle(fixture);
    rect.mockRestore();
    expect(store.buckets()).toBe(500);
    expect(text(fixture)).toContain('4 more instances not shown');
  });

  it('shows empty states', async () => {
    const { fixture, store } = await render();
    store.addFilter({ field: 'MachineName', op: 'eq', value: 'none' });
    await settle(fixture);
    expect(text(fixture)).toContain('No timestamped events match');
    await store.clearWorkspace();
    await settle(fixture);
    expect(text(fixture)).toContain('Open log files or a folder');
  });
});
