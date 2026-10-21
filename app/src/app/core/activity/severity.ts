import type { ErrorCode } from '../generated/errors';

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

/**
 * The engine's error codes for outcomes that are not IMT failing: the user's input, choices or environment (a cleared
 * job, a refused permission, an offline machine, a missing model). Anything else coded (`internal`, `engine`,
 * `failed`, `io`, `malformed`, `integrity`, …) is unexpected.
 */
const EXPECTED_CODES: ReadonlySet<ErrorCode> = new Set<ErrorCode>([
  'invalid', 'notFound', 'notGranted', 'busy', 'cancelled', 'limit', 'unsupported', 'unavailable', 'fileExists', 'denied',
  'missing', 'noModel', 'noRuntime', 'contextOverflow', 'toolLoopLimit', 'offline', 'network', 'timeout', 'proxyAuth',
  'proxy', 'tls', 'gated', 'refused', 'notACapture', 'notAvailable', 'unreachable', 'noRelease', 'noPlatform',
  'tooLarge', 'diskFull', 'memory', 'signatureInvalid', 'invalidManifest',
]);

/** True for a failure the user can fix themselves (shown as is, logged as a warning). */
export function isUserError(error: unknown): boolean {
  if (typeof DOMException !== 'undefined' && error instanceof DOMException && error.name === 'AbortError') return true;
  // A typed engine error says what it is; the wording is only a fallback for errors without a code.
  const code = (error as { code?: unknown } | null)?.code;
  if (typeof code === 'string') return EXPECTED_CODES.has(code as ErrorCode);
  return USER_ERROR.test(text(error));
}
