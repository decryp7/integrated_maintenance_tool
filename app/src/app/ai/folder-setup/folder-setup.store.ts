import { computed, inject } from '@angular/core';
import { patchState, signalStore, withComputed, withMethods, withState } from '@ngrx/signals';
import { DebriefBackend } from '../../core/backend';
import { WorkspaceStore } from '../../core/workspace.store';
import { sanitizeQuery } from '../../lifecycle/core-ui-state';
import { newId } from '../../projects/bookmarks/ids';
import { SavedQueryStore } from '../../projects/bookmarks/saved-query.store';
import { UndoStack } from '../../undo/undo-stack';
import { toAiError } from '../ai-backend';
import { FolderSetupBackend } from './folder-setup-backend';
import { ComposedProfile, FolderRefinement, FolderSurvey, PROPOSAL_GROUPS, ProposalGroup, SetupProposal, SkippedProposal, SurveyOutcome, SurveyProgress } from './folder-setup.types';
import { JobsBackend } from '../../jobs/jobs-backend';
import { ParsersBackend } from '../../settings/parsers/parsers-backend';

/** Where the wizard is: survey → proposals → review → applied (or failed / cancelled on the way). */
export type SetupPhase = 'idle' | 'surveying' | 'proposals' | 'review' | 'applying' | 'applied' | 'failed' | 'cancelled';
export type AiPhase = 'idle' | 'running' | 'done' | 'failed';

export interface FolderSetupState {
  phase: SetupPhase;
  /** The survey job the wizard follows. */
  surveyJob: number | null;
  /** The AI refinement job the wizard follows. */
  aiJob: number | null;
  progress: SurveyProgress | null;
  survey: FolderSurvey | null;
  proposals: SetupProposal[];
  skipped: SkippedProposal[];
  selected: string[];
  ai: AiPhase;
  aiMessage: string | null;
  aiNotes: string[];
  composed: ComposedProfile | null;
  composing: boolean;
  error: string | null;
}

export const INITIAL_SETUP: FolderSetupState = {
  phase: 'idle',
  surveyJob: null,
  aiJob: null,
  progress: null,
  survey: null,
  proposals: [],
  skipped: [],
  selected: [],
  ai: 'idle',
  aiMessage: null,
  aiNotes: [],
  composed: null,
  composing: false,
  error: null,
};

function message(e: unknown): string {
  if (typeof e === 'string') return e;
  if (e instanceof Error) return e.message;
  return $localize`:@@ai.folderSetup.failed:The folder set-up failed.`;
}

/** Proposals that set the same singular thing (the instance key) exclude each other. */
function exclusiveGroup(p: SetupProposal): string | null {
  return p.fragments.some((f) => f.kind === 'instanceKey') ? 'instanceKey' : null;
}

/**
 * `ids` plus everything they require (transitively). Pure, for the store and tests.
 */
export function withRequirements(ids: readonly string[], proposals: readonly SetupProposal[]): string[] {
  const byId = new Map(proposals.map((p) => [p.id, p]));
  const out = new Set<string>();
  const visit = (id: string) => {
    if (out.has(id) || !byId.has(id)) return;
    out.add(id);
    for (const r of byId.get(id)?.requires ?? []) visit(r);
  };
  ids.forEach(visit);
  return [...out];
}

/** `ids` without `removed` and everything that requires them (transitively). */
export function withoutDependents(ids: readonly string[], removed: string, proposals: readonly SetupProposal[]): string[] {
  const gone = new Set([removed]);
  let changed = true;
  while (changed) {
    changed = false;
    for (const p of proposals) {
      if (!gone.has(p.id) && ids.includes(p.id) && p.requires.some((r) => gone.has(r))) {
        gone.add(p.id);
        changed = true;
      }
    }
  }
  return ids.filter((id) => !gone.has(id));
}

