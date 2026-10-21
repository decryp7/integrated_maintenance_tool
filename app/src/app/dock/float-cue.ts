import { Point, Rect } from './dock-geometry';

/**
 * The cue shown while a tab is dragged off the dock's drop zones: a ghost of the panel where it will appear and a
 * "Release to float" (inside the window) or "Release to open in a new window" (outside it) hint. Geometry and the
 * outcome are pure; `FloatCue` only draws them.
 */

/** What releasing a dragged tab does where the pointer is. */
export type TabDropOutcome = 'dock' | 'float' | 'window' | 'none';

export interface Size {
  width: number;
  height: number;
}

export interface DropContext {
  /** The pointer is inside the window (client area). */
  inside: boolean;
  /** Over a dock drop zone: a guide of the compass or the dock's edges, or a tab strip. */
  overDropZone: boolean;
  /** The panel can float (it is docked, or one of several tabs of a floating group). */
  canFloat: boolean;
  /** Another OS window can be opened for it. */
  canPopOut: boolean;
}

/** The outcome of a release: dockview docks it on a drop zone; elsewhere IMT floats it or opens it in a window. */
export function tabDropOutcome(c: DropContext): TabDropOutcome {
  if (!c.inside) return c.canPopOut ? 'window' : 'none';
  if (c.overDropZone) return 'dock';
  return c.canFloat ? 'float' : 'none';
}

/** Smallest and largest size of a panel floated by dragging, px (a big docked panel floats at a usable size). */
export const FLOAT_MIN: Size = { width: 240, height: 160 };
export const FLOAT_MAX: Size = { width: 560, height: 380 };
/** Where the pointer holds the ghost, from its top-left corner (like a title bar; matches pop-out placement). */
const GRAB = { x: 120, y: 12 };

function clamp(v: number, lo: number, hi: number): number {
  return Math.min(Math.max(v, lo), Math.max(lo, hi));
}

/** The size a dragged panel floats at: its docked size, within `FLOAT_MIN`..`FLOAT_MAX`. */
export function floatSize(panel: Size | null): Size {
  const w = panel?.width ?? FLOAT_MAX.width;
  const h = panel?.height ?? FLOAT_MAX.height;
  return { width: Math.round(clamp(w, FLOAT_MIN.width, FLOAT_MAX.width)), height: Math.round(clamp(h, FLOAT_MIN.height, FLOAT_MAX.height)) };
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

/** The hint for an outcome that shows the cue (null: no cue). */
export function cueHint(outcome: TabDropOutcome): string | null {
  switch (outcome) {
    case 'float':
      return $localize`:@@dock.floatCue.float:Release to float`;
    case 'window':
      return $localize`:@@dock.floatCue.window:Release to open in a new window`;
    default:
      return null;
  }
}

/**
 * Draws the cue: a fixed layer over the whole window (no pointer events), plus a polite live region that announces
 * the hint when it changes. `show` with a non-cue outcome (or `hide`) clears it.
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
  }

  dispose(): void {
    this.ghost.remove();
    this.live.remove();
  }
}
