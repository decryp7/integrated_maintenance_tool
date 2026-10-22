import { computed, inject } from '@angular/core';
import { patchState, signalStore, withComputed, withMethods, withState } from '@ngrx/signals';
import { NoteImage, NoteItem } from '../../core/api.types';
import { failureText } from '../../core/cmd-error';
import { NotesBackend } from '../notes-backend';
import { NotesStore } from '../notes.store';

interface ImageState {
  image: NoteImage | null;
  opening: boolean;
  /** Why the last open failed (shown inline with Retry). */
  error: string | null;
  /** The note whose regions are highlighted. */
  focused: string | null;
}

/**
 * The image view's state (ADR 0021 §4): the image the engine decoded (its clean copy as a `data:` URL and the
 * original's size, which regions are measured in), the notes on it and the highlighted one.
 */
export const NoteImageStore = signalStore(
  { providedIn: 'root' },
  withState<ImageState>({ image: null, opening: false, error: null, focused: null }),
  withComputed((s, notes = inject(NotesStore)) => ({
    /** The notes on the shown image. */
    notes: computed<NoteItem[]>(() => {
      const key = s.image()?.key;
      return key ? notes.items().filter((n) => n.anchor.kind === 'image' && n.anchor.image.key === key) : [];
    }),
  })),
  withMethods((store, backend = inject(NotesBackend)) => {
    let retry: (() => Promise<boolean>) | null = null;
    const open = async (load: () => Promise<NoteImage>, focused: string | null): Promise<boolean> => {
      retry = () => open(load, focused);
      patchState(store, { opening: true, error: null });
      try {
        patchState(store, { image: await load(), opening: false, focused });
        return true;
      } catch (e) {
        patchState(store, { opening: false, error: failureText(e) });
        return false;
      }
    };
    return {
      /** Opens an image file of the workspace (or one the user chose). */
      showPath: (path: string) => open(() => backend.openNoteImage(path), null),
      /** Shows the image notes mark by `key` (opening it again when needed), highlighting note `focused`. */
      showKey(key: string, focused: string | null = null): Promise<boolean> {
        if (store.image()?.key === key) {
          patchState(store, { focused });
          return Promise.resolve(true);
        }
        return open(() => backend.getNoteImage(key), focused);
      },
      focus: (id: string | null) => patchState(store, { focused: id }),
      retry: () => (retry ? retry() : Promise.resolve(false)),
      dismissError: () => patchState(store, { error: null }),
      close: () => patchState(store, { image: null, focused: null, error: null }),
    };
  }),
);
