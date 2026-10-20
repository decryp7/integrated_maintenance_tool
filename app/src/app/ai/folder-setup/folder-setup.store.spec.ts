import { TestBed } from '@angular/core/testing';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { ImtBackend } from '../../core/backend';
import { Logger } from '../../core/logger';
import { JobsBackend } from '../../jobs/jobs-backend';
import { JobOutcome } from '../../jobs/jobs.types';
import { MockBackend } from '../../core/mock/mock-backend';
import { WorkspaceStore } from '../../core/workspace.store';
import { SavedQueryStore } from '../../projects/bookmarks/saved-query.store';
import { UndoStack } from '../../undo/undo-stack';
import { MemoryLogger } from '../../../testing/helpers';
import { AiError } from '../ai-backend';
import { FolderSetupBackend } from './folder-setup-backend';
import { FolderRefinement } from './folder-setup.types';
import { recommendedSelection, toggleSelection, withoutDependents, withRequirements, FolderSetupStore } from './folder-setup.store';
import { SetupProposal } from './folder-setup.types';
import { mockSetup, MockFolderSetupBackend, MockFolderSetupOptions } from './mock-folder-setup-backend';

function p(id: string, extra: Partial<SetupProposal> = {}): SetupProposal {
  return {
    id,
    group: 'markers',
    origin: 'heuristic',
    title: id,
    summary: '',
    fragments: [],
    requires: [],
    toml: '',
    preview: { kind: 'dashboard', tiles: 1 },
    recommended: false,
    ...extra,
  };
}

const key = (id: string, recommended = false) => p(id, { group: 'instances', fragments: [{ kind: 'instanceKey', key: [id] }], recommended });

describe('folder set-up selection rules', () => {
  const list = [p('a'), p('b', { requires: ['a'] }), p('c', { requires: ['b'] }), key('Machine', true), key('App'), p('chart', { requires: ['Machine'] })];

  it('selecting adds what is required, transitively', () => {
    expect(withRequirements(['c'], list).sort()).toEqual(['a', 'b', 'c']);
    expect(toggleSelection([], 'c', list).sort()).toEqual(['a', 'b', 'c']);
    expect(withRequirements(['nope'], list)).toEqual([]);
  });

  it('deselecting drops what depends on it', () => {
    expect(withoutDependents(['a', 'b', 'c', 'Machine'], 'a', list)).toEqual(['Machine']);
    expect(toggleSelection(['a', 'b', 'c'], 'b', list)).toEqual(['a']);
    expect(toggleSelection(['a'], 'unknown', list)).toEqual(['a']);
  });

  it('only one instance key is selected at a time, and its dependents go with it', () => {
    const sel = toggleSelection(['Machine', 'chart'], 'App', list);
    expect(sel).toContain('App');
    expect(sel).not.toContain('Machine');
    expect(sel).not.toContain('chart');
  });

  it('the recommended selection is consistent', () => {
    const l = [key('Machine', true), key('App', true), p('chart', { requires: ['Machine'], recommended: true })];
    expect(recommendedSelection(l).sort()).toEqual(['Machine', 'chart']);
  });
});

function setup(options: MockFolderSetupOptions = { stageMs: 0 }) {
  const workspace = new MockBackend({ latency: false });
  const backend = new MockFolderSetupBackend(workspace, options);
  TestBed.configureTestingModule({
    providers: [
      { provide: ImtBackend, useValue: workspace },
      { provide: FolderSetupBackend, useValue: backend },
      { provide: Logger, useClass: MemoryLogger },
    ],
  });
  return { store: TestBed.inject(FolderSetupStore), backend, workspace };
}

