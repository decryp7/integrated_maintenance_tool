import { ComponentFixture, TestBed } from '@angular/core/testing';
import { describe, expect, it } from 'vitest';
import { all, buttonByText, el, settle, text } from '../../../testing/helpers';
import { MockProfilingBackend } from '../../core/mock/mock-profiling';
import { ProfilingBackend } from '../profiling-backend';
import { FakeContext } from '../../analysis/test-setup';
import { CapturesStore } from '../captures.store';
import { ProfileViewState } from '../profile-view-state';
import { openDemoCaptures, provideProfilingTesting, setupProfiling } from '../testing/profiling-test-support';
import { MOCK_CAPTURE_PATHS } from '../../core/mock/mock-profiling-data';
import { IMT_COMMANDS, ImtCommand } from '../../extensions/tokens';
import { RecentService } from '../../core/recent/recent.service';
import { MOCK_RECENT_KEY } from '../../core/recent/mock-recent-backend';
import { CapturesView } from './captures-view';

async function mount(opts: Parameters<typeof setupProfiling>[0] = {}, open = true) {
  const doubles = setupProfiling(opts);
  if (open) await openDemoCaptures();
  const fixture: ComponentFixture<CapturesView> = TestBed.createComponent(CapturesView);
  fixture.detectChanges();
  await settle(fixture, 4);
  return { fixture, doubles, ctx: doubles.context as FakeContext, state: TestBed.inject(ProfileViewState), store: TestBed.inject(CapturesStore) };
}

const cap = (f: ComponentFixture<unknown>, name: string) => all(f, 'li.cap').find((l) => l.querySelector('.name')!.textContent === name)!;

