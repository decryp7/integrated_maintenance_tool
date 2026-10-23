import { TestBed } from '@angular/core/testing';
import { all, buttonByText, settle, text } from '../../testing/helpers';
import { DebriefBackend } from '../core/backend';
import { WorkspaceStore } from '../core/workspace.store';
import { ProjectBackend } from '../core/tools-backend';
import { CommandRegistry } from '../extensions/command-registry';
import { DEBRIEF_COMMANDS } from '../extensions/tokens';
import { CoreUiState } from '../lifecycle/core-ui-state';
import { UI_STATE_CONTRIBUTIONS, UiStateRegistry } from '../lifecycle/ui-state';
import { mockTools, FakePicker, MemoryRecentProjects, provideToolsTesting } from '../tools/mock/test-support';
import { DIALOG_REF, DialogService } from '../ui/dialog.service';
import { SavedQueryStore } from './bookmarks/saved-query.store';
import { OpenRequests, SESSION_FILE_NAME } from './open-requests';
import { ProjectCommands } from './project-commands';
import { ProjectPicker } from './project-picker';
import { isProjectPath, ProjectService, projectNameFor } from './project.service';
import { ProjectUiState } from './project-ui-state';
import { RecentProjectsDialog, RecentProjectsDialogService } from './recent-dialog';
import { UnsavedChangesDialog, UnsavedChangesGuard } from './unsaved-changes';
import { RECENT_PROJECTS } from './recent-projects';
import { SavedQueryCommands } from './saved-query-commands';
import { at } from '../../testing/must';

async function setup() {
  TestBed.configureTestingModule({
    providers: [
      ...provideToolsTesting(),
      { provide: DIALOG_REF, useValue: { close: () => undefined, closed: Promise.resolve(undefined) } },
      { provide: UI_STATE_CONTRIBUTIONS, multi: true, useFactory: () => TestBed.inject(CoreUiState).contributions },
      { provide: UI_STATE_CONTRIBUTIONS, multi: true, useFactory: () => TestBed.inject(ProjectUiState).contributions },
      { provide: DEBRIEF_COMMANDS, multi: true, useFactory: () => [...TestBed.inject(ProjectCommands).commands, ...TestBed.inject(SavedQueryCommands).commands] },
    ],
  });
  const projects = TestBed.inject(ProjectService);
  await settle();
  return {
    projects,
    store: TestBed.inject(WorkspaceStore),
    tools: mockTools(),
    picker: TestBed.inject(ProjectPicker) as unknown as FakePicker,
    recent: TestBed.inject(RECENT_PROJECTS) as MemoryRecentProjects,
    registry: TestBed.inject(UiStateRegistry),
    commands: TestBed.inject(CommandRegistry),
  };
}

describe('project helpers', () => {
  it('recognises project files and names them', () => {
    expect(isProjectPath('C:\\x\\A.DEBRIEFPROJ')).toBe(true);
    expect(isProjectPath('/x/a.zip')).toBe(false);
    expect(isProjectPath('/x/Case.DEBRIEFZ')).toBe(true);
    expect(isProjectPath('/x/folder.debrief')).toBe(true);
    expect(projectNameFor('/x/Case 7.debriefz')).toBe('Case 7');
    expect(projectNameFor('/x/folder.debrief')).toBe('folder');
    expect(projectNameFor('/x/Incident 42.debriefproj')).toBe('Incident 42');
    expect(projectNameFor('')).toBe('Untitled');
  });
});

