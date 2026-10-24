import { TestBed } from '@angular/core/testing';
import { pluginInfo, provideFakeDock, provideTinyBackend } from '../../../testing/helpers';
import { ExceptionGroup, Finding, fullCoverage, WorkspaceSummary } from '../../core/api.types';
import { AnalysisBackend } from '../../core/analysis-backend';

import { COMMAND_CONTEXT } from '../../extensions/command-registry';

import { CommandContext } from '../../extensions/tokens';

import { FormatWizard } from '../../format/format-wizard';

import { DialogService } from '../../ui/dialog.service';
import { HealthActions } from './health-actions';
import { actionButtons, groupBySeverity, healthStats, panelTarget, pluginProblems, zoomWindow } from './health-model';
import { at } from '../../../testing/must';

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
    const s = healthStats(summary, 4, [crash, crash, exc]);
    expect(s).toMatchObject({ sources: 2, events: 50, instances: 4, from: 1, to: 9, errors: 4, crashes: 2 });
    expect(healthStats({ ...summary, levels: {} } as unknown as WorkspaceSummary, 0, []).errors).toBe(0);
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

describe('HealthActions', () => {
  function setup(groups: ExceptionGroup[] | { reject: unknown } = [group()]) {
    const ctx = new FakeCtx();
    const analysis = { queryExceptions: vi.fn(async () => (Array.isArray(groups) ? { result: groups, coverage: fullCoverage(groups.length) } : Promise.reject(groups.reject))) };
    TestBed.configureTestingModule({
      providers: [provideTinyBackend(), provideFakeDock(), { provide: COMMAND_CONTEXT, useValue: ctx }, { provide: AnalysisBackend, useValue: analysis }],
    });
    return { ctx, actions: TestBed.inject(HealthActions) };
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
    await TestBed.inject(DialogService).whenLoaded();
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
