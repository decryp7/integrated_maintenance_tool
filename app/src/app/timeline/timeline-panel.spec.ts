import { TestBed } from '@angular/core/testing';
import { all, buttonByText, el, provideTinyBackend, settle, text, tinyBackend, tinyDataset, manualFrames } from '../../testing/helpers';
import { MockBackend } from '../core/mock/mock-backend';
import { Lane } from '../core/api.types';
import { WorkspaceStore } from '../core/workspace.store';
import { MarkerVm, mergeMarkers } from './marker-layout';
import { stableLaneOrder, TimelinePanel } from './timeline-panel';
import { TimelineZoom } from './zoom';
import { AnalysisTargets } from '../analysis/analysis-targets';
import { TimeCursor } from '../analysis/time-cursor';
import { momentAt, WIDTHS } from '../analysis/windows';
import { COMMAND_CONTEXT } from '../extensions/command-registry';
import { ContextMenuService } from '../ui/context-menu.service';
import { at } from '../../testing/must';

const mk = (kind: string, left: number, count = 1, id = left): MarkerVm => ({
  cluster: { kind, bucket: Math.round(left), ts: left, id, count, message: '' },
  left,
  color: null,
  label: kind,
});

const lane = (key: string, first: number): Lane => ({ key, first, last: first + 1, total: 1, matched: 1, density: [], markers: [], sources: [] });

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
    at(all<HTMLButtonElement>(fixture, '.lane-label'), 1).click();
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
    const before = store.timeline()!;
    w.dispatchEvent(pointer('pointerup', 250 + 300));
    await settle(fixture);
    const t = store.timeline()!;
    expect(store.hasRange()).toBe(true);
    expect(store.query().from).toBeGreaterThanOrEqual(t.from);
    // The brush is also the shared selection ("Analyse this moment", compare).
    const span = before.to - before.from;
    expect(TestBed.inject(TimeCursor).selection()).toEqual({ from: before.from + (100 / 800) * span, to: before.from + (300 / 800) * span });
    w.dispatchEvent(pointer('pointerdown', 10));
    w.dispatchEvent(pointer('pointerdown', 260, { button: 2 }));
    expect(fixture.nativeElement.querySelector('.brush')).toBeNull();
  });

  it('shows a cursor time while hovering the plot', async () => {
    const { fixture } = await render();
    const frames = manualFrames();
    wrap(fixture).dispatchEvent(pointer('pointermove', 450));
    frames.flush();
    await settle(fixture);
    expect(el(fixture, '.cursor-label').textContent).toMatch(/14 Oct/);
    wrap(fixture).dispatchEvent(pointer('pointermove', 5));
    frames.flush();
    await settle(fixture);
    expect(fixture.nativeElement.querySelector('.cursor:not([hidden])')).toBeNull();
    wrap(fixture).dispatchEvent(pointer('pointerleave', 5));
    frames.restore();
  });

  it('publishes the hovered time and draws the line for a time hovered in another view', async () => {
    const { fixture, store } = await render();
    const cursor = TestBed.inject(TimeCursor);
    const t = store.timeline()!;
    const frames = manualFrames();
    const hover = vi.spyOn(cursor, 'hover');
    // Many moves within one frame publish once (the last one).
    for (const x of [250 + 100, 250 + 200, 250 + 400]) wrap(fixture).dispatchEvent(pointer('pointermove', x));
    expect(hover).not.toHaveBeenCalled();
    frames.flush();
    expect(hover).toHaveBeenCalledTimes(1);
    expect(cursor.hovered()).toBe(t.from + (t.to - t.from) / 2);
    // A move still waiting when the pointer leaves is dropped.
    wrap(fixture).dispatchEvent(pointer('pointermove', 250 + 100));
    wrap(fixture).dispatchEvent(pointer('pointerleave', 0));
    frames.flush();
    frames.restore();
    expect(cursor.hovered()).toBeNull();
    await settle(fixture);
    const shown = () => fixture.nativeElement.querySelector('.cursor:not([hidden])');
    expect(shown()).toBeNull();

    cursor.hover(t.from + (t.to - t.from) / 4);
    await settle(fixture);
    expect(el<HTMLElement>(fixture, '.cursor').style.left).toBe('200px');
    cursor.hover(t.from);
    await settle(fixture);
    expect(el<HTMLElement>(fixture, '.cursor').style.left).toBe('0px');
    cursor.hover(t.to + 1);
    await settle(fixture);
    expect(shown()).toBeNull();
  });

  it('offers "Analyse this moment" on a right-click of the time axis', async () => {
    const showView = vi.fn();
    TestBed.configureTestingModule({ providers: [provideTinyBackend(), { provide: COMMAND_CONTEXT, useValue: { showView } }] });
    const fixture = TestBed.createComponent(TimelinePanel);
    const store = TestBed.inject(WorkspaceStore);
    await settle(fixture);
    const t = store.timeline()!;
    const menu = TestBed.inject(ContextMenuService);
    const targets = TestBed.inject(AnalysisTargets);
    const event = new MouseEvent('contextmenu', { bubbles: true, cancelable: true, clientX: 250 + 400, clientY: 40 });
    el(fixture, '.axis').dispatchEvent(event);
    expect(event.defaultPrevented).toBe(true);
    const items = menu.menu()!.items;
    expect(items.map((i) => i.label)).toEqual(['Analyse this moment']);
    at(items, 0).run();
    const mid = t.from + (t.to - t.from) / 2;
    expect(targets.moment()).toEqual(momentAt(mid, null, at(WIDTHS, 1).ms));
    expect(showView).toHaveBeenCalledWith('moment');

    // Inside a selected range, the moment is that range.
    TestBed.inject(TimeCursor).select({ from: t.from, to: t.to });
    el(fixture, '.axis').dispatchEvent(new MouseEvent('contextmenu', { bubbles: true, cancelable: true, clientX: 250 + 400 }));
    at(menu.menu()!.items, 0).run();
    expect(targets.moment()).toEqual({ from: t.from, to: t.to });
  });

  it('ignores an axis right-click before the timeline has loaded', () => {
    TestBed.configureTestingModule({ providers: [provideTinyBackend()] });
    const fixture = TestBed.createComponent(TimelinePanel);
    fixture.detectChanges();
    const event = new MouseEvent('contextmenu', { bubbles: true, cancelable: true });
    el(fixture, '.axis').dispatchEvent(event);
    expect(event.defaultPrevented).toBe(false);
    expect(TestBed.inject(ContextMenuService).menu()).toBeNull();
  });

  it('shows a failed timeline request inline with Retry instead of the global banner', async () => {
    const backend = tinyBackend();
    vi.spyOn(backend, 'queryTimeline').mockRejectedValueOnce(new Error('engine down'));
    TestBed.configureTestingModule({ providers: [provideTinyBackend(backend)] });
    const fixture = TestBed.createComponent(TimelinePanel);
    const store = TestBed.inject(WorkspaceStore);
    await settle(fixture);
    const card = el(fixture, '.err-card[role=alert]');
    expect(card.textContent).toContain('Could not load the timeline');
    expect(card.textContent).toContain('engine down');
    expect(store.error()).toBeNull();
    (card.querySelector('button') as HTMLButtonElement).click();
    await settle(fixture);
    expect(fixture.nativeElement.querySelector('.err-card')).toBeNull();
    expect(all(fixture, '.lane').length).toBeGreaterThan(0);
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
    const buttons = all<HTMLButtonElement>(fixture, '.zoom .icon-btn');
    const back = at(buttons, 0);
    const fwd = at(buttons, 1);
    const zin = at(buttons, 2);
    const zout = at(buttons, 3);
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

  it('shares one 24px target between close markers: the keyboard activates the most important, a click picks the glyph', async () => {
    const backend = tinyBackend();
    const real = backend.queryTimeline.bind(backend);
    vi.spyOn(backend, 'queryTimeline').mockImplementation(async (req) => {
      const t = await real(req);
      const lane = at(t.lanes, 0);
      const mid = (t.from + t.to) / 2;
      lane.markers = [
        { kind: 'warning', bucket: 1, ts: mid + (t.to - t.from) / 1000, id: 3, count: 1, message: 'w' },
        { kind: 'fatal', bucket: 1, ts: mid, id: 5, count: 1, message: 'f' },
      ];
      return t;
    });
    TestBed.configureTestingModule({ providers: [provideTinyBackend(backend)] });
    const fixture = TestBed.createComponent(TimelinePanel);
    const store = TestBed.inject(WorkspaceStore);
    const jump = vi.spyOn(store, 'jumpTo');
    await settle(fixture);
    const lane0 = at(all(fixture, '.lane'), 0);
    const targets = lane0.querySelectorAll<HTMLButtonElement>('.marker');
    expect(targets).toHaveLength(1);
    const target = at([...targets], 0);
    expect(target.getAttribute('aria-label')).toMatch(/^Fatal at .*; Warning at /);
    expect(parseFloat(target.style.width)).toBeGreaterThanOrEqual(24);
    expect(target.querySelectorAll('.glyph')).toHaveLength(2);
    expect(target.querySelectorAll('.glyph.primary')).toHaveLength(1);
    target.click();
    expect(jump).toHaveBeenLastCalledWith(5);
    const warning = [...target.querySelectorAll<HTMLElement>('.glyph')].find((g) => !g.classList.contains('primary'))!;
    warning.dispatchEvent(new PointerEvent('pointerenter', { clientX: 1, clientY: 1 }));
    await settle(fixture);
    expect(el(fixture, '.tip').textContent).toContain('Warning');
    warning.click();
    expect(jump).toHaveBeenLastCalledWith(3);
  });

  it('collapses warnings of a busy lane into a tick strip and keeps counts in the tooltip', async () => {
    const backend = tinyBackend();
    const real = backend.queryTimeline.bind(backend);
    vi.spyOn(backend, 'queryTimeline').mockImplementation(async (req) => {
      const t = await real(req);
      const lane = at(t.lanes, 0);
      const span = t.to - t.from;
      lane.markers = Array.from({ length: 60 }, (_, i) => ({ kind: 'warning', bucket: i, ts: t.from + (span * i) / 60, id: 2, count: 1 + (i % 3), message: 'w' }));
      lane.markers.push({ kind: 'fatal', bucket: 59, ts: t.to, id: 5, count: 1, message: 'Host terminated' });
      return t;
    });
    TestBed.configureTestingModule({ providers: [provideTinyBackend(backend)] });
    const fixture = TestBed.createComponent(TimelinePanel);
    await settle(fixture);
    const firstLane = at(all(fixture, '.lane'), 0);
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

describe('TimelinePanel custom markers', () => {
  const PROFILE = `name = "x"
[[marker]]
kind = "pay"
label = "Payment failed"
badge = "1"
shape = "hexagon"
color = "#3987e5"
description = "Payment gateway rejected the capture"
when = [{ field = "@message", op = "contains", value = "capture" }]

[[marker]]
kind = "plain"
label = "Plain"
when = [{ field = "@level", op = "eq", value = "Fatal" }]
`;

  async function render() {
    const backend = tinyBackend();
    await backend.saveProfile(PROFILE);
    TestBed.configureTestingModule({ providers: [provideTinyBackend(backend)] });
    const fixture = TestBed.createComponent(TimelinePanel);
    const store = TestBed.inject(WorkspaceStore);
    await settle(fixture);
    return { fixture, store };
  }

  it('lists custom markers in the legend with their glyph, description and a kind filter', async () => {
    const { fixture, store } = await render();
    const items = all<HTMLButtonElement>(fixture, '.legend .lg');
    expect(items.map((i) => i.textContent?.trim())).toEqual(['Payment failed', 'Plain']);
    const pay = at(items, 0);
    expect(pay.title).toBe('Payment gateway rejected the capture');
    expect(el(fixture, '#' + pay.getAttribute('aria-describedby')).textContent).toBe('Payment gateway rejected the capture');
    expect(items[1]?.getAttribute('aria-describedby')).toBeNull();
    expect(items[1]?.title).toBe('Plain');
    expect(pay.querySelector('imt-marker-glyph')!.getAttribute('data-glyph')).toBe('badge');
    expect(pay.getAttribute('aria-pressed')).toBe('false');
    // Without a kind filter no item looks switched off (the others are dimmed only while one kind is picked).
    expect(el(fixture, '.legend').classList).not.toContain('filtering');
    pay.click();
    await settle(fixture);
    expect(store.query().markers).toEqual(['pay']);
    expect(all<HTMLButtonElement>(fixture, '.legend .lg')[0]?.getAttribute('aria-pressed')).toBe('true');
    expect(el(fixture, '.legend').classList).toContain('filtering');
    at(all<HTMLButtonElement>(fixture, '.legend .lg'), 0).click();
    await settle(fixture);
    expect(store.query().markers).toEqual([]);
    expect(el(fixture, '.legend').classList).not.toContain('filtering');
  });

  it('draws the badge on the lane and describes it in the tooltip', async () => {
    const { fixture } = await render();
    const marker = all<HTMLButtonElement>(fixture, '.marker').find((m) => m.getAttribute('aria-label')?.startsWith('Payment failed'))!;
    expect(marker.querySelector<HTMLElement>('imt-marker-glyph .badge-text')!.dataset['t']).toBe('1');
    marker.dispatchEvent(new PointerEvent('pointerenter', { clientX: 10, clientY: 10 }));
    await settle(fixture);
    expect(el(fixture, '.tip-desc').textContent).toBe('Payment gateway rejected the capture');
    marker.dispatchEvent(new PointerEvent('pointerleave'));
    await settle(fixture);
  });
});

describe('TimelinePanel annotations', () => {
  async function render(backend = tinyBackend()) {
    TestBed.configureTestingModule({ providers: [provideTinyBackend(backend)] });
    const fixture = TestBed.createComponent(TimelinePanel);
    const store = TestBed.inject(WorkspaceStore);
    await settle(fixture, 8);
    return { fixture, store };
  }
  const laneOf = (f: Awaited<ReturnType<typeof render>>['fixture'], key: string) =>
    all(f, '.lane').find((l) => l.querySelector('.lane-label .key')?.textContent?.trim() === key)!;

  it('draws spans as bands on their lanes, machine-wide spans on every lane of the machine, and points as glyphs', async () => {
    const { fixture } = await render();
    const web01 = laneOf(fixture, 'WEB01 · 100');
    const kinds = [...web01.querySelectorAll('.ann')].map((b) => b.getAttribute('data-kind'));
    expect(kinds).toEqual(expect.arrayContaining(['net.decryptology.windows/service-running', 'net.decryptology.windows/downtime', 'net.decryptology.windows/app-crash']));
    expect(laneOf(fixture, 'WEB01 · 200').querySelector('[data-kind="net.decryptology.windows/downtime"]')).not.toBeNull();
    expect(laneOf(fixture, 'WEB02 · 300').querySelector('[data-kind="net.decryptology.windows/downtime"]')).toBeNull();
    const point = web01.querySelector<HTMLElement>('[data-kind="net.decryptology.windows/app-crash"]')!;
    expect(point.classList).toContain('point');
    // Severity is an icon (and a pattern class), not only a colour.
    expect(point.classList).toContain('sev-critical');
    expect(point.querySelector('imt-severity-badge')!.textContent).toContain('Critical');
  });

  it('lists the kinds in an "Annotations" legend whose items hide and show them', async () => {
    const { fixture } = await render();
    const legend = el(fixture, '.ann-legend');
    expect(legend.getAttribute('aria-label')).toBe('Annotations');
    expect(legend.textContent).toContain('Annotations');
    const downtime = el<HTMLButtonElement>(fixture, '.ann-legend [data-kind="net.decryptology.windows/downtime"]');
    expect(downtime.getAttribute('aria-pressed')).toBe('true');
    downtime.click();
    await settle(fixture);
    expect(downtime.getAttribute('aria-pressed')).toBe('false');
    expect(fixture.nativeElement.querySelector('.lane [data-kind="net.decryptology.windows/downtime"]')).toBeNull();
    downtime.click();
    await settle(fixture);
    expect(fixture.nativeElement.querySelector('.lane [data-kind="net.decryptology.windows/downtime"]')).not.toBeNull();
  });

  it('keeps bands out of the tab order and lists every annotation in view, once, in the Evidence menu', async () => {
    const { fixture, store } = await render();
    expect(all(fixture, '.ann').every((b) => b.getAttribute('tabindex') === '-1')).toBe(true);
    const menu = TestBed.inject(ContextMenuService);
    const open = vi.spyOn(menu, 'open');
    el<HTMLButtonElement>(fixture, '.ann-legend .ann-list').click();
    const items = at(open.mock.calls, 0)[2];
    const labels = items.map((i) => i.label);
    // The downtime is drawn on two lanes but listed once.
    expect(labels.filter((l) => l.startsWith('Unexpected shutdown'))).toHaveLength(1);
    expect(labels.length).toBe(new Set(labels).size);
    items.find((i) => i.label.startsWith('Unexpected shutdown'))!.run();
    await settle(fixture);
    expect(store.query().filters).toEqual([{ field: 'MachineName', op: 'eq', value: 'WEB01' }]);
  });

  it('caps the Evidence menu and says how many more are in view', async () => {
    const backend = tinyBackend();
    const real = backend.annotations.bind(backend);
    vi.spyOn(backend, 'annotations').mockImplementation(async (req) => {
      const [first] = await real(req);
      return Array.from({ length: 53 }, (_, i) => ({ ...first!, label: `A${i}`, from: first!.from + i, to: first!.from + i }));
    });
    const { fixture } = await render(backend);
    const menu = TestBed.inject(ContextMenuService);
    const open = vi.spyOn(menu, 'open');
    el<HTMLButtonElement>(fixture, '.ann-legend .ann-list').click();
    const items = at(open.mock.calls, 0)[2];
    expect(items).toHaveLength(51);
    expect(at(items, 50)).toMatchObject({ label: '3 more — zoom in to list them', disabled: true });
    expect(at(items, 50).run()).toBeUndefined();
  });

  it('shows label, owner and time on hover and applies the evidence query on click', async () => {
    const { fixture, store } = await render();
    const band = el<HTMLButtonElement>(fixture, '.lane [data-kind="net.decryptology.windows/downtime"]');
    band.dispatchEvent(new PointerEvent('pointerenter', { clientX: 20, clientY: 20 }));
    await settle(fixture);
    const tip = el(fixture, '[data-testid="annotation-tip"]').textContent!;
    expect(tip).toContain('Unexpected shutdown');
    expect(tip).toContain('net.decryptology.windows');
    expect(tip).toContain('→');
    expect(tip).toContain('MachineName = WEB01');
    band.dispatchEvent(new PointerEvent('pointerleave'));
    band.click();
    await settle(fixture);
    expect(fixture.nativeElement.querySelector('[data-testid="annotation-tip"]')).toBeNull();
    expect(store.query().filters).toEqual([{ field: 'MachineName', op: 'eq', value: 'WEB01' }]);
    expect(store.hasRange()).toBe(true);
    // Keyboard focus shows the same tooltip.
    const point = el<HTMLButtonElement>(fixture, '.lane .ann.point');
    point.dispatchEvent(new FocusEvent('focus'));
    await settle(fixture);
    expect(el(fixture, '[data-testid="annotation-tip"]').textContent).toContain('Application crashed');
    point.dispatchEvent(new FocusEvent('blur'));
  });

  it('reports a failed request inline and retries', async () => {
    const { fixture } = await render(new MockBackend({ latency: false, dataset: tinyDataset, pluginMarkerKinds: [], fail: { annotations: 1 } }));
    expect(el(fixture, '.ann-err').textContent).toContain('Annotations unavailable');
    expect(all(fixture, '.lane .ann')).toHaveLength(0);
    buttonByText(fixture, 'Retry').click();
    await settle(fixture, 8);
    expect(fixture.nativeElement.querySelector('.ann-err')).toBeNull();
    expect(all(fixture, '.lane .ann').length).toBeGreaterThan(0);
  });

  it('refreshes when the data changes (live tail, reload)', async () => {
    const backend = tinyBackend();
    const spy = vi.spyOn(backend, 'annotations');
    const { fixture, store } = await render(backend);
    const calls = spy.mock.calls.length;
    expect(calls).toBeGreaterThan(0);
    await store.refreshSummary();
    await settle(fixture, 8);
    expect(spy.mock.calls.length).toBeGreaterThan(calls);
  });
});
