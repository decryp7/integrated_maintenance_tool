import { TestBed } from '@angular/core/testing';
import { at } from '../../testing/must';
import { FakeDock, provideFakeDock, provideTinyBackend, settle } from '../../testing/helpers';
import { RecordView } from '../core/api.types';
import { WorkspaceStore } from '../core/workspace.store';
import { CommandRegistry } from '../extensions/command-registry';
import { UiStateRegistry } from '../lifecycle/ui-state';
import { NavigationHistory } from './navigation.service';
import { listenMouseNavigation, NavigationCommands, navigationResetContribution, provideNavigation } from './provide-navigation';

const row = (id: number): RecordView => ({ id, ts: id, tsState: 'parsed', level: 'Information', message: 'm', template: null, exception: null, fields: {}, source: 0, sourceName: 's', line: id }) as RecordView;

function setup(withProvider = false) {
  const dock = new FakeDock();
  TestBed.configureTestingModule({ providers: [provideTinyBackend(), ...provideFakeDock(dock), ...(withProvider ? [provideNavigation()] : [])] });
  const store = TestBed.inject(WorkspaceStore);
  const nav = TestBed.inject(NavigationHistory);
  let t = 0;
  nav.now = () => (t += 10_000);
  return { dock, store, nav };
}

describe('NavigationHistory', () => {
  it('records selected events, zoomed ranges and views brought forward', async () => {
    const { dock, store, nav } = setup();
    nav.start();
    nav.start();
    await settle();
    // The starting range is the first place.
    expect(nav.canBack()).toBe(false);
    store.select(row(5));
    await settle();
    store.setRange(10, 20);
    await settle();
    dock.activeViewId.set('events');
    await settle();
    const jump = vi.spyOn(store, 'jumpTo');
    const patch = vi.spyOn(store, 'patchQuery');
    expect(nav.back()).toBe(true);
    expect(dock.calls).toEqual([]);
    expect(patch).toHaveBeenCalledWith({ from: 10, to: 20 });
    expect(nav.back()).toBe(true);
    expect(jump).toHaveBeenCalledWith(5);
    expect(nav.canForward()).toBe(true);
    expect(nav.forward()).toBe(true);
    expect(nav.forward()).toBe(false);
    // The events view is in front already: it is no stop of its own.
    expect(dock.calls).toEqual([]);
    expect(nav.canForward()).toBe(false);
  });

  it('shows a view that is not in front when going forward to it', async () => {
    const { dock, nav } = setup();
    nav.start();
    await settle();
    dock.activeViewId.set('events');
    await settle();
    dock.activeViewId.set('timeline');
    await settle();
    expect(nav.back()).toBe(true);
    expect(dock.calls).toEqual(['show:events']);
  });

  it('Back passes the view that is already in front (a view place between two events)', async () => {
    const { store, dock, nav } = setup();
    nav.start();
    await settle();
    const jump = vi.spyOn(store, 'jumpTo');
    store.select(row(1));
    await settle();
    dock.activeViewId.set('events');
    await settle();
    store.select(row(5));
    await settle();
    store.select(row(9));
    await settle();
    expect(nav.back()).toBe(true);
    expect(nav.back()).toBe(true);
    expect(jump.mock.calls).toEqual([[5], [1]]);
    expect(dock.calls).toEqual([]);
  });

  it('re-applying a place does not record it again', async () => {
    const { store, nav } = setup();
    nav.start();
    await settle();
    store.setRange(1, 2);
    await settle();
    store.setRange(3, 4);
    await settle();
    expect(nav.back()).toBe(true);
    await settle();
    expect(store.query().from).toBe(1);
    // The watcher saw the restored range as the current place: forward is still possible.
    expect(nav.canForward()).toBe(true);
    expect(nav.forward()).toBe(true);
    await settle();
    expect(store.query().from).toBe(3);
    expect(nav.canForward()).toBe(false);
  });

  it('ignores a sliding live window', async () => {
    const { store, nav } = setup();
    nav.start();
    await settle();
    const before = nav.canBack();
    store.patchQuery({ window: { kind: 'live', lastMs: 1000 }, from: 5, to: 9 });
    await settle();
    store.patchQuery({ from: 6, to: 10 });
    await settle();
    expect(nav.canBack()).toBe(before);
    expect(nav.back()).toBe(false);
  });

  it('back and forward do nothing at the ends', () => {
    const { nav } = setup();
    expect(nav.back()).toBe(false);
    expect(nav.forward()).toBe(false);
    expect(nav.canBack()).toBe(false);
  });

  it('clear forgets every place', () => {
    const { nav } = setup();
    nav.observe({ kind: 'event', id: 1 });
    nav.observe({ kind: 'event', id: 2 });
    expect(nav.canBack()).toBe(true);
    nav.clear();
    expect(nav.canBack()).toBe(false);
    expect(nav.back()).toBe(false);
  });

  it('observing the current place again changes nothing', () => {
    const { nav } = setup();
    nav.observe({ kind: 'event', id: 1 });
    nav.observe({ kind: 'event', id: 1 });
    expect(nav.canBack()).toBe(false);
  });
});

