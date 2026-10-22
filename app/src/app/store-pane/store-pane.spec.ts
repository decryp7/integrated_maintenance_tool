import { TestBed } from '@angular/core/testing';
import { all, buttonByText, el, provideTinyBackend, settle, text, tinyBackend, typeInto } from '../../testing/helpers';
import { at, must } from '../../testing/must';
import { SqlSchema, StoreMemory } from '../core/api.types';
import { CmdError } from '../core/cmd-error';
import { MockBackend } from '../core/mock/mock-backend';
import { COMMAND_CONTEXT } from '../extensions/command-registry';
import { CommandContext } from '../extensions/tokens';
import { parseSavedSql, SavedSqlStore } from '../projects/bookmarks/saved-sql.store';
import { SavedQueries } from '../projects/bookmarks/saved-queries';
import { ProjectUiState } from '../projects/project-ui-state';
import { DialogService } from '../ui/dialog.service';
import { MockStorePaneBackend } from './mock/mock-store-pane-backend';
import { storePaneCommands, storePaneViews } from './provide-store-pane';
import { applyCompletion, byteToIndex, completeSql, rowsToTsv, wordAt } from './sql-complete';
import { SqlConsoleStore } from './sql-console.store';
import { StorePane } from './store-pane';
import { StorePaneBackend, TauriStorePaneBackend } from './store-pane-backend';
import { activityLabel, gauge, maintenanceText, memoryGauges, STORE_VIEW, storageLabel } from './store-pane-model';
import { StorePaneStore } from './store-pane.store';

const ctx = () => ({ showView: vi.fn(), jumpTo: vi.fn(async () => undefined), notifyError: vi.fn() }) as unknown as CommandContext;

function provide(backend: MockBackend = tinyBackend(), context = ctx()) {
  const pane = new MockStorePaneBackend(backend);
  TestBed.configureTestingModule({
    providers: [...provideTinyBackend(backend), { provide: StorePaneBackend, useValue: pane }, { provide: COMMAND_CONTEXT, useValue: context }],
  });
  return { backend, pane, context };
}

const SCHEMA: SqlSchema = {
  version: 1,
  views: [
    { name: 'events', description: 'Events', columns: [{ name: 'id', type: 'UBIGINT', description: '' }, { name: 'level', type: 'VARCHAR', description: '' }] },
    { name: 'sources', description: 'Sources', columns: [{ name: 'name', type: 'VARCHAR', description: '' }] },
  ],
  functions: ['lower', 'length'],
  tableFunctions: ['range'],
  maxRows: 10_000,
  pageRows: 500,
  timeLimitMs: 30_000,
};

