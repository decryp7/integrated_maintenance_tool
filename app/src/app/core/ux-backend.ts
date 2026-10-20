import { inject, Injectable } from '@angular/core';
import { invoke } from './activity/ipc';
import { ImportProgress, ProfileDoc, ProfileTemplate, ProfileVersionInfo } from './api.types';
import { ImtBackend } from './backend';
import { reportBackendFailures } from './failure-reporter';
import { Logger, withFailureLogging } from './logger';
import { isTauri } from './platform';
import { browserMocks } from './mock-registry';

/**
 * The engine commands behind the ease-of-use features: import progress and cancel, profile history,
 * profile templates and the bundled demo. Provided in root (Tauri inside the app, the mock over the mock
 * workspace in a browser), so features and tests need no extra provider.
 */
@Injectable({ providedIn: 'root', useFactory: createUxBackend })
export abstract class UxBackend {
  /** Progress of the running (or last) import; readable while the import runs. */
  abstract importProgress(): Promise<ImportProgress>;
  /** Stops the running import after the files in flight; the summary lists the rest as not imported. */
  abstract cancelImport(): Promise<void>;
  /** Saved previous versions of the active profile, newest first. */
  abstract profileHistory(): Promise<ProfileVersionInfo[]>;
  /** One previous version's text (applied only when saved again). */
  abstract profileVersion(file: string): Promise<ProfileDoc>;
  /** Ready-made profiles for common formats. */
  abstract profileTemplates(): Promise<ProfileTemplate[]>;
  /** The bundled sample folders to open for the demo; empty when they are not installed. */
  abstract demoPaths(): Promise<string[]>;
}

/** Talks to the Rust core through Tauri commands (see `api.types.ts`). */
export class TauriUxBackend extends UxBackend {
  /** `call` is Tauri's `invoke`; tests pass a fake. */
  constructor(private readonly call: typeof invoke = invoke) {
    super();
  }
  importProgress() {
    return this.call<ImportProgress>('import_progress');
  }
  cancelImport() {
    return this.call<void>('cancel_import');
  }
  profileHistory() {
    return this.call<ProfileVersionInfo[]>('profile_history');
  }
  profileVersion(file: string) {
    return this.call<ProfileDoc>('profile_version', { file });
  }
  profileTemplates() {
    return this.call<ProfileTemplate[]>('profile_templates');
  }
  demoPaths() {
    return this.call<string[]>('demo_paths');
  }
}

export function createUxBackend(): UxBackend {
  const backend: UxBackend = isTauri() ? new TauriUxBackend() : new (browserMocks().MockUxBackend)(inject(ImtBackend));
  return withFailureLogging<UxBackend>(backend, inject(Logger), { onFailure: reportBackendFailures() });
}
