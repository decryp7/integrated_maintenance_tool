import { ChangeDetectionStrategy, Component, inject, input } from '@angular/core';
import { formatDateTime, formatDuration } from '../core/format';
import { UiPrefsStore } from '../core/workspace/ui-prefs.store';
import { MarkerGlyph } from '../markers/marker-glyph';
import { MarkerStyles } from '../markers/marker-styles';
import { SeverityBadge } from '../live/severity-badge';
import { AnnotationHover } from './annotation-track';
import { TimelineAnnotations } from './annotations.store';
import { RunBandVm } from './run-layout';
import { runAnomalyText, runDuration, runStatusLabel } from './run-text';
import { RunHover } from './run-track';
import { MarkerHover } from './timeline-lanes';

/** The timeline's hover cards: for a marker, a run band and an annotation. */
@Component({
  selector: 'imt-timeline-tooltips',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [MarkerGlyph, SeverityBadge],
  host: { style: 'display: contents' },
  template: `
    @if (marker(); as h) {
      <div class="popover tip" [style.left.px]="h.x + 14" [style.top.px]="h.y + 12" role="tooltip">
        <div class="tip-head">
          <imt-marker-glyph [kind]="h.marker.cluster.kind" [size]="18" />
          <strong>{{ h.marker.label }}</strong>
          <span class="faint" i18n="@@timeline.tip.count">{h.marker.cluster.count, plural, =1 {1 event} other {{{ h.marker.cluster.count }} events in this interval}}</span>
          <span class="spacer"></span>
          <span class="mono faint">{{ time(h.marker.cluster.ts) }}</span>
        </div>
        @if (styles.description(h.marker.cluster.kind); as d) { <div class="tip-desc">{{ d }}</div> }
        <div class="mono small">{{ h.lane }}</div>
        <div class="tip-msg">{{ h.marker.cluster.message }}</div>
        @if (h.marker.cluster.count > 1) {
          <div class="faint small" i18n="@@timeline.tip.jumpFirst">Click to jump to the first event</div>
        } @else {
          <div class="faint small" i18n="@@timeline.tip.jumpOne">Click to jump to the event</div>
        }
      </div>
    }

    @if (run(); as h) {
      <div class="popover tip" data-testid="run-tip" [style.left.px]="h.x + 14" [style.top.px]="h.y + 12" role="tooltip">
        <div class="tip-head">
          <strong>{{ h.vm.run.spanLabel }}</strong>
          <span class="badge run-status" [attr.data-status]="h.vm.status">{{ statusLabel(h.vm.status) }}</span>
          <span class="spacer"></span>
          <span class="mono faint">{{ runDur(h.vm) }}</span>
        </div>
        <div class="mono small">{{ h.vm.ann.instance }}@if (h.vm.run.group; as g) { · {{ g.field }} = {{ g.value }} }</div>
        @if (h.vm.run.start; as s) {
          <div class="small"><span class="faint" i18n="@@timeline.runTip.started">Started</span> <span class="mono">{{ time(s.ts) }}</span></div>
          <div class="tip-msg">{{ s.message }}</div>
        }
        @if (h.vm.run.stop; as s) {
          <div class="small"><span class="faint" i18n="@@timeline.runTip.stopped">Stopped</span> <span class="mono">{{ time(s.ts) }}</span></div>
          <div class="tip-msg">{{ s.message }}</div>
        } @else if (h.vm.status === 'open') {
          <div class="small faint" i18n="@@timeline.runTip.open">No stop yet: drawn to the lane's last event ({{ time(h.vm.ann.to) //i18n(ph="time") }}).</div>
        } @else if (h.vm.status === 'noStop') {
          <div class="small faint" i18n="@@timeline.runTip.lastEvent">Last event before the next start: {{ time(h.vm.ann.to) //i18n(ph="time") }}</div>
        }
        @if (anomalyText(h.vm); as w) { <div class="small warn-text">⚠ {{ w }}</div> }
        <div class="faint small" i18n="@@timeline.runTip.click">Click to show this run's events</div>
      </div>
    }

    @if (annotation(); as h) {
      <div class="popover tip" data-testid="annotation-tip" [style.left.px]="h.x + 14" [style.top.px]="h.y + 12" role="tooltip">
        <div class="tip-head">
          <imt-severity-badge [severity]="h.vm.ann.severity" />
          <strong>{{ h.vm.ann.label }}</strong>
        </div>
        <div class="mono small">{{ annTime(h.vm.ann.from, h.vm.ann.to) }}</div>
        <div class="faint small" i18n="@@timeline.annTip.from">{{ annKindLabel(h.vm.ann.kind) //i18n(ph="kind") }} · from {{ h.vm.ann.owner //i18n(ph="owner") }}</div>
        @if (h.vm.ann.instance; as inst) { <div class="mono small">{{ inst }}</div> }
        @else if (h.vm.ann.on; as on) { <div class="mono small">{{ on.field }} = {{ on.value }}</div> }
        <div class="faint small" i18n="@@timeline.annTip.evidence">Click to show the evidence</div>
      </div>
    }
  `,
  styles: `
    .spacer { flex: 1; }
    .small { font-size: 11px; }
    .tip { position: fixed; width: 340px; display: flex; flex-direction: column; gap: 4px; pointer-events: none; line-height: 1.45; }
    .tip-head { display: flex; align-items: center; gap: 8px; }
    .tip-desc { color: var(--text); }
    .tip-msg { color: var(--text-2); }
    .warn-text { color: var(--warn-text); }
    .run-status { font-size: 11px; }
  `,
})
export class TimelineTooltips {
  readonly marker = input<MarkerHover | null>(null);
  readonly run = input<RunHover | null>(null);
  readonly annotation = input<AnnotationHover | null>(null);

  private readonly prefs = inject(UiPrefsStore);
  private readonly ann = inject(TimelineAnnotations);
  protected readonly styles = inject(MarkerStyles);
  protected readonly statusLabel = runStatusLabel;

  protected time(ts: number): string {
    return formatDateTime(ts, this.prefs.timezone(), true);
  }

  protected runDur(v: RunBandVm): string {
    return runDuration(v.run);
  }

  protected anomalyText(v: RunBandVm): string | null {
    return runAnomalyText(v.run);
  }

  protected annTime(from: number, to: number): string {
    const tz = this.prefs.timezone();
    return from === to ? formatDateTime(from, tz, true) : `${formatDateTime(from, tz, true)} → ${formatDateTime(to, tz, true)} (${formatDuration(to - from)})`;
  }

  protected annKindLabel(kind: string): string {
    return this.ann.kinds().find((k) => k.kind === kind)?.label ?? kind;
  }
}
