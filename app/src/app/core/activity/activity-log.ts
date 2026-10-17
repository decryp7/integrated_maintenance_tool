import { inject, Injectable } from '@angular/core';
import { Logger } from '../logger';
import { Detail } from './privacy';
import { activity, ActivityRuntime } from './runtime';
import { ACTIVITY_CONTEXT, ActivityParams, Breadcrumb, Outcome, Trigger } from './types';

/**
 * The activity log, as services and components see it: one entry per user operation, logged once at the
 * point of intent as `User {Operation} via {Trigger}` with an `OperationId`, the window, the active view,
 * the duration, the outcome (ok, cancelled, error with `ErrorCode`) and safe parameters only (see
 * `privacy.ts`). Most operations need no call here: commands (`CommandRegistry`), backend changes (the shared
 * backend proxy), dock, query, time-range and selection changes (`ActivityObservers`) are tracked centrally.
 */
@Injectable({ providedIn: 'root' })
export class ActivityLog {
  private readonly logger = inject(Logger);
  /** This window's runtime (shared with the logger and the backend proxy). */
  readonly runtime: ActivityRuntime = activity;

  constructor() {
    this.runtime.setSink((level, message, props) => this.logger[level](ACTIVITY_CONTEXT, message, props));
  }

  /** Runs `fn` as the operation `operation`; logs one entry when it settles. Rethrows its error. */
  track<T>(operation: string, trigger: Trigger | 'auto', params: ActivityParams, fn: () => T | Promise<T>): Promise<T> {
    return this.runtime.track(operation, trigger, params, fn);
  }

  /** Logs an instant operation; returns its id. */
  record(operation: string, trigger: Trigger | 'auto', params: ActivityParams = {}, outcome: Outcome = 'ok'): string {
    return this.runtime.record(operation, trigger, params, outcome);
  }

  /** Merges a burst of changes into one entry (see `ActivityRuntime.coalesce`). */
  coalesce(key: string, operation: string, trigger: Trigger | 'auto', params: ActivityParams, delayMs: number): string {
    return this.runtime.coalesce(key, operation, trigger, params, delayMs);
  }

  /** This window's latest entries, oldest first. */
  breadcrumbs(n = 30): Breadcrumb[] {
    return this.runtime.breadcrumbs(n);
  }

  currentOperationId(): string | undefined {
    return this.runtime.currentOperationId();
  }

  get detail(): Detail {
    return this.runtime.detail;
  }

  setDetail(detail: Detail): void {
    this.runtime.detail = detail;
  }
}
