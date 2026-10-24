import { Injector } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { provideTinyBackend, query, settle } from '../../testing/helpers';
import { TimeCursor } from '../analysis/time-cursor';
import { WorkspaceStore } from '../core/workspace.store';
import { LiveFollow } from '../live/live-follow.store';
import { SettingsStore } from '../settings/settings.store';
import { LiveTail } from '../workspace/live-tail';
import { sharedQuery, sharedRange, sharedRecord, sharedUiState } from './shared-state';

describe('shared UI state of a window group', () => {
  it('parses values from other windows', () => {
    expect(sharedQuery({ ...query({ text: 'x' }), evil: 1, ql: 'level >= error', window: { kind: 'live', lastMs: 5 } })).toEqual({
      ...query({ text: 'x' }),
      ql: 'level >= error',
      window: { kind: 'live', lastMs: 5 },
    });
    expect(sharedQuery('nope')).toEqual(query());
    expect(sharedRecord({ id: 3, message: 'm' })).toEqual({ id: 3, message: 'm' });
    expect(sharedRecord({ id: '3', message: 'm' })).toBeNull();
    expect(sharedRecord(null)).toBeNull();
    expect(sharedRange({ from: 1, to: 2 })).toEqual({ from: 1, to: 2 });
    expect(sharedRange({ from: 1 })).toBeNull();
  });

  it('reads and applies the query, selection, jumps, theme, settings, cursor, follow and host-only state', async () => {
    TestBed.configureTestingModule({ providers: provideTinyBackend() });
    const values = new Map(sharedUiState(TestBed.inject(Injector)).map((v) => [v.key, v]));
    const store = TestBed.inject(WorkspaceStore);
    await settle();
    const apply = (key: string, v: unknown) => values.get(key)!.apply(v);
    const read = (key: string) => values.get(key)!.read();

    apply('query', query({ text: 'boom' }));
    expect(store.query().text).toBe('boom');
    expect(store.textInput()).toBe('boom');
    apply('selected', { id: 1, message: 'hello' });
    expect(read('selected')).toMatchObject({ id: 1 });
    apply('jump', { id: 7, nonce: 99 });
    expect(store.anchor()!.id).toBe(7);
    apply('jump', null);
    apply('scheme', 'light');
    apply('scheme', 'neon');
    expect(store.scheme()).toBe('light');
    apply('theme', 'console');
    apply('theme', 'neon');
    expect(store.theme()).toBe('console');
    apply('theme', 'precision');
    expect(store.theme()).toBe('precision');
    apply('theme', null);
    expect(store.theme()).toBeNull();
    apply('uiFont', 'theme');
    apply('uiFont', 'comic');
    expect(read('uiFont')).toBe('theme');
    apply('timezone', 'local');
    expect(read('timezone')).toBe('local');
    apply('lifecycle', false);
    expect(store.alwaysShowLifecycle()).toBe(false);
    apply('sort', true);
    apply('sort', true);
    expect(store.descending()).toBe(true);
    store.addColumn('old');
    apply('columns', ['a', 'b', 3]);
    expect(store.extraColumns()).toEqual(['a', 'b']);
    apply('columns', 'x');
    apply('cursor', 123);
    expect(TestBed.inject(TimeCursor).hovered()).toBe(123);
    apply('cursor', 'x');
    expect(TestBed.inject(TimeCursor).hovered()).toBeNull();
    apply('range', { from: 5, to: 1 });
    expect(TestBed.inject(TimeCursor).selection()).toEqual({ from: 1, to: 5 });
    apply('follow', { follow: false, preset: '5m', customMinutes: 30, paused: true });
    const follow = TestBed.inject(LiveFollow);
    expect([follow.follow(), follow.preset(), follow.paused()]).toEqual([false, '5m', true]);
    expect(read('follow')).toMatchObject({ preset: '5m' });
    apply('dateFormat', 'iso');
    apply('dateFormat', 'weird');
    apply('density', 'compact');
    apply('density', 'huge');
    const settings = TestBed.inject(SettingsStore);
    expect([settings.dateFormat(), settings.density()]).toEqual(['iso', 'compact']);
    expect(values.get('live')!.direction).toBe('fromHost');
    const live = TestBed.inject(LiveTail);
    const restore = vi.spyOn(live, 'restore').mockResolvedValue();
    apply('live', true);
    expect(restore).toHaveBeenCalled();
    live.enabled.set(true);
    apply('live', false);
    expect(live.enabled()).toBe(false);
    const version = store.dataVersion();
    apply('data', 5);
    await settle();
    expect(store.dataVersion()).toBe(version + 1);
    expect(typeof read('data')).toBe('number');
  });

  describe('a guest that shows less creates fewer stores (Paddock rules 32/34)', () => {
    /** The tokens `sharedUiState` asks the injector for, in order. */
    function asked(scope: 'full' | 'summary' | 'display'): { keys: string[]; tokens: unknown[] } {
      TestBed.configureTestingModule({ providers: provideTinyBackend() });
      const real = TestBed.inject(Injector);
      const tokens: unknown[] = [];
      const spy = { get: (t: unknown) => (tokens.push(t), real.get(t as never)) } as unknown as Injector;
      return { keys: sharedUiState(spy, scope).map((v) => v.key), tokens };
    }

    it('display: only the preference stores, no workspace, cursor, follow or live tail', () => {
      const { keys, tokens } = asked('display');
      expect(keys).toEqual(['theme', 'scheme', 'uiFont', 'timezone', 'lifecycle', 'dateFormat', 'density', 'sourceColors']);
      for (const heavy of [WorkspaceStore, TimeCursor, LiveFollow, LiveTail]) expect(tokens).not.toContain(heavy);
    });

    it('summary: adds the data version through the data store, still no workspace facade', () => {
      const { keys, tokens } = asked('summary');
      expect(keys).toEqual(['theme', 'scheme', 'uiFont', 'timezone', 'lifecycle', 'dateFormat', 'density', 'sourceColors', 'data']);
      for (const heavy of [WorkspaceStore, TimeCursor, LiveFollow, LiveTail]) expect(tokens).not.toContain(heavy);
    });

    it('full: everything, as a pop-out and the host need', () => {
      const { keys, tokens } = asked('full');
      expect(keys).toEqual(expect.arrayContaining(['query', 'selected', 'jump', 'sort', 'columns', 'cursor', 'range', 'follow', 'live', 'data', 'theme']));
      for (const heavy of [WorkspaceStore, TimeCursor, LiveFollow, LiveTail]) expect(tokens).toContain(heavy);
    });
  });
});
