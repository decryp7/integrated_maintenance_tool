import { TestBed } from '@angular/core/testing';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { ImtBackend } from '../../core/backend';
import { Logger } from '../../core/logger';
import { MockAiBackend } from '../../core/mock/mock-ai-backend';
import { MockBackend } from '../../core/mock/mock-backend';
import { WorkspaceStore } from '../../core/workspace.store';
import { COMMAND_CONTEXT } from '../../extensions/command-registry';
import { DIALOG_REF, DialogService } from '../../ui/dialog.service';
import { buttonByText, MemoryLogger, settle } from '../../../testing/helpers';
import { AiBackend } from '../ai-backend';
import { AiStatusStore } from '../ai-status.store';
import { FolderSetupBackend, TauriFolderSetupBackend } from './folder-setup-backend';
import { FolderSetupHint } from './folder-setup-hint';
import { FolderSetupStore } from './folder-setup.store';
import { FolderSetupWizard } from './folder-setup-wizard';
import { MockFolderSetupBackend, naiveDiff, snippet } from './mock-folder-setup-backend';
import { FolderSetupPreview } from './proposal-preview';
import { folderSetupCommands, FolderSetupLauncher, hasOpenFiles } from './provide-folder-setup';
import { ProposalPreview } from './folder-setup.types';

function setup(aiMode: 'ready' | 'setup' | 'off' = 'off') {
  const workspace = new MockBackend({ latency: false });
  const ctx = { showView: vi.fn() };
  const ref = { close: vi.fn(), closed: new Promise(() => undefined) };
  TestBed.configureTestingModule({
    providers: [
      { provide: ImtBackend, useValue: workspace },
      { provide: FolderSetupBackend, useValue: new MockFolderSetupBackend(workspace, { stageMs: 0 }) },
      { provide: AiBackend, useValue: new MockAiBackend({ mode: aiMode, stepMs: 0 }) },
      { provide: Logger, useClass: MemoryLogger },
      { provide: COMMAND_CONTEXT, useValue: ctx },
      { provide: DIALOG_REF, useValue: ref },
    ],
  });
  return { workspace, ctx, ref };
}

async function until(fixture: { detectChanges(): void }, cond: () => boolean, rounds = 100): Promise<void> {
  for (let i = 0; i < rounds && !cond(); i++) {
    await settle();
    fixture.detectChanges();
  }
  expect(cond()).toBe(true);
}

describe('FolderSetupWizard', () => {
  afterEach(() => TestBed.resetTestingModule());

  it('walks survey → proposals → review → apply and offers the views', async () => {
    const { ctx, ref } = setup();
    await TestBed.inject(WorkspaceStore).init();
    const store = TestBed.inject(FolderSetupStore);
    const fixture = TestBed.createComponent(FolderSetupWizard);
    const el = fixture.nativeElement as HTMLElement;
    const q = (id: string) => el.querySelector<HTMLElement>(`[data-testid="${id}"]`);
    void store.start();
    fixture.detectChanges();
    expect(el.querySelector('[role="progressbar"]')).toBeTruthy();
    expect(el.querySelector('[aria-current="step"]')!.textContent).toContain('Survey');
    await until(fixture, () => !!q('review'));
    expect(q('survey-facts')!.textContent).toContain('Sampled 9820 of 9820 events');
    expect(el.querySelector('[aria-current="step"]')!.textContent).toContain('Proposals');
    expect(el.querySelectorAll('fieldset legend')[0]!.textContent).toBe('Instances and lanes');
    // Without AI: the note says how to get better proposals.
    expect(q('ai-off-note')!.textContent).toContain('local AI assistant');
    // Each proposal has a checkbox, its summary and the engine's preview (a mini lane preview for keys).
    const key = q('proposal-instance:MachineName')!;
    expect(key.querySelector<HTMLInputElement>('input[type="checkbox"]')!.checked).toBe(true);
    expect(key.querySelector('[data-testid="lane-preview"]')).toBeTruthy();
    expect(q('skipped')!.textContent).toContain('Skipped proposals (1)');
    // Switching to the other key drops the chart that needs the first one.
    q('proposal-instance:Application')!.querySelector<HTMLInputElement>('input')!.click();
    fixture.detectChanges();
    expect(store.selected()).not.toContain('chart:errors-by-instance');
    q('proposal-instance:MachineName')!.querySelector<HTMLInputElement>('input')!.click();
    fixture.detectChanges();
    q('review')!.click();
    await until(fixture, () => !!q('diff'));
    expect(q('diff-summary')!.textContent).toContain('added');
    expect(q('diff')!.textContent).toContain('+ ');
    q('apply')!.click();
    await until(fixture, () => !!q('applied'));
    q('open-timeline')!.click();
    expect(ctx.showView).toHaveBeenCalledWith('timeline');
    expect(ref.close).toHaveBeenCalled();
  });

  it('offers AI refinement when a model is ready and shows the AI proposals', async () => {
    setup('ready');
    await TestBed.inject(WorkspaceStore).init();
    const status = TestBed.inject(AiStatusStore);
    const store = TestBed.inject(FolderSetupStore);
    const fixture = TestBed.createComponent(FolderSetupWizard);
    const el = fixture.nativeElement as HTMLElement;
    const q = (id: string) => el.querySelector<HTMLElement>(`[data-testid="${id}"]`);
    await store.start();
    await until(fixture, () => status.hasModel() && !!q('ai-refine'));
    q('ai-refine')!.click();
    await until(fixture, () => !!q('proposal-ai:marker:payment-timeout'));
    expect(q('proposal-ai:marker:payment-timeout')!.textContent).toContain('AI');
    expect(el.textContent).toContain('AI proposals added below');
  });

  it('shows cancel, failure and retry states', async () => {
    setup();
    await TestBed.inject(WorkspaceStore).init();
    const store = TestBed.inject(FolderSetupStore);
    const backend = TestBed.inject(FolderSetupBackend);
    const fixture = TestBed.createComponent(FolderSetupWizard);
    vi.spyOn(backend, 'survey').mockResolvedValueOnce({ cancelled: true });
    await store.start();
    fixture.detectChanges();
    expect(fixture.nativeElement.textContent).toContain('The survey was cancelled');
    vi.spyOn(backend, 'survey').mockRejectedValueOnce('nothing is open: open a folder first');
    buttonByText(fixture, 'Survey again').click();
    await until(fixture, () => store.phase() === 'failed');
    expect(fixture.nativeElement.querySelector('[role="alert"]').textContent).toContain('open a folder');
    buttonByText(fixture, 'Try again').click();
    await until(fixture, () => store.phase() === 'proposals');
  });
});

