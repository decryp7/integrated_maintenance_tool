import { ComponentFixture, TestBed } from '@angular/core/testing';
import { afterEach, beforeEach, vi } from 'vitest';
import { all, buttonByText, el, settle, text, typeInto } from '../../../testing/helpers';
import { MockProfilingOptions, MockProfilingBackend } from '../../core/mock/mock-profiling';
import { ThreadTimeline, ThreadTimelineRequest } from '../../core/api.types';
import { ProfileViewState } from '../profile-view-state';
import { openDemoCaptures, setupProfiling } from '../testing/profiling-test-support';
import { brushToRange, lanePath, ThreadTimelineView } from './thread-timeline-view';

interface Setup {
  options?: MockProfilingOptions;
  open?: boolean;
  hasTime?: boolean;
  patch?: (t: ThreadTimeline) => ThreadTimeline;
}

async function mount(s: Setup = {}) {
  const doubles = setupProfiling({ options: s.options });
  const backend = doubles.backend as MockProfilingBackend;
  const realList = backend.list.bind(backend);
  if (s.hasTime === false) vi.spyOn(backend, 'list').mockImplementation(async () => (await realList()).map((c) => ({ ...c, hasTime: false })) as Awaited<ReturnType<typeof realList>>);
  const real = backend.threads.bind(backend);
  const threads = vi.spyOn(backend, 'threads');
  if (s.patch) threads.mockImplementation(async (r) => s.patch!(await real(r)));
  if (s.open !== false) await openDemoCaptures();
  const state = TestBed.inject(ProfileViewState);
  const fixture: ComponentFixture<ThreadTimelineView> = TestBed.createComponent(ThreadTimelineView);
  fixture.detectChanges();
  await settle(fixture, 6);
  const last = (): ThreadTimelineRequest => threads.mock.calls.at(-1)![0];
  return { fixture, state, threads, last, backend };
}

const kindButtons = (f: ComponentFixture<unknown>) => all<HTMLButtonElement>(f, '.kinds button');
const lanes = (f: ComponentFixture<unknown>) => all<HTMLElement>(f, '.lane');
const input = (f: ComponentFixture<unknown>, label: string) => el<HTMLInputElement>(f, `input[aria-label="${label}"]`);

function stubLanes(f: ComponentFixture<unknown>, width = 1224): HTMLElement {
  const lanesEl = el(f, '.lanes');
  lanesEl.getBoundingClientRect = () => ({ left: 0, top: 0, width, height: 100, right: width, bottom: 100, x: 0, y: 0, toJSON: () => ({}) }) as DOMRect;
  return lanesEl;
}

function pointer(target: Element, type: string, clientX: number, button = 0): void {
  target.dispatchEvent(new MouseEvent(type, { clientX, button, bubbles: true }));
}

describe('lanePath and brushToRange', () => {
  it('draws bars for non-empty buckets only and nothing without data', () => {
    expect(lanePath([], 5)).toBe('');
    expect(lanePath([1, 2], 0)).toBe('');
    const d = lanePath([0, 5, 1], 5);
    expect(d.match(/M/g)).toHaveLength(2);
    expect(d).toContain('V0.00'); // the maximum fills the lane
    expect(d).toContain('V80.00');
    expect(lanePath([100, 1], 100)).toContain('V96.00'); // the minimum bar height is 4
  });

  it('maps pixels to a clamped range and ignores tiny drags', () => {
    expect(brushToRange(0, 3, 100, 0, 1000)).toBeNull();
    expect(brushToRange(10, 10, 0, 0, 1000)).toBeNull();
    expect(brushToRange(50, 10, 100, 1000, 2000)).toEqual({ fromNs: 1100, toNs: 1500 });
    expect(brushToRange(-20, 500, 100, 0, 1000)).toEqual({ fromNs: 0, toNs: 1000 });
  });
});

