import { computed, inject } from '@angular/core';
import { patchState, signalStore, withComputed, withMethods, withState } from '@ngrx/signals';
import { QlError, SqlResult, SqlSchema } from '../core/api.types';
import { failureText } from '../core/cmd-error';
import { StorePaneBackend } from './store-pane-backend';

/** Texts kept in the console's history. */
export const SQL_HISTORY = 20;

interface SqlConsoleState {
  text: string;
  explain: boolean;
  /** The run in flight (its id for stopping). */
  runId: string | null;
  /** The last run's result (columns, plan, totals) and every row loaded so far. */
  result: SqlResult | null;
  rows: unknown[][];
  loadingMore: boolean;
  /** The engine refused or could not read the SQL (with the span to underline). */
  error: QlError | null;
  /** The run failed for another reason (a limit, the store). */
  failure: string | null;
  stopped: boolean;
  schema: SqlSchema | null;
  history: string[];
}

let runs = 0;

/**
 * The SQL console (`sql_run`, `sql_page`, `sql_schema`): the editor's text and Explain switch, the run in flight (a job;
 * Stop cancels it), the result with its loaded pages, the engine's error, and the history. The engine checks and runs
 * the SQL; this store only keeps what it answered.
 */
export const SqlConsoleStore = signalStore(
  { providedIn: 'root' },
  withState<SqlConsoleState>({
    text: 'SELECT level, count(*) AS n\nFROM events\nGROUP BY level\nORDER BY n DESC',
    explain: false,
    runId: null,
    result: null,
    rows: [],
    loadingMore: false,
    error: null,
    failure: null,
    stopped: false,
    schema: null,
    history: [],
  }),
  withComputed((store) => ({
    running: computed(() => store.runId() !== null),
    /** More rows than loaded are kept by the engine. */
    hasMore: computed(() => {
      const r = store.result();
      return !!r && store.rows().length < r.total;
    }),
  })),
  withMethods((store, backend = inject(StorePaneBackend)) => ({
    setText: (text: string) => patchState(store, { text, error: null }),
    setExplain: (explain: boolean) => patchState(store, { explain }),
    /** Loads the views, columns and functions to complete (once). */
    async loadSchema(): Promise<void> {
      if (store.schema()) return;
      try {
        patchState(store, { schema: await backend.describeSqlSchema() });
      } catch {
        // Completion is a convenience: the editor works without it.
      }
    },
    /** Runs the editor's text (a second run while one is in flight is ignored). */
    async run(): Promise<void> {
      const sql = store.text();
      if (store.runId() || !sql.trim()) return;
      const runId = `sql-${++runs}`;
      patchState(store, { runId, error: null, failure: null, stopped: false });
      try {
        const out = await backend.querySql(sql, store.explain(), runId);
        const history = [sql, ...store.history().filter((h) => h !== sql)].slice(0, SQL_HISTORY);
        if ('result' in out) patchState(store, { result: out.result, rows: out.result.rows, history });
        else if ('error' in out) patchState(store, { error: out.error });
        else patchState(store, { stopped: true });
      } catch (e) {
        patchState(store, { failure: failureText(e) });
      } finally {
        patchState(store, { runId: null });
      }
    },
    /** Stops the run in flight. */
    async stop(): Promise<void> {
      const id = store.runId();
      if (id) await backend.cancelSql(id).catch(() => false);
    },
    /** The next page of the result. */
    async loadMore(): Promise<void> {
      const r = store.result();
      if (!r || store.loadingMore() || store.rows().length >= r.total) return;
      patchState(store, { loadingMore: true });
      try {
        const page = await backend.fetchSqlPage({ result: r.result, offset: store.rows().length });
        patchState(store, { rows: [...store.rows(), ...page.rows] });
      } catch (e) {
        patchState(store, { failure: failureText(e) });
      } finally {
        patchState(store, { loadingMore: false });
      }
    },
    /** Puts a saved or earlier SQL text in the editor. */
    open(sql: string): void {
      patchState(store, { text: sql, error: null });
    },
  })),
);
