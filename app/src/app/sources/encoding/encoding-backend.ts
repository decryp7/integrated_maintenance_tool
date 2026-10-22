import { inject, Injectable } from '@angular/core';
import { invoke } from '../../core/activity/ipc';
import { EncodingOption } from '../../core/api.types';
import { ImtBackend } from '../../core/backend';
import { reportBackendFailures } from '../../core/failure-reporter';
import { Logger, withFailureLogging } from '../../core/logger';
import { browserMocks } from '../../core/mock-registry';
import { isTauri } from '../../core/platform';

/**
 * The text-encoding commands: the engine detects each source's encoding at import and re-reads one source with an
 * encoding the user chose. Injected as a class token: the Tauri backend in the app, the mock over the mock workspace
 * in a plain browser (dev server, e2e tests).
 */
@Injectable({ providedIn: 'root', useFactory: createEncodingBackend })
export abstract class EncodingBackend {
  /** The encodings "Encoding…" offers (a query). */
  abstract options(): Promise<EncodingOption[]>;
  /** Re-reads one source with that encoding (null: detect it again) as a job; resolves with its id. */
  abstract setSourceEncoding(sourceId: number, encoding: string | null): Promise<number>;
}

/** Talks to the Rust core (see `api.types.ts`, `list_encodings` and `set_source_encoding`). */
export class TauriEncodingBackend extends EncodingBackend {
  /** `call` is Tauri's `invoke`; tests pass a fake. */
  constructor(private readonly call: typeof invoke = invoke) {
    super();
  }
  options() {
    return this.call<EncodingOption[]>('list_encodings');
  }
  setSourceEncoding(sourceId: number, encoding: string | null) {
    return this.call<number>('set_source_encoding', { sourceId, encoding });
  }
}

export function createEncodingBackend(): EncodingBackend {
  const backend: EncodingBackend = isTauri() ? new TauriEncodingBackend() : new (browserMocks().MockEncodingBackend)(inject(ImtBackend));
  return withFailureLogging<EncodingBackend>(backend, inject(Logger), { onFailure: reportBackendFailures() });
}
