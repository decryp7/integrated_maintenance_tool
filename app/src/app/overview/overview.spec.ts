import { TestBed } from '@angular/core/testing';
import { all, buttonByText, FakeDock, pluginInfo, provideFakeDock, provideTinyBackend, settle, text } from '../../testing/helpers';
import { ExceptionGroup, Finding, fullCoverage, WorkspaceSummary } from '../core/api.types';
import { AnalysisBackend } from '../core/analysis-backend';
import { WorkspaceStore } from '../core/workspace.store';
import { COMMAND_CONTEXT } from '../extensions/command-registry';
import { CommandRegistry } from '../extensions/command-registry';
import { CommandContext } from '../extensions/tokens';
import { ViewRegistry } from '../extensions/view-registry';
import { FormatWizard } from '../format/format-wizard';
import { ShareBackend } from '../share/share-backend';
import { DialogService } from '../ui/dialog.service';
import { OverviewActions } from './overview-actions';
import { actionButtons, groupBySeverity, overviewStats, panelTarget, pluginProblems, zoomWindow } from './overview-model';
import { OVERVIEW_AUTO_OPEN_KEY, OverviewPrefs } from './overview-prefs';
import { OverviewView } from './overview-view';
import { OverviewAutoOpen, provideOverview } from './overview.plugin';
import { at } from '../../testing/must';

const crash: Finding = { kind: 'crash', severity: 'critical', title: 'Crash', detail: 'WEB01 stopped', action: { kind: 'zoom', from: 1000, to: 2000, recordId: 7 } };
const exc: Finding = { kind: 'exception', severity: 'warning', title: 'TimeoutException ×9', detail: 'on 2 instances', action: { kind: 'exception', fingerprint: 'fp1', recordId: null } };
const file: Finding = { kind: 'fileNotOpened', severity: 'info', title: 'File not opened', detail: 'x.reg: no importer', action: { kind: 'setupFormat', path: '/logs/x.reg' } };
const panel: Finding = { kind: 'dropped', severity: 'info', title: 'Dropped', detail: '3 dropped', action: { kind: 'openPanel', panel: 'pipeline' } };
const bare: Finding = { severity: 'info', title: 'Note', detail: 'nothing to do', action: null };

class FakeCtx implements CommandContext {
  calls: string[] = [];
  summary = null;
  openPaths = async () => undefined;
  setQuery = (p: object) => void this.calls.push(`query:${JSON.stringify(p)}`);
  addFilter = (f: object) => void this.calls.push(`filter:${JSON.stringify(f)}`);
  selectRecord = () => undefined;
  jumpTo = async (id: number) => void this.calls.push(`jump:${id}`);
  showView = (id: string) => void this.calls.push(`show:${id}`);
  notifyError = (m: string) => void this.calls.push(`error:${m}`);
}

const group = (patch: Partial<ExceptionGroup> = {}): ExceptionGroup => ({
  fingerprint: 'fp1',
  typeName: 'System.TimeoutException',
  message: 'timed out',
  topFrame: null,
  count: 9,
  instances: [],
  firstTs: null,
  lastTs: null,
  sampleIds: [4],
  ...patch,
});

