import { compassCells, edgeCells, GuideCell, hitGuide, Point, previewRect, Rect } from './dock-geometry';
import { DRAG_THRESHOLD } from './dock-target';

const ARROWS: Record<string, string> = { center: '', left: '◀', right: '▶', top: '▲', bottom: '▼' };
/** Where a panel or group drag starts: a tab or a group's header. */
const DRAG_SOURCES = '.dv-tab, .dv-void-container, .dv-tabs-and-actions-container';

/**
 * Draws Visual Studio's dock guides while a panel is dragged: a compass ("diamond") over the group under the
 * pointer and a guide at the middle of each edge of the dock. The guides are pictures only (no pointer events):
 * the same geometry (`compassCells`, `edgeCells`) drives dockview's `dropPositionResolver`, so a drop lands where
 * the highlighted guide says; dockview draws the preview of the result (styled as a dashed rectangle).
 *
 * Tab drags use pointer events (dockview's `dndStrategy: 'pointer'`), so they are followed here with pointer
 * events too. A pop-out window dragged back over the host has no pointer events in this document: the host calls
 * `showAt` / `hide` instead, and the preview rectangle is drawn here.
 */
/** A rect relative to `box` in the same coordinates as `box`. */
function offset(r: Rect, box: { left: number; top: number }): Rect {
  return { x: r.x + box.left, y: r.y + box.top, width: r.width, height: r.height };
}

export class DropGuides {
  private readonly layer = document.createElement('div');
  private readonly compass = document.createElement('div');
  private readonly edges = document.createElement('div');
  private readonly preview = document.createElement('div');
  private pressed: Point | null = null;
  private active = false;
  private group: HTMLElement | null = null;
  /** Driven by `showAt` (a pop-out over the window), which brings its own preview. */
  private external = false;

  constructor(private readonly root: HTMLElement) {
    this.layer.className = 'imt-drop-guides';
    this.layer.setAttribute('aria-hidden', 'true');
    this.compass.className = 'imt-guide-compass';
    this.edges.className = 'imt-guide-edges';
    this.preview.className = 'imt-guide-preview';
    this.preview.hidden = true;
    this.layer.append(this.preview, this.edges, this.compass);
    root.appendChild(this.layer);
    root.addEventListener('pointerdown', this.onDown, true);
    window.addEventListener('pointermove', this.onMove, true);
    window.addEventListener('pointerup', this.onEnd, true);
    window.addEventListener('pointercancel', this.onEnd, true);
  }

  private readonly onDown = (e: PointerEvent): void => {
    const t = e.target as Element | null;
    this.pressed = e.button === 0 && t?.closest?.(DRAG_SOURCES) ? { x: e.clientX, y: e.clientY } : null;
  };

  private readonly onMove = (e: PointerEvent): void => {
    if (!this.pressed) return;
    if (!this.active) {
      if (Math.hypot(e.clientX - this.pressed.x, e.clientY - this.pressed.y) < DRAG_THRESHOLD) return;
      this.start();
    }
    this.draw({ x: e.clientX, y: e.clientY });
  };

  private readonly onEnd = (): void => {
    this.pressed = null;
    this.hide();
  };

  private start(): void {
    this.active = true;
    this.layer.classList.add('on');
    this.drawEdges();
  }

  /** Shows the guides for a point (client coordinates) and, when given, the preview of the drop. */
  showAt(point: Point, preview: Rect | null = null): void {
    this.external = true;
    if (!this.active) this.start();
    this.draw(point);
    this.setPreview(preview);
  }

  hide(): void {
    this.external = false;
    if (!this.active) return;
    this.active = false;
    this.group = null;
    this.layer.classList.remove('on');
    this.preview.hidden = true;
    this.compass.replaceChildren();
    this.edges.replaceChildren();
  }

  private draw(p: Point): void {
    const group = (document.elementFromPoint?.(p.x, p.y) as Element | null)?.closest?.('.dv-groupview') as HTMLElement | null;
    const content = group && this.root.contains(group) ? (group.querySelector('.dv-content-container') as HTMLElement | null) : null;
    if (content !== this.group) {
      this.group = content;
      this.drawCompass();
    }
    this.highlight(p);
  }

  private drawEdges(): void {
    const box = this.root.getBoundingClientRect();
    this.edges.replaceChildren(...edgeCells(box.width, box.height).map((c) => this.cell(c, 0, 0)));
  }

  private drawCompass(): void {
    if (!this.group) {
      this.compass.replaceChildren();
      return;
    }
    const box = this.root.getBoundingClientRect();
    const g = this.group.getBoundingClientRect();
    this.compass.replaceChildren(...compassCells(g.width, g.height).map((c) => this.cell(c, g.left - box.left, g.top - box.top)));
  }

  private cell(c: GuideCell, dx: number, dy: number): HTMLElement {
    const el = document.createElement('div');
    el.className = `imt-guide imt-guide-${c.position}`;
    el.dataset['position'] = c.position;
    el.style.left = `${dx + c.rect.x}px`;
    el.style.top = `${dy + c.rect.y}px`;
    el.style.width = `${c.rect.width}px`;
    el.style.height = `${c.rect.height}px`;
    el.textContent = ARROWS[c.position];
    return el;
  }

  /** Highlights the guide under the pointer (the one a drop would use). */
  private highlight(p: Point): void {
    const box = this.root.getBoundingClientRect();
    const g = this.group?.getBoundingClientRect();
    // A group's compass wins over an edge guide it overlaps, as with dockview's nested drop targets.
    const inCompass = g ? hitGuide(compassCells(g.width, g.height), { x: p.x - g.left, y: p.y - g.top }) : null;
    const inEdges = inCompass ? null : hitGuide(edgeCells(box.width, box.height), { x: p.x - box.left, y: p.y - box.top });
    for (const el of this.compass.children) el.classList.toggle('hot', (el as HTMLElement).dataset['position'] === inCompass);
    for (const el of this.edges.children) el.classList.toggle('hot', (el as HTMLElement).dataset['position'] === inEdges);
    // dockview draws no overlay for a layout-edge drop over a group: preview it here.
    if (!this.external) this.setPreview(inEdges ? offset(previewRect(inEdges, { x: 0, y: 0, width: box.width, height: box.height }, 0.25), box) : null);
  }

  private setPreview(preview: Rect | null): void {
    const box = this.root.getBoundingClientRect();
    this.preview.hidden = !preview;
    if (!preview) return;
    this.preview.style.left = `${preview.x - box.left}px`;
    this.preview.style.top = `${preview.y - box.top}px`;
    this.preview.style.width = `${preview.width}px`;
    this.preview.style.height = `${preview.height}px`;
  }

  dispose(): void {
    this.onEnd();
    this.root.removeEventListener('pointerdown', this.onDown, true);
    window.removeEventListener('pointermove', this.onMove, true);
    window.removeEventListener('pointerup', this.onEnd, true);
    window.removeEventListener('pointercancel', this.onEnd, true);
    this.layer.remove();
  }
}
