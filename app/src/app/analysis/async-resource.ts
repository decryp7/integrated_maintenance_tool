import { signal } from '@angular/core';

export function errorText(e: unknown): string {
  if (typeof e === 'string') return e;
  if (e instanceof Error) return e.message;
  return JSON.stringify(e);
}

/**
 * Loading / error / data signals for one request. Latest call wins; `retry()` repeats the last
 * call. Views render it through `<imt-analysis-state>` (error boundary with Retry).
 */
export class AsyncResource<T> {
  readonly data = signal<T | null>(null);
  readonly loading = signal(false);
  readonly error = signal<string | null>(null);
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
      if (mine === this.seq) this.error.set(errorText(e));
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
