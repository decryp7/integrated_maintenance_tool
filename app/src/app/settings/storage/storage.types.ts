/**
 * Wire contract of Settings ▸ Storage (`app/src-tauri/src/storage/model.rs`, camelCase JSON). The UI only
 * ever sends category ids; folders are resolved and checked in Rust.
 */

/** Navigation id of the Storage section in the Settings screen (UI-only, like General). */
export const STORAGE_ID = '@storage';
/** Command that opens Settings ▸ Storage ("Clear cache…"). */
export const CLEAR_CACHE_COMMAND = 'help.clearCache';

export type StorageCategoryId =
  | 'bundle-extracts'
  | 'webview-cache'
  | 'web-profile'
  | 'ai-runtimes-outdated'
  | 'ai-runtimes'
  | 'ai-models'
  | 'logs'
  | 'error-snapshots'
  | 'sessions'
  | 'stores';

/** `cache`: IMT rebuilds it, cleared together by "Clear cache". `data`: the user's, cleared one at a time. */
export type StorageKind = 'cache' | 'data';

export interface StorageCategory {
  id: StorageCategoryId;
  /** English label from the engine (the UI shows its translation by id). */
  label: string;
  kind: StorageKind;
  /** The folder; null when IMT cannot resolve it on this computer. */
  path: string | null;
  exists: boolean;
  bytes: number;
  files: number;
  /** What clearing would remove now (protected items such as today's log excluded). */
  clearableBytes: number;
  /** The scan stopped at its limits: sizes are lower bounds ("≥"). */
  capped: boolean;
  clearable: boolean;
  /** Why it cannot be cleared, or what clearing keeps. */
  note: string | null;
}

export interface StorageUsage {
  categories: StorageCategory[];
  totalBytes: number;
  capped: boolean;
}

export interface StorageSkipped {
  path: string;
  reason: string;
}

export interface StorageCategoryClearResult {
  id: StorageCategoryId;
  freedBytes: number;
  removedFiles: number;
  /** The first skipped items; `skippedCount` counts them all. */
  skipped: StorageSkipped[];
  skippedCount: number;
  /** The category could not be cleared at all. */
  error: string | null;
}

export interface StorageClearResult {
  freedBytes: number;
  categories: StorageCategoryClearResult[];
  /** Stopped from the background tasks before everything was cleared. */
  cancelled: boolean;
}
