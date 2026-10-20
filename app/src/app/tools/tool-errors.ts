import { failureText } from '../core/cmd-error';
/** Text of a rejected backend call (its message and next step; see `CmdError`). */
export function errorMessage(e: unknown): string {
  return failureText(e);
}

const ACCESS_DENIED = /access (is )?denied|requires? (elevat|administrator)|administrator (rights|privileges)|run as administrator|elevation/i;

/** True when a failure was caused by missing rights, so "Restart as administrator" is the next step. */
export function isAccessDenied(message: string | null | undefined): boolean {
  return !!message && ACCESS_DENIED.test(message);
}

/** The last path segment of a Windows or POSIX path. */
export function baseName(path: string): string {
  return path.split(/[\\/]/).filter(Boolean).at(-1) ?? path;
}
