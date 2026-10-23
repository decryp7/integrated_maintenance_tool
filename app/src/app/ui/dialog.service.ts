import { Injectable, InjectionToken, signal, Type } from '@angular/core';

/** Handle given to a dialog's content component (inject `DIALOG_REF`) to close it with a result. */
export interface DialogRef<R = unknown> {
  close(result?: R): void;
  /** Resolves with the result when the dialog closes (undefined when dismissed). */
  readonly closed: Promise<R | undefined>;
}

export const DIALOG_REF = new InjectionToken<DialogRef>('DIALOG_REF');

export interface DialogOptions {
  /** Shown in the dialog header and used as its accessible name. */
  title: string;
  /** Values for the content component's inputs. */
  inputs?: Record<string, unknown>;
  /** Preferred width in px (default 560). */
  width?: number;
  /**
   * Asked before the dialog closes by any route (its own close, Escape, the header close button,
   * the backdrop). Returning false keeps it open, e.g. to confirm discarding unsaved edits.
   */
  canClose?: () => boolean;
}

export interface OpenDialog {
  id: number;
  component: Type<unknown>;
  options: DialogOptions;
  ref: DialogRef;
}

let nextId = 1;

/**
 * Opens modal dialogs. The content is a standalone component that injects `DIALOG_REF`; the
 * `debrief-dialog-host` in the app shell renders the stack, traps focus and handles Escape.
 */
@Injectable({ providedIn: 'root' })
export class DialogService {
  readonly stack = signal<readonly OpenDialog[]>([]);

  open<R = void>(component: Type<unknown>, options: DialogOptions): DialogRef<R> {
    const id = nextId++;
    let resolve!: (r: R | undefined) => void;
    const closed = new Promise<R | undefined>((r) => (resolve = r));
    const ref: DialogRef<R> = {
      closed,
      close: (result?: R) => {
        if (!this.stack().some((d) => d.id === id)) return;
        if (options.canClose?.() === false) return;
        this.stack.update((s) => s.filter((d) => d.id !== id));
        resolve(result);
      },
    };
    this.stack.update((s) => [...s, { id, component, options, ref: ref as DialogRef }]);
    return ref;
  }

  /** Closes the topmost dialog (Escape), unless its `canClose` says no. */
  dismissTop(): void {
    this.stack().at(-1)?.ref.close();
  }
}
