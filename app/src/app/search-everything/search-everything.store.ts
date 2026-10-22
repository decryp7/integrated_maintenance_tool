import { computed, inject } from '@angular/core';
import { patchState, signalStore, withComputed, withMethods, withState } from '@ngrx/signals';
import { SearchAllReply } from '../core/api.types';
import { CmdError, isCmdErrorDto } from '../core/cmd-error';
import { flatRows, resultGroups } from './search-everything.model';
import { SearchEverythingBackend } from './search-everything-backend';

/** Typing pause before searching (the engine also cancels a window's older search). */
export const SEARCH_DEBOUNCE_MS = 150;

interface SearchEverythingState {
  /** The text as typed. */
  text: string;
  /** The text the results are for. */
  searched: string;
  reply: SearchAllReply | null;
  loading: boolean;
  error: string | null;
}

const initial: SearchEverythingState = { text: '', searched: '', reply: null, loading: false, error: null };

function message(e: unknown): string {
  if (e instanceof CmdError || isCmdErrorDto(e)) return e.message;
  return e instanceof Error ? e.message : String(e);
}

/**
 * The search everything palette's state: the text, the engine's results for it and whether a search is running. Searches
 * after a short pause in typing; an answer to an older text is dropped (the engine cancels it too).
 */
export const SearchEverythingStore = signalStore(
  { providedIn: 'root' },
  withState<SearchEverythingState>(initial),
  withComputed((store) => {
    const groups = computed(() => resultGroups(store.reply(), store.searched()));
    return { groups, rows: computed(() => flatRows(groups())) };
  }),
  withMethods((store) => {
    const backend = inject(SearchEverythingBackend);
    let seq = 0;
    let timer: ReturnType<typeof setTimeout> | undefined;

    async function search(text: string): Promise<void> {
      const mine = ++seq;
      if (!text.trim()) {
        patchState(store, { searched: text, reply: null, loading: false, error: null });
        return;
      }
      patchState(store, { loading: true, error: null });
      try {
        const reply = await backend.searchEverything({ text });
        if (mine === seq) patchState(store, { searched: text, reply, loading: false });
      } catch (e) {
        if (mine === seq) patchState(store, { searched: text, reply: null, loading: false, error: message(e) });
      }
    }

    return {
      /** Starts afresh (the palette opened). */
      reset(): void {
        clearTimeout(timer);
        seq++;
        patchState(store, initial);
      },
      /** The text changed: searches it after {@link SEARCH_DEBOUNCE_MS}. */
      setText(text: string): void {
        patchState(store, { text });
        clearTimeout(timer);
        timer = setTimeout(() => void search(text), SEARCH_DEBOUNCE_MS);
      },
      /** Searches the current text now (Retry, or Enter before the pause ended). */
      searchNow(): Promise<void> {
        clearTimeout(timer);
        return search(store.text());
      },
    };
  }),
);
