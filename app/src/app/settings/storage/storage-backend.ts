import { inject, Injectable } from '@angular/core';
import { invoke } from '../../core/activity/ipc';
import { reportBackendFailures } from '../../core/failure-reporter';
import { Logger, withFailureLogging } from '../../core/logger';
import { isTauri } from '../../core/platform';
import { StorageCategoryId, StoragePolicy, StorageUsage, StoreClearResult, StoreFolders } from './storage.types';
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
  /** Starts clearing as a background maintenance job; resolves with the job at once (its result: what was cleared). */
  abstract clear(ids: StorageCategoryId[]): Promise<number>;
  /** Shows a category's folder in the file manager; resolves with its path. */
  abstract reveal(id: StorageCategoryId): Promise<string>;
  /** The store folders on disk with their state (a query). */
  abstract stores(): Promise<StoreFolders>;
  /** Removes the orphaned stores among `ids`; the engine checks each again and reports the ones it skipped. */
  abstract clearStores(ids: string[]): Promise<StoreClearResult>;
  /** The automatic clean-up of orphaned stores (a query). */
  abstract policy(): Promise<StoragePolicy>;
  /** Sets the automatic clean-up; the engine refuses a value out of range with a message. Resolves with what it stored. */
  abstract setPolicy(policy: StoragePolicy): Promise<StoragePolicy>;
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
  stores() {
    return this.call<StoreFolders>('storage_stores');
  }
  clearStores(ids: string[]) {
    return this.call<StoreClearResult>('storage_stores_clear', { ids });
  }
  policy() {
    return this.call<StoragePolicy>('storage_policy');
  }
  setPolicy(policy: StoragePolicy) {
    return this.call<StoragePolicy>('storage_set_policy', { policy });
  }
}

export function createStorageBackend(): StorageBackend {
  const backend: StorageBackend = isTauri() ? new TauriStorageBackend() : new (browserMocks().MockStorageBackend)(globalThis.location?.search ?? '');
  return withFailureLogging<StorageBackend>(backend, inject(Logger), { onFailure: reportBackendFailures() });
}
