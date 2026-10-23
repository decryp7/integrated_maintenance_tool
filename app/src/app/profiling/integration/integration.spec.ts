import { ComponentFixture, TestBed } from '@angular/core/testing';
import { describe, expect, it } from 'vitest';
import { all, buttonByText, el, settle, text, tinyBackend, MemoryLogger } from '../../../testing/helpers';
import { ProfileSpan } from '../../core/api.types';
import { DebriefBackend } from '../../core/backend';
import { Logger } from '../../core/logger';
import { COMMAND_CONTEXT } from '../../extensions/command-registry';
import { FakeContext } from '../../analysis/test-setup';
import { ProfileViewState } from '../profile-view-state';
import { ProfilingBackend } from '../profiling-backend';
import { openDemoCaptures, setupProfiling } from '../testing/profiling-test-support';
import { ProfileMomentSection } from './profile-moment';
import { labelWidth, layoutSpans, ProfileSpanTrack } from './profile-span-track';
import { at } from '../../../testing/must';

type View = { from: number; to: number };

async function mountTrack(opts: Parameters<typeof setupProfiling>[0] = {}, view: View | null = null, open = true) {
  const doubles = setupProfiling(opts);
  if (open) await openDemoCaptures();
  const spans = await doubles.backend.spans();
  const fixture: ComponentFixture<ProfileSpanTrack> = TestBed.createComponent(ProfileSpanTrack);
  const wide: View = { from: Math.min(...spans.map((s) => s.fromMs)) - 1000, to: Math.max(...spans.map((s) => s.toMs)) + 1000 };
  fixture.componentRef.setInput('view', view ?? wide);
  fixture.detectChanges();
  await settle(fixture, 4);
  return { fixture, doubles, ctx: doubles.context as FakeContext, state: TestBed.inject(ProfileViewState), spans, wide };
}

const chips = (f: ComponentFixture<unknown>) => all<HTMLButtonElement>(f, '[data-testid=profile-span]');

const span = (patch: Partial<ProfileSpan> & Pick<ProfileSpan, 'id' | 'fromMs' | 'toMs'>): ProfileSpan => ({ name: `cap-${patch.id}`, capture: 'profile', kind: null, ...patch });

describe('layoutSpans', () => {
  it('drops spans outside the view, positions in percent, stacks overlapping labels and clips ranges', () => {
    const spans = [
      span({ id: 1, fromMs: 0, toMs: 100, name: 'before' }),
      span({ id: 2, fromMs: 1000, toMs: 3000, name: 'a' }),
      span({ id: 3, fromMs: 1010, toMs: 1500, name: 'b-overlaps' }),
      span({ id: 4, fromMs: 1900, toMs: 1900, name: 'heap', capture: 'heap' }),
      span({ id: 5, fromMs: 2000, toMs: 2500, name: 'inside' }),
    ];
    const out = layoutSpans(spans, { from: 1000, to: 2000 }, 1000);
    expect(out.map((o) => o.span.id)).toEqual([2, 3, 4, 5]);
    expect(out[0]?.left).toBe(0);
    expect(out[0]?.width).toBe(100);
    expect(out[0]?.range).toEqual({ fromNs: 0, toNs: 1_000_000_000 });
    expect(out[1]?.row).toBe(1);
    expect(out[0]?.row).toBe(0);
    expect(out[2]?.range).toBeNull();
    expect(out[3]?.left).toBe(100);
    // a span fully inside is not clipped
    expect(layoutSpans([span({ id: 6, fromMs: 1100, toMs: 1200 })], { from: 1000, to: 2000 })[0]?.range).toBeNull();
    // degenerate view does not divide by zero
    expect(layoutSpans([span({ id: 7, fromMs: 5, toMs: 5 })], { from: 5, to: 5 })[0]?.left).toBe(0);
    expect(labelWidth('x'.repeat(100))).toBe(220);
  });
});

