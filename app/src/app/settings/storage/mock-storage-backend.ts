import type { StorageBackend } from './storage-backend';
import { mockJobs, mockSteps } from '../../jobs/mock-jobs';
import { StorageCategory, StorageCategoryClearResult, StorageCategoryId, StorageClearResult, StorageUsage } from './storage.types';

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

/**
 * In-memory storage: clearing frees the clearable bytes and keeps what the engine would keep. The
 * URL switch `?mockStorageFail=N` makes the first N calls fail (error and Retry paths).
 */
export class MockStorageBackend implements StorageBackend {
  private categories = mockCategories();
  private failures: number;

  constructor(search = '') {
    const n = Number(new URLSearchParams(search).get('mockStorageFail') ?? '0');
    this.failures = Number.isInteger(n) && n > 0 ? Math.min(n, 100) : 0;
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
}
