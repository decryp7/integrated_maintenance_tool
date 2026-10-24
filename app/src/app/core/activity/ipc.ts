import { invoke as tauriInvoke, InvokeArgs, InvokeOptions } from '@tauri-apps/api/core';
import { currentOperationId } from './runtime';
import { OP_HEADER } from './types';
import { fromRejection } from '../cmd-error';
import { reportRefusedCommand } from '../denied-command';

/** `options` with the operation id header added (existing headers kept; an explicit one wins). */
export function withOpHeader(options: InvokeOptions | undefined, op: string): InvokeOptions {
  const h = options?.headers;
  if (h instanceof Headers) {
    const copy = new Headers(h);
    if (!copy.has(OP_HEADER)) copy.set(OP_HEADER, op);
    return { ...options, headers: copy };
  }
  if (Array.isArray(h)) return { ...options, headers: h.some(([k]) => k.toLowerCase() === OP_HEADER) ? h : [...h, [OP_HEADER, op]] };
  return { ...options, headers: { [OP_HEADER]: op, ...(h as Record<string, string> | undefined) } };
}

/**
 * Tauri's `invoke`, carrying the current operation's id as the `debrief-op-id` header so Rust's invoke wrapper
 * (`activity::logged`) logs the command with the user operation that caused it. Every Tauri backend imports
 * `invoke` from here, never from `@tauri-apps/api/core` (a coverage test enforces it): Tauri's IPC internals
 * are frozen, so this is the one place every command passes through. Rejections come back as `CmdError`s
 * (the one error contract, ADR 0016).
 */
export function invoke<T>(cmd: string, args?: InvokeArgs, options?: InvokeOptions): Promise<T> {
  const op = currentOperationId();
  return tauriInvoke<T>(cmd, args, op ? withOpHeader(options, op) : options).catch((e: unknown) => {
    const err = fromRejection(e);
    reportRefusedCommand(cmd, err);
    throw err;
  });
}
