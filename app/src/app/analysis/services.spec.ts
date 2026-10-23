import { TestBed } from '@angular/core/testing';
import { ViewRegistry } from '../extensions/view-registry';
import { CommandRegistry } from '../extensions/command-registry';
import { DebriefBackend } from '../core/backend';
import { Logger } from '../core/logger';
import { AnalysisBackend } from '../core/analysis-backend';
import { WorkspaceStore } from '../core/workspace.store';
import { MemoryLogger, settle } from '../../testing/helpers';
import { AnalysisActions } from './analysis-actions';
import { AnalysisCommands } from './analysis-commands';
import { AnalysisTargets, windowQuery } from './analysis-targets';
import { AnalysisTime } from './analysis-time';
import { ANALYSIS_VIEWS, createAnalysisBackend, provideAnalysis } from './analysis.plugin';
import { QuerySession } from './query-session';
import { TimeCursor } from './time-cursor';
import { FakeContext, loadWorkspace, provideAnalysisTesting, stubBackend } from './test-setup';
import { MockBackend } from '../core/mock/mock-backend';
import { MockAnalysisBackend } from './mock/mock-analysis-backend';
import { QlQuery } from '../core/api.types';
import { EMPTY_QUERY } from '../core/workspace.store';
import { DEMO_START } from '../core/mock/mock-data';


describe('AnalysisTargets', () => {
  it('resolves the current range: selection, then query range, then the workspace', async () => {
    TestBed.configureTestingModule({ providers: provideAnalysisTesting() });
    const targets = TestBed.inject(AnalysisTargets);
    expect(targets.currentRange()).toBeNull();
    const store = await loadWorkspace();
    const w = store.summary()!;
    expect(targets.currentRange()).toEqual({ from: w.from, to: w.to });
    store.patchQuery({ from: DEMO_START + 1000, to: DEMO_START + 5000 });
    expect(targets.currentRange()).toEqual({ from: DEMO_START + 1000, to: DEMO_START + 5000 });
    TestBed.inject(TimeCursor).select({ from: 9, to: 20 });
    expect(targets.currentRange()).toEqual({ from: 9, to: 20 });
    TestBed.inject(TimeCursor).select({ from: 9, to: 9 });
    expect(targets.currentRange()).toEqual({ from: DEMO_START + 1000, to: DEMO_START + 5000 });
    store.patchQuery({ from: 10, to: 5 });
    expect(targets.currentRange()).toBeNull();
    expect(targets.query()).toMatchObject({ from: 10 });
  });

  it('holds the moment, windows and field', () => {
    TestBed.configureTestingModule({ providers: provideAnalysisTesting() });
    const t = TestBed.inject(AnalysisTargets);
    t.setMoment({ from: 1, to: 2 });
    t.setWindowA({ from: 1, to: 2 });
    t.setWindowB({ from: 3, to: 4 });
    t.setField('f');
    expect([t.moment(), t.windowA(), t.windowB(), t.field()]).toEqual([{ from: 1, to: 2 }, { from: 1, to: 2 }, { from: 3, to: 4 }, 'f']);
    t.setWindows(null, null);
    expect([t.windowA(), t.windowB()]).toEqual([null, null]);
    const q = { ...EMPTY_QUERY, ql: 'x' } as QlQuery;
    expect(windowQuery(q, { from: 5, to: 6 })).toMatchObject({ from: 5, to: 6, ql: 'x' });
    expect(windowQuery(q, null)).toBe(q);
  });
});

describe('AnalysisTime', () => {
  it('formats in the zone the user picked', async () => {
    TestBed.configureTestingModule({ providers: provideAnalysisTesting() });
    const store = await loadWorkspace();
    store.setTimezone('utc');
    const time = TestBed.inject(AnalysisTime);
    expect(time.format(Date.UTC(2026, 9, 14, 10, 41, 22, 5))).toBe('14 Oct 10:41:22');
    expect(time.format(Date.UTC(2026, 9, 14, 10, 41, 22, 5), true)).toBe('14 Oct 10:41:22.005');
    expect(time.format(null)).toBe('—');
    expect(time.format(undefined)).toBe('—');
    expect(time.range(null)).toBe('—');
    expect(time.range({ from: 0, to: 90_000 })).toContain('(1 min 30 s)');
  });
});

