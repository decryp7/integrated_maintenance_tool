import { computed, inject, Injectable, InjectionToken, signal, Type } from '@angular/core';
import { DialogModality, dialogIdOf } from '../dialog-windows/dialog-def';
import { dialogDef } from '../dialog-windows/dialog-registry';
import { DialogBackend } from '../dialog-windows/dialog-backend';
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

export interface OpenDialog {
  id: number;
  component: Type<unknown>;
  options: DialogOptions;
  ref: DialogRef;
  modality: DialogModality;
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
  private readonly backend = inject(DialogBackend);
  private readonly inPage = inject(InPageDialogBackend);
  private readonly live = new Map<string, DialogRef>();
  private readonly sessions = new Map<string, DialogSession>();
  private readonly gate = inject(ModalGate);
  private readonly modalWindows = signal(0);

  /** The dialogs shown in the page (`debrief-dialog-host` renders them). */
  readonly stack = this.inPage.stack;
  /** A modal dialog window is open: its parent must not take input (commands, shortcuts) meanwhile. */
  readonly parentBlocked = computed(() => this.modalWindows() > 0);

  open<R = void>(component: Type<unknown>, options: DialogOptions): DialogRef<R> {
    if ((component as unknown as NativeConfirm).nativeConfirm && this.backend.windowed && this.backend.confirm) return this.confirm<R>(options);
    const id = dialogIdOf(component);
    const def = id ? dialogDef(id) : undefined;
    if (def) {
      const existing = this.live.get(def.id);
      if (existing) {
        this.sessions.get(def.id)?.focus();
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
    });
    const closed = session.closed.then((r) => r as R | undefined);
    const ref: DialogRef<R> = { closed, close: (r) => session.close(r) };
    if (def) {
      this.live.set(def.id, ref as DialogRef);
      this.sessions.set(def.id, session);
      const blocks = this.backend.windowed && def.modality === 'modal';
      if (blocks) this.block();
      void closed.then(() => {
        this.live.delete(def.id);
        this.sessions.delete(def.id);
        if (blocks) this.unblock();
      });
    }
    return ref;
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