describe('SQL completion and helpers', () => {
  it('completes views, columns, keywords and functions by prefix', () => {
    expect(wordAt('SELECT lev', 10)).toEqual({ word: 'lev', start: 7 });
    expect(completeSql('SELECT lev', 10, SCHEMA).map((c) => c.text)).toEqual(['level']);
    expect(completeSql('FROM ev', 7, SCHEMA)[0]).toEqual({ text: 'events', kind: 'view', detail: 'Events' });
    expect(completeSql('SELECT l', 8, SCHEMA).map((c) => c.kind)).toEqual(['column', 'keyword', 'function', 'function']);
    expect(completeSql('SELECT sources.', 15, SCHEMA).map((c) => c.text)).toEqual(['name']);
    expect(completeSql('SELECT nothing.', 15, SCHEMA)).toEqual([]);
    expect(completeSql('SELECT ', 7, SCHEMA)).toEqual([]);
    expect(completeSql('FROM events', 11, SCHEMA)).toEqual([]);
    expect(completeSql('SELECT l', 8, null)).toEqual([]);
    expect(completeSql('SELECT l', 8, SCHEMA, 1)).toHaveLength(1);
    expect(applyCompletion('SELECT lev FROM x', 10, 'level')).toEqual({ text: 'SELECT level FROM x', cursor: 12 });
  });

  it('maps byte offsets to string positions and copies rows as TSV', () => {
    expect(byteToIndex('abc', 2)).toBe(2);
    expect(byteToIndex('é b', 2)).toBe(1);
    expect(byteToIndex('ab', 99)).toBe(2);
    expect(byteToIndex('😀x', 4)).toBe(2);
    expect(rowsToTsv(['a', 'b'], [[1, null], ['x\ty', 'l\nm']])).toBe('a\tb\n1\t\nx y\tl m');
  });

  it('words the model', () => {
    expect(gauge('x', 50, 100)).toEqual({ label: 'x', value: 50, max: 100, percent: 50, near: false });
    expect(gauge('x', 90, 100).near).toBe(true);
    expect(gauge('x', 5, 0).percent).toBe(0);
    expect(gauge('x', 500, 100).percent).toBe(100);
    const m = { worker: { pid: 1, rssBytes: 10, lowMemory: false }, importing: true, importBudgetBytes: 100, queryBudgetBytes: 50, duckUsedBytes: 1, duckLimitBytes: 2 } as StoreMemory;
    expect(memoryGauges(m).map((g) => g.max)).toEqual([100, 2]);
    expect(memoryGauges({ ...m, importing: false }).map((g) => g.max)).toEqual([50, 2]);
    expect(memoryGauges({ ...m, worker: null })).toHaveLength(1);
    expect(['ready', 'importing', 'indexing'].map((a) => activityLabel(a as never))).toEqual(['Ready', 'Importing', 'Indexing']);
    expect(['declared', 'promoted', 'partly', 'overflow'].map((s) => storageLabel(s as never)).every((s) => s.length > 3)).toBe(true);
    for (const a of ['compact', 'rebuildIndex', 'clearCachedAnalyses'] as const) expect(maintenanceText(a).confirm).toBeTruthy();
  });
});

describe('TauriStorePaneBackend', () => {
  it('calls the store_* and sql_* commands and cancels a run through its job', async () => {
    const calls: [string, unknown][] = [];
    const hooks: { onEvent?: (e: unknown) => void } = {};
    let finish: (v: unknown) => void = () => undefined;
    const call = vi.fn(async (cmd: string, args?: Record<string, unknown>) => {
      calls.push([cmd, args]);
      if (cmd === 'sql_run') {
        hooks.onEvent = (args as { onEvent: (e: unknown) => void }).onEvent;
        return new Promise((resolve) => (finish = resolve));
      }
      return cmd === 'store_maintenance' ? 7 : {};
    }) as never;
    const b = new TauriStorePaneBackend(call, (fn) => fn);
    await b.overview();
    await b.listSegments({ offset: 5, limit: 10 });
    await b.listFields();
    await b.describeIndex();
    await b.describeMemory();
    expect(await b.maintain('compact')).toBe(7);
    await b.describeSqlSchema();
    await b.fetchSqlPage({ result: 1, offset: 500 });
    expect(await b.cancelSql('none')).toBe(false);
    const run = b.querySql('SELECT 1', true, 'r1');
    await Promise.resolve();
    expect(await b.cancelSql('r1')).toBe(true);
    must(hooks.onEvent)({ kind: 'started', jobId: 42 });
    must(hooks.onEvent)({ kind: 'progress' });
    await Promise.resolve();
    finish({ cancelled: true });
    expect(await run).toEqual({ cancelled: true });
    expect(calls.map((c) => c[0])).toEqual([
      'store_overview',
      'store_segments',
      'store_fields',
      'store_index',
      'store_memory',
      'store_maintenance',
      'sql_schema',
      'sql_page',
      'sql_run',
      'jobs_cancel',
    ]);
    expect(at(calls, 1)[1]).toEqual({ request: { offset: 5, limit: 10 } });
    expect(at(calls, 5)[1]).toEqual({ action: 'compact' });
    expect(at(calls, 9)[1]).toEqual({ id: 42 });
    expect((at(calls, 8)[1] as { sql: string; explain: boolean }).explain).toBe(true);
    // A stop after the job is known cancels it at once.
    const second = b.querySql('SELECT 2', false, 'r2');
    must(hooks.onEvent)({ kind: 'started', jobId: 43 });
    expect(await b.cancelSql('r2')).toBe(true);
    finish({ cancelled: true });
    await second;
    expect(calls.at(-1)).toEqual(['jobs_cancel', { id: 43 }]);
  });
});

