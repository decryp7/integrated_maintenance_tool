import { TestBed } from '@angular/core/testing';
import { beforeEach, describe, expect, it } from 'vitest';
import { DumpDocSummary } from '../core/api.types';
import { CmdError } from '../core/cmd-error';
import { bySnapshotTime, DumpsStore, openNotice } from './dumps.store';
import { MOCK_DUMP_PATHS, MOCK_UMDH_PATHS, MockDumpsBackend } from './mock/mock-dumps-backend';
import { DumpsTesting, setupDumps } from './testing/dumps-test-support';

describe('DumpsStore', () => {
  let t: DumpsTesting;
  let store: InstanceType<typeof DumpsStore>;

  beforeEach(() => {
    t = setupDumps();
    store = TestBed.inject(DumpsStore);
  });

  it('opens a dump and makes it the active one', async () => {
    const opened = await store.pickAndOpen('dump');
    expect(opened).toHaveLength(1);
    expect(store.dump()?.id).toBe(opened[0]!.id);
    expect(store.dumps()).toHaveLength(1);
    expect(store.notice()).toBe('Opened 1 file.');
    expect(store.version()).toBeGreaterThan(0);
  });

  it('notes which document got its symbols, from the engine and once per change', async () => {
    expect(store.symbols()).toBeNull();
    const backend = t.backend as MockDumpsBackend;
    backend.notifySymbolsChanged(4);
    await Promise.resolve();
    expect(store.symbols()).toEqual({ id: 4, rev: 1 });
    backend.notifySymbolsChanged(4);
    expect(store.symbols()).toEqual({ id: 4, rev: 2 });
  });

  it('compares the oldest and the newest snapshot by default', async () => {
    await store.pickAndOpen('umdh');
    const logs = store.umdh();
    expect(logs).toHaveLength(3);
    expect(store.umdhTarget()).toBe(logs[2]!.id);
    expect(store.umdhBase()).toBe(logs[0]!.id);
    store.setUmdh(logs[1]!.id, logs[1]!.id);
    expect(store.umdhBase()).toBeNull();
  });

  it('collects failures, keeps the rest and retries the failed paths', async () => {
    t.picker.dumps = [MOCK_DUMP_PATHS[0]!, '/x/notes.txt'];
    await store.pickAndOpen('dump');
    expect(store.docs()).toHaveLength(1);
    expect(store.error()).toContain('neither a dump nor a UMDH log');
    expect(store.failedPaths()).toEqual(['/x/notes.txt']);
    await store.retry();
    expect(store.failedPaths()).toEqual(['/x/notes.txt']);
    store.dismiss();
    expect(store.error()).toBeNull();
    await store.retry();
    expect(store.error()).toBeNull();
    t.picker.fail = true;
    expect(await store.pickAndOpen('dump')).toEqual([]);
    expect(store.error()).toContain('the dialog failed');
  });

  it('claims only dropped dumps and UMDH logs', async () => {
    const { claimed, opened } = await store.openDropped([MOCK_DUMP_PATHS[0]!, '/x/app.log', MOCK_UMDH_PATHS[0]!]);
    expect(claimed).toEqual([MOCK_DUMP_PATHS[0], MOCK_UMDH_PATHS[0]]);
    expect(opened).toHaveLength(2);
    expect(await store.openDropped([])).toEqual({ claimed: [], opened: [] });
  });

  it('reports missing files of a restore as a notice', async () => {
    await store.restorePaths([MOCK_DUMP_PATHS[0]!, '/missing/old.dmp']);
    expect(store.error()).toBeNull();
    expect(store.notice()).toBe('Opened 1 file. 1 file was not found: old.dmp.');
  });

  it('closes one or all documents and keeps the selection valid', async () => {
    await store.pickAndOpen('dump');
    await store.pickAndOpen('umdh');
    const dump = store.dump()!.id;
    await store.close(dump);
    expect(store.dump()).toBeNull();
    await store.close(dump);
    expect(store.error()).toBeNull();
    await store.closeAll();
    expect(store.docs()).toEqual([]);
    expect(store.umdhTarget()).toBeNull();
  });

  it('shows list failures and refreshes', async () => {
    const backend = t.backend as MockDumpsBackend;
    await backend.open(MOCK_DUMP_PATHS[0]!);
    await store.refresh();
    expect(store.dumps()).toHaveLength(1);
    backend.list = () => Promise.reject(new CmdError({ code: 'internal', message: 'engine down', retryable: true }));
    await store.refresh();
    expect(store.error()).toBe('engine down');
    backend.close = () => Promise.reject(new Error('no'));
    await store.close(1);
    expect(store.error()).toBe('no');
    await store.closeAll();
    expect(store.docs()).toEqual([]);
  });
});

describe('dumps store helpers', () => {
  it('orders logs by time', () => {
    const d = (id: number, timeMs: number | null) => ({ id, timeMs }) as DumpDocSummary;
    expect(bySnapshotTime([d(2, 5), d(1, 5), d(3, 1), d(4, null)]).map((x) => x.id)).toEqual([4, 3, 1, 2]);
  });

  it('words what opened and what was missing', () => {
    expect(openNotice(0, [])).toBeNull();
    expect(openNotice(2, [])).toBe('Opened 2 files.');
    expect(openNotice(0, ['/a/x.dmp', 'C:\\b\\y.dmp'])).toBe('2 files were not found: x.dmp, y.dmp.');
  });
});
