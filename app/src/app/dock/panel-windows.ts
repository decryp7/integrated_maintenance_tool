import { InjectionToken, Signal } from '@angular/core';
import { Point } from './dock-geometry';
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
  /** Moves a docked panel into a window of its own; the panel stays docked (with an inline error) on failure. */
  popOut(id: string, options?: { at?: Point }): Promise<boolean>;
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

export const PANEL_WINDOWS = new InjectionToken<PanelWindows>('PANEL_WINDOWS');
