/**
 * Uncaught errors and unhandled rejections before a window is ready go to the Rust log (`window_boot_error`), so a
 * window that never starts leaves its cause in the log. Installed by `main.ts` before anything else loads; stopped
 * when the window reports ready (Angular's error handler and the Logger take over from there). Only the error's
 * message and its first stack frame are sent, bounded: never file contents or page text.
 */

/** Sends one boot error (kind: `error`, `rejection` or `bootstrap`). */
export type BootErrorSender = (kind: 'error' | 'rejection' | 'bootstrap', message: string) => void;

/** At most this many errors per page load are sent (a loop must not flood the log). */
export const MAX_BOOT_ERRORS = 10;
const MAX_LEN = 400;

let stop: (() => void) | null = null;
let send: BootErrorSender | null = null;
let sent = 0;

/** One bounded line: the message and the first stack frame. */
export function describeError(reason: unknown): string {
  if (reason instanceof Error) {
    const frame = (reason.stack ?? '').split('\n').map((l) => l.trim()).find((l) => l && !l.startsWith(reason.name) && l !== reason.message) ?? '';
    return `${reason.name}: ${reason.message}${frame ? ` @ ${frame}` : ''}`.slice(0, MAX_LEN);
  }
  if (typeof reason === 'string') return reason.slice(0, MAX_LEN);
  return Object.prototype.toString.call(reason);
}

function forward(kind: 'error' | 'rejection' | 'bootstrap', reason: unknown): void {
  if (!send || sent >= MAX_BOOT_ERRORS) return;
  sent++;
  try {
    send(kind, describeError(reason));
  } catch {
    // Reporting must never throw from an error handler.
  }
}

/** Starts forwarding (idempotent). `target` is the window (tests pass a fake). */
export function startBootErrorForwarding(sender: BootErrorSender, target: Pick<Window, 'addEventListener' | 'removeEventListener'> = window): void {
  if (stop) return;
  send = sender;
  sent = 0;
  const onError = (e: Event) => forward('error', (e as ErrorEvent).error ?? (e as ErrorEvent).message);
  const onRejection = (e: Event) => forward('rejection', (e as PromiseRejectionEvent).reason);
  target.addEventListener('error', onError);
  target.addEventListener('unhandledrejection', onRejection);
  stop = () => {
    target.removeEventListener('error', onError);
    target.removeEventListener('unhandledrejection', onRejection);
  };
}

/** Reports a failed bootstrap (the app never started). */
export function reportBootstrapFailure(reason: unknown): void {
  forward('bootstrap', reason);
}

/** Stops forwarding (the window is ready). Idempotent. */
export function stopBootErrorForwarding(): void {
  stop?.();
  stop = null;
}

/** Sends through the one IPC entry point, loaded on use (it is not needed unless something fails). */
export const tauriBootErrorSender: BootErrorSender = (kind, message) => {
  void import('../activity/ipc').then((m) => m.invoke('window_boot_error', { kind, message })).catch(() => undefined);
};