describe('overview model', () => {
  it('groups by severity, most severe first, skipping empty groups', () => {
    const groups = groupBySeverity([file, crash, exc, panel]);
    expect(groups.map((g) => [g.label, g.findings.length])).toEqual([
      ['Critical', 1],
      ['Warning', 1],
      ['Info', 2],
    ]);
    expect(groupBySeverity([])).toEqual([]);
  });

  it('describes plugin problems', () => {
    const problems = pluginProblems([
      pluginInfo({ id: 'ok' }),
      pluginInfo({ id: 'q', status: 'quarantined', lastError: 'boom' }),
      pluginInfo({ id: 'q2', status: 'quarantined' }),
      pluginInfo({ id: 'i', status: 'inactive', inactiveReason: 'needs x' }),
      pluginInfo({ id: 'i2', status: 'inactive' }),
      pluginInfo({ id: 'f', failures: 1, lastError: 'oops' }),
      pluginInfo({ id: 'f2', failures: 2 }),
      pluginInfo({ id: 'w', warnings: ['optional dep missing'] }),
    ]);
    expect(problems.map((p) => p.problem)).toEqual(['disabled after crashes: boom', 'disabled after crashes', 'needs x', 'not running', '1 contained error: oops', '2 contained errors', 'optional dep missing']);
  });

  it('computes the headline numbers', () => {
    const summary = { sources: [{}, {}], total: 50, from: 1, to: 9, levels: { Error: 3, Fatal: 1 }, unrecognized: [{ path: 'a', name: 'a', reason: 'r' }], plugins: [] } as unknown as WorkspaceSummary;
    const s = overviewStats(summary, 4, [crash, crash, exc]);
    expect(s).toMatchObject({ sources: 2, events: 50, instances: 4, from: 1, to: 9, errors: 4, crashes: 2 });
    expect(overviewStats({ ...summary, levels: {} } as unknown as WorkspaceSummary, 0, []).errors).toBe(0);
  });

  it('offers buttons per action kind', () => {
    expect(actionButtons(crash.action).map((b) => b.label)).toEqual(['Jump to time', 'Show these events']);
    expect(actionButtons(exc.action).map((b) => b.label)).toEqual(['Show events', 'Open Exceptions']);
    expect(actionButtons(file.action).map((b) => b.label)).toEqual(['Set up format…']);
    expect(actionButtons(panel.action).map((b) => b.label)).toEqual(['Open Profile pipeline']);
    expect(actionButtons(null)).toEqual([]);
    expect(panelTarget('plugins')).toEqual({ view: 'plugins', title: 'Plugins' });
    expect(panelTarget('traces')).toEqual({ view: 'traces', title: 'Traces' });
  });

  it('widens a zoom window to at least a minute with margins', () => {
    const w = zoomWindow(100_000, 100_000);
    expect(w.to - w.from).toBe(90_000);
    expect(w.from).toBeLessThan(100_000);
    const big = zoomWindow(0, 400_000);
    expect(big).toEqual({ from: -100_000, to: 500_000 });
  });
});

describe('OverviewPrefs', () => {
  afterEach(() => localStorage.removeItem(OVERVIEW_AUTO_OPEN_KEY));

  it('defaults to not opening, remembers the choice and survives broken storage', () => {
    expect(TestBed.inject(OverviewPrefs).autoOpen()).toBe(false);
    TestBed.inject(OverviewPrefs).setAutoOpen(true);
    expect(localStorage.getItem(OVERVIEW_AUTO_OPEN_KEY)).toBe('true');
    TestBed.resetTestingModule();
    expect(TestBed.inject(OverviewPrefs).autoOpen()).toBe(true);
    TestBed.resetTestingModule();
    const get = vi.spyOn(Storage.prototype, 'getItem').mockImplementation(() => {
      throw new Error('denied');
    });
    const set = vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => {
      throw new Error('denied');
    });
    const prefs = TestBed.inject(OverviewPrefs);
    expect(prefs.autoOpen()).toBe(false);
    prefs.setAutoOpen(false);
    expect(prefs.autoOpen()).toBe(false);
    get.mockRestore();
    set.mockRestore();
  });
});