describe('CapturesView', () => {
  beforeEach(() => localStorage.removeItem(MOCK_RECENT_KEY));

  it('shows the empty state', async () => {
    const { fixture } = await mount({}, false);
    expect(text(fixture)).toContain('No profiles or heap snapshots are open.');
    expect(all(fixture, 'li.cap')).toHaveLength(0);
  });

  it('the landing lists recent captures, opens one on click and clears the list', async () => {
    const { fixture } = await mount({}, false);
    expect(el(fixture, '[data-testid=captures-landing]')).toBeTruthy();
    expect(fixture.nativeElement.querySelector('[data-testid=recent-captures]')).toBeNull();
    const recent = TestBed.inject(RecentService);
    await recent.opened([], undefined, [MOCK_CAPTURE_PATHS[0]!, '/demo/profiles/other.folded']);
    fixture.detectChanges();
    const items = all(fixture, '[data-testid=recent-captures] li button');
    expect(items.map((b) => b.getAttribute('title'))).toEqual([MOCK_CAPTURE_PATHS[0], '/demo/profiles/other.folded']);
    expect(items[1]!.textContent).toBe('other.folded');
    items[0]!.click();
    await settle(fixture, 6);
    expect(TestBed.inject(CapturesStore).captures().length).toBe(1);
    TestBed.inject(CapturesStore).closeAll();
    await settle(fixture, 4);
    buttonByText(fixture, 'Clear recent captures').click();
    await settle(fixture, 4);
    expect(fixture.nativeElement.querySelector('[data-testid=recent-captures]')).toBeNull();
    expect(recent.captures()).toEqual([]);
  });

  it('shows Loading… while the first listing is in flight', async () => {
    const { fixture, store } = await mount({ options: { latencyMs: 40 } }, false);
    const pending = store.refresh();
    fixture.detectChanges();
    expect(text(fixture)).toContain('Loading…');
    expect(buttonByText(fixture, 'Open performance capture…').disabled).toBe(true);
    await pending;
    await settle(fixture, 3);
    expect(text(fixture)).not.toContain('Loading…');
  });

  it('lists profiles with kind, source, duration, threads, samples and metrics', async () => {
    const { fixture } = await mount();
    expect(all(fixture, 'li.cap')).toHaveLength(5);
    const orders = cap(fixture, 'orders.speedscope.json');
    expect(orders.textContent).toContain('/demo/profiles/orders.speedscope.json');
    for (const label of ['Source', 'Duration', 'Threads', 'Samples', 'Metrics']) expect(orders.textContent).toContain(label);
    expect(orders.textContent).toMatch(/Samples\s*[\d,]+ · [\d,]+ frames · [\d,]+ events/);
    expect(orders.textContent).toMatch(/\((sampled|exact)\)/);
    expect(orders.querySelector('.tag')!.textContent!.length).toBeGreaterThan(0);
    expect(orders.querySelector('dd')!.getAttribute('title')).toContain('orders.speedscope.json');
  });

  it('lists heap snapshots with their facts', async () => {
    const { fixture } = await mount();
    const heap = cap(fixture, 'heap-1.heapsnapshot');
    for (const label of ['Taken', 'Objects', 'Graph']) expect(heap.textContent).toContain(label);
    expect(heap.textContent).toMatch(/types ·/);
    expect(heap.textContent).toContain('retained sizes');
    expect(heap.textContent).not.toContain('Threads');
  });

  it('shows import notes of a capture', async () => {
    const { fixture } = await mount();
    const note = el(fixture, '[role=note]');
    expect(note.getAttribute('aria-label')).toMatch(/^Import notes for /);
    expect(note.textContent).toContain('Import notes');
    expect(all(fixture, '[role=note] li').length).toBeGreaterThan(0);
  });

  it('marks the active profile and heap with aria-current', async () => {
    const { fixture, state } = await mount();
    const current = all(fixture, 'li.cap[aria-current=true]');
    expect(current).toHaveLength(2);
    expect(cap(fixture, 'orders.speedscope.json').getAttribute('aria-current')).toBe('true');
    expect(cap(fixture, 'heap-1.heapsnapshot').textContent).toContain('active');
    expect(cap(fixture, 'orders-regression.speedscope.json').getAttribute('aria-current')).toBeNull();
    state.setHeap(state.heapId()! + 1);
    fixture.detectChanges();
    expect(cap(fixture, 'heap-1.heapsnapshot').getAttribute('aria-current')).toBeNull();
  });

  it('activating a profile makes it active and shows the hotspots; a heap shows the heap view', async () => {
    const { fixture, ctx, state } = await mount();
    (cap(fixture, 'orders-regression.speedscope.json').querySelector('.name') as HTMLButtonElement).click();
    expect(state.profileId()).toBe(2);
    expect(ctx.last('showView')).toEqual(['profiling-hotspots']);
    (cap(fixture, 'heap-2.heapsnapshot').querySelector('.name') as HTMLButtonElement).click();
    expect(state.heapId()).toBe(4);
    expect(ctx.last('showView')).toEqual(['profiling-heap']);
  });

  it('closes a capture', async () => {
    const { fixture, state } = await mount();
    el<HTMLButtonElement>(fixture, 'button[aria-label="Close orders.speedscope.json"]').click();
    await settle(fixture, 4);
    expect(all(fixture, 'li.cap')).toHaveLength(4);
    expect(state.profileId()).toBe(2);
    expect(cap(fixture, 'orders-regression.speedscope.json').getAttribute('aria-current')).toBe('true');
  });

  it('Open performance capture… opens the picked captures, announces them and shows the hotspots', async () => {
    const { fixture, doubles, ctx } = await mount({}, false);
    doubles.picker.paths = ['/demo/profiles/orders.speedscope.json'];
    buttonByText(fixture, 'Open performance capture…').click();
    await settle(fixture, 6);
    expect(all(fixture, 'li.cap')).toHaveLength(1);
    expect(el(fixture, '.notice').textContent).toContain('Opened 1 capture.');
    expect(ctx.last('showView')).toEqual(['profiling-hotspots']);
  });

  it('Open performance capture… with only a heap snapshot shows the heap view; with nothing shows no view', async () => {
    const { fixture, doubles, ctx } = await mount({}, false);
    doubles.picker.paths = ['/demo/profiles/heap-1.heapsnapshot'];
    buttonByText(fixture, 'Open performance capture…').click();
    await settle(fixture, 6);
    expect(ctx.last('showView')).toEqual(['profiling-heap']);
    ctx.calls.length = 0;
    doubles.picker.paths = [];
    buttonByText(fixture, 'Open performance capture…').click();
    await settle(fixture, 6);
    expect(ctx.calls.filter((c) => c.fn === 'showView')).toHaveLength(0);
  });

  it('shows open failures with Retry and Dismiss', async () => {
    const { fixture, doubles } = await mount({}, false);
    doubles.picker.paths = ['/tmp/readme.txt'];
    buttonByText(fixture, 'Open performance capture…').click();
    await settle(fixture, 6);
    expect(el(fixture, '[role=alert] .msg').textContent).toContain('readme.txt is not a profile or heap snapshot');
    buttonByText(fixture, 'Retry').click();
    await settle(fixture, 6);
    expect(el(fixture, '[role=alert] .msg').textContent).toContain('readme.txt');
    buttonByText(fixture, 'Dismiss').click();
    await settle(fixture, 3);
    expect(all(fixture, '[role=alert]')).toHaveLength(0);
  });

  it('shows a failing listing and recovers with Retry', async () => {
    const real = new MockProfilingBackend();
    let failures = 1;
    const backend: ProfilingBackend = Object.create(real);
    backend.list = () => (failures-- > 0 ? Promise.reject(new Error('engine unreachable')) : real.list());
    const { fixture, store } = await mount({ backend }, false);
    await store.refresh();
    await settle(fixture, 3);
    expect(el(fixture, '[role=alert] .msg').textContent).toContain('engine unreachable');
    buttonByText(fixture, 'Retry').click();
    await settle(fixture, 4);
    expect(all(fixture, '[role=alert]')).toHaveLength(0);
    expect(text(fixture)).toContain('No profiles or heap snapshots are open.');
  });

  describe('landing', () => {
    const landing = (f: ComponentFixture<unknown>) => all(f, '[data-testid=captures-landing]');

    it('is the empty state: Open capture… and a hint, no profile-a-process buttons in a build without them', async () => {
      const { fixture } = await mount({}, false);
      expect(landing(fixture)).toHaveLength(1);
      expect(text(fixture)).toContain('No profiles or heap snapshots are open.');
      expect(text(fixture)).toContain('open once a capture is open');
      expect(all(fixture, '[data-testid=captures-landing] button').map((b) => b.textContent!.trim())).toEqual(['Open capture…']);
    });

    it('Open capture… opens the picker and the landing gives way to the list', async () => {
      const { fixture, doubles, store } = await mount({}, false);
      doubles.picker.paths = [MOCK_CAPTURE_PATHS[0]!];
      buttonByText(fixture, 'Open capture…').click();
      await settle(fixture, 6);
      expect(store.captures()).toHaveLength(1);
      expect(landing(fixture)).toHaveLength(0);
      expect(all(fixture, 'li.cap')).toHaveLength(1);
    });

    it('offers the Profile .NET / native process commands when the build has them, and runs the chosen one', async () => {
      const ran: string[] = [];
      const { providers } = provideProfilingTesting();
      const commands: ImtCommand[] = [
        { id: 'profiling.dotnet', title: 'Profile .NET process…', run: () => void ran.push('dotnet') },
        { id: 'profiling.native', title: 'Profile native process…', run: () => void ran.push('native') },
        { id: 'profiling.other', title: 'Not offered', run: () => void ran.push('other') },
        { id: 'profiling.off', title: 'Off', when: () => false, run: () => void ran.push('off') },
      ];
      TestBed.configureTestingModule({ providers: [...providers, { provide: IMT_COMMANDS, multi: true, useValue: commands }] });
      const fixture = TestBed.createComponent(CapturesView);
      fixture.detectChanges();
      await settle(fixture, 4);
      expect(all(fixture, '[data-testid=captures-landing] button').map((b) => b.textContent!.trim())).toEqual(['Open capture…', 'Profile .NET process…', 'Profile native process…']);
      buttonByText(fixture, 'Profile native process…').click();
      await settle(fixture, 2);
      buttonByText(fixture, 'Profile .NET process…').click();
      await settle(fixture, 2);
      expect(ran).toEqual(['native', 'dotnet']);
    });

    it('disables Open capture… while loading and is replaced by Loading… until the first listing arrives', async () => {
      const { fixture, store } = await mount({ options: { latencyMs: 40 } }, false);
      const pending = store.refresh();
      fixture.detectChanges();
      expect(landing(fixture)).toHaveLength(0);
      await pending;
      await settle(fixture, 3);
      expect(landing(fixture)).toHaveLength(1);
      expect(buttonByText(fixture, 'Open capture…').disabled).toBe(false);
    });
  });
});