describe('ThreadTimelineView', () => {
  beforeEach(() => vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockReturnValue(null));
  afterEach(() => vi.restoreAllMocks());

  it('asks to open a profile', async () => {
    const { fixture, threads } = await mount({ open: false });
    expect(text(fixture)).toContain('Open a profile with timestamps to see its threads over time.');
    expect(threads).not.toHaveBeenCalled();
  });

  it('requests the timeline of the active profile and shows a lane per thread, busiest first', async () => {
    const { fixture, last, state, threads } = await mount();
    expect(last()).toMatchObject({ profileId: state.profileId(), view: state.view(), maxEvents: 500 });
    expect(last().eventKinds!.length).toBeGreaterThan(0);
    const data = await threads.mock.results.at(-1)!.value;
    const names = lanes(fixture).map((l) => l.querySelector('.ellipsis')!.textContent);
    expect(names).toEqual(data.threads.map((t: { thread: { name: string } }) => t.thread.name));
    const totals = data.threads.map((t: { total: number }) => t.total);
    expect(totals).toEqual([...totals].sort((a: number, b: number) => b - a));
    const first = el(fixture, '.plot svg[role=img]');
    expect(first.getAttribute('aria-label')).toBe(`${data.threads[0].thread.name}: ${data.threads[0].total} samples`);
    expect(el(fixture, '.plot svg[role=img] path').getAttribute('d')).toMatch(/^M/);
  });

  it('labels the axis with wall-clock times, or elapsed time without a start', async () => {
    const { fixture } = await mount();
    const axis = all(fixture, '.axis span').map((s) => s.textContent!);
    expect(axis[0]).toMatch(/\d{4}-\d{2}-\d{2}|\d{2}:\d{2}/);
  });

  it('labels the axis with elapsed time when the capture has no start', async () => {
    const { fixture } = await mount({ patch: (t) => ({ ...t, startedMs: null, fromNs: 0, toNs: 2_500_000_000 }) });
    const axis = all(fixture, '.axis span').map((s) => s.textContent);
    expect(axis[1]).toMatch(/2\.5/);
    expect(axis[0]).not.toMatch(/\d{4}-/);
  });

  it('shows event kind toggles with counts and re-requests without a hidden kind', async () => {
    const { fixture, last, threads } = await mount();
    const buttons = kindButtons(fixture);
    expect(buttons.length).toBeGreaterThan(1);
    expect(buttons.every((b) => b.getAttribute('aria-pressed') === 'true')).toBe(true);
    const data: ThreadTimeline = await threads.mock.results.at(-1)!.value;
    const gc = buttons.find((b) => b.textContent!.includes('GC'))!;
    expect(gc.textContent).toContain(`(${data.events.filter((e) => e.kind === 'gc').length})`);
    expect(last().eventKinds).toContain('gc');
    gc.click();
    await settle(fixture, 6);
    expect(last().eventKinds).not.toContain('gc');
    expect(kindButtons(fixture).find((b) => b.textContent!.includes('GC'))!.getAttribute('aria-pressed')).toBe('false');
    kindButtons(fixture).find((b) => b.textContent!.includes('GC'))!.click();
    await settle(fixture, 6);
    expect(last().eventKinds).toContain('gc');
  });

  it('highlights the emphasised kind, dims other events, and clears it when that kind is hidden', async () => {
    const { fixture, state } = await mount();
    state.setEventKind('gc');
    await settle(fixture, 3);
    const hl = kindButtons(fixture).filter((b) => b.classList.contains('hl'));
    expect(hl.map((b) => b.textContent)).toEqual([expect.stringContaining('GC')]);
    expect(all(fixture, '.ev.dim').length).toBeGreaterThan(0);
    expect(all(fixture, '.ev:not(.dim)').length).toBeGreaterThan(0);
    kindButtons(fixture).find((b) => b.textContent!.includes('JIT'))!.click(); // another kind: keeps the emphasis
    expect(state.eventKind()).toBe('gc');
    hl[0].click();
    expect(state.eventKind()).toBeNull();
    await settle(fixture, 3);
    expect(all(fixture, '.ev.dim')).toHaveLength(0);
  });

  it('toggles the thread filter from the lane label', async () => {
    const { fixture, state } = await mount();
    const label = lanes(fixture)[1].querySelector<HTMLButtonElement>('.label')!;
    expect(label.getAttribute('aria-pressed')).toBe('false');
    label.click();
    await settle(fixture, 6);
    const threadId = state.threads()![0];
    expect(state.threads()).toHaveLength(1);
    expect(lanes(fixture).map((l) => l.querySelector('.label')!.getAttribute('aria-pressed'))).toContain('true');
    expect(threadId).toBeGreaterThanOrEqual(0);
    lanes(fixture)[0].querySelector<HTMLButtonElement>('.label')!.click();
    await settle(fixture, 6);
    expect(state.threads()).toBeNull();
  });

  describe('typed range', () => {
    it('applies a range in milliseconds and clears it', async () => {
      const { fixture, state } = await mount();
      expect(buttonByText(fixture, 'Clear range').disabled).toBe(true);
      typeInto(input(fixture, 'Range start (ms)'), '100');
      typeInto(input(fixture, 'Range end (ms)'), '2500.5');
      buttonByText(fixture, 'Apply range').click();
      await settle(fixture, 6);
      expect(state.range()).toEqual({ fromNs: 100e6, toNs: 2500.5e6 });
      expect(input(fixture, 'Range start (ms)').value).toBe('100');
      expect(input(fixture, 'Range end (ms)').value).toBe('2500.5');
      expect(el(fixture, '[data-testid=range-pill]').textContent).toContain('Range');
      buttonByText(fixture, 'Clear range').click();
      await settle(fixture, 6);
      expect(state.range()).toBeNull();
      expect(input(fixture, 'Range start (ms)').value).toBe('');
    });

    it('swaps a reversed range', async () => {
      const { fixture, state } = await mount();
      typeInto(input(fixture, 'Range start (ms)'), '900');
      typeInto(input(fixture, 'Range end (ms)'), '100');
      buttonByText(fixture, 'Apply range').click();
      expect(state.range()).toEqual({ fromNs: 100e6, toNs: 900e6 });
    });

    it('ignores empty, negative and non-numeric input', async () => {
      const { fixture, state } = await mount();
      const apply = () => buttonByText(fixture, 'Apply range').click();
      apply();
      typeInto(input(fixture, 'Range start (ms)'), '5');
      apply();
      typeInto(input(fixture, 'Range end (ms)'), '-3');
      apply();
      typeInto(input(fixture, 'Range start (ms)'), '-1');
      typeInto(input(fixture, 'Range end (ms)'), '9');
      apply();
      expect(state.range()).toBeNull();
    });
  });

  describe('brushing', () => {
    // The lanes box is 1224 wide: the plot starts at 212 and is 1000 px wide.
    it('sets the shared range from a drag, showing the brush while dragging', async () => {
      const { fixture, state, threads } = await mount();
      const data: ThreadTimeline = await threads.mock.results.at(-1)!.value;
      const lanesEl = stubLanes(fixture);
      pointer(lanesEl, 'pointerdown', 312);
      pointer(lanesEl, 'pointermove', 512);
      fixture.detectChanges();
      const brush = el(fixture, '.brush');
      expect(brush.style.left).toBe('312px');
      expect(brush.style.width).toBe('200px');
      pointer(lanesEl, 'pointerup', 712);
      fixture.detectChanges();
      expect(fixture.nativeElement.querySelector('.brush')).toBeNull();
      const span = data.toNs - data.fromNs;
      expect(state.range()).toEqual({ fromNs: Math.floor(data.fromNs + span * 0.1), toNs: Math.ceil(data.fromNs + span * 0.5) });
    });

    it('brushes right to left and clamps to the plot', async () => {
      const { fixture, state, threads } = await mount();
      const data: ThreadTimeline = await threads.mock.results.at(-1)!.value;
      const lanesEl = stubLanes(fixture);
      pointer(lanesEl, 'pointerdown', 712);
      pointer(lanesEl, 'pointermove', 100); // left of the plot: clamped to 0
      fixture.detectChanges();
      expect(el(fixture, '.brush').style.left).toBe('212px');
      pointer(lanesEl, 'pointerup', 100);
      expect(state.range()).toEqual({ fromNs: data.fromNs, toNs: Math.ceil(data.fromNs + (data.toNs - data.fromNs) * 0.5) });
    });

    it('ignores tiny drags, other buttons, presses on the label column and moves without a press', async () => {
      const { fixture, state } = await mount();
      const lanesEl = stubLanes(fixture);
      pointer(lanesEl, 'pointermove', 300); // no brush yet
      pointer(lanesEl, 'pointerup', 300);
      pointer(lanesEl, 'pointerdown', 400);
      pointer(lanesEl, 'pointerup', 402);
      expect(state.range()).toBeNull();
      pointer(lanesEl, 'pointerdown', 400, 2);
      fixture.detectChanges();
      expect(fixture.nativeElement.querySelector('.brush')).toBeNull();
      pointer(lanesEl, 'pointerdown', 50); // in the label column
      fixture.detectChanges();
      expect(fixture.nativeElement.querySelector('.brush')).toBeNull();
      expect(state.range()).toBeNull();
    });

    it('drops the brush on pointercancel', async () => {
      const { fixture, state } = await mount();
      const lanesEl = stubLanes(fixture);
      pointer(lanesEl, 'pointerdown', 400);
      pointer(lanesEl, 'pointermove', 600);
      fixture.detectChanges();
      expect(el(fixture, '.brush')).toBeTruthy();
      pointer(lanesEl, 'pointercancel', 600);
      fixture.detectChanges();
      expect(fixture.nativeElement.querySelector('.brush')).toBeNull();
      pointer(lanesEl, 'pointerup', 800);
      expect(state.range()).toBeNull();
    });

    it('does not start a brush from a lane label press', async () => {
      const { fixture } = await mount();
      stubLanes(fixture);
      lanes(fixture)[0].querySelector('.label')!.dispatchEvent(new MouseEvent('pointerdown', { clientX: 400, bubbles: true }));
      fixture.detectChanges();
      expect(fixture.nativeElement.querySelector('.brush')).toBeNull();
    });
  });

  it('lists the events in view with kind, thread, start and duration', async () => {
    const { fixture, threads } = await mount();
    const data: ThreadTimeline = await threads.mock.results.at(-1)!.value;
    expect(el(fixture, '.events summary').textContent).toBe(`Events in view (${data.events.length})`);
    const rows = all(fixture, '.events tbody tr');
    expect(rows).toHaveLength(data.events.length);
    const cells = rows[0].querySelectorAll('td');
    expect(cells[0].textContent).toMatch(/GC|JIT|Exception|Lock wait|Marker/);
    expect(cells[1].textContent).not.toBe('');
    expect(cells[4].textContent).toBe(data.events[0].label);
  });

  it('names process-wide and unknown threads in the events table', async () => {
    const { fixture } = await mount({
      patch: (t) => ({
        ...t,
        events: [
          { kind: 'gc', thread: null, startNs: 1e6, durNs: 0, label: 'process GC' },
          { kind: 'marker', thread: 9999, startNs: 2e6, durNs: 5e6, label: 'lost' },
        ] as ThreadTimeline['events'],
      }),
    });
    const rows = all(fixture, '.events tbody tr').map((r) => [...r.querySelectorAll('td')].map((c) => c.textContent));
    expect(rows[0][1]).toBe('process');
    expect(rows[0][3]).toBe('—');
    expect(rows[1][1]).toBe('thread 9999');
    expect(rows[1][3]).not.toBe('—');
  });

  it('tells when events were truncated', async () => {
    expect(text((await mount({ patch: (t) => ({ ...t, eventsTruncated: true }) })).fixture)).toContain('Only the first 500 events are shown');
  });

  it('shows no truncation notice normally', async () => {
    expect(text((await mount()).fixture)).not.toContain('Only the first');
  });

  it('says so when the capture has no timestamps', async () => {
    const { fixture, threads } = await mount({ hasTime: false });
    expect(text(fixture)).toContain('This capture has no timestamps');
    expect(threads).not.toHaveBeenCalled();
    expect(all(fixture, '.lane')).toHaveLength(0);
  });

  it('says so when there is no thread activity in the range', async () => {
    const { fixture } = await mount({ patch: (t) => ({ ...t, threads: [], events: [] }) });
    expect(text(fixture)).toContain('No thread activity in this range.');
  });

  it('shows the error with Retry and recovers', async () => {
    const { fixture } = await mount({ options: { fail: { threads: 1 } } });
    expect(el(fixture, '[role=alert]').textContent).toContain('mock: threads failed');
    buttonByText(fixture, 'Retry').click();
    await settle(fixture, 6);
    expect(fixture.nativeElement.querySelector('[role=alert]')).toBeNull();
    expect(lanes(fixture).length).toBeGreaterThan(0);
  });
});
