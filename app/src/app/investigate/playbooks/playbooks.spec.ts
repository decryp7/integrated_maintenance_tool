import { TestBed } from '@angular/core/testing';
import { all, buttonByText, el, provideTinyBackend, settle, text, tinyBackend, typeInto } from '../../../testing/helpers';
import { at, must } from '../../../testing/must';
import { Playbook, PlaybookRun, PlaybookStep, PlaybookStepResult } from '../../core/api.types';
import { CmdError } from '../../core/cmd-error';
import { Platform } from '../../core/platform';
import { WorkspaceStore } from '../../core/workspace.store';
import { COMMAND_CONTEXT } from '../../extensions/command-registry';
import { CommandContext } from '../../extensions/tokens';
import { PlaybookBackend } from '../investigate-backend';
import { MockPlaybookBackend } from '../mock/mock-investigate-backends';
import {
  actionLabel,
  actionQuery,
  defaultAction,
  defaultExpect,
  expectLabel,
  finishedText,
  isRunnable,
  missingField,
  moved,
  newPlaybook,
  newRunId,
  newStep,
  progressOf,
  progressText,
  recordOf,
  slugOf,
  stateOf,
  statusIcon,
  statusLabel,
} from './playbook-model';
import { PlaybookEditor } from './playbook-editor';
import { PlaybookPersistence } from './playbook-persistence';
import { PlaybookRunView } from './playbook-run';
import { PlaybookStore } from './playbook.store';
import { PlaybooksPanel } from './playbooks-panel';
import { playbooksCommands, playbooksViews, providePlaybooks } from './provide-playbooks';

const ctx = () => ({ showView: vi.fn(), jumpTo: vi.fn(async () => undefined), notifyError: vi.fn() }) as unknown as CommandContext;

function provide(backend: PlaybookBackend = new MockPlaybookBackend(tinyBackend()), context = ctx()) {
  TestBed.configureTestingModule({
    providers: [...provideTinyBackend(), { provide: PlaybookBackend, useValue: backend }, { provide: COMMAND_CONTEXT, useValue: context }],
  });
  return { backend, context, store: TestBed.inject(PlaybookStore) };
}

const stepOf = (patch: Partial<PlaybookStep> = {}): PlaybookStep => ({ id: 's1', title: 'Step', instruction: '', action: { type: 'manual' }, ...patch });
const book = (patch: Partial<Playbook> = {}): Playbook => ({ version: 1, id: 'mine', title: 'Mine', description: 'D', tags: [], steps: [stepOf()], ...patch });
const rejecting = (message: string) => new CmdError({ code: 'invalid', message, retryable: false });

