import { invoke } from './activity/ipc';
import { inject, Provider } from '@angular/core';
import { OffsetChange, SetOffsetRequest, SuggestRequest, SuggestResult } from './api.types';
import { ImtBackend } from './backend';
import { reportBackendFailures } from './failure-reporter';
import { Logger, withFailureLogging } from './logger';
import { isTauri } from './platform';

/** Clock-skew correction: per-source time offsets. Injected as a class token (see `provideSkewBackend`). */
export abstract class SkewBackend {
  /** Sets a source's offset (manual or aligned on two events); the engine moves its records. */
  abstract setSourceOffset(request: SetOffsetRequest): Promise<OffsetChange>;
  /** Offsets that would align sources sharing correlation ids, with a confidence each. A pure query. */
  abstract suggestOffsets(request?: SuggestRequest): Promise<SuggestResult>;
}

/** Talks to the Rust core through Tauri commands (see `api.types.ts` for the contract). */
export class TauriSkewBackend extends SkewBackend {
  /** `call` is Tauri's `invoke`; tests pass a fake. */
  constructor(private readonly call: typeof invoke = invoke) {
    super();
  }

  setSourceOffset(request: SetOffsetRequest) {
    return this.call<OffsetChange>('set_source_offset', { request });
  }
  suggestOffsets(request: SuggestRequest = {}) {
    return this.call<SuggestResult>('suggest_offsets', { request });
  }
}

/** True when a backend object also implements the skew commands (the mock backend does). */
export function hasSkew(backend: unknown): backend is SkewBackend {
  const b = backend as Partial<SkewBackend> | null;
  return !!b && typeof b.setSourceOffset === 'function' && typeof b.suggestOffsets === 'function';
}

/** A backend that refuses, for a workspace backend without clock correction. */
export class UnsupportedSkewBackend extends SkewBackend {
  setSourceOffset(): Promise<OffsetChange> {
    return Promise.reject($localize`:@@core.skew.unsupported:this backend cannot correct clock skew`);
  }
  suggestOffsets(): Promise<SuggestResult> {
    return Promise.resolve({ reference: null, fields: [], suggestions: [], insufficient: [] });
  }
}

/**
 * The Rust backend inside Tauri; in a plain browser the in-memory mock workspace (which shifts its own
 * records, so every view reflects the change, and already logs and reports its failures).
 */
export function createSkewBackend(): SkewBackend {
  if (!isTauri()) {
    const workspace = inject(ImtBackend);
    return hasSkew(workspace) ? workspace : new UnsupportedSkewBackend();
  }
  return withFailureLogging<SkewBackend>(new TauriSkewBackend(), inject(Logger), { onFailure: reportBackendFailures() });
}

export function provideSkewBackend(): Provider {
  return { provide: SkewBackend, useFactory: createSkewBackend };
}
