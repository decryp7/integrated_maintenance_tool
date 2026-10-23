import { CmdError } from '../core/cmd-error';
import { JobOutcome } from './jobs.types';

/** Waits for the job (`jobs_wait`); `wait` is the transport (Tauri's `invoke` bound to the command). */
type Wait = (cmd: 'jobs_wait', args: { id: number }) => Promise<JobOutcome>;

/**
 * A command that does long work starts a job and returns its id at once; this waits for the job and gives back the value
 * the command would have returned: a job that is done carries it in its result (`value`, or the `detail` of the file it
 * wrote); one that failed or was cancelled rejects with a `CmdError` (`cancelled` for Cancel, so callers treat it as
 * the user's own doing). Closing the caller's dialog does not stop the job; the Jobs pane does.
 */
export async function jobValue<T>(wait: Wait, id: number): Promise<T> {
  const o = await wait('jobs_wait', { id });
  if (o.state === 'done') {
    const r = o.result;
    if (r?.type === 'value') return r.value as T;
    if ((r?.type === 'file' || r?.type === 'workspace' || r?.type === 'model') && r.detail !== undefined) return r.detail as T;
    return undefined as T;
  }
  if (o.state === 'cancelled') throw new CmdError({ code: 'cancelled', message: o.error ?? 'Cancelled.', retryable: false });
  throw new CmdError({
    code: 'failed',
    message: o.error ?? (o.state === 'interrupted' ? 'The work was interrupted before it finished.' : 'The work failed.'),
    retryable: true,
  });
}