describe('FolderSetupStore', () => {
  afterEach(() => TestBed.resetTestingModule());

  it('surveys with progress, then lists grouped proposals with the recommended ones selected', async () => {
    const { store, workspace } = setup();
    await TestBed.inject(WorkspaceStore).init();
    const seen: string[] = [];
    const spy = vi.spyOn(workspace, 'workspaceSummary');
    const run = store.start();
    expect(store.phase()).toBe('surveying');
    await run;
    expect(spy).toHaveBeenCalled();
    expect(store.phase()).toBe('proposals');
    seen.push(...store.groups().map((g) => g.group));
    expect(seen).toEqual(['instances', 'markers', 'extractors', 'noise', 'charts', 'queries']);
    expect(store.selected()).toContain('instance:MachineName');
    // The run rule needs both markers, so they are selected with it; the drop is opt-in.
    expect(store.selected()).toEqual(expect.arrayContaining(['span:run', 'marker:reindex-start', 'marker:reindex-stop']));
    expect(store.selected()).not.toContain('noise:0');
    expect(store.skipped().length).toBe(1);
    expect(store.canReview()).toBe(true);
  });

  it('reviews the engine-composed profile and applies it as one undoable change with the saved queries', async () => {
    const { store, workspace } = setup();
    const ws = TestBed.inject(WorkspaceStore);
    await ws.init();
    await store.start();
    const before = (await workspace.getProfile()).toml;
    await store.review();
    expect(store.phase()).toBe('review');
    const composed = store.composed()!;
    expect(composed.toml).toContain('MachineName');
    expect(composed.added).toBeGreaterThan(0);
    expect(composed.savedQueries.length).toBe(1);
    expect(await store.apply()).toBe(true);
    expect(store.phase()).toBe('applied');
    // The profile is saved at once; the files re-import as a job that the workspace follows.
    await vi.waitFor(() => expect(ws.busy()).toBeNull(), { timeout: 5000 });
    expect(ws.summary()!.defaultGroupBy).toEqual(['MachineName']);
    expect(ws.summary()!.markerKinds.map((m) => m.kind)).toContain('reindex-start');
    const saved = TestBed.inject(SavedQueryStore);
    expect(saved.items().map((q) => q.name)[0]).toContain('Failed to capture payment');
    const versionAfterApply = ws.profileVersion();
    expect(versionAfterApply).toBeGreaterThan(0);
    // Undo restores the profile and the saved queries together.
    const undo = TestBed.inject(UndoStack);
    expect(undo.undoLabel()).toBe('folder set-up');
    expect(await undo.undo()).toBe(true);
    expect((await workspace.getProfile()).toml).toBe(before);
    expect(saved.items()).toEqual([]);
    expect(await undo.redo()).toBe(true);
    expect(saved.items().length).toBe(1);
  });

  it('a failed compose or save keeps the user where they were, with the message', async () => {
    const { store, backend } = setup();
    await TestBed.inject(WorkspaceStore).init();
    await store.start();
    vi.spyOn(backend, 'compose').mockRejectedValueOnce('the combined profile does not compile: boom');
    await store.review();
    expect(store.phase()).toBe('proposals');
    expect(store.error()).toContain('boom');
    await store.review();
    vi.spyOn(TestBed.inject(ImtBackend), 'saveProfile').mockRejectedValueOnce(new Error('refused'));
    expect(await store.apply()).toBe(false);
    expect(store.phase()).toBe('review');
    expect(store.error()).toBe('refused');
    store.back();
    expect(store.phase()).toBe('proposals');
    expect(await store.apply()).toBe(false);
  });

  it('cancelling the survey ends in the cancelled state; failures can be retried', async () => {
    const { store, backend } = setup({ stageMs: 150 });
    await TestBed.inject(WorkspaceStore).init();
    const run = store.start();
    // The survey is a job: cancelling needs its id, which the backend returns at once.
    await vi.waitFor(() => expect(store.surveyJob()).not.toBeNull());
    await store.cancel();
    await run;
    expect(store.phase()).toBe('cancelled');
    expect(store.surveyJob()).toBeNull();
    vi.spyOn(backend, 'survey').mockRejectedValueOnce(new Error('nothing is open: open a folder first'));
    await store.start();
    expect(store.phase()).toBe('failed');
    expect(store.error()).toContain('open a folder');
    store.reset();
    expect(store.phase()).toBe('idle');
  });

  it('adds the AI proposals unselected, and reports AI errors without losing the heuristics', async () => {
    const { store } = setup();
    await TestBed.inject(WorkspaceStore).init();
    await store.start();
    const n = store.proposals().length;
    await store.refineWithAi();
    expect(store.ai()).toBe('done');
    expect(store.proposals().length).toBe(n + 1);
    expect(store.selected()).not.toContain('ai:marker:payment-timeout');
    expect(store.skipped().some((s) => s.origin === 'ai')).toBe(true);
    // Running it again does not duplicate proposals.
    await store.refineWithAi();
    expect(store.proposals().length).toBe(n + 1);
    TestBed.resetTestingModule();
    const other = setup({ stageMs: 0, aiError: 'noModel' });
    await TestBed.inject(WorkspaceStore).init();
    await other.store.start();
    await other.store.refineWithAi();
    expect(other.store.ai()).toBe('failed');
    expect(other.store.aiMessage()).toContain('No AI model');
    expect(other.store.proposals().length).toBe(mockSetup([]).proposals.length);
  });

  it('a cancelled AI refinement goes back to idle', async () => {
    const { store, backend } = setup();
    await TestBed.inject(WorkspaceStore).init();
    await store.start();
    vi.spyOn(backend, 'refineWithAi').mockRejectedValueOnce(new AiError('cancelled', 'cancelled'));
    await store.refineWithAi();
    expect(store.ai()).toBe('idle');
    expect(store.aiMessage()).toBeNull();
  });
});

