import { Point, Rect } from './dock-geometry';

/**
 * The cue shown while a tab is dragged off the dock's drop guides: outside the window a translucent ghost of the
 * panel follows the pointer (held at the window's edge) with "Release to open in a new window"; inside the window,
 * off every guide, a small "Release to cancel" next to the pointer while the dragged tab chip dims. Geometry and the
 * outcome are pure; `FloatCue` only draws them.
 */

/**
 * What releasing a dragged tab does where the pointer is: dock on a drop guide, open a new window outside the main
 * window, or nothing (the drag is cancelled and the tab snaps back). A drag never floats a panel inside the window;
 * that is done deliberately ("Float" in the tab menu, Window ▸ Float panel).
 */
export type TabDropOutcome = 'dock' | 'window' | 'cancel';

export interface Size {
  width: number;
  height: number;
}

export interface DropContext {
  /** The pointer is inside the window (client area). */
  inside: boolean;
  /** Over a dock drop zone: a guide of the compass or the dock's edges, or a tab strip. */
  overDropZone: boolean;
  /** Another OS window can be opened for it. */
  canPopOut: boolean;
}

/** The outcome of a release: dockview docks it on a drop zone; outside the window it pops out; elsewhere it is cancelled. */
export function tabDropOutcome(c: DropContext): TabDropOutcome {
  if (!c.inside) return c.canPopOut ? 'window' : 'cancel';
  return c.overDropZone ? 'dock' : 'cancel';
}

/** Smallest size of the new-window preview, and the size of a panel floated from a menu, px. */
export const FLOAT_MIN: Size = { width: 240, height: 160 };
export const FLOAT_MAX: Size = { width: 560, height: 380 };
/** Where the pointer holds the ghost, from its top-left corner (like a title bar; matches pop-out placement). */
const GRAB = { x: 120, y: 12 };

function clamp(v: number, lo: number, hi: number): number {
  return Math.min(Math.max(v, lo), Math.max(lo, hi));
}

/**
 * The size of the new-window preview: the panel's size, at most half the window each way (so it can follow the
 * pointer along the window's edge) and at least `FLOAT_MIN`.
 */
export function previewSize(panel: Size | null, viewport: Size): Size {
  const p = panel ?? FLOAT_MAX;
  return {
    width: Math.round(clamp(p.width, FLOAT_MIN.width, viewport.width / 2)),
    height: Math.round(clamp(p.height, FLOAT_MIN.height, viewport.height / 2)),
  };
}

/**
 * The ghost of the panel for a pointer at `cursor`: held near its top-left like a title bar, shrunk to fit and
 * clamped inside `bounds` (so it stays visible at the window's edge when the pointer is outside).
 */
export function ghostRect(cursor: Point, size: Size, bounds: Rect): Rect {
  const width = Math.max(0, Math.min(size.width, bounds.width));
  const height = Math.max(0, Math.min(size.height, bounds.height));
  const x = clamp(cursor.x - Math.min(GRAB.x, width / 2), bounds.x, bounds.x + bounds.width - width);
  const y = clamp(cursor.y - GRAB.y, bounds.y, bounds.y + bounds.height - height);
  return { x, y, width, height };
}

/** Where the "Release to cancel" hint sits: right of and below the pointer, under the dragged tab chip. */
const CANCEL_HINT = { x: 14, y: 46 };

/** The (zero-size) anchor of the "Release to cancel" hint for a pointer at `cursor`. */
export function cancelHintRect(cursor: Point): Rect {
  return { x: cursor.x + CANCEL_HINT.x, y: cursor.y + CANCEL_HINT.y, width: 0, height: 0 };
}

/** The hint for an outcome (null: no cue, dockview previews the drop on a guide). */
export function cueHint(outcome: TabDropOutcome): string | null {
  switch (outcome) {
    case 'window':
      return $localize`:@@dock.floatCue.window:Release to open in a new window`;
    case 'cancel':
      return $localize`:@@dock.floatCue.cancel:Release to cancel`;
    default:
      return null;
  }
}

/** The class on `<body>` while a release would cancel the drag: dims dockview's dragged tab chip. */
export const DRAG_CANCEL_CLASS = 'imt-drag-cancel';

/**
 * Draws the cue: a fixed layer over the whole window (no pointer events), plus a polite live region that announces
 * the hint when it changes. `show` with an outcome without a hint (or `hide`) clears it.
 */
export class FloatCue {
  private readonly ghost: HTMLElement;
  private readonly hint: HTMLElement;
  private readonly live: HTMLElement;
  private outcome: TabDropOutcome | null = null;

  constructor(private readonly doc: Document = document) {
    this.ghost = doc.createElement('div');
    this.ghost.className = 'imt-float-cue';
    this.ghost.setAttribute('aria-hidden', 'true');
    // An overlay while shown (`OverlayTracker`; hidden, it has no area).
    this.ghost.setAttribute('data-imt-overlay', '');
    this.ghost.hidden = true;
    this.hint = doc.createElement('div');
    this.hint.className = 'imt-float-cue-hint';
    this.ghost.append(this.hint);
    this.live = doc.createElement('div');
    this.live.className = 'sr-only';
    this.live.setAttribute('aria-live', 'polite');
    this.live.setAttribute('role', 'status');
    doc.body.append(this.ghost, this.live);
  }

  /** The outcome shown now (null when hidden). */
  get shown(): TabDropOutcome | null {
    return this.outcome;
  }

  show(outcome: TabDropOutcome, rect: Rect): void {
    const hint = cueHint(outcome);
    if (!hint) {
      this.hide();
      return;
    }
    if (outcome !== this.outcome) {
      this.outcome = outcome;
      this.ghost.dataset['outcome'] = outcome;
      this.hint.textContent = hint;
      this.live.textContent = hint;
      this.doc.body.classList.toggle(DRAG_CANCEL_CLASS, outcome === 'cancel');
    }
    this.ghost.hidden = false;
    this.ghost.style.left = `${rect.x}px`;
    this.ghost.style.top = `${rect.y}px`;
    this.ghost.style.width = `${rect.width}px`;
    this.ghost.style.height = `${rect.height}px`;
  }

  hide(): void {
    if (this.outcome === null) return;
    this.outcome = null;
    this.ghost.hidden = true;
    delete this.ghost.dataset['outcome'];
    this.live.textContent = '';
    this.doc.body.classList.remove(DRAG_CANCEL_CLASS);
  }

  dispose(): void {
    this.hide();
    this.ghost.remove();
    this.live.remove();
  }
}
