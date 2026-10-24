import { afterNextRender, EnvironmentProviders, inject, Injector, makeEnvironmentProviders, provideAppInitializer, InjectionToken } from '@angular/core';
import { isTauri } from '../platform';
import { stopBootErrorForwarding } from './boot-errors';

/**
 * The start-up contract (ADR 0057), the UI half:
 * - nothing awaits IPC before the first paint (initializers start work and return);
 * - start-up calls the window needs get a short per-attempt timeout and are retried (`retryUntilAnswered`), because
 *   the first IPC of a fresh webview can go unanswered and the content process can be paused for seconds;
 * - once the frame is painted the window reports `window_ready` to Rust, whose watchdog reloads a window that never
 *   does and then asks the user (Reload / Safe mode / Keep waiting).
 */

/** Why Debrief reloaded this window before this page loaded (the `window_ready` reply). */
export type Recovery = 'unresponsive' | 'terminated';

/** The reply to `window_ready`. */
export interface ReadyReply {
  recovered: Recovery | null;
}

export interface RetryOptions {
  /** How long one attempt may take before another is sent (ms). */
  timeoutMs: number;
  /** Attempts in all; after the last one the call waits for whichever attempt answers first, without a deadline. */
  attempts: number;
  /** Timer (tests). */
  setTimer?: (fn: () => void, ms: number) => unknown;
}

/**
 * Calls `call`, and calls it again whenever an attempt has not answered within `timeoutMs`, up to `attempts` calls.
 * The first answer of any attempt wins (an earlier attempt that answers late still counts); a rejection is an answer
 * too (the backend replied: retrying would not help) and rejects. There is no overall deadline
 * (a running start-up is never given up on: the user sees "Still working"); only idempotent reads may use this.
 */
export function retryUntilAnswered<T>(call: () => Promise<T>, options: RetryOptions): Promise<T> {
  const setTimer = options.setTimer ?? ((fn: () => void, ms: number) => setTimeout(fn, ms));
  return new Promise<T>((resolve, reject) => {
    let settled = false;
    let sent = 0;
    const attempt = () => {
      if (settled) return;
      sent++;
      let p: Promise<T>;
      try {
        p = call();
      } catch (e) {
        p = Promise.reject(e);
      }
      p.then(
        (v) => {
          if (settled) return;
          settled = true;
          resolve(v);
        },
        (e: unknown) => {
          if (settled) return;
          settled = true;
          reject(e);
        },
      );
      if (sent < options.attempts) setTimer(attempt, options.timeoutMs);
    };
    attempt();
  });
}

/** Per-attempt timeout and attempts of the start-up calls. */
export const STARTUP_RETRY: RetryOptions = { timeoutMs: 4000, attempts: 4 };

/** Sends `window_ready` (retried like every start-up call). */
export type ReadySender = () => Promise<ReadyReply>;

/** What the window does when Debrief tells it that it was reloaded (main window: a calm notice). */
export const ON_RECOVERED = new InjectionToken<(why: Recovery) => void>('ON_RECOVERED');

/** Sends `window_ready` through the one IPC entry point (loaded on use: the browser build has no IPC). */
export const tauriReady: ReadySender = () => import('../activity/ipc').then((m) => m.invoke<ReadyReply>('window_ready'));

/**
 * Reports ready once the first frame is painted (render-driven, not timer-driven: a paused page reports when it
 * resumes) and hands a recovery reason to `onRecovered`. Never throws: without an answer the Rust watchdog acts.
 */
export async function reportReady(send: ReadySender, onRecovered: ((why: Recovery) => void) | null, retry: RetryOptions = STARTUP_RETRY): Promise<ReadyReply | null> {
  stopBootErrorForwarding();
  try {
    const reply = await retryUntilAnswered(send, retry);
    if (reply?.recovered && onRecovered) onRecovered(reply.recovered);
    return reply;
  } catch {
    return null;
  }
}

/** The start-up contract for a window: report ready after the first render. One line in each window's config. */
export function provideStartupContract(send: ReadySender | null = isTauri() ? tauriReady : null): EnvironmentProviders {
  return makeEnvironmentProviders([
    provideAppInitializer(() => {
      const injector = inject(Injector);
      if (!send) {
        stopBootErrorForwarding();
        return;
      }
      afterNextRender(() => void reportReady(send, injector.get(ON_RECOVERED, null)), { injector });
    }),
  ]);
}