describe('playbook model', () => {
  it('names every status, action and expectation, with an icon per status', () => {
    const states = ['pass', 'fail', 'info', 'error', 'skipped', 'pending'] as const;
    expect(new Set(states.map(statusLabel)).size).toBe(6);
    expect(new Set(states.map(statusIcon)).size).toBe(6);
    const actions = ['manual', 'query', 'filter', 'openView', 'check'] as const;
    expect(new Set(actions.map(actionLabel)).size).toBe(5);
    expect(new Set((['noMatch', 'anyMatch', 'atLeast', 'atMost'] as const).map(expectLabel)).size).toBe(4);
  });

  it('builds default actions and expectations', () => {
    expect(defaultAction('manual')).toEqual({ type: 'manual' });
    expect(defaultAction('query')).toEqual({ type: 'query', ql: '' });
    expect(defaultAction('filter')).toEqual({ type: 'filter', ql: '' });
    expect(defaultAction('openView')).toEqual({ type: 'openView', view: '' });
    expect(defaultAction('check')).toEqual({ type: 'check', ql: '', expect: { kind: 'noMatch' } });
    expect(defaultExpect('atLeast', 3)).toEqual({ kind: 'atLeast', n: 3 });
    expect(defaultExpect('anyMatch')).toEqual({ kind: 'anyMatch' });
    expect(actionQuery({ type: 'manual' })).toBeNull();
    expect(actionQuery({ type: 'query', ql: 'x' })).toBe('x');
    expect(isRunnable({ type: 'check', ql: 'x', expect: { kind: 'noMatch' } })).toBe(true);
    expect(isRunnable({ type: 'manual' })).toBe(false);
    expect(isRunnable({ type: 'openView', view: 'v' })).toBe(false);
  });

  it('counts progress, ignoring records of steps that are gone', () => {
    const run: PlaybookRun = {
      id: 'r',
      playbook: book({ steps: [stepOf({ id: 'a' }), stepOf({ id: 'b' }), stepOf({ id: 'c' })] }),
      startedAt: 1,
      steps: [
        { stepId: 'a', status: 'pass', summary: '', count: null, sampleIds: [], note: '', at: 1 },
        { stepId: 'b', status: 'fail', summary: '', count: 2, sampleIds: [], note: '', at: 1 },
        { stepId: 'gone', status: 'error', summary: '', count: null, sampleIds: [], note: '', at: 1 },
      ],
    };
    const p = progressOf(run);
    expect(p).toMatchObject({ done: 2, total: 3, passed: 1, failed: 1, errors: 0, finished: false });
    expect(progressText(p)).toBe('2 of 3 steps');
    expect(stateOf(run, at(run.playbook.steps, 2))).toBe('pending');
    expect(stateOf(run, at(run.playbook.steps, 0))).toBe('pass');
    expect(recordOf(run, 'b')?.count).toBe(2);
    const done = progressOf({ ...run, steps: ['a', 'b', 'c'].map((id) => ({ stepId: id, status: 'skipped' as const, summary: '', count: null, sampleIds: [], note: '', at: 1 })) });
    expect(done.finished).toBe(true);
    expect(finishedText(done)).toBe('Finished: 0 passed, 0 failed, 0 errors, 3 skipped');
    expect(progressOf({ ...run, playbook: book({ steps: [] }), steps: [] }).finished).toBe(false);
  });

  it('makes ids, new playbooks and steps, and moves items', () => {
    expect(slugOf('Disk Full!', new Set())).toBe('disk-full');
    expect(slugOf('Disk Full!', new Set(['disk-full', 'disk-full-2']))).toBe('disk-full-3');
    expect(slugOf('***', new Set())).toBe('playbook');
    expect(newPlaybook(new Set(['new-playbook'])).id).toBe('new-playbook-2');
    expect(newStep([stepOf({ id: 'step-2' })]).id).toBe('step-3');
    expect(newStep([]).id).toBe('step-1');
    expect(moved([1, 2, 3], 0, 1)).toEqual([2, 1, 3]);
    expect(moved([1, 2, 3], 0, -1)).toEqual([1, 2, 3]);
    expect(moved([1, 2, 3], 2, 1)).toEqual([1, 2, 3]);
    expect(moved([1, 2, 3], 9, -1)).toEqual([1, 2, 3]);
    expect(newRunId(255, 3)).toBe('run-73-3');
  });

  it('points at the first missing field, for convenience only', () => {
    expect(missingField(book())).toBeNull();
    expect(missingField(book({ title: ' ' }))).toMatch(/title/);
    expect(missingField(book({ steps: [] }))).toMatch(/step/);
    expect(missingField(book({ steps: [stepOf({ title: '' })] }))).toMatch(/title/);
    expect(missingField(book({ steps: [stepOf({ action: { type: 'query', ql: ' ' } })] }))).toMatch(/needs a query/);
    expect(missingField(book({ steps: [stepOf({ action: { type: 'openView', view: '' } })] }))).toMatch(/view id/);
    expect(missingField(book({ steps: [stepOf({ action: { type: 'openView', view: 'problems' } })] }))).toBeNull();
  });
});