/** Toggles one proposal: selecting adds what it needs (and drops a rival instance key), deselecting drops what needs it. */
export function toggleSelection(ids: readonly string[], id: string, proposals: readonly SetupProposal[]): string[] {
  const p = proposals.find((x) => x.id === id);
  if (!p) return [...ids];
  if (ids.includes(id)) return withoutDependents(ids, id, proposals);
  let next = [...ids];
  const added = withRequirements([id], proposals);
  for (const a of added) {
    const group = exclusiveGroup(proposals.find((x) => x.id === a)!);
    if (!group) continue;
    for (const rival of proposals.filter((x) => x.id !== a && exclusiveGroup(x) === group && next.includes(x.id))) {
      next = withoutDependents(next, rival.id, proposals);
    }
  }
  return [...new Set([...next, ...added])];
}

/** The recommended selection, made consistent (requirements in, rival instance keys out). */
export function recommendedSelection(proposals: readonly SetupProposal[]): string[] {
  let ids: string[] = [];
  for (const p of proposals.filter((x) => x.recommended)) if (!ids.includes(p.id)) ids = toggleSelection(ids, p.id, proposals);
  return ids;
}

/** The "Set up analysis for this folder…" wizard: its state and the engine calls behind each step. */
export const FolderSetupStore = signalStore(
  { providedIn: 'root' },
  withState<FolderSetupState>(INITIAL_SETUP),
  withComputed((s) => ({
    groups: computed(() =>
      PROPOSAL_GROUPS.map((group: ProposalGroup) => ({ group, proposals: s.proposals().filter((p) => p.group === group) })).filter((g) => g.proposals.length > 0),
    ),
    selectedProposals: computed(() => s.proposals().filter((p) => s.selected().includes(p.id))),
    canReview: computed(() => s.phase() === 'proposals' && s.selected().length > 0 && !s.composing()),
  })),
  withMethods(
    (
      s,
      backend = inject(FolderSetupBackend),
      debrief = inject(DebriefBackend),
      jobs = inject(JobsBackend),
      workspace = inject(WorkspaceStore),
      savedQueries = inject(SavedQueryStore),
      undo = inject(UndoStack),
      parsers = inject(ParsersBackend),
    ) => ({
      /**
       * Starts a survey of the open files (from any phase) as a background analysis job and follows it while the
       * wizard is open (`detach` when it closes: the job's message reopens the wizard with the proposals).
       */
      async start(): Promise<void> {
        patchState(s, { ...INITIAL_SETUP, phase: 'surveying' });
        let id: number | null = null;
        try {
          id = await backend.survey((progress) => {
            if (id === null || s.surveyJob() === id) patchState(s, { progress });
          });
          patchState(s, { surveyJob: id });
          const end = await jobs.wait(id);
          if (s.surveyJob() !== id) return;
          if (end.state === 'failed') throw end.error ?? message(null);
          if (end.state !== 'done' || end.result?.type !== 'folderSurvey') {
            patchState(s, { phase: 'cancelled', surveyJob: null });
            return;
          }
          this.showSurvey(end.result.outcome as SurveyOutcome);
        } catch (e) {
          if (id === null || s.surveyJob() === id) patchState(s, { phase: 'failed', surveyJob: null, error: message(e) });
        }
      },
      /** Shows a survey's outcome (its job ended while the wizard followed it, or View of its message). */
      showSurvey(out: SurveyOutcome): void {
        if ('cancelled' in out) {
          patchState(s, { phase: 'cancelled', surveyJob: null });
          return;
        }
        const { survey, proposals, skipped } = out.setup;
        patchState(s, { phase: 'proposals', surveyJob: null, survey, proposals, skipped, selected: recommendedSelection(proposals) });
      },
      async cancel(): Promise<void> {
        const id = s.surveyJob();
        if (id !== null) await jobs.cancel(id);
      },
      /** The wizard closed: running jobs go on, and their messages (View) reopen it with their result. */
      detach(): void {
        const survey = s.surveyJob();
        const ai = s.aiJob();
        patchState(s, { surveyJob: null, aiJob: null, ai: ai === null ? s.ai() : 'idle', phase: survey === null ? s.phase() : 'idle' });
        for (const id of [survey, ai]) if (id !== null) void jobs.unwait(id).catch(() => undefined);
      },
      toggle(id: string): void {
        patchState(s, { selected: toggleSelection(s.selected(), id, s.proposals()), composed: null });
      },
      isSelected(id: string): boolean {
        return s.selected().includes(id);
      },
      /** Adds the AI's validated proposals (unselected) and its skipped ones. */
      async refineWithAi(): Promise<void> {
        if (s.ai() === 'running') return;
        patchState(s, { ai: 'running', aiMessage: null, aiNotes: [] });
        try {
          const id = await backend.refineWithAi();
          patchState(s, { aiJob: id });
          const end = await jobs.wait(id);
          if (s.aiJob() !== id) return;
          patchState(s, { aiJob: null });
          if (end.state === 'failed') throw new Error(end.error ?? message(null));
          if (end.state !== 'done' || end.result?.type !== 'proposal') {
            patchState(s, { ai: 'idle' });
            return;
          }
          this.addRefinement(end.result.proposal as FolderRefinement);
        } catch (e) {
          const err = toAiError(e);
          patchState(s, { ai: err.code === 'cancelled' ? 'idle' : 'failed', aiMessage: err.code === 'cancelled' ? null : err.message, aiJob: null });
        }
      },
      /** Adds the AI's proposals (the job ended while the wizard followed it, or View of its message). */
      addRefinement(r: FolderRefinement): void {
        const known = new Set(s.proposals().map((p) => p.id));
        patchState(s, {
          ai: 'done',
          phase: s.phase() === 'idle' ? 'proposals' : s.phase(),
          proposals: [...s.proposals(), ...r.proposals.filter((p) => !known.has(p.id))],
          skipped: [...s.skipped(), ...r.skipped],
          aiNotes: r.notes,
        });
      },
      async cancelAi(): Promise<void> {
        const id = s.aiJob();
        if (id !== null) await jobs.cancel(id);
      },
      /** Asks the engine for the combined profile and its diff. */
      async review(): Promise<void> {
        const fragments = s.selectedProposals().flatMap((p) => p.fragments);
        if (!fragments.length) return;
        patchState(s, { composing: true, error: null });
        try {
          const composed = await backend.compose(fragments);
          patchState(s, { composed, composing: false, phase: 'review' });
        } catch (e) {
          patchState(s, { composing: false, error: message(e) });
        }
      },
      back(): void {
        patchState(s, { phase: 'proposals', error: null });
      },
      /**
       * Saves the combined profile through the normal profile save (validated, archived, re-imported) and adds
       * the saved queries, as one undoable change; new format parsers go to the library (replacing one with the same
       * id) and the files are read again with them (ADR 0026; the library is not part of the undo).
       */
      async apply(): Promise<boolean> {
        const composed = s.composed();
        if (!composed || s.phase() !== 'review') return false;
        patchState(s, { phase: 'applying', error: null });
        try {
          const before = (await debrief.getProfile()).toml;
          void workspace.applyProfile(await debrief.saveProfile(composed.toml));
          for (const toml of composed.parsers) await parsers.save(toml, { overwrite: true });
          if (composed.parsers.length) void workspace.followReread(await parsers.reread());
          const queriesBefore = savedQueries.items();
          const queriesAfter = [
            ...queriesBefore,
            ...composed.savedQueries.map((q) => ({ id: newId('sq'), name: q.name, query: sanitizeQuery({ ql: q.ql }) })),
          ];
          savedQueries.replace(queriesAfter);
          const after = composed.toml;
          undo.record({
            label: $localize`:Undo history entry@@ai.folderSetup.undo:folder set-up`,
            do: async () => {
              void workspace.applyProfile(await debrief.saveProfile(after));
              savedQueries.replace(queriesAfter);
            },
            undo: async () => {
              void workspace.applyProfile(await debrief.saveProfile(before));
              savedQueries.replace(queriesBefore);
            },
          });
          patchState(s, { phase: 'applied' });
          return true;
        } catch (e) {
          patchState(s, { phase: 'review', error: message(e) });
          return false;
        }
      },
      reset(): void {
        patchState(s, INITIAL_SETUP);
      },
    }),
  ),
);
