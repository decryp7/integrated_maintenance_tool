import { TestBed } from '@angular/core/testing';
import { AnalysisBackend } from '../core/analysis-backend';
import { WorkspaceStore } from '../core/workspace.store';
import { CHECK_DELAY_MS, QuerySession } from './query-session';
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