describe('ProjectService', () => {
  it('is clean after the workspace loads and dirty after a change that a project would save', async () => {
    const { projects, store } = await setup();
    expect(projects.dirty()).toBe(false);
    store.patchQuery({ text: 'x' }); store.setTextInput('x');
    expect(projects.dirty()).toBe(true);
    expect(projects.title()).toBe('● Untitled — Debrief');
    await settle();
    expect(document.title).toBe('● Untitled — Debrief');
  });

  it('shows no unsaved dot when an untitled project has nothing open, e.g. after Close workspace (E2)', async () => {
    const { projects, store, picker } = await setup();
    store.patchQuery({ text: 'x' }); store.setTextInput('x');
    expect(projects.dirty()).toBe(true);
    await store.clearWorkspace();
    expect(store.summary()?.sources).toEqual([]);
    expect(projects.dirty()).toBe(false);
    expect(projects.title()).toBe('Untitled — Debrief');
    // A saved project that was emptied still has unsaved changes.
    picker.save = '/p/empty.debriefz';
    await store.openPaths(['/demo/extra']);
    await projects.saveAs();
    await store.clearWorkspace();
    expect(projects.dirty()).toBe(true);
  });

  it('does not count smart search switching how the text matches as an unsaved change', async () => {
    const { projects, store } = await setup();
    store.setTextInput('timed out');
    await projects.saveAs();
    expect(projects.dirty()).toBe(false);
    // The debounce applies the text as a word search; the background scan may then add partial matches.
    store.patchQuery({ text: 'timed out', textMode: 'words' });
    store.patchQuery({ textMode: 'wordsOrSubstring' });
    expect(projects.dirty()).toBe(false);
    store.setTextInput('timed');
    expect(projects.dirty()).toBe(true);
  });

  it('ignores selection and layout changes for the unsaved flag, but not sources', async () => {
    const { projects, store } = await setup();
    store.jumpTo(1);
    store.select({ id: 1, ts: 0, tsState: 'parsed', level: 'Error', message: 'm', template: null, exception: null, fields: {}, source: 0, sourceName: 's', line: 1, byteOffset: 0, group: '', markers: [] });
    expect(projects.dirty()).toBe(false);
    await store.openPaths(['/demo/extra']);
    expect(projects.dirty()).toBe(true);
  });

  it('Save As asks for a file, writes the project and remembers it', async () => {
    const { projects, tools, picker, recent, store } = await setup();
    store.patchQuery({ text: 'abc' }); store.setTextInput('abc');
    expect(await projects.save()).toBe(true);
    expect(picker.saveNames).toEqual(['Untitled.debriefz']);
    expect(projects.path()).toBe('/p/saved.debriefz');
    expect(projects.name()).toBe('saved');
    expect(projects.dirty()).toBe(false);
    expect(tools.calls).toContain('saveProject:/p/saved.debriefz');
    expect(store.notice()).toBe('Saved saved');
    expect(recent.items).toEqual([{ path: '/p/saved.debriefz', name: 'saved', openedAt: expect.any(Number) }]);
    expect(projects.recent()).toEqual(recent.items);
    tools.calls.length = 0;
    expect(await projects.save()).toBe(true);
    expect(picker.saveNames).toHaveLength(1);
    expect(tools.calls).toEqual(['saveProject:/p/saved.debriefz']);
  });

  it('an edit made while the save is in flight stays unsaved (the baseline is what was written)', async () => {
    const { projects, tools, store } = await setup();
    store.patchQuery({ text: 'saved' }); store.setTextInput('saved');
    let release!: () => void;
    const gate = new Promise<void>((r) => (release = r));
    const real = tools.saveProject.bind(tools);
    vi.spyOn(tools, 'saveProject').mockImplementation(async (...args) => {
      await gate;
      return real(...args);
    });
    const saving = projects.save();
    await settle();
    store.patchQuery({ text: 'edited meanwhile' }); store.setTextInput('edited meanwhile');
    release();
    expect(await saving).toBe(true);
    expect(projects.dirty()).toBe(true);
    store.patchQuery({ text: 'saved' }); store.setTextInput('saved');
    expect(projects.dirty()).toBe(false);
  });

  it('never replaces another file without asking; Save writes over the project\'s own file', async () => {
    const { projects, tools } = await setup();
    const dialogs = TestBed.inject(DialogService);
    expect(await projects.saveAs()).toBe(true);
    // Save to the project's own file: no question.
    expect(await projects.save()).toBe(true);
    expect(dialogs.stack()).toHaveLength(0);
    // Save As onto an existing file asks; keeping the file leaves the project as it was.
    tools.calls.length = 0;
    let saving = projects.saveAs();
    await settle();
    expect(dialogs.stack()[0]?.options.title).toBe('Replace file?');
    at(dialogs.stack(), 0).ref.close();
    expect(await saving).toBe(false);
    expect(tools.calls).toEqual([]);
    saving = projects.saveAs();
    await settle();
    at(dialogs.stack(), 0).ref.close(true);
    expect(await saving).toBe(true);
    expect(tools.calls).toEqual(['saveProject:/p/saved.debriefz']);
  });

  it('keeps everything when the save dialog is cancelled or saving fails', async () => {
    const { projects, picker, tools, store } = await setup();
    picker.save = null;
    expect(await projects.saveAs()).toBe(false);
    picker.save = '/p/x.debriefz';
    vi.spyOn(tools, 'saveProject').mockRejectedValue('locked');
    expect(await projects.save()).toBe(false);
    expect(store.error()).toBe('Could not save the project: locked');
    expect(projects.path()).toBeNull();
    expect(projects.busy()).toBe(false);
  });

  it('round-trips the UI state through save and open', async () => {
    const { projects, store, registry, picker } = await setup();
    store.patchQuery({ text: 'needle', levels: ['Error'] }); store.setTextInput('needle');
    store.setTextInput('needle');
    TestBed.inject(SavedQueryStore).replace([{ id: 'q1', name: 'Timeouts', query: { text: 'timeout' } }]);
    await projects.saveAs();
    store.clearQuery();
    TestBed.inject(SavedQueryStore).replace([]);
    expect(projects.dirty()).toBe(true);
    picker.open = '/p/saved.debriefz';
    await projects.openFromDialog();
    expect(store.query()).toMatchObject({ text: 'needle', levels: ['Error'] });
    expect(TestBed.inject(SavedQueryStore).items()).toHaveLength(1);
    expect(projects.name()).toBe('saved');
    expect(projects.dirty()).toBe(false);
    expect(registry.serialize()['savedQueries']).toHaveLength(1);
  });

  it('does nothing when the open dialog is cancelled; reports unknown files', async () => {
    const { projects, picker, store } = await setup();
    picker.open = null;
    await projects.openFromDialog();
    expect(projects.path()).toBeNull();
    expect(await projects.open('/nope.debriefz')).toBe(false);
    expect(store.error()).toContain('Could not open the project');
  });

  it('shows missing paths after opening and can dismiss them', async () => {
    const { projects, tools, store } = await setup();
    await tools.saveProject('/p/missing.debriefz', 'M', {});
    expect(await projects.open('/p/missing.debriefz')).toBe(true);
    expect(projects.missing()).toEqual(['/demo/gone/app.log']);
    expect(store.notice()).toBe('1 path of this project could not be found');
    projects.dismissMissing();
    expect(projects.missing()).toEqual([]);
    vi.spyOn(tools, 'openProject').mockResolvedValue({ ...(await tools.openProject('/p/missing.debriefz')), missing: ['/a', '/b'] });
    await projects.open('/p/missing.debriefz');
    expect(store.notice()).toBe('2 paths of this project could not be found');
  });

  it('opens a read-only project as unsaved: Save asks for a new file', async () => {
    const { projects, tools } = await setup();
    await tools.saveProject('/p/shared.debriefz', 'Shared', {});
    vi.spyOn(tools, 'openProject').mockResolvedValue({ ...(await tools.openProject('/p/shared.debriefz')), readOnly: true });
    expect(await projects.open('/p/shared.debriefz')).toBe(true);
    expect(projects.path()).toBeNull();
    expect(projects.name()).toBe('Shared');
  });

  it('refuses to save a project anywhere but an .debriefz file (the mock, like the engine)', async () => {
    const { tools } = await setup();
    await expect(tools.saveProject('/p/x.debriefproj', 'X', {})).rejects.toThrow(/.debriefz/);
  });

  it('opens the session without making it the current project', async () => {
    const { projects, recent, tools } = await setup();
    await tools.saveSession({ version: 1, query: { query: { text: 'sess' }, textInput: 'sess' } });
    await projects.open('/x/last-session.debriefproj', { session: true });
    expect(TestBed.inject(WorkspaceStore).query().text).toBe('sess');
    expect(projects.path()).toBeNull();
    expect(recent.items).toEqual([]);
  });

  it('New closes the workspace and resets name, path and UI state', async () => {
    const { projects, store, picker } = await setup();
    await projects.saveAs();
    expect(picker.save).toBe('/p/saved.debriefz');
    store.patchQuery({ text: 'z' }); store.setTextInput('z');
    TestBed.inject(SavedQueryStore).replace([{ id: 'q', name: 'n', query: {} }]);
    await projects.newProject();
    expect(projects.name()).toBe('Untitled');
    expect(projects.path()).toBeNull();
    expect(store.summary()?.sources).toEqual([]);
    expect(store.textInput()).toBe('');
    expect(TestBed.inject(SavedQueryStore).items()).toEqual([]);
    expect(projects.dirty()).toBe(false);
  });

  it('clears the recent list', async () => {
    const { projects, recent } = await setup();
    await projects.saveAs();
    projects.clearRecent();
    expect(recent.items).toEqual([]);
    expect(projects.recent()).toEqual([]);
  });
});

