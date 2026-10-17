import { ChangeDetectionStrategy, Component, ElementRef, inject } from '@angular/core';
import { WorkspaceStore } from '../core/workspace.store';
import { EventsGrid } from '../grid/events-grid';
import { TimelinePanel } from '../timeline/timeline-panel';
import { FilterBar } from './filter-bar';

const MIN_HEIGHT = 120;
const GRID_RESERVE = 200;

/** Clamps a requested timeline height to [MIN_HEIGHT, max]. */
export function clampHeight(h: number, max: number): number {
  return Math.max(MIN_HEIGHT, Math.min(max, h));
}

/** The main tab: filters, per-instance timeline, a draggable splitter and the event grid. */
@Component({
  selector: 'imt-timeline-view',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [FilterBar, TimelinePanel, EventsGrid],
  template: `
    <imt-filter-bar />
    <imt-timeline-panel id="imt-timeline-panel" [style.height.px]="store.timelineHeight()" />
    <div
      class="splitter"
      role="separator"
      aria-orientation="horizontal"
      aria-label="Resize timeline"
      aria-controls="imt-timeline-panel"
      [attr.aria-valuenow]="store.timelineHeight()"
      [attr.aria-valuemin]="minHeight"
      [attr.aria-valuemax]="maxHeight()"
      tabindex="0"
      (pointerdown)="startDrag($event)"
      (keydown.arrowup)="nudge(-24, $event)"
      (keydown.arrowdown)="nudge(24, $event)"
      (keydown.home)="setHeight(minHeight, $event)"
      (keydown.end)="setHeight(maxHeight(), $event)"
    ></div>
    <imt-events-grid />
  `,
  styles: `
    :host { flex: 1; min-height: 0; display: flex; flex-direction: column; }
    imt-timeline-panel { flex: none; min-height: 120px; }
    imt-events-grid { flex: 1; min-height: 120px; }
    .splitter { height: 5px; flex: none; cursor: row-resize; background: var(--border); border-top: 2px solid var(--bg-1); border-bottom: 2px solid var(--bg-1); }
    .splitter:hover, .splitter:focus-visible { background: var(--accent); }
  `,
})
export class TimelineView {
  protected readonly store = inject(WorkspaceStore);
  private readonly host = inject<ElementRef<HTMLElement>>(ElementRef);

  protected readonly minHeight = MIN_HEIGHT;

  /** Largest timeline height that still leaves room for the grid. */
  protected maxHeight(): number {
    return Math.max(MIN_HEIGHT, this.host.nativeElement.clientHeight - GRID_RESERVE);
  }

  protected startDrag(e: PointerEvent): void {
    const el = e.target as HTMLElement;
    el.setPointerCapture(e.pointerId);
    const startY = e.clientY;
    const startH = this.store.timelineHeight();
    const max = this.maxHeight();
    const move = (ev: PointerEvent) => this.store.setTimelineHeight(clampHeight(startH + ev.clientY - startY, max));
    const up = () => {
      el.removeEventListener('pointermove', move);
      el.removeEventListener('pointerup', up);
    };
    el.addEventListener('pointermove', move);
    el.addEventListener('pointerup', up);
  }

  protected nudge(dy: number, e?: Event): void {
    e?.preventDefault();
    this.store.setTimelineHeight(clampHeight(this.store.timelineHeight() + dy, this.maxHeight()));
  }

  protected setHeight(h: number, e?: Event): void {
    e?.preventDefault();
    this.store.setTimelineHeight(clampHeight(h, this.maxHeight()));
  }
}
