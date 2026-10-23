import { inject, Injectable, signal } from '@angular/core';
import { PerformanceBackend } from '../settings/performance/performance-backend';
import { CmdError } from './cmd-error';

/** What the engine said about a log store that is not running (see `engine/store_failure.rs`). */
export interface StoreDown {
  /** One plain sentence. */
  message: string;
  /** The next step for this cause. */
  hint: string | null;
  /** The technical chain, for Details and bug reports. */
  detail: string | null;
}

/** The engine's plain message for a store that cannot run starts with this (`StoreFailure::message`). */
const PREFIX = "Debrief's log store";

/** Whether `e` is the engine's "the log store is not running" error. */
export function isStoreDown(e: unknown): e is CmdError {
  return e instanceof CmdError && e.code === 'unavailable' && e.message.startsWith(PREFIX);
}

type Listener = { failed: (e: CmdError) => void; ok: () => void };
let listener: Listener | null = null;

/** Commands that only succeed with a running store: their success means the store is back. */
const STORE_COMMANDS = new Set(['workspace_summary', 'query_page', 'query_timeline']);

/** Called by `invoke` for every command that succeeded. */
export function reportCommandSuccess(cmd: string): void {
  if (STORE_COMMANDS.has(cmd)) listener?.ok();
}

/** Called by `invoke` for every rejected command; the window's `StoreHealth` listens. */
export function reportCommandFailure(e: unknown): void {
  if (isStoreDown(e)) listener?.failed(e);
}

/**
 * One persistent fact for the window: the log store is not running, and why. Panels show one calm empty state from it,
 * the status bar shows it once with the details, and the repeated toast and banner for each failing command are
 * suppressed. It clears when a store command succeeds again or the user retries successfully.
 */
@Injectable({ providedIn: 'root' })
export class StoreHealth {
  /** The store's failure now, or null while it runs. */
  readonly down = signal<StoreDown | null>(null);

  constructor() {
    listener = {
      failed: (e) => this.down.set({ message: e.message, hint: e.hint ?? null, detail: e.detail ?? null }),
      ok: () => this.recovered(),
    };
  }

  private readonly performance = inject(PerformanceBackend);

  /**
   * "Try again": lets the engine start the store worker again (lifting a quarantine after repeated failures), then the
   * caller retries its own request. If the store is still down, the next failing command reports it again.
   */
  async tryAgain(): Promise<void> {
    await this.performance.retryStoreWorker();
    this.recovered();
  }

  /** The store answered: it is running. */
  recovered(): void {
    if (this.down()) this.down.set(null);
  }
}