describe('NavigationCommands', () => {
  it('are available only when there is somewhere to go and run back/forward', async () => {
    const { nav, store } = setup();
    const cmds = TestBed.inject(NavigationCommands).commands;
    const back = at(cmds, 0);
    const forward = at(cmds, 1);
    expect(back.id).toBe('nav.back');
    expect(back.keybinding).toBe('Alt+Left');
    expect(forward.id).toBe('nav.forward');
    expect(forward.keybinding).toBe('Alt+Right');
    expect(back.when?.(undefined as never)).toBe(false);
    expect(forward.when?.(undefined as never)).toBe(false);
    nav.observe({ kind: 'range', from: 1, to: 2 });
    nav.observe({ kind: 'event', id: 7 });
    expect(back.when?.(undefined as never)).toBe(true);
    const jump = vi.spyOn(store, 'jumpTo');
    const patch = vi.spyOn(store, 'patchQuery');
    await back.run(undefined as never);
    expect(patch).toHaveBeenCalledWith({ from: 1, to: 2 });
    expect(forward.when?.(undefined as never)).toBe(true);
    await forward.run(undefined as never);
    expect(jump).toHaveBeenCalledWith(7);
  });
});

describe('listenMouseNavigation', () => {
  it('goes back on button 3 and forward on button 4, ignores other buttons, and stops when removed', () => {
    const { nav } = setup();
    const back = vi.spyOn(nav, 'back');
    const forward = vi.spyOn(nav, 'forward');
    const target = new EventTarget() as unknown as Window;
    const remove = listenMouseNavigation(nav, target);
    const fire = (button: number) => {
      const e = new MouseEvent('mouseup', { button, cancelable: true });
      target.dispatchEvent(e);
      return e;
    };
    expect(fire(3).defaultPrevented).toBe(true);
    expect(back).toHaveBeenCalledTimes(1);
    expect(fire(4).defaultPrevented).toBe(true);
    expect(forward).toHaveBeenCalledTimes(1);
    for (const b of [0, 1, 2]) expect(fire(b).defaultPrevented).toBe(false);
    expect(back).toHaveBeenCalledTimes(1);
    expect(forward).toHaveBeenCalledTimes(1);
    remove();
    fire(3);
    expect(back).toHaveBeenCalledTimes(1);
  });
});

describe('navigationResetContribution', () => {
  it('saves nothing and forgets the places on restore and on reset', () => {
    const { nav } = setup();
    const c = navigationResetContribution(nav);
    expect(c.key).toBe('navigationHistory');
    expect(c.tracksChanges).toBe(false);
    expect(c.capture()).toBeUndefined();
    nav.observe({ kind: 'event', id: 1 });
    nav.observe({ kind: 'event', id: 2 });
    c.restore({});
    expect(nav.canBack()).toBe(false);
    nav.observe({ kind: 'event', id: 1 });
    nav.observe({ kind: 'event', id: 2 });
    c.reset?.();
    expect(nav.canBack()).toBe(false);
  });
});

describe('provideNavigation', () => {
  it('registers the commands, the ui-state contribution and starts recording', async () => {
    const add = vi.spyOn(window, 'addEventListener');
    const { dock, nav } = setup(true);
    const registry = TestBed.inject(CommandRegistry);
    expect(registry.find('nav.back')).toBeTruthy();
    expect(registry.find('nav.forward')).toBeTruthy();
    expect(TestBed.inject(UiStateRegistry).contributions().some((c) => c.key === 'navigationHistory')).toBe(true);
    expect(add).toHaveBeenCalledWith('mouseup', expect.any(Function));
    dock.activeViewId.set('events');
    await settle();
    dock.activeViewId.set('inspector');
    await settle();
    expect(nav.canBack()).toBe(true);
    add.mockRestore();
  });
});
