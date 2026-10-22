import { computed, DestroyRef, effect, inject, untracked } from '@angular/core';
import { patchState, signalStore, withComputed, withHooks, withMethods, withProps, withState } from '@ngrx/signals';
import { AiLauncher } from '../ai/ai-launcher';
import { DiagnosticRow, FixEvaluation, IndexStatus } from '../core/api.types';
import { ImtBackend } from '../core/backend';
import { failureText } from '../core/cmd-error';
import { ConsumerGate } from '../core/panel-visibility';
import { EMPTY_QUERY } from '../core/workspace/query.store';
import { WorkspaceStore } from '../core/workspace.store';
import { COMMAND_CONTEXT } from '../extensions/command-registry';
import { ParsersBackend } from '../settings/parsers/parsers-backend';
import { SourceFileActions } from '../sources/source-file-actions';
import { countProblems, groupProblems, ProblemItem, problemKey, quickFixOf } from './problems-model';
import { ProblemsBackend } from './problems-backend';
import { RefreshThrottle } from './refresh-throttle';

/** A quick fix's preview (`evaluate_fix`): running, its result, or why it failed. */
export type FixPreview = { state: 'running' } | { state: 'done'; result: FixEvaluation } | { state: 'failed'; error: string };

/** The parser that changes when a source no library parser read gets a fix (the fallback, ADR 0026). */
const FALLBACK_PARSER = 'generic';
/** Lines of the file the AI gets when no stored event is at the problem's line. */
const AI_SAMPLE_LINES = 40;
const AI_SAMPLE_BYTES = 16 * 1024;
/**
 * At most one listing per this many ms after workspace changes (the summary changes up to ~4x/s in live tail): while
 * the Problems pane is visible, and while it is not (the status-bar badge still counts, less often).
 */
export const PROBLEMS_REFRESH_VISIBLE_MS = 2_000;
export const PROBLEMS_REFRESH_HIDDEN_MS = 10_000;

interface ProblemsState {
  rows: DiagnosticRow[];
  /** The first listing arrived (the badge shows nothing before). */
  loaded: boolean;
  loading: boolean;
  /** Listing failed (shown with Retry). */
  loadError: string | null;
  index: IndexStatus | null;
  /** The problem shown in the details (its `problemKey`). */
  selectedKey: string | null;
  /** Fix previews by problem key. */
  previews: Record<string, FixPreview>;
  /** The problem whose fix is being applied. */
  applying: string | null;
  /** The last action failed / what it did. */
  actionError: string | null;
  notice: string | null;
}

const INITIAL: ProblemsState = {
  rows: [],
  loaded: false,
  loading: false,
  loadError: null,
  index: null,
  selectedKey: null,
  previews: {},
  applying: null,
  actionError: null,
  notice: null,
};

/**
 * The window's import problems as the engine lists them (`import_problems`), the Problems pane's selection, and the
 * quick fixes: a preview of what the engine's fix would repair (`evaluate_fix`), applying it (the source's parser
 * changed by the fix's `[format]` fragment, saved in the library, the files read again: ADR 0026), and "Ask the AI
 * to fix", which opens the AI's parser proposal for the user to review. Listed again when the workspace changes, at
 * most once per `PROBLEMS_REFRESH_VISIBLE_MS` while the pane is visible (`gate`) and less often while it is not.
 */
