import { TestBed } from '@angular/core/testing';
import { provideTinyBackend, settle } from '../../testing/helpers';
import { ImtBackend } from '../core/backend';
import { EMPTY_QUERY, WorkspaceStore } from '../core/workspace.store';
import { CommandRegistry } from '../extensions/command-registry';
import { UiStateRegistry } from '../lifecycle/ui-state';
import { isTextEntry } from '../shell/title-bar';
import { ProfileUndo } from './profile-undo';
import { provideUndo, REDO_COMMAND, UNDO_COMMAND, UndoCommands, undoResetContribution } from './provide-undo';
import { QueryUndo, queryChangeLabel, trackedPart } from './query-undo';
import { RangeUndo } from './range-undo';
import { recordSnapshot } from './snapshot-undo';
import { MERGE_WINDOW_MS, UNDO_LIMIT, UndoStack } from './undo-stack';

/** A profile save re-imports as a job: waits for it to finish and for the view to follow. */
async function applied(fixture?: Parameters<typeof settle>[0]): Promise<void> {
  await vi.waitFor(() => expect(TestBed.inject(WorkspaceStore).busy()).toBeNull(), { timeout: 5000 });
  await settle(fixture);
}

function stack(): UndoStack {
  TestBed.configureTestingModule({});
  return TestBed.inject(UndoStack);
}

/** A counter edited through the stack. */
function counter(undo: UndoStack) {
  let value = 0;
  const set = (label: string, to: number, mergeKey?: string, group?: string) => {
    const from = value;
    value = to;
    undo.record({ label, mergeKey, group, do: () => void (value = to), undo: () => void (value = from) });
  };
  return { get: () => value, set };
}

describe('UndoStack', () => {
  it('undoes and redoes in order and names the next step', async () => {
    const undo = stack();
    const c = counter(undo);
    expect(undo.canUndo()).toBe(false);
    expect(await undo.undo()).toBe(false);
    expect(await undo.redo()).toBe(false);
    c.set('one', 1);
    c.set('two', 2);
    expect(undo.undoLabel()).toBe('two');
    expect(undo.size()).toBe(2);
    expect(await undo.undo()).toBe(true);
    expect(c.get()).toBe(1);
    expect(undo.redoLabel()).toBe('two');
    expect(undo.canRedo()).toBe(true);
    expect(await undo.undo()).toBe(true);
    expect(c.get()).toBe(0);
    expect(await undo.redo()).toBe(true);
    expect(c.get()).toBe(1);
    // A new edit drops what could have been redone.
    c.set('three', 3);
    expect(undo.canRedo()).toBe(false);
    expect(undo.undoLabel()).toBe('three');
  });

  it('merges consecutive edits with the same key inside the window only', async () => {
    const undo = stack();
    let now = 1000;
    undo.now = () => now;
    const c = counter(undo);
    c.set('typing', 1, 'k');
    now += 100;
    c.set('typing', 2, 'k');
    expect(undo.size()).toBe(1);
    now += MERGE_WINDOW_MS + 1;
    c.set('typing', 3, 'k');
    expect(undo.size()).toBe(2);
    await undo.undo();
    expect(c.get()).toBe(2);
    await undo.undo();
    expect(c.get()).toBe(0);
    // A redone entry is not merged into by the next edit.
    await undo.redo();
    c.set('typing', 9, 'k');
    expect(undo.size()).toBe(2);
  });

  it('keeps a bounded history', () => {
    const undo = stack();
    const c = counter(undo);
    for (let i = 1; i <= UNDO_LIMIT + 5; i++) c.set(`e${i}`, i);
    expect(undo.size()).toBe(UNDO_LIMIT);
  });

  it('clears everything, or one group', async () => {
    const undo = stack();
    const c = counter(undo);
    c.set('a', 1);
    c.set('draft', 2, undefined, 'g');
    c.set('draft', 3, undefined, 'g');
    await undo.undo();
    undo.discard('g');
    expect(undo.undoLabel()).toBe('a');
    expect(undo.canRedo()).toBe(false);
    undo.clear();
    expect(undo.canUndo()).toBe(false);
  });

  it('reports failures and keeps the history where it was', async () => {
    const undo = stack();
    undo.record({ label: 'save', do: () => Promise.reject(new Error('refused')), undo: () => Promise.reject('engine down') });
    expect(await undo.undo()).toBe(false);
    expect(undo.lastError()).toBe('Could not undo save: engine down');
    expect(undo.undoLabel()).toBe('save');
    undo.record({ label: 'other', do: () => undefined, undo: () => undefined });
    await undo.undo();
    await undo.undo();
    expect(undo.lastError()).toBe('Could not undo save: engine down');
  });

  it('reports a failing redo', async () => {
    const undo = stack();
    undo.record({ label: 'x', do: () => Promise.reject(new Error('nope')), undo: () => undefined });
    await undo.undo();
    expect(await undo.redo()).toBe(false);
    expect(undo.lastError()).toBe('Could not redo x: nope');
  });

  it('executes an edit and records it only when it worked', async () => {
    const undo = stack();
    let v = 0;
    expect(await undo.execute({ label: 'set', do: () => void (v = 5), undo: () => void (v = 0) })).toBe(true);
    expect(v).toBe(5);
    expect(await undo.execute({ label: 'bad edit', do: () => Promise.reject(new Error('no')), undo: () => undefined })).toBe(false);
    expect(undo.lastError()).toBe('Bad edit failed: no');
    expect(undo.size()).toBe(1);
  });

  it('ignores what is recorded while an undo is applied, and runs one step at a time', async () => {
    const undo = stack();
    let release!: () => void;
    undo.record({
      label: 'slow',
      do: () => undefined,
      undo: () => {
        undo.record({ label: 'echo', do: () => undefined, undo: () => undefined });
        return new Promise<void>((r) => (release = r));
      },
    });
    const first = undo.undo();
    expect(undo.applying()).toBe(true);
    expect(undo.canUndo()).toBe(false);
    expect(await undo.redo()).toBe(false);
    release();
    expect(await first).toBe(true);
    expect(undo.size()).toBe(0);
    expect(undo.redoLabel()).toBe('slow');
  });
});