describe('ProjectCommands, SavedQueryCommands and the recent dialog', () => {
  it('runs New / Open / Save / Save As / Recent through the command registry', async () => {
    const { commands, picker, tools, projects } = await setup();
    expect(await commands.run('project.save')).toBe(true);
    expect(projects.path()).toBe('/p/saved.debriefz');
    picker.save = '/p/other.debriefz';
    await commands.run('project.saveAs');
    expect(projects.name()).toBe('other');
    picker.open = '/p/saved.debriefz';
    await commands.run('project.open');
    expect(projects.name()).toBe('saved');
    expect(tools.calls.filter((c) => c.startsWith('saveProject'))).toHaveLength(2);
    await commands.run('project.new');
    expect(projects.name()).toBe('Untitled');
    await commands.run('project.recent');
    expect(TestBed.inject(DialogService).stack()[0]?.options.title).toBe('Recent projects');
    expect(commands.find('project.save')?.keybinding).toBe('Mod+S');
  });

  it('the saved queries command shows the view', async () => {
    const { commands } = await setup();
    expect(await commands.run('savedQueries.show')).toBe(true);
  });

  it('the recent dialog lists projects, opens one and clears the list', async () => {
    const { projects, tools } = await setup();
    await projects.saveAs();
    TestBed.inject(RecentProjectsDialogService).open();
    const dialogs = TestBed.inject(DialogService);
    const d = at(dialogs.stack(), 0);
    expect(d.options.title).toBe('Recent projects');
    const f = TestBed.createComponent(RecentProjectsDialog);
    await settle(f);
    expect(text(f)).toContain('saved');
    expect(all(f, '.item debrief-path-text')[0]?.textContent?.trim()).toBe('/p/');
    // J1: the item says when the project was last opened or saved.
    expect(all(f, '.item [data-testid=recent-when]')[0]?.textContent?.trim()).not.toBe('');
    tools.calls.length = 0;
    at(all<HTMLButtonElement>(f, '.item'), 0).click();
    await settle(f);
    expect(projects.name()).toBe('saved');
    buttonByText(f, 'Clear list').click();
    await settle(f);
    expect(text(f)).toContain('No recent projects');
    expect(buttonByText(f, 'Clear list').disabled).toBe(true);
    buttonByText(f, 'Close').click();
  });
});

