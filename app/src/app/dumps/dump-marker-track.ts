import { ChangeDetectionStrategy, Component, computed, effect, inject, input, signal, untracked } from '@angular/core';
import { DumpMarker } from '../core/api.types';
import { formatDateTime } from '../core/format';
import { WorkspaceStore } from '../core/workspace.store';
import { COMMAND_CONTEXT } from '../extensions/command-registry';
import { headlineText } from './dump-headline';
import { DumpsBackend } from './dumps-backend';
import { DumpsStore } from './dumps.store';

/** A marker placed on the visible log range (percent of the plot). */
export interface MarkerVm {
  marker: DumpMarker;
  left: number;
  /** Label row, so labels of nearby markers do not cover each other. */
  row: number;
}

/** Estimated label width in px (as the profile span labels). */
const labelWidth = (name: string) => Math.min(200, 14 + name.length * 6.5);

/** Places markers inside the visible range (dropping the others) and stacks nearby labels. Pure. */
export function layoutMarkers(markers: readonly DumpMarker[], view: { from: number; to: number }, widthPx = 1000): MarkerVm[] {
  const span = Math.max(1, view.to - view.from);
  const rowEnds: number[] = [];
  return markers
    .filter((m) => m.timeMs >= view.from && m.timeMs <= view.to)
    .sort((a, b) => a.timeMs - b.timeMs || a.id - b.id)
    .map((m) => {
      const left = ((m.timeMs - view.from) / span) * 100;
      const x = (left / 100) * widthPx;
      let row = rowEnds.findIndex((end) => end <= x);
      if (row < 0) row = rowEnds.push(0) - 1;
      rowEnds[row] = x + labelWidth(m.name) + 4;
      return { marker: m, left, row };
    });
}

/**
 * Dumps and UMDH snapshots as markers on the log timeline, at the time the dump was written (or the snapshot's file
 * time). The label opens the document. Renders nothing without the dumps backend.
 */
@Component({
  selector: 'imt-dump-marker-track',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    @for (m of items(); track m.marker.id) {
      <div class="line" [class.critical]="m.marker.severity === 'critical'" [style.left.%]="m.left" aria-hidden="true"></div>
      <button
        type="button"
        class="chip"
        data-testid="dump-marker"
        [style.left.%]="m.left"
        [style.bottom.px]="4 + m.row * 16"
        [title]="title(m)"
        [attr.aria-label]="label(m)"
        (pointerdown)="$event.stopPropagation()"
        (click)="open(m)"
      >{{ m.marker.kind === 'dump' ? '✖ ' : '▲ ' }}{{ m.marker.name }}</button>
    }
  `,
  styles: `
    :host { position: absolute; inset: 0; pointer-events: none; }
    .line { position: absolute; top: 50px; bottom: 0; width: 0; border-left: 2px dotted var(--s3); }
    .line.critical { border-left-color: var(--lvl-fatal); }
    .chip { pointer-events: auto; position: absolute; max-width: 200px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font: inherit; font-size: var(--fs-2xs); line-height: 14px; padding: 0 5px; border-radius: 3px; border: 1px solid var(--border-3); background: var(--bg-3); color: var(--text); cursor: pointer; z-index: 2; }
    .chip:hover { background: var(--bg-hover); }
    .chip:focus-visible { outline: 2px solid var(--focus-ring); }
  `,
})
export class DumpMarkerTrack {
  /** The visible log range (Unix ms). */
  readonly view = input<{ from: number; to: number } | null>(null);
  /** Plot width in px (for stacking labels). */
  readonly width = input(1000);
  private readonly backend = inject(DumpsBackend, { optional: true });
  /** Only with the dumps feature installed (the store needs its backend). */
  private readonly store = this.backend ? inject(DumpsStore) : null;
  private readonly context = inject(COMMAND_CONTEXT, { optional: true });
  private readonly workspace = inject(WorkspaceStore);
  private readonly markers = signal<DumpMarker[]>([]);

  protected readonly items = computed(() => {
    const v = this.view();
    return v ? layoutMarkers(this.markers(), v, this.width()) : [];
  });

  constructor() {
    effect(() => {
      this.store?.version();
      untracked(() => void this.load());
    });
  }

  private async load(): Promise<void> {
    if (!this.backend) return;
    try {
      this.markers.set(await this.backend.markers());
    } catch {
      // The timeline keeps working without markers; the backend wrapper logs the failure.
      this.markers.set([]);
    }
  }

  protected title(m: MarkerVm): string {
    const when = formatDateTime(m.marker.timeMs, this.workspace.timezone(), true);
    const name = m.marker.name;
    const headline = headlineText(m.marker.headline);
    return $localize`:@@dumps.marker.title:${name}:name: — ${when}:when:: ${headline}:headline:. Click to open.`;
  }

  protected label(m: MarkerVm): string {
    const name = m.marker.name;
    return m.marker.kind === 'dump' ? $localize`:@@dumps.marker.openDump:Open dump ${name}:name:` : $localize`:@@dumps.marker.openUmdh:Open UMDH log ${name}:name:`;
  }

  protected open(m: MarkerVm): void {
    if (m.marker.kind === 'dump') {
      this.store?.setActiveDump(m.marker.id);
      this.context?.showView('dump');
    } else {
      this.store?.setUmdh(m.marker.id, this.store.umdhBase());
      this.context?.showView('umdh');
    }
  }
}
