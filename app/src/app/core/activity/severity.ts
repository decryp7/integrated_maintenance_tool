/**
 * Severity of a failure: mistakes the user can fix (validation, query syntax, missing files, rights,
 * cancellation) are warnings; anything else is an unexpected error. Crashes are fatal (Rust only).
 */
const USER_ERROR =
  /\binvalid\b|\bnot valid\b|syntax|parse error|unexpected (token|character|end)|does not compile|no such file|not found|cannot find|already (exists|open)|access (is )?denied|permission|requires? (elevat|administrator)|cancel+ed|aborted|not allowed|must (be|not|end|start)|too (long|large|many)|unsupported|empty|expected |regex|cannot export there|\bnot an? \w/i;

function text(e: unknown): string {
  if (typeof e === 'string') return e;
  if (e instanceof Error) return e.message;
  try {
    return JSON.stringify(e) ?? String(e);
  } catch {
    return String(e);
  }
}

/** True for a failure the user can fix themselves (shown as is, logged as a warning). */
export function isUserError(error: unknown): boolean {
  if (typeof DOMException !== 'undefined' && error instanceof DOMException && error.name === 'AbortError') return true;
  return USER_ERROR.test(text(error));
}
