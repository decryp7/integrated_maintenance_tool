import { FIELDS_OPEN_KEY } from './sidebar-sections';
import { Type } from '@angular/core';
import { ComponentFixture, TestBed } from '@angular/core/testing';
import { all, buttonByText, el, FakeDock, provideFakeDock, provideTinyBackend, settle, text, tinyBackend } from '../../testing/helpers';
import { PaletteService } from '../palette/palette.service';
import { Platform } from '../core/platform';
import { WorkspaceStore } from '../core/workspace.store';
import { TimelineZoom } from '../timeline/zoom';
import { provideToolsBackend } from '../core/tools-backend';
import { LifecycleService } from '../lifecycle/lifecycle.service';
import { LiveTail } from '../workspace/live-tail';
import { Sidebar } from './sidebar';
import { StatusBar } from './status-bar';
import { TitleBar } from './title-bar';
import { IMT_COMMANDS } from '../extensions/tokens';
import { memoryHost, MockWindowingBackend } from '../windowing/mock-windowing-backend';
import { WindowCommands } from '../windowing/window-commands';
import { WindowingBackend } from '../windowing/windowing-backend';
import { WindowingService } from '../windowing/windowing.service';
import { MENU_MODE } from './menu/app-menu.service';
import { NATIVE_MENU_PORT, NativeMenuPort } from './menu/native-menu';
import { NativeMenuModel } from './menu/native-menu.types';
import { at } from '../../testing/must';

const DROP_DEBUG_PROFILE = `name = "p"\n[[pipeline]]\nstep = "drop"\nwhen = [{ field = "@level", op = "eq", value = "Debug" }]\n`;

const fakePlatform = {
  isTauri: () => false,
  pickPaths: async (dir: boolean) => [dir ? '/picked/dir' : '/picked/file.log'],
  onFilesDropped: async () => () => undefined,
  pickProfileToOpen: async () => null,
  pickProfileToSave: async () => null,
};

function setup<T>(cmp: Type<T>, backend = tinyBackend()) {
  const dock = new FakeDock();
  TestBed.configureTestingModule({ providers: [provideTinyBackend(backend), provideFakeDock(dock), provideToolsBackend(), { provide: Platform, useValue: fakePlatform }] });
  const fixture = TestBed.createComponent(cmp);
  return { fixture, store: TestBed.inject(WorkspaceStore), backend, dock };
}

