import { Injector, signal } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { provideDockTesting, settle } from '../../testing/helpers';
import { provideToolsBackend } from '../core/tools-backend';
import { WorkspaceStore } from '../core/workspace.store';
import { CommandRegistry } from '../extensions/command-registry';
import { DockService } from '../dock/dock.service';
import { LAYOUT_STORE } from '../dock/layout-store';
import { SessionService } from '../lifecycle/session.service';
import { OpenRequests } from '../projects/open-requests';
import { PopoutBackend } from './popout-backend';
import { POPOUT_ROUTE } from './popout-route';
import { pinDocumentTitle, PopoutShell } from './popout-shell';
import { FakeBackend } from './popout-testing';
import { INERT_OPEN_REQUESTS, INERT_SESSION, providePopout } from './provide-popout';
import { PANEL_WINDOWS } from '../dock/panel-windows';
import { PopoutService } from './popout.service';
import { UiSync } from './ui-sync';
import { provideUndo } from '../undo/provide-undo';

const ROUTE = { view: 'inspector', host: 'main', label: 'p-main-1' };

describe('PopoutShell', () => {
  let backend: FakeBackend;

  async function shell(setup: (b: FakeBackend) => void = () => undefined) {
    backend = new FakeBackend();
    setup(backend);
    TestBed.configureTestingModule({
      providers: [
        provideDockTesting(),
        ...provideToolsBackend(),
        provideUndo(),
        { provide: LAYOUT_STORE, useValue: { load: () => null, save: () => undefined, clear: () => undefined } },
        { provide: PopoutBackend, useValue: backend },
        { provide: POPOUT_ROUTE, useValue: ROUTE },
      ],
    });
    // The host's end of the group: answers hello with its state and receives commands.
    const host = TestBed.runInInjectionContext(() => new UiSync());
    const hostQuery = signal('host');
    const commands: string[] = [];
    host.start(backend.bus.port(), true, 'main', [{ key: 'hostTitle', direction: 'fromHost', read: () => 'Incident 42', apply: () => undefined }, { key: 'x', read: () => hostQuery(), apply: () => undefined }], 'oh');
    host.onCommand((c, a) => commands.push(`${c}:${a}`));
    const fixture = TestBed.createComponent(PopoutShell);
    await settle(fixture);
    await vi.waitFor(async () => {
      await settle(fixture);
      expect((fixture.nativeElement as HTMLElement).querySelector('debrief-inspector')).not.toBeNull();
    });
    return { fixture, el: fixture.nativeElement as HTMLElement, commands, host };
  }

  it('shows the panel title, the host project and the view once it caught up with the host', async () => {
    const { el } = await shell();
    expect(el.querySelector('h1')!.textContent).toBe('Inspector');
    expect(el.querySelector('.host')!.textContent).toBe('Incident 42');
    expect(el.querySelector('main')!.getAttribute('aria-busy')).toBe('false');
  });

  it('docks back and keeps itself on top through the backend; failures show inline', async () => {
    const { fixture, el } = await shell();
    el.querySelector<HTMLButtonElement>('button[aria-label="Dock back into the main window"]')!.click();
    await settle(fixture);
    expect(backend.docked).toEqual(['self']);
    const onTop = el.querySelector<HTMLButtonElement>('button[aria-label="Keep on top of Debrief"]')!;
    onTop.click();
    await settle(fixture);
    expect(backend.onTop).toEqual([true]);
    expect(onTop.getAttribute('aria-pressed')).toBe('true');
    backend.fail = new Error('not allowed');
    onTop.click();
    await settle(fixture);
    expect(el.querySelector('.toast')!.textContent).toContain('not allowed');
    backend.dockSelf = async () => {
      throw new Error('gone');
    };
    el.querySelector<HTMLButtonElement>('button[aria-label="Dock back into the main window"]')!.click();
    await settle(fixture);
    expect(TestBed.inject(WorkspaceStore).error()).toContain('Could not dock the panel back: gone');
  });

  it('sends undo/redo to the host, runs other keys here, and shows views in the host', async () => {
    const { commands } = await shell();
    const press = (init: KeyboardEventInit, target: EventTarget = document) => target.dispatchEvent(new KeyboardEvent('keydown', { bubbles: true, cancelable: true, ...init }));
    press({ key: 'z', ctrlKey: true, metaKey: false });
    press({ key: 'z', metaKey: true });
    press({ key: 'Z', ctrlKey: true, shiftKey: true });
    press({ key: 'Z', metaKey: true, shiftKey: true });
    const input = document.createElement('input');
    document.body.appendChild(input);
    press({ key: 'z', ctrlKey: true }, input);
    input.remove();
    press({ key: 'x' });
    TestBed.inject(DockService).show('metrics');
    TestBed.inject(DockService).show('inspector');
    expect(commands.filter((c) => c === 'run:edit.undo')).toHaveLength(1);
    expect(commands.filter((c) => c === 'run:edit.redo')).toHaveLength(1);
    expect(commands).toContain('showView:metrics');
    expect(commands).not.toContain('showView:inspector');
  });

  it('keeps the window title on the panel even when something else sets it', async () => {
    await shell();
    await vi.waitFor(() => expect(document.title).toBe('Inspector \u2014 Incident 42'));
    document.title = 'Untitled \u2014 Debrief';
    await vi.waitFor(() => expect(document.title).toBe('Inspector \u2014 Incident 42'));
  });

  it('hides "Keep on top of Debrief" where windows cannot stay on top (a browser)', async () => {
    const { el } = await shell((b) => (b.supportsKeepOnTop = false));
    expect(el.querySelector('button[aria-label="Keep on top of Debrief"]')).toBeNull();
  });

  it('dragging the title bar lets the OS move the window, then tells the host the drag ended', async () => {
    const { el, commands } = await shell((b) => (b.windowDrag = true));
    el.querySelector('.titles')!.dispatchEvent(new PointerEvent('pointerdown', { button: 0, bubbles: true }));
    await vi.waitFor(() => expect(commands).toContain('dragEnd:p-main-1'));
  });

  it('dragging the title bar in a browser sends the pointer screen position to the host', async () => {
    const { el, commands } = await shell((b) => (b.chrome = 'browser'));
    const handle = el.querySelector<HTMLElement>('debrief-popout-title-bar .bar')!;
    handle.dispatchEvent(new PointerEvent('pointerdown', { button: 2, bubbles: true }));
    handle.dispatchEvent(new PointerEvent('pointerdown', { button: 0, bubbles: true, pointerId: 7 }));
    await settle();
    handle.dispatchEvent(new PointerEvent('pointermove', { screenX: 300, screenY: 200, bubbles: true }));
    handle.dispatchEvent(new PointerEvent('pointerup', { bubbles: true }));
    handle.dispatchEvent(new PointerEvent('pointermove', { screenX: 1, screenY: 1, bubbles: true }));
    expect(commands).toEqual(['dragOver:{"label":"p-main-1","x":300,"y":200}', 'dragEnd:p-main-1']);
  });

  it('runs the shared native menu\'s clicks here, or in the host for layout and history commands', async () => {
    const { commands } = await shell();
    const run = vi.spyOn(TestBed.inject(CommandRegistry), 'run').mockResolvedValue(true);
    backend.menu!('window.layout.reset');
    backend.menu!('view.toggleSidebar');
    backend.menu!('edit.undo');
    backend.menu!('palette.open');
    expect(commands).toEqual(['run:window.layout.reset', 'run:view.toggleSidebar', 'run:edit.undo']);
    expect(run.mock.calls).toEqual([['palette.open']]);
  });

  it('closes itself when the host goes away', async () => {
    const close = vi.spyOn(window, 'close').mockImplementation(() => undefined);
    const { host } = await shell();
    host.goodbye();
    expect(close).toHaveBeenCalled();
    close.mockRestore();
  });
});

