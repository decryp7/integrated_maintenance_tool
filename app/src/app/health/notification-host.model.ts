import { InjectionToken, Signal } from '@angular/core';
import { isHealthCause } from './service-health.types';

export type HostSeverity = 'info' | 'success' | 'warning' | 'error' | 'critical';

/** A button on a notification. */
export interface HostAction {
  label: string;
  run: () => void | Promise<void>;
  kind?: 'primary' | 'default' | 'ghost';
  /** Names the action (`notification-action-<id>` is its test id). */
  id?: string;
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
  /** Marks one occurrence of the cause (its latest time): a new stamp restarts the toast's time. */
  readonly stamp?: string | number;
  /** How long a toast that times out stays, ms (default `TOAST_MS`). */
  readonly ttl?: number;
  /** Stays until dismissed or resolved, whatever its severity (an offer, a result the user should read). */
  readonly sticky?: boolean;
  /** The user dismissed it, or its time ran out (the source forgets it). Sources only post, update and resolve. */
  readonly dismiss: () => void;
}

/** A feature's notifications, as a signal the host reads. The source owns lifetimes: errors stay until resolved. */
export interface NotificationSource {
  readonly items: Signal<readonly HostNotification[]>;
}

export const NOTIFICATION_SOURCES = new InjectionToken<readonly NotificationSource[]>('NOTIFICATION_SOURCES');

/** How long a toast that times out stays (the message stays in Notifications). */
export const TOAST_MS = 8000;

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

/** Errors, critical failures and sticky notifications stay until resolved or dismissed; the rest time out. */
export function persists(n: Pick<HostNotification, 'severity' | 'sticky'>): boolean {
  return !!n.sticky || n.severity === 'error' || n.severity === 'critical';
}