describe('TitleBar', () => {
  /** Menu items (menubar and open menus, which CDK renders in an overlay) by their label. */
  const menuItem = (label: string): HTMLElement => {
    const found = [...document.querySelectorAll<HTMLElement>('[role=menuitem], [role=menuitemcheckbox]')].find(
      (e) => (e.querySelector('.label')?.textContent ?? e.textContent)?.trim() === label,
    );
    if (!found) throw new Error(`No menu item "${label}"`);
    return found;
  };
  /** Opens the menus along the path and chooses its last item. */
  const choose = async (fixture: ComponentFixture<unknown>, ...path: string[]) => {
    for (const label of path) {
      menuItem(label).click();
      await settle(fixture);
    }
  };

  it('runs menu commands', async () => {
    const { fixture, store, dock } = setup(TitleBar);
    await settle(fixture);
    expect(text(fixture)).toMatch(/Indexed · \d+(\.\d)? (ms|s|min)/);
    await choose(fixture, 'View', 'Explorer');
    expect(store.sidebarOpen()).toBe(false);
    await choose(fixture, 'View', 'Metrics');
    expect(dock.calls).toContain('toggle:metrics');
    await choose(fixture, 'View', 'Inspector');
    expect(dock.calls).toContain('toggle:inspector');
    await choose(fixture, 'View', 'Theme', 'Light');
    expect(store.scheme()).toBe('light');
    await choose(fixture, 'View', 'Theme', 'System');
    expect(store.scheme()).toBe('system');
    await choose(fixture, 'View', 'Theme', 'Console');
    expect(store.theme()).toBe('console');
    await choose(fixture, 'Analyze', 'Marked events only');
    expect(store.markedOnly()).toBe(true);
    await choose(fixture, 'Analyze', 'Clear all filters');
    expect(store.markedOnly()).toBe(false);
    await choose(fixture, 'Analyze', 'Time zone', 'Local');
    expect(store.timezone()).toBe('local');
    await choose(fixture, 'Analyze', 'Time zone', 'UTC');
    expect(store.timezone()).toBe('utc');
    await choose(fixture, 'Analyze', 'Search events');
    expect(store.focusSearchNonce()).toBe(1);
    TestBed.inject(TimelineZoom).zoomIn();
    await choose(fixture, 'Analyze', 'Previous time range');
    expect(store.hasRange()).toBe(false);
    await choose(fixture, 'Analyze', 'Next time range');
    expect(store.hasRange()).toBe(true);
    await choose(fixture, 'Analyze', 'Fit time range');
    expect(store.hasRange()).toBe(false);
    await choose(fixture, 'File', 'Open files…');
    await settle(fixture);
    expect(store.summary()?.sources.some((s) => s.path === '/picked/file.log')).toBe(true);
    await choose(fixture, 'File', 'Re-import all');
    await choose(fixture, 'File', 'Close workspace');
    await settle(fixture);
    expect(store.summary()?.total).toBe(0);
    // Choosing an item closes the menus.
    expect(document.querySelector('.menu')).toBeNull();
  });

  it('focuses search with / unless typing, and opens it from the palette', async () => {
    const { fixture, store } = setup(TitleBar);
    await settle(fixture);
    document.dispatchEvent(new KeyboardEvent('keydown', { key: '/' }));
    expect(store.focusSearchNonce()).toBe(1);
    // Ctrl/⌘+/ is the query bar's shortcut, not the search box's.
    document.dispatchEvent(new KeyboardEvent('keydown', { key: '/', ctrlKey: true }));
    document.dispatchEvent(new KeyboardEvent('keydown', { key: '/', metaKey: true }));
    expect(store.focusSearchNonce()).toBe(1);
    const input = document.createElement('input');
    document.body.appendChild(input);
    input.dispatchEvent(new KeyboardEvent('keydown', { key: '/', bubbles: true }));
    expect(store.focusSearchNonce()).toBe(1);
    input.remove();
    el<HTMLButtonElement>(fixture, '.palette').click();
    expect(TestBed.inject(PaletteService).isOpen()).toBe(true);
  });

  it('lists the core views as checkable items at the top of View, and the rest in category submenus', async () => {
    const { fixture, dock } = setup(TitleBar);
    await settle(fixture);
    await choose(fixture, 'View');
    const view = document.querySelector<HTMLElement>('[role=menu][data-menu=View]')!;
    const top = [...view.children].filter((e) => e.matches('[role=menuitemcheckbox], [role=menuitem]'));
    const label = (e: Element) => (e.querySelector('.label')?.textContent ?? '').trim();
    expect(top.map(label)).toEqual(['Explorer', 'Fields', 'Inspector', 'Timeline', 'Events', 'Metrics', 'Profile editor', 'Plugins', 'Other windows…', 'Theme', 'Layout', 'Source colours']);
    expect(menuItem('Timeline').getAttribute('aria-checked')).toBe('true');
    expect(menuItem('Events').getAttribute('aria-checked')).toBe('false');
    expect(menuItem('Plugins').getAttribute('aria-haspopup')).toBe('menu');
    await choose(fixture, 'Plugins', 'Manage plugins');
    expect(dock.calls).toContain('toggle:plugins');
    await choose(fixture, 'View', 'Other windows…');
    expect(TestBed.inject(PaletteService).mode()).toBe('views');
  });

  it('has a Window menu with one checkable item per window; New and Close window are in File', async () => {
    const windowing = new MockWindowingBackend(memoryHost());
    TestBed.configureTestingModule({
      providers: [{ provide: WindowingBackend, useValue: windowing }, { provide: IMT_COMMANDS, multi: true, useFactory: () => TestBed.inject(WindowCommands).commands }],
    });
    const { fixture } = setup(TitleBar);
    await TestBed.inject(WindowingService).init();
    await settle(fixture);
    await choose(fixture, 'File');
    expect(menuItem('New window')).toBeTruthy();
    expect(menuItem('Close window')).toBeTruthy();
    await choose(fixture, 'New window');
    await choose(fixture, 'Window');
    const items = [...document.querySelectorAll<HTMLElement>('[role=menuitemcheckbox]')];
    expect(items.map((i) => i.getAttribute('aria-checked'))).toEqual(['true', 'false']);
    expect(items[1]?.textContent).toContain('IMT — w-2');
    at(items, 1).click();
    await settle(fixture);
    expect(windowing.calls).toEqual(['newWindow', 'focusWindow:w-2']);
  });

  it('hides the in-app menu bar in native mode (macOS) and mirrors the menus to the native menu bar', async () => {
    const sent: NativeMenuModel[] = [];
    let chosen: (id: string) => void = () => undefined;
    const port: NativeMenuPort = {
      setAppMenu: async (m) => void sent.push(m),
      onMenu: async (h) => {
        chosen = h;
        return () => undefined;
      },
    };
    TestBed.configureTestingModule({ providers: [{ provide: MENU_MODE, useValue: 'native' }, { provide: NATIVE_MENU_PORT, useValue: port }] });
    const { fixture, dock } = setup(TitleBar);
    await settle(fixture);
    expect(fixture.nativeElement.querySelector('imt-menu-bar')).toBeNull();
    expect(fixture.nativeElement.querySelector('[role=menubar]')).toBeNull();
    expect(fixture.nativeElement.querySelector('.palette')).not.toBeNull();
    expect(sent.length).toBe(1);
    expect(sent[0]?.menus.map((m) => m.id)).toEqual(['app', 'file', 'view', 'analyze', 'help']);
    expect(JSON.stringify(sent[0]?.menus[0])).toContain('settings.open');
    // About and Settings are in the app menu only.
    expect(JSON.stringify(at(sent, 0).menus.find((m) => m.id === 'help'))).not.toContain('help.about');
    chosen('view:metrics');
    expect(dock.calls).toContain('toggle:metrics');
    // Unchanged menus are not sent again; a change is.
    TestBed.inject(WorkspaceStore).setScheme('dark');
    await settle(fixture);
    expect(sent.length).toBe(2);
    await settle(fixture);
    expect(sent.length).toBe(2);
  });

  it('runs key-bound commands from the keyboard', async () => {
    const { fixture } = setup(TitleBar);
    await settle(fixture);
    document.dispatchEvent(new KeyboardEvent('keydown', { key: 'k', ctrlKey: true, bubbles: true }));
    expect(TestBed.inject(PaletteService).isOpen()).toBe(true);
  });

  it('shows busy state', async () => {
    const { fixture } = setup(TitleBar);
    fixture.detectChanges();
    expect(text(fixture)).toContain('Loading workspace');
  });

  it('embeds the project name, unsaved dot and Administrator badge at its end', async () => {
    const { fixture, store } = setup(TitleBar);
    await TestBed.inject(LifecycleService).init();
    await settle(fixture);
    const host = fixture.nativeElement as HTMLElement;
    expect(host.lastElementChild?.tagName.toLowerCase()).toBe('imt-title-extras');
    expect(el(fixture, '[data-testid=project-title]').textContent).toContain('Untitled');
    expect(host.querySelector('[data-testid=admin-badge]')).toBeNull();
    await store.openPaths(['/more/added.log']);
    await settle(fixture);
    expect(host.querySelector('[aria-label="Unsaved changes"]')).not.toBeNull();
  });
});

