import { ChangeDetectionStrategy, Component, computed, effect, inject, input, signal, untracked } from '@angular/core';
import { ProfileSpan, ProfileTimeRange } from '../../core/api.types';
import { formatDateTime, formatDuration } from '../../core/format';
import { WorkspaceStore } from '../../core/workspace.store';
import { ProfilingBackend } from '../profiling-backend';
import { ProfileViewState } from '../profile-view-state';
import { ProfilingNavigator } from '../profiling-navigator';

/** A capture span placed on the visible log range (percent of the plot). */
export interface SpanVm {
  span: ProfileSpan;
  left: number;
  width: number;
  /** The part of the capture inside the visible range, in ns since its start (null = all of it). */
  range: ProfileTimeRange | null;
  /** Label row, so labels of nearby captures do not cover each other. */
  row: number;
}

/** Estimated label width in px (monospace-ish 10px font, capped like the CSS). */
export const labelWidth = (name: string) => Math.min(220, 12 + name.length * 6.5);

/** Places spans on the visible range (dropping those outside) and stacks overlapping labels. Pure. */
export function layoutSpans(spans: readonly ProfileSpan[], view: { from: number; to: number }, widthPx = 1000): SpanVm[] {
  const span = Math.max(1, view.to - view.from);
  const rowEnds: number[] = [];
  return spans
    .filter((s) => s.toMs >= view.from && s.fromMs <= view.to)
    .sort((a, b) => a.fromMs - b.fromMs || a.id - b.id)
    .map((s) => {
      const from = Math.max(s.fromMs, view.from);
      const to = Math.min(s.toMs, view.to);
      const clipped = s.capture === 'profile' && (from > s.fromMs || to < s.toMs);
      const left = ((from - view.from) / span) * 100;
      const x = (left / 100) * widthPx;
      let row = rowEnds.findIndex((end) => end <= x);
      if (row < 0) row = rowEnds.push(0) - 1;
      rowEnds[row] = x + labelWidth(s.name) + 4;
      return {
        span: s,
        row,
        left,
        width: ((to - from) / span) * 100,
        range: clipped ? { fromNs: Math.round((from - s.fromMs) * 1e6), toNs: Math.round((to - s.fromMs) * 1e6) } : null,
      };
    });
}

/**
 * Profile capture spans as labelled bands on the log timeline (heap snapshots as a line). The label
 * opens the profile on the visible part of the capture. Renders nothing without the profiling plugin.
 */
@Component({
  selector: 'debrief-profile-span-track',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    @for (s of items(); track s.span.id) {
      <div class="band" [class.heap]="s.span.capture === 'heap'" [style.left.%]="s.left" [style.width.%]="s.width" aria-hidden="true"></div>
      <button
        type="button"
        class="chip"
        data-testid="profile-span"
        [style.left.%]="s.left"
        [style.top.px]="52 + s.row * 16"
        [title]="title(s)"
        [attr.aria-label]="openLabel(s)"
        (pointerdown)="$event.stopPropagation()"
        (click)="open(s)"
      >{{ s.span.capture === 'heap' ? '◆ ' : '▶ ' }}{{ s.span.name }}</button>
    }
  `,
  styles: `
    :host { position: absolute; inset: 0; pointer-events: none; }
    .band { position: absolute; top: 50px; bottom: 0; min-width: 2px; background: color-mix(in srgb, var(--s4) 12%, transparent); border-left: 1px dashed var(--s4); border-right: 1px dashed var(--s4); }
    .band.heap { background: none; border-right: 0; }
    .chip { pointer-events: auto; position: absolute; max-width: 220px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font: inherit; font-size: var(--fs-2xs); line-height: 14px; padding: 0 5px; border-radius: 3px; border: 1px solid var(--s4); background: var(--bg-3); color: var(--text); cursor: pointer; z-index: 2; }
    .chip:hover { background: var(--bg-hover); }
    .chip:focus-visible { outline: 2px solid var(--focus-ring); }
  `,
})
export class ProfileSpanTrack {
  /** The visible log range (Unix ms). */
  readonly view = input<{ from: number; to: number } | null>(null);
  /** Plot width in px (for stacking labels). */
  readonly width = input(1000);
  private readonly backend = inject(ProfilingBackend, { optional: true });
  private readonly state = inject(ProfileViewState);
  private readonly nav = inject(ProfilingNavigator);
  private readonly workspace = inject(WorkspaceStore);
  private readonly spans = signal<ProfileSpan[]>([]);

  protected readonly items = computed(() => {
    const v = this.view();
    return v ? layoutSpans(this.spans(), v, this.width()) : [];
  });

  constructor() {
    effect(() => {
      this.state.capturesVersion();
      untracked(() => void this.load());
    });
  }

  private async load(): Promise<void> {
    if (!this.backend) return;
    try {
      this.spans.set(await this.backend.spans());
    } catch {
      // The timeline keeps working without spans; the backend wrapper logs the failure.
      this.spans.set([]);
    }
  }

  protected title(s: SpanVm): string {
    const tz = this.workspace.timezone();
    const when = s.span.fromMs === s.span.toMs ? formatDateTime(s.span.fromMs, tz, true) : `${formatDateTime(s.span.fromMs, tz, true)} (${formatDuration(s.span.toMs - s.span.fromMs)})`;
    const name = s.span.name;
    return $localize`:@@profiling.spanTrack.title:${name}:name: — ${when}:when:. Click to open.`;
  }

  protected openLabel(s: SpanVm): string {
    const name = s.span.name;
    if (s.span.capture === 'heap') {
      return s.range
        ? $localize`:@@profiling.spanTrack.openHeapRange:Open heap snapshot ${name}:name: on the visible range`
        : $localize`:@@profiling.spanTrack.openHeap:Open heap snapshot ${name}:name:`;
    }
    return s.range
      ? $localize`:@@profiling.spanTrack.openProfileRange:Open capture ${name}:name: on the visible range`
      : $localize`:@@profiling.spanTrack.openProfile:Open capture ${name}:name:`;
  }

  protected open(s: SpanVm): void {
    if (s.span.capture === 'heap') this.nav.openHeap(s.span.id);
    else this.nav.openOnRange(s.span.id, s.range);
  }
}
