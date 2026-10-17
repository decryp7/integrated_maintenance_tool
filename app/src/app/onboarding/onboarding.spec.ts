import { TestBed } from '@angular/core/testing';
import { buttonByText, FakeDock, provideFakeDock, provideTinyBackend, settle, text } from '../../testing/helpers';
import { ImtBackend } from '../core/backend';
import { WorkspaceStore } from '../core/workspace.store';
import { UxBackend } from '../core/ux-backend';
import { CommandRegistry } from '../extensions/command-registry';
import { Welcome, WelcomeService } from '../welcome/welcome';
import { READ_ONLY_PROMISE } from '../shared/read-only';
import { DEMO_MISSING, DemoService } from './demo.service';
import { provideOnboarding } from './onboarding.plugin';
import { popoverPosition, TourOverlay } from './tour-overlay';
import { TOUR_STEPS } from './tour-steps';
import { TOUR_DONE_KEY, TourService } from './tour.service';

/** A visible element for the tour to find. */
function box(): HTMLElement {
  const el = document.createElement('div');
  el.getBoundingClientRect = () => ({ top: 100, left: 50, width: 400, height: 200, right: 450, bottom: 300, x: 50, y: 100, toJSON: () => null }) as DOMRect;
  return el;
}

function setupTour(found: (selector: string) => Element | null = () => box()) {
  const dock = new FakeDock();
  TestBed.configureTestingModule({ providers: [provideTinyBackend(), provideFakeDock(dock)] });
  const tour = TestBed.inject(TourService);
  tour.query = found;
  return { tour, dock };
}

describe('TourService', () => {
  beforeEach(() => localStorage.removeItem(TOUR_DONE_KEY));
  afterEach(() => vi.useRealTimers());

  it('walks through every step, opening each panel, and remembers it was done', async () => {
    const { tour, dock } = setupTour();
    expect(tour.done()).toBe(false);
    tour.start();
    expect(tour.index()).toBe(0);
    expect(tour.spot()).toBeNull();
    expect(tour.isFirst()).toBe(true);
    for (let i = 1; i < TOUR_STEPS.length; i++) {
      tour.next();
      await settle();
      expect(tour.index()).toBe(i);
      expect(tour.spot()).toEqual({ top: 100, left: 50, width: 400, height: 200 });
    }
    expect(dock.calls).toEqual(TOUR_STEPS.filter((s) => s.view).map((s) => `show:${s.view}`));
    expect(tour.isLast()).toBe(true);
    tour.back();
    await settle();
    expect(tour.index()).toBe(TOUR_STEPS.length - 2);
    tour.remeasure();
    tour.next();
    await settle();
    tour.next();
    expect(tour.active()).toBe(false);
    expect(tour.done()).toBe(true);
    tour.next();
    tour.back();
    tour.remeasure();
    expect(tour.active()).toBe(false);
  });

  it('starts by itself only once, and skipping counts as done', () => {
    const { tour } = setupTour();
    tour.startOnce();
    expect(tour.active()).toBe(true);
    tour.startOnce();
    tour.back();
    expect(tour.index()).toBe(0);
    tour.skip();
    expect(tour.done()).toBe(true);
    tour.startOnce();
    expect(tour.active()).toBe(false);
  });

  it('skips steps whose panel never appears, in either direction', async () => {
    vi.useFakeTimers();
    const { tour } = setupTour((s) => (s === 'imt-timeline-panel' || s === 'imt-overview-view' ? null : box()));
    tour.start();
    tour.next();
    await vi.advanceTimersByTimeAsync(2000);
    expect(tour.step()?.id).toBe('events');
    tour.back();
    await vi.advanceTimersByTimeAsync(2000);
    expect(tour.index()).toBe(0);
    // The last step is missing: moving past it finishes the tour.
    tour.start();
    for (let i = 0; i < 4; i++) {
      tour.next();
      await vi.advanceTimersByTimeAsync(2000);
    }
    expect(tour.step()?.id).toBe('markers');
    tour.next();
    await vi.advanceTimersByTimeAsync(2000);
    expect(tour.active()).toBe(false);
  });

  it('ignores invisible targets and stale lookups, and survives storage failures', async () => {
    vi.useFakeTimers();
    const hidden = document.createElement('div');
    const { tour } = setupTour(() => hidden);
    tour.start();
    tour.next();
    await vi.advanceTimersByTimeAsync(100);
    tour.skip();
    await vi.advanceTimersByTimeAsync(2000);
    expect(tour.active()).toBe(false);
    const get = vi.spyOn(Storage.prototype, 'getItem').mockImplementation(() => {
      throw new Error('denied');
    });
    const set = vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => {
      throw new Error('denied');
    });
    expect(tour.done()).toBe(false);
    tour.finish();
    get.mockRestore();
    set.mockRestore();
  });
});

