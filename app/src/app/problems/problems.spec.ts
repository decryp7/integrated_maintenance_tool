import { TestBed } from '@angular/core/testing';
import { all, buttonByText, el, FakeDock, provideFakeDock, provideTinyBackend, settle, text, tinyBackend } from '../../testing/helpers';
import { at } from '../../testing/must';
import { AiLauncher } from '../ai/ai-launcher';
import { DiagnosticRow, FixEvaluation } from '../core/api.types';
import { MockBackend } from '../core/mock/mock-backend';
import { WorkspaceStore } from '../core/workspace.store';
import { COMMAND_CONTEXT } from '../extensions/command-registry';
import { CommandContext } from '../extensions/tokens';
import { MockParsersBackend } from '../settings/parsers/mock-parsers-backend';
import { ParsersBackend } from '../settings/parsers/parsers-backend';
import { SourceFileActions } from '../sources/source-file-actions';
import { DialogService } from '../ui/dialog.service';
import { MockProblemsBackend } from './mock/mock-problems-backend';
import { ProblemsBadge } from './problems-badge';
import { ProblemsBackend, TauriProblemsBackend } from './problems-backend';
import {
  countProblems,
  countText,
  evaluationText,
  evaluationTone,
  groupProblems,
  kindHint,
  kindLabel,
  parserCanFix,
  PROBLEMS_VIEW,
  problemKey,
  problemsTitle,
  quickFixOf,
} from './problems-model';
import { ProblemsPanel, severityLabel } from './problems-panel';
import { problemsCommands, problemsViews, startProblemsTitle } from './provide-problems';
import { ProblemsStore } from './problems.store';
import { ProblemKind } from '../core/api.types';

const NO_TS = '/logs/legacy-no-timestamps.csv';
const FORMAT = '%a %b %d %H:%M:%S %Y';

const row = (patch: Partial<DiagnosticRow> = {}): DiagnosticRow => ({
  source: 1,
  sourceName: 'app.log',
  path: '/logs/app.log',
  kind: 'timestampUnparsed',
  severity: 'warning',
  count: 3,
  sampleLines: [2, 3],
  ...patch,
});

const KINDS: ProblemKind[] = [
  'lineTooLong',
  'eventTooLarge',
  'lineNotParsed',
  'timestampUnparsed',
  'fieldInvalid',
  'encoding',
  'unknownFormat',
  'truncatedArchive',
  'eventSplit',
  'unmatchedLine',
  'routeLimit',
];

const ctx = () => ({ showView: vi.fn(), jumpTo: vi.fn(async () => undefined), notifyError: vi.fn() }) as unknown as CommandContext;

function provide(backend: MockBackend = tinyBackend(), context = ctx(), dock = new FakeDock()) {
  const problems = new MockProblemsBackend(backend);
  const parsers = new MockParsersBackend(backend);
  TestBed.configureTestingModule({
    providers: [
      ...provideTinyBackend(backend),
      ...provideFakeDock(dock),
      { provide: ProblemsBackend, useValue: problems },
      { provide: ParsersBackend, useValue: parsers },
      { provide: COMMAND_CONTEXT, useValue: context },
    ],
  });
  return { backend, problems, parsers, context, dock };
}

/** The workspace with the tiny dataset and a file whose timestamps the engine cannot read. */
async function withUntimedFile(backend: MockBackend): Promise<void> {
  await TestBed.inject(WorkspaceStore).init();
  await backend.openPaths([NO_TS]);
  await TestBed.inject(WorkspaceStore).refreshSummary();
}

