import { ComponentFixture, TestBed } from '@angular/core/testing';
import { By } from '@angular/platform-browser';
import { afterEach, beforeEach, vi } from 'vitest';
import { all, buttonByText, el, settle, text, typeInto } from '../../../testing/helpers';
import { FunctionDiffPage } from '../../core/api.types';
import { CapturesStore } from '../captures.store';
import { ProfileViewState } from '../profile-view-state';
import { openDemoCaptures, setupProfiling } from '../testing/profiling-test-support';
import { FlameChart } from '../flame/flame-chart';
import { MockProfilingBackend } from '../../core/mock/mock-profiling';
import { CompareView } from './compare-view';

async function mount(opts: { open?: string[]; baseline?: boolean; target?: number } = {}) {
  const doubles = setupProfiling();
  const backend = doubles.backend as MockProfilingBackend;
  const diffFlame = vi.spyOn(backend, 'diffFlame');
  const functionDiff = vi.spyOn(backend, 'functionDiff');
  if (opts.open) await TestBed.inject(CapturesStore).openPaths(opts.open);
  else await openDemoCaptures();
  const state = TestBed.inject(ProfileViewState);
  if (opts.target !== undefined) state.setProfile(opts.target);
  if (opts.baseline) state.setBaseline(1);
  const fixture: ComponentFixture<CompareView> = TestBed.createComponent(CompareView);
  fixture.detectChanges();
  await settle(fixture, 6);
  return { fixture, state, backend, diffFlame, functionDiff };
}

const wait = (ms: number) => new Promise((r) => setTimeout(r, ms));
const chart = (f: ComponentFixture<unknown>) => f.debugElement.query(By.directive(FlameChart)).componentInstance as FlameChart;
const rowsText = (f: ComponentFixture<unknown>) => all(f, 'section.table tbody tr').map((r) => r.textContent!.replace(/\s+/g, ' ').trim());

