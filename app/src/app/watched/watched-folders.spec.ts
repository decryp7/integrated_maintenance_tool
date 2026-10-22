import { TestBed } from '@angular/core/testing';
import { settle, all, el, text } from '../../testing/helpers';
import { ImtBackend } from '../core/backend';
import { MockBackend } from '../core/mock/mock-backend';
import { Platform } from '../core/platform';
import { WorkspaceStore } from '../core/workspace.store';
import { parseWatchedFolders, WatchedFoldersStore } from './watched-folders.store';
import { WatchedFoldersView } from './watched-folders-view';

function setup(picked: string[] = ['/logs/incoming']) {
  const backend = new MockBackend({ latency: false });
  TestBed.configureTestingModule({
    providers: [
      { provide: ImtBackend, useValue: backend },
      { provide: Platform, useValue: { pickPaths: async () => picked, isTauri: () => false } },
    ],
  });
  return { backend, store: TestBed.inject(WatchedFoldersStore), workspace: TestBed.inject(WorkspaceStore) };
}

describe('parseWatchedFolders', () => {
  it('keeps well-formed entries, defaults enabled and caps the list', () => {
    expect(parseWatchedFolders('x')).toEqual([]);
    expect(parseWatchedFolders([{ path: '/a' }, { path: '/b', enabled: false }, { path: 3 }, null, { path: '' }])).toEqual([
      { path: '/a', enabled: true },
      { path: '/b', enabled: false },
    ]);
    expect(parseWatchedFolders(Array.from({ length: 20 }, (_, i) => ({ path: `/f${i}` })))).toHaveLength(8);
  });
});

describe('WatchedFoldersStore', () => {
  it('adds, pauses and removes folders and refreshes the workspace when the engine imported', async () => {
    const { store, workspace } = setup();
    await settle(undefined, 3);
    await store.add('/logs/incoming');
    expect(store.folders().map((f) => f.path)).toEqual(['/logs/incoming']);
    expect(store.saved()).toEqual([{ path: '/logs/incoming', enabled: true }]);
    const before = workspace.summary()?.sources.length ?? 0;
    await new Promise((r) => setTimeout(r, 0));
    await store.imported();
    await store.setEnabled('/logs/incoming', false);
    expect(store.folders()[0]?.enabled).toBe(false);
    await store.remove('/logs/incoming');
    expect(store.folders()).toEqual([]);
    expect(workspace.summary()?.sources.length ?? 0).toBeGreaterThanOrEqual(before);
  });

  it('shows the engine refusal of a ninth folder', async () => {
    const { store } = setup();
    for (let i = 0; i < 8; i++) await store.add(`/f${i}`);
    await store.add('/f8');
    expect(store.error()).toContain('at most 8');
    expect(store.folders()).toHaveLength(8);
  });

  it('applies a saved list: adds what is listed, pauses what was paused and drops the rest', async () => {
    const { store } = setup();
    await store.add('/old');
    await store.replace([{ path: '/a', enabled: true }, { path: '/b', enabled: false }]);
    expect(store.folders().map((f) => [f.path, f.enabled])).toEqual([['/a', true], ['/b', false]]);
  });
});

describe('WatchedFoldersView', () => {
  it('lists folders, adds the picked one and removes it again', async () => {
    setup(['/logs/incoming']);
    const fixture = TestBed.createComponent(WatchedFoldersView);
    fixture.detectChanges();
    expect(text(fixture)).toContain('No folders are watched.');
    el<HTMLButtonElement>(fixture, '[data-testid=watch-add]').click();
    await settle(fixture, 4);
    expect(all(fixture, '.item')).toHaveLength(1);
    expect(text(fixture)).toContain('/logs/incoming');
    expect(text(fixture)).toContain('1 file imported');
    const box = el<HTMLInputElement>(fixture, '.item input');
    expect(box.getAttribute('aria-label')).toBe('Watch the folder incoming');
    el<HTMLButtonElement>(fixture, '.item .icon-btn').click();
    await settle(fixture, 4);
    expect(text(fixture)).toContain('No folders are watched.');
  });
});
