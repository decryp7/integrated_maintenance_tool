import { effect, signal, Signal, untracked } from '@angular/core';
import { TransferMeter } from './units';

/** What a progress display needs from a transfer. */
export interface TransferSpeed {
  /** Smoothed bytes per second; 0 while unknown. */
  rate: number;
  /** Seconds left; null while unknown. */
  etaSecs: number | null;
}

const UNKNOWN: TransferSpeed = { rate: 0, etaSecs: null };

/**
 * Smoothed speed and ETA of whatever `progress` reports (cumulative bytes and the total, or null while nothing
 * transfers). Samples are timestamped on arrival, so it works for any backend. Call in an injection context.
 */
export function transferSpeed(progress: () => { bytes: number; total: number } | null, now: () => number = Date.now): Signal<TransferSpeed> {
  const meter = new TransferMeter();
  const out = signal<TransferSpeed>(UNKNOWN);
  // effect-state: samples progress over time into a speed (needs wall-clock history, not derivable)
  effect(() => {
    const p = progress();
    untracked(() => {
      if (!p) {
        meter.reset();
        out.set(UNKNOWN);
        return;
      }
      meter.sample(p.bytes, now());
      out.set({ rate: meter.rate, etaSecs: meter.etaSecs(p.total) });
    });
  });
  return out.asReadonly();
}