describe('OverviewActions', () => {
  function setup(groups: ExceptionGroup[] | { reject: unknown } = [group()]) {
    const ctx = new FakeCtx();
    const analysis = { queryExceptions: vi.fn(async () => (Array.isArray(groups) ? { result: groups, coverage: fullCoverage(groups.length) } : Promise.reject(groups.reject))) };
    TestBed.configureTestingModule({
      providers: [provideTinyBackend(), provideFakeDock(), { provide: COMMAND_CONTEXT, useValue: ctx }, { provide: AnalysisBackend, useValue: analysis }],
    });
    return { ctx, actions: TestBed.inject(OverviewActions) };
  }

  it('jumps to the time and the event, or shows the range in the grid', async () => {
    const { ctx, actions } = setup();
    await actions.run(crash, 'jump');
    expect(ctx.calls[0]).toMatch(/^query:/);
    expect(ctx.calls[1]).toBe('jump:7');
    ctx.calls = [];
    await actions.run({ ...crash, action: { kind: 'zoom', from: 1, to: 2, recordId: null } }, 'jump');
    expect(ctx.calls).toHaveLength(1);
    ctx.calls = [];
    await actions.run(crash, 'showRange');
    expect(ctx.calls).toEqual(['query:{"from":1000,"to":2000}', 'show:events']);
  });

  it('filters to an exception group found by fingerprint', async () => {
    const { ctx, actions } = setup();
    await actions.run(exc, 'showException');
    expect(ctx.calls).toEqual(['filter:{"field":"@exception","op":"contains","value":"System.TimeoutException"}', 'show:events', 'jump:4']);
    ctx.calls = [];
    await actions.run({ ...exc, action: { kind: 'exception', fingerprint: 'fp1', recordId: 9 } }, 'showException');
    expect(ctx.calls.at(-1)).toBe('jump:9');
    await actions.run(exc, 'openExceptions');
    expect(ctx.calls.at(-1)).toBe('show:exceptions');
  });

  it('explains an exception group that is gone, and one without samples', async () => {
    const { ctx, actions } = setup([group({ fingerprint: 'other' }), group({ fingerprint: 'fp2', sampleIds: [] })]);
    await actions.run(exc, 'showException');
    expect(ctx.calls[0]).toMatch(/^error:That exception is not in the current view/);
    ctx.calls = [];
    await actions.run({ ...exc, action: { kind: 'exception', fingerprint: 'fp2', recordId: null } }, 'showException');
    expect(ctx.calls).toEqual([expect.stringMatching(/^filter:/), 'show:events']);
  });

  it('reports a failing engine call instead of throwing', async () => {
    const { ctx, actions } = setup({ reject: new Error('engine down') });
    await actions.run(exc, 'showException');
    expect(ctx.calls).toEqual(['error:TimeoutException ×9: engine down']);
    TestBed.resetTestingModule();
    const second = setup({ reject: 'plain text' });
    await second.actions.run(exc, 'showException');
    expect(second.ctx.calls).toEqual(['error:TimeoutException ×9: plain text']);
  });

  it('opens the format wizard and panels; ignores mismatched or missing actions', async () => {
    const { ctx, actions } = setup();
    await actions.run(file, 'setupFormat');
    const stack = TestBed.inject(DialogService).stack();
    expect(at(stack, 0).component).toBe(FormatWizard);
    expect(at(stack, 0).options.inputs).toEqual({ path: '/logs/x.reg', name: 'x.reg' });
    await actions.run(panel, 'openPanel');
    expect(ctx.calls).toEqual(['show:profile']);
    const other = { ...bare, action: { kind: 'openPanel', panel: 'x' } } as Finding;
    for (const id of ['jump', 'showRange', 'showException', 'setupFormat'] as const) await actions.run(other, id);
    await actions.run({ ...bare, action: { kind: 'zoom', from: 1, to: 2, recordId: null } }, 'openPanel');
    await actions.run(bare, 'jump');
    expect(ctx.calls).toEqual(['show:profile']);
    expect(TestBed.inject(DialogService).stack()).toHaveLength(1);
  });
});