export const ProblemsStore = signalStore(
  { providedIn: 'root' },
  withState<ProblemsState>(INITIAL),
  // The Problems pane registers here; changes are listed sooner while it is visible.
  withProps(() => ({ gate: new ConsumerGate() })),
  withComputed((store) => ({
    groups: computed(() => groupProblems(store.rows())),
    counts: computed(() => countProblems(store.rows())),
    selected: computed<ProblemItem | null>(() => {
      const key = store.selectedKey();
      const row = key ? store.rows().find((r) => problemKey(r) === key) : undefined;
      return row ? { key: key!, row } : null;
    }),
  })),
  withMethods(
    (
      store,
      backend = inject(ProblemsBackend),
      imt = inject(ImtBackend),
      parsers = inject(ParsersBackend),
      workspace = inject(WorkspaceStore),
      ctx = inject(COMMAND_CONTEXT),
      files = inject(SourceFileActions),
      ai = inject(AiLauncher),
    ) => {
      let seq = 0;
      const rowOf = (key: string): DiagnosticRow | null => store.rows().find((r) => problemKey(r) === key) ?? null;
      const setPreview = (key: string, preview: FixPreview) => patchState(store, (s) => ({ previews: { ...s.previews, [key]: preview } }));

      /** The id of the stored event that starts at `line` of the row's source; null when none does (a skipped line). */
      const eventAt = async (row: DiagnosticRow, line: number): Promise<number | null> => {
        if (row.source === null) return null;
        const page = await imt.queryPage({
          query: { ...EMPTY_QUERY, sources: [row.source], filters: [{ field: '@line', op: 'eq', value: String(line) }] },
          offset: 0,
          limit: 1,
          descending: false,
        });
        return page.rows[0]?.id ?? null;
      };

      const load = async (): Promise<void> => {
        const mine = ++seq;
        patchState(store, { loading: true, loadError: null });
        try {
          const [rows, index] = await Promise.all([backend.problems(), backend.indexStatus().catch(() => null)]);
          if (mine !== seq) return;
          const keys = new Set(rows.map(problemKey));
          // Previews are of the stored events: after a re-read they are out of date.
          patchState(store, (s) => ({ rows, index, loaded: true, previews: {}, selectedKey: s.selectedKey && keys.has(s.selectedKey) ? s.selectedKey : null }));
        } catch (e) {
          if (mine === seq) patchState(store, { loadError: failureText(e) });
        } finally {
          if (mine === seq) patchState(store, { loading: false });
        }
      };

      return {
        load,
        select: (key: string | null) => patchState(store, { selectedKey: key }),
        dismiss: () => patchState(store, { actionError: null, notice: null }),

        /** Asks the engine how many of the affected stored events the row's fix would repair. Changes nothing. */
        async preview(key: string): Promise<void> {
          const row = rowOf(key);
          const fix = row ? quickFixOf(row) : null;
          if (!fix) return;
          setPreview(key, { state: 'running' });
          try {
            setPreview(key, { state: 'done', result: await backend.evaluateFix(fix) });
          } catch (e) {
            setPreview(key, { state: 'failed', error: failureText(e) });
          }
        },

        /**
         * Applies the row's fix: the engine merges its `[format]` fragment over the source's parser (a built-in's
         * becomes an override), the library saves it, and the files read with that parser are read again.
         */
        async apply(key: string): Promise<boolean> {
          const row = rowOf(key);
          if (!row?.fix || store.applying()) return false;
          const base = workspace.summary()?.sources.find((s) => s.id === row.source)?.parser ?? FALLBACK_PARSER;
          patchState(store, { applying: key, actionError: null, notice: null });
          try {
            const draft = await parsers.draft({ fileName: row.sourceName, base, regex: null, template: null, timestampFormat: null, parserOverride: row.fix.parserOverride });
            await parsers.save(draft.toml, { previousId: base, overwrite: true });
            await workspace.followReread(await parsers.reread());
            const id = draft.id;
            patchState(store, { notice: $localize`:@@problems.applied:Changed the parser ${id}:id: and read its files again.` });
            await load();
            return true;
          } catch (e) {
            patchState(store, { actionError: failureText(e) });
            return false;
          } finally {
            patchState(store, { applying: null });
          }
        },

        /**
         * "Ask the AI to fix": the AI generates a parser (ADR 0032). For a file of this window, Rust reads the file and
         * the engine checks each draft on it; otherwise the lines around the first problem (the stored event there,
         * else the start of the file) are the sample. The proposal opens for the user to review and apply. Nothing
         * changes until they do.
         */
        async askAi(key: string): Promise<void> {
          const row = rowOf(key);
          if (!row) return;
          patchState(store, { actionError: null });
          try {
            const line = row.sampleLines[0];
            const recordId = line !== undefined ? await eventAt(row, line) : null;
            if (workspace.summary()?.sources.some((s) => s.id === row.source)) {
              await ai.propose('parser', { sourceId: row.source, recordId, fileName: row.sourceName });
              return;
            }
            if (recordId !== null) {
              await ai.propose('parser', { recordId, fileName: row.sourceName });
              return;
            }
            const head = await imt.readFileHead(row.path, AI_SAMPLE_BYTES);
            const sample = head.split(/\r?\n/).filter((l) => l.length > 0).slice(0, AI_SAMPLE_LINES).join('\n');
            await ai.propose('parser', { sample, fileName: row.sourceName });
          } catch (e) {
            patchState(store, { actionError: failureText(e) });
          }
        },

        /**
         * Jumps to a problem's line: the stored event there in the grid, or (a line that was not stored, e.g. skipped)
         * the file in the editor at that line.
         */
        async jumpTo(key: string, line: number): Promise<void> {
          const row = rowOf(key);
          if (!row) return;
          patchState(store, { actionError: null });
          try {
            const id = await eventAt(row, line);
            if (id !== null) {
              ctx.showView('events');
              await ctx.jumpTo(id);
              return;
            }
            const source = workspace.summary()?.sources.find((s) => s.id === row.source);
            if (!source) {
              patchState(store, { actionError: $localize`:@@problems.jump.notImported:The file was not imported: there is no line to show.` });
              return;
            }
            await files.run(source, 'openAtLine', line);
          } catch (e) {
            patchState(store, { actionError: failureText(e) });
          }
        },
      };
    },
  ),
  withHooks({
    onInit(store, workspace = inject(WorkspaceStore), destroyRef = inject(DestroyRef)) {
      const throttle = new RefreshThrottle(
        () => void store.load(),
        () => (store.gate.active() ? PROBLEMS_REFRESH_VISIBLE_MS : PROBLEMS_REFRESH_HIDDEN_MS),
      );
      destroyRef.onDestroy(() => throttle.dispose());
      // The sources changed (opened, re-read, removed, appended to): list again, coalesced.
      effect(() => {
        workspace.summary();
        untracked(() => throttle.request());
      });
      // The pane became visible (or hidden): a pending listing moves to the new interval.
      effect(() => {
        store.gate.active();
        untracked(() => throttle.reschedule());
      });
    },
  }),
);
