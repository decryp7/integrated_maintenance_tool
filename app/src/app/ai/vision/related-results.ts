import { ChangeDetectionStrategy, Component, computed, inject, input, signal } from '@angular/core';
import { TimeCursor } from '../../analysis/time-cursor';
import { formatDateTime } from '../../core/format';
import { WorkspaceStore } from '../../core/workspace.store';
import { PANEL } from '../../dock/dock-context';
import { COMMAND_CONTEXT } from '../../extensions/command-registry';
import { ClockDialogService } from '../../skew/clock-section';
import { TimelineZoom } from '../../timeline/zoom';
import { RelatedLink, RelatedNote, RelatedOutcome } from '../ai.types';
import { AiCitations } from '../chat/citation-actions';
import { AiRichText } from '../chat/rich-text';
import { groupBySource, imageTimeLabel, noteLabel, reasonLabel, screenKindLabel } from './related-format';

/** Half the time window "Show on timeline" zooms to around an event. */
const AROUND_MS = 60_000;

/**
 * "Find related logs": the engine's explanation, the time it read off the image, its notes (no readable time, the
 * clocks disagree → adjust the clock) and the linked events in its order with their reasons, each with "Open in grid"
 * and "Show on timeline".
 */
@Component({
  selector: 'debrief-ai-related-results',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [AiRichText],
  template: `
    @let o = outcome();
    <section class="related" aria-labelledby="ai-related-title" data-testid="ai-related">
      <h4 id="ai-related-title" i18n="@@ai.vision.related.title">Related logs</h4>
      <p class="facts muted">
        {{ kind() }}@if (o.facts.summary) {: {{ o.facts.summary }}}
      </p>
      @if (o.result.time; as t) {
        <p class="time" data-testid="ai-related-time">{{ timeLabel(t) }}</p>
      }
      @if (o.explanation.text) {
        <debrief-ai-rich-text [text]="o.explanation.text" [citations]="o.explanation.citations" (record)="open($event)" (query)="query($event)" (link)="web($event.url, $event.title)" />
      }
      @for (n of o.result.notes; track $index) {
        <p class="note" role="note" [attr.data-testid]="'ai-related-note-' + n.kind">
          {{ note(n) }}
          @if (n.kind === 'clockDisagreement') {
            <button type="button" class="btn small" data-testid="ai-related-clock" (click)="adjustClock(n.sourceId, n.sourceName)" i18n="@@ai.vision.related.adjustClock">Adjust the clock of {{ n.sourceName }}…</button>
          }
        </p>
      }
      @if (o.result.links.length) {
        @for (g of groups(); track g.sourceName) {
          <h5 class="source">{{ g.sourceName }}</h5>
          <ul class="links" [attr.aria-label]="g.sourceName">
            @for (l of g.links; track l.event.id) {
              <li class="link-row" data-testid="ai-related-link">
                <div class="head">
                  <span class="mono ts">{{ time(l.event.ts) }}</span>
                  <span [class]="'lvl mono lvl-' + l.event.level">{{ level(l) }}</span>
                  @if (l.event.group) {
                    <span class="group muted">{{ l.event.group }}</span>
                  }
                </div>
                <p class="msg">{{ l.event.message }}</p>
                <ul class="reasons" aria-label="Why it is linked" i18n-aria-label="@@ai.vision.related.reasons">
                  @for (r of l.reasons; track $index) {
                    <li class="chip reason" data-testid="ai-related-reason">{{ reason(r) }}</li>
                  }
                </ul>
                <div class="actions">
                  <button type="button" class="btn ghost small" (click)="open(l.event.id)" [attr.aria-label]="openLabel(l)" i18n="@@ai.vision.related.open">Open in grid</button>
                  @if (l.event.ts !== null) {
                    <button type="button" class="btn ghost small" (click)="showOnTimeline(l.event.ts)" [attr.aria-label]="timelineLabel(l)" i18n="@@ai.vision.related.timeline">Show on timeline</button>
                  }
                </div>
              </li>
            }
          </ul>
        }
        <p class="muted small" i18n="@@ai.vision.related.examined">{{ o.result.examined }} events examined.</p>
      } @else {
        <p class="muted" data-testid="ai-related-none" i18n="@@ai.vision.related.none">No event of the loaded logs matches this image.</p>
      }
      @if (notice(); as n) {
        <p class="err" role="alert">{{ n }}</p>
      }
    </section>
  `,
  styles: `
    :host { display: block; }
    h4 { margin: 0 0 4px; font-size: var(--fs-md); }
    h5 { margin: 8px 0 4px; font-size: var(--fs-sm); color: var(--text-2); font-weight: 600; }
    p { margin: 0 0 6px; }
    .facts, .time { font-size: var(--fs-sm); }
    .note { font-size: var(--fs-sm); padding: 4px 8px; border-left: 3px solid var(--lvl-warn); background: var(--bg-1); display: flex; gap: 8px; align-items: center; flex-wrap: wrap; }
    .links { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 6px; }
    .link-row { border: 1px solid var(--border); border-radius: 6px; padding: 6px 8px; display: flex; flex-direction: column; gap: 4px; }
    .head { display: flex; gap: 8px; align-items: center; font-size: var(--fs-sm); }
    .ts { color: var(--text-2); }
    .lvl { font-size: var(--fs-xs); font-weight: 600; }
    .lvl-Error, .lvl-Fatal { color: var(--lvl-error); }
    .lvl-Warning { color: var(--lvl-warn); }
    .msg { margin: 0; overflow-wrap: anywhere; }
    .reasons { list-style: none; margin: 0; padding: 0; display: flex; flex-wrap: wrap; gap: 4px; }
    .reason { font-size: var(--fs-xs); }
    .actions { display: flex; gap: 4px; flex-wrap: wrap; }
    .small { font-size: var(--fs-xs); }
    .err { color: var(--danger-text, var(--lvl-error)); }
  `,
})
export class AiRelatedResults {
  readonly outcome = input.required<RelatedOutcome>();