describe('StorePaneStore', () => {
  it('loads each report on its tab, pages segments, and keeps a failure per report', async () => {
    const { pane } = provide();
    const store = TestBed.inject(StorePaneStore);
    await store.show('overview');
    expect(store.overview().value?.events).toBeGreaterThan(0);
    await store.show('segments');
    expect(store.segments().value?.rows.length).toBeGreaterThan(0);
    await store.segmentsAt(-5);
    expect(store.segments().value?.offset).toBe(0);
    await store.show('fields');
    expect(store.fields().value?.fields.length).toBeGreaterThan(0);
    await store.show('index');
    expect(store.index().value?.catchingUp).toBe(false);
    await store.show('memory');
    expect(store.memory().value?.worker?.pid).toBe(4242);
    await store.show('sql');
    expect(store.tab()).toBe('sql');
    await store.refresh();
    vi.spyOn(pane, 'describeIndex').mockRejectedValue(new CmdError({ code: 'failed', message: 'the store worker stopped', retryable: true }));
    await store.show('index');
    expect(store.index().error).toBe('the store worker stopped');
    expect(store.index().value).not.toBeNull();
    await store.maintain('clearCachedAnalyses');
    expect(store.notice()?.error).toBe(false);
    vi.spyOn(pane, 'maintain').mockRejectedValue(new CmdError({ code: 'busy', message: 'busy: an import is running', retryable: true }));
    await store.maintain('compact');
    expect(store.notice()).toEqual({ text: 'busy: an import is running', error: true });
    store.dismissNotice();
    expect(store.notice()).toBeNull();
  });
});

describe('SqlConsoleStore', () => {
  it('runs, pages, keeps history, shows refusals and failures, and stops', async () => {
    const { pane } = provide();
    const sql = TestBed.inject(SqlConsoleStore);
    await sql.loadSchema();
    await sql.loadSchema();
    expect(sql.schema()?.views.length).toBe(4);
    sql.setText('SELECT * FROM events');
    await sql.run();
    expect(sql.result()?.eventIds).toBe(true);
    expect(sql.rows().length).toBe(Math.min(500, must(sql.result()).total));
    expect(sql.history()).toEqual(['SELECT * FROM events']);
    vi.spyOn(pane, 'querySql').mockResolvedValueOnce({ result: { result: 9, columns: [{ name: 'n', type: 'BIGINT' }], rows: [[1]], total: 2, truncated: false, eventIds: false, plan: null, elapsedMs: 1 } });
    vi.spyOn(pane, 'fetchSqlPage').mockResolvedValueOnce({ offset: 1, rows: [[2]] });
    await sql.run();
    expect(sql.hasMore()).toBe(true);
    await sql.loadMore();
    expect(sql.rows()).toEqual([[1], [2]]);
    expect(sql.hasMore()).toBe(false);
    await sql.loadMore();
    sql.setText('DROP VIEW events');
    await sql.run();
    expect(sql.error()?.message).toContain('DROP');
    expect(sql.error()?.offset).toBe(0);
    sql.setText('  ');
    await sql.run();
    vi.spyOn(pane, 'querySql').mockRejectedValueOnce(new CmdError({ code: 'invalid', message: 'Stopped after 30 s.', retryable: false }));
    sql.setText('SELECT 1');
    await sql.run();
    expect(sql.failure()).toBe('Stopped after 30 s.');
    vi.spyOn(pane, 'querySql').mockResolvedValueOnce({ cancelled: true });
    await sql.run();
    expect(sql.stopped()).toBe(true);
    // A stop while running reaches the backend.
    let release: (v: never) => void = () => undefined;
    vi.spyOn(pane, 'querySql').mockReturnValueOnce(new Promise((r) => (release = r)));
    const cancel = vi.spyOn(pane, 'cancelSql');
    const running = sql.run();
    expect(sql.running()).toBe(true);
    await sql.run();
    await sql.stop();
    expect(cancel).toHaveBeenCalled();
    release({ cancelled: true } as never);
    await running;
    expect(sql.running()).toBe(false);
    await sql.stop();
    vi.spyOn(pane, 'querySql').mockResolvedValueOnce({ result: { result: 3, columns: [], rows: [], total: 5, truncated: false, eventIds: false, plan: null, elapsedMs: 1 } });
    await sql.run();
    vi.spyOn(pane, 'fetchSqlPage').mockRejectedValueOnce(new CmdError({ code: 'notFound', message: 'this result is gone', retryable: false }));
    await sql.loadMore();
    expect(sql.failure()).toBe('this result is gone');
    sql.open('SELECT 2');
    expect(sql.text()).toBe('SELECT 2');
    sql.setExplain(true);
    expect(sql.explain()).toBe(true);
  });

  it('works without a schema', async () => {
    const { pane } = provide();
    vi.spyOn(pane, 'describeSqlSchema').mockRejectedValue(new Error('down'));
    const sql = TestBed.inject(SqlConsoleStore);
    await sql.loadSchema();
    expect(sql.schema()).toBeNull();
  });
});

