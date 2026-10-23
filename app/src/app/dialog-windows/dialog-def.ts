import { Injector, Type } from '@angular/core';

/**
 * Whether a dialog blocks the window that opened it. Modal: the user must answer before going on (the parent is
 * disabled and dimmed). Modeless: a tool kept beside the work (the parent stays usable; opening it again focuses it).
 */
export type DialogModality = 'modal' | 'modeless';

/** A group of providers a dialog window loads on top of the base set. */
export type DialogFeature = 'ai' | 'tools' | 'share' | 'updates' | 'diagnostics' | 'analysis' | 'profiling' | 'native' | 'dotnet' | 'workspace' | 'summary' | 'skew';

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
  /**
   * Each distinct input is its own dialog (a clock adjustment per source, a narrative per event): opening the same
   * id with the same input focuses it, with another input opens another window. Without it, one window per id.
   */
  readonly instanced?: boolean;
  /**
   * A modal whose answer cannot be skipped because it has no Cancel (a choice between two ways forward): Escape, Cmd/Ctrl+W
   * and a window close request leave it open. Every other modal treats them as its Cancel.
   */
  readonly requiredChoice?: boolean;
  /** Feature providers the dialog's window needs besides the small base set (see `dialog-bootstrap.ts`). */
  readonly features?: readonly DialogFeature[];
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

/** A short stable key of an input value (FNV-1a over its JSON): the identity of an instanced dialog. */
export function instanceKey(inputs: unknown): string {
  let h = 0x811c9dc5;
  for (const ch of JSON.stringify(inputs) ?? '') {
    h ^= ch.charCodeAt(0);
    h = Math.imul(h, 0x01000193) >>> 0;
  }
  return h.toString(36);
}
