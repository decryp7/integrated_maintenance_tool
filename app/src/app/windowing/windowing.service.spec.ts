import { TestBed } from '@angular/core/testing';
import { provideTinyBackend, settle } from '../../testing/helpers';
import { WorkspaceStore } from '../core/workspace.store';
import { CommandRegistry } from '../extensions/command-registry';
import { DEBRIEF_COMMANDS } from '../extensions/tokens';
import { DialogService } from '../ui/dialog.service';
import { LiveTail } from '../workspace/live-tail';
import { ClosePrompt } from './close-prompt';
import { memoryHost, MockWindowingBackend } from './mock-windowing-backend';
import { OpenAskDialog } from './open-ask-dialog';
import { WindowCommands } from './window-commands';
import { UnsavedChangesGuard } from '../projects/unsaved-changes';
import { isMac } from '../extensions/keybinding';
import { WindowingBackend } from './windowing-backend';
import { CAPTURE_COMMAND, WindowingService } from './windowing.service';

function setup(href = 'http://localhost/?mock-tasks') {
  const backend = new MockWindowingBackend(memoryHost(href));
  TestBed.configureTestingModule({
    providers: [...provideTinyBackend(), { provide: WindowingBackend, useValue: backend }, { provide: DEBRIEF_COMMANDS, multi: true, useFactory: () => TestBed.inject(WindowCommands).commands }],
  });
  return { backend, service: TestBed.inject(WindowingService), store: TestBed.inject(WorkspaceStore), dialogs: TestBed.inject(DialogService) };
}

describe('WindowingService', () => {
  it('loads windows and tasks once, however often it is started', async () => {
    const { service, backend } = setup();
    const list = vi.spyOn(backend, 'listWindows');
    await Promise.all([service.init(), service.init()]);
    expect(list).toHaveBeenCalledTimes(1);
    expect(service.label()).toBe('main');
    expect(service.windows().map((w) => w.label)).toEqual(['main']);
    expect(service.runningCount()).toBe(3);
  });

  it('follows the windows-changed and background-tasks events', async () => {
    const { service, backend } = setup();
    await service.init();
    await service.newWindow();
    expect(service.windows().map((w) => w.label)).toEqual(['main', 'w-2']);
    await backend.stopBackgroundTask('monitor');
    expect(service.tasks().map((t) => t.id)).toEqual(['live-main', 'symbols']);
  });

  it('opens the close prompt once when the shell asks', async () => {
    const { service, backend, dialogs } = setup();
    await service.init();
    await service.closeWindow();
    await service.closeWindow();
    expect(dialogs.stack().map((d) => d.component)).toEqual([ClosePrompt]);
    expect(dialogs.stack()[0]?.options.title).toBe('Close Debrief?');
    dialogs.dismissTop();
    await settle();
    await backend.closeWindow();
    expect(dialogs.stack()).toHaveLength(1);
  });

  it('asks where to open files from other apps', async () => {
    const { service, backend, dialogs } = setup();
    await service.init();
    backend.emitOpenAsk({ count: 2, names: ['a.log', 'b.log'] });
    expect(dialogs.stack()[0]?.component).toBe(OpenAskDialog);
    expect(dialogs.stack()[0]?.options.title).toBe('Open 2 files');
    expect(dialogs.stack()[0]?.options.inputs).toEqual({ ask: { count: 2, names: ['a.log', 'b.log'] } });
  });

  it('runs the capture command and turns live tail off when the tray says so', async () => {
    const { service, backend } = setup();
    await service.init();
    const run = vi.spyOn(TestBed.inject(CommandRegistry), 'run').mockResolvedValue(true);
    const live = vi.spyOn(TestBed.inject(LiveTail), 'set').mockResolvedValue();
    backend.emitCapture();
    backend.emitLiveStopped();
    expect(run).toHaveBeenCalledWith(CAPTURE_COMMAND);
    expect(live).toHaveBeenCalledWith(false);
  });

  it('focuses windows, cycling to the next one', async () => {
    const { service, backend } = setup();
    await service.init();
    await service.focusNext();
    expect(backend.calls.filter((c) => c.startsWith('focusWindow'))).toEqual([]);
    await service.newWindow();
    await service.newWindow();
    await service.focusNext();
    await service.focus('w-3');
    expect(backend.calls.filter((c) => c.startsWith('focusWindow'))).toEqual(['focusWindow:w-2', 'focusWindow:w-3']);
    service.label.set('w-3');
    await service.focusNext();
    expect(backend.calls.at(-1)).toBe('focusWindow:main');
  });

  it('stops tasks and shows failures as errors', async () => {
    const { service, store } = setup();
    await service.init();
    await service.stopTask('live-main');
    expect(service.tasks().map((t) => t.id)).toEqual(['monitor', 'symbols']);
    await service.stopTask('symbols');
    expect(store.error()).toBe('Could not stop the background work: Downloading symbols cannot be stopped');
    expect(service.runningCount()).toBe(2);
  });

  it('reports failed window commands', async () => {
    const { service, backend, store } = setup();
    vi.spyOn(backend, 'newWindow').mockRejectedValue(new Error('no display'));
    await service.newWindow();
    expect(store.error()).toBe('Could not open a new window: no display');
    vi.spyOn(backend, 'closeWindow').mockRejectedValue('denied');
    await service.closeWindow();
    expect(store.error()).toBe('Could not close the window: denied');
    vi.spyOn(backend, 'focusWindow').mockRejectedValue('gone');
    await service.focus('x');
    expect(store.error()).toBe('Could not switch windows: gone');
  });

  it('keeps working when subscriptions or the first load fail, and unsubscribes on destroy', async () => {
    const { service, backend } = setup();
    vi.spyOn(backend, 'label').mockRejectedValue(new Error('x'));
    vi.spyOn(backend, 'onCapture').mockRejectedValue(new Error('x'));
    vi.spyOn(backend, 'backgroundTasks').mockRejectedValue(new Error('x'));
    await service.init();
    expect(service.tasks()).toEqual([]);
    expect(service.label()).toBe('');
    expect(backend.listeners).toBe(5);
    TestBed.resetTestingModule();
    expect(backend.listeners).toBe(0);
  });
});

