import { signal } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { provideDockTesting, settle } from '../../testing/helpers';
import { ContextMenuService } from '../ui/context-menu.service';
import { DockHost } from './dock-host';
import { DockKeys, matchesByCode } from './dock-keys';
import { provideDocking } from './provide-docking';
import { DockService, PANEL } from './dock.service';
import { FullScreenPrefs, OsWindow } from './os-window';
import { EMPTY_EXTRAS, LAYOUT_STORE, LayoutStore, PoppedOutPanel, SavedLayout } from './layout-store';
import { PANEL_WINDOWS, PanelWindows } from './panel-windows';
import { DockSwitcher } from './switcher-overlay';
import { loadDockview } from './dockview-loader';
import { at } from '../../testing/must';

// dockview is loaded on demand (see dockview-loader); load it once so attach() is synchronous here.
beforeAll(() => loadDockview());

/** A PanelWindows double recording what the dock asks of it. */
function fakeWindows(): PanelWindows & { calls: string[]; ids: ReturnType<typeof signal<ReadonlySet<string>>> } {
  const ids = signal<ReadonlySet<string>>(new Set());
  const calls: string[] = [];
  return {
    calls,
    ids,
    poppedIds: ids,
    canPopOut: () => true,
    popOut: async (id, o) => {
      calls.push(`popOut:${id}${o?.at ? `@${o.at.x},${o.at.y}` : ''}${o?.rect ? `#${o.rect.x},${o.rect.y},${o.rect.width}x${o.rect.height}` : ''}`);
      return true;
    },
    focus: (id) => void calls.push(`focus:${id}`),
    dockBack: async (id) => void calls.push(`dockBack:${id}`),
    discard: async (id) => void calls.push(`discard:${id}`),
    records: (): PoppedOutPanel[] => [],
    restore: async (r) => void calls.push(`restore:${r.map((x) => x.id).join(',')}`),
    beginDragOut: () => null,
  };
}

describe('DockService VS-style features', () => {
  let container: HTMLElement;
  let saved: SavedLayout | null;
  let windows: ReturnType<typeof fakeWindows>;

  function setup(stored: SavedLayout | null = null) {
    saved = stored;
    windows = fakeWindows();
    const store: LayoutStore = { load: () => saved, save: (l) => (saved = l), clear: () => (saved = null) };
    TestBed.configureTestingModule({ providers: [provideDockTesting(), { provide: LAYOUT_STORE, useValue: store }, { provide: PANEL_WINDOWS, useValue: windows }] });
    container = document.createElement('div');
    document.body.appendChild(container);
    const dock = TestBed.inject(DockService);
    dock.attach(container);
    // The feature tests start from the fuller arrangement: the inspector docked, metrics beside the timeline.
    if (!stored) {
      dock.pin(PANEL.inspector);
      dock.show(PANEL.metrics, { inactive: true });
    }
    return dock;
  }

  afterEach(() => {
    TestBed.inject(DockService).detach();
    container.remove();
    document.documentElement.classList.remove('debrief-full-screen');
  });

  const api = (d: DockService) => d['api']!;
  const tabOf = (id: string) => container.querySelector<HTMLElement>(`.debrief-tab[data-panel-id="${id}"]`)!.closest<HTMLElement>('[role=tab]')!;
  const persisted = () => new Promise((r) => setTimeout(r, 300));

  it('classifies views: the inspector is a tool window, the timeline a document', () => {
    const dock = setup();
    expect(dock.kindOf('inspector')).toBe('tool');
    expect(dock.kindOf('timeline')).toBe('document');
  });

  it('auto-hides a tool window, slides it out, pins it back where it was, and refuses documents', () => {
    const dock = setup();
    dock.unpin('timeline');
    expect(dock.autoHide().items).toHaveLength(0);
    dock.unpin('inspector');
    expect(api(dock).getPanel('inspector')).toBeUndefined();
    expect(dock.isOpen('inspector')).toBe(true);
    expect(dock.autoHide().items[0]).toMatchObject({ id: 'inspector', title: 'Inspector' });
    // Showing an auto-hidden view slides it out instead of docking it.
    dock.show('inspector');
    expect(dock.autoHide().open).toBe('inspector');
    dock.autoHideEvent({ type: 'resize', id: 'inspector', size: 500 });
    expect(at(dock.autoHide().items, 0).size).toBe(500);
    dock.pin('inspector');
    expect(dock.autoHide().items).toHaveLength(0);
    expect(api(dock).getPanel('inspector')).toBeDefined();
    dock.pin('inspector');
  });

  it('auto hide all / pin all / toggle the active one / close an auto-hidden one', () => {
    const dock = setup();
    expect(dock.autoHideAll()).toEqual(['inspector']);
    expect(dock.autoHide().items.map((i) => i.id)).toEqual(['inspector']);
    // Nothing docked is left to hide.
    expect(dock.autoHideAll()).toEqual([]);
    // pinIds pins only what is still auto-hidden: an unknown or already pinned id is left alone.
    dock.pinIds(['nope', 'inspector']);
    expect(dock.autoHide().items).toHaveLength(0);
    dock.pinIds(['inspector']);
    expect(api(dock).getPanel('inspector')).toBeDefined();
    api(dock).getPanel('inspector')!.api.setActive();
    dock.toggleAutoHideActive();
    expect(dock.autoHide().items).toHaveLength(1);
    dock.show('inspector');
    dock.toggleAutoHideActive();
    expect(dock.autoHide().items).toHaveLength(0);
    dock.unpin('inspector');
    dock.close('inspector');
    expect(dock.isOpen('inspector')).toBe(false);
  });

  it('pins a tab to the left of its strip; "close all documents" and "close others" keep it', () => {
    const dock = setup();
    // The profile editor is not in the default layout: opened as a document in the group beside the timeline.
    dock.show('profile', { inactive: true });
    expect(api(dock).getPanel('profile')!.group).toBe(api(dock).getPanel('metrics')!.group);
    dock.runPanelAction('profile', 'pinTab');
    expect(at(api(dock).getPanel('profile')!.group.panels, 0).id).toBe('profile');
    expect(container.querySelector('.debrief-tab[data-panel-id="profile"]')!.classList.contains('pinned')).toBe(true);
    dock.runPanelAction('metrics', 'closeOthers');
    // Only the tabs of metrics' own group close, and the pinned one stays; the timeline is another group.
    expect(dock.isOpen('profile')).toBe(true);
    expect(dock.isOpen('timeline')).toBe(true);
    dock.closeAllDocuments();
    expect([...dock.openIds()].sort()).toEqual(['inspector', 'profile']);
    dock.runPanelAction('profile', 'unpinTab');
    expect(dock.pinnedTabs().size).toBe(0);
  });

  it('runs the panel menu actions: close to the right, float, dock, new groups, move to next group, dock as document, pop out', () => {
    const dock = setup();
    const groups = () => api(dock).groups.filter((g) => g.api.location.type === 'grid').length;
    dock.show('profile', { inactive: true });
    dock.runPanelAction('metrics', 'closeRight');
    expect(dock.isOpen('profile')).toBe(false);
    expect(dock.isOpen('metrics')).toBe(true);
    dock.runPanelAction('timeline', 'closeRight');
    expect(dock.isOpen('timeline')).toBe(true);
    dock.close('metrics');
    dock.show('metrics');
    dock.runPanelAction('metrics', 'float');
    expect(api(dock).getPanel('metrics')!.group.api.location.type).toBe('floating');
    dock.runPanelAction('metrics', 'dock');
    expect(api(dock).getPanel('metrics')!.group.api.location.type).toBe('grid');
    // A group needs a second tab for the panel to be split off it.
    dock.show('profile', { inactive: true });
    const before = groups();
    dock.runPanelAction('metrics', 'newVerticalGroup');
    expect(groups()).toBe(before + 1);
    dock.runPanelAction('metrics', 'moveToNextGroup');
    dock.show('profile');
    // Splitting a panel off needs another tab in its group.
    api(dock).getPanel('metrics')!.api.moveTo({ group: api(dock).getPanel('profile')!.group, position: 'center' });
    dock.runPanelAction('profile', 'newHorizontalGroup');
    dock.runPanelAction('inspector', 'dockAsDocument');
    expect(api(dock).getPanel('inspector')!.group.panels.some((p) => dock.kindOf(p.id) === 'document')).toBe(true);
    dock.runPanelAction('events', 'popout');
    dock.runPanelAction('events', 'autoHide');
    dock.runPanelAction('events', 'close');
    expect(dock.isOpen('events')).toBe(false);
    dock.runPanelAction('missing', 'close');
    expect(windows.calls).toContain('popOut:events');
  });

  it('opens the panel menu from a right click, Shift+F10 and the Menu key, with VS items', () => {
    const dock = setup();
    const menus = TestBed.inject(ContextMenuService);
    tabOf('inspector').dispatchEvent(new MouseEvent('contextmenu', { bubbles: true, cancelable: true, clientX: 10, clientY: 20 }));
    expect(menus.menu()!.items.map((i) => i.label)).toEqual(['Float', 'Pop out to window', 'Dock', 'Dock as tabbed document', 'Auto hide', 'Pin tab', 'Close']);
    expect(menus.menu()).toMatchObject({ x: 10, y: 20 });
    menus.menu()!.items.find((i) => i.label === 'Auto hide')!.run();
    expect(dock.autoHide().items).toHaveLength(1);
    menus.close();
    tabOf('timeline').dispatchEvent(new KeyboardEvent('keydown', { key: 'F10', shiftKey: true, bubbles: true }));
    expect(menus.menu()!.items.map((i) => i.label)).toContain('Close all but this');
    menus.close();
    tabOf('events').dispatchEvent(new KeyboardEvent('keydown', { key: 'ContextMenu', bubbles: true }));
    expect(menus.menu()).not.toBeNull();
    menus.close();
    api(dock).getPanel('events')!.api.setActive();
    dock.openActivePanelMenu();
    expect(menus.menu()).not.toBeNull();
    container.dispatchEvent(new MouseEvent('contextmenu', { bubbles: true }));
  });

  it('pops out from the group panel menu and from a tab dragged out of the window (pointer events)', async () => {
    const dock = setup();
    const menus = TestBed.inject(ContextMenuService);
    container.querySelector<HTMLButtonElement>('.dv-groupview button[aria-label="Panel menu"]')!.click();
    menus.menu()!.items.find((i) => i.label === 'Pop out to window')!.run();
    menus.close();
    expect(windows.calls[0]).toMatch(/^popOut:/);
    const pointer = (type: string, target: EventTarget, init: PointerEventInit) => target.dispatchEvent(new PointerEvent(type, { bubbles: true, button: 0, ...init }));
    pointer('pointerdown', tabOf('metrics'), { clientX: 10, clientY: 10, screenX: 10, screenY: 10 });
    pointer('pointermove', window, { clientX: 40, clientY: 10, screenX: 40, screenY: 10 });
    pointer('pointerup', window, { clientX: -50, clientY: 100, screenX: 30, screenY: 60 });
    expect(windows.calls).toContain('popOut:metrics@30,60');
    // Released inside: no pop-out.
    pointer('pointerdown', tabOf('metrics'), { clientX: 10, clientY: 10 });
    pointer('pointermove', window, { clientX: 60, clientY: 10 });
    pointer('pointerup', window, { clientX: 60, clientY: 10 });
    expect(windows.calls.filter((c) => c.startsWith('popOut:metrics'))).toHaveLength(1);
    api(dock).getPanel('events')!.api.setActive();
    await dock.popOutActive();
    expect(windows.calls).toContain('popOut:events');
    expect(await dock.popOut('')).toBe(false);
  });

  it('cancels a tab released inside the window off the drop zones, with a cue for cancelling and for a new window', async () => {
    const dock = setup();
    const root = { left: 0, top: 0, width: 800, height: 600, right: 800, bottom: 600, x: 0, y: 0, toJSON: () => ({}) } as DOMRect;
    container.getBoundingClientRect = () => root;
    // jsdom has no hit testing; dockview's own pointer drag asks for it once the drag starts.
    Object.defineProperty(document, 'elementsFromPoint', { value: () => [], configurable: true });
    Object.defineProperty(document, 'elementFromPoint', { value: () => null, configurable: true });
    onTestFinished(() => {
      delete (document as { elementsFromPoint?: unknown }).elementsFromPoint;
      delete (document as { elementFromPoint?: unknown }).elementFromPoint;
    });
    const ghost = () => document.querySelector<HTMLElement>('.debrief-float-cue')!;
    const pointer = (type: string, target: EventTarget, x: number, y: number) =>
      target.dispatchEvent(new PointerEvent(type, { bubbles: true, button: 0, clientX: x, clientY: y, screenX: x, screenY: y }));
    expect(dock.tabDropOutcome('metrics', { x: 400, y: 300 })).toBe('cancel');
    expect(dock.tabDropOutcome('metrics', { x: 26, y: 300 })).toBe('dock');
    expect(dock.tabDropOutcome('metrics', { x: -20, y: 300 })).toBe('window');
    pointer('pointerdown', tabOf('metrics'), 10, 10);
    pointer('pointermove', window, 400, 300);
    expect(ghost().textContent).toBe('Release to cancel');
    expect(ghost().style.left).toBe('414px');
    pointer('pointermove', window, 26, 300);
    expect(ghost().hidden).toBe(true);
    pointer('pointermove', window, -20, 300);
    expect(ghost().textContent).toBe('Release to open in a new window');
    pointer('pointermove', window, 400, 300);
    pointer('pointerup', window, 400, 300);
    expect(ghost().hidden).toBe(true);
    await new Promise((r) => setTimeout(r));
    // Nothing floated and nothing popped out: the tab stays where it was.
    expect(api(dock).getPanel('metrics')!.group.api.location.type).toBe('grid');
    expect(windows.calls.some((c) => c.startsWith('popOut:metrics'))).toBe(false);
    dock.detach();
    expect(document.querySelector('.debrief-float-cue')).toBeNull();
  });

  it('outside the window the ghost window of the shell replaces the in-window preview, and the pop-out opens where the ghost was', async () => {
    const dock = setup();
    const root = { left: 0, top: 0, width: 800, height: 600, right: 800, bottom: 600, x: 0, y: 0, toJSON: () => ({}) } as DOMRect;
    container.getBoundingClientRect = () => root;
    Object.defineProperty(document, 'elementsFromPoint', { value: () => [], configurable: true });
    Object.defineProperty(document, 'elementFromPoint', { value: () => null, configurable: true });
    onTestFinished(() => {
      delete (document as { elementsFromPoint?: unknown }).elementsFromPoint;
      delete (document as { elementFromPoint?: unknown }).elementFromPoint;
    });
    const shown: boolean[] = [];
    let finished = 0;
    const begun: string[] = [];
    windows.beginDragOut = (id) => {
      begun.push(id);
      return {
        native: true,
        show: (outside) => void shown.push(outside),
        finish: async () => (finished++, { x: 700, y: 50, width: 560, height: 420 }),
      };
    };
    const cue = () => document.querySelector<HTMLElement>('.debrief-float-cue')!;
    const pointer = (type: string, target: EventTarget, x: number, y: number) =>
      target.dispatchEvent(new PointerEvent(type, { bubbles: true, button: 0, clientX: x, clientY: y, screenX: x, screenY: y }));
    pointer('pointerdown', tabOf('metrics'), 10, 10);
    pointer('pointermove', window, 400, 300);
    pointer('pointermove', window, -20, 300);
    expect(begun).toEqual(['metrics']);
    expect(shown).toEqual([false, true]);
    // The ghost window shows the new window; the hint is still announced.
    expect(cue().hidden).toBe(true);
    expect([...document.querySelectorAll('[role="status"]')].map((e) => e.textContent)).toContain('Release to open in a new window');
    pointer('pointerup', window, -20, 300);
    expect(finished).toBe(1);
    await new Promise((r) => setTimeout(r));
    expect(windows.calls).toContain('popOut:metrics#700,50,560x420');
    expect(dock.isOpen('metrics')).toBe(true);
    // Escape outside the window: the ghost ends and nothing pops out.
    windows.calls.length = 0;
    pointer('pointerdown', tabOf('events'), 10, 10);
    pointer('pointermove', window, -20, 300);
    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
    expect(finished).toBe(2);
    pointer('pointerup', window, -20, 300);
    await new Promise((r) => setTimeout(r));
    expect(windows.calls.some((c) => c.startsWith('popOut:'))).toBe(false);
  });

  it('shows guides for and docks a panel dropped from outside dockview (a pop-out over the window)', () => {
    const dock = setup();
    const root = { left: 0, top: 0, width: 800, height: 600, right: 800, bottom: 600, x: 0, y: 0, toJSON: () => ({}) } as DOMRect;
    container.getBoundingClientRect = () => root;
    expect(dock.dropTargetAt({ x: 26, y: 300 })).toEqual({ kind: 'edge', position: 'left' });
    expect(dock.dropTargetAt({ x: 400, y: 300 })).toBeNull();
    expect(dock.dropTargetAt({ x: 900, y: 300 })).toBeNull();
    dock.showDropGuides({ x: 26, y: 300 });
    expect(container.querySelector('.debrief-drop-guides')!.classList.contains('on')).toBe(true);
    expect((container.querySelector('.debrief-guide-preview') as HTMLElement).hidden).toBe(false);
    dock.showDropGuides({ x: 400, y: 300 });
    expect((container.querySelector('.debrief-guide-preview') as HTMLElement).hidden).toBe(true);
    dock.showDropGuides(null);
    expect(container.querySelector('.debrief-drop-guides')!.classList.contains('on')).toBe(false);
    dock.takeOut('inspector');
    expect(dock.dockAt('inspector', { kind: 'edge', position: 'left' })).toBe(true);
    expect(dock.dockAt('inspector', { kind: 'edge', position: 'left' })).toBe(false);
    dock.takeOut('metrics');
    const group = api(dock).getPanel('events')!.group.id;
    expect(dock.dockAt('metrics', { kind: 'group', groupId: group, position: 'center' })).toBe(true);
    expect(api(dock).getPanel('metrics')!.group.id).toBe(group);
    dock.takeOut('profile');
    expect(dock.dockAt('profile', { kind: 'group', groupId: 'gone', position: 'top' })).toBe(false);
    expect(dock.dockAt('profile', { kind: 'edge', position: 'center' })).toBe(true);
  });

  it('treats popped-out panels as open: show focuses the window, close discards it', () => {
    const dock = setup();
    dock.takeOut('metrics');
    windows.ids.set(new Set(['metrics']));
    expect(dock.isOpen('metrics')).toBe(true);
    dock.show('metrics');
    dock.show('metrics', { inactive: true });
    dock.close('metrics');
    expect(windows.calls).toEqual(['focus:metrics', 'discard:metrics']);
  });

  it('captures a slot and puts a panel back there', () => {
    const dock = setup();
    // Metrics is alone in its group beside the timeline: its slot is against that neighbour, not a tab.
    expect(dock.captureSlot('metrics')?.direction).not.toBe(undefined);
    dock.show('profile', { inactive: true });
    const slot = dock.captureSlot('metrics')!;
    expect(slot).toMatchObject({ referencePanel: 'profile', direction: 'within' });
    dock.takeOut('metrics');
    dock.restoreSlot('metrics', slot);
    expect(api(dock).getPanel('metrics')!.group).toBe(api(dock).getPanel('profile')!.group);
    // Alone in its group: placed against a neighbour, or by default when there is none.
    expect(dock.captureSlot('inspector')).not.toBeNull();
    dock.takeOut('inspector');
    dock.restoreSlot('inspector', { referencePanel: 'gone', direction: 'left' });
    expect(dock.isOpen('inspector')).toBe(true);
    dock.takeOut('inspector');
    dock.restoreSlot('inspector', { referencePanel: 'events', direction: 'right', size: 300 });
    expect(api(dock).getPanel('inspector')).toBeDefined();
    expect(dock.captureSlot('missing')).toBeNull();
    expect(dock.panelRect('missing')).toBeNull();
  });

  // Regression (run-spans.e2e:58 under load): grow-back put a view back at its window edge by removing and re-adding its
  // panel, which destroyed the component and its state (an open "New run rule" form closed under the user's click).
  it('moves an existing panel back to its slot without re-creating it, at a window edge too', () => {
    const dock = setup();
    const slot = { referencePanel: null, direction: 'right' as const, edge: 'right' as const, size: 300 };
    expect(dock.captureSlot('inspector')).not.toBeNull();
    const panel = api(dock).getPanel('inspector')!;
    panel.api.moveTo({ group: api(dock).getPanel('events')!.group, position: 'center' });
    expect(dock.moveToSlot('inspector', slot)).toBe(true);
    expect(api(dock).getPanel('inspector')).toBe(panel);
    expect(panel.group).not.toBe(api(dock).getPanel('events')!.group);
    // Against a neighbour that is gone there is nowhere to move it: the caller re-adds it.
    expect(dock.moveToSlot('inspector', { referencePanel: 'gone', direction: 'left' })).toBe(false);
    expect(dock.moveToSlot('missing', slot)).toBe(false);
  });

  it('the Ctrl+Tab navigator lists panels most recently used first and switches on commit', () => {
    const dock = setup();
    api(dock).getPanel('events')!.api.setActive();
    api(dock).getPanel('inspector')!.api.setActive();
    dock.switch();
    expect(dock.switcher()!.items.slice(0, 2).map((i) => i.id)).toEqual(['inspector', 'events']);
    expect(dock.switcher()!.index).toBe(1);
    dock.switch();
    dock.switch(true);
    dock.commitSwitch();
    expect(dock.switcher()).toBeNull();
    expect(api(dock).activePanel!.id).toBe('events');
    dock.switch();
    dock.cancelSwitch();
    expect(dock.switcher()).toBeNull();
  });

  it('full screen hides the chrome and maximizes the document well, and/or goes OS full screen; again restores', () => {
    const dock = setup();
    const os = vi.spyOn(TestBed.inject(OsWindow), 'setFullscreen').mockResolvedValue();
    const prefs = TestBed.inject(FullScreenPrefs);
    prefs.set('os');
    dock.toggleFullScreen();
    expect(document.documentElement.classList.contains('debrief-full-screen')).toBe(false);
    expect(os).toHaveBeenLastCalledWith(true);
    dock.toggleFullScreen();
    expect(os).toHaveBeenLastCalledWith(false);
    prefs.set('chrome');
    dock.toggleFullScreen();
    dock.toggleFullScreen();
    expect(os).toHaveBeenCalledTimes(2);
    prefs.set('both');
    dock.toggleFullScreen();
    expect(os).toHaveBeenCalledTimes(3);
    expect(document.documentElement.classList.contains('debrief-full-screen')).toBe(true);
    expect(api(dock).hasMaximizedGroup()).toBe(true);
    dock.toggleFullScreen();
    expect(document.documentElement.classList.contains('debrief-full-screen')).toBe(false);
    expect(api(dock).hasMaximizedGroup()).toBe(false);
  });

  it('saves the extras with the layout and restores them on attach (pop-outs reopen)', async () => {
    const dock = setup();
    dock.show('profile', { inactive: true });
    dock.unpin('inspector');
    dock.setPinned('profile', true);
    await persisted();
    expect(saved!.extras.autoHidden.map((i) => i.id)).toEqual(['inspector']);
    expect(saved!.extras.pinnedTabs).toEqual(['profile']);
    expect(dock.savedLayout()).toEqual(saved);
    const layout = { ...saved!, extras: { ...saved!.extras, poppedOut: [{ id: 'events', bounds: null, monitor: null, slot: { referencePanel: null, direction: 'within' as const } }] } };
    dock.detach();
    TestBed.resetTestingModule();
    container.remove();
    const again = setup(layout);
    expect(again.autoHide().items.map((i) => i.id)).toEqual(['inspector']);
    expect(again.pinnedTabs().has('profile')).toBe(true);
    expect(windows.calls).toEqual(['restore:events']);
  });

  it('applies a saved layout, closing pop-outs it does not have; a broken one falls back to the default', () => {
    const dock = setup();
    dock.unpin('inspector');
    const layout = dock.currentLayout()!;
    dock.pinAll();
    windows.ids.set(new Set(['metrics']));
    expect(dock.applyLayout(layout)).toBe(true);
    expect(dock.autoHide().items.map((i) => i.id)).toEqual(['inspector']);
    expect(windows.calls).toContain('discard:metrics');
    expect(dock.applyLayout({ layout: { grid: null } as never, extras: EMPTY_EXTRAS })).toBe(false);
    expect(dock.isOpen('timeline')).toBe(true);
  });

  it('asks the host to show views when it runs in a pop-out window', () => {
    const dock = setup();
    const shown: string[] = [];
    dock.setRemote((id) => shown.push(id));
    dock.show('home');
    dock.setRemote(null);
    expect(shown).toEqual(['home']);
  });
});

describe('DockHost auto-hide bars, slide-out and navigator', () => {
  async function host() {
    const store: LayoutStore = { load: () => null, save: () => undefined, clear: () => undefined };
    TestBed.configureTestingModule({ providers: [provideDockTesting(), { provide: LAYOUT_STORE, useValue: store }] });
    const fixture = TestBed.createComponent(DockHost);
    document.body.appendChild(fixture.nativeElement);
    await settle(fixture);
    return { fixture, dock: TestBed.inject(DockService), el: fixture.nativeElement as HTMLElement };
  }

  afterEach(() => document.querySelectorAll('debrief-dock-host, [ng-version]').forEach((e) => e.remove()));

  it('shows a side bar tab for an auto-hidden tool window that slides out on hover and hides on leave and Escape', async () => {
    const { fixture, dock, el } = await host();
    dock.unpin(PANEL.inspector);
    await settle(fixture);
    const tab = el.querySelector<HTMLButtonElement>('.debrief-ah-tab')!;
    expect(tab.textContent!.trim()).toBe('Inspector');
    expect(tab.closest('nav')!.getAttribute('aria-label')).toMatch(/^Auto-hidden tool windows/);
    tab.dispatchEvent(new MouseEvent('mouseenter'));
    await settle(fixture);
    const flyout = el.querySelector<HTMLElement>('.debrief-ah-flyout')!;
    expect(flyout.getAttribute('aria-label')).toBe('Inspector');
    await vi.waitFor(() => expect(flyout.querySelector('.debrief-ah-body')!.children.length).toBe(1));
    tab.dispatchEvent(new MouseEvent('mouseleave'));
    flyout.dispatchEvent(new MouseEvent('mouseenter'));
    await new Promise((r) => setTimeout(r, 350));
    expect(dock.autoHide().open).toBe('inspector');
    flyout.dispatchEvent(new MouseEvent('mouseleave'));
    await new Promise((r) => setTimeout(r, 350));
    expect(dock.autoHide().open).toBeNull();
    // Keyboard: Enter on the tab (a click without pointer details) opens it until Escape.
    tab.dispatchEvent(new MouseEvent('click', { detail: 0, bubbles: true }));
    await settle(fixture);
    expect(dock.autoHide().openedBy).toBe('keyboard');
    el.querySelector('.debrief-ah-flyout')!.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
    await settle(fixture);
    expect(dock.autoHide().open).toBeNull();
  });

  it('hides the slide-out on a click outside or when focus leaves; pins from its header; resizes with its grip', async () => {
    const { fixture, dock, el } = await host();
    dock.unpin(PANEL.inspector);
    await settle(fixture);
    el.querySelector<HTMLButtonElement>('.debrief-ah-tab')!.dispatchEvent(new MouseEvent('click', { detail: 1, bubbles: true }));
    await settle(fixture);
    document.body.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true }));
    expect(dock.autoHide().open).toBeNull();
    dock.autoHideEvent({ type: 'click', id: 'inspector' });
    await settle(fixture);
    const outside = document.createElement('button');
    document.body.appendChild(outside);
    el.querySelector('.debrief-ah-flyout')!.dispatchEvent(new FocusEvent('focusout', { bubbles: true, relatedTarget: outside }));
    expect(dock.autoHide().open).toBeNull();
    outside.remove();
    dock.autoHideEvent({ type: 'click', id: 'inspector' });
    await settle(fixture);
    const grip = el.querySelector<HTMLElement>('.debrief-ah-grip')!;
    const start = at(dock.autoHide().items, 0).size;
    grip.dispatchEvent(new PointerEvent('pointerdown', { clientX: 100, clientY: 0, bubbles: true }));
    document.dispatchEvent(new PointerEvent('pointermove', { clientX: 140, clientY: 0 }));
    document.dispatchEvent(new PointerEvent('pointerup'));
    expect(at(dock.autoHide().items, 0).size).not.toBe(start);
    el.querySelector<HTMLButtonElement>('button[aria-label="Pin Inspector"]')!.click();
    await settle(fixture);
    expect(dock.isOpen('inspector')).toBe(true);
    expect(el.querySelector('.debrief-ah-bar')).toBeNull();
  });

  it('hides a slide-out on Escape anywhere, unless something else handled the key', async () => {
    const { fixture, dock } = await host();
    dock.revealInspector();
    await settle(fixture);
    expect(dock.autoHide().open).toBe('inspector');
    const handled = new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true });
    handled.preventDefault();
    document.body.dispatchEvent(handled);
    expect(dock.autoHide().open).toBe('inspector');
    document.body.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }));
    expect(dock.autoHide().open).toBe('inspector');
    document.body.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
    expect(dock.autoHide().open).toBeNull();
  });

  it('shows an inline notice with Retry and Dismiss', async () => {
    const { fixture, dock, el } = await host();
    const retry = vi.fn();
    dock.notice.set({ message: 'Could not pop out', retry });
    await settle(fixture);
    expect(el.querySelector('.debrief-dock-notice')!.textContent).toContain('Could not pop out');
    [...el.querySelectorAll<HTMLButtonElement>('.debrief-dock-notice button')].find((b) => b.textContent === 'Retry')!.click();
    expect(retry).toHaveBeenCalled();
    dock.notice.set({ message: 'x' });
    await settle(fixture);
    [...el.querySelectorAll<HTMLButtonElement>('.debrief-dock-notice button')].find((b) => b.textContent === 'Dismiss')!.click();
    expect(dock.notice()).toBeNull();
  });

  it('renders the navigator with tool windows and documents in two lists', async () => {
    const { fixture, dock, el } = await host();
    dock.pin(PANEL.inspector);
    dock.switch();
    await settle(fixture);
    const lists = [...el.querySelectorAll('debrief-dock-switcher [role=listbox]')];
    expect(lists).toHaveLength(2);
    expect(at(lists, 0).textContent).toContain('Inspector');
    expect(at(lists, 1).textContent).toContain('Timeline');
    const option = el.querySelector<HTMLElement>('debrief-dock-switcher [role=option]')!;
    option.dispatchEvent(new PointerEvent('pointerenter'));
    option.click();
    expect(dock.switcher()).toBeNull();
    dock.switch();
    await settle(fixture);
    el.querySelector<HTMLElement>('.debrief-switcher-backdrop')!.dispatchEvent(new PointerEvent('pointerdown'));
    expect(dock.switcher()).toBeNull();
    expect(TestBed.createComponent(DockSwitcher)).toBeTruthy();
  });
});

