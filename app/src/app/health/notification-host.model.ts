import { InjectionToken, Signal } from '@angular/core';
import { isHealthCause } from './service-health.types';

export type HostSeverity = 'info' | 'success' | 'warning' | 'error' | 'critical';

/** A button on a notification. */
export interface HostAction {
  label: string;
  run: () => void | Promise<void>;
  kind?: 'primary' | 'default' | 'ghost';
  testId?: string;
}

/**
 * One notification as the host shows it. Every notification has a `cause`: the same cause is one notification, updated in
 * place. Features adapt their own state to this shape (see `NOTIFICATION_SOURCES`); the host knows nothing of them.
 */
export interface HostNotification {
  readonly cause: string;
  readonly severity: HostSeverity;
  readonly title: string;
  /** A short sentence under the title. */
  readonly body?: string | null;
  /** Technical text, collapsed behind "Details". */
  readonly details?: string | null;
  /** Tells one occurrence of the cause from the next (Details start folded again for each). Defaults to the cause. */
  readonly instance?: string | number;
  /** How many times the cause happened. */
  readonly count?: number;
  readonly actions?: readonly HostAction[];
  /** Overrides `data-testid` of the toast (stable ids tests rely on). */
  readonly testId?: string;
  /** The user dismissed it (the source forgets it). */
  readonly dismiss: () => void;
  /** The pointer or focus is on it / left it (a source that times out pauses its timer). */
  readonly hold?: () => void;
  readonly release?: () => void;
}

/** A feature's notifications, as a signal the host reads. The source owns lifetimes: errors stay until resolved. */
export interface NotificationSource {
  readonly items: Signal<readonly HostNotification[]>;
}

export const NOTIFICATION_SOURCES = new InjectionToken<readonly NotificationSource[]>('NOTIFICATION_SOURCES');

/** The most toasts at once; older ones wait (hidden) until a newer one goes. */
export const MAX_VISIBLE = 3;

/**
 * The one display rule as a pure function: one toast per cause (the later source's entry replaces an earlier one with the
 * same cause, keeping the later position), a cause the status chip already states is never toasted, and at most `max`
 * (the newest) are shown.
 */
export function visibleNotifications(items: readonly HostNotification[], max = MAX_VISIBLE): HostNotification[] {
  const byCause = new Map<string, HostNotification>();
  for (const n of items) {
    if (isHealthCause(n.cause)) continue;
    byCause.delete(n.cause);
    byCause.set(n.cause, n);
  }
  return [...byCause.values()].slice(-max);
}

/** Errors and critical failures stay until resolved or dismissed; the rest time out (in the source). */
export function persists(severity: HostSeverity): boolean {
  return severity === 'error' || severity === 'critical';
}
