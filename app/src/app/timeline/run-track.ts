import { ChangeDetectionStrategy, Component, input, output } from '@angular/core';
import { RunBandVm } from './run-layout';
import { runStatusLabel } from './run-text';

/** Pointer position and the run under it, for the timeline's tooltip. */
export interface RunHover {
  x: number;
  y: number;
  vm: RunBandVm;
}

/**
 * The runs of one lane: translucent bands from start to stop, drawn behind the lane's markers. The status is shown
 * by shape and pattern, never by colour alone: a stopped run has solid ends; a run still running has no right end, fades
 * out and says "still running"; a run without a stop is hatched with a warning glyph at its end; a stop without a start
 * is a warning glyph; a run over its maximum duration gets a clock-warning glyph. Clicking filters to the run.
 *
 * Bands are pointer shortcuts and stay out of the tab order (markers sit on top of them); the timeline legend's
 * Evidence menu lists every run in view, which is the keyboard path and the equivalent control (WCAG 2.5.8).
 */
@Component({
  selector: 'imt-run-track',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    @for (v of items(); track v.id) {
      <button
        type="button"
        tabindex="-1"
        class="run"
        [class]="'st-' + v.status"
        [class.point]="v.point"
        [class.clip-l]="v.clippedLeft"
        [class.clip-r]="v.clippedRight"
        [attr.data-kind]="v.ann.kind"
        [attr.data-status]="v.status"
        [style.left.%]="v.left"
        [style.width.%]="v.point ? null : v.width"
        [style.--run-color]="v.run.color"
        [attr.aria-label]="ariaLabel(v)"
        (click)="activated.emit(v)"
        (pointerenter)="hovered.emit({ x: $event.clientX, y: $event.clientY, vm: v })"
        (pointerleave)="hovered.emit(null)"
      >
        @if (v.anomaly) {
          <span class="warn" [class]="'warn a-' + v.anomaly" aria-hidden="true">
            <svg viewBox="0 0 16 16" width="14" height="14">
              @if (v.anomaly === 'tooLong') {
                <circle cx="8" cy="8" r="6.5" class="glyph-bg" /><path d="M8 4.5V8l2.5 1.5" class="glyph-fg" />
              } @else {
                <path d="M8 1.5 15 14.5H1z" class="glyph-bg" /><path d="M8 6v4M8 11.6v.9" class="glyph-fg" />
              }
            </svg>
          </span>
        }
      </button>
      @if (v.status === 'open' && !v.clippedRight) {
        <!-- Outside the button, so its accessible name stays the label (WCAG 2.5.3); the status is in that name too. -->
        <span class="still" [style.right.%]="100 - v.left - v.width" aria-hidden="true" i18n="Label at the open end of a run band@@runs.track.stillRunning">still running</span>
      }
    }
  `,
  styles: `
    :host { position: absolute; inset: 0; pointer-events: none; }
    .run {
      --run-color: var(--accent);
      position: absolute; top: 5px; bottom: 5px; margin: 0; padding: 0; display: flex; align-items: center; justify-content: flex-end;
      border: 1.5px solid color-mix(in srgb, var(--run-color) 75%, transparent); border-radius: 4px;
      background: color-mix(in srgb, var(--run-color) 16%, transparent); cursor: pointer; pointer-events: auto; overflow: visible; z-index: 0;
    }
    .run:hover { background: color-mix(in srgb, var(--run-color) 26%, transparent); }
    /* Ends outside the view are open (no border) so a clipped run never looks stopped. */
    .run.clip-l { border-left-style: none; border-top-left-radius: 0; border-bottom-left-radius: 0; }
    .run.clip-r { border-right-style: none; border-top-right-radius: 0; border-bottom-right-radius: 0; }
    /* Still running: no right end, fading out, with a label. */
    .run.st-open { border-right-style: none; border-top-right-radius: 0; border-bottom-right-radius: 0; background: linear-gradient(90deg, color-mix(in srgb, var(--run-color) 18%, transparent) 55%, transparent); border-image: linear-gradient(90deg, color-mix(in srgb, var(--run-color) 75%, transparent) 55%, transparent) 1; }
    /* No stop seen: hatched, ending in a warning glyph. */
    .run.st-noStop { border-style: dashed; background-image: repeating-linear-gradient(135deg, color-mix(in srgb, var(--run-color) 35%, transparent) 0 1px, transparent 1px 6px); }
    .still { position: absolute; top: 9px; margin-right: 4px; padding: 0 3px; font-size: var(--fs-2xs); line-height: 14px; white-space: nowrap; color: var(--text-2); background: var(--bg-1); border-radius: 3px; pointer-events: none; z-index: 0; }
    .warn { position: absolute; right: -8px; top: 50%; width: 16px; height: 16px; margin-top: -8px; display: flex; align-items: center; justify-content: center; pointer-events: none; }
    .glyph-bg { fill: var(--bg-1); stroke: var(--warn-text); stroke-width: 1.4; stroke-linejoin: round; }
    .glyph-fg { fill: none; stroke: var(--warn-text); stroke-width: 1.6; stroke-linecap: round; }
    /* A stop without start is a 24px hit area (WCAG 2.5.8) around its glyph. */
    .run.point { top: 4px; bottom: auto; width: 24px; height: 24px; margin-left: -12px; border: 0; background: transparent; justify-content: center; z-index: 1; }
    .run.point .warn { position: static; margin: 0; }
  `,
})
export class RunTrack {
  readonly items = input.required<readonly RunBandVm[]>();
  readonly hovered = output<RunHover | null>();
  readonly activated = output<RunBandVm>();

  protected ariaLabel(v: RunBandVm): string {
    const label = v.run.spanLabel;
    const status = runStatusLabel(v.status);
    return $localize`:@@runs.track.aria:${label}:label: (${status}:status:) — show this run`;
  }
}
