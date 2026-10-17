import { ComponentFixture, TestBed } from '@angular/core/testing';
import { describe, expect, it } from 'vitest';
import { all, buttonByText, el, settle, text } from '../../../testing/helpers';
import { Evidence, FrameDto } from '../../core/api.types';
import { FakeContext } from '../../analysis/test-setup';
import { ProfileViewState } from '../profile-view-state';
import { openDemoCaptures, setupProfiling } from '../testing/profiling-test-support';
import { evidenceSummary, FindingsView } from './findings-view';

async function mount(opts: Parameters<typeof setupProfiling>[0] = {}, open = true) {
  const doubles = setupProfiling(opts);
  if (open) await openDemoCaptures();
  const fixture: ComponentFixture<FindingsView> = TestBed.createComponent(FindingsView);
  fixture.detectChanges();
  await settle(fixture, 6);
  return { fixture, doubles, ctx: doubles.context as FakeContext, state: TestBed.inject(ProfileViewState) };
}

const frame: FrameDto = { id: 3, name: 'A.B.C', module: 'A', file: null, line: null, kind: 'managed', isFramework: false };
const ev = (patch: Partial<Evidence> = {}): Evidence => ({ view: 'callTree', profileId: 1, baselineId: null, frame: null, path: [], metric: 'timeNs', range: null, threads: null, eventKind: null, ...patch });

describe('evidenceSummary', () => {
  it('describes the target view and its narrowing', () => {
    expect(evidenceSummary(ev())).toBe('call tree');
    expect(evidenceSummary(ev({ view: 'flame', frame }))).toBe('flame graph · A.B.C');
    expect(evidenceSummary(ev({ view: 'butterfly', path: [1, 2, 3] }))).toBe('callers and callees · path of 3 frames');
    expect(evidenceSummary(ev({ view: 'timeline', threads: [1], eventKind: 'gc' }))).toBe('thread timeline · 1 thread · gc events');
    expect(evidenceSummary(ev({ view: 'diff', threads: [1, 2] }))).toBe('comparison · 2 threads');
    expect(evidenceSummary(ev({ view: 'hotspots', range: { fromNs: 0, toNs: 2_000_000 } }))).toMatch(/^hotspots · .+ – .+/);
  });
});

describe('FindingsView', () => {
  it('shows the no-capture state', async () => {
    const { fixture } = await mount({}, false);
    expect(text(fixture)).toContain('Open a profile to see automatic findings.');
  });

  it('lists findings with severity label, title, detail, value and evidence summary', async () => {
    const { fixture } = await mount();
    const items = all(fixture, 'li.finding');
    expect(items.length).toBeGreaterThan(1);
    const lock = items.find((i) => i.getAttribute('data-kind') === 'lockContention')!;
    expect(lock.querySelector('imt-severity-badge')!.textContent).toContain('Critical');
    expect(lock.querySelector('.title')!.textContent).toBe('Lock contention in OrderRepository.Save');
    expect(lock.querySelector('.detail')!.textContent).toContain('Monitor.Enter');
    expect(lock.querySelector('.mono')!.textContent).toMatch(/\d/);
    expect(lock.textContent).toContain('call tree · ');
    expect(items.some((i) => i.querySelector('imt-severity-badge')!.textContent?.includes('Info'))).toBe(true);
    expect(items.some((i) => i.querySelector('imt-severity-badge')!.textContent?.includes('Warning'))).toBe(true);
    // Sorted critical first.
    expect(items[0].querySelector('imt-severity-badge')!.textContent).toContain('Critical');
    expect(text(fixture)).not.toContain('including regressions');
  });

  it('opens the evidence: shows its view and applies it to the shared state', async () => {
    const { fixture, ctx, state } = await mount();
    el<HTMLButtonElement>(fixture, 'button[aria-label="Open evidence for Lock contention in OrderRepository.Save"]').click();
    expect(ctx.last('showView')).toEqual(['profiling-tree']);
    expect(state.selectedFrame()!.name).toContain('Monitor.Enter');
    expect(state.reveal()!.path.length).toBeGreaterThan(0);
    const gc = all(fixture, 'li.finding').find((i) => i.getAttribute('data-kind') === 'gcPressure');
    if (gc) {
      gc.querySelector('button')!.click();
      expect(ctx.last('showView')).toEqual(['profiling-threads']);
      expect(state.eventKind()).toBe('gc');
    }
  });

  it('adds regression findings and a note when a baseline is set', async () => {
    const { fixture, state } = await mount();
    const target = state.profileId()!;
    state.setProfile(2);
    state.setBaseline(target);
    await settle(fixture, 6);
    expect(text(fixture)).toContain('including regressions against the baseline');
    expect(all(fixture, 'li.finding').some((i) => i.getAttribute('data-kind') === 'regression')).toBe(true);
    const reg = all(fixture, 'li.finding').find((i) => i.getAttribute('data-kind') === 'regression')!;
    reg.querySelector('button')!.click();
    expect(state.baselineId()).toBe(target);
  });

  it('ignores a baseline equal to the profile', async () => {
    const { fixture, doubles, state } = await mount();
    state.setBaseline(state.profileId()!);
    await settle(fixture, 6);
    expect(all(fixture, 'li.finding').length).toBeGreaterThan(0);
    expect(doubles.backend).toBeTruthy();
  });

  it('shows the empty state when nothing crosses a threshold', async () => {
    const { fixture, state } = await mount();
    state.setThreads([99999]);
    await settle(fixture, 6);
    expect(text(fixture)).toContain('No findings: nothing in this view crosses a threshold.');
  });

  it('shows an error with Retry', async () => {
    const { fixture } = await mount({ options: { fail: { findings: 1 } } });
    expect(el(fixture, '[role=alert]').textContent).toContain('findings failed');
    buttonByText(fixture, 'Retry').click();
    await settle(fixture, 6);
    expect(all(fixture, 'li.finding').length).toBeGreaterThan(0);
  });
});
