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
 * The runs of one lane, on stacked sub-tracks below the lane's markers (`row` from `layoutRuns`), each labelled with
 * what it is (a view, a job, "Running") where it is wide enough. The theme's `runs` trait draws them as a line (calm),
 * a pill (console) or a bracket. The status is shown by shape and pattern, never by colour alone: a stopped run has
 * closed ends; a run still running has no right end, fades out and says "still running"; a run without a stop is dashed
 * with a warning glyph at its end; a stop without a start is a warning glyph; a run over its maximum duration gets a
 * clock-warning glyph. Ends outside the view stay open. Clicking filters to the run.
 *
 * Bands are pointer shortcuts and stay out of the tab order (markers sit above them); the timeline legend's Evidence
 * menu lists every run in view, which is the keyboard path and the equivalent control (WCAG 2.5.8).
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
        [attr.data-row]="v.row"
        [style.--row]="v.row"
        [style.left.%]="v.left"
        [style.width.%]="v.point ? null : v.width"
        [style.--run-color]="v.run.color ?? laneColor()"
        [attr.aria-label]="ariaLabel(v)"
        (click)="activated.emit(v)"
        (pointerenter)="hovered.emit({ x: $event.clientX, y: $event.clientY, vm: v })"
        (pointerleave)="hovered.emit(null)"
      >
        @if (v.labelFits) {
          <span class="run-label" aria-hidden="true">{{ v.label }}</span>
        }
        @if (v.anomaly) {
          <span class="warn" [class]="'warn a-' + v.anomaly" aria-hidden="true">
            <svg viewBox="0 0 16 16" width="12" height="12">
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
        <span class="still" [style.--row]="v.row" [style.right.%]="100 - v.left - v.width" aria-hidden="true" i18n="Label at the open end of a run band@@runs.track.stillRunning">still running</span>
      }
    }
  `,
  styles: `
    :host { position: absolute; left: 0; right: 0; top: var(--lane-h); bottom: 0; pointer-events: none; }
    .run {
      --run-color: var(--accent);
      --row: 0;
      position: absolute; top: calc(var(--row) * var(--run-row-h)); height: var(--run-row-h); min-width: 2px; margin: 0; padding: 0 3px 3px;
      display: flex; align-items: center; gap: 4px; border: 0; border-radius: 0; background: transparent; color: var(--text-2);
      font: inherit; font-size: var(--fs-2xs); line-height: 1; cursor: pointer; pointer-events: auto; overflow: visible; z-index: 0;
    }
    .run-label { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
    .run:hover { color: var(--text); }

    /* runs: line (the default). A 2 px rule at the bottom of the sub-track with end caps; the label sits above it. */
    .run::before { content: ''; position: absolute; left: 0; right: 0; bottom: 1px; height: 2px; border-radius: 1px; background: var(--run-color); }
    .run::after { content: ''; position: absolute; left: 0; right: 0; bottom: 0; height: 6px; border-left: 1.5px solid var(--run-color); border-right: 1.5px solid var(--run-color); }
    .run:hover::before { height: 3px; }
    .run.clip-l::after { border-left: 0; }
    .run.clip-r::after { border-right: 0; }
    /* Still running: no right end, fading out, with a label. */
    .run.st-open::before { background: linear-gradient(90deg, var(--run-color) 55%, transparent); }
    .run.st-open::after { border-right: 0; }
    /* No stop seen: dashed, ending in a warning glyph. */
    .run.st-noStop::before { background: repeating-linear-gradient(90deg, var(--run-color) 0 5px, transparent 5px 8px); }
    .run.st-noStop::after { border-right: 0; }

    /* runs: pill. A rounded band in the run's colour with the label inside. */
    :host-context([data-runs='pill']) .run { top: calc(var(--row) * var(--run-row-h) + 1px); height: calc(var(--run-row-h) - 2px); padding: 0 8px; border-radius: 999px; color: var(--text); background: color-mix(in srgb, var(--run-color) 14%, transparent); box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--run-color) 42%, transparent); }
    :host-context([data-runs='pill']) .run::before, :host-context([data-runs='pill']) .run::after { display: none; }
    :host-context([data-runs='pill']) .run:hover { background: color-mix(in srgb, var(--run-color) 24%, transparent); }
    :host-context([data-runs='pill']) .run.clip-l { border-top-left-radius: 0; border-bottom-left-radius: 0; }
    :host-context([data-runs='pill']) .run.clip-r, :host-context([data-runs='pill']) .run.st-open { border-top-right-radius: 0; border-bottom-right-radius: 0; }
    :host-context([data-runs='pill']) .run.st-open { box-shadow: none; background: linear-gradient(90deg, color-mix(in srgb, var(--run-color) 18%, transparent) 55%, transparent); }
    :host-context([data-runs='pill']) .run.st-noStop { box-shadow: none; outline: 1px dashed color-mix(in srgb, var(--run-color) 60%, transparent); outline-offset: -1px; background-image: repeating-linear-gradient(135deg, color-mix(in srgb, var(--run-color) 32%, transparent) 0 1px, transparent 1px 6px); }

    /* runs: bracket. A thin rule with ticks down at both ends; the label below it. */
    :host-context([data-runs='bracket']) .run { padding: 3px 3px 0; align-items: flex-start; }
    :host-context([data-runs='bracket']) .run::before { top: 1px; bottom: auto; height: 1px; border-radius: 0; }
    :host-context([data-runs='bracket']) .run::after { top: 1px; bottom: auto; height: 5px; border-left-width: 1px; border-right-width: 1px; }

    .still { position: absolute; top: calc(var(--row) * var(--run-row-h)); height: var(--run-row-h); display: flex; align-items: center; margin-right: 2px; padding: 0 3px; font-size: var(--fs-2xs); line-height: 1; white-space: nowrap; color: var(--text-2); background: var(--bg-1); border-radius: var(--r-1); pointer-events: none; z-index: 0; }
    .warn { position: absolute; right: -6px; top: 50%; width: 12px; height: 12px; margin-top: -6px; display: flex; align-items: center; justify-content: center; pointer-events: none; }
    .glyph-bg { fill: var(--bg-1); stroke: var(--warn-text); stroke-width: 1.4; stroke-linejoin: round; }
    .glyph-fg { fill: none; stroke: var(--warn-text); stroke-width: 1.6; stroke-linecap: round; }
    /* A stop without start is a glyph on its sub-track, in a 24 px wide hit area. */
    .run.point, :host-context([data-runs]) .run.point { width: 24px; margin-left: -12px; padding: 0; justify-content: center; background: transparent; box-shadow: none; outline: none; z-index: 1; }
    .run.point::before, .run.point::after { display: none; }
    .run.point .warn { position: static; margin: 0; }
  `,
})
export class RunTrack {
  readonly items = input.required<readonly RunBandVm[]>();
  /** The lane's source colour: runs without a colour of their own use it (else the accent). */
  readonly laneColor = input<string | null>(null);
  readonly hovered = output<RunHover | null>();
  readonly activated = output<RunBandVm>();

  protected ariaLabel(v: RunBandVm): string {
    const label = v.label;
    const status = runStatusLabel(v.status);
    return $localize`:@@runs.track.aria:${label}:label: (${status}:status:) — show this run`;
  }
}