describe('FolderSetupPreview', () => {
  afterEach(() => TestBed.resetTestingModule());

  const render = (preview: ProposalPreview) => {
    const f = TestBed.createComponent(FolderSetupPreview);
    f.componentRef.setInput('preview', preview);
    f.detectChanges();
    return (f.nativeElement as HTMLElement).textContent!.replace(/\s+/g, ' ');
  };

  it('renders every kind of engine preview', () => {
    expect(render({ kind: 'lanes', lanes: [{ name: 'A', count: 2, from: null, to: null }], totalLanes: 10, unassigned: 3 })).toContain('10 lanes on the sample, 3 events without a value');
    expect(render({ kind: 'matches', matches: 2, sampled: 9, examples: ['x'] })).toContain('Matches 2 of 9');
    expect(render({ kind: 'extracted', matches: 1, sampled: 9, examples: [{ Cpu: 2, Name: 'a' }] })).toContain('Cpu=2, Name=a');
    expect(render({ kind: 'runs', runs: 1, closed: 1, open: 0, noStop: 0, lanes: 2 })).toContain('1 run on 2 lanes');
    expect(render({ kind: 'pipeline', sampled: 9, kept: 8, dropped: 1, changed: 0, examples: [{ before: 'ping', after: '' }] })).toContain('(dropped)');
    expect(render({ kind: 'chart', series: ['a', 'b'], points: 4 })).toContain('2 series: a, b');
    expect(render({ kind: 'dashboard', tiles: 5 })).toContain('5 tiles');
    expect(render({ kind: 'query', matches: 1 })).toContain('Finds 1 event now');
    expect(render({ kind: 'parse', parsed: 3, lines: 4, withTimestamp: 3, examples: ['m'] })).toContain('Reads 3 events from 4 sample lines');
  });
});

