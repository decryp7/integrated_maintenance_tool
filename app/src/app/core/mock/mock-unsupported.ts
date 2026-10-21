import { CmdError } from '../cmd-error';

/**
 * What the engine rejects with for an operation the log store does not offer yet (code `unsupported`, not
 * retryable), for the mocks to mimic. The UI shows its message and hint inline, without Retry.
 */
export function mockUnsupported(message: string, hint?: string): CmdError {
  return new CmdError(hint === undefined ? { code: 'unsupported', message, retryable: false } : { code: 'unsupported', message, retryable: false, hint });
}
