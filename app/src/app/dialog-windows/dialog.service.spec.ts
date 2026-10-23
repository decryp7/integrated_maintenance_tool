import { Component, inject } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { CommandRegistry } from '../extensions/command-registry';
import { DEBRIEF_COMMANDS } from '../extensions/tokens';
import { provideTinyBackendBare } from '../../testing/helpers';
import { DIALOG_REF, DialogService } from '../ui/dialog.service';
import { ConfirmDialog } from '../ui/confirm-dialog';
import { DialogPresenter } from './dialog-presenter';
import { ConfirmQuestion, DialogSession, DialogShowRequest } from './dialog-session';
import { InPageDialogBackend } from './in-page-dialog-backend';
import { ModalGate } from './modal-gate';

@Component({ selector: 'debrief-modal-x', template: `<button (click)="ref.close('yes')">ok</button>` })
class ModalX {
  static readonly dialogId = 'unsaved-changes';
  protected readonly ref = inject(DIALOG_REF);
}
@Component({ selector: 'debrief-modeless-x', template: '' })
class ModelessX {
  static readonly dialogId = 'about';
}
@Component({ selector: 'debrief-overlay-x', template: '' })
class OverlayX {}

/** A windowed backend that records what it was asked to show; each session is resolved by the test. */
class FakeWindows implements DialogPresenter {
  readonly windowed = true;
  readonly shown: DialogShowRequest[] = [];
  readonly sessions: { resolve: (r?: unknown) => void; focused: number; closes: unknown[] }[] = [];
  answer = true;
  asked: ConfirmQuestion[] = [];
  show(request: DialogShowRequest): DialogSession {
    this.shown.push(request);
    let resolve!: (r?: unknown) => void;
    const closed = new Promise<unknown>((r) => (resolve = r));
    const rec = { resolve, focused: 0, closes: [] as unknown[] };
    this.sessions.push(rec);
    return { closed, focus: () => rec.focused++, close: (r) => rec.closes.push(r) };
  }
  confirm = async (q: ConfirmQuestion) => {
    this.asked.push(q);
    return this.answer;
  };
}

describe('DialogService with windows', () => {
  let windows: FakeWindows;
  beforeEach(() => {
    windows = new FakeWindows();
    TestBed.configureTestingModule({ providers: [{ provide: DialogPresenter, useValue: windows }] });
  });

  it('passes the typed inputs and the registered size to the backend and returns the result', async () => {
    const dialogs = TestBed.inject(DialogService);
    const ref = dialogs.open<string>(ModalX, { title: 'Unsaved', inputs: { name: 'a.debriefz' } });
    expect(windows.shown[0]).toMatchObject({ title: 'Unsaved', inputs: { name: 'a.debriefz' }, width: 440 });
    expect(windows.shown[0]!.def?.id).toBe('unsaved-changes');
    windows.sessions[0]!.resolve('yes');
    expect(await ref.closed).toBe('yes');
    ref.close('later');
    expect(windows.sessions[0]!.closes).toEqual(['later']);
  });

  it('opens one window per dialog id: opening again focuses it and returns the same ref', async () => {
    const dialogs = TestBed.inject(DialogService);
    const a = dialogs.open(ModelessX, { title: 'About' });
    const b = dialogs.open(ModelessX, { title: 'About' });
    expect(b).toBe(a);
    expect(windows.shown).toHaveLength(1);
    expect(windows.sessions[0]!.focused).toBe(1);
    windows.sessions[0]!.resolve();
    await a.closed;
    await Promise.resolve();
    dialogs.open(ModelessX, { title: 'About' });
    expect(windows.shown).toHaveLength(2);
  });

  it('a modal window blocks its parent (commands and the gate) until it closes; a modeless one does not', async () => {
    const ran = vi.fn();
    TestBed.configureTestingModule({ providers: [...provideTinyBackendBare(), { provide: DEBRIEF_COMMANDS, useValue: [{ id: 't.x', title: 'x', keybinding: 'F9', run: ran }], multi: true }] });
    const dialogs = TestBed.inject(DialogService);
    const gate = TestBed.inject(ModalGate);
    const registry = TestBed.inject(CommandRegistry);
    const modeless = dialogs.open(ModelessX, { title: 'About' });
    expect(dialogs.parentBlocked()).toBe(false);
    expect(gate.blocked()).toBe(false);
    const modal = dialogs.open(ModalX, { title: 'Q' });
    expect(dialogs.parentBlocked()).toBe(true);
    expect(gate.blocked()).toBe(true);
    expect(await registry.run('t.x')).toBe(false);
    expect(registry.runForKey(new KeyboardEvent('keydown', { key: 'F9' }))).toBe(false);
    expect(ran).not.toHaveBeenCalled();
    windows.sessions[1]!.resolve();
    await modal.closed;
    await Promise.resolve();
    expect(await registry.run('t.x')).toBe(true);
    expect(ran).toHaveBeenCalledTimes(1);
    expect(gate.blocked()).toBe(false);
    windows.sessions[0]!.resolve();
    await modeless.closed;
  });

  it('shows overlays (no registered id) without a single-instance rule and without blocking', () => {
    const dialogs = TestBed.inject(DialogService);
    dialogs.open(OverlayX, { title: 'Palette' });
    dialogs.open(OverlayX, { title: 'Palette' });
    expect(windows.shown).toHaveLength(2);
    expect(windows.shown[0]!.def).toBeNull();
    expect(dialogs.parentBlocked()).toBe(false);
  });

  it('asks a yes/no question with the native message box and blocks the parent while it is open', async () => {
    const dialogs = TestBed.inject(DialogService);
    const ref = dialogs.open<boolean>(ConfirmDialog, { title: 'Delete?', inputs: { message: 'Sure?', detail: '/a/b', confirmLabel: 'Delete', danger: true } });
    expect(dialogs.parentBlocked()).toBe(true);
    expect(await ref.closed).toBe(true);
    expect(windows.asked[0]).toEqual({ title: 'Delete?', message: 'Sure?\n\n/a/b', ok: 'Delete', cancel: expect.any(String), danger: true });
    expect(windows.shown).toHaveLength(0);
    await Promise.resolve();
    expect(dialogs.parentBlocked()).toBe(false);
    windows.answer = false;
    expect(await dialogs.open<boolean>(ConfirmDialog, { title: 'x' }).closed).toBeUndefined();
    windows.confirm = () => Promise.reject(new Error('boom')) as never;
    expect(await dialogs.open<boolean>(ConfirmDialog, { title: 'x', inputs: { message: 'm' } }).closed).toBeUndefined();
  });
});