describe('StatusBar', () => {
  it('summarises the workspace', async () => {
    const { fixture, store } = setup(StatusBar);
    fixture.detectChanges();
    expect(text(fixture)).toContain('No workspace');
    await settle(fixture);
    store.setMatchTotal(3);
    await settle(fixture);
    expect(text(fixture)).toContain('2 sources');
    expect(text(fixture)).toContain('11 events');
    expect(text(fixture)).toMatch(/\d+ instances/);
    expect(text(fixture)).toContain('3 matching');
    expect(text(fixture)).toContain('Importers: json · text');
    el<HTMLButtonElement>(fixture, '.tz').click();
    expect(store.timezone()).toBe('local');
    await store.clearWorkspace();
    await settle(fixture);
    expect(text(fixture)).toContain('Importers: —');
    expect(text(fixture)).toContain('0 sources');
  });

  it('shows how many events the pipeline dropped', async () => {
    const { fixture, store, backend } = setup(StatusBar);
    await settle(fixture);
    expect(text(fixture)).not.toContain('dropped by pipeline');
    await store.applyProfile(await backend.saveProfile(DROP_DEBUG_PROFILE));
    await settle(fixture);
    expect(text(fixture)).toContain('1 dropped by pipeline');
  });

  it('has a Live tail toggle that reflects and switches the live state', async () => {
    const { fixture, store } = setup(StatusBar);
    await settle(fixture);
    const live = TestBed.inject(LiveTail);
    const button = el<HTMLButtonElement>(fixture, 'button[aria-label="Live tail"]');
    expect(button.tagName).toBe('BUTTON');
    expect(button.getAttribute('aria-pressed')).toBe('false');
    expect(button.disabled).toBe(false);
    expect(button.title).toContain('Follow the opened files');
    button.click();
    await vi.waitFor(() => expect(live.enabled()).toBe(true));
    await settle(fixture);
    expect(button.getAttribute('aria-pressed')).toBe('true');
    expect(button.classList).toContain('on');
    expect(button.title).toContain('Click to stop');
    // While on it can always be turned off, even with nothing left to follow.
    await store.clearWorkspace();
    await settle(fixture);
    expect(button.disabled).toBe(false);
    button.click();
    await vi.waitFor(() => expect(live.enabled()).toBe(false));
    await settle(fixture);
    expect(button.getAttribute('aria-pressed')).toBe('false');
    expect(button.disabled).toBe(true);
    expect(button.title).toContain('Open files');
  });

  it('shows why live tail failed', async () => {
    const { fixture, backend } = setup(StatusBar);
    await settle(fixture);
    vi.spyOn(backend, 'setLive').mockRejectedValueOnce('watch limit reached');
    el<HTMLButtonElement>(fixture, 'button[aria-label="Live tail"]').click();
    await vi.waitFor(() => expect(TestBed.inject(LiveTail).error()).toBe('watch limit reached'));
    await settle(fixture);
    const button = el<HTMLButtonElement>(fixture, 'button[aria-label="Live tail"]');
    expect(button.getAttribute('aria-pressed')).toBe('false');
    expect(button.classList).toContain('failed');
    expect(button.title).toBe('Live tail failed: watch limit reached');
  });

  it('pluralises one source', async () => {
    const { fixture, store } = setup(StatusBar);
    await settle(fixture);
    await store.removeSource(1);
    await settle(fixture);
    expect(el(fixture, '.strong').textContent?.trim()).toBe('1 source');
  });
});

