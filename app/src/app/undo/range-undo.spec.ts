import { TestBed } from '@angular/core/testing';
import { provideTinyBackend, settle } from '../../testing/helpers';
import { WorkspaceStore } from '../core/workspace.store';
import { provideUndo } from './provide-undo';
import { RangeUndo, rangeOf } from './range-undo';
import { UndoStack } from './undo-stack';

function setup(withProvider = false) {
  TestBed.configureTestingModule({ providers: [provideTinyBackend(), ...(withProvider ? [provideUndo()] : [])] });
  const range = TestBed.inject(RangeUndo);
  const undo = TestBed.inject(UndoStack);
  if (!withProvider) {
    range.start();
    range.start();
  }
  return { range, undo, store: TestBed.inject(WorkspaceStore) };
}

describe('rangeOf', () => {
  it('turns missing ends into null', () => {
    expect(rangeOf({})).toEqual({ from: null, to: null });
    expect(rangeOf({ from: 1, to: undefined })).toEqual({ from: 1, to: null });
    expect(rangeOf({ from: 0, to: 5 })).toEqual({ from: 0, to: 5 });
  });
});

describe('RangeUndo', () => {
  it('records a zoom as one merged entry and undoes/redoes it without re-recording', async () => {
    const { undo, store } = setup();
    await settle();
    store.setRange(10, 100);
    await settle();
    store.setRange(20, 90);
    await settle();
    store.setRange(30, 80);
    await settle();
    expect(undo.size()).toBe(1);
    expect(undo.undoLabel()).toBe('zoom');
    await undo.undo();
    await settle();
    expect(rangeOf(store.query())).toEqual({ from: null, to: null });
    expect(undo.size()).toBe(0);
    expect(undo.canRedo()).toBe(true);
    await undo.redo();
    await settle();
    expect(rangeOf(store.query())).toEqual({ from: 30, to: 80 });
    expect(undo.size()).toBe(1);
  });

  it('keeps the rest of the query when restoring the range', async () => {
    const { undo, store } = setup();
    await settle();
    store.setRange(1, 2);
    await settle();
    store.toggleLevel('Error');
    await settle();
    await undo.undo();
    await settle();
    expect(store.query().levels).toEqual(['Error']);
    expect(store.query().from).toBeNull();
  });

  it('ignores a live window and an equal range', async () => {
    const { undo, store, range } = setup();
    await settle();
    store.patchQuery({ window: { kind: 'live', lastMs: 1000 }, from: 5, to: 6 });
    await settle();
    store.patchQuery({ from: 7, to: 8 });
    await settle();
    expect(undo.size()).toBe(0);
    range.observe({ from: null, to: null });
    range.observe({ from: null, to: null });
    expect(undo.size()).toBe(0);
  });

  it('the first observation is only a starting point', () => {
    const { undo, range } = setup();
    range.observe({ from: 1, to: 2 });
    expect(undo.size()).toBe(0);
    range.observe({ from: 3, to: 4 });
    expect(undo.size()).toBe(1);
  });

  it('rebase takes the current range as the new starting point without recording', async () => {
    const { undo, store, range } = setup();
    await settle();
    store.restoreQuery({ ...store.query(), from: 40, to: 50 });
    range.rebase();
    range.observe({ from: 40, to: 50 });
    expect(undo.size()).toBe(0);
    range.observe({ from: 41, to: 50 });
    expect(undo.size()).toBe(1);
  });

  it('is started by provideUndo()', async () => {
    const { undo, store } = setup(true);
    await settle();
    store.setRange(5, 6);
    await settle();
    expect(undo.undoLabel()).toBe('zoom');
  });
});
