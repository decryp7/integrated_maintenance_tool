import { describe, expect, it } from 'vitest';
import { CmdError } from '../core/cmd-error';
import { jobValue } from './job-value';
import { JobOutcome } from './jobs.types';

const outcome = (o: Partial<JobOutcome>): JobOutcome => ({ id: 1, state: 'done', result: null, error: null, message: null, ...o });
const waiting = (o: JobOutcome) => (async () => o) as never;

describe('jobValue', () => {
  it('asks for the job and gives back the value, or the detail of the file the job wrote', async () => {
    const asked: unknown[] = [];
    const wait = (async (cmd: string, args: unknown) => (asked.push([cmd, args]), outcome({ result: { type: 'value', value: { n: 1 } } }))) as never;
    expect(await jobValue<{ n: number }>(wait, 7)).toEqual({ n: 1 });
    expect(asked).toEqual([['jobs_wait', { id: 7 }]]);
    expect(await jobValue(waiting(outcome({ result: { type: 'file', path: '/a', detail: 'd' } })), 1)).toBe('d');
    expect(await jobValue(waiting(outcome({ result: { type: 'file', path: '/a' } })), 1)).toBeUndefined();
  });

  it('gives back the detail of the workspace a job opened or the model it added', async () => {
    expect(await jobValue(waiting(outcome({ result: { type: 'workspace', detail: { total: 3 } } })), 1)).toEqual({ total: 3 });
    expect(await jobValue(waiting(outcome({ result: { type: 'model', fileName: 'm.gguf', detail: { fileName: 'm.gguf' } } })), 1)).toEqual({ fileName: 'm.gguf' });
    expect(await jobValue(waiting(outcome({ result: { type: 'workspace' } })), 1)).toBeUndefined();
  });

  it('rejects with the reason: cancelled as the user own doing, failed or interrupted with a message', async () => {
    const cancelled = await jobValue(waiting(outcome({ state: 'cancelled', error: 'stopped' })), 1).catch((e) => e);
    expect(cancelled).toBeInstanceOf(CmdError);
    expect(cancelled).toMatchObject({ code: 'cancelled', message: 'stopped', retryable: false });
    const failed = await jobValue(waiting(outcome({ state: 'failed', error: 'no space left' })), 1).catch((e) => e);
    expect(failed).toMatchObject({ code: 'failed', message: 'no space left', retryable: true });
    const interrupted = await jobValue(waiting(outcome({ state: 'interrupted' })), 1).catch((e) => e);
    expect((interrupted as Error).message).toContain('interrupted');
  });
});