describe('PlaybookStore', () => {
  it('loads the library and reports a failure', async () => {
    const { store, backend } = provide();
    await store.load();
    expect(store.isLoaded()).toBe(true);
    expect(store.library().templates.length).toBe(2);
    vi.spyOn(backend, 'library').mockRejectedValueOnce(rejecting('disk gone'));
    await store.load();
    expect(store.error()).toBe('disk gone');
    expect(store.isLoading()).toBe(false);
  });

  it('copies a template into the library and selects the copy', async () => {
    const { store } = provide();
    await store.load();
    const tpl = at(store.library().templates, 0);
    store.select(tpl.id);
    expect(store.selectedIsTemplate()).toBe(true);
    expect(store.selected()?.id).toBe(tpl.id);
    const saved = await store.saveCopyOfTemplate(tpl);
    expect(saved?.id).toBe(tpl.id);
    expect(store.library().playbooks.map((p) => p.id)).toEqual([tpl.id]);
    expect(store.selectedIsTemplate()).toBe(false);
  });

  it('keeps the engine message when a save fails, and removes playbooks', async () => {
    const { store } = provide();
    await store.load();
    expect(await store.save(book({ id: 'Bad Id' }))).toBeNull();
    expect(store.actionError()).toMatch(/lowercase/);
    store.dismissError();
    expect(store.actionError()).toBeNull();
    expect(await store.saveCopyOfTemplate(book({ id: 'Bad' }))).toBeNull();
    expect((await store.save(book()))?.id).toBe('mine');
    store.startEdit(book());
    expect(await store.remove('mine')).toBe(true);
    expect(store.selectedId()).toBeNull();
    expect(store.library().playbooks).toEqual([]);
    store.select('other');
    expect(await store.remove('mine')).toBe(false);
    expect(store.selectedId()).toBe('other');
  });

  it('reports a failing remove', async () => {
    const { store, backend } = provide();
    vi.spyOn(backend, 'remove').mockRejectedValueOnce(rejecting('locked'));
    expect(await store.remove('x')).toBe(false);
    expect(store.actionError()).toBe('locked');
  });

  it('imports (saving what the file held) and exports', async () => {
    const { store, backend } = provide();
    const imported = await store.import('/demo/shared.json');
    expect(imported?.id).toBe('service-crashed');
    expect(store.selectedId()).toBe('service-crashed');
    expect(await store.export(at(store.library().playbooks, 0), '/out.json')).toBe(true);
    expect(store.notice()).toMatch(/Exported/);
    store.dismissNotice();
    expect(store.notice()).toBeNull();
    vi.spyOn(backend, 'importFile').mockRejectedValueOnce(rejecting('not a playbook'));
    expect(await store.import('/bad.json')).toBeNull();
    expect(store.actionError()).toBe('not a playbook');
    vi.spyOn(backend, 'exportFile').mockRejectedValueOnce(rejecting('read only'));
    expect(await store.export(book(), '/ro.json')).toBe(false);
    expect(store.notice()).toBeNull();
    expect(store.actionError()).toBe('read only');
  });

  it('edits a copy and starts new playbooks with a free id', async () => {
    const { store } = provide();
    await store.load();
    store.startNew();
    expect(store.editingIsNew()).toBe(true);
    expect(store.editing()?.id).toBe('new-playbook');
    store.cancelEdit();
    expect(store.editing()).toBeNull();
    const original = book();
    store.startEdit(original);
    must(store.editing()).title = 'changed';
    expect(original.title).toBe('Mine');
    await store.save(must(store.editing()));
    expect(store.editing()).toBeNull();
  });

  it('runs a playbook: records steps, notes and the workspace filter', async () => {
    const { store } = provide();
    await TestBed.inject(WorkspaceStore).init();
    await store.load();
    const pb = at(store.library().templates, 0);
    expect(await store.runStep(at(pb.steps, 0))).toBeNull();
    const run = must(await store.startRun(pb));
    expect(store.activeRun()?.id).toBe(run.id);
    expect(store.record()?.runs.length).toBe(1);
    const check = at(pb.steps, 0);
    const result = must(await store.runStep(check));
    expect(['pass', 'fail']).toContain(result.status);
    expect(store.activeRun()?.steps.map((s) => s.stepId)).toEqual(['fatal']);
    expect(store.resultOf(run.id, 'fatal')).toEqual(result);
    expect(store.busyStep()).toBeNull();

    // a note before a result waits; on a recorded step it is stored with the engine
    expect(await store.setNote(at(pb.steps, 3), 'later')).toBe(true);
    expect(store.noteFor(must(store.activeRun()), 'before')).toBe('later');
    expect(await store.recordManual(at(pb.steps, 3), 'pass')).toBe(true);
    expect(recordOf(must(store.activeRun()), 'before')).toMatchObject({ status: 'pass', note: 'later' });
    expect(await store.setNote(check, 'looked at it')).toBe(true);
    expect(store.noteFor(must(store.activeRun()), 'fatal')).toBe('looked at it');
    expect(await store.setNote(at(pb.steps, 3), 'edited')).toBe(true);
    expect(recordOf(must(store.activeRun()), 'before')?.note).toBe('edited');
    expect(await store.recordManual(at(pb.steps, 2), 'skipped', 'n/a')).toBe(true);
    expect(recordOf(must(store.activeRun()), 'problems')).toMatchObject({ status: 'skipped', note: 'n/a' });
    expect(progressOf(must(store.activeRun())).done).toBe(3);

    // a filter step also becomes the workspace's query
    const filter = stepOf({ id: 'f', action: { type: 'filter', ql: 'level = fatal' } });
    await store.startRun(book({ steps: [filter] }));
    await store.runStep(filter);
    expect(TestBed.inject(WorkspaceStore).query().ql).toBe('level = fatal');
  });

  it('refuses without an active run, and keeps working after a backend failure', async () => {
    const { store, backend } = provide();
    expect(await store.recordManual(stepOf(), 'pass')).toBe(false);
    expect(await store.setNote(stepOf(), 'x')).toBe(false);
    const run = must(await store.startRun(book()));
    vi.spyOn(backend, 'runStep').mockRejectedValueOnce(rejecting('engine down'));
    const q = stepOf({ action: { type: 'query', ql: 'x' } });
    expect(await store.runStep(q)).toBeNull();
    expect(store.actionError()).toBe('engine down');
    expect(store.busyStep()).toBeNull();
    vi.spyOn(backend, 'record').mockRejectedValueOnce(rejecting('no such step'));
    expect(await store.runStep(stepOf({ id: 'zzz', action: { type: 'query', ql: 'x' } }))).toBeNull();
    expect(store.actionError()).toBe('no such step');
    expect(store.activeRun()?.steps).toEqual([]);
    vi.spyOn(backend, 'start').mockRejectedValueOnce(rejecting('bad id'));
    expect(await store.startRun(book())).toBeNull();
    expect(store.runs().length).toBe(1);
    store.closeRun();
    expect(store.activeRun()).toBeNull();
    store.openRun(run.id);
    expect(store.activeRun()?.id).toBe(run.id);
    store.openRun('missing');
    expect(store.activeRun()?.id).toBe(run.id);
    store.deleteRun('other');
    expect(store.activeRun()?.id).toBe(run.id);
    store.deleteRun(run.id);
    expect(store.activeRun()).toBeNull();
    expect(store.record()).toBeNull();
  });

  it('lists the runs of the selected playbook, newest first', async () => {
    const { store } = provide();
    const a = must(await store.startRun(book()));
    const b = must(await store.startRun(book()));
    await store.startRun(book({ id: 'other' }));
    store.select('mine');
    expect(store.selectedRuns().map((r) => r.id).sort()).toEqual([a.id, b.id].sort());
  });

  it('shows the events of a step by applying its query and jumping to the first sample', async () => {
    const { store } = provide();
    const ws = TestBed.inject(WorkspaceStore);
    const jump = vi.spyOn(ws, 'jumpTo');
    store.showEvents('level = error', [7, 8]);
    expect(ws.query().ql).toBe('level = error');
    expect(jump).toHaveBeenCalledWith(7);
    store.showEvents(null, []);
    expect(jump).toHaveBeenCalledTimes(1);
  });
});

