import { afterNextRender, EnvironmentProviders, inject, Injector, makeEnvironmentProviders, provideAppInitializer, InjectionToken } from '@angular/core';
import { StartupBackend } from './startup-backend';
import { stopBootErrorForwarding } from './boot-errors';

/**
 * The start-up contract (ADR 0058), the UI half:
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
  /** With `recovered`: the window's last UI state checkpoint, to apply once the workspace is loaded. */
  ui?: unknown;
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

/**
 * What the window does with the ready reply (or null when none came): the main window shows a calm notice after a
 * recovery reload, restores the UI state checkpoint and then starts checkpointing. Optional per window.
 */
export const AFTER_READY = new InjectionToken<(reply: ReadyReply | null) => void>('AFTER_READY');

/**
 * Reports ready once the first frame is painted (render-driven, not timer-driven: a paused page reports when it
 * resumes) and hands the reply (null when there was none) to `after`. Never throws: without an answer the Rust
 * watchdog acts.
 */
export async function reportReady(send: ReadySender, after: ((reply: ReadyReply | null) => void) | null, retry: RetryOptions = STARTUP_RETRY): Promise<ReadyReply | null> {
  stopBootErrorForwarding();
  let reply: ReadyReply | null;
  try {
    reply = await retryUntilAnswered(send, retry);
  } catch {
    reply = null;
  }
  after?.(reply);
  return reply;
}

/** The start-up contract for a window: report ready after the first render. One line in each window's config. */
export function provideStartupContract(send?: ReadySender): EnvironmentProviders {
  return makeEnvironmentProviders([
    provideAppInitializer(() => {
      const injector = inject(Injector);
      const sender = send ?? (() => injector.get(StartupBackend).reportReady());
      afterNextRender(() => void reportReady(sender, injector.get(AFTER_READY, null)), { injector });
    }),
  ]);
}