describe('ProfileSpanTrack', () => {
  it('renders a chip per open capture inside the view, with title and accessible name', async () => {
    const { fixture, spans } = await mountTrack();
    expect(chips(fixture)).toHaveLength(spans.length);
    const orders = chips(fixture).find((c) => c.textContent?.includes('orders.speedscope.json'))!;
    expect(orders.textContent).toContain('▶ ');
    expect(orders.getAttribute('aria-label')).toBe('Open capture orders.speedscope.json');
    expect(orders.title).toContain('Click to open.');
    const heap = chips(fixture).find((c) => c.textContent?.includes('heap-1'))!;
    expect(heap.textContent).toContain('◆ ');
    expect(heap.getAttribute('aria-label')).toBe('Open heap snapshot heap-1.heapsnapshot');
    expect(all(fixture, '.band.heap').length).toBeGreaterThan(0);
  });

  it('stacks overlapping labels on different rows via the top style', async () => {
    const { fixture } = await mountTrack();
    const tops = new Set(chips(fixture).map((c) => c.style.top));
    expect(tops.size).toBeGreaterThan(1);
    for (const t of tops) expect(t).toMatch(/^\d+px$/);
    expect(chips(fixture).every((c) => parseInt(c.style.top) >= 52)).toBe(true);
  });

  it('renders no chips without a view or when the view excludes all captures', async () => {
    const { fixture, wide } = await mountTrack();
    fixture.componentRef.setInput('view', { from: wide.to + 10_000, to: wide.to + 20_000 });
    fixture.detectChanges();
    expect(chips(fixture)).toHaveLength(0);
    fixture.componentRef.setInput('view', null);
    fixture.detectChanges();
    expect(chips(fixture)).toHaveLength(0);
  });

  it('opens a profile on the clipped visible range', async () => {
    const orders = (await mountTrack()).spans.find((s) => s.name === 'orders.speedscope.json')!;
    TestBed.resetTestingModule();
    const mid = orders.fromMs + (orders.toMs - orders.fromMs) / 2;
    const { fixture, ctx, state } = await mountTrack({}, { from: orders.fromMs - 5000, to: mid });
    const chip = chips(fixture).find((c) => c.textContent?.includes('orders.speedscope.json'))!;
    expect(chip.getAttribute('aria-label')).toContain('on the visible range');
    chip.click();
    expect(ctx.last('showView')).toEqual(['profiling-hotspots']);
    expect(state.range()!.fromNs).toBe(0);
    expect(state.range()!.toNs).toBeGreaterThan(0);
    expect(state.range()!.toNs).toBeLessThan(orders.toMs * 1e6);
    expect(state.profileId()).not.toBeNull();
  });

  it('opens a fully visible profile without a range and a heap snapshot in the heap view', async () => {
    const { fixture, ctx, state } = await mountTrack();
    chips(fixture).find((c) => c.textContent?.includes('regression'))!.click();
    expect(state.profileId()).toBe(2);
    expect(state.range()).toBeNull();
    expect(ctx.last('showView')).toEqual(['profiling-hotspots']);
    chips(fixture).find((c) => c.textContent?.includes('heap-2'))!.click();
    expect(state.heapId()).toBe(4);
    expect(ctx.last('showView')).toEqual(['profiling-heap']);
  });

  it('does not let a pointer-down on a chip reach the timeline', async () => {
    const { fixture } = await mountTrack();
    let reached = false;
    (fixture.nativeElement as HTMLElement).addEventListener('pointerdown', () => (reached = true));
    at(chips(fixture), 0).dispatchEvent(new Event('pointerdown', { bubbles: true }));
    expect(reached).toBe(false);
  });

  it('reloads the spans when captures change', async () => {
    const { fixture, doubles, state, spans } = await mountTrack({}, null, false);
    expect(chips(fixture)).toHaveLength(0);
    await openDemoCaptures();
    state.capturesChanged();
    fixture.componentRef.setInput('view', { from: 0, to: Number.MAX_SAFE_INTEGER / 4 });
    await settle(fixture, 4);
    expect(chips(fixture)).toHaveLength((await doubles.backend.spans()).length);
    expect(spans).toHaveLength(0);
  });

  it('renders nothing without the profiling backend', async () => {
    TestBed.configureTestingModule({
      providers: [
        { provide: DebriefBackend, useFactory: () => tinyBackend() },
        { provide: Logger, useFactory: () => new MemoryLogger() },
        { provide: COMMAND_CONTEXT, useValue: new FakeContext() },
      ],
    });
    const fixture = TestBed.createComponent(ProfileSpanTrack);
    fixture.componentRef.setInput('view', { from: 0, to: 1e15 });
    fixture.detectChanges();
    await settle(fixture, 3);
    expect(chips(fixture)).toHaveLength(0);
  });

  it('shows no chips when loading the spans fails', async () => {
    setupProfiling({ options: { fail: { spans: 3 } } });
    const fixture = TestBed.createComponent(ProfileSpanTrack);
    fixture.componentRef.setInput('view', { from: 0, to: 1e15 });
    fixture.detectChanges();
    await settle(fixture, 4);
    expect(chips(fixture)).toHaveLength(0);
  });
});

