import { computed, DestroyRef, effect, inject, Injectable, signal, untracked } from '@angular/core';
import { ImportProgress } from '../core/api.types';
import { WorkspaceStore } from '../core/workspace.store';
import { WorkspaceStatusStore } from '../core/workspace/status.store';
import { UxBackend } from '../core/ux-backend';

/** How often progress is read while an import runs. */
export const POLL_MS = 250;
/** Imports running longer than this also show the progress toast. */
export const LONG_IMPORT_MS = 1500;

/** What the last cancelled import read. */
export interface PartialImport {
  done: number;
  total: number;
}

/**
 * Follows the engine's import progress while the workspace imports files (its busy state is an import: open,
 * re-import, profile apply): polls `import_progress`, offers Cancel, and remembers a cancelled import so a
 * "partially imported" banner can say what was read. The import job's progress events carry a fraction and file
 * counts but not the current file or the bytes the toast shows, so the poll stays until the job reports them.
 */
@Injectable({ providedIn: 'root' })
export class ImportProgressService {
  private readonly store = inject(WorkspaceStore);
  private readonly status = inject(WorkspaceStatusStore);
  private readonly backend = inject(UxBackend);

  /** Latest progress of the running import; null when none runs. */
  readonly progress = signal<ImportProgress | null>(null);
  /** The running import has taken longer than `LONG_IMPORT_MS` (the toast shows). */
  readonly long = signal(false);
  readonly cancelling = signal(false);
  /** The last import was cancelled; null once dismissed or after a complete import. */
  readonly partial = signal<PartialImport | null>(null);
  readonly active = computed(() => this.progress() !== null);

  private timer: ReturnType<typeof setInterval> | undefined;
  private longTimer: ReturnType<typeof setTimeout> | undefined;
  private tracking = false;

  constructor() {
    effect(() => {
      const importing = this.status.importing();
      untracked(() => (importing ? this.start() : this.stop()));
    });
    inject(DestroyRef).onDestroy(() => this.clearTimers());
  }

  /** Asks the engine to stop after the files in flight. */
  async cancel(): Promise<void> {
    if (!this.tracking || this.cancelling()) return;
    this.cancelling.set(true);
    try {
      await this.backend.cancelImport();
    } catch (e) {
      this.cancelling.set(false);
      const reason = e instanceof Error ? e.message : String(e);
      this.store.setError($localize`:@@import.progress.cancelFailed:Could not cancel the import: ${reason}:error:`);
    }
  }

  dismissPartial(): void {
    this.partial.set(null);
  }

  /** Re-imports everything, which reads the files a cancel skipped. */
  async reimportAll(): Promise<void> {
    this.partial.set(null);
    await this.store.reload();
  }

  private start(): void {
    if (this.tracking) return;
    this.tracking = true;
    this.partial.set(null);
    this.cancelling.set(false);
    this.long.set(false);
    this.progress.set({ running: true, done: 0, total: 0, cancelled: false, bytesDone: 0, bytesTotal: 0, current: null, elapsedMs: 0, etaMs: null });
    this.timer = setInterval(() => void this.poll(), POLL_MS);
    this.longTimer = setTimeout(() => this.long.set(true), LONG_IMPORT_MS);
    void this.poll();
  }

  private async stop(): Promise<void> {
    if (!this.tracking) return;
    this.tracking = false;
    this.clearTimers();
    this.progress.set(null);
    this.long.set(false);
    this.cancelling.set(false);
    try {
      const last = await this.backend.importProgress();
      if (last.cancelled && last.done < last.total) this.partial.set({ done: last.done, total: last.total });
    } catch {
      /* no final progress: nothing to report */
    }
  }

  private async poll(): Promise<void> {
    try {
      const p = await this.backend.importProgress();
      if (this.tracking && p.running) this.progress.set(p);
    } catch {
      /* progress is a convenience: the import itself reports its errors */
    }
  }

  private clearTimers(): void {
    clearInterval(this.timer);
    clearTimeout(this.longTimer);
  }
}
