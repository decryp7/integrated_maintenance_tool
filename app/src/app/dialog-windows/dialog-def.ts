import { Injector, Type } from '@angular/core';

/**
 * Whether a dialog blocks the window that opened it. Modal: the user must answer before going on (the parent is
 * disabled and dimmed). Modeless: a tool kept beside the work (the parent stays usable; opening it again focuses it).
 */
export type DialogModality = 'modal' | 'modeless';

/** One dialog of Debrief (ADR 0050): its id, modality and how its content is loaded in its own window. */
export interface DialogDef {
  /** Stable id (`ai-setup`): the dialog window's route, its remembered bounds' key, its test id. */
  readonly id: string;
  readonly modality: DialogModality;
  /** Preferred content width in px. */
  readonly width: number;
  readonly height?: number;
  readonly minWidth?: number;
  readonly minHeight?: number;
  /** The content component, loaded when the dialog window opens (the host never loads it). */
  readonly load: () => Promise<Type<unknown>>;
}

/** What a dialog component declares about itself: its id (and, optionally, whether it may close now). */
export interface DialogComponent {
  readonly dialogId?: string;
  /** Asked before the dialog closes by any route; false keeps it open (unsaved edits). Runs in the dialog's window. */
  dialogCanClose?(injector: Injector): boolean;
}

/** The dialog id a component class declares, if any. */
export function dialogIdOf(component: Type<unknown>): string | undefined {
  const id = (component as unknown as DialogComponent).dialogId;
  return typeof id === 'string' && id ? id : undefined;
}
