import { Point } from './dock-geometry';
import { moveTab, pressTab, TabDrag } from './dock-target';
import { TabDropOutcome } from './float-cue';
import { tabPanelId } from './panel-tab';

/** What a tab drag off the dock's drop zones does, provided by the dock. */
export interface TabDragHost {
  /** What releasing the dragged tab at this point (client coordinates) would do. */
  outcome(id: string, client: Point): TabDropOutcome;
  /** Opens the panel in its own window at a screen point. */
  popOut(id: string, screen: Point): void;
  /** Floats the panel in the window at a client point. */
  float(id: string, client: Point): void;
  /** Shows the cue for this outcome at the pointer; `none` and `dock` clear it. */
  cue(id: string, client: Point, outcome: TabDropOutcome): void;
}

/**
 * Follows a tab dragged off the dock's drop zones, using pointer events (works in WKWebView and WebView2, where
 * HTML5 drag-and-drop is unreliable or taken by the native file-drop handler): released outside the window it pops
 * out, released inside but away from every drop zone it floats there; dockview handles drops on the zones. While
 * dragging, the host shows a cue of what a release would do. Escape cancels the drag (dockview's too).
 *
 * Once a press on a tab becomes a drag, the pointer is captured, so the release is reported even outside the
 * window; a capture lost without a release counts as a release at the last position.
 */
export class TabDragOut {
  private drag: TabDrag | null = null;
  private pointerId = 0;
  private captured: Element | null = null;

  constructor(
    private readonly root: HTMLElement,
    private readonly host: TabDragHost,
  ) {
    root.addEventListener('pointerdown', this.onDown, true);
    window.addEventListener('pointermove', this.onMove, true);
    window.addEventListener('pointerup', this.onUp, true);
    window.addEventListener('pointercancel', this.onCancel, true);
    window.addEventListener('keydown', this.onKey, true);
  }

  private readonly onDown = (e: PointerEvent): void => {
    const id = e.button === 0 ? tabPanelId(e.target) : null;
    this.end();
    this.drag = id ? pressTab(id, { x: e.clientX, y: e.clientY }, { x: e.screenX, y: e.screenY }) : null;
    this.pointerId = e.pointerId;
    if (id) this.captured = (e.target as Element).closest('[role="tab"]');
  };

  private readonly onMove = (e: PointerEvent): void => {
    if (!this.drag) return;
    const was = this.drag.dragging;
    this.drag = moveTab(this.drag, { x: e.clientX, y: e.clientY }, { x: e.screenX, y: e.screenY });
    if (!this.drag.dragging) return;
    if (!was) this.capture(e.pointerId);
    this.host.cue(this.drag.id, this.drag.last, this.host.outcome(this.drag.id, this.drag.last));
  };

  private readonly onUp = (e: PointerEvent): void => {
    this.finish({ x: e.clientX, y: e.clientY }, { x: e.screenX, y: e.screenY });
  };

  private readonly onCancel = (): void => {
    this.end();
  };

  /** Escape while dragging cancels: nothing floats or pops out, and dockview's drag ends without a drop. */
  private readonly onKey = (e: KeyboardEvent): void => {
    if (e.key !== 'Escape' || !this.drag?.dragging) return;
    e.preventDefault();
    e.stopPropagation();
    this.end();
    try {
      window.dispatchEvent(new PointerEvent('pointercancel', { pointerId: this.pointerId, bubbles: true }));
    } catch {
      /* no PointerEvent constructor: dockview's drag ends on release, without a drop target highlighted by us */
    }
  };

  /** Capture lost without a pointerup (some webviews when the pointer leaves the window): release where it was. */
  private readonly onLostCapture = (): void => {
    if (this.drag) this.finish(this.drag.last, this.drag.lastScreen);
  };

  private finish(client: Point, screen: Point): void {
    const drag = this.drag ? moveTab(this.drag, client, screen) : null;
    this.end();
    if (!drag?.dragging) return;
    const outcome = this.host.outcome(drag.id, client);
    if (outcome === 'window') this.host.popOut(drag.id, screen);
    else if (outcome === 'float') this.host.float(drag.id, client);
  }

  /** Forgets the drag and clears the cue. */
  private end(): void {
    const drag = this.drag;
    this.drag = null;
    this.release();
    if (drag?.dragging) this.host.cue(drag.id, drag.last, 'none');
  }

  private capture(pointerId: number): void {
    const el = this.captured as (Element & { setPointerCapture?: (id: number) => void }) | null;
    if (!el?.setPointerCapture) return;
    try {
      el.setPointerCapture(pointerId);
      el.addEventListener('lostpointercapture', this.onLostCapture, { once: true });
    } catch {
      /* the pointer is gone already: the window listeners still see the release when it happens inside */
    }
  }

  private release(): void {
    this.captured?.removeEventListener('lostpointercapture', this.onLostCapture);
    this.captured = null;
  }

  dispose(): void {
    this.end();
    this.root.removeEventListener('pointerdown', this.onDown, true);
    window.removeEventListener('pointermove', this.onMove, true);
    window.removeEventListener('pointerup', this.onUp, true);
    window.removeEventListener('pointercancel', this.onCancel, true);
    window.removeEventListener('keydown', this.onKey, true);
  }
}
