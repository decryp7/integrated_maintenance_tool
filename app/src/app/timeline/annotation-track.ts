import { ChangeDetectionStrategy, Component, input, output } from '@angular/core';
import { SeverityBadge } from '../live/severity-badge';
import { AnnotationVm } from './annotation-layout';

/** Pointer position and the annotation under it, for the timeline's tooltip. */
export interface AnnotationHover {
  x: number;
  y: number;
  vm: AnnotationVm;
}

/**
 * The annotations of one lane (or the full-height ones): translucent bands for spans and glyphs for points.
 * Severity is shown by an icon (octagon, triangle, circle) and a fill pattern (hatched, dotted, plain), never by
 * colour alone. Clicking applies the annotation's evidence query; brushing across a band still zooms.
 *
 * Bands and glyphs are pointer shortcuts and stay out of the tab order: event markers sit on top of them (so a band
 * may be partly covered), and the timeline legend's Evidence menu lists every annotation in view, which is the
 * keyboard path and the equivalent control for WCAG 2.5.8 (target size).
 */
@Component({
  selector: 'imt-annotation-track',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [SeverityBadge],
  template: `
    @for (v of items(); track v.id) {
      <button
        type="button"
        tabindex="-1"
        class="ann"
        [class.point]="v.point"
        [class]="'sev-' + v.ann.severity"
        [attr.data-kind]="v.ann.kind"
        [style.left.%]="v.left"
        [style.width.%]="v.point ? null : v.width"
        [attr.aria-label]="ariaLabel(v)"
        (click)="activated.emit(v)"
        (focus)="hovered.emit({ x: $any($event.target).getBoundingClientRect().left, y: $any($event.target).getBoundingClientRect().bottom, vm: v })"
        (blur)="hovered.emit(null)"
        (pointerenter)="hovered.emit({ x: $event.clientX, y: $event.clientY, vm: v })"
        (pointerleave)="hovered.emit(null)"
      >
        <span class="mark"><imt-severity-badge [severity]="v.ann.severity" [showLabel]="false" [size]="v.point ? 14 : 11" /></span>
      </button>
    }
  `,
  styles: `
    :host { position: absolute; inset: 0; pointer-events: none; }
    .ann { position: absolute; top: 2px; bottom: 2px; padding: 0 0 0 2px; margin: 0; display: flex; align-items: flex-start; justify-content: flex-start; border: 1px solid var(--ann-border); border-radius: 3px; background-color: var(--ann-fill); color: var(--text-2); cursor: pointer; pointer-events: auto; overflow: hidden; z-index: 0; }
    /* Event markers (z-index 1) stay on top of annotations, so a crash glyph never hides the crash marker. */
    .ann:hover { border-color: var(--accent); }
    .ann:focus-visible { border-color: var(--accent); z-index: 2; }
    .ann.sev-critical { --ann-fill: var(--ann-critical); --ann-border: var(--ann-critical-border); background-image: repeating-linear-gradient(135deg, var(--ann-critical-border) 0 1px, transparent 1px 6px); }
    .ann.sev-warning { --ann-fill: var(--ann-warning); --ann-border: var(--ann-warning-border); background-image: radial-gradient(var(--ann-warning-border) 0.8px, transparent 1px); background-size: 5px 5px; }
    .ann.sev-info { --ann-fill: var(--ann-info); --ann-border: var(--ann-info-border); }
    .mark { display: inline-flex; }
    /* A point is a transparent 24px hit area (WCAG 2.5.8) holding the visible 18px circle at the lane's top. */
    /* Points sit above bands (a crash inside a downtime band stays clickable) and below event markers. */
    .ann.point { width: 24px; margin-left: -12px; padding: 0; top: 0; bottom: auto; height: 24px; justify-content: center; border: 0; border-radius: 0; background: transparent; overflow: visible; z-index: 1; }
    .ann.point .mark { width: 18px; height: 18px; box-sizing: border-box; align-items: center; justify-content: center; border: 1px solid var(--ann-border); border-radius: 50%; background-color: var(--bg-1); }
    .ann.point:hover .mark, .ann.point:focus-visible .mark { border-color: var(--accent); }
    .ann.point:focus-visible { outline: none; }
    .ann.point:focus-visible .mark { outline: 2px solid var(--focus-ring); outline-offset: 1px; }
  `,
})
export class AnnotationTrack {
  readonly items = input.required<readonly AnnotationVm[]>();
  readonly hovered = output<AnnotationHover | null>();
  readonly activated = output<AnnotationVm>();

  protected ariaLabel(v: AnnotationVm): string {
    const { label, severity, owner } = v.ann;
    return $localize`:@@timeline.annotationTrack.aria:${label}:label: (${severity}:severity:, ${owner}:owner:) — show the evidence`;
  }
}