describe('Store pane', () => {
  it('shows every tab, confirms maintenance, and runs SQL with completion, errors and Open in Events', async () => {
    const { context } = provide();
    const open = vi.spyOn(TestBed.inject(DialogService), 'open').mockReturnValue({ closed: Promise.resolve(true) } as never);
    const f = TestBed.createComponent(StorePane);
    await settle(f, 8);
    expect(el(f, '[data-testid=store-overview]').textContent).toContain('Size on disk');
    expect(el(f, '[data-testid=store-activity]').textContent).toContain('Ready');
    el(f, '[data-testid=store-maintain-compact]').click();
    await settle(f, 8);
    expect(open).toHaveBeenCalled();
    expect(el(f, '[data-testid=store-notice]').textContent).toContain('Started');
    // Arrow keys move between tabs.
    el(f, '[data-testid=store-tab-overview]').dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowRight', bubbles: true }));
    await settle(f, 8);
    expect(all(f, '[data-testid=store-segments] tbody tr').length).toBeGreaterThan(0);
    for (const tab of ['fields', 'index', 'memory']) {
      el(f, `[data-testid=store-tab-${tab}]`).click();
      await settle(f, 8);
    }
    expect(text(f)).toContain('Cached analyses');
    expect(all(f, '[role=meter]').length).toBe(2);
    el(f, '[data-testid=store-tab-index]').click();
    await settle(f, 8);
    expect(el(f, '[data-testid=store-index]').textContent).toContain('Up to date');
    el(f, '[data-testid=store-tab-fields]').click();
    await settle(f, 8);
    expect(all(f, '[data-testid=store-fields] tbody tr').length).toBeGreaterThan(0);

    el(f, '[data-testid=store-tab-sql]').click();
    await settle(f, 8);
    const editor = el<HTMLTextAreaElement>(f, '[data-testid=sql-editor]');
    typeInto(editor, 'SELECT * FROM ev');
    editor.setSelectionRange(16, 16);
    typeInto(editor, 'SELECT * FROM ev');
    await settle(f);
    expect(el(f, '[data-testid=sql-completions]').textContent).toContain('events');
    editor.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowDown', bubbles: true }));
    editor.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowUp', bubbles: true }));
    editor.dispatchEvent(new KeyboardEvent('keydown', { key: 'Tab', bubbles: true }));
    await settle(f);
    expect(editor.value).toBe('SELECT * FROM events');
    editor.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', ctrlKey: true, bubbles: true }));
    await settle(f, 8);
    expect(all(f, '[data-testid=sql-results] tbody tr').length).toBeGreaterThan(0);
    expect(el(f, '[data-testid=sql-count]').textContent).toContain('rows');
    at(all<HTMLElement>(f, '[data-testid=sql-results] tbody tr'), 1).click();
    await settle(f);
    el(f, '[data-testid=sql-open-events]').click();
    expect(context.jumpTo).toHaveBeenCalledWith(expect.any(Number));
    // A refusal is shown with its span.
    typeInto(editor, 'DROP VIEW events');
    buttonByText(f, 'Run').click();
    await settle(f, 8);
    expect(el(f, '[data-testid=sql-error]').textContent).toContain('DROP');
    expect(el(f, '[data-testid=sql-error] mark').textContent).toBe('DROP');
    // The plan instead of rows.
    typeInto(editor, 'SELECT 1');
    el<HTMLInputElement>(f, '[data-testid=sql-explain]').click();
    buttonByText(f, 'Run').click();
    await settle(f, 8);
    expect(el(f, '[data-testid=sql-plan]').textContent).toContain('READ_PARQUET');
    // Saving puts the SQL next to the saved queries.
    el(f, '[data-testid=sql-save]').click();
    expect(at(TestBed.inject(SavedSqlStore).items(), 0).sql).toBe('SELECT 1');
    editor.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
  });

  it('shows a failed report with Retry', async () => {
    const { pane } = provide();
    vi.spyOn(pane, 'overview').mockRejectedValueOnce(new CmdError({ code: 'failed', message: 'the store worker stopped', retryable: true }));
    const f = TestBed.createComponent(StorePane);
    await settle(f, 8);
    expect(el(f, '[data-testid=store-error]').textContent).toContain('the store worker stopped');
    buttonByText(f, 'Retry').click();
    await settle(f, 8);
    expect(el(f, '[data-testid=store-overview]')).toBeTruthy();
  });
});