describe('CompareView', () => {
  beforeEach(() => {
    vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockReturnValue(null);
    // jsdom does no layout: give the view a width so it asks the engine for a layout.
    vi.spyOn(HTMLElement.prototype, 'clientWidth', 'get').mockReturnValue(1024);
  });
  afterEach(() => vi.restoreAllMocks());

  it('asks to open profiles when none is open', async () => {
    const doubles = setupProfiling();
    void doubles;
    const fixture = TestBed.createComponent(CompareView);
    fixture.detectChanges();
    await settle(fixture);
    expect(text(fixture)).toContain('Open two profiles to compare them.');
  });

  it('asks for a second profile when there is nothing to compare with', async () => {
    const { fixture, functionDiff } = await mount({ open: ['/demo/profiles/orders.speedscope.json'] });
    expect(text(fixture)).toContain('Open a second profile to use as the baseline.');
    expect(functionDiff).not.toHaveBeenCalled();
  });

  it('asks to choose a baseline when another profile is open', async () => {
    const { fixture, diffFlame } = await mount();
    expect(text(fixture)).toContain('Choose a baseline profile to compare with.');
    const options = all<HTMLOptionElement>(fixture, 'select[aria-label="Baseline profile"] option').map((o) => o.textContent);
    expect(options).toEqual(['Choose a baseline…', 'orders-regression.speedscope.json']);
    expect(diffFlame).not.toHaveBeenCalled();
  });

  it('loads the differential flame graph and the function changes when a baseline is picked', async () => {
    const { fixture, state, diffFlame, functionDiff } = await mount();
    const select = el<HTMLSelectElement>(fixture, 'select[aria-label="Baseline profile"]');
    select.value = '2';
    select.dispatchEvent(new Event('change'));
    await settle(fixture, 6);
    expect(state.baselineId()).toBe(2);
    expect(diffFlame.mock.calls.at(-1)![0]).toMatchObject({ baselineId: 2, targetId: 1, view: state.view(), metric: 'timeNs', focus: [], widthPx: 1000 });
    expect(functionDiff.mock.calls.at(-1)![0]).toMatchObject({ baselineId: 2, targetId: 1, view: state.view(), metric: 'timeNs', sort: 'absDeltaSelf', search: null, offset: 0, limit: 200 });
    expect(el(fixture, '.surface').getAttribute('aria-label')).toContain('Differential flame graph of orders.speedscope.json');
    expect(text(fixture)).toContain('less than the baseline');
    expect(text(fixture)).toMatch(/\d+ functions · baseline .* · target /);
    expect(el(fixture, '.muted strong').textContent).toBe('orders.speedscope.json');
  });

  it('flags new functions and arrows the changes when the regression is the target', async () => {
    const { fixture } = await mount({ target: 2, baseline: true });
    const rows = rowsText(fixture);
    const audit = rows.find((r) => r.includes('AuditLog'))!;
    expect(audit).toContain('new');
    expect(el(fixture, 'section.table tbody tr .tag.new').textContent).toBe('new');
    expect(rows.some((r) => r.includes('▲'))).toBe(true);
    expect(all(fixture, 'section.table td.up').length).toBeGreaterThan(0);
    expect(all(fixture, 'section.table td.down').length + all(fixture, 'section.table td.up').length).toBeGreaterThan(0);
    expect(all(fixture, 'section.table th').map((h) => h.textContent)).toContain('Δ self');
  });

  it('flags removed functions when the regression is the baseline', async () => {
    const { fixture, state } = await mount();
    state.setBaseline(2);
    await settle(fixture, 6);
    expect(rowsText(fixture).find((r) => r.includes('AuditLog'))).toContain('removed');
    expect(all(fixture, 'section.table td.down').length).toBeGreaterThan(0);
    expect(rowsText(fixture).some((r) => r.includes('▼'))).toBe(true);
  });

  it('re-requests with the chosen sort', async () => {
    const { fixture, functionDiff } = await mount({ target: 2, baseline: true });
    const select = el<HTMLSelectElement>(fixture, 'select[aria-label="Sort changes"]');
    expect(all<HTMLOptionElement>(fixture, 'select[aria-label="Sort changes"] option')).toHaveLength(5);
    select.value = 'name';
    select.dispatchEvent(new Event('change'));
    await settle(fixture, 6);
    expect(functionDiff.mock.calls.at(-1)![0].sort).toBe('name');
    const names = rowsText(fixture).map((r) => r.split(' ')[0] ?? '');
    expect(names).toEqual([...names].sort((a, b) => a.localeCompare(b)));
  });

  it('debounces the search, trims it and reports when nothing matches', async () => {
    const { fixture, functionDiff } = await mount({ target: 2, baseline: true });
    const calls = functionDiff.mock.calls.length;
    const input = el<HTMLInputElement>(fixture, 'input[aria-label="Search changes"]');
    typeInto(input, '  Aud');
    typeInto(input, '  Audit  ');
    expect(functionDiff.mock.calls.length).toBe(calls);
    await wait(260);
    await settle(fixture, 6);
    expect(functionDiff.mock.calls.at(-1)![0].search).toBe('Audit');
    expect(rowsText(fixture).every((r) => r.includes('Audit'))).toBe(true);
    typeInto(input, 'zzzz-no-such-function');
    await wait(260);
    await settle(fixture, 6);
    expect(text(fixture)).toContain('No functions match.');
  });

  describe('paging', () => {
    function paged(backend: MockProfilingBackend, more: number) {
      const real = backend.functionDiff.bind(backend);
      return vi.spyOn(backend, 'functionDiff').mockImplementation(async (r) => {
        const page = await real({ ...r, offset: 0, limit: 200 });
        const rows = r.offset ? page.rows.slice(0, more).map((x, i) => ({ ...x, key: `extra-${i}`, name: `Extra${i}` })) : page.rows;
        return { ...page, rows, totalRows: page.rows.length + more } satisfies FunctionDiffPage;
      });
    }

    it('loads the remaining rows', async () => {
      const doubles = setupProfiling();
      const backend = doubles.backend as MockProfilingBackend;
      const spy = paged(backend, 3);
      await openDemoCaptures();
      const state = TestBed.inject(ProfileViewState);
      state.setProfile(2);
      state.setBaseline(1);
      const fixture = TestBed.createComponent(CompareView);
      fixture.detectChanges();
      await settle(fixture, 6);
      const before = rowsText(fixture).length;
      buttonByText(fixture, 'Load 3 more').click();
      await settle(fixture, 4);
      expect(spy.mock.calls.at(-1)![0]).toMatchObject({ offset: before, limit: 200 });
      expect(rowsText(fixture)).toHaveLength(before + 3);
      expect(text(fixture)).toContain('Extra0');
      expect(all(fixture, 'button.more')).toHaveLength(0);
    });

    it('keeps the button when loading more fails', async () => {
      const doubles = setupProfiling();
      const backend = doubles.backend as MockProfilingBackend;
      const spy = paged(backend, 2);
      await openDemoCaptures();
      const state = TestBed.inject(ProfileViewState);
      state.setProfile(2);
      state.setBaseline(1);
      const fixture = TestBed.createComponent(CompareView);
      fixture.detectChanges();
      await settle(fixture, 6);
      const before = rowsText(fixture).length;
      spy.mockRejectedValueOnce('boom');
      buttonByText(fixture, 'Load 2 more').click();
      await settle(fixture, 4);
      expect(rowsText(fixture)).toHaveLength(before);
      expect(buttonByText(fixture, 'Load 2 more')).toBeTruthy();
      buttonByText(fixture, 'Load 2 more').click();
      await settle(fixture, 4);
      expect(rowsText(fixture)).toHaveLength(before + 2);
    });
  });

  it('zooms the differential chart with breadcrumbs, and resets from a crumb', async () => {
    const { fixture, diffFlame } = await mount({ target: 2, baseline: true });
    expect(el(fixture, '.crumbs .current').textContent).toBe('All');
    chart(fixture).zoom.emit([0, 1]);
    await settle(fixture, 6);
    expect(diffFlame.mock.calls.at(-1)![0].focus).toEqual([0, 1]);
    const crumbs = all(fixture, '.crumbs button, .crumbs .current').map((c) => c.textContent!.trim());
    expect(crumbs).toHaveLength(3);
    expect(crumbs[1]).toBe('Shop.Orders.Program.Main');
    chart(fixture).zoom.emit([1, 2]);
    await settle(fixture, 6);
    expect(diffFlame.mock.calls.at(-1)![0].focus).toEqual([0, 1, 2]);
    chart(fixture).zoomOut.emit();
    await settle(fixture, 6);
    expect(diffFlame.mock.calls.at(-1)![0].focus).toEqual([0, 1]);
    buttonByText(fixture, 'All').click();
    await settle(fixture, 6);
    expect(diffFlame.mock.calls.at(-1)![0].focus).toEqual([]);
  });

  it('selects the frame the chart moves to', async () => {
    const { fixture, state } = await mount({ target: 2, baseline: true });
    el(fixture, '.surface').dispatchEvent(new KeyboardEvent('keydown', { key: 'Home', bubbles: true }));
    expect(state.selectedFrame()).not.toBeNull();
  });

  it('starts unzoomed when the baseline changes', async () => {
    const { fixture, state } = await mount({ target: 2, baseline: true });
    chart(fixture).zoom.emit([0, 1]);
    await settle(fixture, 6);
    state.setBaseline(1); // same pair: keeps the zoom
    state.setProfile(1);
    state.setBaseline(2);
    await settle(fixture, 6);
    expect(all(fixture, '.crumbs button')).toHaveLength(0);
  });

  it('resets when the baseline is cleared', async () => {
    const { fixture, state } = await mount({ target: 2, baseline: true });
    expect(all(fixture, 'section.table tbody tr').length).toBeGreaterThan(0);
    const select = el<HTMLSelectElement>(fixture, 'select[aria-label="Baseline profile"]');
    select.value = '';
    select.dispatchEvent(new Event('change'));
    await settle(fixture, 6);
    expect(state.baselineId()).toBeNull();
    expect(text(fixture)).toContain('Choose a baseline profile to compare with.');
    expect(all(fixture, 'section.table tbody tr')).toHaveLength(0);
  });

  it('shows errors of the chart and the table with Retry', async () => {
    const doubles = setupProfiling({ options: { fail: { functionDiff: 1, diffFlame: 1 } } });
    void doubles;
    await openDemoCaptures();
    const state = TestBed.inject(ProfileViewState);
    state.setProfile(2);
    state.setBaseline(1);
    const fixture = TestBed.createComponent(CompareView);
    fixture.detectChanges();
    await settle(fixture, 6);
    const alerts = all(fixture, '[role=alert]');
    expect(alerts.map((a) => a.textContent)).toEqual([expect.stringContaining('diffFlame failed'), expect.stringContaining('functionDiff failed')]);
    for (const retry of all<HTMLButtonElement>(fixture, '[role=alert] button')) retry.click();
    await settle(fixture, 6);
    expect(all(fixture, '[role=alert]')).toHaveLength(0);
    expect(all(fixture, 'section.table tbody tr').length).toBeGreaterThan(0);
    expect(all(fixture, '.surface')).toHaveLength(1);
  });
});
