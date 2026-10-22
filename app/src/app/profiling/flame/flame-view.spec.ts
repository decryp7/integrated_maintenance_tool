import { ComponentFixture, TestBed } from '@angular/core/testing';
import { By } from '@angular/platform-browser';
import { afterEach, beforeEach, vi } from 'vitest';
import { all, buttonByText, el, settle, text, typeInto } from '../../../testing/helpers';
import { FlameRequest } from '../../core/api.types';
import { ProfileViewState } from '../profile-view-state';
import { openDemoCaptures, setupProfiling } from '../testing/profiling-test-support';
import { MockProfilingOptions } from '../../core/mock/mock-profiling';
import { FlameChart } from './flame-chart';
import { FlameView } from './flame-view';

async function mount(options: MockProfilingOptions = {}, open = true, truncate = false) {
  const doubles = setupProfiling({ options });
  const real = doubles.backend.flame.bind(doubles.backend);
  const flame = vi.spyOn(doubles.backend, 'flame');
  if (truncate) flame.mockImplementation(async (r) => ({ ...(await real(r)), truncated: true }));
  if (open) await openDemoCaptures();
  const fixture: ComponentFixture<FlameView> = TestBed.createComponent(FlameView);
  fixture.detectChanges();
  await settle(fixture, 6);
  const state = TestBed.inject(ProfileViewState);
  const lastRequest = (): FlameRequest => flame.mock.calls.at(-1)![0];
  return { fixture, state, flame, lastRequest, backend: doubles.backend };
}

const wait = (ms: number) => new Promise((r) => setTimeout(r, ms));
const select = (f: ComponentFixture<unknown>, label: string) => el<HTMLSelectElement>(f, `select[aria-label="${label}"]`);
const chart = (f: ComponentFixture<unknown>) => f.debugElement.query(By.directive(FlameChart)).componentInstance as FlameChart;

