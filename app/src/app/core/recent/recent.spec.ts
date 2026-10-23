import { TestBed } from '@angular/core/testing';
import { MOCK_RECENT_FILES_MAX, MOCK_RECENT_KEY, MockRecentBackend } from './mock-recent-backend';
import { parseRecentLists, RECENT_EVENT, RecentBackend, TauriRecentBackend } from './recent-backend';
import { RecentService } from './recent.service';
import { BackendRecentStore, RECENT_STORE } from '../../welcome/recent-store';
import { BackendRecentProjects, RECENT_PROJECTS } from '../../projects/recent-projects';
import { RecentLists } from '../api.types';

describe('MockRecentBackend', () => {
  beforeEach(() => localStorage.clear());

  it('keeps newest first, deduplicated and capped, and tells listeners', async () => {
    let clock = 1000;
    const b = new MockRecentBackend(MOCK_RECENT_KEY, () => clock);
    const seen: RecentLists[] = [];
    const off = await b.onChanged((l) => seen.push(l));
    await b.opened(['/a', '/b']);
    clock = 2000;
    await b.opened(['/b', '/c'], { path: '/p.debriefproj', name: 'P' });
    expect((await b.list()).files).toEqual([
      { path: '/b', openedAt: 2000 },
      { path: '/c', openedAt: 2000 },
      { path: '/a', openedAt: 1000 },
    ]);
    expect((await b.list()).projects).toEqual([{ path: '/p.debriefproj', name: 'P', openedAt: 2000 }]);
    for (let i = 0; i < 20; i++) await b.opened([`/p${i}`]);
    expect((await b.list()).files).toHaveLength(MOCK_RECENT_FILES_MAX);
    await b.remove('projects', '/p.debriefproj');
    expect((await b.list()).projects).toEqual([]);
    await b.clear('files');
    expect((await b.list()).files).toEqual([]);
    expect(seen.length).toBeGreaterThan(3);
    off();
    expect(b.calls).toEqual(['remove:projects', 'clear:files']);
  });

  it('ignores incompatible, corrupt or malformed data and storage failures', async () => {
    const b = new MockRecentBackend();
    localStorage.setItem(MOCK_RECENT_KEY, JSON.stringify({ version: 99, files: [{ path: '/x', openedAt: 1 }] }));
    expect((await b.list()).files).toEqual([]);
    localStorage.setItem(MOCK_RECENT_KEY, '{nope');
    expect((await b.list()).files).toEqual([]);
    localStorage.setItem(MOCK_RECENT_KEY, JSON.stringify({ version: 2, files: [{ path: '/ok', openedAt: 5 }, '/bare', { path: '/no-time' }], projects: [{ path: '/p', name: 'P' }, null] }));
    expect(await b.list()).toEqual({ files: [{ path: '/ok', openedAt: 5 }], projects: [], captures: [] });
    const set = vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => {
      throw new Error('quota');
    });
    await expect(b.opened(['/a'])).resolves.toBeUndefined();
    set.mockRestore();
  });
});

describe('recent captures', () => {
  it('the mock keeps captures apart from files, newest first and without duplicates, and removes and clears them', async () => {
    localStorage.clear();
    const b = new MockRecentBackend(MOCK_RECENT_KEY, () => 9);
    await b.opened(['/log.log'], undefined, ['/a.folded', '/b.nettrace']);
    await b.opened([], undefined, ['/b.nettrace']);
    const paths = async () => (await b.list()).captures.map((c) => c.path);
    expect(await paths()).toEqual(['/b.nettrace', '/a.folded']);
    expect((await b.list()).files).toEqual([{ path: '/log.log', openedAt: 9 }]);
    await b.remove('captures', '/b.nettrace');
    expect(await paths()).toEqual(['/a.folded']);
    await b.clear('captures');
    expect(await paths()).toEqual([]);
    localStorage.clear();
  });
});

describe('TauriRecentBackend', () => {
  it('calls the recent commands and parses the change event', async () => {
    const calls: [string, unknown][] = [];
    const call = (async (cmd: string, args?: unknown) => {
      calls.push([cmd, args]);
      return cmd === 'recent_list' ? { files: [{ path: '/a', openedAt: 7 }, '/old', 3], projects: [{ path: '/p', name: 'P', openedAt: 8 }, { path: '/q', name: 'Q' }], captures: [{ path: '/c.folded', openedAt: 2 }, '/old', 7] } : undefined;
    }) as never;
    let handler: ((p: unknown) => void) | undefined;
    const listen = async (event: string, h: (p: unknown) => void) => {
      expect(event).toBe(RECENT_EVENT);
      handler = h;
      return () => undefined;
    };
    const b = new TauriRecentBackend(call, listen);
    expect(await b.list()).toEqual({ files: [{ path: '/a', openedAt: 7 }], projects: [{ path: '/p', name: 'P', openedAt: 8 }], captures: [{ path: '/c.folded', openedAt: 2 }] });
    await b.remove('files', '/a');
    await b.clear('projects');
    await b.opened();
    expect(calls).toEqual([
      ['recent_list', undefined],
      ['recent_remove', { kind: 'files', path: '/a' }],
      ['recent_clear', { kind: 'projects' }],
    ]);
    const got: RecentLists[] = [];
    await b.onChanged((l) => got.push(l));
    handler?.({ files: [{ path: '/b', openedAt: 1 }] });
    expect(got).toEqual([{ files: [{ path: '/b', openedAt: 1 }], projects: [], captures: [] }]);
    expect(parseRecentLists(null)).toEqual({ files: [], projects: [], captures: [] });
  });
});

describe('RecentService and its adapters', () => {
  beforeEach(() => localStorage.clear());

  it('follows the backend and survives its failures', async () => {
    const backend = new MockRecentBackend();
    TestBed.configureTestingModule({ providers: [{ provide: RecentBackend, useValue: backend }] });
    const recent = TestBed.inject(RecentService);
    await recent.ready;
    const files = TestBed.inject(RECENT_STORE);
    const projects = TestBed.inject(RECENT_PROJECTS);
    expect(files).toBeInstanceOf(BackendRecentStore);
    expect(projects).toBeInstanceOf(BackendRecentProjects);
    files.add(['/a']);
    projects.add({ path: '/p.debriefproj', name: 'P' });
    await vi.waitFor(() => expect(files.list().map((f) => f.path)).toEqual(['/a']));
    await vi.waitFor(() => expect(projects.list()).toEqual([{ path: '/p.debriefproj', name: 'P', openedAt: expect.any(Number) }]));
    projects.remove('/p.debriefproj');
    await vi.waitFor(() => expect(projects.list()).toEqual([]));
    files.clear();
    projects.clear();
    await vi.waitFor(() => expect(files.list()).toEqual([]));
    // A failing backend leaves the lists as they are.
    const fail = () => Promise.reject(new Error('down'));
    Object.assign(backend, { list: fail, opened: fail, remove: fail, clear: fail });
    await expect(recent.opened(['/x'])).resolves.toBeUndefined();
    await expect(recent.remove('files', '/x')).resolves.toBeUndefined();
    await expect(recent.clear('files')).resolves.toBeUndefined();
    expect(recent.files()).toEqual([]);
  });
});
