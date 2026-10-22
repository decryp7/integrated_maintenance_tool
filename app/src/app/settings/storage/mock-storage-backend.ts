import type { StorageBackend } from './storage-backend';
import { mockJobs, mockSteps } from '../../jobs/mock-jobs';
import {
  MAX_AUTO_CLEAN_DAYS,
  StorageCategory,
  StorageCategoryClearResult,
  StorageCategoryId,
  StorageClearResult,
  StoragePolicy,
  StorageUsage,
  StoreClearResult,
  StoreFolder,
  StoreFolders,
  StoreFolderState,
} from './storage.types';

const MB = 1024 * 1024;

/** Plausible categories shaped like the engine's. Browser development and UI tests only. */
export function mockCategories(): StorageCategory[] {
  const row = (c: Partial<StorageCategory> & Pick<StorageCategory, 'id' | 'label' | 'kind' | 'bytes'>): StorageCategory => ({
    path: `C:\\Users\\demo\\AppData\\Roaming\\imt\\${c.id}`,
    exists: true,
    files: Math.max(1, Math.round(c.bytes / MB)),
    clearableBytes: c.bytes,
    capped: false,
    clearable: true,
    note: null,
    ...c,
  });
  return [
    row({ id: 'bundle-extracts', label: 'Opened bundles (temporary copies)', kind: 'cache', bytes: 12 * MB, note: 'Bundles opened in this session are kept.' }),
    row({ id: 'webview-cache', label: 'Web view cache', kind: 'cache', bytes: 48 * MB, clearableBytes: 45 * MB, note: 'Files the running web view holds open are removed after IMT restarts.' }),
    row({ id: 'ai-runtimes-outdated', label: 'Outdated AI runtimes', kind: 'cache', bytes: 210 * MB }),
    row({ id: 'ai-runtimes', label: 'AI runtimes', kind: 'data', bytes: 230 * MB, note: 'The AI assistant downloads a runtime again when it needs one.' }),
    row({ id: 'ai-models', label: 'AI models', kind: 'data', bytes: 4_400 * MB, clearableBytes: 0, note: 'The active model is kept.' }),
    row({ id: 'logs', label: 'IMT logs', kind: 'data', bytes: 6 * MB, clearableBytes: 5 * MB, note: "Today's log is kept." }),
    row({ id: 'error-snapshots', label: 'Error snapshots', kind: 'data', bytes: 3 * MB }),
    row({ id: 'sessions', label: 'Session backups', kind: 'data', bytes: 1 * MB, clearableBytes: 0, note: 'Sessions of this run are kept.' }),
  ];
}

/** Why the mock keeps something when clearing it (mirrors the engine's protections). */
const KEPT: Partial<Record<StorageCategoryId, string>> = {
  'webview-cache': 'in use by a running program',
  'ai-models': 'the active model: choose another one or remove it in Settings ▸ AI',
  logs: 'IMT is writing to it',
  sessions: 'saved in this session',
};

const DAY = 24 * 60 * 60 * 1000;

/** Demo store folders shaped like the engine's: one open, one for a session, one in use elsewhere, two orphaned. */
export function mockStoreFolders(now = Date.now()): StoreFolder[] {
  const row = (id: string, state: StoreFolderState, mb: number, ageDays: number | null): StoreFolder => ({
    id,
    state,
    bytes: mb * MB,
    files: Math.max(1, Math.round(mb / 2)),
    lastUsedMs: ageDays === null ? null : now - ageDays * DAY,
  });
  return [
    row('store-7f3a91c2', 'open', 640, 0),
    row('store-2b8e04d5', 'session', 310, 1),
    row('store-c91d57aa', 'running', 120, 2),
    row('store-5e60b3f8', 'orphaned', 96, 9),
    row('store-0a4c7712', 'orphaned', 1800, 41),
  ];
}

/** Why the engine keeps a store that is not orphaned (the mock mirrors the messages). */
const STORE_KEPT: Record<Exclude<StoreFolderState, 'orphaned'>, string> = {
  open: 'a window has this store open',
  session: 'an autosaved session reopens it',
  running: 'another IMT process is using it',
};

/**
 * In-memory storage: clearing frees the clearable bytes and keeps what the engine would keep. The
 * URL switch `?mockStorageFail=N` makes the first N calls fail (error and Retry paths).
 */
export class MockStorageBackend implements StorageBackend {
  private categories = mockCategories();
  private folders = mockStoreFolders();
  private autoCleanDays = 0;
  private failures: number;
  private storeFailures: number;

  constructor(search = '') {
    const n = Number(new URLSearchParams(search).get('mockStorageFail') ?? '0');
    this.failures = Number.isInteger(n) && n > 0 ? Math.min(n, 100) : 0;
    const m = Number(new URLSearchParams(search).get('mockStoresFail') ?? '0');
    this.storeFailures = Number.isInteger(m) && m > 0 ? Math.min(m, 100) : 0;
  }

  /** `?mockStoresFail=N` makes the first N store calls fail. */
  private async maybeFailStores(what: string): Promise<void> {
    await Promise.resolve();
    if (this.storeFailures > 0) {
      this.storeFailures--;
      throw `could not ${what}: the stores folder is not readable (mock)`;
    }
  }

