import { TestBed } from '@angular/core/testing';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { ImtBackend } from '../../core/backend';
import { Logger } from '../../core/logger';
import { MockBackend } from '../../core/mock/mock-backend';
import { WorkspaceStore } from '../../core/workspace.store';
import { SavedQueryStore } from '../../projects/bookmarks/saved-query.store';
import { UndoStack } from '../../undo/undo-stack';
import { MemoryLogger } from '../../../testing/helpers';
import { AiError } from '../ai-backend';
import { FolderSetupBackend } from './folder-setup-backend';
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
    expect(ws.summary()!.defaultGroupBy).toEqual(['MachineName']);
    expect(ws.summary()!.markerKinds.map((m) => m.kind)).toContain('reindex-start');
    const saved = TestBed.inject(SavedQueryStore);
    expect(saved.items().map((q) => q.name)[0]).toContain('Failed to capture payment');
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
    const { store, backend } = setup({ stageMs: 5 });
    await TestBed.inject(WorkspaceStore).init();
    const run = store.start();
    await store.cancel();
    await run;
    expect(store.phase()).toBe('cancelled');
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
