import { TestBed } from '@angular/core/testing';
import { FakeDock, provideFakeDock, provideTinyBackend, settle } from '../../testing/helpers';
import { WorkspaceStore } from '../core/workspace.store';
import { CommandRegistry } from '../extensions/command-registry';
import { VIEW_OPEN_PREFIX } from '../extensions/tokens';
import { PaletteService } from '../palette/palette.service';
import { SettingsStore } from '../settings/settings.store';
import { TimelineZoom } from '../timeline/zoom';
import { DialogService } from '../ui/dialog.service';
import { WelcomeService } from '../welcome/welcome';

// Every built-in command does what its title says, run through the registry as the palette and menus run it.
describe('built-in commands', () => {
  let dock: FakeDock;

  async function setup() {
    dock = new FakeDock();
    TestBed.configureTestingModule({ providers: [provideTinyBackend(), provideFakeDock(dock)] });
    const registry = TestBed.inject(CommandRegistry);
    await settle();
    return registry;
  }

  it('switch the theme and the colour scheme', async () => {
    const registry = await setup();
    const store = TestBed.inject(WorkspaceStore);
    for (const theme of ['console', 'precision', 'calm'] as const) {
      expect(await registry.run(`view.theme.${theme}`)).toBe(true);
      expect(store.theme()).toBe(theme);
    }
    for (const scheme of ['dark', 'light', 'system'] as const) {
      await registry.run(`view.scheme.${scheme}`);
      expect(store.scheme()).toBe(scheme);
    }
  });

  it('switch the source colours', async () => {
    const registry = await setup();
    const prefs = TestBed.inject(SettingsStore);
    for (const mode of ['instance', 'off', 'source'] as const) {
      await registry.run(`view.sourceColors.${mode}`);
      expect(prefs.sourceColors()).toBe(mode);
    }
  });

  it('set and toggle the time zone', async () => {
    const registry = await setup();
    const store = TestBed.inject(WorkspaceStore);
    await registry.run('analyze.utc');
    expect(store.timezone()).toBe('utc');
    await registry.run('analyze.toggleUtc');
    expect(store.timezone()).toBe('local');
    await registry.run('analyze.toggleUtc');
    expect(store.timezone()).toBe('utc');
    await registry.run('analyze.local');
    expect(store.timezone()).toBe('local');
  });

  it('toggle marked-only, the sidebar and focus the search box', async () => {
    const registry = await setup();
    const store = TestBed.inject(WorkspaceStore);
    const marked = store.markedOnly();
    await registry.run('analyze.markedOnly');
    expect(store.markedOnly()).toBe(!marked);
    await registry.run('analyze.markedOnly');
    expect(store.markedOnly()).toBe(marked);

    const open = store.sidebarOpen();
    await registry.run('view.toggleSidebar');
    expect(store.sidebarOpen()).toBe(!open);

    const nonce = store.focusSearchNonce();
    await registry.run('analyze.search');
    expect(store.focusSearchNonce()).toBe(nonce + 1);
  });

  it('drive the timeline zoom history', async () => {
    const registry = await setup();
    const zoom = TestBed.inject(TimelineZoom);
    const calls: string[] = [];
    vi.spyOn(zoom, 'fit').mockImplementation(() => void calls.push('fit'));
    vi.spyOn(zoom, 'back').mockImplementation(() => void calls.push('back'));
    vi.spyOn(zoom, 'forward').mockImplementation(() => void calls.push('forward'));
    for (const id of ['analyze.fit', 'analyze.back', 'analyze.forward']) await registry.run(id);
    expect(calls).toEqual(['fit', 'back', 'forward']);
  });

  it('clear the query, re-import, close the workspace and open the palette', async () => {
    const registry = await setup();
    const store = TestBed.inject(WorkspaceStore);
    const clear = vi.spyOn(store, 'clearQuery');
    const reload = vi.spyOn(store, 'reload').mockResolvedValue(undefined as never);
    const close = vi.spyOn(store, 'clearWorkspace').mockResolvedValue(undefined as never);
    await registry.run('analyze.clearFilters');
    await registry.run('file.reimport');
    await registry.run('file.close');
    expect([clear, reload, close].map((s) => s.mock.calls.length)).toEqual([1, 1, 1]);

    await registry.run('palette.open');
    expect(TestBed.inject(PaletteService).isOpen()).toBe(true);
  });

  it('open files and folders through the picker', async () => {
    const registry = await setup();
    const store = TestBed.inject(WorkspaceStore);
    const pick = vi.spyOn(store, 'pickAndOpen').mockResolvedValue(undefined as never);
    await registry.run('file.openFiles');
    await registry.run('file.openFolder');
    expect(pick.mock.calls).toEqual([[false], [true]]);
  });

  it('reset the layout and open About, the notices and the welcome screen', async () => {
    const registry = await setup();
    await registry.run('view.resetLayout');
    expect(dock.calls).toContain('reset');
    const dialogs = TestBed.inject(DialogService);
    await registry.run('help.about');
    expect(dialogs.stack()[0]?.options.title).toBe('About Debrief');
    dialogs.dismissTop();
    await registry.run('help.notices');
    expect(dialogs.stack()[0]?.options.title).toBe('About Debrief');
    dialogs.dismissTop();
    await registry.run('help.welcome');
    expect(TestBed.inject(WelcomeService).visible()).toBe(true);
  });

  it('opens a view, stepping the welcome screen aside when it is shown', async () => {
    const registry = await setup();
    const open = registry.all().find((c) => c.id.startsWith(VIEW_OPEN_PREFIX) && !c.when);
    const id = open?.id ?? registry.all().find((c) => c.id.startsWith(VIEW_OPEN_PREFIX))!.id;
    const view = id.slice(VIEW_OPEN_PREFIX.length);
    const welcome = TestBed.inject(WelcomeService);
    welcome.show();
    expect(welcome.visible()).toBe(true);
    await registry.run(id);
    expect(dock.calls).toContain(`show:${view}`);
    expect(welcome.visible()).toBe(false);

    dock.calls.length = 0;
    await registry.run(id);
    expect(dock.calls).toEqual([`show:${view}`]);
  });
});
