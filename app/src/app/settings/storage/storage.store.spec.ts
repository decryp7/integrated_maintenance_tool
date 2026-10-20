import { TestBed } from '@angular/core/testing';
import { MockStorageBackend } from './mock-storage-backend';
import { JobsBackend } from '../../jobs/jobs-backend';
import { JobOutcome } from '../../jobs/jobs.types';
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
    let finish!: (id: number) => void;
    const backend = new MockStorageBackend();
    const clear = vi.spyOn(backend, 'clear').mockImplementation(() => new Promise((r) => (finish = r)));
    const { store } = setup(backend);
    await store.clear([]);
    expect(clear).not.toHaveBeenCalled();
    const first = store.clear(['logs']);
    expect(store.busy()).toBe(true);
    await store.clear(['sessions']);
    expect(clear).toHaveBeenCalledTimes(1);
    finish(0);
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

  describe('as a background job', () => {
    const result: StorageClearResult = { freedBytes: 5, categories: [], cancelled: false };
    const outcome = (state: JobOutcome['state'], extra: Partial<JobOutcome> = {}): JobOutcome => ({ id: 9, state, result: null, error: null, message: null, ...extra });

    function jobSetup() {
      const backend = new MockStorageBackend();
      const clear = vi.spyOn(backend, 'clear').mockResolvedValue(9);
      let end!: (o: JobOutcome) => void;
      const jobs = { wait: vi.fn(() => new Promise<JobOutcome>((r) => (end = r))), unwait: vi.fn(async () => undefined), cancel: vi.fn(async () => undefined) };
      TestBed.configureTestingModule({
        providers: [
          { provide: StorageBackend, useValue: backend },
          { provide: JobsBackend, useValue: jobs },
        ],
      });
      return { store: TestBed.inject(StorageStore), backend, clear, jobs, end: (o: JobOutcome) => end(o) };
    }

    it('follows the job the clear started and shows its result when it is done', async () => {
      const { store, jobs, end } = jobSetup();
      const run = store.clear(['logs']);
      await Promise.resolve();
      await Promise.resolve();
      expect(store.job()).toBe(9);
      expect(store.clearing()).toEqual(['logs']);
      expect(jobs.wait).toHaveBeenCalledWith(9);
      end(outcome('done', { result: { type: 'storage', result } }));
      await run;
      expect(store.result()).toEqual(result);
      expect(store.job()).toBeNull();
      expect(store.clearing()).toBeNull();
    });

    it('detach stops following: the job goes on and the late outcome is ignored', async () => {
      const { store, jobs, end } = jobSetup();
      store.detach(); // nothing to detach from
      expect(jobs.unwait).not.toHaveBeenCalled();
      const run = store.clear(['logs']);
      await Promise.resolve();
      await Promise.resolve();
      store.detach();
      expect(jobs.unwait).toHaveBeenCalledWith(9);
      expect(store.clearing()).toBeNull();
      expect(store.job()).toBeNull();
      end(outcome('interrupted'));
      await run;
      expect(store.result()).toBeNull();
      expect(store.actionError()).toBeNull();
    });

    it('a failed job shows its reason with Retry; a cancelled one shows nothing', async () => {
      const { store, jobs, end, clear } = jobSetup();
      const run = store.clear(['logs']);
      await Promise.resolve();
      await Promise.resolve();
      end(outcome('failed', { error: 'the disk is locked' }));
      await run;
      expect(store.actionError()).toBe('the disk is locked');
      jobs.wait.mockResolvedValueOnce(outcome('cancelled'));
      await store.retry();
      expect(clear).toHaveBeenCalledTimes(2);
      expect(store.result()).toBeNull();
      expect(store.actionError()).toBeNull();
      jobs.wait.mockResolvedValueOnce(outcome('failed'));
      await store.clear(['logs']);
      expect(store.actionError()).toBe('Clearing failed.');
    });

    it('showResult (View of the job message) shows what was cleared and measures again', async () => {
      const { store, backend } = jobSetup();
      const usage = vi.spyOn(backend, 'usage');
      store.showResult(result);
      expect(store.result()).toEqual(result);
      await vi.waitFor(() => expect(usage).toHaveBeenCalled());
    });

    it('with the mock jobs the clear returns at once and the result arrives from the job', async () => {
      TestBed.resetTestingModule();
      const backend = new MockStorageBackend();
      const { store } = setup(backend);
      await store.load();
      const run = store.clear(['logs']);
      expect(store.busy()).toBe(true);
      await run;
      expect(store.result()?.categories[0]?.id).toBe('logs');
    });
  });
});
