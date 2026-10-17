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
    apply('theme', 'light');
    apply('theme', 'neon');
    expect(store.theme()).toBe('light');
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
});