describe('TourOverlay', () => {
  beforeEach(() => localStorage.removeItem(TOUR_DONE_KEY));

  it('places the popover below, above, inside or centred', () => {
    expect(popoverPosition(null, 1000, 800)).toEqual({ top: 300, left: 330 });
    expect(popoverPosition({ top: 100, left: 0, width: 100, height: 100 }, 1000, 800)).toEqual({ top: 212, left: 12 });
    expect(popoverPosition({ top: 500, left: 900, width: 100, height: 200 }, 1000, 800)).toEqual({ top: 288, left: 648 });
    expect(popoverPosition({ top: 50, left: 100, width: 400, height: 700 }, 1000, 800)).toEqual({ top: 62, left: 130 });
  });

  it('shows the step, is driven by the keyboard and gives focus back', async () => {
    setupTour();
    const tour = TestBed.inject(TourService);
    const fixture = TestBed.createComponent(TourOverlay);
    const before = document.createElement('button');
    document.body.appendChild(before);
    before.focus();
    tour.start();
    await settle(fixture);
    expect(text(fixture)).toContain('Step 1 of 7');
    expect((fixture.nativeElement.querySelector('[role=dialog]') as HTMLElement).getAttribute('aria-labelledby')).toBe('imt-tour-title');
    expect(document.activeElement?.textContent).toBe('Next');
    const key = (k: string, target: EventTarget = document.body) => {
      const e = new KeyboardEvent('keydown', { key: k, bubbles: true });
      target.dispatchEvent(e);
    };
    key('ArrowRight');
    await settle(fixture);
    expect(tour.index()).toBe(1);
    expect(fixture.nativeElement.querySelector('.spot')).not.toBeNull();
    key('Enter');
    await settle(fixture);
    expect(tour.index()).toBe(2);
    key('Enter', document.activeElement!); // on a button the click does the work
    key('ArrowLeft');
    await settle(fixture);
    expect(tour.index()).toBe(1);
    key('x');
    buttonByText(fixture, 'Back').click();
    await settle(fixture);
    expect(tour.index()).toBe(0);
    key('Escape');
    await settle(fixture);
    expect(tour.active()).toBe(false);
    expect(text(fixture)).toBe('');
    expect(document.activeElement).toBe(before);
    key('Escape');
    before.remove();
  });

  it('finds targets in the document, follows resizes and can be skipped by button', async () => {
    TestBed.configureTestingModule({ providers: [provideTinyBackend(), provideFakeDock()] });
    const tour = TestBed.inject(TourService);
    const target = box();
    target.className = 'tour-target-test';
    document.body.appendChild(target);
    expect(tour.query('.tour-target-test')).toBe(target);
    const fixture = TestBed.createComponent(TourOverlay);
    tour.start();
    await settle(fixture);
    window.dispatchEvent(new Event('resize'));
    buttonByText(fixture, 'Skip tour').click();
    await settle(fixture);
    expect(tour.done()).toBe(true);
    target.remove();
  });

  it('offers Finish on the last step', async () => {
    setupTour();
    const tour = TestBed.inject(TourService);
    const fixture = TestBed.createComponent(TourOverlay);
    tour.start();
    for (let i = 1; i < TOUR_STEPS.length; i++) {
      tour.next();
      await settle(fixture);
    }
    buttonByText(fixture, 'Finish').click();
    await settle(fixture);
    expect(tour.done()).toBe(true);
  });
});

