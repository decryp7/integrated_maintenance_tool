import { computed, DestroyRef, effect, inject, untracked } from '@angular/core';
import { patchState, signalStore, withComputed, withHooks, withMethods, withProps, withState } from '@ngrx/signals';
import { toAiError } from '../../ai/ai-backend';
import { IssueMatch, IssueReport } from '../../core/api.types';
import { failureText } from '../../core/cmd-error';
import { ConsumerGate } from '../../core/panel-visibility';
import { WorkspaceStore } from '../../core/workspace.store';
import { RefreshThrottle } from '../../problems/refresh-throttle';
import { KnownIssuesBackend } from '../investigate-backend';

/** At most one match run per this many ms after workspace changes (live tail changes the summary several times a second). */
export const KNOWN_ISSUES_REFRESH_MS = 3_000;

interface KnownIssuesState {
  /** The declared issues present in the open data (null before the first run). */
  report: IssueReport | null;
  /** The AI's suggestions, each verified by the engine. */
  suggestions: IssueMatch[];
  loading: boolean;
  suggesting: boolean;
  /** Matching failed (shown with Retry). */
  error: string | null;
  /** Suggesting failed, or needs a model. */
  suggestError: string | null;
  /** Ids of the suggestions the user accepted (they go into the incident report). */
  accepted: string[];
}

const INITIAL: KnownIssuesState = { report: null, suggestions: [], loading: false, suggesting: false, error: null, suggestError: null, accepted: [] };

/**
 * The known issues (ADR 0039) found in the open data: the engine matches the ones declared in the knowledge files
 * (`match`) and, on request, verifies the ones the AI suggests (`suggest`). Everything shown is what the engine
 * returned. Matched again when the workspace changes, coalesced, while the Problems pane shows the section.
 */
export const KnownIssuesStore = signalStore(
  { providedIn: 'root' },
  withState<KnownIssuesState>(INITIAL),
  withProps(() => ({ gate: new ConsumerGate() })),
  withComputed((s) => ({
    /** The suggestions the user accepted, still present among the current suggestions. */
    acceptedSuggestions: computed(() => s.suggestions().filter((m) => s.accepted().includes(m.issue.id))),
  })),
  withMethods((store, backend = inject(KnownIssuesBackend)) => {
    let seq = 0;
    return {
      async load(): Promise<void> {
        const mine = ++seq;
        patchState(store, { loading: true, error: null });
        try {
          const report = await backend.match();
          if (mine === seq) patchState(store, { report });
        } catch (e) {
          if (mine === seq) patchState(store, { error: failureText(e) });
        } finally {
          if (mine === seq) patchState(store, { loading: false });
        }
      },

      async suggest(): Promise<void> {
        if (store.suggesting()) return;
        patchState(store, { suggesting: true, suggestError: null });
        try {
          patchState(store, { suggestions: await backend.suggest(), accepted: [] });
        } catch (e) {
          const err = toAiError(e);
          const message =
            err.code === 'noModel' || err.code === 'unavailable'
              ? $localize`:@@known.issues.suggest.noModel:Suggestions need an AI model: set one up in Settings > AI.`
              : failureText(err);
          patchState(store, { suggestions: [], accepted: [], suggestError: message });
        } finally {
          patchState(store, { suggesting: false });
        }
      },

      clearSuggestions: () => patchState(store, { suggestions: [], accepted: [], suggestError: null }),

      /** Accepts or un-accepts a suggestion (accepted ones are added to the incident report). */
      toggleAccepted(id: string): void {
        patchState(store, (s) => ({ accepted: s.accepted.includes(id) ? s.accepted.filter((x) => x !== id) : [...s.accepted, id] }));
      },
    };
  }),
  withHooks({
    onInit(store, workspace = inject(WorkspaceStore), destroyRef = inject(DestroyRef)) {
      const throttle = new RefreshThrottle(
        () => void store.load(),
        () => KNOWN_ISSUES_REFRESH_MS,
      );
      destroyRef.onDestroy(() => throttle.dispose());
      // The sources changed, or the section became visible: match again, coalesced.
      effect(() => {
        workspace.summary();
        if (!store.gate.active()) return;
        untracked(() => throttle.request());
      });
    },
  }),
);