describe('problems model', () => {
  const rows = [
    row({ source: 1, kind: 'lineNotParsed', severity: 'info' }),
    row({ source: 1, kind: 'timestampUnparsed', severity: 'warning' }),
    row({ source: null, sourceName: 'x.bin', path: '/x.bin', kind: 'unknownFormat', count: 1, sampleLines: [], detail: 'no enabled parser reads this file' }),
    row({ source: 2, sourceName: 'b.log', path: '/b.log', kind: 'eventSplit', severity: 'critical' }),
  ];

  it('groups by file (most urgent first), counts by severity and titles the tab', () => {
    const groups = groupProblems(rows);
    expect(groups.map((g) => [g.key, g.severity, g.items.map((i) => i.row.kind)])).toEqual([
      ['source:1', 'warning', ['timestampUnparsed', 'lineNotParsed']],
      ['file:/x.bin', 'warning', ['unknownFormat']],
      ['source:2', 'critical', ['eventSplit']],
    ]);
    expect(problemKey(at(rows, 2))).toBe('file:/x.bin/unknownFormat');
    expect(countProblems(rows)).toEqual({ critical: 1, warning: 2, info: 1, total: 4 });
    expect(problemsTitle(countProblems(rows))).toBe('Problems (4)');
    expect(problemsTitle(countProblems([]))).toBe('Problems');
  });

  it('names every kind, says what happened and whether a parser change fixes it', () => {
    expect(new Set(KINDS.map(kindLabel)).size).toBe(KINDS.length);
    expect(KINDS.every((k) => kindHint(k).length > 0)).toBe(true);
    expect(KINDS.filter(parserCanFix)).toEqual(['lineNotParsed', 'timestampUnparsed', 'unknownFormat', 'unmatchedLine']);
    expect((['critical', 'warning', 'info'] as const).map(severityLabel)).toEqual(['Error', 'Warning', 'Information']);
  });

  it('counts events or lines', () => {
    expect(countText(row({ count: 1 }))).toBe('1 event');
    expect(countText(row({ count: 1204 }))).toBe('1,204 events');
    expect(countText(row({ kind: 'lineNotParsed', count: 2 }))).toBe('2 lines');
    expect(countText(row({ kind: 'lineTooLong', count: 1 }))).toBe('1 line');
    expect(countText(row({ kind: 'unknownFormat' }))).toBe('');
  });

  it('turns the engine fix into a quick fix for the source and words its evaluation', () => {
    expect(quickFixOf(row())).toBeNull();
    const fix = { title: 't', parserOverride: '[format]\n' };
    expect(quickFixOf(row({ fix }))).toEqual({ parserOverride: '[format]\n', sources: [1] });
    expect(quickFixOf(row({ fix, source: null }))).toEqual({ parserOverride: '[format]\n', sources: [] });
    const e = (fixed: number, checked: number, unchecked = 0): FixEvaluation => ({ fixed, checked, unchecked });
    expect(evaluationText(e(3, 3))).toBe('Fixes 3 of 3 checked events');
    expect(evaluationText(e(1, 3, 1200))).toBe('Fixes 1 of 3 checked events (1,200 more not checked)');
    expect(evaluationText(e(0, 0))).toBe('No stored events to check');
    expect([e(3, 3), e(1, 3), e(0, 3), e(0, 0)].map(evaluationTone)).toEqual(['good', 'partial', 'none', 'none']);
  });
});