describe('PlaybookPersistence', () => {
  it('captures nothing without runs, and the record with them', async () => {
    const { store } = provide();
    const c = TestBed.inject(PlaybookPersistence).contribution;
    expect(c.key).toBe('playbooks');
    expect(c.capture()).toBeNull();
    await store.startRun(book());
    expect(c.capture()).toMatchObject({ version: 1, runs: [{ playbook: { id: 'mine' } }] });
  });

  it('restores through the engine, keeps its warnings, and resets', async () => {
    const { store } = provide();
    const c = TestBed.inject(PlaybookPersistence).contribution;
    const record = { version: 1, runs: [{ id: 'r1', playbook: book(), startedAt: 5, steps: [] }] };
    c.restore(record);
    await settle();
    expect(store.runs().map((r) => r.id)).toEqual(['r1']);
    expect(store.restoreWarnings()).toEqual([]);
    c.restore({ junk: true });
    await settle();
    expect(store.runs()).toEqual([]);
    expect(store.restoreWarnings().length).toBe(1);
    c.restore(record);
    await settle();
    c.reset?.();
    expect(store.runs()).toEqual([]);
    expect(store.restoreWarnings()).toEqual([]);
  });

  it('survives an engine that rejects the saved record', async () => {
    const { store, backend } = provide();
    vi.spyOn(backend, 'restore').mockRejectedValueOnce(rejecting('engine down'));
    await store.restoreRecord({ version: 1, runs: [] });
    expect(store.restoreWarnings()).toEqual(['engine down']);
  });

  it('is registered with the feature', () => {
    TestBed.configureTestingModule({ providers: [...provideTinyBackend(), providePlaybooks()] });
    expect(TestBed.inject(PlaybookPersistence).contribution.key).toBe('playbooks');
  });
});

