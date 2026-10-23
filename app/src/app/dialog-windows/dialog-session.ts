import { Type } from '@angular/core';
import { DialogDef } from './dialog-def';

/** What a backend needs to show a dialog. `inputs` must be structured-clonable: they cross to another window. */
export interface DialogShowRequest {
  /** The dialog's definition; absent for an overlay (command palette), which never gets a window. */
  readonly def: DialogDef | null;
  readonly component: Type<unknown>;
  readonly title: string;
  readonly inputs: Record<string, unknown>;
  readonly width: number;
  /** Identifies this input among the open windows of an instanced dialog. */
  readonly instance?: string;
  /** In-page only: asked before closing. A window asks its component (`dialogCanClose`) itself. */
  readonly canClose?: () => boolean;
}

/** An open dialog, however it is shown. */
export interface DialogSession {
  /** Resolves when the dialog is gone, with its result (undefined when dismissed). Never rejects. */
  readonly closed: Promise<unknown>;
  /** False once the dialog is gone, before `closed` has been observed (a dialog closed and reopened at once). */
  readonly isOpen?: () => boolean;
  /** Brings the dialog to the front (opening it again). */
  focus(): void;
  /** Asks the dialog to close with a result; a window may refuse (unsaved edits). */
  close(result?: unknown): void;
}

/** A yes/no question for the native message box. */
export interface ConfirmQuestion {
  title: string;
  message: string;
  ok: string;
  cancel: string;
  danger: boolean;
}