describe('DockKeys', () => {
  function keys() {
    TestBed.configureTestingModule({ providers: [provideDockTesting(), { provide: LAYOUT_STORE, useValue: { load: () => null, save: () => undefined, clear: () => undefined } }] });
    const dock = TestBed.inject(DockService);
    const container = document.createElement('div');
    document.body.appendChild(container);
    dock.attach(container);
    const target = document.createElement('div');
    const off = TestBed.inject(DockKeys).listen(target);
    const press = (type: 'keydown' | 'keyup', init: KeyboardEventInit) => {
      const e = new KeyboardEvent(type, { bubbles: true, cancelable: true, ...init });
      target.dispatchEvent(e);
      return e;
    };
    return { dock, press, off, done: () => (dock.detach(), container.remove()) };
  }

  it('drives the navigator: Ctrl+Tab opens and cycles, arrows move, Enter/Escape, releasing Ctrl commits', () => {
    const { dock, press, off, done } = keys();
    expect(press('keydown', { key: 'Tab', ctrlKey: true }).defaultPrevented).toBe(true);
    expect(dock.switcher()).not.toBeNull();
    const first = dock.switcher()!.index;
    press('keydown', { key: 'Tab', ctrlKey: true });
    expect(dock.switcher()!.index).not.toBe(first);
    press('keydown', { key: 'ArrowUp' });
    press('keydown', { key: 'ArrowDown' });
    expect(press('keydown', { key: 'x' }).defaultPrevented).toBe(false);
    press('keyup', { key: 'Control' });
    expect(dock.switcher()).toBeNull();
    press('keydown', { key: 'Tab', ctrlKey: true, shiftKey: true });
    press('keydown', { key: 'Escape' });
    expect(dock.switcher()).toBeNull();
    press('keydown', { key: 'Tab', ctrlKey: true });
    press('keydown', { key: 'Enter' });
    expect(dock.switcher()).toBeNull();
    press('keyup', { key: 'Control' });
    off();
    press('keydown', { key: 'Tab', ctrlKey: true });
    expect(dock.switcher()).toBeNull();
    done();
  });

  it('runs window commands bound with Alt by physical key when the typed character differs', () => {
    TestBed.configureTestingModule({ providers: [provideDocking()] });
    const { dock, press, done } = keys();
    const full = press('keydown', { key: 'Í', code: 'Enter', shiftKey: true, altKey: true });
    expect(full.defaultPrevented).toBe(true);
    expect(dock.fullScreen()).toBe(true);
    // The normal match is left to the command registry (no double run).
    expect(press('keydown', { key: 'Enter', code: 'Enter', shiftKey: true, altKey: true }).defaultPrevented).toBe(false);
    expect(press('keydown', { key: 'q', code: 'KeyQ', altKey: true }).defaultPrevented).toBe(false);
    dock.toggleFullScreen();
    done();
  });

  it('matches Alt bindings by physical key (macOS Option changes the typed character)', () => {
    const e = (init: KeyboardEventInit) => new KeyboardEvent('keydown', init);
    expect(matchesByCode('Ctrl+Alt+1', e({ key: '¡', code: 'Digit1', ctrlKey: true, altKey: true }), true)).toBe(true);
    expect(matchesByCode('Shift+Alt+Enter', e({ key: 'Enter', code: 'Enter', shiftKey: true, altKey: true }), true)).toBe(true);
    expect(matchesByCode('Alt+-', e({ key: '–', code: 'Minus', altKey: true }), true)).toBe(true);
    expect(matchesByCode('Ctrl+Alt+H', e({ key: '˙', code: 'KeyH', ctrlKey: true, altKey: true }), true)).toBe(true);
    expect(matchesByCode('Mod+Alt+O', e({ key: 'ø', code: 'KeyO', metaKey: true, altKey: true }), true)).toBe(true);
    expect(matchesByCode('Mod+Alt+O', e({ key: 'ø', code: 'KeyO', ctrlKey: true, altKey: true }), false)).toBe(true);
    expect(matchesByCode('Ctrl+Alt+1', e({ code: 'Digit2', ctrlKey: true, altKey: true }), true)).toBe(false);
    expect(matchesByCode('Ctrl+1', e({ code: 'Digit1', ctrlKey: true }), true)).toBe(false);
    expect(matchesByCode('Ctrl+Alt+F5', e({ code: 'F5', ctrlKey: true, altKey: true }), true)).toBe(false);
    expect(matchesByCode('Ctrl+Alt+1', e({ code: 'Digit1', altKey: true }), true)).toBe(false);
  });
});

