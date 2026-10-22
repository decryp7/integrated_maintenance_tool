import { Component, inject } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { all, el, settle } from '../../testing/helpers';
import { BUILTIN_VIEWS } from '../builtin/builtin-views';
import { observeFloatingAria } from '../dock/dock.service';
import { DIALOG_REF, DialogService } from './dialog.service';
import { DialogHost } from './dialog-host';

@Component({
  selector: 'debrief-test-dialog',
  template: `<input id="first" /><button id="last" (click)="ref.close('done')">Done</button>`,
})
class TestDialog {
  protected readonly ref = inject(DIALOG_REF);
}

describe('DialogService and DialogHost', () => {
  async function setup() {
    const fixture = TestBed.createComponent(DialogHost);
    const dialogs = TestBed.inject(DialogService);
    await settle(fixture);
    return { fixture, dialogs };
  }
  const key = (target: Element, k: string, init: KeyboardEventInit = {}) => target.dispatchEvent(new KeyboardEvent('keydown', { key: k, bubbles: true, cancelable: true, ...init }));

  it('opens a titled modal dialog, focuses the first control and resolves with the result', async () => {
    const { fixture, dialogs } = await setup();
    const opener = document.createElement('button');
    document.body.appendChild(opener);
    opener.focus();
    const ref = dialogs.open<string>(TestDialog, { title: 'Hello', width: 400 });
    await settle(fixture);
    const dialog = el(fixture, '[role=dialog]');
    expect(dialog.getAttribute('aria-modal')).toBe('true');
    expect(document.getElementById(dialog.getAttribute('aria-labelledby')!)?.textContent).toBe('Hello');
    expect(dialog.style.width).toBe('400px');
    expect(document.activeElement?.id).toBe('first');
    el<HTMLButtonElement>(fixture, '#last').click();
    expect(await ref.closed).toBe('done');
    await settle(fixture);
    expect(fixture.nativeElement.querySelector('[role=dialog]')).toBeNull();
    expect(document.activeElement).toBe(opener);
    ref.close('again');
    opener.remove();
  });

  it('closes with Escape (inside or outside), the close button and the backdrop; dismissTop is a no-op when empty', async () => {
    const { fixture, dialogs } = await setup();
    dialogs.dismissTop();
    let ref = dialogs.open(TestDialog, { title: 'A' });
    await settle(fixture);
    key(el(fixture, '#first'), 'Escape');
    expect(await ref.closed).toBeUndefined();

    ref = dialogs.open(TestDialog, { title: 'B', inputs: {} });
    await settle(fixture);
    document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true }));
    await ref.closed;

    document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));

    ref = dialogs.open(TestDialog, { title: 'C' });
    await settle(fixture);
    el<HTMLButtonElement>(fixture, '[aria-label="Close dialog"]').click();
    await ref.closed;

    ref = dialogs.open(TestDialog, { title: 'D' });
    await settle(fixture);
    el(fixture, '.backdrop').dispatchEvent(new MouseEvent('mousedown', { bubbles: true }));
    await ref.closed;
    expect(dialogs.stack()).toHaveLength(0);
  });

  it('keeps a dialog open by every close route while canClose says no', async () => {
    const { fixture, dialogs } = await setup();
    let allow = false;
    const canClose = vi.fn(() => allow);
    const ref = dialogs.open(TestDialog, { title: 'Guarded', canClose });
    await settle(fixture);
    key(el(fixture, '#first'), 'Escape');
    el<HTMLButtonElement>(fixture, '[aria-label="Close dialog"]').click();
    el(fixture, '.backdrop').dispatchEvent(new MouseEvent('mousedown', { bubbles: true }));
    dialogs.dismissTop();
    ref.close();
    expect(canClose).toHaveBeenCalledTimes(5);
    expect(dialogs.stack()).toHaveLength(1);
    allow = true;
    el<HTMLButtonElement>(fixture, '#last').click();
    expect(await ref.closed).toBe('done');
    expect(dialogs.stack()).toHaveLength(0);
  });

  it('traps Tab within the dialog and stacks dialogs', async () => {
    const { fixture, dialogs } = await setup();
    dialogs.open(TestDialog, { title: 'One' });
    await settle(fixture);
    const dialog = el(fixture, '[role=dialog]');
    const close = el<HTMLButtonElement>(fixture, '[aria-label="Close dialog"]');
    const last = el<HTMLButtonElement>(fixture, '#last');
    last.focus();
    key(last, 'Tab');
    expect(document.activeElement).toBe(close);
    key(close, 'Tab', { shiftKey: true });
    expect(document.activeElement).toBe(last);
    key(dialog, 'Tab');
    key(dialog, 'a');
    dialogs.open(TestDialog, { title: 'Two' });
    await settle(fixture);
    expect(all(fixture, '[role=dialog]')).toHaveLength(2);
    dialogs.dismissTop();
    await settle(fixture);
    expect(all(fixture, '[role=dialog]')).toHaveLength(1);
  });

  it('tolerates a dialog without focusable content', async () => {
    @Component({ selector: 'debrief-empty-dialog', template: 'nothing to focus' })
    class EmptyDialog {}
    const { fixture, dialogs } = await setup();
    dialogs.open(EmptyDialog, { title: 'Empty' });
    await settle(fixture);
    const box = el(fixture, '[role=dialog]');
    box.querySelector('.head button')?.remove();
    key(box, 'Tab');
    expect(box.textContent).toContain('nothing to focus');
  });
});

describe('built-in views', () => {
  it('lazily load their components', async () => {
    const types = await Promise.all(BUILTIN_VIEWS.map((v) => v.component()));
    expect(types.every((t) => typeof t === 'function')).toBe(true);
    expect(BUILTIN_VIEWS.map((v) => v.id)).toEqual(['timeline', 'events', 'inspector', 'metrics', 'profile', 'parsers', 'plugins'].slice(0, BUILTIN_VIEWS.length));
  });
});

describe('observeFloatingAria', () => {
  it('strips aria attributes axe rejects from floating groups, including ones added or changed later', async () => {
    const root = document.createElement('div');
    root.innerHTML = '<div class="dv-resize-container" role="dialog" aria-modal="false" aria-level="0"></div>';
    const handle = observeFloatingAria(root);
    const box = root.firstElementChild!;
    expect(box.hasAttribute('aria-modal')).toBe(false);
    expect(box.hasAttribute('aria-level')).toBe(false);
    box.setAttribute('aria-level', '2');
    const added = document.createElement('div');
    added.className = 'dv-resize-container';
    added.setAttribute('aria-modal', 'false');
    root.appendChild(added);
    await new Promise((r) => setTimeout(r, 0));
    expect(box.hasAttribute('aria-level')).toBe(false);
    expect(added.hasAttribute('aria-modal')).toBe(false);
    handle.dispose();
    box.setAttribute('aria-level', '3');
    await new Promise((r) => setTimeout(r, 0));
    expect(box.getAttribute('aria-level')).toBe('3');
  });
});
