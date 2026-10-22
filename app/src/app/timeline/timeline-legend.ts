import { formatCount } from '../shared/format/units';
import { ChangeDetectionStrategy, Component, computed, inject, input, signal } from '@angular/core';
import { AnnotationDto, MarkerKind } from '../core/api.types';
import { formatDateTime } from '../core/format';
import { UiPrefsStore } from '../core/workspace/ui-prefs.store';
import { WorkspaceDataStore } from '../core/workspace/workspace-data.store';
import { MarkerGlyph } from '../markers/marker-glyph';
import { styleOf } from '../markers/marker-styles';
import { SeverityBadge } from '../live/severity-badge';
import { ContextMenuService, MenuEntry } from '../ui/context-menu.service';
import { TimelineAnnotations } from './annotations.store';
import { TIMELINE_SOURCE } from './timeline-source';

/** At most this many annotations are listed in the Evidence menu (the rest: zoom in). */
const MAX_ANN_MENU = 50;
/** Marker kinds shown before "+N more". */
export const LEGEND_COLLAPSED = 6;
/** Run kinds and annotation kinds each shown before "+N more" (one pressed "more" shows everything). */
export const LEGEND_COLLAPSED_GROUP = 3;

let legendSeq = 0;

/**
 * The legend above the lanes: marker kinds (click shows only that kind), runs and annotation kinds (click hides or
 * shows them), and the Evidence menu, the keyboard path to the annotations and runs in view.
 */