  private readonly citations = inject(AiCitations);
  private readonly ctx = inject(COMMAND_CONTEXT);
  private readonly clock = inject(ClockDialogService);
  private readonly zoom = inject(TimelineZoom);
  private readonly cursor = inject(TimeCursor);
  private readonly workspace = inject(WorkspaceStore);

  protected readonly notice = signal<string | null>(null);
  protected readonly groups = computed(() => groupBySource(this.outcome().result.links));
  protected readonly kind = computed(() => screenKindLabel(this.outcome().facts.screenKind));

  protected readonly time = (ms: number | null): string => formatDateTime(ms, this.workspace.timezone());
  protected readonly reason = reasonLabel;

  protected note(n: RelatedNote): string {
    return noteLabel(n, this.time);
  }

  protected timeLabel(t: NonNullable<RelatedOutcome['result']['time']>): string {
    return imageTimeLabel(t, this.time);
  }

  protected level(l: RelatedLink): string {
    // The engine's level name, as the inspector shows it (the grid's three-letter codes are only for its narrow column).
    return l.event.level;
  }

  protected openLabel(l: RelatedLink): string {
    return $localize`:@@ai.vision.related.openLabel:Open in grid (event ${l.event.id}:id:)`;
  }

  protected timelineLabel(l: RelatedLink): string {
    return $localize`:@@ai.vision.related.timelineLabel:Show on timeline (event ${l.event.id}:id:)`;
  }

  protected async open(id: number): Promise<void> {
    await this.citations.record(id);
  }

  protected async query(ql: string): Promise<void> {
    this.notice.set(await this.citations.query(ql));
  }

  protected async web(url: string, title: string): Promise<void> {
    this.notice.set(await this.citations.web(url, title));
  }

  /** Zooms the timeline to the minutes around the event and marks its moment. */
  protected showOnTimeline(ts: number): void {
    this.zoom.apply({ from: ts - AROUND_MS, to: ts + AROUND_MS });
    this.cursor.select({ from: ts - 500, to: ts + 500 });
    this.ctx.showView(PANEL.timeline);
  }

  /** The clocks disagree: the existing clock-offset correction of that source. */
  protected adjustClock(sourceId: number, sourceName: string): void {
    this.clock.open(sourceId, sourceName);
  }
}
