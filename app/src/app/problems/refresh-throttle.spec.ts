import { DestroyRef, signal } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { FakeDock, provideFakeDock, provideTinyBackend, tinyBackend } from '../../testing/helpers';
import { MockBackend } from '../core/mock/mock-backend';
import { WorkspaceStore } from '../core/workspace.store';
import { COMMAND_CONTEXT } from '../extensions/command-registry';
import { CommandContext } from '../extensions/tokens';
import { MockParsersBackend } from '../settings/parsers/mock-parsers-backend';
import { ParsersBackend } from '../settings/parsers/parsers-backend';
import { ProblemsBackend } from './problems-backend';
import { MockProblemsBackend } from './mock/mock-problems-backend';
import { PROBLEMS_REFRESH_HIDDEN_MS, PROBLEMS_REFRESH_VISIBLE_MS, ProblemsStore } from './problems.store';
import { RefreshThrottle, ThrottleClock } from './refresh-throttle';

/** A clock the test moves by hand. */
class ManualClock implements ThrottleClock {
  t = 0;
  private timers: { at: number; run: () => void; id: number }[] = [];
  private next = 1;
  now(): number {
    return this.t;
  }
  setTimeout(run: () => void, ms: number): unknown {
    const id = this.next++;
    this.timers.push({ at: this.t + ms, run, id });
    return id;
  }
  clearTimeout(handle: unknown): void {
    this.timers = this.timers.filter((t) => t.id !== handle);
  }
  advance(ms: number): void {
    const until = this.t + ms;
    for (;;) {
      const due = this.timers.filter((t) => t.at <= until).sort((a, b) => a.at - b.at)[0];
      if (!due) break;
      this.timers = this.timers.filter((t) => t !== due);
      this.t = due.at;
      due.run();
    }
    this.t = until;
  }
}

describe('RefreshThrottle', () => {
  it('runs the first change at once and coalesces a burst into one run at the end of the interval', () => {
    const clock = new ManualClock();
    const run = vi.fn();
    const t = new RefreshThrottle(run, () => 2000, clock);
    t.request();
    expect(run).toHaveBeenCalledTimes(1);
    // Four changes a second for three seconds: one run per two seconds.
    for (let i = 0; i < 12; i++) {
      clock.advance(250);
      t.request();
    }
    expect(run).toHaveBeenCalledTimes(2);
    expect(t.pending).toBe(true);
    clock.advance(1000);
    expect(run).toHaveBeenCalledTimes(3);
    expect(t.pending).toBe(false);
    // After a quiet interval the next change runs at once again.
    clock.advance(5000);
    t.request();
    expect(run).toHaveBeenCalledTimes(4);
  });

  it('moves a pending run to the new interval when it changes, and runs nothing after dispose', () => {
    const clock = new ManualClock();
    const run = vi.fn();
    let interval = 10_000;
    const t = new RefreshThrottle(run, () => interval, clock);
    t.request();
    clock.advance(1000);
    t.request();
    expect(t.pending).toBe(true);
    interval = 2000;
    t.reschedule();
    clock.advance(1000);
    expect(run).toHaveBeenCalledTimes(2);
    // Nothing pending: rescheduling does nothing.
    t.reschedule();
    expect(run).toHaveBeenCalledTimes(2);
    clock.advance(500);
    t.request();
    t.dispose();
    clock.advance(10_000);
    t.request();
    expect(run).toHaveBeenCalledTimes(2);
  });

  it('uses real timers by default', async () => {
    vi.useFakeTimers();
    try {
      const run = vi.fn();
      const t = new RefreshThrottle(run, () => 100);
      t.request();
      t.request();
      expect(run).toHaveBeenCalledTimes(1);
      await vi.advanceTimersByTimeAsync(100);
      expect(run).toHaveBeenCalledTimes(2);
      t.dispose();
    } finally {
      vi.useRealTimers();
    }
  });
});

describe('ProblemsStore refresh throttle', () => {
  function setup(backend: MockBackend = tinyBackend()) {
    const problems = new MockProblemsBackend(backend);
    TestBed.configureTestingModule({
      providers: [
        ...provideTinyBackend(backend),
        ...provideFakeDock(new FakeDock()),
        { provide: ProblemsBackend, useValue: problems },
        { provide: ParsersBackend, useValue: new MockParsersBackend(backend) },
        { provide: COMMAND_CONTEXT, useValue: { showView: vi.fn(), jumpTo: vi.fn(), notifyError: vi.fn() } as unknown as CommandContext },
      ],
    });
    return { problems };
  }

  it('lists at most once per interval during rapid workspace changes, sooner while the pane is visible', async () => {
    vi.useFakeTimers();
    try {
      const { problems } = setup();
      const list = vi.spyOn(problems, 'problems');
      const workspace = TestBed.inject(WorkspaceStore);
      await workspace.init();
      const store = TestBed.inject(ProblemsStore);
      TestBed.tick();
      await vi.advanceTimersByTimeAsync(0);
      const first = list.mock.calls.length;
      expect(first).toBe(1);

      // Hidden pane: summary changes 4x/s for 4 s list no more than the hidden interval allows.
      for (let i = 0; i < 16; i++) {
        await workspace.refreshSummary();
        TestBed.tick();
        await vi.advanceTimersByTimeAsync(250);
      }
      expect(list.mock.calls.length - first).toBe(0);

      // The pane becomes visible: the pending listing moves to the visible interval.
      const visible = signal(true);
      const destroyed: (() => void)[] = [];
      const destroyRef: Pick<DestroyRef, 'onDestroy'> = {
        onDestroy: (fn: () => void) => {
          destroyed.push(fn);
          return () => undefined;
        },
      };
      store.gate.add(visible, destroyRef);
      TestBed.tick();
      await vi.advanceTimersByTimeAsync(0);
      expect(list.mock.calls.length - first).toBe(1);
      const before = list.mock.calls.length;
      for (let i = 0; i < 16; i++) {
        await workspace.refreshSummary();
        TestBed.tick();
        await vi.advanceTimersByTimeAsync(250);
      }
      // 4 s of changes while visible: about one listing per 2 s.
      const visibleRuns = list.mock.calls.length - before;
      expect(visibleRuns).toBeGreaterThanOrEqual(1);
      expect(visibleRuns).toBeLessThanOrEqual(Math.ceil(4000 / PROBLEMS_REFRESH_VISIBLE_MS));
      destroyed.forEach((fn) => fn());
      await vi.advanceTimersByTimeAsync(PROBLEMS_REFRESH_HIDDEN_MS);
    } finally {
      vi.useRealTimers();
    }
  });
});
