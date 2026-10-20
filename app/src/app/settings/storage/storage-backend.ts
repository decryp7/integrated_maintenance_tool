import { inject, Injectable } from '@angular/core';
import { invoke } from '../../core/activity/ipc';
import { reportBackendFailures } from '../../core/failure-reporter';
import { Logger, withFailureLogging } from '../../core/logger';
import { isTauri } from '../../core/platform';
import { StorageCategoryId, StorageClearResult, StorageUsage } from './storage.types';
import { browserMocks } from '../../core/mock-registry';

/**
 * What Settings ▸ Storage asks the engine. Sizes, what may be removed and the removal itself are decided
 * in Rust (`storage.rs`); the UI names category ids only. Injected as a class token: the Tauri backend in
 * the app, the in-memory mock in a plain browser (dev server, e2e tests).
 */
@Injectable({ providedIn: 'root', useFactory: createStorageBackend })
export abstract class StorageBackend {
  /** What IMT stores, per category (computed off the UI thread, bounded). */
  abstract usage(): Promise<StorageUsage>;
  /** Clears the categories (as a background task); reports what was freed and what stayed. */
  abstract clear(ids: StorageCategoryId[]): Promise<StorageClearResult>;
  /** Shows a category's folder in the file manager; resolves with its path. */
  abstract reveal(id: StorageCategoryId): Promise<string>;
}

/** Talks to `app/src-tauri/src/storage.rs`. */
export class TauriStorageBackend extends StorageBackend {
  /** `call` is Tauri's `invoke`; tests pass a fake. */
  constructor(private readonly call: typeof invoke = invoke) {
    super();
  }
  usage() {
    return this.call<StorageUsage>('storage_usage');
  }
  clear(ids: StorageCategoryId[]) {
    return this.call<StorageClearResult>('storage_clear', { ids });
  }
  reveal(id: StorageCategoryId) {
    return this.call<string>('storage_reveal', { id });
  }
}

export function createStorageBackend(): StorageBackend {
  const backend: StorageBackend = isTauri() ? new TauriStorageBackend() : new (browserMocks().MockStorageBackend)(globalThis.location?.search ?? '');
  return withFailureLogging<StorageBackend>(backend, inject(Logger), { onFailure: reportBackendFailures() });
}
