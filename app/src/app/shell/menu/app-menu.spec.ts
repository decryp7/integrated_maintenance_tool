import { signal } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { FakeDock, provideFakeDock, provideTinyBackend, settle, testLogger } from '../../../testing/helpers';
import { provideToolsBackend } from '../../core/tools-backend';
import { WorkspaceStore } from '../../core/workspace.store';
import { PANEL_WINDOWS, PanelWindows } from '../../dock/panel-windows';
import { PaletteService } from '../../palette/palette.service';
import { WindowingService } from '../../windowing/windowing.service';
import { AppMenu, MENU_MODE } from './app-menu.service';
import { RecentOpener } from './recent-opener';
import { TopMenu } from './menu-model';
import { NATIVE_MENU_PORT, NativeMenuBridge, NativeMenuPort, toNativeModel } from './native-menu';
import { NativeMenuModel } from './native-menu.types';
import { at } from '../../../testing/must';

function setup(extra: unknown[] = []) {
  const dock = new FakeDock();
  TestBed.configureTestingModule({ providers: [provideTinyBackend(), provideFakeDock(dock), provideToolsBackend(), ...(extra as never[])] });
  return { dock, app: TestBed.inject(AppMenu), store: TestBed.inject(WorkspaceStore) };
}

describe('AppMenu', () => {
  it('defaults to the in-app menu bar outside the macOS desktop app', () => {
    setup();
    expect(TestBed.inject(MENU_MODE)).toBe('in-app');
  });

  it('runs view toggles, commands, the window picker, windows and pop-outs; ignores unknown ids', async () => {
    const focused: string[] = [];
    const popouts: Partial<PanelWindows> = { poppedIds: signal(new Set(['inspector'])), focus: (id) => void focused.push(id) };
    const { app, dock, store } = setup([{ provide: PANEL_WINDOWS, useValue: popouts }]);
    const windowing = TestBed.inject(WindowingService);
    const focus = vi.spyOn(windowing, 'focus').mockResolvedValue();
    app.run('view:metrics');
    expect(dock.calls).toEqual(['toggle:metrics']);
    app.run('view.toggleSidebar');
    await settle();
    expect(store.sidebarOpen()).toBe(false);
    app.run('menu.otherWindows');
    expect(TestBed.inject(PaletteService).mode()).toBe('views');
    expect(TestBed.inject(PaletteService).isOpen()).toBe(true);
    app.run('win:w-2');
    expect(focus).toHaveBeenCalledWith('w-2');
    app.run('popout:inspector');
    expect(focused).toEqual(['inspector']);
    app.run('no.such.command');
    await settle();
    expect(store.error()).toBeNull();
    // Pop-outs are listed in the Window menu.
    const window = app.menus().find((m) => m.id === 'window');
    expect(JSON.stringify(window)).toContain('popout:inspector');
  });

  it('runs the Open recent items through the opener', () => {
    const { app } = setup();
    const opener = TestBed.inject(RecentOpener);
    const file = vi.spyOn(opener, 'openFile').mockResolvedValue(undefined);
    const project = vi.spyOn(opener, 'openProject').mockResolvedValue();
    const clear = vi.spyOn(opener, 'clear').mockResolvedValue();
    app.run('recent:file:/logs/a.log');
    app.run('recent:project:/p/x.imtproj');
    app.run('menu.recentClear');
    expect(file).toHaveBeenCalledWith('/logs/a.log');
    expect(project).toHaveBeenCalledWith('/p/x.imtproj');
    expect(clear).toHaveBeenCalledOnce();
  });

  it('Fields shows the sidebar and focuses the fields filter', async () => {
    const { app, store } = setup();
    store.toggleSidebar();
    const host = document.createElement('imt-fields-panel');
    host.innerHTML = '<input aria-label="Filter fields" />';
    document.body.appendChild(host);
    app.run('menu.fields');
    await settle();
    expect(store.sidebarOpen()).toBe(true);
    expect(document.activeElement).toBe(host.querySelector('input'));
    host.remove();
  });
});

