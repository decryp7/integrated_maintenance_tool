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

/**
 * Why a store folder is on disk (`storage_stores`): `open` a window has it open; `session` an autosaved session reopens
 * it (kept so it can be restored); `running` another IMT process uses it; `orphaned` nothing references it (for
 * example the extraction of a project that was closed). Only orphaned stores can be cleaned up.
 */
export type StoreFolderState = 'open' | 'session' | 'running' | 'orphaned';

export interface StoreFolder {
  /** The folder's name; also the id `storage_stores_clear` takes. */
  id: string;
  bytes: number;
  files: number;
  /** When the store was last used (ms since the epoch); null when unknown. */
  lastUsedMs: number | null;
  state: StoreFolderState;
}

/** The store folders on disk, newest first. */
export interface StoreFolders {
  folders: StoreFolder[];
  totalBytes: number;
  /** What cleaning up every orphaned store would free. */
  orphanedBytes: number;
  /** The scan stopped at its limits: sizes are lower bounds. */
  capped: boolean;
}

export interface StoreSkipped {
  id: string;
  /** Why it stayed, for example "a window has this store open". */
  reason: string;
}

export interface StoreClearResult {
  freedBytes: number;
  removed: string[];
  skipped: StoreSkipped[];
  /** Stopped before every store was handled. */
  cancelled: boolean;
}

/** Automatic clean-up of orphaned stores (`storage_policy`). */
export interface StoragePolicy {
  /** Orphaned stores unused for this many days are removed on their own; 0 = off, at most 365. */
  autoCleanDays: number;
}

/** The largest `autoCleanDays` the engine accepts. */
export const MAX_AUTO_CLEAN_DAYS = 365;