describe('AnalysisActions', () => {
  let ctx: FakeContext;
  let actions: AnalysisActions;

  beforeEach(async () => {
    ctx = new FakeContext();
    TestBed.configureTestingModule({ providers: provideAnalysisTesting({ context: ctx, backend: stubBackend() }) });
    await loadWorkspace();
    actions = TestBed.inject(AnalysisActions);
  });

  it('forwards through the command context', () => {
    actions.setQl('a');
    actions.zoomTo({ from: 1, to: 2 });
    actions.filterField('F', 'v');
    actions.showView('moment');
    actions.fail('bad');
    expect(ctx.calls.map((c) => [c.fn, c.args])).toEqual([
      ['setQuery', [{ ql: 'a' }]],
      ['setQuery', [{ from: 1, to: 2 }]],
      ['addFilter', [{ field: 'F', op: 'eq', value: 'v' }]],
      ['showView', ['moment']],
      ['notifyError', ['bad']],
    ]);
  });

  it('jumps to a record through the command context', async () => {
    await actions.jumpTo(3);
    expect(ctx.last('jumpTo')).toEqual([3]);
  });

  it('reveals a sample after zooming, so the grid shows it under the zoomed query', async () => {
    await actions.revealSample({ from: 1, to: 2 }, undefined);
    expect(ctx.calls.map((c) => c.fn)).toEqual(['setQuery']);
    await actions.revealSample({ from: 1, to: 2 }, 3);
    expect(ctx.calls.map((c) => [c.fn, c.args])).toEqual([
      ['setQuery', [{ from: 1, to: 2 }]],
      ['setQuery', [{ from: 1, to: 2 }]],
      ['jumpTo', [3]],
    ]);
  });

  it('ANDs a clause with the workspace ql', () => {
    const store = TestBed.inject(WorkspaceStore);
    actions.narrowQl('a = "1"');
    expect(ctx.last('setQuery')).toEqual([{ ql: 'a = "1"' }]);
    store.patchQuery({ ql: 'x = 1 or y = 2' } as Partial<QlQuery>);
    actions.narrowQl('a = "1"');
    expect(ctx.last('setQuery')).toEqual([{ ql: '(x = 1 or y = 2) and a = "1"' }]);
  });

  it('filters by an id through correlate, trying each alternative field', async () => {
    expect(await actions.filterById('A|B', 'req-1')).toBe(3);
    expect(ctx.last('addFilter')).toEqual([{ field: 'A', op: 'eq', value: 'req-1' }]);
    expect(await actions.filterById(' | ', 'nope')).toBe(0);
    expect(ctx.last('notifyError')).toEqual(['No events carry the id nope.']);
  });
});

