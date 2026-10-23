import { TestBed } from '@angular/core/testing';
import { MemoryLogger } from '../../../testing/helpers';
import { mockJobs } from '../../jobs/mock-jobs';
import { Logger } from '../../core/logger';
import { MockStorageBackend, mockCategories } from './mock-storage-backend';
import { StorageBackend, TauriStorageBackend } from './storage-backend';
import { StorageClearResult } from './storage.types';

/** Starts a clear, checks it returned the job at once, and waits for the job's result. */
async function cleared(b: MockStorageBackend, ids: Parameters<MockStorageBackend['clear']>[0]): Promise<StorageClearResult> {
  const id = await b.clear(ids);
  expect(typeof id).toBe('number');
  const end = await mockJobs().wait(id);
  expect(end.state).toBe('done');
  expect(end.result?.type).toBe('storage');
  return (end.result as { type: 'storage'; result: StorageClearResult }).result;
}

describe('TauriStorageBackend', () => {
  it('maps every method to its command', async () => {
    const calls: [string, unknown][] = [];
    const call = vi.fn(async (cmd: string, args?: unknown) => {
      calls.push([cmd, args]);
      return null;
    });
    const b = new TauriStorageBackend(call as never);
    await b.usage();
    await b.clear(['webview-cache', 'logs']);
    await b.reveal('logs');
    expect(calls).toEqual([
      ['storage_usage', undefined],
      ['storage_clear', { ids: ['webview-cache', 'logs'] }],
      ['storage_reveal', { id: 'logs' }],
    ]);
  });

  it('is the mock in a browser, with failures logged', async () => {
    TestBed.configureTestingModule({ providers: [{ provide: Logger, useFactory: () => new MemoryLogger() }] });
    const backend = TestBed.inject(StorageBackend);
    expect((await backend.usage()).categories.length).toBe(8);
    await expect(backend.clear([])).rejects.toContain('choose what to clear');
    expect((TestBed.inject(Logger) as MemoryLogger).entries[0]?.props?.['Command']).toBe('clear');
  });
});

describe('MockStorageBackend', () => {
  it('lists cache and data categories with a total', async () => {
    const u = await new MockStorageBackend().usage();
    expect(u.categories.filter((c) => c.kind === 'cache').map((c) => c.id)).toEqual(['bundle-extracts', 'webview-cache', 'ai-runtimes-outdated']);
    expect(u.totalBytes).toBe(mockCategories().reduce((n, c) => n + c.bytes, 0));
  });

  it('starts the clear as a maintenance job that is running when it returns, with a message for the user', async () => {
    const b = new MockStorageBackend();
    b.clearMs = 20;
    const id = await b.clear(['logs']);
    const running = (await mockJobs().list()).jobs.find((j) => j.id === id);
    expect(running).toBeUndefined(); // not shown before the engine's 2 s grace
    const end = await mockJobs().wait(id);
    expect(end.state).toBe('done');
    expect(end.message).toMatch(/^Freed /);
  });

  it('frees what is clearable and keeps the rest with a reason', async () => {
    const b = new MockStorageBackend();
    const r = await cleared(b, ['webview-cache', 'logs', 'logs', 'stores']);
    expect(r.categories.map((c) => c.id)).toEqual(['webview-cache', 'logs', 'stores']);
    expect(r.categories[0]?.skipped[0]?.reason).toContain('in use');
    expect(r.categories[1]?.skipped[0]?.reason).toBe('Debrief is writing to it');
    expect(r.freedBytes).toBe((45 + 5) * 1024 * 1024);
    const after = await b.usage();
    expect(after.categories.find((c) => c.id === 'logs')?.clearableBytes).toBe(0);
    expect(after.categories.find((c) => c.id === 'logs')?.bytes).toBe(1024 * 1024);
    await expect(b.clear(['nope' as never])).rejects.toContain('unknown storage category');
  });

  it('reports a category that cannot be cleared', async () => {
    const b = new MockStorageBackend();
    (b as unknown as { categories: ReturnType<typeof mockCategories> }).categories[4]!.clearable = false;
    const r = await cleared(b, ['ai-models']);
    expect(r.categories[0]?.error).toBeTruthy();
  });

  it('fails the first calls with ?mockStorageFail=N, then works', async () => {
    const b = new MockStorageBackend('?mockStorageFail=2');
    await expect(b.usage()).rejects.toContain('not readable');
    await expect(b.reveal('logs')).rejects.toContain('not readable');
    expect(await b.reveal('logs')).toContain('logs');
    await expect(new MockStorageBackend().reveal('stores')).rejects.toContain('unknown');
    expect((await new MockStorageBackend('?mockStorageFail=x').usage()).categories.length).toBe(8);
  });
});
