import { Component, signal } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { provideTinyBackend, settle } from '../../testing/helpers';
import { PopoutBackend } from './popout-backend';
import { POPOUT_ROUTE } from './popout-route';
import { FakeBackend } from './popout-testing';
import { PopoutTitleBar } from './popout-title-bar';

@Component({
  imports: [PopoutTitleBar],
  template: `<imt-popout-title-bar title="Inspector" host="Incident" [onTop]="onTop()" (dockBack)="docked = docked + 1" (toggleOnTop)="onTop.set(!onTop())" />`,
})
class Host {
  readonly onTop = signal(false);
  docked = 0;
}

describe('PopoutTitleBar', () => {
  async function render(chrome: 'overlay' | 'frameless' | 'browser' = 'frameless') {
    const backend = new FakeBackend();
    backend.chrome = chrome;
    TestBed.configureTestingModule({ providers: [...provideTinyBackend(), { provide: PopoutBackend, useValue: backend }, { provide: POPOUT_ROUTE, useValue: { view: 'inspector', host: 'main', label: 'p-main-1' } }] });
    const fixture = TestBed.createComponent(Host);
    await settle(fixture);
    const el = fixture.nativeElement as HTMLElement;
    const button = (name: string) => el.querySelector<HTMLButtonElement>(`button[aria-label="${name}"]`);
    return { fixture, el, backend, button, host: fixture.componentInstance };
  }

  it('shows the panel and host titles, Dock back and Always on top, all reachable by keyboard', async () => {
    const { el, button, host, fixture } = await render();
    expect(el.querySelector('h1')!.textContent).toBe('Inspector');
    expect(el.querySelector('.host')!.textContent).toBe('Incident');
    button('Dock back into the main window')!.click();
    expect(host.docked).toBe(1);
    const onTop = button('Always on top')!;
    expect(onTop.getAttribute('aria-pressed')).toBe('false');
    expect(onTop.querySelector('path')!.getAttribute('fill')).toBe('none');
    onTop.click();
    await settle(fixture);
    expect(onTop.getAttribute('aria-pressed')).toBe('true');
    expect(onTop.querySelector('path')!.getAttribute('fill')).toBe('currentColor');
    for (const b of el.querySelectorAll('button')) {
      expect(b.tabIndex).toBe(0);
      expect(b.getAttribute('aria-label')).toBeTruthy();
      expect(b.querySelector('svg')!.getAttribute('aria-hidden')).toBe('true');
      expect(b.title).toBeTruthy();
    }
  });

  it('frameless: minimise, maximise/restore and close call the window; double-clicking the bar maximises', async () => {
    const { el, button, backend, fixture } = await render();
    expect(el.querySelector('[role=group][aria-label=Window]')).not.toBeNull();
    button('Minimise')!.click();
    button('Maximise')!.click();
    await settle(fixture);
    expect(button('Restore')).not.toBeNull();
    expect(el.querySelectorAll('.edge')).toHaveLength(0);
    button('Restore')!.click();
    await settle(fixture);
    expect(el.querySelectorAll('.edge')).toHaveLength(8);
    el.querySelector('.titles')!.dispatchEvent(new MouseEvent('dblclick', { bubbles: true }));
    await settle(fixture);
    expect(button('Restore')).not.toBeNull();
    // Double-clicking a button is not a title bar double-click.
    button('Restore')!.dispatchEvent(new MouseEvent('dblclick', { bubbles: true }));
    button('Close (dock back)')!.click();
    expect(backend.windowCalls).toEqual(['minimize', 'toggleMaximize', 'toggleMaximize', 'toggleMaximize', 'close']);
    // A resize elsewhere (OS snap) updates the button.
    backend.maximized = false;
    backend.resized!();
    await settle(fixture);
    expect(button('Maximise')).not.toBeNull();
  });

  it('frameless: the edges resize the window', async () => {
    const { el, backend } = await render();
    el.querySelector('.edge.se')!.dispatchEvent(new PointerEvent('pointerdown', { button: 0, bubbles: true }));
    el.querySelector('.edge.n')!.dispatchEvent(new PointerEvent('pointerdown', { button: 2, bubbles: true }));
    expect(backend.windowCalls).toEqual(['resize:SouthEast']);
  });

  it('macOS overlay: no drawn window buttons (the native traffic lights), title inset past them', async () => {
    const { el, button, fixture } = await render('overlay');
    expect(button('Minimise')).toBeNull();
    expect(fixture.debugElement.children[0].nativeElement.classList.contains('mac')).toBe(true);
    expect(el.querySelectorAll('.edge')).toHaveLength(0);
  });

  it('a browser popup: no window buttons, and a double-click does nothing', async () => {
    const { el, button, backend } = await render('browser');
    expect(button('Minimise')).toBeNull();
    el.querySelector('.titles')!.dispatchEvent(new MouseEvent('dblclick', { bubbles: true }));
    expect(backend.windowCalls).toEqual([]);
  });

  it('dims while the window is inactive', async () => {
    const { fixture } = await render();
    const bar = fixture.debugElement.children[0].nativeElement as HTMLElement;
    window.dispatchEvent(new Event('blur'));
    await settle(fixture);
    expect(bar.classList.contains('inactive')).toBe(true);
    window.dispatchEvent(new Event('focus'));
    await settle(fixture);
    expect(bar.classList.contains('inactive')).toBe(false);
  });
});