describe('AnalysisCommands', () => {
  async function setup() {
    TestBed.configureTestingModule({ providers: [...provideAnalysisTesting()] });
    const store = await loadWorkspace();
    return { store, cmds: TestBed.inject(AnalysisCommands), targets: TestBed.inject(AnalysisTargets), cursor: TestBed.inject(TimeCursor), ctx: new FakeContext() };
  }
  const find = (cmds: AnalysisCommands, id: string) => cmds.commands.find((c) => c.id === id)!;

  it('registers unique, documented commands', async () => {
    const { cmds } = await setup();
    const ids = cmds.commands.map((c) => c.id);
    expect(new Set(ids).size).toBe(ids.length);
    expect(ids).toEqual(expect.arrayContaining(['query.focus', 'moment.analyse', 'moment.selected', 'compare.previous', 'compare.beforeAfter', 'fieldstats.show']));
    expect(cmds.commands.every((c) => c.title && c.category)).toBe(true);
  });

  it('show commands open their view and need a loaded workspace', async () => {
    const { cmds, ctx } = await setup();
    for (const [id, view] of [['query.results', 'query-results'], ['anomalies.show', 'anomalies'], ['patterns.show', 'patterns'], ['exceptions.show', 'exceptions'], ['compare.show', 'compare'], ['fieldstats.show', 'field-stats'], ['traces.show', 'traces']] as [string, string][]) {
      const c = find(cmds, id);
      expect(c.when!({ ...ctx, summary: null } as never)).toBe(false);
      expect(c.when!({ ...ctx, summary: {} } as never)).toBe(true);
      await c.run(ctx);
      expect(ctx.last('showView')).toEqual([view]);
    }
  });

  it('focuses the search field, runs and clears the query', async () => {
    const { cmds, ctx } = await setup();
    const session = TestBed.inject(QuerySession);
    await find(cmds, 'query.focus').run(ctx);
    expect(session.focusNonce()).toBe(1);
    session.text.set('level >= error');
    await find(cmds, 'query.run').run(ctx);
    expect(session.matched()).toBeGreaterThan(0);
    await find(cmds, 'query.clear').run(ctx);
    expect(session.matched()).toBeNull();
  });

  it('opens the moment of the current range or the brushed selection', async () => {
    const { cmds, ctx, targets, cursor, store } = await setup();
    const analyse = find(cmds, 'moment.analyse');
    const selected = find(cmds, 'moment.selected');
    expect(selected.when!(ctx)).toBe(false);
    analyse.run(ctx);
    expect(targets.moment()).toEqual({ from: store.summary()!.from, to: store.summary()!.to });
    expect(ctx.last('showView')).toEqual(['moment']);
    cursor.select({ from: 100, to: 200 });
    expect(selected.when!(ctx)).toBe(true);
    selected.run(ctx);
    expect(targets.moment()).toEqual({ from: 100, to: 200 });
  });

  it('prepares compare windows', async () => {
    const { cmds, ctx, targets, cursor } = await setup();
    const range = targets.currentRange()!;
    find(cmds, 'compare.previous').run(ctx);
    // The range is all the data, so there is nothing before it: its two halves are compared.
    const half = Math.round((range.from + range.to) / 2);
    expect(targets.windowA()).toEqual({ from: range.from, to: half });
    expect(targets.windowB()).toEqual({ from: half, to: range.to });
    expect(ctx.last('showView')).toEqual(['compare']);
    find(cmds, 'compare.beforeAfter').run(ctx);
    const mid = Math.round((range.from + range.to) / 2);
    expect(targets.windowA()).toEqual({ from: mid - 300_000, to: mid });
    cursor.select({ from: 5_000_000, to: 6_000_000 });
    find(cmds, 'compare.beforeAfter').run(ctx);
    expect(targets.windowB()).toEqual({ from: 5_000_000, to: 5_300_000 });
  });

  it('does nothing to the windows before a workspace is loaded', () => {
    TestBed.configureTestingModule({ providers: provideAnalysisTesting() });
    const cmds = TestBed.inject(AnalysisCommands);
    const targets = TestBed.inject(AnalysisTargets);
    const ctx = new FakeContext();
    find(cmds, 'compare.previous').run(ctx);
    find(cmds, 'compare.beforeAfter').run(ctx);
    expect(targets.windowA()).toBeNull();
  });
});

describe('analysis plugin', () => {
  it('registers views and commands through the extension points', async () => {
    TestBed.configureTestingModule({ providers: [provideAnalysis(), { provide: DebriefBackend, useFactory: () => new MockBackend({ latency: false }) }, { provide: Logger, useFactory: () => new MemoryLogger() }] });
    await loadWorkspace();
    const views = TestBed.inject(ViewRegistry);
    expect(views.all.map((v) => v.id)).toEqual(ANALYSIS_VIEWS.map((v) => v.id));
    expect(views.available().map((v) => v.id)).not.toContain('query');
    const commands = TestBed.inject(CommandRegistry);
    expect(commands.find('moment.analyse')).toBeDefined();
    expect(await commands.run('anomalies.show')).toBe(true);
    // The browser build gets the mock backend, wrapped so failures are logged.
    expect(TestBed.inject(AnalysisBackend)).toBeDefined();
    expect(await TestBed.inject(AnalysisBackend).checkQuery('| nope')).not.toBeNull();
  });

  it('loads every view component lazily', async () => {
    for (const v of ANALYSIS_VIEWS) expect(typeof (await v.component())).toBe('function');
    expect(ANALYSIS_VIEWS.map((v) => v.id)).not.toContain('query');
  });

  it('createAnalysisBackend picks the mock outside Tauri', () => {
    TestBed.configureTestingModule({ providers: provideAnalysisTesting() });
    const backend = TestBed.runInInjectionContext(() => createAnalysisBackend());
    expect(backend).toBeInstanceOf(MockAnalysisBackend);
    void settle;
    void WorkspaceStore;
  });
});