describe('recordSnapshot', () => {
  it('records only real changes and writes the snapshots back', async () => {
    const undo = stack();
    let state = { n: 1 };
    const target = { read: () => state, write: (v: { n: number }) => void (state = v) };
    recordSnapshot(undo, target, 'noop', () => (state = { n: 1 }));
    expect(undo.size()).toBe(0);
    recordSnapshot(undo, target, 'inc', () => (state = { n: 2 }), { mergeKey: 'm', group: 'g' });
    await undo.undo();
    expect(state).toEqual({ n: 1 });
    await undo.redo();
    expect(state).toEqual({ n: 2 });
  });
});

describe('QueryUndo', () => {
  function setup() {
    TestBed.configureTestingModule({ providers: [provideTinyBackend()] });
    const q = TestBed.inject(QueryUndo);
    q.start();
    q.start();
    return { q, store: TestBed.inject(WorkspaceStore), undo: TestBed.inject(UndoStack) };
  }

  it('labels a change by what differs first', () => {
    const base = trackedPart(EMPTY_QUERY);
    expect(queryChangeLabel(base, { ...base, filters: [{ field: 'a', op: 'eq', value: '1' }] })).toBe('field filter change');
    expect(queryChangeLabel(base, { ...base, levels: ['Error'] })).toBe('level filter change');
    expect(queryChangeLabel(base, { ...base, text: 'x' })).toBe('search text change');
    expect(queryChangeLabel(base, { ...base, regex: true })).toBe('search option change');
    expect(queryChangeLabel(base, { ...base, caseSensitive: true })).toBe('search option change');
    expect(queryChangeLabel(base, { ...base, markers: ['crash'] })).toBe('marked-only filter change');
    expect(queryChangeLabel(base, { ...base, groupBy: ['Machine'] })).toBe('group-by change');
    expect(queryChangeLabel(base, { ...base, ql: 'level >= error' })).toBe('query change');
    expect(trackedPart({ ...EMPTY_QUERY, ql: 'x' }).ql).toBe('x');
  });

  it('records filter changes, undoes them without touching the time range, and does not record its own restores', async () => {
    const { store, undo } = setup();
    await settle();
    store.toggleLevel('Error');
    await settle();
    store.addFilter({ field: 'Machine', op: 'eq', value: 'WEB01' });
    await settle();
    expect(undo.undoLabel()).toBe('field filter change');
    store.setRange(10, 20);
    await settle();
    expect(undo.size()).toBe(2);
    await undo.undo();
    await settle();
    expect(store.query().filters).toEqual([]);
    expect(store.query().levels).toEqual(['Error']);
    expect(store.query().from).toBe(10);
    expect(undo.size()).toBe(1);
    expect(undo.redoLabel()).toBe('field filter change');
    await undo.redo();
    await settle();
    expect(store.query().filters.length).toBe(1);
    expect(undo.size()).toBe(2);
  });

  it('merges typing in the search box and restores the box text', async () => {
    const { store, undo, q } = setup();
    await settle();
    q.observe({ ...EMPTY_QUERY, text: 'pay' });
    q.observe({ ...EMPTY_QUERY, text: 'payment' });
    expect(undo.size()).toBe(1);
    await undo.undo();
    expect(store.textInput()).toBe('');
    q.rebase();
    expect(undo.canRedo()).toBe(true);
  });
});