describe('views and commands', () => {
  it('registers the view and loads its panel lazily', async () => {
    const view = at(playbooksViews(), 0);
    expect(view).toMatchObject({ id: 'playbooks', title: 'Playbooks', icon: 'check' });
    expect(await view.component()).toBe(PlaybooksPanel);
  });

  it('shows the view, and "new" also opens the editor', () => {
    const { store, context } = provide();
    const [show, create] = playbooksCommands(store);
    expect(at([show, create].map((c) => must(c).title), 0)).toBe('View: Playbooks');
    must(show).run(context);
    expect(context.showView).toHaveBeenCalledWith('playbooks');
    must(create).run(context);
    expect(create?.title).toBe('Playbooks: New playbook');
    expect(store.editing()).not.toBeNull();
  });
});

describe('PlaybooksPanel', () => {
  async function open(backend?: PlaybookBackend) {
    const p = provide(backend);
    const fixture = TestBed.createComponent(PlaybooksPanel);
    fixture.detectChanges();
    await settle(fixture);
    return { ...p, fixture };
  }
  const click = (f: ReturnType<typeof TestBed.createComponent>, id: string) => {
    el<HTMLButtonElement>(f, `[data-testid="${id}"]`).click();
  };

  it('lists templates and my playbooks, copies a template and starts a run', async () => {
    const { fixture, store } = await open();
    expect(text(fixture)).toContain('A service crashed or restarted');
    expect(el(fixture, '[data-testid="pb-mine-empty"]')).toBeTruthy();
    expect(el(fixture, '[data-testid="pb-none"]')).toBeTruthy();
    click(fixture, 'pb-template-service-crashed');
    await settle(fixture);
    expect(all(fixture, '[data-testid="pb-outline"] li').length).toBe(4);
    click(fixture, 'pb-use-template');
    await settle(fixture);
    expect(el(fixture, '[data-testid="pb-item-service-crashed"]').getAttribute('aria-current')).toBe('true');
    click(fixture, 'pb-start');
    await settle(fixture);
    expect(text(fixture)).toContain('0 of 4 steps');
    expect(store.activeRun()).not.toBeNull();
    expect(all(fixture, 'debrief-playbook-run .step').length).toBe(4);
  });

  it('shows the loading error with retry', async () => {
    const backend = new MockPlaybookBackend(tinyBackend());
    vi.spyOn(backend, 'library').mockRejectedValueOnce(rejecting('nope'));
    const { fixture, store } = await open(backend);
    expect(el(fixture, '[data-testid="pb-load-error"]').textContent).toContain('nope');
    buttonByText(fixture, 'Retry').click();
    await settle(fixture);
    expect(store.error()).toBeNull();
  });

  it('shows warnings, past runs with open and delete, and exports and imports through the file dialogs', async () => {
    const { fixture, store } = await open();
    const platform = TestBed.inject(Platform);
    vi.spyOn(platform, 'pickOpenPath').mockResolvedValueOnce('/in.json').mockResolvedValueOnce(null);
    vi.spyOn(platform, 'pickSavePath').mockResolvedValueOnce('/out.json').mockResolvedValueOnce(null);
    click(fixture, 'pb-import');
    await settle(fixture);
    expect(store.selectedId()).toBe('service-crashed');
    click(fixture, 'pb-import');
    await settle(fixture);
    click(fixture, 'pb-export');
    await settle(fixture);
    expect(el(fixture, '[data-testid="pb-notice"]').textContent).toContain('Exported');
    click(fixture, 'pb-export');
    await settle(fixture);

    const run = must(await store.startRun(must(store.selected())));
    store.closeRun();
    await settle(fixture);
    expect(all(fixture, '[data-testid="pb-runs"] li').length).toBe(1);
    click(fixture, `pb-open-run-${run.id}`);
    await settle(fixture);
    expect(text(fixture)).toContain('0 of 4 steps');
    store.closeRun();
    await settle(fixture);
    click(fixture, `pb-delete-run-${run.id}`);
    await settle(fixture);
    expect(store.runs()).toEqual([]);

    await store.restoreRecord({ nope: 1 });
    await settle(fixture);
    expect(el(fixture, '[data-testid="pb-warning"]')).toBeTruthy();
    store.restoreWarnings();
  });

  it('edits, deletes and creates playbooks', async () => {
    const { fixture, store } = await open();
    click(fixture, 'pb-new');
    await settle(fixture);
    expect(el(fixture, '[data-testid="playbook-editor"]')).toBeTruthy();
    click(fixture, 'pb-cancel');
    await settle(fixture);
    await store.save(book());
    await settle(fixture);
    click(fixture, 'pb-edit');
    await settle(fixture);
    expect(el<HTMLInputElement>(fixture, '[data-testid="pb-title"]').value).toBe('Mine');
    click(fixture, 'pb-cancel');
    await settle(fixture);
    click(fixture, 'pb-delete');
    await settle(fixture);
    expect(store.library().playbooks).toEqual([]);
  });

  it('shows the action error of a failed delete', async () => {
    const backend = new MockPlaybookBackend(tinyBackend());
    const { fixture, store } = await open(backend);
    await store.save(book());
    await settle(fixture);
    vi.spyOn(backend, 'remove').mockRejectedValueOnce(rejecting('locked'));
    click(fixture, 'pb-delete');
    await settle(fixture);
    expect(el(fixture, '[data-testid="pb-action-error"]').textContent).toContain('locked');
    click(fixture, 'pb-action-error');
  });
});

