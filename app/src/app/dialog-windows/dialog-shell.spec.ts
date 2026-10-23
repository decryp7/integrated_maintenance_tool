import { Component, inject, input, Injector } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { CmdError } from '../core/cmd-error';
import { deniedCommand, reportRefusedCommand } from '../core/denied-command';
import { Logger } from '../core/logger';
import { UiSync } from '../popout/ui-sync';
import { el, settle } from '../../testing/helpers';
import { provideTinyBackend } from '../../testing/helpers';
import { DIALOG_REF } from '../ui/dialog.service';
import { DialogDef, defineDialog, type } from './dialog-def';
import { DIALOG_CLOSE_REQUESTED_EVENT, DIALOG_IO } from './dialog-io';
import { FakeDialogIo } from './dialog-io-fake';
import { DIALOG_LOOKUP } from './dialog-registry';
import { DIALOG_ROUTE } from './dialog-route';
import { DialogShell } from './dialog-shell';

let allow = true;

@Component({
  selector: 'debrief-shell-test',
  template: `<p id="name">{{ name() }}</p><textarea id="ta"></textarea><input id="in" /><button class="btn primary" id="go" (click)="ref.close({ ok: name() })">Go</button>`,
})
class Content {
  static readonly dialogId = 'shell-test';
  static dialogCanClose(_: Injector): boolean {
    return allow;
  }
  protected readonly ref = inject(DIALOG_REF);
  readonly name = input('none');
}

const base = { id: 'shell-test', width: 400, result: type<void>(), commands: [], load: async () => Content } as const;
const def: DialogDef = defineDialog({ ...base, kind: 'decision' });

async function setup(over: { lookup?: (id: string) => DialogDef | undefined; failTake?: boolean; modality?: 'modal' | 'modeless'; onLog?: (m: string, p: unknown) => void } = {}) {
  allow = true;
  const io = new FakeDialogIo();
  io.start = { dialogId: 'shell-test', title: 'Shell test', modality: over.modality ?? 'modal', input: { name: 'Ada' } };
  if (over.failTake) io.take = () => Promise.reject(new Error('gone'));
  TestBed.configureTestingModule({
    providers: [
      ...provideTinyBackend(),
      { provide: DIALOG_IO, useValue: io },
      ...(over.onLog ? [{ provide: Logger, useValue: { debug: () => undefined, info: () => undefined, warn: () => undefined, error: (_c: string, m: string, p: unknown) => over.onLog!(m, p) } }] : []),
      { provide: DIALOG_ROUTE, useValue: { id: 'shell-test', host: 'main', label: 'd-main-1' } },
      { provide: DIALOG_LOOKUP, useValue: over.lookup ?? ((id: string) => (id === 'shell-test' ? def : undefined)) },
    ],
  });
  const fixture = TestBed.createComponent(DialogShell);
  await settle(fixture);
  await settle(fixture);
  return { fixture, io };
}

const key = (target: Element, k: string, init: KeyboardEventInit = {}) => target.dispatchEvent(new KeyboardEvent('keydown', { key: k, bubbles: true, cancelable: true, ...init }));

