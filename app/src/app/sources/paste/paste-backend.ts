import { inject, Injectable } from '@angular/core';
import { invoke } from '../../core/activity/ipc';
import { WorkspaceSummary } from '../../core/api.types';
import { DebriefBackend } from '../../core/backend';
import { reportBackendFailures } from '../../core/failure-reporter';
import { Logger, withFailureLogging } from '../../core/logger';
import { browserMocks } from '../../core/mock-registry';
import { isTauri } from '../../core/platform';

/**
 * The "Paste as log" command: the engine saves pasted text as a source file in the project and opens it through the
 * normal importer detection. Injected as a class token: the Tauri backend in the app, the mock over the mock
 * workspace in a plain browser (dev server, e2e tests).
 */
@Injectable({ providedIn: 'root', useFactory: createPasteBackend })
export abstract class PasteBackend {
  /** Saves `text` as a log named `name` (null: the engine names it) and opens it; resolves with the new workspace. */
  abstract pasteLog(text: string, name: string | null): Promise<WorkspaceSummary>;
}

/** Talks to the Rust core (see `api.types.ts`, `paste_log`). */
export class TauriPasteBackend extends PasteBackend {
  /** `call` is Tauri's `invoke`; tests pass a fake. */
  constructor(private readonly call: typeof invoke = invoke) {
    super();
  }
  pasteLog(text: string, name: string | null) {
    return this.call<WorkspaceSummary>('paste_log', { text, name });
  }
}

export function createPasteBackend(): PasteBackend {
  const backend: PasteBackend = isTauri() ? new TauriPasteBackend() : new (browserMocks().MockPasteBackend)(inject(DebriefBackend));
  return withFailureLogging<PasteBackend>(backend, inject(Logger), { onFailure: reportBackendFailures() });
}
