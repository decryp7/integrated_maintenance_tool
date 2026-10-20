import { CmdErrorDto, ErrorCode } from './api.types';

/**
 * A command's rejection (`CmdErrorDto`) as an `Error`: `message` is shown as is, `code` and `retryable`
 * decide what the UI offers. `invoke` (core/activity/ipc.ts) turns every rejection into one, so error
 * helpers that read `Error.message` show the right text.
 */
export class CmdError extends Error implements CmdErrorDto {
  readonly code: ErrorCode;
  readonly retryable: boolean;
  readonly field?: string;
  readonly line?: number;
  readonly hint?: string;
  readonly detail?: string;

  constructor(dto: CmdErrorDto) {
    super(dto.message);
    this.name = 'CmdError';
    this.code = dto.code;
    this.retryable = dto.retryable;
    if (dto.field !== undefined) this.field = dto.field;
    if (dto.line !== undefined) this.line = dto.line;
    if (dto.hint !== undefined) this.hint = dto.hint;
    if (dto.detail !== undefined) this.detail = dto.detail;
  }
}

/** Whether `v` has the wire shape of a command error. */
export function isCmdErrorDto(v: unknown): v is CmdErrorDto {
  if (!v || typeof v !== 'object' || v instanceof CmdError) return false;
  const o = v as Record<string, unknown>;
  return typeof o['code'] === 'string' && typeof o['message'] === 'string' && typeof o['retryable'] === 'boolean';
}

/** A rejection as thrown to callers: a `CmdError` for the wire shape, anything else as it was. */
export function fromRejection(e: unknown): unknown {
  return isCmdErrorDto(e) ? new CmdError(e) : e;
}

/** Whether trying the same request again may succeed (unknown failures: yes). */
export function isRetryable(e: unknown): boolean {
  return e instanceof CmdError ? e.retryable : true;
}

/** The text to show for a failure: its message, then the next step when the engine gave one. */
export function failureText(e: unknown): string {
  if (e instanceof CmdError) return e.hint ? `${e.message} ${e.hint}` : e.message;
  if (typeof e === 'string') return e;
  if (e instanceof Error) return e.message;
  try {
    return JSON.stringify(e);
  } catch {
    return String(e);
  }
}
