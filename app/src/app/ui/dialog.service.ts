import { computed, inject, Injectable, Injector, InjectionToken, signal, Type } from '@angular/core';
import { WorkspaceStore } from '../core/workspace.store';
import { DialogInputsOf, DialogModality, dialogIdOf, instanceKey } from '../dialog-windows/dialog-def';
import { DialogId, DialogResultOf, dialogDef } from '../dialog-windows/dialog-registry';
import { DialogPresenter } from '../dialog-windows/dialog-presenter';
import { DialogSession } from '../dialog-windows/dialog-session';
import { ModalGate } from '../dialog-windows/modal-gate';
import { InPageDialogBackend } from '../dialog-windows/in-page-dialog-backend';

/** Handle given to a dialog's content component (inject `DIALOG_REF`) to close it with a result. */
export interface DialogRef<R = unknown> {
  close(result?: R): void;
  /** Resolves with the result when the dialog closes (undefined when dismissed). */
  readonly closed: Promise<R | undefined>;
}

export const DIALOG_REF = new InjectionToken<DialogRef>('DIALOG_REF');

export interface DialogOptions {
  /** Shown as the title of the dialog (the window's title bar) and used as its accessible name. */
  title: string;
  /** Values for the content component's inputs. They cross to the dialog's window, so they must be plain data. */
  inputs?: Record<string, unknown>;
  /** Preferred width in px (default: the dialog definition's). */
  width?: number;
  /** In-page dialogs only: asked before the dialog closes. A window asks the component (`dialogCanClose`). */
  canClose?: () => boolean;
}

/** A dialog component of the registry: it declares its id as `static readonly dialogId`. */
export type RegisteredDialog = Type<unknown> & { readonly dialogId: DialogId };

/** The options for opening a registered dialog: its inputs are the component's `input()` signals. */
export type RegisteredDialogOptions<C extends RegisteredDialog> = Omit<DialogOptions, 'inputs'> & { inputs?: DialogInputsOf<InstanceType<C>> };

export interface OpenDialog {
  id: number;
  component: Type<unknown>;
  options: DialogOptions;
  ref: DialogRef;
  modality: DialogModality;
  /** A modal without a Cancel: Escape and the backdrop leave it open (`DialogDef.requiredChoice`). */
  requiredChoice?: boolean;
}

/** A component that is a plain yes/no question: windowed, it is the native message box (`ConfirmDialog`). */
interface NativeConfirm {
  readonly nativeConfirm?: boolean;
}

/**
 * Opens dialogs (ADR 0050). A dialog is a standalone component with a registered id (`dialogDef`); the backend
 * shows it as an OS window (desktop) or in the page (browser build). Calls are the same either way. One dialog
 * per id is open at a time: opening it again focuses the open one and returns its ref.
 */
@Injectable({ providedIn: 'root' })
export class DialogService {
  private readonly backend = inject(DialogPresenter);
  private readonly inPage = inject(InPageDialogBackend);
  private readonly live = new Map<string, DialogRef>();
  private readonly sessions = new Map<string, DialogSession>();
  private readonly gate = inject(ModalGate);
  private readonly injector = inject(Injector);
  private readonly modalWindows = signal(0);

  /** The dialogs shown in the page (`debrief-dialog-host` renders them). */
  readonly stack = this.inPage.stack;
  /** A modal dialog window is open: its parent must not take input (commands, shortcuts) meanwhile. */
  readonly parentBlocked = computed(() => this.modalWindows() > 0);

