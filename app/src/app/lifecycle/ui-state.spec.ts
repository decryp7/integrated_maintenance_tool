import { signal } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { settle, testLogger } from '../../testing/helpers';
import { EMPTY_QUERY, WorkspaceStore } from '../core/workspace.store';
import { DockService } from '../dock/dock.service';
import { FakeDock } from '../../testing/helpers';
import { provideToolsTesting } from '../tools/mock/test-support';
import { CoreUiState, sanitizeQuery } from './core-ui-state';
import { UI_STATE_CONTRIBUTIONS, UiStateRegistry } from './ui-state';

describe('UiStateRegistry', () => {
  function setup(contributions: unknown[] = []) {
    TestBed.configureTestingModule({
      providers: [...provideToolsTesting(), { provide: UI_STATE_CONTRIBUTIONS, multi: true, useValue: contributions }],
    });
    return TestBed.inject(UiStateRegistry);
  }

  it('serializes every contribution under its key with a version, and restores them', () => {
    let a = 1;
    const restored: unknown[] = [];
    const registry = setup([{ key: 'a', capture: () => a, restore: (v: unknown) => restored.push(v) }]);
    expect(registry.serialize()).toEqual({ version: 1, a: 1 });
    registry.restore({ a: 5, other: 1 });
    expect(restored).toEqual([5]);
    a = 2;
    expect(registry.serialize()['a']).toBe(2);
  });

  it('resets contributions whose key is missing, and treats junk as empty', () => {
    const reset = vi.fn();
    const restore = vi.fn();
    const registry = setup([{ key: 'a', capture: () => 1, restore, reset }, { key: 'b', capture: () => 2, restore }]);
    registry.restore({ b: 1 });
    expect(reset).toHaveBeenCalledTimes(1);
    registry.restore(null);
    registry.restore([1]);
    registry.restore('x');
    expect(reset).toHaveBeenCalledTimes(4);
    expect(restore).toHaveBeenCalledTimes(1);
  });

  it('contains failures per contribution', () => {
    const good = vi.fn();
    const registry = setup([
      { key: 'bad', capture: () => { throw new Error('nope'); }, restore: () => { throw new Error('nah'); } },
      { key: 'good', capture: () => 1, restore: good },
    ]);
    expect(registry.serialize()).toEqual({ version: 1, good: 1 });
    registry.restore({ bad: 1, good: 2 });
    expect(good).toHaveBeenCalledWith(2);
    expect(testLogger().entries.filter((e) => e.level === 'warn').map((e) => e.message)).toEqual(['UI state could not be captured', 'Saved UI state could not be restored']);
    registry.restore({ good: 1 });
  });

  it('registers at runtime (deduplicated by key) and unregisters', () => {
    const registry = setup([{ key: 'a', capture: () => 1, restore: () => undefined }]);
    const off = registry.register({ key: 'b', capture: () => 2, restore: () => undefined });
    registry.register({ key: 'a', capture: () => 99, restore: () => undefined });
    expect(registry.serialize()).toEqual({ version: 1, a: 1, b: 2 });
    off();
    expect(registry.serialize()).toEqual({ version: 1, a: 1 });
  });

  it('works without any contribution and exposes reactive snapshots', async () => {
    TestBed.configureTestingModule({ providers: provideToolsTesting() });
    const registry = TestBed.inject(UiStateRegistry);
    expect(registry.serialize()).toEqual({ version: 1 });
    expect(registry.snapshot()).toBe('{"version":1}');
  });

  it('tracked snapshot ignores transient contributions', () => {
    const transient = signal(1);
    const tracked = signal(1);
    const registry = setup([
      { key: 't', tracksChanges: false, capture: () => transient(), restore: () => undefined },
      { key: 'k', capture: () => tracked(), restore: () => undefined },
    ]);
    const before = registry.trackedSnapshot();
    transient.set(2);
    expect(registry.trackedSnapshot()).toBe(before);
    expect(JSON.parse(registry.snapshot()).t).toBe(2);
    tracked.set(2);
    expect(registry.trackedSnapshot()).not.toBe(before);
  });
});

describe('CoreUiState', () => {
  async function setup() {
    TestBed.configureTestingModule({ providers: [...provideToolsTesting(), { provide: UI_STATE_CONTRIBUTIONS, multi: true, useFactory: () => TestBed.inject(CoreUiState).contributions }] });
    await settle();
    return { registry: TestBed.inject(UiStateRegistry), store: TestBed.inject(WorkspaceStore), dock: TestBed.inject(DockService) as unknown as FakeDock };
  }

  it('captures the query, the layout and the selection', async () => {
    const { registry, store, dock } = await setup();
    store.patchQuery({ levels: ['Error'] });
    store.setTextInput('boom'); // typed, debounce not elapsed yet
    dock.openIds.set(new Set(['timeline', 'events']));
    const ui = registry.serialize();
    expect(ui.query).toMatchObject({ query: { text: 'boom', levels: ['Error'] }, textInput: 'boom' });
    expect(ui.layout).toEqual({ id: 'imt.layout.v1', openViews: ['events', 'timeline'] });
    expect(ui.selection).toEqual({ recordId: null });
  });

  it('restores the query, opens saved views and jumps to the selection', async () => {
    const { registry, store, dock } = await setup();
    registry.restore({
      query: { query: { text: 'fail', evil: 1, regex: true }, textInput: 'fail' },
      layout: { openViews: ['events', 'nope', 7] },
      selection: { recordId: 42 },
    });
    expect(store.query()).toMatchObject({ text: 'fail', regex: true, levels: [] });
    expect((store.query() as unknown as Record<string, unknown>)['evil']).toBeUndefined();
    expect(store.textInput()).toBe('fail');
    expect(dock.calls).toContain('show:events');
    expect(dock.calls).not.toContain('show:nope');
    expect(store.anchor()?.id).toBe(42);
  });

  it('falls back sensibly for partial or malformed state, and resets the query when absent', async () => {
    const { registry, store, dock } = await setup();
    store.patchQuery({ text: 'old' }); store.setTextInput('old');
    registry.restore({ query: { query: { text: 'q' } }, layout: { openViews: 'x' }, selection: { recordId: 'x' } });
    expect(store.textInput()).toBe('q');
    registry.restore({ query: null, layout: null, selection: null });
    expect(store.query()).toEqual(EMPTY_QUERY);
    store.patchQuery({ text: 'again' }); store.setTextInput('again');
    registry.restore({});
    expect(store.query()).toEqual(EMPTY_QUERY);
    expect(dock.calls.filter((c) => c.startsWith('show:'))).toEqual([]);
    expect(sanitizeQuery(null)).toEqual(EMPTY_QUERY);
    expect(sanitizeQuery('x')).toEqual(EMPTY_QUERY);
  });

  it('does not reopen views that are already open', async () => {
    const { registry, dock } = await setup();
    registry.restore({ layout: { openViews: ['timeline'] } });
    expect(dock.calls).toEqual([]);
  });
});