@Component({
  selector: 'imt-timeline-legend',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [MarkerGlyph, SeverityBadge],
  host: { style: 'display: contents' },
  template: `
    @if (legendItems().length > 0 || ann.kinds().length || ann.runKinds().length || ann.error()) {
      <div class="legend-row" [id]="listId" [class.collapsed]="!expanded()">
        <div class="legend" role="group" aria-label="Markers" i18n-aria-label="@@timeline.legend.markers" [class.filtering]="src.markers().length > 0" [class.collapsed]="!expanded()">
          @for (k of shownItems(); track k.kind.kind) {
            <button
              type="button"
              class="lg"
              [attr.aria-pressed]="k.active"
              [attr.aria-describedby]="k.descId"
              [title]="k.title"
              (click)="toggleKind(k.kind.kind, k.active)"
            ><span class="pill"><imt-marker-glyph [kind]="k.kind.kind" [size]="16" />{{ k.kind.label }}</span></button>
            @if (k.descId; as id) { <span class="sr" [id]="id">{{ k.title }}</span> }
          }
          @if (hidden() > 0 || expanded()) {
            <button type="button" class="lg more" data-testid="legend-more" [attr.aria-expanded]="expanded()" [attr.aria-controls]="listId" (click)="expanded.set(!expanded())">
              <span class="pill">{{ expanded() ? fewerLabel : moreLabel(hidden()) }}</span>
            </button>
          }
        </div>
        @if (ann.runKinds().length) {
          <div class="run-legend" role="group" aria-label="Runs" i18n-aria-label="@@timeline.legend.runsAria">
            <button
              type="button"
              class="lg runs-toggle"
              data-testid="runs-toggle"
              [attr.aria-pressed]="ann.runsShown()"
              [title]="ann.runsShown() ? hideRunsTitle : showRunsTitle"
              (click)="ann.toggleRuns()"
            ><span class="pill"><span class="run-swatch" aria-hidden="true"></span><span class="lg-title" i18n="@@timeline.legend.runs">Runs</span></span></button>
            @if (ann.runsShown()) {
              @for (k of shownRunKinds(); track k.kind) {
                <button
                  type="button"
                  class="lg"
                  [attr.aria-pressed]="ann.isRunKindShown(k.kind)"
                  [attr.data-run-kind]="k.kind"
                  [title]="runKindTitle(k)"
                  (click)="ann.toggleRunKind(k.kind)"
                ><span class="pill">{{ k.label }} <span class="faint mono">{{ fmt(k.count) }}</span>@if (k.anomalies) { <span class="run-anoms" [attr.aria-label]="anomaliesAria(k.anomalies)">⚠ {{ k.anomalies }}</span> }</span></button>
              }
            }
            @if (!ann.kinds().length && !ann.error() && inView().length) {
              <!-- Without plugin annotations the keyboard path to the runs lives here. -->
              <button
                type="button"
                class="lg ann-list"
                aria-haspopup="menu"
                title="Show the events of a run in view"
                i18n-title="@@timeline.legend.runEvidenceTitle"
                (click)="evidenceMenu($event)"
              ><span class="pill" i18n="@@timeline.legend.evidence">Evidence…</span></button>
            }
          </div>
        }
        @if (ann.kinds().length || ann.error()) {
          <div class="ann-legend" role="group" aria-label="Annotations" i18n-aria-label="@@timeline.legend.annotationsAria">
            <span class="lg-title" i18n="@@timeline.legend.annotations">Annotations</span>
            @for (k of shownAnnKinds(); track k.kind) {
              <button
                type="button"
                class="lg"
                [attr.aria-pressed]="ann.isShown(k.kind)"
                [attr.data-kind]="k.kind"
                [title]="annKindTitle(k)"
                (click)="ann.toggle(k.kind)"
              ><span class="pill"><imt-severity-badge [severity]="k.severity" [showLabel]="false" [size]="12" />{{ k.label }}</span></button>
            }
            @if (inView().length) {
              <!-- The keyboard path to every annotation, and the equivalent control (WCAG 2.5.8) for bands under markers. -->
              <button
                type="button"
                class="lg ann-list"
                aria-haspopup="menu"
                title="Show the evidence of an annotation in view"
                i18n-title="@@timeline.legend.evidenceTitle"
                (click)="evidenceMenu($event)"
              ><span class="pill" i18n="@@timeline.legend.evidence">Evidence…</span></button>
            }
            @if (ann.error(); as e) {
              <span class="ann-err" role="alert" [title]="e" i18n="@@timeline.legend.unavailable">Annotations unavailable</span>
              <button type="button" class="btn" (click)="ann.retry()" i18n="@@timeline.legend.retry">Retry</button>
            }
          </div>
        }
      </div>
    }
  `,
  styles: `
    /* The legend is one row until "+N more" is pressed; clipped sideways only, so the 24px hit areas are not cut. */
    .legend-row.collapsed { flex-wrap: nowrap; overflow-x: clip; }
    .legend-row.collapsed > div { flex-wrap: nowrap; flex: none; }
    .legend-row { flex: none; display: flex; flex-wrap: wrap; align-items: center; gap: 4px 16px; padding: 0 14px 6px; }
    .legend, .ann-legend, .run-legend { display: flex; flex-wrap: wrap; align-items: center; gap: 4px 6px; font-size: var(--fs-sm); color: var(--muted); min-width: 0; }
    /* The button is a transparent 24px hit area (WCAG 2.5.8) around the visible pill; the negative margin keeps the row height. */
    .lg { display: inline-flex; align-items: center; min-height: 24px; margin: -2px 0; padding: 0; border: 0; background: transparent; color: inherit; font: inherit; cursor: pointer; }
    .lg:focus-visible { outline: none; }
    .pill { display: inline-flex; align-items: center; gap: 5px; white-space: nowrap; border: 1px solid transparent; padding: 1px 6px; border-radius: 10px; }
    .lg:focus-visible .pill { outline: 2px solid var(--focus-ring); outline-offset: 1px; }
    .lg:hover .pill { background: var(--bg-hover); }
    .lg[aria-pressed='true'] .pill { border-color: var(--accent); background: var(--accent-soft); }
    /* One line; clipped sideways only, so the 24px hit areas (which overhang by 2px) are not cut. */
    .legend.collapsed { flex-wrap: nowrap; overflow-x: clip; }
    .more .pill { color: var(--chip-text); }
    .lg-title { font-weight: 600; color: var(--text-2); white-space: nowrap; }
    .legend.filtering .lg[aria-pressed='false'], .ann-legend .lg[aria-pressed='false'], .run-legend .lg[aria-pressed='false'] { opacity: 0.5; text-decoration: line-through; }
    .run-legend .lg[aria-pressed='true'] .pill { border-color: transparent; background: transparent; }
    .ann-legend .lg[aria-pressed='true'] .pill { border-color: transparent; background: transparent; }
    .ann-err { color: var(--danger-text); white-space: nowrap; }
    .sr { position: absolute; width: 1px; height: 1px; overflow: hidden; clip-path: inset(50%); }
    .run-swatch { width: 14px; height: 10px; border: 1.5px solid color-mix(in srgb, var(--accent) 75%, transparent); border-radius: 3px; background: color-mix(in srgb, var(--accent) 16%, transparent); }
    .run-legend .lg-title { font-weight: 600; }
    .run-anoms { color: var(--warn-text); }
  `,
})
export class TimelineLegend {
  protected readonly fmt = formatCount;
  /** The annotations and runs in view, in time order (the Evidence menu lists them). */
  readonly inView = input.required<readonly AnnotationDto[]>();

  protected readonly src = inject(TIMELINE_SOURCE);
  protected readonly ann = inject(TimelineAnnotations);
  private readonly data = inject(WorkspaceDataStore);
  private readonly prefs = inject(UiPrefsStore);
  private readonly menu = inject(ContextMenuService);
  protected readonly hideRunsTitle = $localize`:@@timeline.legend.hideRuns:Hide the runs between start and stop markers`;
  protected readonly showRunsTitle = $localize`:@@timeline.legend.showRuns:Show the runs between start and stop markers`;

