import { Point } from './dock-geometry';

/** The part of a floating window that moves it: the empty part of its tab strip (`floatingGroupDragHandle: 'tabbar'`). */
const MOVE_HANDLE = '.dv-resize-container .dv-void-container';

export type PressLike = Pick<PointerEvent, 'button' | 'shiftKey' | 'pointerType'>;

/**
 * Does this press move a floating window? A primary mouse press on its move handle does; Shift+drag redocks the group
 * and touch or pen go through dockview's long-press gesture, so those stay dockview's.
 */
export function isFloatingMovePress(e: PressLike, target: Element | null): boolean {
  return e.button === 0 && !e.shiftKey && e.pointerType === 'mouse' && !!target?.closest?.(MOVE_HANDLE);
}

/**
 * Lets a floating window be moved with the mouse. With `dndStrategy: 'pointer'`, dockview arms both the window move
 * and a redock drag of the group on a press on the move handle, and whichever sees the first pointer movement wins:
 * the redock drag listens first, so any first movement of 5 px or more (a quick drag, a coarse trackpad) started a
 * redock instead and the window seemed stuck. dockview cancels the redock once the window starts moving, so this
 * hands the window move one movement before the real one, where the pointer was pressed (it moves nothing).
 */
export class FloatingMove {
  private pressed: { id: number; at: Point } | null = null;
  private priming = false;

  constructor(
    private readonly root: HTMLElement,
    private readonly win: Window = window,
  ) {
    root.addEventListener('pointerdown', this.onDown, true);
    win.addEventListener('pointermove', this.onMove, true);
    win.addEventListener('pointerup', this.onEnd, true);
    win.addEventListener('pointercancel', this.onEnd, true);
  }

  private readonly onDown = (e: PointerEvent): void => {
    this.pressed = isFloatingMovePress(e, e.target instanceof Element ? e.target : null) ? { id: e.pointerId, at: { x: e.clientX, y: e.clientY } } : null;
  };

  private readonly onMove = (e: PointerEvent): void => {
    const pressed = this.pressed;
    if (this.priming || !pressed || e.pointerId !== pressed.id) return;
    this.pressed = null;
    this.priming = true;
    try {
      this.win.dispatchEvent(new PointerEvent('pointermove', { pointerId: pressed.id, pointerType: 'mouse', isPrimary: true, buttons: 1, clientX: pressed.at.x, clientY: pressed.at.y, bubbles: true }));
    } catch {
      /* no PointerEvent constructor: dockview's own order applies */
    } finally {
      this.priming = false;
    }
  };

  private readonly onEnd = (): void => {
    this.pressed = null;
  };

  dispose(): void {
    this.pressed = null;
    this.root.removeEventListener('pointerdown', this.onDown, true);
    this.win.removeEventListener('pointermove', this.onMove, true);
    this.win.removeEventListener('pointerup', this.onEnd, true);
    this.win.removeEventListener('pointercancel', this.onEnd, true);
  }
}