async function mountMoment(opts: Parameters<typeof setupProfiling>[0] = {}, open = true, range: { from: number; to: number } | null = null) {
  const doubles = setupProfiling(opts);
  if (open) await openDemoCaptures();
  const spans = await doubles.backend.spans();
  const orders = spans.find((s) => s.name === 'orders.speedscope.json')!;
  const fixture: ComponentFixture<ProfileMomentSection> = TestBed.createComponent(ProfileMomentSection);
  fixture.componentRef.setInput('range', range ?? { from: orders.fromMs, to: orders.toMs });
  fixture.detectChanges();
  await settle(fixture, 6);
  return { fixture, doubles, ctx: doubles.context as FakeContext, state: TestBed.inject(ProfileViewState), orders };
}

describe('ProfileMomentSection', () => {
  it('lists the profiles with samples in the window with their top functions', async () => {
    const { fixture } = await mountMoment();
    expect(text(fixture)).toContain('Profile samples in this window');
    const card = el(fixture, '[aria-label="Profile orders.speedscope.json"]');
    expect(card.textContent).toMatch(/[\d,]+ samples · /);
    expect(card.querySelector('[data-measurement]')).not.toBeNull();
    const items = [...card.querySelectorAll('ol.top li')];
    expect(items.length).toBeGreaterThan(0);
    expect(items[0]?.textContent).toMatch(/^\d+(\.\d+)?%/);
  });

  it('opens the profile on the moment range', async () => {
    const { fixture, ctx, state } = await mountMoment();
    buttonByText(fixture, 'Open capture on this range').click();
    expect(ctx.last('showView')).toEqual(['profiling-hotspots']);
    expect(state.profileId()).toBe(1);
    expect(state.range()).not.toBeNull();
    expect(state.range()!.toNs).toBeGreaterThan(state.range()!.fromNs);
  });

  it('is hidden when no profile has samples in the window', async () => {
    const { fixture } = await mountMoment({}, true, { from: 1, to: 2 });
    expect(text(fixture).trim()).toBe('');
  });

  it('is hidden without a range', async () => {
    const { fixture } = await mountMoment();
    fixture.componentRef.setInput('range', null);
    fixture.detectChanges();
    await settle(fixture, 3);
    expect(text(fixture).trim()).toBe('');
  });

  it('is hidden without the profiling backend', async () => {
    TestBed.configureTestingModule({
      providers: [
        { provide: DebriefBackend, useFactory: () => tinyBackend() },
        { provide: Logger, useFactory: () => new MemoryLogger() },
        { provide: COMMAND_CONTEXT, useValue: new FakeContext() },
      ],
    });
    const fixture = TestBed.createComponent(ProfileMomentSection);
    fixture.componentRef.setInput('range', { from: 0, to: 1e15 });
    fixture.detectChanges();
    await settle(fixture, 3);
    expect(TestBed.inject(ProfilingBackend, null, { optional: true })).toBeNull();
    expect(text(fixture).trim()).toBe('');
  });

  it('shows the error with Retry, then the results', async () => {
    const { fixture } = await mountMoment({ options: { fail: { moment: 1 } } });
    expect(el(fixture, '[role=alert]').textContent).toContain('moment failed');
    buttonByText(fixture, 'Retry').click();
    await settle(fixture, 6);
    expect(all(fixture, '[role=alert]')).toHaveLength(0);
    expect(all(fixture, 'section.card').length).toBeGreaterThan(0);
  });

  it('refreshes when captures change', async () => {
    const { fixture, state } = await mountMoment({}, false, { from: 0, to: 1 });
    expect(text(fixture).trim()).toBe('');
    await openDemoCaptures();
    const orders = (await TestBed.inject(ProfilingBackend).spans()).find((s) => s.name === 'orders.speedscope.json')!;
    fixture.componentRef.setInput('range', { from: orders.fromMs, to: orders.toMs });
    state.capturesChanged();
    await settle(fixture, 6);
    expect(all(fixture, 'section.card').length).toBeGreaterThan(0);
  });
});
