import { TestBed } from '@angular/core/testing';
import { provideDockTesting, settle } from '../../testing/helpers';
import { provideToolsBackend } from '../core/tools-backend';
import { DockService } from '../dock/dock.service';
import { LAYOUT_STORE, LayoutStore } from '../dock/layout-store';
import { PANEL_WINDOWS } from '../dock/panel-windows';
import { CommandRegistry } from '../extensions/command-registry';
import { PopoutBackend } from './popout-backend';
import { POPOUT_ROUTE } from './popout-route';
import { BOUNDS_KEY, MAX_POPOUTS, PopoutService } from './popout.service';
import { FakeBackend } from './popout-testing';
import { loadDockview } from '../dock/dockview-loader';

// dockview is loaded on demand (see dockview-loader); load it once so attach() is synchronous here.
beforeAll(() => loadDockview());

describe('PopoutService (host side)', () => {
  let backend: FakeBackend;
  let container: HTMLElement;

  async function setup(route: unknown = null) {
    localStorage.clear();
    backend = new FakeBackend();
    const store: LayoutStore = { load: () => null, save: () => undefined, clear: () => undefined };
    TestBed.configureTestingModule({
      providers: [
        provideDockTesting(),
        ...provideToolsBackend(),
        { provide: LAYOUT_STORE, useValue: store },
        { provide: PopoutBackend, useValue: backend },
        { provide: PANEL_WINDOWS, useExisting: PopoutService },
        { provide: POPOUT_ROUTE, useValue: route },
      ],
    });
    container = document.createElement('div');
    document.body.appendChild(container);
    const dock = TestBed.inject(DockService);
    dock.attach(container);
    const popouts = TestBed.inject(PopoutService);
    await popouts.startHost();
    return { dock, popouts };
  }

  afterEach(() => {
    TestBed.inject(DockService).detach();
    container.remove();
  });

  const docked = (dock: DockService, id: string) => !!dock['api']!.getPanel(id);

  it('pops a panel out next to the host window and docks it back where it was when the window closes', async () => {
    const { dock, popouts } = await setup();
    expect(await popouts.popOut('inspector')).toBe(true);
    expect(backend.opened[0]).toMatchObject({ viewId: 'inspector', title: 'Inspector', bounds: { x: 1208, y: 40 } });
    expect(docked(dock, 'inspector')).toBe(false);
    expect(dock.isOpen('inspector')).toBe(true);
    expect(popouts.poppedIds().has('inspector')).toBe(true);
    // Again: the existing window comes to the front.
    expect(await popouts.popOut('inspector')).toBe(true);
    expect(popouts.records()).toEqual([expect.objectContaining({ id: 'inspector', slot: expect.any(Object) })]);
    await popouts.dockBack('inspector');
    expect(docked(dock, 'inspector')).toBe(true);
    expect(popouts.poppedIds().size).toBe(0);
    // Bounds are remembered per panel.
    expect(JSON.parse(localStorage.getItem(BOUNDS_KEY)!)['inspector'].bounds.x).toBe(1208);
  });

  it('keeps the panel docked and shows an inline error with Retry when the window cannot be created', async () => {
    const { dock, popouts } = await setup();
    backend.fail = new Error('no more windows');
    expect(await popouts.popOut('metrics')).toBe(false);
    expect(docked(dock, 'metrics')).toBe(true);
    expect(dock.notice()!.message).toContain('Could not pop out Metrics: no more windows');
    backend.fail = null;
    dock.notice()!.retry!();
    await settle();
    expect(popouts.poppedIds().has('metrics')).toBe(true);
    expect(dock.notice()).toBeNull();
  });

  it('allows at most twelve pop-outs per window', async () => {
    const { dock, popouts } = await setup();
    for (let i = 0; i < MAX_POPOUTS; i++) popouts['popped'].update((m) => new Map(m).set(`v${i}`, { label: `p-main-${i + 50}`, viewId: `v${i}`, slot: { referencePanel: null, direction: 'within' }, bounds: null, monitor: null }));
    expect(popouts.canPopOut()).toBe(false);
    expect(await popouts.popOut('metrics')).toBe(false);
    expect(dock.notice()!.message).toContain('At most 12');
    expect(docked(dock, 'metrics')).toBe(true);
  });

  it('discarding closes the window without docking; a window already gone docks immediately', async () => {
    const { dock, popouts } = await setup();
    await popouts.popOut('metrics');
    await popouts.discard('metrics');
    expect(dock.isOpen('metrics')).toBe(false);
    await popouts.popOut('events');
    backend['infos'].clear();
    await popouts.dockBack('events');
    expect(docked(dock, 'events')).toBe(true);
    await popouts.dockBack('nothing');
    await popouts.discard('nothing');
    // A close event for a label that is not ours is ignored.
    backend.closed!({ label: 'p-main-99', viewId: 'timeline' });
    expect(docked(dock, 'timeline')).toBe(true);
  });

  it('restores saved pop-outs on a visible monitor (a lost monitor falls back to the primary display)', async () => {
    const { dock, popouts } = await setup();
    dock.takeOut('inspector');
    await popouts.restore([
      { id: 'inspector', bounds: { x: 4000, y: 100, width: 400, height: 500 }, monitor: 'Gone', slot: { referencePanel: 'events', direction: 'right' } },
      { id: 'no-such-view', bounds: null, monitor: null, slot: { referencePanel: null, direction: 'within' } },
    ]);
    expect(backend.opened).toHaveLength(1);
    expect(backend.opened[0].bounds).toEqual({ x: 760, y: 290, width: 400, height: 500 });
    // A failed restore docks the panel at its slot instead.
    await popouts.dockBack('inspector');
    dock.takeOut('inspector');
    backend.fail = new Error('denied');
    await popouts.restore([{ id: 'inspector', bounds: null, monitor: null, slot: { referencePanel: 'events', direction: 'right' } }]);
    expect(docked(dock, 'inspector')).toBe(true);
  });

  it('opens a dragged-out panel under the pointer and reuses remembered bounds otherwise', async () => {
    const { popouts } = await setup();
    await popouts.popOut('metrics', { at: { x: 900, y: 500 } });
    expect(backend.opened[0].bounds).toMatchObject({ y: 488 });
    backend.bounds['metrics'] = { x: 10, y: 20, width: 600, height: 400 };
    backend.changed!();
    await settle();
    expect(popouts.records()[0].bounds).toEqual({ x: 10, y: 20, width: 600, height: 400 });
    await popouts.dockBack('metrics');
    await popouts.popOut('metrics');
    expect(backend.opened.at(-1)!.bounds).toEqual({ x: 10, y: 20, width: 600, height: 400 });
  });

  it('adopts pop-outs the shell already has (the host UI reloaded)', async () => {
    localStorage.clear();
    backend = new FakeBackend();
    backend.existing = [{ label: 'p-main-7', host: 'main', viewId: 'inspector', title: 'Inspector', bounds: null, monitor: null, alwaysOnTop: false }];
    TestBed.configureTestingModule({
      providers: [provideDockTesting(), ...provideToolsBackend(), { provide: LAYOUT_STORE, useValue: { load: () => null, save: () => undefined, clear: () => undefined } }, { provide: PopoutBackend, useValue: backend }, { provide: PANEL_WINDOWS, useExisting: PopoutService }],
    });
    container = document.createElement('div');
    document.body.appendChild(container);
    const dock = TestBed.inject(DockService);
    dock.attach(container);
    await TestBed.inject(PopoutService).startHost();
    expect(docked(dock, 'inspector')).toBe(false);
    expect(dock.isOpen('inspector')).toBe(true);
  });

  it('runs allowed commands from pop-outs: showing views and global undo', async () => {
    const { dock, popouts } = await setup();
    const run = vi.spyOn(TestBed.inject(CommandRegistry), 'run').mockResolvedValue(true);
    dock.close('metrics');
    popouts['hostCommand']('showView', 'metrics');
    popouts['hostCommand']('showView', 'nope');
    popouts['hostCommand']('run', 'edit.undo');
    popouts['hostCommand']('run', 'file.close');
    expect(dock.isOpen('metrics')).toBe(true);
    expect(run.mock.calls).toEqual([['edit.undo']]);
  });

  it('docks a pop-out resting on a guide where the release cannot be observed (OS move events, settle rule)', async () => {
    vi.useFakeTimers({ toFake: ['setInterval', 'clearInterval', 'Date'] });
    try {
      const { dock, popouts } = await setup();
      await popouts.popOut('inspector');
      const target = { kind: 'edge' as const, position: 'left' as const };
      const at = vi.spyOn(dock, 'dropTargetAt').mockReturnValue(target);
      const guides = vi.spyOn(dock, 'showDropGuides');
      backend.origin = { x: 100, y: 50 };
      backend.moved!({ label: 'p-main-1', viewId: 'inspector', x: 140, y: 90 });
      await settle();
      expect(at).toHaveBeenCalledWith({ x: 40, y: 40 });
      expect(guides).toHaveBeenLastCalledWith({ x: 40, y: 40 });
      // Not ours, or outside the window: ignored / guides hidden.
      backend.moved!({ label: 'p-main-9', viewId: 'x', x: 1, y: 1 });
      vi.advanceTimersByTime(400);
      await settle();
      expect(dock['api']!.getPanel('inspector')!.group.api.location.type).toBe('grid');
      expect(popouts.poppedIds().size).toBe(0);
      expect(backend.docked).toEqual(['p-main-1']);
      expect(guides).toHaveBeenLastCalledWith(null);
    } finally {
      vi.useRealTimers();
    }
  });

  it('docks an OS-moved pop-out only when the button is released, not while it hovers a guide', async () => {
    vi.useFakeTimers({ toFake: ['setInterval', 'clearInterval', 'Date'] });
    try {
      const { dock, popouts } = await setup();
      await popouts.popOut('inspector');
      vi.spyOn(dock, 'dropTargetAt').mockReturnValue({ kind: 'edge', position: 'left' });
      backend.buttonDown = true;
      backend.moved!({ label: 'p-main-1', viewId: 'inspector', x: 40, y: 40 });
      await settle();
      // Resting on the guide with the button held, far longer than the settle time and the idle limit.
      for (let i = 0; i < 50; i++) {
        vi.advanceTimersByTime(100);
        await settle();
      }
      expect(popouts.poppedIds().has('inspector')).toBe(true);
      expect(backend.docked).toEqual([]);
      backend.buttonDown = false;
      vi.advanceTimersByTime(100);
      await settle();
      expect(popouts.poppedIds().size).toBe(0);
      expect(backend.docked).toEqual(['p-main-1']);
    } finally {
      vi.useRealTimers();
    }
  });

  it('ignores a pop-out moved without the button held (snapping, keyboard, restore)', async () => {
    const { dock, popouts } = await setup();
    await popouts.popOut('inspector');
    const guides = vi.spyOn(dock, 'showDropGuides');
    backend.buttonDown = false;
    backend.moved!({ label: 'p-main-1', viewId: 'inspector', x: 40, y: 40 });
    await settle();
    expect(guides).not.toHaveBeenCalled();
    expect(popouts.poppedIds().has('inspector')).toBe(true);
  });

  it('docks on the pop-out shell drag handle release, and gives up a drag released elsewhere', async () => {
    const { dock, popouts } = await setup();
    await popouts.popOut('inspector');
    const target = { kind: 'edge' as const, position: 'right' as const };
    const at = vi.spyOn(dock, 'dropTargetAt').mockReturnValue(null);
    popouts['hostCommand']('dragOver', JSON.stringify({ label: 'p-main-1', x: 10, y: 10 }));
    await settle();
    popouts['hostCommand']('dragEnd', 'p-main-1');
    expect(popouts.poppedIds().has('inspector')).toBe(true);
    popouts['hostCommand']('dragOver', 'not json');
    popouts['hostCommand']('dragOver', JSON.stringify({ label: 'p-main-1', x: 'a' }));
    popouts['hostCommand']('dragOver', JSON.stringify({ label: 'p-main-1', x: 5000, y: 5000 }));
    await settle();
    at.mockReturnValue(target);
    popouts['hostCommand']('dragOver', JSON.stringify({ label: 'p-main-1', x: 20, y: 20 }));
    await settle();
    popouts['hostCommand']('dragEnd', 'p-main-1');
    await settle();
    expect(popouts.poppedIds().size).toBe(0);
    expect(dock['api']!.getPanel('inspector')).toBeDefined();
  });

  it('does nothing in a pop-out window', async () => {
    const { popouts } = await setup({ view: 'inspector', host: 'main', label: 'p-main-1' });
    expect(popouts.canPopOut()).toBe(false);
    expect(await popouts.popOut('metrics')).toBe(false);
    expect(backend.opened).toHaveLength(0);
  });
});
