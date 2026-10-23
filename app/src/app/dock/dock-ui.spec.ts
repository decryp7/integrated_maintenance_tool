import { CommandRegistry } from '../extensions/command-registry';
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
import { PERSPECTIVE_KEYS, PERSPECTIVE_STORE, PerspectiveState, PerspectiveStore, Perspectives } from './perspectives';
import { WindowLayouts } from './window-layouts';
import { loadDockview } from './dockview-loader';
import { at } from '../../testing/must';

// dockview is loaded on demand (see dockview-loader); load it once so attach() is synchronous here.
beforeAll(() => loadDockview());

/** Perspective state kept in memory, so specs do not share it through localStorage. */
function memoryPerspectives(): PerspectiveStore {
  let state: PerspectiveState = { active: 'investigate', layouts: {} };
  return { load: () => state, save: (s) => void (state = s) };
}

describe('window layouts, dialogs and commands', () => {
  let container: HTMLElement;
  let stored: readonly NamedLayout[];

  const close = vi.fn();

  function setup() {
    close.mockReset();
    stored = [];
    const named: NamedLayoutStore = { load: () => [...stored], save: (l) => (stored = l) };
    TestBed.configureTestingModule({
      providers: [provideDockTesting(), { provide: LAYOUT_STORE, useValue: { load: () => null, save: () => undefined, clear: () => undefined } }, { provide: NAMED_LAYOUT_STORE, useValue: named }, { provide: DIALOG_REF, useValue: { close, closed: Promise.resolve() } }, { provide: PERSPECTIVE_STORE, useValue: memoryPerspectives() }],
    });
    container = document.createElement('div');
    document.body.appendChild(container);
    const dock = TestBed.inject(DockService);
    dock.attach(container);
    return { dock, layouts: TestBed.inject(WindowLayouts), store: TestBed.inject(WorkspaceStore), perspectives: TestBed.inject(Perspectives) };
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

  it('the save dialog asks for a name, refuses an empty one and makes the saved perspective the active one', async () => {
    const { perspectives } = setup();
    const fixture = TestBed.createComponent(SaveLayoutDialog);
    await settle(fixture);
    const el = fixture.nativeElement as HTMLElement;
    const input = el.querySelector<HTMLInputElement>('#debrief-layout-name')!;
    expect(input.value).toBe(`Perspective ${perspectives.list().length + 1}`);
    input.value = '   ';
    input.dispatchEvent(new Event('input'));
    el.querySelector('form')!.dispatchEvent(new Event('submit'));
    await settle(fixture);
    expect(el.querySelector('[role=alert]')!.textContent).toContain('Type a name');
    expect(close).not.toHaveBeenCalled();
    expect(perspectives.list().some((p) => !p.builtIn)).toBe(false);
    input.value = 'Mine';
    input.dispatchEvent(new Event('input'));
    el.querySelector('form')!.dispatchEvent(new Event('submit'));
    expect(close).toHaveBeenCalledWith('Mine');
    expect(perspectives.active().id).toBe('saved:Mine');
  });

  it('the save dialog cancels without saving', async () => {
    const { perspectives } = setup();
    const fixture = TestBed.createComponent(SaveLayoutDialog);
    await settle(fixture);
    [...(fixture.nativeElement as HTMLElement).querySelectorAll<HTMLButtonElement>('button')].find((b) => b.textContent!.includes('Cancel'))!.click();
    expect(close).toHaveBeenCalledWith();
    expect(perspectives.list().some((p) => !p.builtIn)).toBe(false);
  });

  it('the manage dialog lists the perspectives with their keys and switches to one', async () => {
    const { dock, perspectives } = setup();
    const fixture = TestBed.createComponent(ManageLayoutsDialog);
    await settle(fixture);
    const el = fixture.nativeElement as HTMLElement;
    const rows = [...el.querySelectorAll('[data-testid=perspective-row]')];
    expect(rows.map((r) => r.querySelector('.name')!.textContent!.replace(/\s+/g, ' ').trim())).toEqual(['Investigate(current)', 'Diagnostics']);
    expect(rows[0]!.textContent).toContain('1');
    expect(rows[1]!.textContent).toContain('2');
    // Built-in perspectives can be switched to, but not renamed, moved or deleted.
    expect(el.querySelector('button[aria-label="Rename Diagnostics"]')).toBeNull();
    expect(el.querySelector<HTMLButtonElement>('button[aria-label="Switch to Investigate"]')!.disabled).toBe(true);
    el.querySelector<HTMLButtonElement>('button[aria-label="Switch to Diagnostics"]')!.click();
    expect(perspectives.active().id).toBe('diagnostics');
    expect(close).toHaveBeenCalled();
    expect(dock.isOpen('timeline')).toBe(true);
  });

  it('the manage dialog renames, reorders and deletes saved perspectives', async () => {
    const { perspectives } = setup();
    perspectives.saveCurrent('One');
    perspectives.saveCurrent('Two');
    const fixture = TestBed.createComponent(ManageLayoutsDialog);
    await settle(fixture);
    const el = fixture.nativeElement as HTMLElement;
    const button = (label: string) => el.querySelector<HTMLButtonElement>(`button[aria-label="${label}"]`)!;
    const saved = () => perspectives.list().filter((p) => !p.builtIn).map((p) => p.name);
    expect(button('Move One up').disabled).toBe(true);
    expect(button('Move Two down').disabled).toBe(true);
    button('Move Two up').click();
    expect(saved()).toEqual(['Two', 'One']);
    await settle(fixture);
    // Rename One (Enter confirms), keeping it where it is.
    button('Rename One').click();
    await settle(fixture);
    const input = el.querySelector<HTMLInputElement>('input[aria-label="New name for One"]')!;
    input.value = 'Uno';
    input.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter' }));
    await settle(fixture);
    expect(saved()).toEqual(['Two', 'Uno']);
    expect(el.querySelector('input')).toBeNull();
    // Escape abandons a rename.
    button('Rename Uno').click();
    await settle(fixture);
    el.querySelector('input')!.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', cancelable: true }));
    await settle(fixture);
    expect(el.querySelector('input')).toBeNull();
    expect(saved()).toEqual(['Two', 'Uno']);
    // Deleting the active one falls back to Investigate.
    expect(perspectives.active().id).toBe('saved:Two');
    button('Delete Two').click();
    await settle(fixture);
    expect(saved()).toEqual(['Uno']);
    expect(perspectives.active().id).toBe('investigate');
    expect(el.querySelectorAll('[data-testid=perspective-row]')).toHaveLength(3);
  });

  it('the manage dialog resets the active perspective', async () => {
    const { dock, perspectives } = setup();
    dock.show('metrics');
    expect(dock.isOpen('metrics')).toBe(true);
    const fixture = TestBed.createComponent(ManageLayoutsDialog);
    await settle(fixture);
    [...(fixture.nativeElement as HTMLElement).querySelectorAll<HTMLButtonElement>('button')].find((b) => b.textContent!.includes('Reset perspective'))!.click();
    expect(dock.isOpen('metrics')).toBe(false);
    expect(dock.autoHide().items.map((i) => i.id)).toEqual(['inspector']);
    expect(perspectives.active().id).toBe('investigate');
    expect(close).toHaveBeenCalled();
  });

  it('registers the window commands and runs them', async () => {
    const { dock, perspectives } = setup();
    const open = vi.spyOn(TestBed.inject(DialogService), 'open');
    const commands = TestBed.inject(DockCommands).commands;
    const find = (id: string) => commands.find((c) => c.id === id)!;
    const run = (id: string) => find(id).run({} as never);
    const when = (id: string) => find(id).when?.({} as never) ?? true;
    // The old numbered window-layout commands are gone; perspectives take Mod+1..9.
    expect(commands.some((c) => c.id.startsWith('window.layout.apply'))).toBe(false);
    expect(commands.filter((c) => c.id.startsWith('window.perspective.')).map((c) => c.keybinding)).toEqual(Array.from({ length: PERSPECTIVE_KEYS }, (_, i) => `Mod+${i + 1}`));
    expect(find('window.perspective.2').title).toBe('Perspective: Diagnostics');
    expect(find('window.perspective.5').title).toBe('Perspective: 5');
    expect(when('window.perspective.1')).toBe(true);
    expect(when('window.perspective.3')).toBe(false);
    expect(when('window.popOut')).toBe(false);
    // The inspector starts auto-hidden.
    expect(when('window.pinAll')).toBe(true);
    await run('window.pinAll');
    expect(when('window.pinAll')).toBe(false);
    run('window.layout.save');
    run('window.layout.manage');
    expect(open).toHaveBeenCalledTimes(2);
    expect(open.mock.calls.map((c) => c[0])).toEqual([SaveLayoutDialog, ManageLayoutsDialog]);
    perspectives.saveCurrent('A');
    perspectives.switchTo('investigate');
    expect(when('window.perspective.3')).toBe(true);
    expect(find('window.perspective.3').title).toBe('Perspective: A');
    run('window.perspective.3');
    expect(perspectives.active().id).toBe('saved:A');
    run('window.perspective.2');
    expect(perspectives.active().id).toBe('diagnostics');
    await run('window.autoHideAll');
    expect(when('window.pinAll')).toBe(true);
    await run('window.pinAll');
    await run('window.autoHide');
    const registry = TestBed.inject(CommandRegistry);
    const lookup = vi.spyOn(registry, 'find');
    lookup.mockReturnValue({ id: 'window.autoHide', title: 'x', keybinding: 'Ctrl+Alt+J', run: () => undefined });
    expect(dock.autoHideShortcut()).toMatch(/^(Ctrl\+Alt\+J|⌃⌥J)$/);
    lookup.mockReturnValue({ id: 'window.autoHide', title: 'x', run: () => undefined });
    expect(dock.autoHideShortcut()).toBe('');
    vi.restoreAllMocks();
    await run('window.panelMenu');
    await run('window.switcher');
    expect(dock.switcher()).not.toBeNull();
    dock.cancelSwitch();
    await run('window.fullScreen');
    expect(dock.fullScreen()).toBe(true);
    await run('window.layout.reset');
    expect(dock.fullScreen()).toBe(false);
    expect(perspectives.active().id).toBe('diagnostics');
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
    const layer = root.querySelector('.debrief-drop-guides')!;
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
    expect(root.querySelectorAll('.debrief-guide-edges .debrief-guide')).toHaveLength(4);
    expect(root.querySelectorAll('.debrief-guide-compass .debrief-guide')).toHaveLength(5);
    expect(root.querySelector('.debrief-guide.hot')!.getAttribute('data-position')).toBe('center');
    document.elementFromPoint = () => null;
    pointer('pointermove', window, 26, 300);
    expect(root.querySelectorAll('.debrief-guide-compass .debrief-guide')).toHaveLength(0);
    expect(root.querySelector('.debrief-guide-edges .debrief-guide.hot')!.getAttribute('data-position')).toBe('left');
    pointer('pointerup', window, 26, 300);
    expect(layer.classList.contains('on')).toBe(false);
    // Driven from outside (a pop-out over the window), with a preview of the drop.
    guides.showAt({ x: 26, y: 300 }, { x: 0, y: 0, width: 200, height: 600 });
    expect(layer.classList.contains('on')).toBe(true);
    const preview = root.querySelector<HTMLElement>('.debrief-guide-preview')!;
    expect([preview.hidden, preview.style.width]).toEqual([false, '200px']);
    guides.showAt({ x: 400, y: 30 });
    expect(preview.hidden).toBe(true);
    guides.hide();
    guides.hide();
    pointer('pointercancel', window, 0, 0);
    guides.dispose();
    expect(root.querySelector('.debrief-drop-guides')).toBeNull();
    document.elementFromPoint = original;
    root.remove();
  });

  it('shows no guides while a floating window is moved by its tab strip; Shift+drag (redock) shows them', () => {
    const root = document.createElement('div');
    root.innerHTML = '<div class="dv-resize-container"><div class="dv-void-container"></div></div>';
    document.body.appendChild(root);
    const guides = new DropGuides(root);
    const layer = root.querySelector('.debrief-drop-guides')!;
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