describe('UnsavedChangesGuard', () => {
  async function dirtySetup() {
    const ctx = await setup();
    ctx.store.setTextInput('keep me');
    expect(ctx.projects.dirty()).toBe(true);
    return { ...ctx, dialogs: TestBed.inject(DialogService) };
  }

  /** Runs a command without waiting, answers the dialog it opens, then waits for the command. */
  async function answer(dialogs: DialogService, run: Promise<unknown>, choice: string | undefined) {
    await vi.waitFor(() => expect(dialogs.stack()).toHaveLength(1));
    const d = at(dialogs.stack(), 0);
    expect(d.options.title).toBe('Unsaved changes');
    d.ref.close(choice);
    await run;
  }

  it('does not ask when the project is clean', async () => {
    const { commands, projects } = await setup();
    await commands.run('project.new');
    expect(TestBed.inject(DialogService).stack()).toHaveLength(0);
    expect(projects.name()).toBe('Untitled');
    expect(await TestBed.inject(UnsavedChangesGuard).confirm('x')).toBe(true);
  });

  it('Cancel (or Escape) keeps the current project and its changes', async () => {
    const { commands, dialogs, store, tools } = await dirtySetup();
    await answer(dialogs, commands.run('project.new'), 'cancel');
    expect(store.textInput()).toBe('keep me');
    expect(store.summary()!.sources.length).toBeGreaterThan(0);
    await answer(dialogs, commands.run('project.open'), undefined);
    expect(store.textInput()).toBe('keep me');
    expect(tools.calls.some((c) => c.startsWith('saveProject'))).toBe(false);
  });

  it('Discard goes on without saving', async () => {
    const { commands, dialogs, store, projects, tools } = await dirtySetup();
    await answer(dialogs, commands.run('project.new'), 'discard');
    expect(store.textInput()).toBe('');
    expect(store.summary()!.sources).toHaveLength(0);
    expect(projects.dirty()).toBe(false);
    expect(tools.calls.some((c) => c.startsWith('saveProject'))).toBe(false);
  });

  it('Save saves first and then goes on; a cancelled save stops the action', async () => {
    const { commands, dialogs, store, projects, tools, picker } = await dirtySetup();
    picker.save = null;
    await answer(dialogs, commands.run('project.new'), 'save');
    expect(store.textInput()).toBe('keep me');
    expect(projects.dirty()).toBe(true);
    picker.save = '/p/kept.debriefz';
    picker.open = '/p/kept.debriefz';
    await answer(dialogs, commands.run('project.open'), 'save');
    expect(tools.calls).toContain('saveProject:/p/kept.debriefz');
    expect(projects.name()).toBe('kept');
    expect(projects.dirty()).toBe(false);
  });

  it('asks before opening a recent project', async () => {
    const { dialogs, projects, store } = await dirtySetup();
    await projects.saveAs();
    store.setTextInput('changed again');
    const f = TestBed.createComponent(RecentProjectsDialog);
    await settle(f);
    const open = vi.spyOn(projects, 'open');
    at(all<HTMLButtonElement>(f, '.item'), 0).click();
    await answer(dialogs, settle(f), 'cancel');
    expect(open).not.toHaveBeenCalled();
    at(all<HTMLButtonElement>(f, '.item'), 0).click();
    await answer(dialogs, settle(f), 'discard');
    await vi.waitFor(() => expect(open).toHaveBeenCalledWith('/p/saved.debriefz'));
  });

  it('the dialog names the project and closes with the chosen answer', async () => {
    let result: unknown = 'none';
    TestBed.configureTestingModule({ providers: [{ provide: DIALOG_REF, useValue: { close: (r: unknown) => (result = r), closed: Promise.resolve(undefined) } }] });
    const f = TestBed.createComponent(UnsavedChangesDialog);
    f.componentRef.setInput('name', 'Incident 42');
    f.componentRef.setInput('action', 'start a new project');
    await settle(f);
    expect(text(f)).toContain('Save the changes to Incident 42 before you start a new project?');
    expect(all<HTMLButtonElement>(f, 'button').map((b) => b.textContent?.trim())).toEqual(['Cancel', 'Discard', 'Save']);
    for (const [label, value] of [['Save', 'save'], ['Discard', 'discard'], ['Cancel', 'cancel']] as const) {
      buttonByText(f, label).click();
      expect(result).toBe(value);
    }
  });
});

