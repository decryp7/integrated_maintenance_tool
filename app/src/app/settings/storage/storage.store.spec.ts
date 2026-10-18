import { TestBed } from '@angular/core/testing';
import { MockStorageBackend } from './mock-storage-backend';
import { StorageBackend } from './storage-backend';
import { StorageStore } from './storage.store';
import { StorageClearResult } from './storage.types';

function setup(backend: StorageBackend = new MockStorageBackend()) {
  TestBed.configureTestingModule({ providers: [{ provide: StorageBackend, useValue: backend }] });
  return { store: TestBed.inject(StorageStore), backend };
}

describe('StorageStore', () => {
  it('loads the usage and derives what "Clear cache" removes', async () => {
    const { store } = setup();
    expect(store.categories()).toEqual([]);
    await store.load();
    expect(store.loading()).toBe(false);
    expect(store.categories()).toHaveLength(8);
    expect(store.cache().map((c) => c.id)).toEqual(['bundle-extracts', 'webview-cache', 'ai-runtimes-outdated']);
    expect(store.cacheBytes()).toBe((12 + 45 + 210) * 1024 * 1024);
  });

  it('shows a failed measurement and recovers on retry', async () => {
    const { store } = setup(new MockStorageBackend('?mockStorageFail=1'));
    await store.load();
    expect(store.loadError()).toContain('not readable');
    expect(store.usage()).toBeNull();
    await store.load();
    expect(store.loadError()).toBeNull();
    expect(store.usage()).not.toBeNull();
  });

  it('clears the cache categories, keeps the result and measures again', async () => {
    const backend = new MockStorageBackend();
    const clear = vi.spyOn(backend, 'clear');
    const { store } = setup(backend);
    await store.load();
    await store.clearCache();
    expect(clear).toHaveBeenCalledWith(['bundle-extracts', 'webview-cache', 'ai-runtimes-outdated']);
    expect(store.result()?.freedBytes).toBe((12 + 45 + 210) * 1024 * 1024);
    expect(store.clearing()).toBeNull();
    expect(store.cacheBytes()).toBe(0);
    store.dismiss();
    expect(store.result()).toBeNull();
  });

  it('ignores an empty or overlapping clear', async () => {
    let finish!: (r: StorageClearResult) => void;
    const backend = new MockStorageBackend();
    const clear = vi.spyOn(backend, 'clear').mockImplementation(() => new Promise((r) => (finish = r)));
    const { store } = setup(backend);
    await store.clear([]);
    expect(clear).not.toHaveBeenCalled();
    const first = store.clear(['logs']);
    expect(store.busy()).toBe(true);
    await store.clear(['sessions']);
    expect(clear).toHaveBeenCalledTimes(1);
    finish({ freedBytes: 0, categories: [], cancelled: false });
    await first;
    expect(store.busy()).toBe(false);
  });

  it('retries a failed clear with the same categories', async () => {
    const backend = new MockStorageBackend();
    const clear = vi.spyOn(backend, 'clear').mockRejectedValueOnce('the folder is locked');
    const { store } = setup(backend);
    await store.clear(['logs']);
    expect(store.actionError()).toBe('the folder is locked');
    await store.retry();
    expect(clear).toHaveBeenNthCalledWith(2, ['logs']);
    expect(store.actionError()).toBeNull();
    expect(store.result()?.categories[0]?.id).toBe('logs');
  });

  it('reveals a folder, retrying after a failure', async () => {
    const { store } = setup(new MockStorageBackend('?mockStorageFail=1'));
    await store.reveal('logs');
    expect(store.actionError()).toContain('not readable');
    await store.retry();
    expect(store.revealed()).toContain('logs');
    expect(store.actionError()).toBeNull();
    // Nothing to retry is a no-op.
    store.dismiss();
    await store.retry();
    expect(store.revealed()).toBeNull();
  });
});