describe('auto-hide button and sides', () => {
  function dockWithInspector() {
    TestBed.configureTestingModule({ providers: [provideDockTesting()] });
    const container = document.createElement('div');
    document.body.appendChild(container);
    const dock = TestBed.inject(DockService);
    dock.attach(container);
    dock.pin(PANEL.inspector);
    return { dock, container };
  }

  it('applies to docked tool windows only, not to documents', () => {
    const { dock } = dockWithInspector();
    expect(dock.canAutoHide('inspector')).toBe(true);
    expect(dock.canAutoHide('timeline')).toBe(false);
    expect(dock.canAutoHide('nope')).toBe(false);
  });

  it('auto-hides to the chosen side and remembers it for the next time', () => {
    const { dock } = dockWithInspector();
    dock.unpin('inspector', 'bottom');
    expect(at(dock.autoHide().items, 0).side).toBe('bottom');
    dock.pin('inspector');
    dock.unpin('inspector');
    expect(at(dock.autoHide().items, 0).side).toBe('bottom');
    dock.pin('inspector');
    dock.unpin('inspector', 'left');
    expect(at(dock.autoHide().items, 0).side).toBe('left');
  });

  it('offers left, right and bottom, never the top', () => {
    const { dock } = dockWithInspector();
    const open = vi.spyOn(TestBed.inject(ContextMenuService), 'open');
    dock.openAutoHideSideMenu('inspector', null);
    const items = open.mock.calls[0]![2];
    expect(items.map((i) => i.label)).toEqual(['Auto-hide to the left', 'Auto-hide to the right', 'Auto-hide to the bottom']);
    items[2]!.run();
    expect(at(dock.autoHide().items, 0).side).toBe('bottom');
  });

  it('shows the header button only for a tool window, named after it, and chooses the side with the context menu', () => {
    const { dock, container } = dockWithInspector();
    const buttons = [...container.querySelectorAll<HTMLButtonElement>('.debrief-dock-autohide')];
    const b = buttons.find((x) => !x.hidden && x.getAttribute('aria-label') === 'Auto-hide Inspector')!;
    expect(b).toBeTruthy();
    expect(b.getAttribute('aria-pressed')).toBe('false');
    // The tooltip names the user's current binding, read when the pointer arrives (it may have been changed).
    const shortcut = vi.spyOn(dock, 'autoHideShortcut').mockReturnValue('⌃⌥H');
    b.dispatchEvent(new MouseEvent('mouseenter'));
    expect(b.title).toContain('(⌃⌥H)');
    shortcut.mockReturnValue('');
    b.dispatchEvent(new MouseEvent('mouseenter'));
    expect(b.title).not.toContain('(');
    expect(b.title).toContain('Right-click');
    // Groups of documents carry the button hidden, not disabled.
    expect(buttons.some((x) => x.hidden && !x.disabled)).toBe(true);
    const open = vi.spyOn(TestBed.inject(ContextMenuService), 'open');
    b.dispatchEvent(new MouseEvent('contextmenu', { bubbles: true, cancelable: true }));
    expect(open).toHaveBeenCalled();
    b.click();
    expect(dock.autoHide().items.map((i) => i.id)).toEqual(['inspector']);
  });
});
