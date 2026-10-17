import { isUserError } from '../core/activity/severity';

/**
 * A failed backend command is unexpected unless it is a mistake the user can fix (validation, query syntax,
 * missing files, rights, cancellation): those are shown as they are and need no screenshot. The rule is the
 * one the log uses for severity (`core/activity/severity.ts`).
 */
export function isUnexpectedBackendError(error: unknown): boolean {
  return !isUserError(error);
}

/** Decides per call; the default is "unexpected only". */
export type BackendFailureClassifier = (command: string, error: unknown) => boolean;

export const DEFAULT_CLASSIFIER: BackendFailureClassifier = (_command, error) => isUnexpectedBackendError(error);