describe('PlaybookRunView', () => {
  async function runOf(steps: PlaybookStep[], context = ctx()) {
    const p = provide(undefined, context);
    await TestBed.inject(WorkspaceStore).init();
    const run = must(await p.store.startRun(book({ steps })));
    const fixture = TestBed.createComponent(PlaybookRunView);
    fixture.componentRef.setInput('run', run);
    fixture.detectChanges();
    await settle(fixture);
    const refresh = async () => {
      fixture.componentRef.setInput('run', must(p.store.activeRun()));
      fixture.detectChanges();
      await settle(fixture);
    };
    return { ...p, fixture, refresh };
  }
  const status = (f: ReturnType<typeof TestBed.createComponent>, id: string) => el(f, `[data-testid="pb-status-${id}"]`).textContent?.trim() ?? '';

  it('shows each step pending with its action, runs a check and shows Pass or Fail with text', async () => {
    const steps = [
      stepOf({ id: 'chk', title: 'No fatals', instruction: 'Look', action: { type: 'check', ql: 'level = fatal', expect: { kind: 'atLeast', n: 2 } } }),
      stepOf({ id: 'qry', title: 'Errors', action: { type: 'query', ql: 'level >= error' } }),
      stepOf({ id: 'view', title: 'Problems', action: { type: 'openView', view: 'problems' } }),
      stepOf({ id: 'man', title: 'By hand' }),
    ];
    const { fixture, store, context, backend, refresh } = await runOf(steps);
    expect(status(fixture, 'chk')).toContain('Not run');
    expect(text(fixture)).toContain('At least n events match (2)');
    expect(el(fixture, '[data-testid="pb-run-step-chk"] code').textContent).toBe('level = fatal');

    click(fixture, 'pb-run-chk');
    await settle(fixture);
    await refresh();
    expect(status(fixture, 'chk')).toMatch(/Pass|Fail/);
    expect(el(fixture, '[data-testid="pb-summary-chk"]').textContent).toContain('Expected atLeast');
    expect(text(fixture)).toContain('1 of 4 steps');
    expect(buttonByText(fixture, 'Run again')).toBeTruthy();

    vi.spyOn(backend, 'runStep').mockResolvedValueOnce({ status: 'info', summary: 'Found 3', count: 3, sampleIds: [4, 5], columns: [], rows: [] });
    click(fixture, 'pb-run-qry');
    await settle(fixture);
    await refresh();
    expect(status(fixture, 'qry')).toContain('Info');
    click(fixture, 'pb-show-qry');
    expect(TestBed.inject(WorkspaceStore).query().ql).toBe('level >= error');

    click(fixture, 'pb-open-view');
    expect(context.showView).toHaveBeenCalledWith('problems');
    click(fixture, 'pb-skip-view');
    await settle(fixture);
    await refresh();
    expect(status(fixture, 'view')).toContain('Skipped');

    const note = el<HTMLTextAreaElement>(fixture, '[data-testid="pb-note-man"]');
    typeInto(note, 'checked the moment view', 'change');
    await settle(fixture);
    click(fixture, 'pb-done-man');
    await settle(fixture);
    await refresh();
    expect(status(fixture, 'man')).toContain('Pass');
    expect(must(store.activeRun()).steps.find((s) => s.stepId === 'man')?.note).toBe('checked the moment view');
    expect(el<HTMLTextAreaElement>(fixture, '[data-testid="pb-note-man"]').value).toBe('checked the moment view');
    expect(el(fixture, '[data-testid="pb-finished"]').textContent).toContain('Finished');
    expect(all(fixture, '[data-testid="pb-progress"]').length).toBe(1);
  });

  it('renders the table of a stats result and an error with dismiss', async () => {
    const q = stepOf({ id: 'stats', action: { type: 'query', ql: 'x | stats count()' } });
    const { fixture, store, backend, refresh } = await runOf([q]);
    const result: PlaybookStepResult = { status: 'fail', summary: 'Two sources', count: 2, sampleIds: [], columns: ['source', 'errors'], rows: [['api', 3], [null, { a: 1 }]] };
    vi.spyOn(backend, 'runStep').mockResolvedValueOnce(result);
    click(fixture, 'pb-run-stats');
    await settle(fixture);
    await refresh();
    const cells = all(fixture, '[data-testid="pb-table-stats"] tbody td').map((c) => c.textContent);
    expect(cells).toEqual(['api', '3', '', '{"a":1}']);
    expect(status(fixture, 'stats')).toContain('Fail');
    expect(el(fixture, '[data-testid="pb-count-stats"]').textContent).toContain('2 events');
    expect(all(fixture, '[data-testid="pb-show-stats"]').length).toBe(0);

    vi.spyOn(backend, 'runStep').mockRejectedValueOnce(rejecting('engine down'));
    click(fixture, 'pb-run-stats');
    await settle(fixture);
    expect(el(fixture, '[data-testid="pb-run-error"]').textContent).toContain('engine down');
    buttonByText(fixture, 'Dismiss').click();
    await settle(fixture);
    expect(store.actionError()).toBeNull();
  });

  function click(f: ReturnType<typeof TestBed.createComponent>, id: string) {
    el<HTMLButtonElement>(f, `[data-testid="${id}"]`).click();
  }
});

