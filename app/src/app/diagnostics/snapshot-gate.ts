/** Limits for automatic snapshots: at most one per `minIntervalMs`, and `maxPerSession` in total. */
export interface GateLimits {
  minIntervalMs: number;
  maxPerSession: number;
}

export const DEFAULT_GATE_LIMITS: GateLimits = { minIntervalMs: 10_000, maxPerSession: 20 };

/**
 * Decides whether another snapshot may be taken. Pure state machine over an injected clock.
 * Manual captures bypass both limits (the user asked) but still count.
 */
export class SnapshotGate {
  private last = Number.NEGATIVE_INFINITY;
  private taken = 0;

  constructor(private readonly limits: GateLimits = DEFAULT_GATE_LIMITS) {}

  /** Records a capture and returns true when allowed. */
  tryAcquire(now: number, manual = false): boolean {
    if (!manual && (this.taken >= this.limits.maxPerSession || now - this.last < this.limits.minIntervalMs)) return false;
    this.last = now;
    this.taken++;
    return true;
  }

  get count(): number {
    return this.taken;
  }
}
