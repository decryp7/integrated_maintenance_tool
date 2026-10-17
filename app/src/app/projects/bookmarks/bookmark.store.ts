import { inject } from '@angular/core';
import { patchState, signalStore, withMethods, withState } from '@ngrx/signals';
import { RecordView } from '../../core/api.types';
import { Bookmark } from '../../core/tools.types';
import { WorkspaceStore } from '../../core/workspace.store';
import { newId } from './ids';
import { UndoStack } from '../../undo/undo-stack';
import { recordSnapshot } from '../../undo/snapshot-undo';

interface BookmarkState {
  items: Bookmark[];
}

/** Keeps only well-formed bookmarks of untrusted saved state. */
export function parseBookmarks(value: unknown): Bookmark[] {
  if (!Array.isArray(value)) return [];
  const out: Bookmark[] = [];
  for (const raw of value as Partial<Bookmark>[]) {
    if (!raw || typeof raw.id !== 'string' || (raw.kind !== 'record' && raw.kind !== 'range')) continue;
    const base = { id: raw.id, note: typeof raw.note === 'string' ? raw.note : '', label: typeof raw.label === 'string' ? raw.label : '', createdAt: typeof raw.createdAt === 'number' ? raw.createdAt : 0 };
    if (raw.kind === 'record' && typeof raw.recordId === 'number') out.push({ ...base, kind: 'record', recordId: raw.recordId, ts: raw.ts ?? null, level: raw.level });
    if (raw.kind === 'range' && typeof raw.from === 'number' && typeof raw.to === 'number') out.push({ ...base, kind: 'range', from: raw.from, to: raw.to });
  }
  return out;
}

/** Bookmarks on records and time ranges, each with a note. Saved in the project's UI state. */
export const BookmarkStore = signalStore(
  { providedIn: 'root' },
  withState<BookmarkState>({ items: [] }),
  withMethods((store, workspace = inject(WorkspaceStore), undo = inject(UndoStack)) => {
    const state = { read: () => store.items(), write: (v: Bookmark[]) => patchState(store, { items: v }) };
    const edit = (label: string, mutate: () => void, mergeKey?: string) => recordSnapshot(undo, state, label, mutate, { mergeKey });
    return {
      /** Bookmarks the record shown in the inspector. Returns false when nothing is selected. */
      addSelected(note = ''): boolean {
        const r: RecordView | null = workspace.selected();
        if (!r) return false;
        const label = r.message.length > 80 ? `${r.message.slice(0, 80)}…` : r.message;
        const item: Bookmark = { id: newId('bm'), kind: 'record', recordId: r.id, ts: r.ts, level: r.level, label, note, createdAt: Date.now() };
        edit('new bookmark', () => patchState(store, (s) => ({ items: [...s.items, item] })));
        return true;
      },
      /** Bookmarks the time range of the current query. Returns false when no range is set. */
      addRange(note = ''): boolean {
        const { from, to } = workspace.query();
        if (from == null || to == null) return false;
        const item: Bookmark = { id: newId('bm'), kind: 'range', from, to, label: 'Time range', note, createdAt: Date.now() };
        edit('new bookmark', () => patchState(store, (s) => ({ items: [...s.items, item] })));
        return true;
      },
      setNote: (id: string, note: string) => edit('bookmark note edit', () => patchState(store, (s) => ({ items: s.items.map((b) => (b.id === id ? { ...b, note } : b)) })), `bookmark-note:${id}`),
      remove: (id: string) => edit('bookmark removal', () => patchState(store, (s) => ({ items: s.items.filter((b) => b.id !== id) }))),
      replace: (items: Bookmark[]) => patchState(store, { items }),
      /**
       * Follows records the engine renumbered (e.g. after a clock offset moved a source in time):
       * record bookmarks in `moved` get their new id; those mapped to null are kept with their old id.
       */
      remapRecords(moved: ReadonlyMap<number, number | null>) {
        if (!moved.size) return;
        patchState(store, (s) => ({
          items: s.items.map((b) => {
            const next = b.kind === 'record' && b.recordId !== undefined ? moved.get(b.recordId) : undefined;
            return next == null ? b : { ...b, recordId: next };
          }),
        }));
      },
      /** Scrolls the grid to the record, or applies the time range. */
      jump(b: Bookmark): void {
        if (b.kind === 'record' && b.recordId !== undefined) workspace.jumpTo(b.recordId);
        else if (b.from !== undefined && b.to !== undefined) workspace.setRange(b.from, b.to);
      },
    };
  }),
);
