import { Point } from './dock-geometry';
import { moveTab, pressTab, releaseTab, TabDrag } from './dock-target';
import { tabPanelId } from './panel-tab';

/**
 * Pops a tab out when it is dragged out of the window and released there, using pointer events (works in
 * WKWebView and WebView2, where HTML5 drag-and-drop is unreliable or taken by the native file-drop handler).
 * Once a press on a tab becomes a drag, the pointer is captured, so the release is reported even outside the
 * window; a capture lost without a release counts as a release at the last position.
 */
export class TabDragOut {
  private drag: TabDrag | null = null;
  private captured: Element | null = null;

  constructor(
    private readonly root: HTMLElement,
    private readonly popOut: (id: string, at: Point) => void,
    private readonly viewport: () => { width: number; height: number } = () => ({ width: window.innerWidth, height: window.innerHeight }),
  ) {
    root.addEventListener('pointerdown', this.onDown, true);
    window.addEventListener('pointermove', this.onMove, true);
    window.addEventListener('pointerup', this.onUp, true);
    window.addEventListener('pointercancel', this.onCancel, true);
  }

  private readonly onDown = (e: PointerEvent): void => {
    const id = e.button === 0 ? tabPanelId(e.target) : null;
    this.release();
    this.drag = id ? pressTab(id, { x: e.clientX, y: e.clientY }, { x: e.screenX, y: e.screenY }) : null;
    if (id) this.captured = (e.target as Element).closest('[role="tab"]');
  };

  private readonly onMove = (e: PointerEvent): void => {
    if (!this.drag) return;
    const was = this.drag.dragging;
    this.drag = moveTab(this.drag, { x: e.clientX, y: e.clientY }, { x: e.screenX, y: e.screenY });
    if (!was && this.drag.dragging) this.capture(e.pointerId);
  };

  private readonly onUp = (e: PointerEvent): void => {
    this.finish({ x: e.clientX, y: e.clientY }, { x: e.screenX, y: e.screenY });
  };

  private readonly onCancel = (): void => {
    this.release();
    this.drag = null;
  };

  /** Capture lost without a pointerup (some webviews when the pointer leaves the window): release where it was. */
  private readonly onLostCapture = (): void => {
    if (this.drag) this.finish(this.drag.last, this.drag.lastScreen);
  };

  private finish(client: Point, screen: Point): void {
    const drag = this.drag;
    this.drag = null;
    this.release();
    const out = drag ? releaseTab(drag, client, screen, this.viewport()) : null;
    if (out) this.popOut(out.popOut, out.at);
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
    this.release();
    this.root.removeEventListener('pointerdown', this.onDown, true);
    window.removeEventListener('pointermove', this.onMove, true);
    window.removeEventListener('pointerup', this.onUp, true);
    window.removeEventListener('pointercancel', this.onCancel, true);
  }
}