describe('ProfileUndo', () => {
  function setup() {
    TestBed.configureTestingModule({ providers: [provideTinyBackend()] });
    return { p: TestBed.inject(ProfileUndo), undo: TestBed.inject(UndoStack), backend: TestBed.inject(ImtBackend), store: TestBed.inject(WorkspaceStore) };
  }

  it('saves, then undoes and redoes through the validated save', async () => {
    const { p, undo, backend, store } = setup();
    await settle();
    const before = (await backend.getProfile()).toml;
    const after = `${before}\n# edited\n`;
    const versions = store.profileVersion();
    // The save returns the re-import job at once; the workspace follows it.
    expect(await p.save('profile edit', after)).toEqual(expect.any(Number));
    await applied();
    expect(store.profileVersion()).toBe(versions + 1);
    expect(undo.undoLabel()).toBe('profile edit');
    const save = vi.spyOn(backend, 'saveProfile');
    await undo.undo();
    expect(save).toHaveBeenLastCalledWith(before);
    expect((await backend.getProfile()).toml).toBe(before);
    await undo.redo();
    expect((await backend.getProfile()).toml).toBe(after);
  });

  it('records nothing for an unchanged profile or a refused save', async () => {
    const { p, undo } = setup();
    p.record('same', 'a', 'a');
    expect(undo.size()).toBe(0);
    await expect(p.save('bad', 'x = = 1')).rejects.toBeTruthy();
    expect(undo.size()).toBe(0);
  });

  it('reports an undo the engine refuses', async () => {
    const { p, undo } = setup();
    p.record('broken', 'x = = 1', 'name = "x"');
    expect(await undo.undo()).toBe(false);
    expect(undo.lastError()).toMatch(/^Could not undo broken/);
  });
});

describe('undo commands', () => {
  function setup() {
    TestBed.configureTestingModule({ providers: [provideTinyBackend(), provideUndo()] });
    return { registry: TestBed.inject(CommandRegistry), undo: TestBed.inject(UndoStack), store: TestBed.inject(WorkspaceStore) };
  }

  it('undo and redo with feedback, bound to Mod+Z and Mod+Shift+Z', async () => {
    const { registry, undo, store } = setup();
    await settle();
    expect(registry.find(UNDO_COMMAND)?.keybinding).toBe('Mod+Z');
    expect(registry.find(REDO_COMMAND)?.keybinding).toBe('Mod+Shift+Z');
    await registry.run(UNDO_COMMAND);
    expect(store.notice()).toBe('Nothing to undo');
    await registry.run(REDO_COMMAND);
    expect(store.notice()).toBe('Nothing to redo');
    store.toggleLevel('Error');
    await settle();
    await registry.run(UNDO_COMMAND);
    expect(store.notice()).toBe('Undone: level filter change');
    expect(store.query().levels).toEqual([]);
    await registry.run(REDO_COMMAND);
    expect(store.notice()).toBe('Redone: level filter change');
    expect(store.query().levels).toEqual(['Error']);
    // A failure reaches the error toast.
    undo.record({ label: 'x', do: () => undefined, undo: () => Promise.reject('refused') });
    await registry.run(UNDO_COMMAND);
    await settle();
    expect(store.error()).toBe('Could not undo x: refused');
  });

  it('forgets the history when saved UI state is applied, without recording the restored query', async () => {
    const { undo, store } = setup();
    await settle();
    store.toggleLevel('Error');
    await settle();
    expect(undo.canUndo()).toBe(true);
    const registry = TestBed.inject(UiStateRegistry);
    registry.restore(null);
    expect(undo.canUndo()).toBe(false);
    store.toggleLevel('Warning');
    await Promise.resolve();
    await settle();
    expect(undo.canUndo()).toBe(false);
    expect(TestBed.inject(UndoCommands)).toBeTruthy();
  });

  it('the reset contribution saves nothing and also resets on restore', async () => {
    TestBed.configureTestingModule({ providers: [provideTinyBackend()] });
    const undo = TestBed.inject(UndoStack);
    const c = undoResetContribution(undo, TestBed.inject(QueryUndo), TestBed.inject(RangeUndo));
    expect(c.capture()).toBeUndefined();
    expect(c.tracksChanges).toBe(false);
    undo.record({ label: 'x', do: () => undefined, undo: () => undefined });
    c.restore({});
    expect(undo.canUndo()).toBe(false);
  });
});

describe('isTextEntry', () => {
  it('tells text fields from other controls', () => {
    const make = (tag: string, type?: string) => {
      const e = document.createElement(tag);
      if (type) e.setAttribute('type', type);
      return e;
    };
    expect(isTextEntry(null)).toBe(false);
    expect(isTextEntry(make('textarea'))).toBe(true);
    expect(isTextEntry(make('input'))).toBe(true);
    expect(isTextEntry(make('input', 'search'))).toBe(true);
    expect(isTextEntry(make('input', 'checkbox'))).toBe(false);
    expect(isTextEntry(make('button'))).toBe(false);
    const div = make('div');
    Object.defineProperty(div, 'isContentEditable', { value: true });
    expect(isTextEntry(div)).toBe(true);
  });
});