describe('DialogService in the page', () => {
  it('shows the dialog in the page backend, modal with a backdrop semantics, and answers', async () => {
    const dialogs = TestBed.inject(DialogService);
    const ref = dialogs.open<string>(ModalX, { title: 'Q', inputs: {} });
    expect(dialogs.stack()).toHaveLength(1);
    expect(dialogs.stack()[0]!.modality).toBe('modal');
    expect(dialogs.parentBlocked()).toBe(false);
    ref.close('a');
    expect(await ref.closed).toBe('a');
    expect(dialogs.stack()).toHaveLength(0);
  });

  it('keeps one instance of a dialog id and shows modeless dialogs as modeless', () => {
    const dialogs = TestBed.inject(DialogService);
    const a = dialogs.open(ModelessX, { title: 'About' });
    const b = dialogs.open(ModelessX, { title: 'About' });
    expect(b).toBe(a);
    expect(dialogs.stack()).toHaveLength(1);
    expect(dialogs.stack()[0]!.modality).toBe('modeless');
    // opening it again raises it, but not over a modal dialog that is being answered
    dialogs.open(ModalX, { title: 'Q' });
    dialogs.open(ModelessX, { title: 'About' });
    expect(dialogs.stack().at(-1)?.modality).toBe('modal');
  });

  it('asks canClose before closing, from the options and from the component', () => {
    const dialogs = TestBed.inject(DialogService);
    let allow = false;
    const ref = dialogs.open(OverlayX, { title: 'O', canClose: () => allow });
    ref.close();
    expect(dialogs.stack()).toHaveLength(1);
    allow = true;
    ref.close();
    expect(dialogs.stack()).toHaveLength(0);
    class Guarded {
      static readonly dialogId = 'guarded';
      static dialogCanClose = () => false;
    }
    const g = TestBed.inject(InPageDialogBackend).show({ def: null, component: Guarded, title: 'G', inputs: {}, width: 300 });
    g.close();
    expect(TestBed.inject(InPageDialogBackend).stack()).toHaveLength(1);
  });
});

describe('raising an in-page dialog', () => {
  it('brings it to the front, except over a modal dialog opened after it', async () => {
    const { raise } = await import('./in-page-dialog-backend');
    const d = (id: number, modality: 'modal' | 'modeless') => ({ id, modality }) as never;
    const stack = [d(1, 'modeless'), d(2, 'modeless'), d(3, 'modal')];
    expect(raise(stack, 1)).toBe(stack);
    expect(raise(stack, 9)).toBe(stack);
    expect(raise([d(1, 'modeless'), d(2, 'modeless')], 1).map((x) => x.id)).toEqual([2, 1]);
  });
});