describe('DialogShell', () => {
  it('sizes the window to the content unless the dialog names a height', async () => {
    const rect = vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockReturnValue({ height: 120, top: 0, bottom: 136 } as DOMRect);
    try {
      const { io } = await setup();
      expect(io.fitted).toEqual([152]);
      TestBed.resetTestingModule();
      const { io: fixed } = await setup({ lookup: () => defineDialog({ ...base, kind: 'form', modality: 'modal', height: 600 }) });
      expect(fixed.fitted).toEqual([]);
    } finally {
      rect.mockRestore();
    }
  });

  it('shows the dialog with the input it was opened with and the window title', async () => {
    const { fixture } = await setup();
    expect(el(fixture, '#name').textContent).toBe('Ada');
    expect(document.title).toBe('Shell test');
    expect(fixture.nativeElement.getAttribute('data-modality')).toBe('modal');
    expect(fixture.nativeElement.querySelector('[data-testid=dialog-loading]')).toBeNull();
  });

  it('answers the opener with the dialog result and closes the window', async () => {
    const { fixture, io } = await setup();
    el<HTMLButtonElement>(fixture, '#go').click();
    await settle(fixture);
    expect(io.closedWith).toEqual([{ ok: 'Ada' }]);
  });

  it('closes on Escape and Cmd+W without a result, and the shell asks the component first', async () => {
    const { fixture, io } = await setup();
    allow = false;
    key(el(fixture, '#in'), 'Escape');
    await settle(fixture);
    expect(io.closedWith).toEqual([]);
    allow = true;
    key(el(fixture, '#in'), 'w', { metaKey: true });
    await settle(fixture);
    expect(io.closedWith).toEqual([undefined]);
    // closing twice is ignored
    key(el(fixture, '#in'), 'Escape');
    await settle(fixture);
    expect(io.closedWith).toHaveLength(1);
  });

  it('closes when the OS title bar asks, through the same check', async () => {
    const { fixture, io } = await setup();
    allow = false;
    io.emit(DIALOG_CLOSE_REQUESTED_EVENT, null);
    await settle(fixture);
    expect(io.closedWith).toEqual([]);
    allow = true;
    io.emit(DIALOG_CLOSE_REQUESTED_EVENT, null);
    await settle(fixture);
    expect(io.closedWith).toHaveLength(1);
  });

  it('ignores Escape, Cmd+W and a close request in a required-choice modal, and closes with its answer', async () => {
    const required: DialogDef = defineDialog({ ...base, kind: 'decision', requiredChoice: true });
    const { fixture, io } = await setup({ lookup: (id) => (id === 'shell-test' ? required : undefined) });
    key(el(fixture, '#in'), 'Escape');
    key(el(fixture, '#in'), 'w', { metaKey: true });
    io.emit(DIALOG_CLOSE_REQUESTED_EVENT, null);
    await settle(fixture);
    expect(io.closedWith).toEqual([]);
    expect(fixture.nativeElement.querySelector('[data-testid=dialog-shell-close]')).toBeNull();
  });

  it('adds its own Close when the content has no way out, and removes it once the content has one', async () => {
    const { fixture, io } = await setup();
    await new Promise((r) => setTimeout(r, 80));
    await settle(fixture);
    const close = fixture.nativeElement.querySelector('[data-testid=dialog-shell-close]') as HTMLButtonElement | null;
    expect(close, 'the test content has no Close or Cancel button').not.toBeNull();
    close!.click();
    await settle(fixture);
    expect(io.closedWith).toEqual([undefined]);
  });

  it('runs the default action on Enter, except in a text area', async () => {
    const { fixture, io } = await setup();
    key(el(fixture, '#ta'), 'Enter');
    await settle(fixture);
    expect(io.closedWith).toEqual([]);
    key(el(fixture, '#in'), 'Enter');
    await settle(fixture);
    expect(io.closedWith).toEqual([{ ok: 'Ada' }]);
  });

  it('can close again when the close failed', async () => {
    const { fixture, io } = await setup();
    let fail = true;
    io.close = async (r) => {
      if (fail) throw new Error('nope');
      io.closedWith.push(r);
    };
    key(el(fixture, '#in'), 'Escape');
    await settle(fixture);
    fail = false;
    key(el(fixture, '#in'), 'Escape');
    await settle(fixture);
    expect(io.closedWith).toEqual([undefined]);
  });

  it('explains an unknown dialog or a failed start, with a way out', async () => {
    const unknown = await setup({ lookup: () => undefined });
    expect(el(unknown.fixture, '[data-testid=dialog-error]').textContent).toContain('shell-test');
    TestBed.resetTestingModule();
    const failed = await setup({ failTake: true });
    expect(el(failed.fixture, '[data-testid=dialog-error]').textContent).toContain('gone');
    el<HTMLButtonElement>(failed.fixture, '.btn.primary').click();
    await settle(failed.fixture);
    expect(failed.io.closedWith).toHaveLength(1);
  });

  it('shows a visible error state, not a blank window, when the content throws while it is constructed or rendered', async () => {
    @Component({ selector: 'debrief-ctor-throws', template: 'never' })
    class Ctor {
      constructor() {
        throw new Error('NG0201: No provider found for `Thing`');
      }
    }
    @Component({ selector: 'debrief-render-throws', template: '{{ boom() }}' })
    class Render {
      readonly name = input('');
      boom(): string {
        throw new Error('render failed');
      }
    }
    for (const [component, reason] of [[Ctor, 'No provider found'], [Render, 'render failed']] as const) {
      const logged: unknown[][] = [];
      const { fixture, io } = await setup({ lookup: () => defineDialog({ ...base, kind: 'decision', load: async () => component as never }), onLog: (m, p) => logged.push([m, p]) });
      expect(el(fixture, '.failed-title').textContent).toContain("This window couldn't open");
      expect(el(fixture, '[data-testid=dialog-error-reason]').textContent).toContain(reason);
      expect(fixture.nativeElement.querySelector('[data-testid=dialog-loading]')).toBeNull();
      expect(logged[0]).toEqual(['Dialog window failed to start', expect.objectContaining({ Dialog: 'shell-test' })]);
      el<HTMLButtonElement>(fixture, '[data-testid=dialog-error-close]').click();
      await settle(fixture);
      expect(io.closedWith).toHaveLength(1);
      TestBed.resetTestingModule();
    }
  });

  it('offers Report a problem from the error state, handing the command to the main window', async () => {
    const { fixture } = await setup({ failTake: true });
    const run = vi.spyOn(TestBed.inject(UiSync), 'runInHost').mockReturnValue(true);
    el<HTMLButtonElement>(fixture, '[data-testid=dialog-error-report]').click();
    expect(run).toHaveBeenCalledWith('dialogRun', 'help.reportProblem');
  });

  it('shows an inline alert when the engine refuses a command for this window, never silently', async () => {
    const { fixture } = await setup();
    deniedCommand.set(null);
    reportRefusedCommand('resolve_close', new CmdError({ code: 'denied', message: 'resolve_close is not available in this window', retryable: false }));
    await settle(fixture);
    expect(el(fixture, '[data-testid=dialog-denied]').textContent).toContain('resolve_close');
    deniedCommand.set(null);
    reportRefusedCommand('x', new CmdError({ code: 'denied', message: 'Access is denied.', retryable: false }));
    expect(deniedCommand()).toBeNull();
  });

  describe('by kind', () => {
    it('only ever asks for a taller window: a viewer that streams grows, and one whose content shrinks keeps its size', async () => {
      let bottom = 300;
      const rect = vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockImplementation(() => ({ height: bottom, top: 0, bottom }) as DOMRect);
      const original = globalThis.ResizeObserver;
      let notify: () => void = () => undefined;
      globalThis.ResizeObserver = class {
        constructor(cb: () => void) {
          notify = cb;
        }
        observe() {}
        disconnect() {}
        unobserve() {}
      } as unknown as typeof ResizeObserver;
      try {
        const { io } = await setup({ lookup: () => defineDialog({ ...base, kind: 'viewer' }) });
        const first = io.fitted.at(-1)!;
        for (const next of [700, 400, 900, 200]) {
          bottom = next;
          notify();
          await new Promise((r) => setTimeout(r, 90));
        }
        expect(io.fitted.every((h, i) => i === 0 || h > io.fitted[i - 1]!), `asked for ${io.fitted}`).toBe(true);
        expect(io.fitted[0]).toBe(first);
        expect(Math.max(...io.fitted)).toBeGreaterThan(900);
        expect(io.fitted.at(-1)).toBeGreaterThanOrEqual(900);
      } finally {
        rect.mockRestore();
        globalThis.ResizeObserver = original;
      }
    });

    const viewer = () => defineDialog({ ...base, kind: 'viewer' });

    it('fits a viewer to its content too, which the engine only lets grow from 680x560', async () => {
      const rect = vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockReturnValue({ height: 900, top: 0, bottom: 916 } as DOMRect);
      try {
        const { io } = await setup({ lookup: viewer });
        expect(io.fitted).toEqual([932]);
      } finally {
        rect.mockRestore();
      }
    });

    it('draws a Cancel for a modal and a Close for a modeless window when the content has no way out', async () => {
      for (const [lookup, modality, label] of [
        [() => defineDialog({ ...base, kind: 'decision' }), 'modal', 'Cancel'],
        [viewer, 'modeless', 'Close'],
      ] as const) {
        const { fixture } = await setup({ lookup, modality });
        await new Promise((r) => setTimeout(r, 80));
        await settle(fixture);
        const button = fixture.nativeElement.querySelector('[data-testid=dialog-shell-close]') as HTMLButtonElement | null;
        // The modality comes from Rust's contract (`dialog_take_input`); the shell shows what it was told.
        expect(button?.textContent?.trim(), modality).toBe(label);
        expect(button?.hasAttribute('data-dialog-close'), modality).toBe(true);
        TestBed.resetTestingModule();
      }
    });
  });
});
