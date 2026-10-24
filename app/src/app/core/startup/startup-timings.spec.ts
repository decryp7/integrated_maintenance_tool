import { ChangeDetectionStrategy, Component } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { describe, expect, it, vi } from 'vitest';
import { settle } from '../../../testing/helpers';
import { provideToolsTesting } from '../../tools/mock/test-support';
import { WorkspaceDataStore } from '../workspace/workspace-data.store';
import { WorkspaceStatusStore } from '../workspace/status.store';
import { StartupBackend } from './startup-backend';
import { BROWSER_CLOCK, PageClock, PageMark, provideStartupTimings } from './startup-timings';

@Component({ selector: 'debrief-timings-host', template: '<p>frame</p>', changeDetection: ChangeDetectionStrategy.OnPush })
class Host {}

class FakeStartup {
  readonly marks: { mark: PageMark; at: number }[] = [];
  reportReady = vi.fn(async () => ({ recovered: null }));
  checkpointUi = vi.fn(async () => undefined);
  reportMark = vi.fn(async (mark: PageMark, at: number) => void this.marks.push({ mark, at }));
  startupTimings = vi.fn(async () => ({ marks: [], steps: [], complete: false }));
}

async function start(clock: PageClock) {
  const startup = new FakeStartup();
  TestBed.configureTestingModule({
    providers: [...provideToolsTesting(), { provide: StartupBackend, useValue: startup }, provideStartupTimings(clock)],
  });
  const fixture = TestBed.createComponent(Host);
  await fixture.whenStable();
  return { startup, fixture };
}

describe('provideStartupTimings', () => {
  it('reports the first paint, the first workspace load and the first frame after it, once each', async () => {
    let now = 1000;
    const { startup, fixture } = await start({ now: () => now++, firstPaint: () => 500 });
    await vi.waitFor(() => expect(startup.marks.map((m) => m.mark).sort()).toEqual(['firstInteractive', 'firstPaint', 'workspaceLoaded']));
    // Each is sent with its own time (Rust orders them by it): the paint's from the browser, the frame after the load later.
    const at = (m: PageMark) => startup.marks.find((x) => x.mark === m)!.at;
    expect(at('firstPaint')).toBe(500);
    expect(at('firstInteractive')).toBeGreaterThan(at('workspaceLoaded'));
    // Later loads and renders send nothing more.
    await TestBed.inject(WorkspaceDataStore).reload();
    fixture.detectChanges();
    await settle(fixture);
    expect(startup.reportMark).toHaveBeenCalledTimes(3);
  });

  it('uses the render time when the browser has no paint timing, and counts a failed first load as loaded', async () => {
    const { startup } = await start({ now: () => 42, firstPaint: () => null });
    TestBed.inject(WorkspaceStatusStore).setError('the engine is unavailable');
    await vi.waitFor(() => expect(startup.marks.map((m) => m.mark)).toContain('firstInteractive'));
    expect(startup.marks.every((m) => m.at === 42)).toBe(true);
  });

  it('ignores a failed report (the timings never break the start)', async () => {
    const startup = new FakeStartup();
    startup.reportMark.mockRejectedValue(new Error('no'));
    TestBed.configureTestingModule({ providers: [...provideToolsTesting(), { provide: StartupBackend, useValue: startup }, provideStartupTimings({ now: () => 1, firstPaint: () => null })] });
    const fixture = TestBed.createComponent(Host);
    await fixture.whenStable();
    await vi.waitFor(() => expect(startup.reportMark).toHaveBeenCalledTimes(3));
    expect(fixture.nativeElement.textContent).toContain('frame');
  });

  it('reads the browser clock: first contentful paint from the page origin', () => {
    expect(BROWSER_CLOCK.now()).toBeGreaterThan(0);
    const fcp = BROWSER_CLOCK.firstPaint();
    expect(fcp === null || fcp >= performance.timeOrigin).toBe(true);
  });
});
