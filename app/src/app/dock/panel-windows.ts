import { InjectionToken, Signal } from '@angular/core';
import { Point, Rect } from './dock-geometry';
import { PoppedOutPanel } from './layout-store';

/**
 * Panels popped out into OS windows of their own, as the dock sees them (implemented by the pop-out feature,
 * `PopoutService`). The dock depends on this port only, so it works without pop-outs (tests, a pop-out window).
 */
export interface PanelWindows {
  /** Ids of the panels currently shown in pop-out windows. */
  readonly poppedIds: Signal<ReadonlySet<string>>;
  /** False when pop-outs are impossible here or the per-window limit is reached. */
  canPopOut(): boolean;
  /**
   * Moves a docked panel into a window of its own: exactly at `rect` (where a dragged tab's ghost was), under the
   * screen point `at`, or where it belongs. The panel stays docked (with an inline error) on failure.
   */
  popOut(id: string, options?: { at?: Point; rect?: Rect }): Promise<boolean>;
  /**
   * A tab of `id` started a drag: the native preview of the window it would open (the shell's drag ghost), or null
   * where there is none (then the dock draws its own preview inside the window).
   */
  beginDragOut(id: string): DragOutPreview | null;
  /** Brings a popped-out panel's window to the front. */
  focus(id: string): void;
  /** Docks a popped-out panel back into this window. */
  dockBack(id: string): Promise<void>;
  /** Closes a popped-out panel (its window closes; nothing docks back). */
  discard(id: string): Promise<void>;
  /** The pop-outs to save with the layout. */
  records(): PoppedOutPanel[];
  /** Reopens saved pop-outs (on restore); panels that cannot be popped out dock instead. */
  restore(records: readonly PoppedOutPanel[]): Promise<void>;
}

/** The native preview of a tab dragged out of the window: a ghost window that follows the pointer outside it. */
export interface DragOutPreview {
  /** Does the ghost show the "new window" preview (false once it turned out it cannot: draw the in-window one)? */
  readonly native: boolean;
  /** The pointer is outside the window (show the ghost) or back inside (hide it). */
  show(outside: boolean): void;
  /** The drag ended (released or cancelled): hides the ghost; where it was when it showed (screen px), else null. */
  finish(): Promise<Rect | null>;
}

export const PANEL_WINDOWS = new InjectionToken<PanelWindows>('PANEL_WINDOWS');