  /** A registered dialog: the inputs are the component's, the answer is the `result` its registry entry declares. */
  open<C extends RegisteredDialog>(component: C, options: RegisteredDialogOptions<C>): DialogRef<DialogResultOf<C['dialogId']>>;
  /** Anything else (a plain question, `ConfirmDialog`, a dialog shown only in the page): the caller states the answer. */
  open<R = void>(component: Type<unknown> & { readonly dialogId?: undefined }, options: DialogOptions): DialogRef<R>;
  open<R = void>(component: Type<unknown>, options: DialogOptions): DialogRef<R> {
    if ((component as unknown as NativeConfirm).nativeConfirm && this.backend.windowed && this.backend.confirm) return this.confirm<R>(options);
    const id = dialogIdOf(component);
    const def = id ? dialogDef(id) : undefined;
    // One window per dialog id. In the page only modeless dialogs are single (modal ones stack, each blocking the last).
    const key = def ? (def.instanced ? `${def.id}#${instanceKey(options.inputs ?? {})}` : def.id) : '';
    if (def && (this.backend.windowed || def.modality === 'modeless')) {
      const existing = this.sessions.get(key)?.isOpen?.() === false ? undefined : this.live.get(key);
      if (existing) {
        this.sessions.get(key)?.focus();
        return existing as DialogRef<R>;
      }
    }
    const session = this.backend.show({
      def: def ?? null,
      component,
      title: options.title,
      inputs: options.inputs ?? {},
      width: options.width ?? def?.width ?? 560,
      canClose: options.canClose,
      instance: def?.instanced ? key.split('#')[1] : undefined,
    });
    const closed = session.closed.then((r) => r as R | undefined);
    const ref: DialogRef<R> = { closed, close: (r) => session.close(r) };
    if (def) {
      const single = this.backend.windowed || def.modality === 'modeless';
      if (single) {
        this.live.set(key, ref as DialogRef);
        this.sessions.set(key, session);
      }
      const blocks = this.backend.windowed && def.modality === 'modal';
      if (blocks) this.block();
      void closed.then(() => {
        if (single && this.live.get(key) === (ref as DialogRef)) {
          this.live.delete(key);
          this.sessions.delete(key);
        }
        if (blocks) this.unblock();
        if (this.backend.windowed) this.refreshAfterWindow();
      });
    }
    return ref;
  }

  /**
   * `open` for a registered dialog whose component is loaded when it is needed: callers pass `() => import(...)` instead of
   * importing the class, so the dialog's code is not part of the first-paint set of the window that can open it (Paddock
   * rule 35). The ref is returned at once; `close` before the chunk arrived is applied as soon as it opened, and a chunk
   * that fails to load counts as a dismissed dialog (logged).
   */
  openLazy<C extends RegisteredDialog>(load: () => Promise<C>, options: RegisteredDialogOptions<C>): DialogRef<DialogResultOf<C['dialogId']>> {
    type R = DialogResultOf<C['dialogId']>;
    let inner: DialogRef<R> | undefined;
    let early: { result?: R } | undefined;
    const closed = load().then(
      (component) => {
        inner = this.open(component, options);
        if (early) inner.close(early.result);
        return inner.closed;
      },
      (err: unknown) => {
        console.error('a dialog could not be loaded', err);
        return undefined;
      },
    );
    return {
      closed,
      close: (result) => (inner ? inner.close(result) : (early = { result })),
    };
  }

  /**
   * A dialog window works on its own stores and the engine; what it changed in the engine (the profile, sources,
   * parsers) reaches this window by reloading the summary. (Interaction state syncs through `UiSync`.)
   */
  private refreshAfterWindow(): void {
    try {
      void this.injector.get(WorkspaceStore).refreshSummary().catch(() => undefined);
    } catch {
      /* no workspace (a dialog window before it loaded): nothing to refresh */
    }
  }

  private block(): void {
    this.modalWindows.update((n) => n + 1);
    this.gate.enter();
  }

  private unblock(): void {
    this.modalWindows.update((n) => n - 1);
    this.gate.leave();
  }

  /** The native message box for a question; `inputs` are the ConfirmDialog's. */
  private confirm<R>(options: DialogOptions): DialogRef<R> {
    const i = options.inputs ?? {};
    const text = (k: string, fallback: string) => (typeof i[k] === 'string' && i[k] ? (i[k] as string) : fallback);
    const detail = text('detail', '');
    const answered = this.backend.confirm!({
      title: options.title,
      message: detail ? `${text('message', '')}\n\n${detail}` : text('message', ''),
      ok: text('confirmLabel', 'OK'),
      cancel: text('cancelLabel', 'Cancel'),
      danger: i['danger'] === true,
    });
    this.block();
    const closed = answered.then(
      (yes) => (yes ? (true as R) : undefined),
      () => undefined,
    );
    void closed.then(() => this.unblock());
    return { closed, close: () => undefined };
  }

  /** Closes the topmost in-page dialog (Escape), unless its `canClose` says no. */
  dismissTop(): void {
    this.stack().at(-1)?.ref.close();
  }
}
