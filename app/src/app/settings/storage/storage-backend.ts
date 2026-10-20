import { inject, Injectable } from '@angular/core';
import { invoke } from '../../core/activity/ipc';
import { reportBackendFailures } from '../../core/failure-reporter';
import { Logger, withFailureLogging } from '../../core/logger';
import { isTauri } from '../../core/platform';
import { MockStorageBackend } from './mock-storage-backend';
import { StorageCategoryId, StorageUsage } from './storage.types';

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
  /** Starts clearing as a background maintenance job; resolves with the job at once (its result: what was cleared). */
  abstract clear(ids: StorageCategoryId[]): Promise<number>;
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
    return this.call<number>('storage_clear', { ids });
  }
  reveal(id: StorageCategoryId) {
    return this.call<string>('storage_reveal', { id });
  }
}

export function createStorageBackend(): StorageBackend {
  const backend: StorageBackend = isTauri() ? new TauriStorageBackend() : new MockStorageBackend(globalThis.location?.search ?? '');
  return withFailureLogging<StorageBackend>(backend, inject(Logger), { onFailure: reportBackendFailures() });
}
