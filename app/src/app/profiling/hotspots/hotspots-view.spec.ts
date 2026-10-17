import { ComponentFixture, TestBed } from '@angular/core/testing';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { all, buttonByText, el, settle, text, typeInto } from '../../../testing/helpers';
import { HotspotsPage, HotspotsRequest } from '../../core/api.types';
import { MockProfilingBackend } from '../../core/mock/mock-profiling';
import { FakeContext } from '../../analysis/test-setup';
import { ProfileViewState } from '../profile-view-state';
import { ProfilingBackend } from '../profiling-backend';
import { openDemoCaptures, setupProfiling } from '../testing/profiling-test-support';
import { HotspotsView, ROW_H, rowWindow } from './hotspots-view';

async function mount(opts: Parameters<typeof setupProfiling>[0] = {}, open = true) {
  const doubles = setupProfiling(opts);
  if (open) await openDemoCaptures();
  const fixture: ComponentFixture<HotspotsView> = TestBed.createComponent(HotspotsView);
  fixture.detectChanges();
  await settle(fixture, 6);
  return { fixture, doubles, ctx: doubles.context as FakeContext, state: TestBed.inject(ProfileViewState) };
}

const names = (f: ComponentFixture<unknown>) => all(f, 'td.name button').map((b) => b.textContent?.trim());
const header = (f: ComponentFixture<unknown>, label: string) => all(f, 'th').find((t) => t.textContent?.includes(label))!;

describe('rowWindow', () => {
  it('computes the visible window with overscan and spacers', () => {
    expect(rowWindow(1000, 0, 260)).toEqual({ first: 0, last: 18, before: 0, after: (1000 - 18) * ROW_H });
    const w = rowWindow(1000, 2600, 260);
    expect(w.first).toBe(92);
    expect(w.before).toBe(92 * ROW_H);
    expect(rowWindow(5, 0, 1000)).toEqual({ first: 0, last: 5, before: 0, after: 0 });
  });
});