describe('WindowCommands', () => {
  it('opens, closes and switches windows; switching only with several windows', async () => {
    const { service, backend } = setup('http://localhost/');
    await service.init();
    const registry = TestBed.inject(CommandRegistry);
    const byId = (id: string) => registry.find(id)!;
    expect(byId('window.new').keybinding).toBe('Mod+Shift+N');
    expect(byId('window.new').category).toBe('Window');
    expect(registry.available().some((c) => c.id === 'window.next')).toBe(false);
    expect(await registry.run('window.new')).toBe(true);
    expect(registry.available().some((c) => c.id === 'window.next')).toBe(true);
    expect(await registry.run('window.next')).toBe(true);
    expect(await registry.run('window.close')).toBe(true);
    expect(backend.calls).toEqual(['newWindow', 'focusWindow:w-2', 'closeWindow']);
  });

  it('quits Debrief in one step after asking about unsaved work', async () => {
    const confirm = vi.fn(async (_action: string) => false);
    TestBed.overrideProvider(UnsavedChangesGuard, { useValue: { confirm } });
    const { service, backend } = setup('http://localhost/');
    await service.init();
    const registry = TestBed.inject(CommandRegistry);
    expect(await registry.run('app.quit')).toBe(true);
    expect(backend.calls).not.toContain('quitApp');
    confirm.mockResolvedValueOnce(true);
    await registry.run('app.quit');
    expect(confirm).toHaveBeenCalledWith('quit Debrief');
    expect(backend.calls).toContain('quitApp');
    expect(registry.find('app.quit')?.title).toBe(isMac() ? 'Quit Debrief' : 'Exit');
  });
});
