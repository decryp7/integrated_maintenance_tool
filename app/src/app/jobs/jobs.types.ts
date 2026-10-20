// Wire contract of the jobs commands and the `imt://jobs` event (camelCase JSON), mirrored by
// app/src-tauri/src/jobs/{model,queue,persist}.rs. Change both sides together.

/** Each kind has its own queue and limit (enforced by the engine). */
export type JobKind = 'ai' | 'research' | 'analysis' | 'import' | 'export' | 'download' | 'collection' | 'capture' | 'maintenance';

export const JOB_KINDS: readonly JobKind[] = ['ai', 'research', 'analysis', 'import', 'export', 'download', 'collection', 'capture', 'maintenance'];

export type JobState = 'queued' | 'running' | 'done' | 'failed' | 'cancelled' | 'interrupted';

/** What started a job. */
export type JobLink =
  | { type: 'event'; recordId: number }
  | { type: 'range'; from: number; to: number }
  | { type: 'query'; ql: string }
  | { type: 'path'; path: string }
  | { type: 'chat'; sessionId: string }
  | { type: 'model'; id: string };

export interface JobSource {
  /** The window the job belongs to; null for app-wide work (downloads, storage). */
  window: string | null;
  link: JobLink | null;
}

export interface JobProgress {
  /** 0..1. */
  fraction: number | null;
  done: number | null;
  total: number | null;
  etaMs: number | null;
}

/** Where a finished job's result is (`jobs_result`). */
export type JobResult =
  | { type: 'narrative'; narrative: unknown }
  | { type: 'proposal'; proposal: unknown }
  | { type: 'chat'; sessionId: string }
  | { type: 'file'; path: string }
  | { type: 'workspace' }
  | { type: 'view'; view: string; ql: string | null }
  | { type: 'model'; fileName: string | null }
  | { type: 'folderSurvey'; outcome: unknown }
  | { type: 'storage'; result: unknown };

export type JobResultType = JobResult['type'];

/** A job as the pane shows it. */
export interface JobInfo {
  id: number;
  kind: JobKind;
  title: string;
  source: JobSource;
  state: JobState;
  progress: JobProgress;
  message: string | null;
  /** Epoch ms. */
  queuedAt: number;
  startedAt: number | null;
  finishedAt: number | null;
  elapsedMs: number;
  etaMs: number | null;
  cancellable: boolean;
  retryable: boolean;
  /** A queued AI job: can be moved up and down. */
  movable: boolean;
  /** 1-based place in its kind's queue while queued. */
  position: number | null;
  resultType: JobResultType | null;
  error: string | null;
  /** Finished, needed attention, not viewed yet. */
  unseen: boolean;
  retryOf: number | null;
}

/** `jobs_wait`: how a job ended, with its whole result, for the dialog that followed it while it ran. */
export interface JobOutcome {
  id: number;
  state: JobState;
  result: JobResult | null;
  error: string | null;
  message: string | null;
}

/** A queue the engine paused (the AI after a quarantine). */
export interface PausedKind {
  kind: JobKind;
  reason: string;
}

/** The window's visible jobs (running, queued in order, then finished newest first) and paused queues. */
export interface JobsSnapshot {
  jobs: JobInfo[];
  paused: PausedKind[];
}

/** What `imt://jobs` carries: the change since the window's last event (`version` follows the one before by 1): the jobs
 * in their order by id, those new or changed in full, the paused queues. Mirrors `jobs::JobsDelta`. */
export interface JobsDelta {
  version: number;
  order: number[];
  changed: JobInfo[];
  paused: PausedKind[];
}

/** The finished jobs saved with the project and session (UI state key `jobs`). */
export interface JobsRecord {
  version: number;
  jobs: unknown[];
}

export interface RestoredJobs {
  snapshot: JobsSnapshot;
  warning: string | null;
}

export const EMPTY_JOBS: JobsSnapshot = { jobs: [], paused: [] };