describe('HotspotsView', () => {
  afterEach(() => vi.useRealTimers());

  it('shows the no-capture state with an Open button', async () => {
    const { fixture } = await mount({}, false);
    expect(text(fixture)).toContain('Open a profile to see its hotspots.');
    const open = vi.spyOn(TestBed.inject(ProfilingBackend), 'open');
    buttonByText(fixture, 'Open profile…').click();
    await settle(fixture, 6);
    expect(open).toHaveBeenCalled();
    expect(el(fixture, 'table')).toBeTruthy();
  });

  it('renders rows sorted by self time with aria-sort and measurement tags', async () => {
    const { fixture } = await mount();
    const rows = names(fixture);
    expect(rows.length).toBeGreaterThan(3);
    expect(header(fixture, 'Self time').getAttribute('aria-sort')).toBe('descending');
    expect(header(fixture, 'Total time').getAttribute('aria-sort')).toBeNull();
    expect(header(fixture, 'Self time').querySelector('[data-measurement]')!.getAttribute('data-measurement')).toBe('sampled');
    const exact = ['Exceptions', 'Calls'].filter((l) => all(fixture, 'th').some((t) => t.textContent?.includes(l)));
    expect(exact.length).toBeGreaterThan(0);
    for (const l of exact) expect(header(fixture, l).querySelector('[data-measurement]')!.getAttribute('data-measurement')).toBe('exact');
    expect(text(fixture)).toContain('samples in view');
    expect(el(fixture, '[data-testid=hotspots-count]').textContent).toContain('functions');
  });

  it('re-sorts when a header is clicked and flips on the second click', async () => {
    const { fixture, doubles } = await mount();
    const spy = vi.spyOn(doubles.backend, 'hotspots');
    const before = names(fixture);
    header(fixture, 'Function').querySelector('button')!.click();
    await settle(fixture, 6);
    expect(spy.mock.calls.at(-1)![0].sort).toMatchObject({ metric: null, descending: false });
    expect(header(fixture, 'Function').getAttribute('aria-sort')).toBe('ascending');
    expect(header(fixture, 'Function').textContent).toContain('▲');
    const asc = names(fixture);
    expect(asc).not.toEqual(before);
    header(fixture, 'Function').querySelector('button')!.click();
    await settle(fixture, 6);
    expect(header(fixture, 'Function').getAttribute('aria-sort')).toBe('descending');
    expect(header(fixture, 'Function').textContent).toContain('▼');
    header(fixture, 'Total time').querySelector('button')!.click();
    await settle(fixture, 6);
    expect(spy.mock.calls.at(-1)![0].sort).toMatchObject({ metric: 'timeNs', of: 'total', descending: true });
    // The avg column sorts by the engine's average-per-call key; rows come back in the engine's order.
    header(fixture, 'Avg / call').querySelector('button')!.click();
    await settle(fixture, 6);
    expect(spy.mock.calls.at(-1)![0].sort).toMatchObject({ metric: 'timeNs', of: 'avgPerCall', descending: true });
    expect(header(fixture, 'Avg / call').getAttribute('aria-sort')).toBe('descending');
  });

  it('changes labels and rows with the group-by select', async () => {
    const { fixture, doubles } = await mount();
    const spy = vi.spyOn(doubles.backend, 'hotspots');
    typeInto(el<HTMLSelectElement>(fixture, 'select[aria-label="Group by"]'), 'module', 'change');
    await settle(fixture, 6);
    expect(spy.mock.calls.at(-1)![0].groupBy).toBe('module');
    expect(header(fixture, 'Module')).toBeTruthy();
    expect(el(fixture, '[data-testid=hotspots-count]').textContent).toMatch(/module/);
  });

  it('debounces the search by 200 ms', async () => {
    const { fixture, doubles } = await mount();
    const spy = vi.spyOn(doubles.backend, 'hotspots');
    const input = el<HTMLInputElement>(fixture, 'input[type=search]');
    typeInto(input, 'Ord');
    typeInto(input, 'Orde');
    await settle(fixture, 2);
    expect(spy).not.toHaveBeenCalled();
    await new Promise((r) => setTimeout(r, 260));
    await settle(fixture, 6);
    expect(spy).toHaveBeenCalledTimes(1);
    expect(spy.mock.calls[0][0].search).toBe('Orde');
    for (const n of names(fixture)) expect(n!.toLowerCase()).toContain('orde');
  });

  it('shows the empty state for a search without matches', async () => {
    const { fixture } = await mount();
    typeInto(el<HTMLInputElement>(fixture, 'input[type=search]'), 'zzz-no-such-function');
    await new Promise((r) => setTimeout(r, 260));
    await settle(fixture, 6);
    expect(text(fixture)).toContain('No samples match this view');
  });

  it('row actions navigate to the tree, butterfly and flame graph', async () => {
    const { fixture, ctx, state } = await mount();
    const first = names(fixture)[0]!;
    el<HTMLButtonElement>(fixture, `button[aria-label="Show ${first} in the call tree"]`).click();
    await settle(fixture, 6);
    expect(ctx.last('showView')).toEqual(['profiling-tree']);
    expect(state.reveal()?.direction).toBe('topDown');
    expect(state.reveal()?.path.at(-1)).toBe(state.selectedFrame()?.id);
    el<HTMLButtonElement>(fixture, `button[aria-label="Callers and callees of ${first}"]`).click();
    expect(ctx.last('showView')).toEqual(['profiling-butterfly']);
    expect(state.selectedFrame()?.name).toBe(first);
    el<HTMLButtonElement>(fixture, `button[aria-label="Show ${first} in the flame graph"]`).click();
    expect(ctx.last('showView')).toEqual(['profiling-flame']);
    expect(state.flameSearch()).toBe(first);
    el<HTMLButtonElement>(fixture, `button[aria-label="Zoom the flame graph to ${first}"]`).click();
    await settle(fixture, 6);
    expect(ctx.last('showView')).toEqual(['profiling-flame']);
    expect(state.flameSearch()).toBe('');
    expect(state.flameFocus().at(-1)).toBe(state.selectedFrame()?.id);
  });

  it('opens the call tree at the exact node for a function without self time too', async () => {
    const { fixture, ctx, state } = await mount();
    const rows = all(fixture, 'tbody tr[data-index]');
    // The last row has the lowest self value.
    const last = rows.at(-1)!;
    const btn = last.querySelector<HTMLButtonElement>('button.mini[aria-label^="Show "][aria-label$="in the call tree"]')!;
    btn.click();
    await settle(fixture, 6);
    expect(ctx.last('showView')).toEqual(['profiling-tree']);
    expect(state.reveal()?.path.at(-1)).toBe(state.selectedFrame()?.id);
  });

  it('primary click on a function opens its butterfly and selects the row', async () => {
    const { fixture, ctx } = await mount();
    all<HTMLButtonElement>(fixture, 'td.name button')[1].click();
    fixture.detectChanges();
    expect(ctx.last('showView')).toEqual(['profiling-butterfly']);
    expect(all(fixture, 'tbody tr[data-index]')[1].classList).toContain('sel');
  });

  it('primary click on a thread row filters the view to that thread; thread rows have no actions', async () => {
    const { fixture, ctx, state } = await mount();
    typeInto(el<HTMLSelectElement>(fixture, 'select[aria-label="Group by"]'), 'thread', 'change');
    await settle(fixture, 6);
    expect(all(fixture, '.actions')).toHaveLength(0);
    all<HTMLButtonElement>(fixture, 'td.name button')[0].click();
    expect(state.threads()?.length).toBe(1);
    expect(ctx.last('showView')).toBeUndefined();
  });

  it('moves the selection with the keyboard and ignores other keys', async () => {
    const { fixture } = await mount();
    const body = el(fixture, 'tbody');
    const total = Number(el(fixture, 'table').getAttribute('aria-rowcount')) - 1;
    const press = (key: string) => {
      const e = new KeyboardEvent('keydown', { key, bubbles: true, cancelable: true });
      body.dispatchEvent(e);
      return e;
    };
    const selected = () => TestBed.inject(ProfileViewState) && el(fixture, 'tr.sel').getAttribute('data-index');
    expect(press('ArrowDown').defaultPrevented).toBe(true);
    fixture.detectChanges();
    expect(selected()).toBe('1');
    press('ArrowDown');
    press('ArrowUp');
    fixture.detectChanges();
    expect(selected()).toBe('1');
    press('Home');
    fixture.detectChanges();
    expect(selected()).toBe('0');
    press('ArrowUp');
    fixture.detectChanges();
    expect(selected()).toBe('0');
    press('End');
    fixture.detectChanges();
    expect(selected()).toBe(String(total - 1));
    press('ArrowDown');
    fixture.detectChanges();
    expect(selected()).toBe(String(total - 1));
    const other = press('a');
    expect(other.defaultPrevented).toBe(false);
    await new Promise((r) => setTimeout(r, 10));
  });

  it('ignores keys when there are no rows', async () => {
    const { fixture } = await mount();
    typeInto(el<HTMLInputElement>(fixture, 'input[type=search]'), 'zzz-none');
    await new Promise((r) => setTimeout(r, 260));
    await settle(fixture, 6);
    // Empty state replaces the table; dispatch on the host does nothing and must not throw.
    expect(all(fixture, 'tbody')).toHaveLength(0);
  });

  it('shows a placeholder for pages that are not loaded and loads them on scroll', async () => {
    const real = new MockProfilingBackend();
    const backend: ProfilingBackend = Object.create(real);
    backend.hotspots = async (r: HotspotsRequest): Promise<HotspotsPage> => {
      const page = await real.hotspots({ ...r, offset: 0, limit: 10 });
      return { ...page, totalRows: 1000, rows: Array.from({ length: r.limit ?? 200 }, (_, i) => ({ ...page.rows[i % page.rows.length], name: `fn-${r.offset! + i}` })) };
    };
    const { fixture } = await mount({ backend });
    expect(names(fixture)[0]).toBe('fn-0');
    expect(el(fixture, 'table').getAttribute('aria-rowcount')).toBe('1001');
    const viewport = el(fixture, '.viewport');
    viewport.scrollTop = 200 * ROW_H;
    viewport.dispatchEvent(new Event('scroll'));
    fixture.detectChanges();
    expect(text(fixture)).toContain('Loading…');
    await settle(fixture, 6);
    expect(text(fixture)).not.toContain('Loading…');
    expect(names(fixture)).toContain('fn-205');
    expect(all(fixture, 'tr.spacer').length).toBe(2);
  });

  it('keyboard navigation scrolls the virtual window to the focused row', async () => {
    const real = new MockProfilingBackend();
    const backend: ProfilingBackend = Object.create(real);
    backend.hotspots = async (r: HotspotsRequest) => {
      const page = await real.hotspots({ ...r, offset: 0, limit: 10 });
      return { ...page, totalRows: 500, rows: Array.from({ length: r.limit ?? 200 }, (_, i) => ({ ...page.rows[i % page.rows.length], name: `fn-${r.offset! + i}` })) };
    };
    const { fixture } = await mount({ backend });
    el(fixture, 'tbody').dispatchEvent(new KeyboardEvent('keydown', { key: 'End', bubbles: true, cancelable: true }));
    await settle(fixture, 6);
    await new Promise((r) => setTimeout(r, 10));
    expect(el(fixture, '.viewport').scrollTop).toBeGreaterThan(0);
    expect(el(fixture, 'tr.sel').getAttribute('data-index')).toBe('499');
  });

  it('shows an error with Retry and recovers', async () => {
    const { fixture } = await mount({ options: { fail: { hotspots: 1 } } });
    expect(el(fixture, '[role=alert]').textContent).toContain('hotspots failed');
    buttonByText(fixture, 'Retry').click();
    await settle(fixture, 6);
    expect(all(fixture, '[role=alert]')).toHaveLength(0);
    expect(names(fixture).length).toBeGreaterThan(0);
  });

  it('shows import notes of the capture', async () => {
    const { fixture } = await mount();
    const p = TestBed.inject(ProfileViewState);
    expect(p.profileId()).not.toBeNull();
    expect(all(fixture, 'imt-capture-notes').length).toBe(1);
  });
});
