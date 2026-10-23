import { computed, effect, inject, Injectable, signal, untracked } from '@angular/core';
import { ImportPhase, ImportProgress } from '../core/api.types';
import { WorkspaceStore } from '../core/workspace.store';
import { WorkspaceStatusStore } from '../core/workspace/status.store';
import { UxBackend } from '../core/ux-backend';
import { JobsStore } from '../jobs/jobs.store';
import { JobInfo, JobProgress } from '../jobs/jobs.types';

/** Imports running longer than this also show the progress toast. */
export const LONG_IMPORT_MS = 1500;

/** What the last cancelled import read. */
export interface PartialImport {
  done: number;
  total: number;
}

/** What the widgets show before the import's job reports (the engine lists a job once it ran a moment). */
const STARTING: ImportProgress = { running: true, phase: 'finding', done: 0, total: 0, cancelled: false, bytesDone: 0, bytesTotal: 0, current: null, elapsedMs: 0, etaMs: null };

/**
 * The import phase from the job's one phase (label and scale): an indeterminate count with no total is the
 * folder walk, an indeterminate phase after the total is known is the tail (its label is the step), a count
 * without bytes is the check of each file, and anything else reads the files.
 */
function importPhaseOf(p: JobProgress): { phase: ImportPhase; stage?: string } {
  const phase = p.phase;
  if (!phase) return { phase: 'importing' };
  if (phase.scale === 'indeterminate') return (p.total ?? 0) > 0 ? { phase: 'finishing', stage: phase.label } : { phase: 'finding' };
  return { phase: (p.bytesTotal ?? 0) > 0 ? 'importing' : 'detecting' };
}

/** A running import job's progress (\`debrief://jobs\`) in the shape the widgets show. */
export function jobImportProgress(job: JobInfo): ImportProgress {
  const p = job.progress;
  const { phase, stage } = importPhaseOf(p);
  return {
    running: true,
    phase,
    done: p.done ?? 0,
    total: p.total ?? 0,
    cancelled: false,
    bytesDone: p.bytesDone ?? 0,
    bytesTotal: p.bytesTotal ?? 0,
    current: p.current ?? null,
    elapsedMs: job.elapsedMs,
    etaMs: p.etaMs ?? job.etaMs,
    ...(stage ? { stage } : {}),
  };
}

/**
 * Follows the engine's import while the workspace imports files (its busy state is an import: open, re-import,
 * profile apply): the progress comes from the import's job (files, bytes and the current file, from the jobs
 * store), Cancel stops it, and a cancelled import is remembered so a "partially imported" banner can say what was
 * read (one \`import_progress\` read when the import ends).
 */
@Injectable({ providedIn: 'root' })
export class ImportProgressService {
  private readonly store = inject(WorkspaceStore);
  private readonly status = inject(WorkspaceStatusStore);
  private readonly backend = inject(UxBackend);
  private readonly jobs = inject(JobsStore);

  private readonly tracking = signal(false);
  /** The import's job: the busy state's when it is known, else the newest running import of this window. */
  private readonly job = computed(() => {
    const busy = this.status.busy();
    const running = this.jobs.snapshot().jobs.filter((j) => j.kind === 'import' && j.state === 'running');
    const known = busy?.kind === 'import' && busy.jobId !== null ? running.find((j) => j.id === busy.jobId) : undefined;
    return known ?? running.at(-1) ?? null;
  });

  /** Latest progress of the running import; null when none runs. */
  readonly progress = computed<ImportProgress | null>(() => {
    if (!this.tracking()) return null;
    const job = this.job();
    return job ? jobImportProgress(job) : STARTING;
  });
  /** The running import has taken longer than \`LONG_IMPORT_MS\` (the toast shows). */
  readonly long = signal(false);
  readonly cancelling = signal(false);
  /** The last import was cancelled; null once dismissed or after a complete import. */
  readonly partial = signal<PartialImport | null>(null);
  readonly active = computed(() => this.progress() !== null);

  private longTimer: ReturnType<typeof setTimeout> | undefined;

  constructor() {
    effect((onCleanup) => {
      const importing = this.status.importing();
      untracked(() => (importing ? this.start() : void this.stop()));
      onCleanup(() => clearTimeout(this.longTimer));
    });
  }

  /** Asks the engine to stop after the files in flight. */
  async cancel(): Promise<void> {
    if (!this.tracking() || this.cancelling()) return;
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
    if (this.tracking()) return;
    this.tracking.set(true);
    this.partial.set(null);
    this.cancelling.set(false);
    this.long.set(false);
    clearTimeout(this.longTimer);
    this.longTimer = setTimeout(() => this.long.set(true), LONG_IMPORT_MS);
  }

  private async stop(): Promise<void> {
    if (!this.tracking()) return;
    this.tracking.set(false);
    clearTimeout(this.longTimer);
    this.long.set(false);
    this.cancelling.set(false);
    try {
      const last = await this.backend.importProgress();
      if (last.cancelled && last.done < last.total) this.partial.set({ done: last.done, total: last.total });
    } catch {
      /* no final progress: nothing to report */
    }
  }
}
