import { ComponentFixture, TestBed } from '@angular/core/testing';
import { describe, expect, it, vi } from 'vitest';
import { all, buttonByText, el, settle, text } from '../../../testing/helpers';
import { FrameDto } from '../../core/api.types';
import { FakeContext } from '../../analysis/test-setup';
import { ProfileViewState } from '../profile-view-state.store';
import { ProfilingBackend } from '../profiling-backend';
import { openDemoCaptures, setupProfiling } from '../testing/profiling-test-support';
import { ButterflyView } from './butterfly-view';
import { at } from '../../../testing/must';

async function mount(opts: Parameters<typeof setupProfiling>[0] = {}, open = true) {
  const doubles = setupProfiling(opts);
  if (open) await openDemoCaptures();
  const fixture: ComponentFixture<ButterflyView> = TestBed.createComponent(ButterflyView);
  fixture.detectChanges();
  await settle(fixture, 6);
  return { fixture, doubles, ctx: doubles.context as FakeContext, state: TestBed.inject(ProfileViewState) };
}

/** A function with callers and callees, and the top-level one (no caller). */
async function pickFrames(doubles: { backend: Pick<ProfilingBackend, 'hotspots' | 'butterfly'> }, state: InstanceType<typeof ProfileViewState>) {
  const page = await doubles.backend.hotspots({ profileId: state.profileId()!, view: state.view(), metric: 'timeNs', limit: 50 });
  let middle: FrameDto | null = null;
  let top: FrameDto | null = null;
  for (const r of page.rows) {
    if (!r.frame) continue;
    const b = await doubles.backend.butterfly({ profileId: state.profileId()!, view: state.view(), frame: r.frame.id, metric: 'timeNs' });
    if (!middle && b.callers.some((c: { frame: unknown }) => c.frame) && b.callees.length) middle = r.frame;
    if (!top && b.callers.some((c: { frame: unknown }) => !c.frame)) top = r.frame;
  }
  return { middle: middle!, top: top! };
}

describe('ButterflyView', () => {
  it('shows the no-capture state', async () => {
    const { fixture } = await mount({}, false);
    expect(text(fixture)).toContain('Open a performance capture to see callers and callees.');
  });

  it('is empty until a frame is selected', async () => {
    const { fixture } = await mount();
    expect(text(fixture)).toContain('Select a function');
    expect(all(fixture, 'ul.list')).toHaveLength(0);
  });

  it('lists callers and callees with percentages and the focused function', async () => {
    const { fixture, doubles, state } = await mount();
    const { middle } = await pickFrames(doubles as never, state);
    state.selectFrame(middle);
    await settle(fixture, 6);
    expect(text(fixture)).toContain(middle.name);
    expect(el(fixture, '#bf-callers').textContent).toMatch(/Callers \(\d+\)/);
    expect(el(fixture, '#bf-callees').textContent).toMatch(/Callees \(\d+\)/);
    const callers = all<HTMLButtonElement>(fixture, 'ul[aria-labelledby=bf-callers] button');
    const callees = all<HTMLButtonElement>(fixture, 'ul[aria-labelledby=bf-callees] button');
    expect(callers.length).toBeGreaterThan(0);
    expect(callees.length).toBeGreaterThan(0);
    expect(callers[0]?.querySelector('.pct')!.textContent).toMatch(/\d/);
    expect(callers[0]?.getAttribute('aria-label')).toMatch(/^Caller .*of the function/);
    expect(callees[0]?.getAttribute('aria-label')).toMatch(/^Callee /);
    expect(el(fixture, '[aria-label="Selected function"]').textContent).toMatch(/total .* · self /);
    expect(all(fixture, '[data-measurement]').length).toBeGreaterThan(0);
  });

  it('selects a clicked caller or callee and re-queries for it', async () => {
    const { fixture, doubles, state } = await mount();
    const { middle } = await pickFrames(doubles as never, state);
    state.selectFrame(middle);
    await settle(fixture, 6);
    const spy = vi.spyOn(doubles.backend, 'butterfly');
    const caller = all<HTMLButtonElement>(fixture, 'ul[aria-labelledby=bf-callers] button').find((b) => !b.disabled)!;
    caller.click();
    await settle(fixture, 6);
    expect(state.selectedFrame()!.id).not.toBe(middle.id);
    expect(spy.mock.calls.at(-1)![0].frame).toBe(state.selectedFrame()!.id);
    expect(el(fixture, '[aria-label="Selected function"] strong').textContent).toBe(state.selectedFrame()!.name);
  });

  it('disables the "top of the stack" caller row and ignores clicks', async () => {
    const { fixture, doubles, state } = await mount();
    const { top } = await pickFrames(doubles as never, state);
    state.selectFrame(top);
    await settle(fixture, 6);
    const row = all<HTMLButtonElement>(fixture, 'ul[aria-labelledby=bf-callers] button').find((b) => b.textContent?.includes('(top of the stack'))!;
    expect(row.disabled).toBe(true);
    expect(row.getAttribute('aria-label')).toContain('(top of the stack)');
    row.click();
    expect(state.selectedFrame()!.id).toBe(top.id);
  });

  it('shows empty-list texts for a function without callees', async () => {
    const { fixture, doubles, state } = await mount();
    const page = await doubles.backend.hotspots({ profileId: state.profileId()!, view: state.view(), metric: 'timeNs', limit: 50 });
    for (const r of page.rows) {
      const b = await doubles.backend.butterfly({ profileId: state.profileId()!, view: state.view(), frame: r.frame!.id, metric: 'timeNs' });
      if (!b.callees.length) {
        state.selectFrame(r.frame);
        await settle(fixture, 6);
        expect(text(fixture)).toContain('No callees: all of its time is self time.');
        return;
      }
    }
    throw new Error('no leaf function in the demo profile');
  });

  it('shows the function in the flame graph', async () => {
    const { fixture, doubles, state, ctx } = await mount();
    const { middle } = await pickFrames(doubles as never, state);
    state.selectFrame(middle);
    await settle(fixture, 6);
    buttonByText(fixture, 'Show in flame graph').click();
    expect(ctx.last('showView')).toEqual(['profiling-flame']);
    expect(state.flameSearch()).toBe(middle.name);
  });

  it('zooms the flame graph to the function', async () => {
    const { fixture, doubles, state, ctx } = await mount();
    const { middle } = await pickFrames(doubles as never, state);
    state.selectFrame(middle);
    await settle(fixture, 6);
    buttonByText(fixture, 'Zoom flame graph to it').click();
    await settle(fixture, 6);
    expect(ctx.last('showView')).toEqual(['profiling-flame']);
    expect(state.flameFocus().at(-1)).toBe(middle.id);
  });

  it('shows an error with Retry', async () => {
    const { fixture, doubles, state } = await mount({ options: { fail: { butterfly: 1 } } });
    const frame = at((await doubles.backend.hotspots({ profileId: state.profileId()!, view: state.view(), metric: 'timeNs' })).rows, 0).frame;
    state.selectFrame(frame);
    await settle(fixture, 6);
    expect(el(fixture, '[role=alert]').textContent).toContain('butterfly failed');
    buttonByText(fixture, 'Retry').click();
    await settle(fixture, 6);
    expect(all(fixture, '[role=alert]')).toHaveLength(0);
    expect(all(fixture, 'ul.list')).toHaveLength(2);
  });
});