describe('OpenRequests', () => {
  afterEach(() => vi.useRealTimers());

  it('routes projects to open_project and other files to the workspace', async () => {
    const { tools, projects, store } = await setup();
    await tools.saveProject('/p/a.debriefz', 'A', {});
    const requests = TestBed.inject(OpenRequests);
    await requests.handle(['/p/a.debriefz', '/logs/x.log', '/logs/y.zip']);
    expect(projects.name()).toBe('A');
    expect(store.summary()?.sources.map((s) => s.path)).toEqual(expect.arrayContaining(['/logs/x.log', '/logs/y.zip']));
    await requests.handle([]);
  });

  it('opens the autosaved session file as a session', async () => {
    const { tools, projects } = await setup();
    await tools.saveSession({ version: 1 });
    await TestBed.inject(OpenRequests).handle([`/data/${SESSION_FILE_NAME}`]);
    expect(projects.path()).toBeNull();
  });

  it('takes pending paths at start and again when the shell announces more; leaves live tail changes to LiveTail', async () => {
    const { tools, store } = await setup();
    const backend = TestBed.inject(ProjectBackend);
    let openHandler: () => void = () => undefined;
    const stopped = vi.fn();
    vi.spyOn(backend, 'onOpenRequested').mockImplementation(async (h) => ((openHandler = h), stopped));
    const pending = vi.spyOn(backend, 'takePendingOpen').mockResolvedValueOnce(['/logs/first.log']).mockResolvedValueOnce(['/logs/second.log']);
    const requests = TestBed.inject(OpenRequests);
    await requests.start();
    await requests.start();
    expect(pending).toHaveBeenCalledTimes(1);
    expect(store.summary()?.sources.some((s) => s.path === '/logs/first.log')).toBe(true);
    openHandler();
    await settle();
    expect(store.summary()?.sources.some((s) => s.path === '/logs/second.log')).toBe(true);

    requests.stop();
    requests.stop();
    expect(stopped).toHaveBeenCalledTimes(1);
    expect(tools.calls).toBeDefined();
  });

  it('reports a failing queue and a failing subscription', async () => {
    const { store } = await setup();
    const backend = TestBed.inject(ProjectBackend);
    vi.spyOn(backend, 'takePendingOpen').mockRejectedValue(new Error('queue'));
    await TestBed.inject(OpenRequests).drain();
    expect(store.error()).toContain('queue');
    TestBed.resetTestingModule();
    const again = await setup();
    vi.spyOn(TestBed.inject(ProjectBackend), 'onOpenRequested').mockRejectedValue(new Error('no events'));
    await TestBed.inject(OpenRequests).start();
    expect(again.store.error()).toBeNull();
  });

  it('waits (bounded) for the first workspace load before opening', async () => {
    TestBed.configureTestingModule({ providers: provideToolsTesting() });
    vi.spyOn(TestBed.inject(DebriefBackend), 'workspaceSummary').mockRejectedValue(new Error('engine not ready'));
    const store = TestBed.inject(WorkspaceStore);
    vi.useFakeTimers();
    const requests = TestBed.inject(OpenRequests);
    const started = requests.start();
    await vi.advanceTimersByTimeAsync(20 * 100 + 10);
    await started;
    expect(store.summary()).toBeNull();
  });
});