describe('FolderSetupStore as background jobs', () => {
  afterEach(() => TestBed.resetTestingModule());

  const outcome = (state: JobOutcome['state'], extra: Partial<JobOutcome> = {}): JobOutcome => ({ id: 12, state, result: null, error: null, message: null, ...extra });
  const refinement: FolderRefinement = { proposals: [p('ai:x', { origin: 'ai' })], skipped: [], notes: ['one note'] };

  function jobSetup() {
    const workspace = new MockBackend({ latency: false });
    const backend = new MockFolderSetupBackend(workspace, { stageMs: 0 });
    const survey = vi.spyOn(backend, 'survey').mockResolvedValue(12);
    const refine = vi.spyOn(backend, 'refineWithAi').mockResolvedValue(21);
    const ends: Record<number, (o: JobOutcome) => void> = {};
    const jobs = {
      wait: vi.fn((id: number) => new Promise<JobOutcome>((r) => (ends[id] = r))),
      unwait: vi.fn(async (_id: number) => undefined),
      cancel: vi.fn(async (_id: number) => undefined),
    };
    TestBed.configureTestingModule({
      providers: [
        { provide: ImtBackend, useValue: workspace },
        { provide: FolderSetupBackend, useValue: backend },
        { provide: JobsBackend, useValue: jobs },
        { provide: Logger, useClass: MemoryLogger },
      ],
    });
    return { store: TestBed.inject(FolderSetupStore), survey, refine, jobs, ends };
  }

  it('start returns at once with the survey job, follows it and shows its proposals when it is done', async () => {
    const { store, survey, jobs, ends } = jobSetup();
    const run = store.start();
    expect(store.phase()).toBe('surveying');
    await vi.waitFor(() => expect(store.surveyJob()).toBe(12));
    expect(survey).toHaveBeenCalledTimes(1);
    expect(jobs.wait).toHaveBeenCalledWith(12);
    // Progress for the running job is accepted; a stale callback after the end is ignored by the job check.
    const onProgress = survey.mock.calls[0]![0];
    onProgress({ stage: 'sampling', done: 1, total: 4 });
    expect(store.progress()).toEqual({ stage: 'sampling', done: 1, total: 4 });
    ends[12]!(outcome('done', { result: { type: 'folderSurvey', outcome: { setup: mockSetup([]) } } }));
    await run;
    expect(store.phase()).toBe('proposals');
    expect(store.surveyJob()).toBeNull();
    expect(store.proposals().length).toBeGreaterThan(0);
    expect(store.canReview()).toBe(true);
  });

  it('a failed or cancelled survey job ends in failed or cancelled, with the engine reason', async () => {
    const { store, jobs, ends } = jobSetup();
    let run = store.start();
    await vi.waitFor(() => expect(store.surveyJob()).toBe(12));
    ends[12]!(outcome('failed', { error: 'nothing is open: open a folder first' }));
    await run;
    expect([store.phase(), store.error()]).toEqual(['failed', 'nothing is open: open a folder first']);
    run = store.start();
    await vi.waitFor(() => expect(store.surveyJob()).toBe(12));
    ends[12]!(outcome('cancelled'));
    await run;
    expect(store.phase()).toBe('cancelled');
    run = store.start();
    await vi.waitFor(() => expect(store.surveyJob()).toBe(12));
    ends[12]!(outcome('failed'));
    await run;
    expect(store.phase()).toBe('failed');
    expect(store.error()).toBeTruthy();
    // Cancel goes to the job; with no job it does nothing.
    await store.cancel();
    expect(jobs.cancel).not.toHaveBeenCalled();
    run = store.start();
    await vi.waitFor(() => expect(store.surveyJob()).toBe(12));
    await store.cancel();
    expect(jobs.cancel).toHaveBeenCalledWith(12);
    ends[12]!(outcome('interrupted'));
    await run;
  });

  it('showSurvey (View of the survey message) shows the proposals or the cancelled state', () => {
    const { store } = jobSetup();
    store.showSurvey({ cancelled: true });
    expect(store.phase()).toBe('cancelled');
    store.showSurvey({ setup: mockSetup([]) });
    expect(store.phase()).toBe('proposals');
    expect(store.selected().length).toBeGreaterThan(0);
  });

  it('refineWithAi returns at once with the AI job and adds the proposals when it is done', async () => {
    const { store, refine, jobs, ends } = jobSetup();
    store.showSurvey({ setup: mockSetup([]) });
    const n = store.proposals().length;
    const run = store.refineWithAi();
    expect(store.ai()).toBe('running');
    await vi.waitFor(() => expect(store.aiJob()).toBe(21));
    expect(refine).toHaveBeenCalled();
    expect(jobs.wait).toHaveBeenCalledWith(21);
    // A second request while one runs is ignored.
    await store.refineWithAi();
    expect(refine).toHaveBeenCalledTimes(1);
    ends[21]!(outcome('done', { result: { type: 'proposal', proposal: { ...refinement, kind: 'folderSetup' } } }));
    await run;
    expect(store.ai()).toBe('done');
    expect(store.aiJob()).toBeNull();
    expect(store.proposals().length).toBe(n + 1);
    expect(store.aiNotes()).toEqual(['one note']);
  });

  it('a failed, cancelled or other-result AI job ends the refinement without losing the proposals', async () => {
    const { store, jobs, ends } = jobSetup();
    store.showSurvey({ setup: mockSetup([]) });
    const n = store.proposals().length;
    let run = store.refineWithAi();
    await vi.waitFor(() => expect(store.aiJob()).toBe(21));
    ends[21]!(outcome('failed', { error: 'the model ran out of memory' }));
    await run;
    expect([store.ai(), store.aiMessage()]).toEqual(['failed', 'the model ran out of memory']);
    run = store.refineWithAi();
    await vi.waitFor(() => expect(store.aiJob()).toBe(21));
    await store.cancelAi();
    expect(jobs.cancel).toHaveBeenCalledWith(21);
    ends[21]!(outcome('cancelled'));
    await run;
    expect(store.ai()).toBe('idle');
    run = store.refineWithAi();
    await vi.waitFor(() => expect(store.aiJob()).toBe(21));
    ends[21]!(outcome('done', { result: { type: 'workspace' } }));
    await run;
    expect(store.ai()).toBe('idle');
    expect(store.proposals().length).toBe(n);
    // Cancel with no AI job does nothing.
    jobs.cancel.mockClear();
    await store.cancelAi();
    expect(jobs.cancel).not.toHaveBeenCalled();
  });

  it('addRefinement (View of the AI message) opens the proposals even when the wizard was closed', () => {
    const { store } = jobSetup();
    expect(store.phase()).toBe('idle');
    store.addRefinement(refinement);
    expect([store.phase(), store.ai(), store.proposals().map((x) => x.id)]).toEqual(['proposals', 'done', ['ai:x']]);
    store.addRefinement(refinement);
    expect(store.proposals()).toHaveLength(1);
  });

  it('detach (the wizard closed) stops following both jobs, which go on in Jobs', async () => {
    const { store, jobs, ends } = jobSetup();
    const survey = store.start();
    await vi.waitFor(() => expect(store.surveyJob()).toBe(12));
    store.detach();
    expect(jobs.unwait).toHaveBeenCalledWith(12);
    expect([store.surveyJob(), store.phase()]).toEqual([null, 'idle']);
    ends[12]!(outcome('interrupted'));
    await survey;
    // The late outcome of a detached survey does not bring the wizard back.
    expect(store.phase()).toBe('idle');

    store.showSurvey({ setup: mockSetup([]) });
    const ai = store.refineWithAi();
    await vi.waitFor(() => expect(store.aiJob()).toBe(21));
    store.detach();
    expect(jobs.unwait).toHaveBeenCalledWith(21);
    expect([store.aiJob(), store.ai()]).toEqual([null, 'idle']);
    ends[21]!(outcome('done', { result: { type: 'proposal', proposal: { ...refinement, kind: 'folderSetup' } } }));
    await ai;
    expect(store.ai()).toBe('idle');
    // Nothing running: nothing to release.
    jobs.unwait.mockClear();
    store.detach();
    expect(jobs.unwait).not.toHaveBeenCalled();
  });
});
