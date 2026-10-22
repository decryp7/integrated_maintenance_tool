import { Point, Rect } from './dock-geometry';
import { TabDropOutcome } from './float-cue';
import { DragOutPreview } from './panel-windows';

/** What the cue of a dragged tab shows for an outcome: the shell's ghost window, the in-window cue, both or neither. */
export interface CueMode {
  /** The native ghost follows the pointer outside the window. */
  ghost: boolean;
  /** The in-window cue is drawn (the "Release to cancel" hint, or the new-window preview when there is no ghost). */
  drawn: boolean;
}

/**
 * The cue for an outcome: outside the window the native ghost shows the new window when there is one (the in-window
 * preview stays hidden, its hint is still announced), else the in-window preview; inside, off the drop zones, the
 * "Release to cancel" hint; on a drop zone dockview previews the drop itself.
 */
export function cueMode(outcome: TabDropOutcome | null, nativeGhost: boolean): CueMode {
  switch (outcome) {
    case 'window':
      return { ghost: nativeGhost, drawn: !nativeGhost };
    case 'cancel':
      return { ghost: false, drawn: true };
    default:
      return { ghost: false, drawn: false };
  }
}

/** The in-window cue (`FloatCue`). */
export interface InWindowCue {
  show(outcome: TabDropOutcome, rect: Rect, drawn?: boolean): void;
  hide(): void;
}

/** What the drag cue needs from the dock. */
export interface DragOutCueHost {
  /** Where the in-window cue goes for this outcome (null: none). */
  cueRect(id: string, p: Point, outcome: TabDropOutcome | null): Rect | null;
  /** The native preview for a drag of `id` (null: there is none here). */
  beginDragOut(id: string): DragOutPreview | null;
}

/**
 * The cue of one tab drag at a time: the native ghost outside the window (begun with the drag, shown while the
 * pointer is outside, ended with the drag) and the in-window cue. When the drag ends, where the ghost was is kept
 * for the pop-out that follows a release outside (`takeEnd`).
 */
export class DragOutCue {
  private preview: DragOutPreview | null = null;
  private dragged: string | null = null;
  private ending: Promise<Rect | null> | null = null;

  constructor(
    private readonly cue: InWindowCue,
    private readonly host: DragOutCueHost,
  ) {}

  /** The drag of `id` is at `p` with this outcome; null: it ended (released or cancelled). */
  update(id: string, p: Point, outcome: TabDropOutcome | null): void {
    if (outcome === null) {
      this.ending = this.preview?.finish() ?? null;
      this.preview = null;
      this.dragged = null;
      this.cue.hide();
      return;
    }
    if (this.dragged !== id) {
      void this.preview?.finish();
      this.ending = null;
      this.dragged = id;
      this.preview = this.host.beginDragOut(id);
    }
    const mode = cueMode(outcome, this.preview?.native ?? false);
    this.preview?.show(mode.ghost);
    const rect = this.host.cueRect(id, p, outcome);
    if (rect) this.cue.show(outcome, rect, mode.drawn);
    else this.cue.hide();
  }

  /**
   * Where the ghost of the drag that just ended was (null: it was not showing), then forgotten; null right away when
   * the drag had no ghost, so a release without one pops out at once.
   */
  takeEnd(): Promise<Rect | null> | null {
    const ending = this.ending;
    this.ending = null;
    return ending;
  }

  dispose(): void {
    void this.preview?.finish();
    this.preview = null;
    this.dragged = null;
    this.ending = null;
  }
}