describe('problems backends', () => {
  it('the Tauri backend calls the diagnostics commands', async () => {
    const call = vi.fn(async () => undefined) as never;
    const b = new TauriProblemsBackend(call);
    await b.problems();
    await b.evaluateFix({ parserOverride: '[format]\n', sources: [1] });
    await b.indexStatus();
    expect((call as unknown as { mock: { calls: unknown[] } }).mock.calls).toEqual([['import_problems'], ['evaluate_fix', { fix: { parserOverride: '[format]\n', sources: [1] } }], ['index_status']]);
  });

  it('the mock lists source problems, skipped lines, untimed events with a fix and unread files', async () => {
    const backend = tinyBackend();
    await backend.openPaths([NO_TS]);
    const mock = new MockProblemsBackend(backend);
    const rows = await mock.problems();
    expect(rows.map((r) => [r.sourceName, r.kind])).toEqual([
      ['app.log', 'lineNotParsed'],
      ['legacy-no-timestamps.csv', 'timestampUnparsed'],
      ['export.reg', 'unknownFormat'],
    ]);
    const untimed = rows.find((r) => r.kind === 'timestampUnparsed')!;
    expect(untimed.fix?.parserOverride).toContain(FORMAT);
    expect(untimed.sampleLines).toEqual([2, 3, 4]);
    expect(rows.find((r) => r.kind === 'unknownFormat')).toMatchObject({ source: null, detail: 'No importer' });

    const good = await mock.evaluateFix(quickFixOf(untimed)!);
    expect(good.fixed).toBe(good.checked);
    expect(good.checked).toBeGreaterThan(0);
    const wrong = await mock.evaluateFix({ parserOverride: '[format]\ntimestamp_formats = ["%H"]\n' });
    expect(wrong.fixed).toBe(0);
    expect(await mock.evaluateFix({ parserOverride: '[format]\n', sources: [0] })).toMatchObject({ checked: 0, notes: [expect.stringContaining('no stored events')] });
    await expect(mock.evaluateFix({ parserOverride: '[format' })).rejects.toMatchObject({ code: 'invalid' });
    await expect(mock.evaluateFix({ parserOverride: 'format = 1' })).rejects.toMatchObject({ code: 'invalid' });
    expect((await mock.indexStatus()).ready).toBe(true);
  });

  it('the mock parser draft merges a fix over the base and needs one', async () => {
    const backend = tinyBackend();
    const parsers = new MockParsersBackend(backend);
    const draft = await parsers.draft({ fileName: 'a.csv', base: 'generic', regex: null, template: null, timestampFormat: null, parserOverride: `[format]\ntimestamp_formats = ["${FORMAT}"]\n` });
    expect(draft).toMatchObject({ id: 'generic', origin: 'override' });
    expect(draft.toml).toContain(FORMAT);
    expect(draft.toml).toMatch(/version = "\d+\.\d+\.\d+"/);
    await expect(parsers.draft({ fileName: 'a.csv', base: null, regex: null, template: null, timestampFormat: null, parserOverride: '[format]\n' })).rejects.toMatchObject({ code: 'invalid' });
    await expect(parsers.draft({ fileName: 'a.csv', base: 'generic', regex: null, template: null, timestampFormat: null, parserOverride: '[format' })).rejects.toMatchObject({ code: 'invalid' });
  });
});

describe('ProblemsStore', () => {
  it('lists the problems when the workspace changes, previews and applies the fix', async () => {
    const { backend } = provide();
    await withUntimedFile(backend);
    const store = TestBed.inject(ProblemsStore);
    await store.load();
    expect(store.loaded()).toBe(true);
    expect(store.counts()).toMatchObject({ warning: 2, info: 1, total: 3 });
    const key = problemKey(store.rows().find((r) => r.kind === 'timestampUnparsed')!);

    store.select(key);
    expect(store.selected()?.row.kind).toBe('timestampUnparsed');
    await store.preview(key);
    const p = store.previews()[key];
    expect(p?.state).toBe('done');
    // A row without a fix has nothing to preview.
    const plain = problemKey(store.rows().find((r) => r.kind === 'lineNotParsed')!);
    await store.preview(plain);
    expect(store.previews()[plain]).toBeUndefined();

    expect(await store.apply(key)).toBe(true);
    expect(store.notice()).toMatch(/^Changed the parser .* and read its files again\.$/);
    expect(store.rows().some((r) => r.kind === 'timestampUnparsed')).toBe(false);
    expect(store.selected()).toBeNull();
    expect(store.previews()).toEqual({});
    expect(await store.apply('nope')).toBe(false);
  });

  it('shows why listing, previewing or applying failed', async () => {
    const { backend, problems, parsers } = provide();
    await withUntimedFile(backend);
    const store = TestBed.inject(ProblemsStore);
    const failing = vi.spyOn(problems, 'problems').mockRejectedValueOnce('engine busy');
    await store.load();
    expect(store.loadError()).toContain('engine busy');
    failing.mockRestore();
    await store.load();
    expect(store.loadError()).toBeNull();
    const key = problemKey(store.rows().find((r) => r.fix)!);
    vi.spyOn(problems, 'evaluateFix').mockRejectedValueOnce('no store');
    await store.preview(key);
    expect(store.previews()[key]).toEqual({ state: 'failed', error: expect.stringContaining('no store') });
    vi.spyOn(parsers, 'save').mockRejectedValueOnce('disk full');
    expect(await store.apply(key)).toBe(false);
    expect(store.actionError()).toContain('disk full');
    expect(store.applying()).toBeNull();
    store.dismiss();
    expect(store.actionError()).toBeNull();
  });

  it('jumps to the stored event at a line, else opens the file at that line', async () => {
    const { backend, context } = provide();
    await withUntimedFile(backend);
    const store = TestBed.inject(ProblemsStore);
    await store.load();
    const untimed = store.rows().find((r) => r.kind === 'timestampUnparsed')!;
    await store.jumpTo(problemKey(untimed), 2);
    expect(context.showView).toHaveBeenCalledWith('events');
    expect(context.jumpTo).toHaveBeenCalledTimes(1);

    const run = vi.spyOn(TestBed.inject(SourceFileActions), 'run').mockResolvedValue(true);
    await store.jumpTo(problemKey(untimed), 99_999);
    expect(run).toHaveBeenCalledWith(expect.objectContaining({ id: untimed.source }), 'openAtLine', 99_999);

    const unread = store.rows().find((r) => r.kind === 'unknownFormat')!;
    await store.jumpTo(problemKey(unread), 1);
    expect(store.actionError()).toContain('was not imported');
    await store.jumpTo('nope', 1);
  });

  it('"Ask the AI to fix" proposes a parser from the event at the line or the start of the file', async () => {
    const { backend } = provide();
    await withUntimedFile(backend);
    const store = TestBed.inject(ProblemsStore);
    await store.load();
    const propose = vi.spyOn(TestBed.inject(AiLauncher), 'propose').mockResolvedValue();
    const untimed = store.rows().find((r) => r.kind === 'timestampUnparsed')!;
    await store.askAi(problemKey(untimed));
    expect(propose).toHaveBeenLastCalledWith('parser', { recordId: expect.any(Number), fileName: 'legacy-no-timestamps.csv' });

    const head = vi.spyOn(backend, 'readFileHead').mockResolvedValue('first line\n\nsecond line\n');
    const skipped = store.rows().find((r) => r.kind === 'lineNotParsed')!;
    await store.askAi(problemKey(skipped));
    expect(head).toHaveBeenCalledWith('/logs/web02/app.log', expect.any(Number));
    expect(propose).toHaveBeenLastCalledWith('parser', { sample: 'first line\nsecond line', fileName: 'app.log' });

    head.mockRejectedValueOnce('outside the workspace');
    await store.askAi(problemKey(skipped));
    expect(store.actionError()).toContain('outside the workspace');
    await store.askAi('nope');
  });
});