describe('FlameView', () => {
  let width: ReturnType<typeof vi.spyOn>;
  beforeEach(() => {
    vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockReturnValue(null);
    // jsdom does no layout: the view is 1024 px wide unless a test says otherwise.
    width = vi.spyOn(HTMLElement.prototype, 'clientWidth', 'get').mockReturnValue(1024);
  });
  afterEach(() => vi.restoreAllMocks());

  it('asks to open a profile when none is open', async () => {
    const { fixture, flame } = await mount({}, false);
    expect(text(fixture)).toContain('Open a performance capture to see its flame graph.');
    expect(flame).not.toHaveBeenCalled();
  });

  it('requests the active profile with the default layout and renders the chart', async () => {
    const { fixture, lastRequest, state } = await mount();
    const r = lastRequest();
    expect(r).toMatchObject({ profileId: state.profileId(), direction: 'topDown', focus: [], order: 'name', search: null, searchRegex: false, metric: 'timeNs' });
    expect(r.view).toEqual(state.view());
    expect(r.widthPx).toBe(1000); // the view's width less the chart's padding
    expect(el(fixture, '.surface').getAttribute('aria-label')).toContain('Flame graph of orders.speedscope.json');
    expect(text(fixture)).toContain('Managed');
  });

  it('switches between flame and icicle', async () => {
    const { fixture } = await mount();
    const flame = buttonByText(fixture, 'Flame');
    const icicle = buttonByText(fixture, 'Icicle');
    expect([flame.getAttribute('aria-pressed'), icicle.getAttribute('aria-pressed')]).toEqual(['true', 'false']);
    icicle.click();
    fixture.detectChanges();
    expect([flame.getAttribute('aria-pressed'), icicle.getAttribute('aria-pressed')]).toEqual(['false', 'true']);
    expect(chart(fixture).orientation()).toBe('icicle');
    flame.click();
    fixture.detectChanges();
    expect(chart(fixture).orientation()).toBe('flame');
  });

  it('re-requests with the other stack direction and resets the zoom', async () => {
    const { fixture, lastRequest, state } = await mount();
    state.setFlameFocus([0, 1]);
    await settle(fixture);
    expect(lastRequest().focus).toEqual([0, 1]);
    const s = select(fixture, 'Stack direction');
    s.value = 'bottomUp';
    s.dispatchEvent(new Event('change'));
    await settle(fixture);
    expect(lastRequest().direction).toBe('bottomUp');
    expect(state.flameFocus()).toEqual([]);
    expect(lastRequest().focus).toEqual([]);
  });

  it('changes the colouring and the order', async () => {
    const { fixture, lastRequest } = await mount();
    const by = select(fixture, 'Colour by');
    by.value = 'module';
    by.dispatchEvent(new Event('change'));
    await settle(fixture);
    expect(chart(fixture).colorBy()).toBe('module');
    expect(text(fixture)).toContain('Other modules');
    const order = select(fixture, 'Order');
    order.value = 'value';
    order.dispatchEvent(new Event('change'));
    await settle(fixture);
    expect(lastRequest().order).toBe('value');
  });

  it('debounces the search into the shared state and shows the matched share', async () => {
    const { fixture, state, lastRequest } = await mount();
    expect(text(fixture)).not.toContain('Matched');
    const input = el<HTMLInputElement>(fixture, 'input[aria-label="Search frames"]');
    typeInto(input, 'Ord');
    typeInto(input, 'Order');
    expect(state.flameSearch()).toBe('');
    await wait(260);
    await settle(fixture);
    expect(state.flameSearch()).toBe('Order');
    expect(lastRequest().search).toBe('Order');
    expect(el(fixture, '[data-testid=flame-matched]').textContent).toMatch(/Matched \d+\.\d% \(/);
  });

  it('sends the regex flag', async () => {
    const { fixture, lastRequest } = await mount();
    const box = all<HTMLInputElement>(fixture, 'label.inline input[type=checkbox]').find((c) => c.parentElement?.textContent?.includes('Regex'))!;
    box.checked = true;
    box.dispatchEvent(new Event('change'));
    await settle(fixture);
    expect(lastRequest().searchRegex).toBe(true);
  });

  it('shows breadcrumbs for the zoom path, jumps to a crumb and resets', async () => {
    const { fixture, state } = await mount();
    expect(el(fixture, 'nav.crumbs .current').textContent).toBe('All');
    expect(all(fixture, 'nav.crumbs button')).toHaveLength(0);
    state.setFlameFocus([0, 1, 2]);
    await settle(fixture);
    const names = all(fixture, 'nav.crumbs .crumb').map((x) => x.textContent!.trim());
    expect(names).toEqual(['All', 'Shop.Orders.Program.Main', 'System.Threading.ThreadPoolWorkQueue.Dispatch', 'Shop.Orders.Api.OrderController.Post']);
    expect(el(fixture, 'nav.crumbs .current').textContent).toContain('OrderController.Post');
    buttonByText(fixture, 'Shop.Orders.Program.Main').click();
    expect(state.flameFocus()).toEqual([0]);
    await settle(fixture);
    buttonByText(fixture, 'Reset zoom').click();
    expect(state.flameFocus()).toEqual([]);
    await settle(fixture);
    expect(f(fixture, 'Reset zoom')).toBe(false);
  });

  it('zooms from the chart, continuing the current focus, and zooms out', async () => {
    const { fixture, state } = await mount();
    chart(fixture).zoom.emit([0, 1]);
    expect(state.flameFocus()).toEqual([0, 1]);
    chart(fixture).zoom.emit([1, 2, 3]); // depth 0 of the new graph is the focused node
    expect(state.flameFocus()).toEqual([0, 1, 2, 3]);
    chart(fixture).zoomOut.emit();
    expect(state.flameFocus()).toEqual([0, 1, 2]);
  });

  it('records the selected frame', async () => {
    const { fixture, state } = await mount();
    el(fixture, '.surface').dispatchEvent(new KeyboardEvent('keydown', { key: 'Home', bubbles: true }));
    expect(state.selectedFrame()?.name).toBe('Shop.Orders.Program.Main');
  });

  it('tells when the layout was truncated', async () => {
    const { fixture } = await mount({}, true, true);
    expect(el(fixture, '.notice').textContent).toContain('More frames than the drawing limit');
    expect(el(fixture, '.surface').getAttribute('aria-label')).toContain('truncated');
  });

  it('waits for its measured width before the first request, then asks once', async () => {
    const observers: ResizeObserverCallback[] = [];
    vi.stubGlobal(
      'ResizeObserver',
      class {
        constructor(cb: ResizeObserverCallback) {
          observers.push(cb);
        }
        observe() {}
        disconnect() {}
      },
    );
    try {
      width.mockReturnValue(0);
      const { fixture, flame, lastRequest } = await mount();
      expect(flame).not.toHaveBeenCalled();
      width.mockReturnValue(824);
      for (const cb of observers) cb([], {} as ResizeObserver);
      await settle(fixture, 6);
      expect(flame).toHaveBeenCalledTimes(1);
      expect(lastRequest().widthPx).toBe(800);
      // A resize asks again for the new width; an unchanged width does not.
      for (const cb of observers) cb([], {} as ResizeObserver);
      await settle(fixture, 6);
      expect(flame).toHaveBeenCalledTimes(1);
      width.mockReturnValue(624);
      for (const cb of observers) cb([], {} as ResizeObserver);
      await settle(fixture, 6);
      expect(flame).toHaveBeenCalledTimes(2);
      expect(lastRequest().widthPx).toBe(600);
    } finally {
      vi.unstubAllGlobals();
    }
  });

  it('names the zoom breadcrumbs from the engine answer', async () => {
    const { fixture, state, flame } = await mount();
    state.setFlameFocus([0, 1]);
    await settle(fixture, 6);
    const g = await flame.mock.results.at(-1)!.value;
    expect(g.focusFrames.map((x: { id: number }) => x.id)).toEqual([0, 1]);
    expect(all(fixture, 'nav.crumbs .crumb').map((x) => x.textContent!.trim())).toEqual(['All', ...g.focusFrames.map((x: { name: string }) => x.name)]);
  });

  it('shows the error with Retry and recovers', async () => {
    const { fixture } = await mount({ fail: { flame: 1 } }); // one request: the view waits for its width
    expect(el(fixture, '[role=alert]').textContent).toContain('mock: flame failed');
    expect(f(fixture, '.surface')).toBe(false);
    buttonByText(fixture, 'Retry').click();
    await settle(fixture, 6);
    expect(f(fixture, '[role=alert]')).toBe(false);
    expect(f(fixture, '.surface')).toBe(true);
  });

  it('says so when no samples match', async () => {
    const { fixture, flame } = await mount();
    flame.mockResolvedValue({ rects: [], frames: [], focusFrames: [], focusValue: 0, grandTotal: 0, matchedValue: 0, matchedPct: 0, truncated: false, maxDepth: 0, metric: 'timeNs', measurement: 'sampled' });
    TestBed.inject(ProfileViewState).setFlameSearch('zzz');
    await settle(fixture, 6);
    expect(text(fixture)).toContain('No samples match this view.');
    expect(text(fixture)).toContain('Matched 0.0% (');
  });
});

function f(fixture: ComponentFixture<unknown>, selectorOrText: string): boolean {
  const root = fixture.nativeElement as HTMLElement;
  if (/^[.[#]/.test(selectorOrText)) return !!root.querySelector(selectorOrText);
  return [...root.querySelectorAll('button')].some((b) => b.textContent?.trim() === selectorOrText);
}
