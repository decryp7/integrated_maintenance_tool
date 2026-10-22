import { TestBed } from '@angular/core/testing';
import { provideDockTesting, settle } from '../../testing/helpers';
import { WorkspaceStore } from '../core/workspace.store';
import { DIALOG_REF, DialogService } from '../ui/dialog.service';
import { DockCommands } from './dock-commands';
import { DockService } from './dock.service';
import { DropGuides } from './drop-guides';
import { ManageLayoutsDialog, SaveLayoutDialog } from './layout-dialogs';
import { LAYOUT_STORE, toEnvelope } from './layout-store';
import { NAMED_LAYOUT_STORE, NamedLayout, NamedLayoutStore } from './named-layouts';
import { dockLayoutContribution } from './provide-docking';
import { WindowLayouts } from './window-layouts';
import { loadDockview } from './dockview-loader';
import { at } from '../../testing/must';

// dockview is loaded on demand (see dockview-loader); load it once so attach() is synchronous here.
beforeAll(() => loadDockview());

describe('window layouts, dialogs and commands', () => {
  let container: HTMLElement;
  let stored: readonly NamedLayout[];

  const close = vi.fn();

  function setup() {
    close.mockReset();
    stored = [];
    const named: NamedLayoutStore = { load: () => [...stored], save: (l) => (stored = l) };
    TestBed.configureTestingModule({
      providers: [provideDockTesting(), { provide: LAYOUT_STORE, useValue: { load: () => null, save: () => undefined, clear: () => undefined } }, { provide: NAMED_LAYOUT_STORE, useValue: named }, { provide: DIALOG_REF, useValue: { close, closed: Promise.resolve() } }],
    });
    container = document.createElement('div');
    document.body.appendChild(container);
    const dock = TestBed.inject(DockService);
    dock.attach(container);
    return { dock, layouts: TestBed.inject(WindowLayouts), store: TestBed.inject(WorkspaceStore) };
  }

  afterEach(() => {
    TestBed.inject(DockService).detach();
    container.remove();
  });

  it('saves the current arrangement by name and applies it (by name and by number)', () => {
    const { dock, layouts, store } = setup();
    expect(layouts.saveCurrent('  ')).toBeNull();
    expect(layouts.saveCurrent('Triage')).toBe('Triage');
    expect(store.notice()).toContain('Triage');
    expect(stored.map((l) => l.name)).toEqual(['Triage']);
    dock.show('metrics');
    expect(layouts.has(1)).toBe(true);
    expect(layouts.applyAt(1)).toBe(true);
    expect(dock.isOpen('metrics')).toBe(false);
    expect(layouts.applyAt(2)).toBe(false);
    expect(layouts.apply('nope')).toBe(false);
    layouts.rename('Triage', 'Debug');
    layouts.move('Debug', 1);
    layouts.remove('Debug');
    expect(stored).toEqual([]);
  });

  it('reports a saved layout that cannot be applied', () => {
    const { dock, layouts, store } = setup();
    layouts.saveCurrent('Broken');
    stored = [{ ...at(stored, 0), saved: { ...at(stored, 0).saved, layout: { grid: null } as never } }];
    const fresh = TestBed.runInInjectionContext(() => new WindowLayouts());
    expect(fresh.apply('Broken')).toBe(false);
    expect(store.error()).toContain('could not be applied');
    expect(dock.isOpen('timeline')).toBe(true);
  });

  it('the save dialog asks for a name and refuses an empty one', async () => {
    setup();
    const fixture = TestBed.createComponent(SaveLayoutDialog);
    await settle(fixture);
    const el = fixture.nativeElement as HTMLElement;
    const input = el.querySelector<HTMLInputElement>('#imt-layout-name')!;
    expect(input.value).toBe('Layout 1');
    input.value = '   ';
    input.dispatchEvent(new Event('input'));
    el.querySelector('form')!.dispatchEvent(new Event('submit'));
    await settle(fixture);
    expect(el.querySelector('[role=alert]')!.textContent).toContain('Type a name');
    input.value = 'Mine';
    input.dispatchEvent(new Event('input'));
    el.querySelector('form')!.dispatchEvent(new Event('submit'));
    expect(close).toHaveBeenCalledWith('Mine');
  });

  it('the manage dialog applies, renames, reorders, deletes and resets', async () => {
    const { dock, layouts } = setup();
    layouts.saveCurrent('One');
    layouts.saveCurrent('Two');
    const fixture = TestBed.createComponent(ManageLayoutsDialog);
    await settle(fixture);
    const el = fixture.nativeElement as HTMLElement;
    const button = (label: string) => el.querySelector<HTMLButtonElement>(`button[aria-label="${label}"]`)!;
    expect(el.textContent).toContain('Ctrl+Alt+1');
    button('Move Two up').click();
    expect(layouts.list().map((l) => l.name)).toEqual(['Two', 'One']);
    button('Rename One').click();
    await settle(fixture);
    const input = el.querySelector<HTMLInputElement>('input[aria-label="New name for One"]')!;
    input.value = 'Uno';
    input.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter' }));
    await settle(fixture);
    expect(layouts.list().map((l) => l.name)).toEqual(['Two', 'Uno']);
    button('Rename Uno').click();
    await settle(fixture);
    el.querySelector('input')!.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' }));
    button('Delete Two').click();
    await settle(fixture);
    button('Apply Uno').click();
    expect(close).toHaveBeenCalled();
    dock.show('metrics');
    dock.pin('inspector');
    [...el.querySelectorAll<HTMLButtonElement>('button')].find((b) => b.textContent!.includes('Reset window layout'))!.click();
    expect(dock.isOpen('metrics')).toBe(false);
    expect(dock.autoHide().items.map((i) => i.id)).toEqual(['inspector']);
    layouts.remove('Uno');
    await settle(fixture);
    expect(el.textContent).toContain('No saved window layouts yet');
  });

  it('registers the window commands and runs them', async () => {
    const { dock, layouts } = setup();
    const open = vi.spyOn(TestBed.inject(DialogService), 'open');
    const commands = TestBed.inject(DockCommands).commands;
    const run = (id: string) => commands.find((c) => c.id === id)!.run({} as never);
    const when = (id: string) => commands.find((c) => c.id === id)!.when?.({} as never) ?? true;
    expect(commands.find((c) => c.id === 'window.layout.apply1')!.keybinding).toBe('Ctrl+Alt+1');
    expect(when('window.layout.apply1')).toBe(false);
    expect(when('window.popOut')).toBe(false);
    // The inspector starts auto-hidden.
    expect(when('window.pinAll')).toBe(true);
    await run('window.pinAll');
    expect(when('window.pinAll')).toBe(false);
    run('window.layout.save');
    run('window.layout.manage');
    run('window.layout.apply');
    expect(open).toHaveBeenCalledTimes(3);
    layouts.saveCurrent('A');
    expect(when('window.layout.apply1')).toBe(true);
    await run('window.layout.apply1');
    await run('window.autoHideAll');
    expect(when('window.pinAll')).toBe(true);
    await run('window.pinAll');
    await run('window.autoHide');
    await run('window.panelMenu');
    await run('window.switcher');
    expect(dock.switcher()).not.toBeNull();
    dock.cancelSwitch();
    await run('window.fullScreen');
    expect(dock.fullScreen()).toBe(true);
    await run('window.layout.reset');
    expect(dock.fullScreen()).toBe(false);
    await run('window.closeAllDocuments');
    expect(dock.isOpen('timeline')).toBe(false);
    await run('window.popOut');
  });

  it('floats the active docked panel from the Window menu command, and only a docked one', () => {
    const { dock } = setup();
    const commands = TestBed.inject(DockCommands).commands;
    const float = commands.find((c) => c.id === 'window.float')!;
    expect(float.title).toBe('Float panel');
    dock.show('metrics');
    expect(float.when!({} as never)).toBe(true);
    float.run({} as never);
    expect(dock['api']!.getPanel('metrics')!.group.api.location.type).toBe('floating');
    expect(float.when!({} as never)).toBe(false);
    expect(dock.floatActive()).toBe(false);
    dock.detach();
  });

  it('saves the whole arrangement in projects and sessions without marking them unsaved', async () => {
    const { dock } = setup();
    const c = dockLayoutContribution(dock);
    expect(c.tracksChanges).toBe(false);
    dock.unpin('inspector');
    await new Promise((r) => setTimeout(r, 300));
    const captured = c.capture() as ReturnType<typeof toEnvelope>;
    expect(captured.version).toBe(2);
    dock.pinAll();
    c.restore(captured);
    expect(dock.autoHide().items).toHaveLength(1);
    c.restore('garbage');
    expect(dock.autoHide().items).toHaveLength(1);
    dock.detach();
    expect(c.capture()).toBeNull();
  });
});

