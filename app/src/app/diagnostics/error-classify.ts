import { describeError } from '../core/logger';

/**
 * Messages the engine returns for mistakes the user can fix (validation, query syntax, missing
 * files, rights, cancellation). These are shown to the user as they are and do not need a screenshot.
 */
const USER_ERROR =
  /\binvalid\b|\bnot valid\b|syntax|parse error|unexpected (token|character|end)|does not compile|no such file|not found|cannot find|already (exists|open)|access (is )?denied|permission|requires? (elevat|administrator)|cancel+ed|aborted|not allowed|must (be|not|end|start)|too (long|large|many)|unsupported|empty|expected |regex|cannot export there/i;

/** What a failed backend command means. */
export function isUnexpectedBackendError(error: unknown): boolean {
  if (error instanceof DOMException && error.name === 'AbortError') return false;
  return !USER_ERROR.test(describeError(error));
}

/** Decides per call; the default is "unexpected only". */
export type BackendFailureClassifier = (command: string, error: unknown) => boolean;

export const DEFAULT_CLASSIFIER: BackendFailureClassifier = (_command, error) => isUnexpectedBackendError(error);