describe('folder set-up hint, command and backends', () => {
  afterEach(() => TestBed.resetTestingModule());

  it('the hint appears after the user opens several files and opens the wizard', async () => {
    const { workspace } = setup();
    const ws = TestBed.inject(WorkspaceStore);
    await ws.init();
    const fixture = TestBed.createComponent(FolderSetupHint);
    fixture.detectChanges();
    const el = fixture.nativeElement as HTMLElement;
    expect(el.querySelector('[data-testid="folder-setup-hint"]')).toBeNull();
    await ws.openPaths(['/logs']);
    fixture.detectChanges();
    expect(el.querySelector('[data-testid="folder-setup-hint"]')).toBeTruthy();
    const open = vi.spyOn(TestBed.inject(FolderSetupLauncher), 'open').mockImplementation(() => undefined);
    el.querySelector<HTMLButtonElement>('[data-testid="folder-setup-hint-open"]')!.click();
    fixture.detectChanges();
    expect(open).toHaveBeenCalled();
    expect(el.querySelector('[data-testid="folder-setup-hint"]')).toBeNull();
    await ws.openPaths(['/more']);
    fixture.detectChanges();
    el.querySelector<HTMLButtonElement>('button[aria-label="Dismiss"]')!.click();
    fixture.detectChanges();
    expect(el.querySelector('[data-testid="folder-setup-hint"]')).toBeNull();
    expect(workspace).toBeTruthy();
  });

  it('the command needs open files; the launcher opens the dialog and stops a running survey on close', async () => {
    setup();
    const ws = TestBed.inject(WorkspaceStore);
    expect(hasOpenFiles(ws)).toBe(false);
    await ws.init();
    expect(hasOpenFiles(ws)).toBe(true);
    const launcher = TestBed.inject(FolderSetupLauncher);
    const [cmd] = folderSetupCommands(launcher, ws);
    expect(cmd!.title).toBe('Set up analysis for this folder…');
    expect(cmd!.when!({} as never)).toBe(true);
    const dialogs = TestBed.inject(DialogService);
    let close!: () => void;
    vi.spyOn(dialogs, 'open').mockImplementation(() => ({ close: () => undefined, closed: new Promise<undefined>((r) => (close = () => r(undefined))) }));
    const store = TestBed.inject(FolderSetupStore);
    const cancel = vi.spyOn(store, 'cancel');
    const cancelAi = vi.spyOn(store, 'cancelAi');
    vi.spyOn(TestBed.inject(FolderSetupBackend), 'survey').mockReturnValue(new Promise(() => undefined));
    void cmd!.run({} as never);
    expect(store.phase()).toBe('surveying');
    close();
    await settle();
    expect(cancel).toHaveBeenCalled();
    expect(cancelAi).not.toHaveBeenCalled();
  });

  it('the Tauri backend sends the documented commands', async () => {
    const call = vi.fn().mockResolvedValue({});
    const channel = vi.fn().mockReturnValue('channel');
    const b = new TauriFolderSetupBackend(call as never, channel as never);
    await b.survey('s1', () => undefined);
    expect(call).toHaveBeenLastCalledWith('folder_survey', { surveyId: 's1', request: null, onProgress: 'channel' });
    await b.cancelSurvey('s1');
    expect(call).toHaveBeenLastCalledWith('folder_survey_cancel', { surveyId: 's1' });
    await b.compose([{ kind: 'instanceKey', key: ['A'] }]);
    expect(call).toHaveBeenLastCalledWith('folder_setup_compose', { fragments: [{ kind: 'instanceKey', key: ['A'] }] });
    await b.refineWithAi({ maxEvents: 10 });
    expect(call).toHaveBeenLastCalledWith('ai_refine_folder_setup', { request: { maxEvents: 10 } });
    await b.cancelAi();
    expect(call).toHaveBeenLastCalledWith('ai_cancel');
    call.mockRejectedValueOnce({ code: 'noModel', message: 'no model' });
    await expect(b.refineWithAi()).rejects.toMatchObject({ code: 'noModel', message: 'no model' });
  });

  it('the mock composes like the engine: rejects nothing selected and duplicate markers, keeps the rest', async () => {
    const workspace = new MockBackend({ latency: false });
    const b = new MockFolderSetupBackend(workspace, { stageMs: 0 });
    await expect(b.compose([])).rejects.toThrow('select at least one');
    const marker = { kind: 'marker' as const, marker: { kind: 'x1', when: [] } };
    const c = await b.compose([marker, { kind: 'parseRegex', regex: '^(?P<message>.*)$' }, { kind: 'savedQuery', name: 'Q', ql: 'level >= error' }]);
    expect(c.savedQueries).toEqual([{ name: 'Q', ql: 'level >= error' }]);
    await workspace.saveProfile(c.toml);
    await expect(b.compose([marker])).rejects.toThrow('already has a marker named "x1"');
    expect(await b.cancelSurvey('none')).toBe(false);
    expect(naiveDiff('a\nb\nc', 'a\nx\nc').map((d) => d.op)).toEqual(['same', 'remove', 'add', 'same']);
    expect(snippet([{ kind: 'savedQuery', name: 'Q', ql: 'x' }, { kind: 'instanceKey', key: ['A'] }])).toContain('# Saved query "Q": x');
    const summary = await workspace.workspaceSummary();
    vi.spyOn(workspace, 'workspaceSummary').mockResolvedValueOnce({ ...summary, sources: [] });
    await expect(b.survey('s', () => undefined)).rejects.toThrow('open a folder');
  });
});