describe('Sidebar', () => {
  // The Fields section starts collapsed; these tests use it expanded.
  beforeEach(() => localStorage.setItem(FIELDS_OPEN_KEY, 'true'));
  afterEach(() => localStorage.removeItem(FIELDS_OPEN_KEY));
  it('lists sources and unrecognised files and manages them', async () => {
    const { fixture, store } = setup(Sidebar);
    await settle(fixture);
    expect(all(fixture, '.src:not(.dim) .name').map((n) => n.textContent)).toEqual(['web01/app.clef', 'web02/app.log']);
    expect(el(fixture, '.src.dim').textContent).toContain('system/export.reg');
    expect(el(fixture, '.src').getAttribute('title')).toContain('7 events');
    el<HTMLButtonElement>(fixture, '[aria-label="Remove app.log"]').click();
    await settle(fixture);
    expect(store.summary()?.sources).toHaveLength(1);
    el<HTMLButtonElement>(fixture, '[aria-label="Open files"]').click();
    await settle(fixture);
    el<HTMLButtonElement>(fixture, '[aria-label="Open folder"]').click();
    await settle(fixture);
    expect(store.summary()?.sources).toHaveLength(3);
    el<HTMLButtonElement>(fixture, '[aria-label="Re-import all"]').click();
    await settle(fixture);
    expect(store.summary()?.sources).toHaveLength(2);
  });

  it('mentions pipeline-dropped events in the source tooltip', async () => {
    const { fixture, store, backend } = setup(Sidebar);
    await settle(fixture);
    expect(at(all(fixture, '.src'), 1).getAttribute('title')).not.toContain('dropped');
    await store.applyProfile(await backend.saveProfile(DROP_DEBUG_PROFILE));
    await settle(fixture);
    expect(all(fixture, '.src')[1]?.getAttribute('title')).toContain('1 dropped by pipeline');
  });

  it('shows an empty state without the Fields section; the header buttons open files (the welcome page offers them too)', async () => {
    const { fixture, store } = setup(Sidebar);
    await settle(fixture);
    await store.clearWorkspace();
    await settle(fixture);
    expect(text(fixture)).toContain('No files open');
    expect(fixture.nativeElement.querySelector('imt-fields-panel')).toBeNull();
    expect(all(fixture, '.empty button')).toHaveLength(0);
    el<HTMLButtonElement>(fixture, 'button[aria-label="Open folder"]').click();
    await settle(fixture);
    expect(store.summary()?.sources).toHaveLength(1);
    expect(fixture.nativeElement.querySelector('imt-fields-panel')).not.toBeNull();
    await store.clearWorkspace();
    await settle(fixture);
    el<HTMLButtonElement>(fixture, 'button[aria-label="Open files"]').click();
    await settle(fixture);
    expect(store.summary()?.sources).toHaveLength(1);
  });

  it('shows source errors', async () => {
    const backend = tinyBackend();
    const original = backend.workspaceSummary.bind(backend);
    vi.spyOn(backend, 'workspaceSummary').mockImplementation(async () => {
      const s = await original();
      at(s.sources, 0).error = 'Permission denied';
      return s;
    });
    const { fixture } = setup(Sidebar, backend);
    await settle(fixture);
    expect(el(fixture, '.src-error').textContent).toBe('Permission denied');
  });

  it('filters fields and opens facets with actions', async () => {
    const { fixture, store } = setup(Sidebar);
    await settle(fixture);
    typeInto(el<HTMLInputElement>(fixture, 'input.filter'), 'machine');
    await settle(fixture);
    const fields = all<HTMLButtonElement>(fixture, '.field');
    expect(fields).toHaveLength(1);
    expect(fields[0]?.classList.contains('grouped')).toBe(true);
    at(fields, 0).click();
    await settle(fixture);
    expect(text(fixture)).toContain('2 values in current results');
    expect(all(fixture, '.value')).toHaveLength(2);
    el<HTMLButtonElement>(fixture, '[aria-label="Filter MachineName equals WEB01"]').click();
    expect(store.query().filters).toEqual([{ field: 'MachineName', op: 'eq', value: 'WEB01' }]);
    await settle(fixture);
    expect(fixture.nativeElement.querySelector('.facet')).toBeNull();

    at(fields, 0).click();
    await settle(fixture);
    el<HTMLButtonElement>(fixture, '[aria-label="Filter MachineName not equal WEB01"]').click();
    at(fields, 0).click();
    await settle(fixture);
    buttonByText(fixture, 'Group by').click();
    expect(store.effectiveGroupBy()).toEqual(['MachineName']);
    at(fields, 0).click();
    await settle(fixture);
    buttonByText(fixture, 'Add to group').click();
    expect(store.effectiveGroupBy()).toEqual(['MachineName']);
    at(fields, 0).click();
    await settle(fixture);
    buttonByText(fixture, 'Add column').click();
    expect(store.extraColumns()).toContain('MachineName');
    at(fields, 0).click();
    await settle(fixture);
    at(fields, 0).click();
    await settle(fixture);
    expect(fixture.nativeElement.querySelector('.facet')).toBeNull();
  });

  it('marks derived fields and handles facet errors', async () => {
    const backend = tinyBackend();
    const { fixture, store } = setup(Sidebar, backend);
    await settle(fixture);
    typeInto(el<HTMLInputElement>(fixture, 'input.filter'), 'cpu');
    await settle(fixture);
    expect(el(fixture, '.field .badge').textContent).toBe('fx');
    vi.spyOn(backend, 'queryFacet').mockRejectedValueOnce('facet failed');
    el<HTMLButtonElement>(fixture, '.field').click();
    await settle(fixture);
    expect(store.error()).toBeNull();
    expect(el(fixture, '.facet [role=alert]').textContent).toContain('facet failed');
    buttonByText(fixture, 'Retry').click();
    await settle(fixture);
    expect(all(fixture, '.facet [role=alert]')).toHaveLength(0);
  });
});

function typeInto(input: HTMLInputElement, value: string): void {
  input.value = value;
  input.dispatchEvent(new Event('input'));
}
