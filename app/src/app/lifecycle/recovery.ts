import { effect, inject, Injectable, Injector, Provider, untracked } from '@angular/core';
import { toObservable } from '@angular/core/rxjs-interop';
import { filter, firstValueFrom, map } from 'rxjs';
import { Logger } from '../core/logger';
import { StartupBackend } from '../core/startup/startup-backend';
import { AFTER_READY, ReadyReply, Recovery } from '../core/startup/startup-contract';
import { WorkspaceStore } from '../core/workspace.store';
import { POPOUT_ROUTE } from '../popout/popout-route';
import { UiStateRegistry } from './ui-state';

/** Quiet period before a changed UI state is checkpointed in Rust's memory. */
export const CHECKPOINT_MS = 1000;

/** What the window says after Debrief reloaded it (ADR 0058): calm, with what was kept. */
export function recoveryNotice(why: Recovery): string {
  return why === 'terminated'
    ? $localize`:@@lifecycle.recovered.terminated:The window's content stopped unexpectedly and was reloaded. Your open files and view are kept.`
    : $localize`:@@lifecycle.recovered.unresponsive:The window did not finish starting and was reloaded. Your open files and view are kept.`;
}

/**
 * A recovery reload restores the window as it was (ADR 0058): the engine kept the data; the query, filters,
 * selection and the rest of the UI state come from the checkpoint Rust kept in memory, applied once the workspace is
 * loaded. Checkpointing starts only after that, so the reloaded page's empty start never replaces the checkpoint.
 */
@Injectable({ providedIn: 'root' })
export class WindowRecovery {
  private readonly store = inject(WorkspaceStore);
  private readonly registry = inject(UiStateRegistry);
  private readonly backend = inject(StartupBackend);
  private readonly logger = inject(Logger);
  private readonly injector = inject(Injector);
  private checkpointing = false;
  private timer: ReturnType<typeof setTimeout> | undefined;

  /** Handles the ready reply (null: none came). */
  async afterReady(reply: ReadyReply | null): Promise<void> {
    try {
      if (reply?.recovered) {
        this.store.setNotice(recoveryNotice(reply.recovered));
        this.logger.info('ui.startup', 'Window reloaded by Debrief', { Reason: reply.recovered, RestoresUi: reply.ui != null });
        if (reply.ui !== null && typeof reply.ui === 'object') {
          await this.loaded();
          this.registry.restore(reply.ui);
        }
      }
    } finally {
      this.startCheckpoints();
    }
  }

  /** Resolves when the workspace has loaded (or failed to: the state is restored either way). */
  private loaded(): Promise<void> {
    const done = () => this.store.summary() !== null || this.store.error() !== null;
    if (done()) return Promise.resolve();
    return firstValueFrom(toObservable(this.store.summary, { injector: this.injector }).pipe(map(done), filter(Boolean))).then(() => undefined);
  }

  /** Sends the UI state to Rust whenever it changes (debounced). Idempotent. */
  startCheckpoints(): void {
    if (this.checkpointing) return;
    this.checkpointing = true;
    // A side effect outside the signal graph: the state goes to Rust over IPC (rule 28).
    effect(
      () => {
        const snapshot = this.registry.snapshot();
        untracked(() => this.schedule(snapshot));
      },
      { injector: this.injector },
    );
  }

  private schedule(snapshot: string): void {
    clearTimeout(this.timer);
    this.timer = setTimeout(() => {
      this.timer = undefined;
      void this.backend.checkpointUi(JSON.parse(snapshot)).catch(() => undefined);
    }, CHECKPOINT_MS);
  }
}

/** The main window's handling of the ready reply. A pop-out has no UI state of its own: nothing to do there. */
export function provideRecovery(): Provider {
  return {
    provide: AFTER_READY,
    useFactory: () => {
      if (inject(POPOUT_ROUTE)) return () => undefined;
      const recovery = inject(WindowRecovery);
      return (reply: ReadyReply | null) => void recovery.afterReady(reply);
    },
  };
}