  protected readonly markerKinds = computed(() => this.data.summary()?.markerKinds ?? []);
  protected readonly listId = `timeline-legend-${++legendSeq}`;
  /** The full list of marker kinds in the data is shown ("+N more" was pressed). */
  protected readonly expanded = signal(false);
  protected readonly fewerLabel = $localize`:@@timeline.legend.fewer:Show fewer`;

  /** Events per marker kind in the current timeline (the engine's clusters), to list only kinds that occur. */
  private readonly occurring = computed(() => {
    const counts = new Map<string, number>();
    for (const lane of this.src.timeline()?.lanes ?? []) for (const m of lane.markers) counts.set(m.kind, (counts.get(m.kind) ?? 0) + m.count);
    return counts;
  });

  /**
   * The marker kinds that occur in the data (or that the timeline is restricted to), most frequent first, with their
   * state: pressed when the timeline shows only that kind.
   */
  protected readonly legendItems = computed(() => {
    const m = this.src.markers();
    const counts = this.occurring();
    const present = this.markerKinds().filter((k) => counts.has(k.kind) || m.includes(k.kind));
    const order = (k: MarkerKind) => (m.includes(k.kind) ? Number.MAX_SAFE_INTEGER : (counts.get(k.kind) ?? 0));
    return [...present].sort((a, b) => order(b) - order(a)).map((kind: MarkerKind) => {
      const description = styleOf(kind).description;
      return {
        kind,
        active: m.length === 1 && m[0] === kind.kind,
        title: description ?? kind.label,
        descId: description ? 'lg-desc-' + kind.kind.replace(/[^\w-]/g, '_') : null,
      };
    });
  });

  /** One line until expanded. */
  protected readonly shownItems = computed(() => (this.expanded() ? this.legendItems() : this.legendItems().slice(0, LEGEND_COLLAPSED)));
  protected readonly shownRunKinds = computed(() => (this.expanded() ? this.ann.runKinds() : this.ann.runKinds().slice(0, LEGEND_COLLAPSED_GROUP)));
  protected readonly shownAnnKinds = computed(() => (this.expanded() ? this.ann.kinds() : this.ann.kinds().slice(0, LEGEND_COLLAPSED_GROUP)));
  /** Entries held back by the one-row legend (marker, run and annotation kinds together). */
  protected readonly hidden = computed(
    () =>
      this.legendItems().length -
      this.shownItems().length +
      (this.ann.runsShown() ? this.ann.runKinds().length - this.shownRunKinds().length : 0) +
      (this.ann.kinds().length - this.shownAnnKinds().length),
  );

  protected moreLabel(n: number): string {
    return $localize`:@@timeline.legend.more:+${n}:count: more`;
  }

  /** Clicking a legend item shows only that marker's events; clicking again clears it. */
  protected toggleKind(kind: string, active: boolean): void {
    this.src.showOnlyMarker(active ? null : kind);
  }

  protected runKindTitle(k: { kind: string; label: string; count: number }): string {
    const { label, count } = k;
    return this.ann.isRunKindShown(k.kind)
      ? $localize`:@@timeline.legend.hideRunKind:Hide ${label}:label: runs (${count}:count:)`
      : $localize`:@@timeline.legend.showRunKind:Show ${label}:label: runs (${count}:count:)`;
  }

  protected anomaliesAria(n: number): string {
    return $localize`:@@timeline.legend.runAnomalies:${n}:count: anomalies`;
  }

  protected annKindTitle(k: { kind: string; label: string; owner: string; count: number }): string {
    const { label, owner, count } = k;
    return this.ann.isShown(k.kind)
      ? $localize`:@@timeline.legend.hideAnnotation:Hide ${label}:label: (${owner}:owner:, ${count}:count:)`
      : $localize`:@@timeline.legend.showAnnotation:Show ${label}:label: (${owner}:owner:, ${count}:count:)`;
  }

  /** Lists the annotations in view; choosing one shows its evidence like clicking its band or glyph. */
  protected evidenceMenu(e: MouseEvent): void {
    const anns = this.inView();
    const at = (e.currentTarget as HTMLElement).getBoundingClientRect();
    const time = (ts: number) => formatDateTime(ts, this.prefs.timezone(), true);
    const items: MenuEntry[] = anns.slice(0, MAX_ANN_MENU).map((a) =>
      a.run
        ? { label: `${a.label} · ${a.instance ?? ''} · ${time(a.from)}`, run: () => this.ann.showRun(a) }
        : { label: `${a.label} · ${time(a.from)}`, run: () => this.ann.showEvidence(a) },
    );
    const more = anns.length - items.length;
    if (more > 0) items.push({ label: $localize`:@@timeline.legend.evidenceMore:${more}:count: more — zoom in to list them`, run: () => undefined, disabled: true });
    this.menu.open(at.left, at.bottom, items, e.currentTarget as HTMLElement);
  }
}