describe('pinDocumentTitle', () => {
  it('re-applies the title until cleaned up', async () => {
    const stop = pinDocumentTitle(() => 'Pinned', TestBed.inject(Injector));
    TestBed.tick();
    expect(document.title).toBe('Pinned');
    document.title = 'Other';
    await vi.waitFor(() => expect(document.title).toBe('Pinned'));
    stop();
    document.title = 'Free';
    await new Promise((r) => setTimeout(r, 10));
    expect(document.title).toBe('Free');
  });
});

describe('providePopout', () => {
  it('in a normal window: the dock gets the pop-out service as its panel windows', () => {
    TestBed.configureTestingModule({ providers: [provideDockTesting(), ...provideToolsBackend(), providePopout('')] });
    expect(TestBed.inject(PANEL_WINDOWS)).toBe(TestBed.inject(PopoutService));
  });

  it('in a pop-out window: no session autosave, no queued files, and the shell replaces the app', async () => {
    TestBed.configureTestingModule({ providers: [provideDockTesting(), ...provideToolsBackend(), providePopout('#/popout?view=inspector&host=main&label=p-main-1')] });
    expect(TestBed.inject(SessionService)).toBe(INERT_SESSION);
    expect(TestBed.inject(OpenRequests)).toBe(INERT_OPEN_REQUESTS);
    INERT_SESSION.start();
    expect(await INERT_SESSION.flush()).toBe(false);
    expect(await INERT_SESSION.saveNow()).toBe(false);
    INERT_OPEN_REQUESTS.stop();
    await INERT_OPEN_REQUESTS.start();
    await INERT_OPEN_REQUESTS.drain();
    await INERT_OPEN_REQUESTS.handle();
  });
});
