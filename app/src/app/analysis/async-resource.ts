import { computed, signal } from '@angular/core';
import { Analyzed } from '../core/api.types';
import { failureText, isRetryable } from '../core/cmd-error';

export function errorText(e: unknown): string {
  return failureText(e);
}

/**
 * Loading / error / data signals for one request. Latest call wins; `retry()` repeats the last
 * call. Views render it through `<debrief-analysis-state>` (error boundary with Retry).
 */
export class AsyncResource<T> {
  readonly data = signal<T | null>(null);
  readonly loading = signal(false);
  readonly error = signal<string | null>(null);
  /** Whether the last failure may pass on Retry (the engine says so). */
  readonly retryable = signal(true);
  private seq = 0;
  private last: (() => Promise<T>) | null = null;

  async load(fn: () => Promise<T>): Promise<void> {
    this.last = fn;
    const mine = ++this.seq;
    this.loading.set(true);
    this.error.set(null);
    try {
      const value = await fn();
      if (mine === this.seq) this.data.set(value);
    } catch (e) {
      if (mine === this.seq) {
        this.error.set(errorText(e));
        this.retryable.set(isRetryable(e));
      }
    } finally {
      if (mine === this.seq) this.loading.set(false);
    }
  }

  retry(): Promise<void> {
    return this.last ? this.load(this.last) : Promise.resolve();
  }

  /** Forgets data and cancels in-flight results (e.g. when the input is cleared). */
  reset(): void {
    this.seq++;
    this.last = null;
    this.data.set(null);
    this.error.set(null);
    this.loading.set(false);
  }
}

/** An `AsyncResource` of an analysis: its `result` for the view, its `coverage` for `<debrief-coverage-note>`. */
export class AnalysisResource<T> extends AsyncResource<Analyzed<T>> {
  readonly result = computed(() => this.data()?.result ?? null);
  readonly coverage = computed(() => this.data()?.coverage ?? null);
}
