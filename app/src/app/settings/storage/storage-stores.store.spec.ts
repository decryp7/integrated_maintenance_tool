import { TestBed } from '@angular/core/testing';
import { MockStorageBackend } from './mock-storage-backend';
import { StorageBackend } from './storage-backend';
import { StorageStore } from './storage.store';

function setup(backend: StorageBackend = new MockStorageBackend()) {
  TestBed.configureTestingModule({ providers: [{ provide: StorageBackend, useValue: backend }] });
  return { store: TestBed.inject(StorageStore), backend };
}

describe('StorageStore (project stores)', () => {
  it('loads the folders and derives the orphaned ones', async () => {
    const { store } = setup();
    expect(store.folders()).toEqual([]);
    await store.loadStores();
    expect(store.folders()).toHaveLength(5);
    expect(store.orphanedFolders().map((f) => f.state)).toEqual(['orphaned', 'orphaned']);
    expect(store.storesLoading()).toBe(false);
  });

  it('shows a failed listing and recovers on retry', async () => {
    const { store } = setup(new MockStorageBackend('?mockStoresFail=1'));
    await store.loadStores();
    expect(store.storesError()).toContain('not readable');
    expect(store.stores()).toBeNull();
    await store.loadStores();
    expect(store.storesError()).toBeNull();
    expect(store.stores()).not.toBeNull();
  });

  it('lets only orphaned stores be ticked, and sums what that frees', async () => {
    const { store } = setup();
    await store.loadStores();
    const [o1, o2] = store.orphanedFolders();
    const open = store.folders().find((f) => f.state === 'open')!;
    store.toggleStore(open.id, true);
    expect(store.selected()).toEqual([]);
    store.toggleStore(o1!.id, true);
    store.toggleStore(o1!.id, true);
    store.toggleStore(o2!.id, true);
    expect(store.selected()).toEqual([o1!.id, o2!.id]);
    expect(store.selectedBytes()).toBe(o1!.bytes + o2!.bytes);
    store.toggleStore(o1!.id, false);
    expect(store.selected()).toEqual([o2!.id]);
  });

  it('cleans up the selection, keeps the result and lists again without the removed store', async () => {
    const { store, backend } = setup();
    const clear = vi.spyOn(backend, 'clearStores');
    await store.loadStores();
    const o1 = store.orphanedFolders()[0]!;
    store.toggleStore(o1.id, true);
    await store.clearSelectedStores();
    expect(clear).toHaveBeenCalledWith([o1.id]);
    expect(store.storesResult()?.removed).toEqual([o1.id]);
    expect(store.folders().map((f) => f.id)).not.toContain(o1.id);
    expect(store.selected()).toEqual([]);
    expect(store.storesClearing()).toBe(false);
    store.dismissStores();
    expect(store.storesResult()).toBeNull();
  });

  it('cleans up all orphaned stores', async () => {
    const { store } = setup();
    await store.loadStores();
    await store.clearAllOrphaned();
    expect(store.orphanedFolders()).toEqual([]);
    expect(store.folders()).toHaveLength(3);
    expect(store.storesResult()?.removed).toHaveLength(2);
  });

  it('ignores an empty clean-up and keeps a failure as an inline error', async () => {
    const backend = new MockStorageBackend();
    const clear = vi.spyOn(backend, 'clearStores');
    const { store } = setup(backend);
    await store.loadStores();
    await store.clearSelectedStores();
    expect(clear).not.toHaveBeenCalled();
    clear.mockRejectedValueOnce('the store folder is locked');
    await store.clearAllOrphaned();
    expect(store.storesActionError()).toBe('the store folder is locked');
    expect(store.storesResult()).toBeNull();
    store.dismissStores();
    expect(store.storesActionError()).toBeNull();
  });

  it('reads and saves the policy; a refusal is kept as the message', async () => {
    const { store } = setup();
    await store.loadPolicy();
    expect(store.policy()).toEqual({ autoCleanDays: 0 });
    await store.setAutoCleanDays(30);
    expect(store.policy()).toEqual({ autoCleanDays: 30 });
    await store.setAutoCleanDays(999);
    expect(store.policyError()).toContain('between 0');
    expect(store.policy()).toEqual({ autoCleanDays: 30 });
    store.setPolicyError('x');
    expect(store.policyError()).toBe('x');
    await store.setAutoCleanDays(0);
    expect(store.policyError()).toBeNull();
  });

  it('shows a policy that cannot be read', async () => {
    const { store } = setup(new MockStorageBackend('?mockStoresFail=1'));
    await store.loadPolicy();
    expect(store.policyError()).toContain('read');
    expect(store.policy()).toBeNull();
  });
});
