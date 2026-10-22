import { MockStorageBackend, mockStoreFolders } from './mock-storage-backend';
import { TauriStorageBackend } from './storage-backend';

describe('TauriStorageBackend (stores)', () => {
  it('maps the store methods to their commands', async () => {
    const calls: [string, unknown][] = [];
    const call = vi.fn(async (cmd: string, args?: unknown) => {
      calls.push([cmd, args]);
      return null;
    });
    const b = new TauriStorageBackend(call as never);
    await b.stores();
    await b.clearStores(['store-a', 'store-b']);
    await b.policy();
    await b.setPolicy({ autoCleanDays: 30 });
    expect(calls).toEqual([
      ['storage_stores', undefined],
      ['storage_stores_clear', { ids: ['store-a', 'store-b'] }],
      ['storage_policy', undefined],
      ['storage_set_policy', { policy: { autoCleanDays: 30 } }],
    ]);
  });
});

describe('MockStorageBackend (stores)', () => {
  it('lists five folders newest first: open, session, running and two orphaned', async () => {
    const s = await new MockStorageBackend().stores();
    expect(s.folders.map((f) => f.state)).toEqual(['open', 'session', 'running', 'orphaned', 'orphaned']);
    const times = s.folders.map((f) => f.lastUsedMs ?? 0);
    expect([...times].sort((a, b) => b - a)).toEqual(times);
    expect(s.totalBytes).toBe(s.folders.reduce((n, f) => n + f.bytes, 0));
    expect(s.orphanedBytes).toBe(s.folders.filter((f) => f.state === 'orphaned').reduce((n, f) => n + f.bytes, 0));
    expect(s.capped).toBe(false);
    expect(mockStoreFolders(0)[0]?.lastUsedMs).toBe(0);
  });

  it('removes only the orphaned ones among the ids and says why it kept the others', async () => {
    const b = new MockStorageBackend();
    const before = await b.stores();
    const open = before.folders.find((f) => f.state === 'open')!;
    const session = before.folders.find((f) => f.state === 'session')!;
    const running = before.folders.find((f) => f.state === 'running')!;
    const [o1, o2] = before.folders.filter((f) => f.state === 'orphaned');
    const result = await b.clearStores([o1!.id, open.id, session.id, running.id, 'nope', o1!.id]);
    expect(result.removed).toEqual([o1!.id]);
    expect(result.freedBytes).toBe(o1!.bytes);
    expect(result.cancelled).toBe(false);
    expect(result.skipped).toEqual([
      { id: open.id, reason: 'a window has this store open' },
      { id: session.id, reason: 'an autosaved session reopens it' },
      { id: running.id, reason: 'another IMT process is using it' },
      { id: 'nope', reason: 'there is no such store' },
    ]);
    const after = await b.stores();
    expect(after.folders.map((f) => f.id)).not.toContain(o1!.id);
    expect(after.folders.map((f) => f.id)).toContain(o2!.id);
  });

  it('refuses an empty clean-up', async () => {
    await expect(new MockStorageBackend().clearStores([])).rejects.toContain('choose');
  });

  it('keeps the policy in memory and validates 0..=365', async () => {
    const b = new MockStorageBackend();
    expect(await b.policy()).toEqual({ autoCleanDays: 0 });
    expect(await b.setPolicy({ autoCleanDays: 365 })).toEqual({ autoCleanDays: 365 });
    expect(await b.policy()).toEqual({ autoCleanDays: 365 });
    for (const bad of [-1, 366, 1.5, Number.NaN]) await expect(b.setPolicy({ autoCleanDays: bad })).rejects.toContain('between 0');
    expect(await b.policy()).toEqual({ autoCleanDays: 365 });
  });

  it('?mockStoresFail=N fails the first N store calls', async () => {
    const b = new MockStorageBackend('?mockStoresFail=4');
    await expect(b.stores()).rejects.toContain('list the project stores');
    await expect(b.clearStores(['x'])).rejects.toContain('clean up');
    await expect(b.policy()).rejects.toContain('read');
    await expect(b.setPolicy({ autoCleanDays: 1 })).rejects.toContain('save');
    expect((await b.stores()).folders).toHaveLength(5);
    // The category calls keep their own switch.
    expect((await b.usage()).categories).toHaveLength(8);
  });
});