describe('Saved SQL', () => {
  it('is parsed leniently, saved in the project, listed with a badge and opens in the console', async () => {
    const { context } = provide();
    expect(parseSavedSql(null)).toEqual([]);
    expect(parseSavedSql([{ id: 'a', name: 'n', sql: 'SELECT 1' }, { id: 1 }, null])).toEqual([{ id: 'a', name: 'n', sql: 'SELECT 1' }]);
    const saved = TestBed.inject(SavedSqlStore);
    saved.save('', '  ');
    expect(saved.items()).toEqual([]);
    saved.save('', 'SELECT 1');
    saved.save('mine', 'SELECT 2');
    expect(saved.items().map((s) => s.name)).toEqual(['SQL 1', 'mine']);
    const ui = must(TestBed.inject(ProjectUiState).contributions.find((c) => c.key === 'savedSql'));
    const captured = ui.capture();
    must(ui.reset)();
    expect(saved.items()).toEqual([]);
    ui.restore(captured);
    expect(saved.items()).toHaveLength(2);
    const f = TestBed.createComponent(SavedQueries);
    await settle(f);
    const rows = all(f, '[data-testid=saved-sql]');
    expect(rows).toHaveLength(2);
    expect(at(rows, 0).textContent).toContain('SQL');
    buttonByText(f, 'Open').click();
    expect(TestBed.inject(SqlConsoleStore).text()).toBe('SELECT 1');
    expect(TestBed.inject(StorePaneStore).tab()).toBe('sql');
    expect(context.showView).toHaveBeenCalledWith(STORE_VIEW);
    saved.remove(at(saved.items(), 0).id);
    expect(saved.items()).toHaveLength(1);
  });
});

describe('provideStorePane', () => {
  it('offers the view and the palette commands', () => {
    const { context } = provide();
    expect(at(storePaneViews(), 0).id).toBe(STORE_VIEW);
    const pane = TestBed.inject(StorePaneStore);
    const [show, sqlCmd] = [at(storePaneCommands(pane), 0), at(storePaneCommands(pane), 1)];
    void sqlCmd.run(context);
    expect(pane.tab()).toBe('sql');
    void show.run(context);
    expect(pane.tab()).toBe('overview');
    expect(context.showView).toHaveBeenCalledWith(STORE_VIEW);
  });
});