describe('OverviewView', () => {
  function setup(findings: Finding[] | (() => Promise<Finding[]>)) {
    const answer = typeof findings === 'function' ? findings : async () => findings;
    const overview = vi.fn(async () => ({ result: await answer(), coverage: fullCoverage(3) }));
    const ctx = new FakeCtx();
    TestBed.configureTestingModule({
      providers: [provideTinyBackend(), provideFakeDock(), { provide: ShareBackend, useValue: { overview } }, { provide: COMMAND_CONTEXT, useValue: ctx }, { provide: AnalysisBackend, useValue: { queryExceptions: async () => ({ result: [], coverage: fullCoverage(0) }) } }],
    });
    const fixture = TestBed.createComponent(OverviewView);
    return { fixture, overview, ctx };
  }

  afterEach(() => localStorage.removeItem(OVERVIEW_AUTO_OPEN_KEY));

  it('shows the numbers, the unrecognised files and the findings by severity with their actions', async () => {
    const { fixture, overview, ctx } = setup([crash, exc, file]);
    await settle(fixture);
    expect(overview).toHaveBeenCalledWith(expect.objectContaining({ text: '', filters: [] }));
    const t = text(fixture);
    expect(t).toContain('Sources');
    expect(t).toContain('1 file not opened');
    expect(t).toContain('export.reg — No importer');
    expect(t).toContain('→');
    expect(all(fixture, 'section.group h3').map((h) => h.textContent?.replace(/\s+/g, ' ').trim())).toEqual(['Critical (1)', 'Warning (1)', 'Info (1)']);
    expect(t).not.toContain('Nothing alarming found');
    buttonByText(fixture, 'Show these events').click();
    await settle(fixture);
    expect(ctx.calls).toContain('show:events');
  });

  it('says when nothing alarming was found, lists plugin problems and refreshes', async () => {
    const { fixture, overview } = setup([]);
    const store = TestBed.inject(WorkspaceStore);
    await settle(fixture);
    expect(text(fixture)).toContain('Nothing alarming found');
    store.applySummary({ ...store.summary()!, plugins: [pluginInfo({ id: 'p', name: 'Bad plugin', status: 'quarantined' })], from: null, to: null });
    await settle(fixture);
    expect(text(fixture)).toContain('1 plugin with problems');
    expect(text(fixture)).toContain('No timestamps');
    const calls = overview.mock.calls.length;
    buttonByText(fixture, 'Refresh').click();
    await settle(fixture);
    expect(overview.mock.calls.length).toBe(calls + 1);
  });

  it('shows an inline error with Retry, and a hint before anything is opened', async () => {
    let fail = true;
    const { fixture } = setup(async () => (fail ? Promise.reject(new Error('engine busy')) : [bare]));
    await settle(fixture);
    expect(text(fixture)).toContain('engine busy');
    fail = false;
    buttonByText(fixture, 'Retry').click();
    await settle(fixture);
    expect(text(fixture)).toContain('Note');
    const store = TestBed.inject(WorkspaceStore);
    store.applySummary({ ...store.summary()!, sources: [], unrecognized: [] });
    await settle(fixture);
    expect(text(fixture)).toContain('Open files or a folder to see a health check here.');
  });

  it('toggles opening after imports', async () => {
    const { fixture } = setup([]);
    await settle(fixture);
    const box = fixture.nativeElement.querySelector('input[type=checkbox]') as HTMLInputElement;
    expect(box.checked).toBe(false);
    box.click();
    expect(TestBed.inject(OverviewPrefs).autoOpen()).toBe(true);
  });
});

describe('provideOverview', () => {
  afterEach(() => localStorage.removeItem(OVERVIEW_AUTO_OPEN_KEY));

  it('does not open the view after an import by default', async () => {
    const dock = new FakeDock();
    TestBed.configureTestingModule({ providers: [provideTinyBackend(), provideFakeDock(dock), provideOverview()] });
    TestBed.inject(OverviewAutoOpen);
    const store = TestBed.inject(WorkspaceStore);
    await settle();
    await store.openPaths(['/x.log']);
    await settle();
    expect(dock.calls).toEqual([]);
  });

  it('registers the view and command, and opens the view after each import when turned on', async () => {
    const dock = new FakeDock();
    TestBed.configureTestingModule({ providers: [provideTinyBackend(), provideFakeDock(dock), provideOverview()] });
    TestBed.inject(OverviewPrefs).setAutoOpen(true);
    TestBed.inject(OverviewAutoOpen);
    const store = TestBed.inject(WorkspaceStore);
    await settle();
    expect(TestBed.inject(ViewRegistry).find('overview')?.defaultPosition).toBe('right');
    expect(await (await import('../extensions/view-registry')).isViewAvailable(TestBed.inject(ViewRegistry).find('overview')!, store.summary())).toBe(true);
    const component = await TestBed.inject(ViewRegistry).find('overview')!.component();
    expect(component).toBe(OverviewView);
    expect(dock.calls).toEqual([]);
    await store.openPaths(['/x.log']);
    await settle();
    expect(dock.calls).toEqual(['show:overview']);
    TestBed.inject(OverviewPrefs).setAutoOpen(false);
    await store.openPaths(['/y.log']);
    await settle();
    expect(dock.calls).toEqual(['show:overview']);
    expect(await TestBed.inject(CommandRegistry).run('view.overview')).toBe(true);
    expect(dock.calls).toEqual(['show:overview', 'show:overview']);
  });
});
