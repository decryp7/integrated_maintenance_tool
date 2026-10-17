import { ProfilingError, ProfilingErrorCode } from '../core/api.types';

const CODES: readonly ProfilingErrorCode[] = ['notACapture', 'io', 'limit', 'unsupported', 'invalid', 'notFound'];

/**
 * A rejected profiling command, as an `Error` (so every error display shows its message) that keeps
 * the engine's code. Views decide by `code`, never by matching the message.
 */
export class ProfilingFailure extends Error implements ProfilingError {
  constructor(
    readonly code: ProfilingErrorCode,
    message: string,
  ) {
    super(message);
    this.name = 'ProfilingFailure';
  }
}

function isProfilingError(e: unknown): e is ProfilingError {
  if (e === null || typeof e !== 'object') return false;
  const { code, message } = e as Record<string, unknown>;
  return typeof message === 'string' && typeof code === 'string' && (CODES as readonly string[]).includes(code);
}

/**
 * Parses whatever a command rejected with (the engine's `{ code, message }`; a plain string when
 * Tauri itself refused the arguments) into a `ProfilingFailure`.
 */
export function toProfilingFailure(e: unknown): ProfilingFailure {
  if (e instanceof ProfilingFailure) return e;
  if (isProfilingError(e)) return new ProfilingFailure(e.code, e.message);
  if (typeof e === 'string') return new ProfilingFailure('invalid', e);
  if (e instanceof Error) return new ProfilingFailure('invalid', e.message);
  return new ProfilingFailure('invalid', JSON.stringify(e) ?? String(e));
}

/** The code of a profiling failure, or null for anything else. */
export function profilingErrorCode(e: unknown): ProfilingErrorCode | null {
  return e instanceof ProfilingFailure || isProfilingError(e) ? e.code : null;
}