describe('PlaybookEditor', () => {
  async function edit(pb: Playbook = book(), isNew = false) {
    const p = provide();
    await p.store.load();
    if (isNew) p.store.startNew();
    else p.store.startEdit(pb);
    const fixture = TestBed.createComponent(PlaybookEditor);
    fixture.detectChanges();
    await settle(fixture);
    return { ...p, fixture };
  }
  const input = (f: ReturnType<typeof TestBed.createComponent>, id: string) => el<HTMLInputElement>(f, `[data-testid="${id}"]`);

  it('edits the fields, adds, reorders and removes steps, then saves', async () => {
    const { fixture, store } = await edit(book({ id: 'e', steps: [stepOf({ id: 'a', title: 'A' }), stepOf({ id: 'b', title: 'B' })] }));
    typeInto(input(fixture, 'pb-title'), 'Renamed');
    typeInto(input(fixture, 'pb-description'), 'Why');
    typeInto(input(fixture, 'pb-step-title-0'), 'First');
    buttonByText(fixture, 'Add step').click();
    await settle(fixture);
    expect(all(fixture, '.step').length).toBe(3);
    el<HTMLButtonElement>(fixture, '[data-testid="pb-step-2"] button[aria-label="Move step up"]').click();
    await settle(fixture);
    el<HTMLButtonElement>(fixture, '[data-testid="pb-step-0"] button[aria-label="Move step down"]').click();
    await settle(fixture);
    el<HTMLButtonElement>(fixture, '[data-testid="pb-step-0"] button[aria-label="Remove step"]').click();
    await settle(fixture);
    expect(all(fixture, '.step').length).toBe(2);
    el<HTMLButtonElement>(fixture, '[data-testid="pb-save"]').click();
    await settle(fixture);
    const saved = must(store.library().playbooks.find((p) => p.id === 'e'));
    expect(saved).toMatchObject({ title: 'Renamed', description: 'Why' });
    expect(saved.steps.map((s) => s.title).sort()).toEqual(['B', 'First']);
    expect(saved.steps.length).toBe(2);
    expect(store.editing()).toBeNull();
  });

  it('shows the fields of each action type', async () => {
    const { fixture } = await edit(book({ id: 'e2' }));
    const select = el<HTMLSelectElement>(fixture, '[data-testid="pb-step-action-0"]');
    typeInto(select, 'check', 'change');
    await settle(fixture);
    typeInto(el(fixture, '[data-testid="pb-step-ql-0"]'), 'level = fatal');
    typeInto(el(fixture, '[data-testid="pb-step-expect-0"]'), 'atLeast', 'change');
    await settle(fixture);
    typeInto(input(fixture, 'pb-step-n-0'), '4');
    await settle(fixture);
    expect(input(fixture, 'pb-step-n-0').value).toBe('4');
    typeInto(input(fixture, 'pb-step-n-0'), '-3');
    await settle(fixture);
    expect(input(fixture, 'pb-step-n-0').value).toBe('0');
    typeInto(el(fixture, '[data-testid="pb-step-expect-0"]'), 'anyMatch', 'change');
    await settle(fixture);
    expect(all(fixture, '[data-testid="pb-step-n-0"]').length).toBe(0);
    typeInto(select, 'query', 'change');
    await settle(fixture);
    typeInto(el(fixture, '[data-testid="pb-step-ql-0"]'), 'x');
    typeInto(select, 'filter', 'change');
    await settle(fixture);
    expect(input(fixture, 'pb-step-ql-0').value).toBe('');
    typeInto(select, 'openView', 'change');
    await settle(fixture);
    typeInto(input(fixture, 'pb-step-view-0'), 'problems');
    await settle(fixture);
    expect(input(fixture, 'pb-step-view-0').value).toBe('problems');
    typeInto(select, 'manual', 'change');
    await settle(fixture);
    expect(all(fixture, '[data-testid="pb-step-view-0"]').length).toBe(0);
  });

  it('checks required fields early and shows the engine message when the save fails', async () => {
    const { fixture, store } = await edit(book({ title: '' }));
    el<HTMLButtonElement>(fixture, '[data-testid="pb-save"]').click();
    await settle(fixture);
    expect(el(fixture, '[data-testid="pb-editor-problem"]').textContent).toMatch(/title/);
    expect(store.library().playbooks).toEqual([]);
    typeInto(input(fixture, 'pb-title'), 'Fine');
    await settle(fixture);
    expect(all(fixture, '[data-testid="pb-editor-problem"]').length).toBe(0);

  });

  it('shows the engine message when the save fails', async () => {
    const n = await edit(book({ id: 'Bad Id' }), false);
    el<HTMLButtonElement>(n.fixture, '[data-testid="pb-save"]').click();
    await settle(n.fixture);
    expect(el(n.fixture, '[data-testid="pb-save-error"]').textContent).toMatch(/lowercase/);
  });

  it('lets a new playbook choose its id', async () => {
    const { fixture, store } = await edit(book(), true);
    typeInto(input(fixture, 'pb-id'), 'my-own');
    await settle(fixture);
    el<HTMLButtonElement>(fixture, '[data-testid="pb-save"]').click();
    await settle(fixture);
    expect(store.library().playbooks.map((p) => p.id)).toEqual(['my-own']);
  });
});