describe('Problems pane', () => {
  it('shows the problems by file, selects with the keyboard, previews and applies the quick fix', async () => {
    const { backend } = provide();
    await withUntimedFile(backend);
    const f = TestBed.createComponent(ProblemsPanel);
    await settle(f);
    await TestBed.inject(ProblemsStore).load();
    await settle(f);
    expect(el(f, '[data-testid=problems-summary]').textContent).toContain('2 warnings, 1 messages');
    const tree = el(f, '[role=tree]');
    const items = () => all<HTMLElement>(f, '[role=treeitem]');
    expect(items().map((i) => i.getAttribute('aria-level'))).toEqual(['1', '2', '1', '2', '1', '2']);
    expect(items().filter((i) => i.tabIndex === 0)).toHaveLength(1);
    expect(text(f)).toContain('Select a problem');

    // Left folds the first file, Right unfolds it, Down moves to its problem, Enter selects it.
    tree.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowLeft', bubbles: true }));
    await settle(f);
    expect(items()).toHaveLength(5);
    tree.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowRight', bubbles: true }));
    await settle(f);
    expect(items()).toHaveLength(6);
    tree.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowDown', bubbles: true }));
    tree.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }));
    await settle(f);
    expect(el(f, '[data-testid=problem-title]').textContent).toContain('Lines that are not events');
    expect(el(f, '[data-testid=ask-ai-fix]')).toBeTruthy();
    tree.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowLeft', bubbles: true }));
    await settle(f);
    expect(document.activeElement?.getAttribute('aria-level')).toBe('1');

    // The untimed file's problem has a quick fix.
    const untimed = items().find((i) => i.textContent?.includes('without a readable timestamp'))!;
    untimed.click();
    await settle(f);
    expect(untimed.getAttribute('aria-selected')).toBe('true');
    expect(all(f, '[data-testid=problem-line]').map((b) => b.getAttribute('aria-label'))).toEqual(['Go to line 2', 'Go to line 3', 'Go to line 4']);
    expect(el(f, '[data-testid=quick-fix]').textContent).toContain(FORMAT);
    el<HTMLButtonElement>(f, '[data-testid=fix-preview]').click();
    await settle(f, 8);
    expect(el(f, '[data-testid=fix-evaluation]').getAttribute('data-tone')).toBe('good');
    el<HTMLButtonElement>(f, '[data-testid=fix-apply]').click();
    await settle(f, 12);
    expect(text(f)).toContain('read its files again');
    expect(text(f)).not.toContain('without a readable timestamp');
  });

  it('says when there are no problems, and shows a failed listing with Retry', async () => {
    const backend = tinyBackend({ dataset: () => ({ sources: [], unrecognized: [], records: [] }) });
    const { problems } = provide(backend);
    const failing = vi.spyOn(problems, 'problems').mockRejectedValueOnce('engine busy');
    const f = TestBed.createComponent(ProblemsPanel);
    await settle(f);
    expect(text(f)).toContain('could not be listed: engine busy');
    buttonByText(f, 'Retry').click();
    await settle(f);
    expect(el(f, '[data-testid=problems-empty]')).toBeTruthy();
    failing.mockRestore();
  });

  it('shows the search index while it is not ready, and opens the format wizard for an unread file', async () => {
    const { problems } = provide();
    vi.spyOn(problems, 'indexStatus').mockResolvedValue({ indexed: 5, total: 10, ready: false });
    const open = vi.spyOn(TestBed.inject(DialogService), 'open').mockReturnValue({ closed: Promise.resolve(undefined) } as never);
    const f = TestBed.createComponent(ProblemsPanel);
    await TestBed.inject(WorkspaceStore).init();
    await TestBed.inject(ProblemsStore).load();
    await settle(f);
    expect(el(f, '[data-testid=problems-index]').textContent).toContain('5 of 10 events indexed');
    all<HTMLElement>(f, '[role=treeitem]')
      .find((i) => i.textContent?.includes('no parser reads it'))!
      .click();
    await settle(f);
    expect(text(f)).toContain('No importer');
    buttonByText(f, 'Set up format…').click();
    await settle(f, 8);
    await vi.waitFor(() => expect(open).toHaveBeenCalledWith(expect.anything(), expect.objectContaining({ inputs: { path: '/logs/system/export.reg', name: 'export.reg' } })), { timeout: 5_000 });
  });
});

