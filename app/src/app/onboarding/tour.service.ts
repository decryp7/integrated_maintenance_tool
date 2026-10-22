import { computed, inject, Injectable, signal } from '@angular/core';
import { DockService } from '../dock/dock.service';
import { TOUR_STEPS, TourStep } from './tour-steps';

export const TOUR_DONE_KEY = 'debrief.tour.done.v1';
/** How long a step waits for its panel to appear before it is skipped. */
export const TARGET_WAIT_MS = 1500;
const TARGET_POLL_MS = 50;

/** Where the spotlight goes (viewport coordinates). */
export interface Spot {
  top: number;
  left: number;
  width: number;
  height: number;
}

/**
 * The guided tour: which step shows, where its spotlight is, and whether the user has finished or skipped
 * it (remembered in the browser, so it starts by itself only once).
 */
@Injectable({ providedIn: 'root' })
export class TourService {
  private readonly dock = inject(DockService);
  readonly steps: readonly TourStep[] = TOUR_STEPS;

  private readonly indexSignal = signal(-1);
  /** Index of the step showing; -1 while the tour is off. */
  readonly index = this.indexSignal.asReadonly();
  readonly active = computed(() => this.indexSignal() >= 0);
  readonly step = computed<TourStep | null>(() => this.steps[this.indexSignal()] ?? null);
  /** The spotlight of the current step; null for a centred step (or while its target is being found). */
  readonly spot = signal<Spot | null>(null);
  readonly isFirst = computed(() => this.indexSignal() === 0);
  readonly isLast = computed(() => this.indexSignal() === this.steps.length - 1);

  private seq = 0;
  private target: Element | null = null;

  /** Finds a step's element; replaceable in tests. */
  query: (selector: string) => Element | null = (selector) => document.querySelector(selector);

  /** True once the tour was completed or skipped. */
  done(): boolean {
    try {
      return localStorage.getItem(TOUR_DONE_KEY) === 'true';
    } catch {
      return false;
    }
  }

  /** Starts the tour from the first step (from Help, always; after the demo, only when not done yet). */
  start(): void {
    void this.goTo(0, 1);
  }

  /** Starts the tour unless the user has already been through it. */
  startOnce(): void {
    if (!this.done() && !this.active()) this.start();
  }

  next(): void {
    if (!this.active()) return;
    if (this.isLast()) this.finish();
    else void this.goTo(this.indexSignal() + 1, 1);
  }

  back(): void {
    if (this.active() && !this.isFirst()) void this.goTo(this.indexSignal() - 1, -1);
  }

  /** Leaves the tour; it does not start by itself again. */
  skip(): void {
    this.finish();
  }

  finish(): void {
    this.seq++;
    this.indexSignal.set(-1);
    this.spot.set(null);
    this.target = null;
    try {
      localStorage.setItem(TOUR_DONE_KEY, 'true');
    } catch {
      /* storage unavailable: the tour may offer itself again next time */
    }
  }

  /** Measures the current target again (window resized, layout changed). */
  remeasure(): void {
    if (this.target) this.spot.set(measure(this.target));
  }

  /** Shows step `i`; a step whose target cannot be found is skipped in the direction of travel. */
  private async goTo(i: number, dir: 1 | -1): Promise<void> {
    const mine = ++this.seq;
    let at = i;
    while (at >= 0 && at < this.steps.length) {
      const step = this.steps[at];
      if (!step) break;
      this.indexSignal.set(at);
      this.spot.set(null);
      this.target = null;
      if (!step.target) return;
      if (step.view) this.dock.show(step.view);
      const el = await this.waitFor(step.target, mine);
      if (mine !== this.seq) return;
      if (el) {
        this.target = el;
        el.scrollIntoView?.({ block: 'nearest' });
        this.spot.set(measure(el));
        return;
      }
      at += dir;
    }
    // Nothing left past the end (the first step needs no target, so going back always stops there).
    this.finish();
  }

  private async waitFor(selector: string, mine: number): Promise<Element | null> {
    const deadline = Date.now() + TARGET_WAIT_MS;
    for (;;) {
      const el = this.query(selector);
      if (el && visible(el)) return el;
      if (mine !== this.seq || Date.now() >= deadline) return null;
      await new Promise((r) => setTimeout(r, TARGET_POLL_MS));
    }
  }
}

function measure(el: Element): Spot {
  const r = el.getBoundingClientRect();
  return { top: r.top, left: r.left, width: r.width, height: r.height };
}

function visible(el: Element): boolean {
  const r = el.getBoundingClientRect();
  return r.width > 0 && r.height > 0;
}