describe('DemoService, commands and the welcome button', () => {
  beforeEach(() => localStorage.removeItem(TOUR_DONE_KEY));

  function setup(paths: string[] | Error = ['/demo/samples/serilog-clef']) {
    const ux = { demoPaths: vi.fn(async () => (paths instanceof Error ? Promise.reject(paths) : paths)) };
    TestBed.configureTestingModule({ providers: [provideTinyBackend(), provideFakeDock(), provideOnboarding(), { provide: UxBackend, useValue: ux }] });
    const tour = TestBed.inject(TourService);
    tour.query = () => box();
    return { demo: TestBed.inject(DemoService), store: TestBed.inject(WorkspaceStore), tour, ux };
  }

  it('opens the demo through the normal open path and starts the tour once', async () => {
    const { demo, store, tour } = setup();
    await settle();
    const open = vi.spyOn(TestBed.inject(ImtBackend), 'openPaths');
    TestBed.inject(WelcomeService).show();
    expect(await demo.start()).toBe(true);
    expect(open).toHaveBeenCalledWith(['/demo/samples/serilog-clef']);
    expect(TestBed.inject(WelcomeService).visible()).toBe(false);
    expect(tour.active()).toBe(true);
    tour.skip();
    expect(await demo.start()).toBe(true);
    expect(tour.active()).toBe(false);
    expect(store.importVersion()).toBe(2);
  });

  it('explains when the samples are missing, the open fails or the lookup fails', async () => {
    const { demo, store } = setup([]);
    await settle();
    expect(await demo.start()).toBe(false);
    expect(store.error()).toBe(DEMO_MISSING);
    TestBed.resetTestingModule();
    const failing = setup(new Error('no resources'));
    await settle();
    expect(await failing.demo.start()).toBe(false);
    expect(failing.store.error()).toBe('Could not open the demo: no resources');
    TestBed.resetTestingModule();
    const bad = setup();
    await settle();
    vi.spyOn(TestBed.inject(ImtBackend), 'openPaths').mockRejectedValue('disk gone');
    expect(await bad.demo.start()).toBe(false);
    expect(bad.tour.active()).toBe(false);
    TestBed.resetTestingModule();
    const odd = setup();
    vi.spyOn(odd.store, 'openPaths').mockRejectedValue('weird');
    expect(await odd.demo.start()).toBe(false);
    expect(odd.store.error()).toBe('Could not open the demo: weird');
  });

  it('ignores a second start while loading', async () => {
    const { demo } = setup();
    await settle();
    const first = demo.start();
    expect(await demo.start()).toBe(false);
    expect(await first).toBe(true);
  });

  it('registers Try the demo and Take the tour', async () => {
    const { tour } = setup();
    const registry = TestBed.inject(CommandRegistry);
    await settle();
    expect(await registry.run('help.tour')).toBe(true);
    expect(tour.active()).toBe(true);
    tour.skip();
    expect(await registry.run('help.demo')).toBe(true);
    const store = TestBed.inject(WorkspaceStore);
    store.applySummary({ ...store.summary()!, sources: [] });
    expect(await registry.run('help.tour')).toBe(false);
  });

  it('the welcome screen offers the demo and the read-only promise', async () => {
    const { ux } = setup();
    const fixture = TestBed.createComponent(Welcome);
    await settle(fixture);
    expect(text(fixture)).toContain(READ_ONLY_PROMISE);
    buttonByText(fixture, 'Try a demo').click();
    await settle(fixture);
    expect(ux.demoPaths).toHaveBeenCalled();
  });
});
