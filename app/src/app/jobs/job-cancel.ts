import { invoke } from '../core/activity/ipc';
import { JobInfo, JobsSnapshot } from './jobs.types';

/**
 * Jobs are the one cancel authority (`jobs_cancel`): work the UI started (an import, research, a set-up)
 * is stopped through its job. These helpers are for backends that hand the UI no job id of their own.
 */
export function isActive(job: JobInfo): boolean {
  return job.state === 'queued' || job.state === 'running';
}

/** Cancels the calling window's active jobs that `match`; how many were asked to stop. */
export async function cancelActiveJobs(call: typeof invoke, match: (job: JobInfo) => boolean): Promise<number> {
  const snapshot = await call<JobsSnapshot>('jobs_list');
  const ids = snapshot.jobs.filter((j) => isActive(j) && match(j)).map((j) => j.id);
  for (const id of ids) await call<void>('jobs_cancel', { id });
  return ids.length;
}

/** Cancels job `id`; false when there is none to cancel. */
export async function cancelJob(call: typeof invoke, id: number | null | undefined): Promise<boolean> {
  if (id === null || id === undefined) return false;
  await call<void>('jobs_cancel', { id });
  return true;
}
