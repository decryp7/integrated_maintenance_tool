import { effect, inject, untracked } from '@angular/core';
import { patchState, signalStore, withHooks, withState } from '@ngrx/signals';
import { Query } from '../api.types';
import { DebriefBackend } from '../backend';
import { CmdError, isCmdErrorDto } from '../cmd-error';
import { Logger } from '../logger';
import { QueryStore } from './query.store';
import { WorkspaceDataStore } from './workspace-data.store';

interface SmartSearchState {
  /** Partial matches the background scan added to the current search (0: none, or not scanned yet). */
  added: number;
  /** Whether the background scan is running. */
  scanning: boolean;
}

/** The query as the scan sees it: everything but how the text matches (switching that must not scan again). */
function scanKey(q: Query): string {
  return JSON.stringify({ ...q, textMode: undefined });
}

/** Whether the query is a smart (word) search the engine can add partial matches to. */
export function isWordSearch(q: Query): boolean {
  return q.text.trim() !== '' && !q.regex && !q.caseSensitive && (q.textMode === 'words' || q.textMode === 'wordsOrSubstring');
}

/**
 * Smart search, step two: the search box answers with word matches at once (`textMode: 'words'`, from the engine's
 * full-text index); then the engine's background scan (`search_partial`, a cancellable job) counts the events that hold
 * the text only inside words. When it finds some, the query switches to `wordsOrSubstring` so every view adds them,
 * and `added` tells the user how many ("N more partial matches added"). A newer search supersedes the scan.
 */
export const SmartSearchStore = signalStore(
  { providedIn: 'root' },
  withState<SmartSearchState>({ added: 0, scanning: false }),
  withHooks({
    onInit(store) {
      const backend = inject(DebriefBackend);
      const query = inject(QueryStore);
      const data = inject(WorkspaceDataStore);
      const logger = inject(Logger);
      let seq = 0;
      let lastKey = '';
      // effect-state: IPC scan keyed on the query; progress and result land in the store
      effect(() => {
        const q = query.effectiveQuery();
        data.dataVersion();
        const loaded = data.summary() !== null;
        const key = scanKey(q);
        if (!loaded || !isWordSearch(q)) {
          lastKey = '';
          seq++;
          untracked(() => patchState(store, { added: 0, scanning: false }));
          return;
        }
        if (key === lastKey) return;
        lastKey = key;
        const mine = ++seq;
        untracked(() => patchState(store, { added: 0, scanning: true }));
        backend
          .searchPartial({ ...q, textMode: 'words' })
          .then((found) => {
            if (mine !== seq) return;
            patchState(store, { scanning: false, added: found?.added ?? 0 });
            if (found && found.added > 0 && query.query().textMode === 'words') query.patchQuery({ textMode: 'wordsOrSubstring' });
          })
          .catch((e) => {
            if (mine !== seq) return;
            // Word matches are shown already; partial matches are a bonus, so a failed scan is only logged.
            logger.warn('search', 'Partial matches are unavailable', { Code: e instanceof CmdError || isCmdErrorDto(e) ? e.code : 'unknown' });
            patchState(store, { scanning: false });
          });
      });
    },
  }),
);