describe('DropGuides', () => {
  it('follows a pointer drag from a tab: the compass over the group under it, the edge guides, the hit cell', () => {
    const root = document.createElement('div');
    root.innerHTML = '<div class="dv-tab"></div><div class="other"></div><div class="dv-groupview"><div class="dv-content-container"></div></div>';
    document.body.appendChild(root);
    const rect = (x: number, y: number, w: number, h: number) => () => ({ left: x, top: y, width: w, height: h, right: x + w, bottom: y + h, x, y, toJSON: () => ({}) }) as DOMRect;
    root.getBoundingClientRect = rect(0, 0, 800, 600);
    const content = root.querySelector<HTMLElement>('.dv-content-container')!;
    content.getBoundingClientRect = rect(100, 100, 400, 300);
    const original = document.elementFromPoint;
    document.elementFromPoint = () => content;
    const guides = new DropGuides(root);
    const layer = root.querySelector('.imt-drop-guides')!;
    const pointer = (type: string, target: EventTarget, x: number, y: number, button = 0) => target.dispatchEvent(new PointerEvent(type, { bubbles: true, clientX: x, clientY: y, button }));
    // Presses elsewhere, with another button, or a press that does not move far enough: no guides.
    pointer('pointerdown', root.querySelector('.other')!, 0, 0);
    pointer('pointermove', window, 300, 300);
    pointer('pointerdown', root.querySelector('.dv-tab')!, 0, 0, 2);
    pointer('pointermove', window, 300, 300);
    pointer('pointerdown', root.querySelector('.dv-tab')!, 0, 0);
    pointer('pointermove', window, 2, 2);
    expect(layer.classList.contains('on')).toBe(false);
    // The compass centre of the 400x300 content at (100,100) is at (300,250).
    pointer('pointermove', window, 300, 250);
    expect(layer.classList.contains('on')).toBe(true);
    expect(root.querySelectorAll('.imt-guide-edges .imt-guide')).toHaveLength(4);
    expect(root.querySelectorAll('.imt-guide-compass .imt-guide')).toHaveLength(5);
    expect(root.querySelector('.imt-guide.hot')!.getAttribute('data-position')).toBe('center');
    document.elementFromPoint = () => null;
    pointer('pointermove', window, 26, 300);
    expect(root.querySelectorAll('.imt-guide-compass .imt-guide')).toHaveLength(0);
    expect(root.querySelector('.imt-guide-edges .imt-guide.hot')!.getAttribute('data-position')).toBe('left');
    pointer('pointerup', window, 26, 300);
    expect(layer.classList.contains('on')).toBe(false);
    // Driven from outside (a pop-out over the window), with a preview of the drop.
    guides.showAt({ x: 26, y: 300 }, { x: 0, y: 0, width: 200, height: 600 });
    expect(layer.classList.contains('on')).toBe(true);
    const preview = root.querySelector<HTMLElement>('.imt-guide-preview')!;
    expect([preview.hidden, preview.style.width]).toEqual([false, '200px']);
    guides.showAt({ x: 400, y: 30 });
    expect(preview.hidden).toBe(true);
    guides.hide();
    guides.hide();
    pointer('pointercancel', window, 0, 0);
    guides.dispose();
    expect(root.querySelector('.imt-drop-guides')).toBeNull();
    document.elementFromPoint = original;
    root.remove();
  });

  it('shows no guides while a floating window is moved by its tab strip; Shift+drag (redock) shows them', () => {
    const root = document.createElement('div');
    root.innerHTML = '<div class="dv-resize-container"><div class="dv-void-container"></div></div>';
    document.body.appendChild(root);
    const guides = new DropGuides(root);
    const layer = root.querySelector('.imt-drop-guides')!;
    const handle = root.querySelector('.dv-void-container')!;
    const pointer = (type: string, target: EventTarget, x: number, shiftKey = false) =>
      target.dispatchEvent(new PointerEvent(type, { bubbles: true, clientX: x, clientY: 0, button: 0, pointerType: 'mouse', shiftKey }));
    pointer('pointerdown', handle, 0);
    pointer('pointermove', window, 100);
    expect(layer.classList.contains('on')).toBe(false);
    pointer('pointerup', window, 100);
    pointer('pointerdown', handle, 0, true);
    pointer('pointermove', window, 100);
    expect(layer.classList.contains('on')).toBe(true);
    guides.dispose();
    root.remove();
  });
});
