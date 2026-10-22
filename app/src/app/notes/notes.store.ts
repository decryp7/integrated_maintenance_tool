import { computed, effect, inject, untracked } from '@angular/core';
import { patchState, signalStore, withComputed, withHooks, withMethods, withState } from '@ngrx/signals';
import { Note, NoteChange, NoteDraft, NoteItem, NotesList } from '../core/api.types';
import { failureText } from '../core/cmd-error';
import { WorkspaceStore } from '../core/workspace.store';
import { allTags, byEvent, bySource, matches, NO_FILTER, NoteFilter } from './notes-model';
import { NotesBackend } from './notes-backend';
import { NotesPersistence } from './notes-persistence';

interface NotesState {
  list: NotesList | null;
  loading: boolean;
  /** Listing failed (shown with Retry). */
  loadError: string | null;
  /** The last change failed. */
  actionError: string | null;
  busy: boolean;
  filter: NoteFilter;
}

const INITIAL: NotesState = { list: null, loading: false, loadError: null, actionError: null, busy: false, filter: NO_FILTER };

/**
 * The window's notes as the engine lists them (ADR 0021), the Notes panel's filter, and the outcome of changes. The
 * engine checks every change; after one the saved record is refreshed (the project becomes unsaved) and the list is
 * read again. The list is read again too when saved notes were restored or the workspace changed (anchors may
 * resolve differently).
 */
export const NotesStore = signalStore(
  { providedIn: 'root' },
  withState<NotesState>(INITIAL),
  withComputed((store) => {
    const items = computed<NoteItem[]>(() => store.list()?.notes ?? []);
    return {
      items,
      filtered: computed(() => items().filter((n) => matches(n, store.filter()))),
      tags: computed(() => allTags(items())),
      byEvent: computed(() => byEvent(items())),
      bySource: computed(() => bySource(items())),
      maxTextBytes: computed(() => store.list()?.maxTextBytes ?? 16 * 1024),
      maxRegions: computed(() => store.list()?.maxRegions ?? 16),
    };
  }),
  withMethods((store, backend = inject(NotesBackend), persistence = inject(NotesPersistence)) => {
    const load = async (): Promise<void> => {
      patchState(store, { loading: true, loadError: null });
      try {
        patchState(store, { list: await backend.listNotes() });
      } catch (e) {
        patchState(store, { loadError: failureText(e) });
      } finally {
        patchState(store, { loading: false });
      }
    };
    /** Runs a change, then lists again; resolves with its result, or null when it failed (the error is shown). */
    const change = async <T>(run: () => Promise<T>): Promise<T | null> => {
      patchState(store, { busy: true, actionError: null });
      try {
        const r = await run();
        await persistence.sync();
        await load();
        return r;
      } catch (e) {
        patchState(store, { actionError: failureText(e) });
        return null;
      } finally {
        patchState(store, { busy: false });
      }
    };
    return {
      load,
      add: (draft: NoteDraft): Promise<Note | null> => change(() => backend.addNote(draft)),
      update: (id: string, c: NoteChange): Promise<Note | null> => change(() => backend.updateNote(id, c)),
      remove: (id: string): Promise<boolean | null> => change(() => backend.removeNote(id)),
      get: (id: string): NoteItem | undefined => store.list()?.notes.find((n) => n.id === id),
      setFilter: (patch: Partial<NoteFilter>) => patchState(store, (s) => ({ filter: { ...s.filter, ...patch } })),
      clearFilter: () => patchState(store, { filter: NO_FILTER }),
      dismissError: () => patchState(store, { actionError: null }),
    };
  }),
  withHooks({
    onInit(store, persistence = inject(NotesPersistence), workspace = inject(WorkspaceStore)) {
      // Saved notes restored, or the workspace changed: list again (anchors may resolve differently).
      effect(() => {
        persistence.restored();
        workspace.summary();
        untracked(() => void store.load());
      });
    },
  }),
);