  private async maybeFail(what: string): Promise<void> {
    await Promise.resolve();
    if (this.failures > 0) {
      this.failures--;
      throw `could not ${what}: the storage folder is not readable (mock)`;
    }
  }

  async usage(): Promise<StorageUsage> {
    await this.maybeFail('measure IMT’s storage');
    const categories = this.categories.map((c) => ({ ...c }));
    return { categories, totalBytes: categories.reduce((n, c) => n + c.bytes, 0), capped: categories.some((c) => c.capped) };
  }

  /** Like the engine: checked now, cleared by a maintenance job (resolves with it at once). */
  async clear(ids: StorageCategoryId[]): Promise<number> {
    await this.maybeFail('clear IMT’s storage');
    if (!ids.length) throw 'choose what to clear';
    for (const id of ids) if (!this.categories.some((x) => x.id === id) && !(['stores'] as string[]).includes(id)) throw `unknown storage category '${id}'`;
    const job = mockJobs().start({ kind: 'maintenance', title: "Clear IMT's storage", appWide: true }, async (ctx) => {
      await mockSteps(ctx, this.clearMs, 4);
      const result = this.clearNow(ids);
      ctx.message(`Freed ${(result.freedBytes / MB).toFixed(1)} MB.`);
      return { value: result, result: { type: 'storage', result } };
    });
    job.done.catch(() => undefined);
    return job.id;
  }

  /** How long a clear takes (ms). */
  clearMs = 0;

  private clearNow(ids: StorageCategoryId[]): StorageClearResult {
    const results: StorageCategoryClearResult[] = [];
    for (const id of [...new Set(ids)]) {
      const c = this.categories.find((x) => x.id === id);
      if (!c) {
        if (!(['stores'] as string[]).includes(id)) throw `unknown storage category '${id}'`;
        results.push({ id, freedBytes: 0, removedFiles: 0, skipped: [], skippedCount: 0, error: null });
        continue;
      }
      if (!c.clearable) {
        results.push({ id, freedBytes: 0, removedFiles: 0, skipped: [], skippedCount: 0, error: c.note ?? 'cannot be cleared' });
        continue;
      }
      const freed = c.clearableBytes;
      const kept = c.bytes - freed;
      const reason = KEPT[id];
      const skipped = kept > 0 && reason ? [{ path: `${c.path}\\kept-${id}`, reason }] : [];
      results.push({ id, freedBytes: freed, removedFiles: Math.round(freed / MB), skipped, skippedCount: skipped.length, error: null });
      c.bytes = kept;
      c.clearableBytes = 0;
      c.files = kept ? Math.max(1, Math.round(kept / MB)) : 0;
    }
    return { freedBytes: results.reduce((n, r) => n + r.freedBytes, 0), categories: results, cancelled: false };
  }

  async reveal(id: StorageCategoryId): Promise<string> {
    await this.maybeFail('open the folder');
    const c = this.categories.find((x) => x.id === id);
    if (!c?.path) throw 'unknown storage category';
    return c.path;
  }

  async stores(): Promise<StoreFolders> {
    await this.maybeFailStores('list the project stores');
    const folders = this.folders.map((f) => ({ ...f })).sort((a, b) => (b.lastUsedMs ?? 0) - (a.lastUsedMs ?? 0));
    const orphaned = folders.filter((f) => f.state === 'orphaned');
    return {
      folders,
      totalBytes: folders.reduce((n, f) => n + f.bytes, 0),
      orphanedBytes: orphaned.reduce((n, f) => n + f.bytes, 0),
      capped: false,
    };
  }

  /** Like the engine: only orphaned stores go; every other id is skipped with the reason. */
  async clearStores(ids: string[]): Promise<StoreClearResult> {
    await this.maybeFailStores('clean up the project stores');
    if (!ids.length) throw 'choose the stores to clean up';
    const removed: string[] = [];
    const skipped: StoreClearResult['skipped'] = [];
    let freedBytes = 0;
    for (const id of [...new Set(ids)]) {
      const f = this.folders.find((x) => x.id === id);
      if (!f) skipped.push({ id, reason: 'there is no such store' });
      else if (f.state !== 'orphaned') skipped.push({ id, reason: STORE_KEPT[f.state] });
      else {
        removed.push(id);
        freedBytes += f.bytes;
      }
    }
    this.folders = this.folders.filter((f) => !removed.includes(f.id));
    return { freedBytes, removed, skipped, cancelled: false };
  }

  async policy(): Promise<StoragePolicy> {
    await this.maybeFailStores('read the clean-up setting');
    return { autoCleanDays: this.autoCleanDays };
  }

  async setPolicy(policy: StoragePolicy): Promise<StoragePolicy> {
    await this.maybeFailStores('save the clean-up setting');
    const days = policy.autoCleanDays;
    if (!Number.isInteger(days) || days < 0 || days > MAX_AUTO_CLEAN_DAYS) {
      throw `automatic clean-up must be between 0 (off) and ${MAX_AUTO_CLEAN_DAYS} days`;
    }
    this.autoCleanDays = days;
    return { autoCleanDays: days };
  }
}
