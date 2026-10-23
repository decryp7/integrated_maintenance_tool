import { computed, inject } from '@angular/core';
import { patchState, signalStore, withComputed, withMethods, withState } from '@ngrx/signals';
import { NameClarification, SearchAllReply, ValueMatch } from '../core/api.types';
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
  /** The value the engine thinks the words mean ("Did you mean ...?"); null when none or when it is what was typed. */
  suggestion: ValueMatch | null;
  /** The question to ask when two things match equally; null when there is no tie. */
  clarify: NameClarification | null;
  /** The values behind `clarify.options`, in the engine's order. */
  options: ValueMatch[];
  /** What the user just told Debrief about a match, so it can be taken back. */
  feedback: NameFeedbackNote | null;
  loading: boolean;
  error: string | null;
}

/** A correction the user made: what they typed, what it was for and whether they accepted or rejected it. */
export interface NameFeedbackNote {
  kind: 'accepted' | 'rejected';
  typed: string;
  value: string;
}

/** What the engine thinks the words mean: the best value that is not what was typed, and the tie question if any. */
interface Understanding {
  suggestion: ValueMatch | null;
  clarify: NameClarification | null;
  options: ValueMatch[];
}

const NOTHING: Understanding = { suggestion: null, clarify: null, options: [] };

const initial: SearchEverythingState = { text: '', searched: '', reply: null, suggestion: null, clarify: null, options: [], feedback: null, loading: false, error: null };

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

    /** What the engine thinks the words mean. A failed lookup only means no suggestion. */
    async function understand(text: string): Promise<Understanding> {
      try {
        const { values, clarify } = await backend.findValues({ text, limit: 3 });
        const options = clarify ? clarify.options.flatMap((id) => values.filter((v) => v.group === id)) : [];
        if (clarify && options.length > 1) return { suggestion: null, clarify, options };
        return { ...NOTHING, suggestion: values.find((v) => v.value.toLowerCase() !== text.trim().toLowerCase()) ?? null };
      } catch {
        return NOTHING;
      }
    }

    async function search(text: string): Promise<void> {
      const mine = ++seq;
      if (!text.trim()) {
        patchState(store, { searched: text, reply: null, ...NOTHING, loading: false, error: null });
        return;
      }
      patchState(store, { loading: true, error: null });
      try {
        const [reply, understood] = await Promise.all([backend.searchEverything({ text }), understand(text)]);
        if (mine === seq) patchState(store, { searched: text, reply, ...understood, loading: false });
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
      /** Takes the suggested value as the search. */
      acceptSuggestion(): void {
        const value = store.suggestion()?.value;
        if (!value) return;
        patchState(store, { text: value, feedback: null });
        clearTimeout(timer);
        void search(value);
      },
      /** "Yes, that is what I meant": remembers that the typed words mean `match`, and searches its name. */
      async acceptName(match: ValueMatch): Promise<void> {
        const typed = store.searched();
        if (!match.group || !typed.trim()) return;
        try {
          await backend.acceptName({ text: typed, group: match.group });
        } catch (e) {
          patchState(store, { error: message(e) });
          return;
        }
        patchState(store, { text: match.value, feedback: { kind: 'accepted', typed, value: match.value } });
        clearTimeout(timer);
        await search(match.value);
      },
      /** "Not that": remembers that the typed words do not mean `match`, and searches the typed words again. */
      async rejectName(match: ValueMatch): Promise<void> {
        const typed = store.searched();
        if (!match.group || !typed.trim()) return;
        try {
          await backend.rejectName({ text: typed, group: match.group });
        } catch (e) {
          patchState(store, { error: message(e) });
          return;
        }
        await search(typed);
        patchState(store, { feedback: { kind: 'rejected', typed, value: match.value } });
      },
      /** Takes back the latest correction and searches the words as they were typed. */
      async undoName(): Promise<void> {
        const note = store.feedback();
        if (!note) return;
        try {
          await backend.undoName();
        } catch (e) {
          patchState(store, { error: message(e) });
          return;
        }
        patchState(store, { text: note.typed, feedback: null });
        clearTimeout(timer);
        await search(note.typed);
      },
      /** Searches the current text now (Retry, or Enter before the pause ended). */
      searchNow(): Promise<void> {
        clearTimeout(timer);
        return search(store.text());
      },
    };
  }),
);
