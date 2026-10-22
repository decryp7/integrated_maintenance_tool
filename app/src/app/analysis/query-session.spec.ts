import { TestBed } from '@angular/core/testing';
import { AnalysisBackend } from '../core/analysis-backend';
import { WorkspaceStore } from '../core/workspace.store';
import { CHECK_DELAY_MS, newQueryId, QuerySession } from './query-session';
import { QueryProgress, StreamedQueryOutcome } from '../core/api.types';
import { FakeContext, loadWorkspace, provideAnalysisTesting } from './test-setup';
import { QueryHistory } from './history-store';
import { COMMAND_CONTEXT } from '../extensions/command-registry';

const wait = (ms: number) => new Promise((r) => setTimeout(r, ms));

describe('QuerySession', () => {
  let ctx: FakeContext;
  let session: QuerySession;

  beforeEach(async () => {
    ctx = new FakeContext();
    TestBed.configureTestingModule({ providers: provideAnalysisTesting({ context: ctx }) });
    await loadWorkspace();
    session = TestBed.inject(QuerySession);
  });

  it('checks and completes shortly after the text changes, latest edit wins', async () => {
    session.edit('level >= wa', 11);
    session.edit('| nope', 6);
    await wait(CHECK_DELAY_MS + 60);
    expect(session.text()).toBe('| nope');
    expect(session.error()).toMatchObject({ message: "unknown stage 'nope'" });
    session.edit('lev', 3);
    await wait(CHECK_DELAY_MS + 60);
    expect(session.error()).toBeNull();
    expect(session.completions().map((c) => c.label)).toContain('level');
    session.dismissCompletions();
    expect(session.completions()).toEqual([]);
    session.edit('   ', 3);
    await wait(CHECK_DELAY_MS + 60);
    expect(session.error()).toBeNull();
  });

  it('can complete at the cursor without editing', async () => {
    session.text.set('x | ');
    await session.complete(4);
    expect(session.completions().map((c) => c.label)).toContain('stats');
  });

  it('ignores a stale response and survives backend failures', async () => {
    const backend = TestBed.inject(AnalysisBackend);
    vi.spyOn(backend, 'completeQuery').mockRejectedValueOnce(new Error('down'));
    session.edit('a', 1);
    await wait(CHECK_DELAY_MS + 60);
    expect(session.completions()).toEqual([]);
    session.edit('lev', 3);
    session.edit('level >', 7);
    await wait(CHECK_DELAY_MS + 60);
    expect(session.text()).toBe('level >');
  });

  it('applies an events result as the workspace ql and remembers it', async () => {
    session.text.set('level >= error');
    expect(await session.run()).toBe('events');
    expect(ctx.last('setQuery')).toEqual([{ ql: 'level >= error' }]);
    expect(session.matched()).toBeGreaterThan(0);
    expect(session.history.entries()).toEqual(['level >= error']);
    expect(session.running()).toBe(false);
    expect(session.table()).toBeNull();
  });

  it('keeps a table result for the results view and opens it', async () => {
    session.text.set('| stats count() by level');
    expect(await session.run()).toBe('table');
    expect(session.table()?.columns).toEqual(['level', 'count()']);
    expect(ctx.last('showView')).toEqual(['query-results']);
    expect(ctx.calls.some((c) => c.fn === 'setQuery')).toBe(false);
    expect(session.matched()).toBeNull();
    expect(session.tableText()).toBe('| stats count() by level');
    expect(session.tableStale()).toBeNull();
  });

  it('reports engine errors without touching the workspace', async () => {
    session.text.set('level >= warning | frob');
    expect(await session.run()).toBeNull();
    expect(session.error()).toMatchObject({ offset: 19, len: 4 });
    expect(session.runError()).toContain('unknown stage');
    expect(session.history.entries()).toEqual([]);
    expect(ctx.calls.some((c) => c.fn === 'setQuery')).toBe(false);
    session.edit('level', 5);
    expect(session.runError()).toBeNull();
  });

  it('reports command failures', async () => {
    vi.spyOn(TestBed.inject(AnalysisBackend), 'runQuery').mockRejectedValueOnce('IPC down');
    session.text.set('x');
    expect(await session.run()).toBeNull();
    expect(session.runError()).toBe('IPC down');
    expect(session.running()).toBe(false);
  });

  it('shows progress while running and resolves a cancel as a cancelled run, not an error', async () => {
    const backend = TestBed.inject(AnalysisBackend);
    let progress: ((p: QueryProgress) => void) | undefined;
    let finish: ((o: StreamedQueryOutcome) => void) | undefined;
    const ids: string[] = [];
    vi.spyOn(backend, 'runQueryStreaming').mockImplementation((_q, id, onProgress) => {
      ids.push(id);
      progress = onProgress;
      return new Promise((r) => (finish = r));
    });
    const cancel = vi.spyOn(backend, 'cancelQuery').mockImplementation(async () => (finish?.({ cancelled: true }), true));
    session.text.set('level >= warning');
    const run = session.run();
    expect(session.running()).toBe(true);
    progress?.({ stage: 'matching', done: 10, total: 100 });
    expect(session.progress()).toEqual({ stage: 'matching', done: 10, total: 100 });
    await session.cancel();
    expect(cancel).toHaveBeenCalledWith(ids[0]);
    expect(await run).toBeNull();
    expect(session.cancelled()).toBe(true);
    expect(session.runError()).toBeNull();
    expect(session.error()).toBeNull();
    expect(session.running()).toBe(false);
    expect(session.progress()).toBeNull();
    expect(ids[0]).toMatch(/^[A-Za-z0-9-]{1,64}$/);
    // Nothing to cancel when idle; the next run clears the note.
    cancel.mockClear();
    await session.cancel();
    expect(cancel).not.toHaveBeenCalled();
    vi.mocked(backend.runQueryStreaming).mockResolvedValueOnce({ result: { kind: 'events', ids: [1], fields: null } });
    expect(await session.run()).toBe('events');
    expect(session.cancelled()).toBe(false);
  });

  it('a new run replaces the one in flight, whose late outcome is ignored', async () => {
    const backend = TestBed.inject(AnalysisBackend);
    const pending: ((o: StreamedQueryOutcome) => void)[] = [];
    const progress: ((p: QueryProgress) => void)[] = [];
    vi.spyOn(backend, 'runQueryStreaming').mockImplementation((_q, _id, onProgress) => (progress.push(onProgress), new Promise((r) => pending.push(r))));
    const cancel = vi.spyOn(backend, 'cancelQuery').mockRejectedValue('gone');
    session.text.set('a');
    const first = session.run();
    session.text.set('b');
    const second = session.run();
    expect(cancel).toHaveBeenCalledTimes(1);
    progress[0]?.({ stage: 'matching', done: 1, total: 2 });
    expect(session.progress()).toBeNull();
    pending[0]?.({ result: { kind: 'events', ids: [9, 9, 9], fields: null } });
    expect(await first).toBeNull();
    expect(session.running()).toBe(true);
    pending[1]?.({ result: { kind: 'events', ids: [1], fields: null } });
    expect(await second).toBe('events');
    expect(session.matched()).toBe(1);
    // An empty run also drops the one in flight.
    session.text.set('c');
    const third = session.run();
    session.text.set('');
    expect(await session.run()).toBe('events');
    expect(session.running()).toBe(false);
    pending[2]?.({ cancelled: true });
    expect(await third).toBeNull();
    expect(session.cancelled()).toBe(false);
  });

  it('makes valid, distinct query ids', () => {
    const a = newQueryId(0);
    const b = newQueryId(0);
    expect(a).not.toBe(b);
    expect(newQueryId(Number.MAX_SAFE_INTEGER)).toMatch(/^[A-Za-z0-9-]{1,64}$/);
  });

  it('an empty query clears the applied ql', async () => {
    session.text.set('  ');
    expect(await session.run()).toBe('events');
    expect(ctx.last('setQuery')).toEqual([{ ql: '' }]);
    session.clearApplied();
    expect(ctx.last('setQuery')).toEqual([{ ql: '' }]);
  });

  it('loads text from history with the cursor at the end', async () => {
    session.load('café = 1');
    expect(session.text()).toBe('café = 1');
    session.requestFocus();
    expect(session.focusNonce()).toBe(1);
    TestBed.inject(QueryHistory);
  });
});

describe('QuerySession with the real command context', () => {
  it('puts ql on the workspace query and exposes it as applied', async () => {
    TestBed.configureTestingModule({ providers: provideAnalysisTesting() });
    const store = await loadWorkspace();
    expect(TestBed.inject(COMMAND_CONTEXT)).toBeDefined();
    const session = TestBed.inject(QuerySession);
    expect(session.applied()).toBe('');
    session.text.set('level >= error');
    await session.run();
    expect((store.query() as { ql?: string }).ql).toBe('level >= error');
    expect(session.applied()).toBe('level >= error');
    session.clearApplied();
    expect(session.applied()).toBe('');
    expect(TestBed.inject(WorkspaceStore)).toBe(store);
  });
});