describe('native menu bar', () => {
  const MENUS: TopMenu[] = [
    {
      id: 'edit',
      label: 'Edit',
      items: [{ kind: 'item', id: 'edit.undo', label: 'Undo filter change', keybinding: 'Mod+Z' }],
    },
    {
      id: 'view',
      label: 'View',
      items: [
        { kind: 'item', id: 'view.toggleSidebar', label: 'Explorer', keybinding: 'Mod+B', checked: true },
        { kind: 'separator' },
        { kind: 'submenu', id: 'view.analysis', label: 'Analysis', items: [{ kind: 'item', id: 'view:anomalies', label: 'Anomalies', checked: false }] },
        { kind: 'group', label: 'Windows registry', items: [{ kind: 'item', id: 'view:registry', label: 'Registry', checked: false }] },
      ],
    },
  ];

  it('turns the menus into the wire model (groups as captions; no shortcut where the standard Edit items have one)', () => {
    expect(toNativeModel(MENUS)).toEqual({
      menus: [
        { id: 'edit', label: 'Edit', items: [{ kind: 'item', id: 'edit.undo', label: 'Undo filter change', keybinding: null, checked: null, enabled: true }] },
        {
          id: 'view',
          label: 'View',
          items: [
            { kind: 'item', id: 'view.toggleSidebar', label: 'Explorer', keybinding: 'Mod+B', checked: true, enabled: true },
            { kind: 'separator' },
            { kind: 'submenu', label: 'Analysis', items: [{ kind: 'item', id: 'view:anomalies', label: 'Anomalies', keybinding: null, checked: false, enabled: true }] },
            { kind: 'header', label: 'Windows registry' },
            { kind: 'item', id: 'view:registry', label: 'Registry', keybinding: null, checked: false, enabled: true },
          ],
        },
      ],
    });
  });

  function bridge(port: Partial<NativeMenuPort>) {
    const menus = signal(MENUS);
    const ran: string[] = [];
    TestBed.configureTestingModule({
      providers: [
        provideTinyBackend(),
        provideFakeDock(),
        provideToolsBackend(),
        { provide: AppMenu, useValue: { menus, run: (id: string) => ran.push(id), mode: 'native' } },
        { provide: NATIVE_MENU_PORT, useValue: { setAppMenu: async () => undefined, onMenu: async () => () => undefined, ...port } },
      ],
    });
    const b = TestBed.inject(NativeMenuBridge);
    b.start();
    b.start();
    return { b, menus, ran };
  }

  it('sends the model once per change and again when the window gets the focus', async () => {
    const sent: NativeMenuModel[] = [];
    const { menus } = bridge({ setAppMenu: async (m) => void sent.push(m) });
    await settle();
    expect(sent).toHaveLength(1);
    menus.set([...MENUS]);
    await settle();
    expect(sent).toHaveLength(1);
    menus.set([at(MENUS, 0)]);
    await settle();
    expect(sent).toHaveLength(2);
    window.dispatchEvent(new Event('focus'));
    expect(sent).toHaveLength(3);
  });

  it('retries after a failed update and logs it', async () => {
    let fail = true;
    const sent: NativeMenuModel[] = [];
    bridge({
      setAppMenu: async (m) => {
        if (fail) throw new Error('no menu');
        sent.push(m);
      },
    });
    await settle();
    expect(testLogger().entries.some((e) => e.category === 'ui.menu' && e.level === 'warn')).toBe(true);
    fail = false;
    window.dispatchEvent(new Event('focus'));
    await settle();
    expect(sent).toHaveLength(1);
  });

  it('runs chosen items, but not the echo of a shortcut that just ran the same command', async () => {
    let chosen: (id: string) => void = () => undefined;
    const { ran } = bridge({
      onMenu: async (h) => {
        chosen = h;
        return () => undefined;
      },
    });
    await settle();
    chosen('view:anomalies');
    expect(ran).toEqual(['view:anomalies']);
    // Ctrl/Cmd+K runs the palette command from the page; the menu bar's echo of it is ignored.
    document.dispatchEvent(new KeyboardEvent('keydown', { key: 'k', ctrlKey: true, metaKey: false, bubbles: true }));
    document.dispatchEvent(new KeyboardEvent('keydown', { key: 'k', ctrlKey: false, metaKey: true, bubbles: true }));
    chosen('palette.open');
    expect(ran).toEqual(['view:anomalies']);
    chosen('file.openFiles');
    expect(ran).toEqual(['view:anomalies', 'file.openFiles']);
  });

  it('degrades when native menu events are unavailable', async () => {
    bridge({ onMenu: () => Promise.reject(new Error('no events')) });
    await settle();
    expect(testLogger().entries.some((e) => e.message.includes('Native menu events are unavailable'))).toBe(true);
  });
});