describe('Problems badge, views and commands', () => {
  it('the status-bar badge counts the problems and opens the pane', async () => {
    const { context } = provide();
    const f = TestBed.createComponent(ProblemsBadge);
    await TestBed.inject(WorkspaceStore).init();
    await TestBed.inject(ProblemsStore).load();
    await settle(f);
    const badge = el<HTMLButtonElement>(f, '[data-testid=status-problems]');
    expect(badge.getAttribute('aria-label')).toBe('2 problems in the opened files: show the Problems pane');
    expect(badge.textContent).toContain('2');
    badge.click();
    expect(context.showView).toHaveBeenCalledWith(PROBLEMS_VIEW);
  });

  it('offers the pane in the View menu and the palette, and counts in its tab title', async () => {
    const { dock } = provide();
    const [view] = problemsViews();
    expect(view).toMatchObject({ id: PROBLEMS_VIEW, category: 'core', defaultPosition: 'bottom' });
    expect(await view!.component()).toBe(ProblemsPanel);
    const store = TestBed.inject(ProblemsStore);
    const context = TestBed.inject(COMMAND_CONTEXT);
    const commands = problemsCommands(store);
    await at(commands, 0).run(context);
    expect(context.showView).toHaveBeenCalledWith(PROBLEMS_VIEW);
    const load = vi.spyOn(store, 'load');
    await at(commands, 1).run(context);
    expect(load).toHaveBeenCalled();

    TestBed.runInInjectionContext(() => startProblemsTitle());
    await TestBed.inject(WorkspaceStore).init();
    await store.load();
    TestBed.tick();
    expect(dock.calls.filter((c) => c.startsWith('title:problems'))).toEqual([]);
    dock.openIds.set(new Set([PROBLEMS_VIEW]));
    TestBed.tick();
    expect(dock.calls).toContain('title:problems:Problems (2)');
  });
});
