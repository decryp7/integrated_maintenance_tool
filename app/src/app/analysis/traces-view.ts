import { ChangeDetectionStrategy, Component, computed, effect, inject, signal, untracked } from '@angular/core';
import { formatCount, formatDuration } from '../core/format';
import { RequestSpan, TracesResult, WaterfallRow } from '../core/api.types';
import { AnalysisBackend } from '../core/analysis-backend';
import { AnalysisActions } from './analysis-actions';
import { barWidth, shortId } from './analysis-format';
import { AnalysisState } from './analysis-state';
import { ANALYSIS_STYLES } from './analysis-styles';
import { AnalysisTargets } from './analysis-targets';
import { AnalysisTime } from './analysis-time';
import { AsyncResource } from './async-resource';

export const DEFAULT_ID_FIELD = 'RequestId|CorrelationId|TraceId';

/** Geometry (percent of the trace) of one waterfall bar. */
export function barGeometry(row: WaterfallRow, total: number): { left: number; width: number } {
  if (total <= 0) return { left: 0, width: 100 };
  const left = Math.min(100, Math.max(0, (row.startOffsetMs / total) * 100));
  return { left, width: Math.min(100 - left, Math.max(0.5, barWidth(row.durationMs, total))) };
}

/** Total extent of a waterfall in ms. */
export function waterfallTotal(rows: readonly WaterfallRow[]): number {
  return Math.max(0, ...rows.map((r) => r.startOffsetMs + r.durationMs));
}

/** Traces (spans with a waterfall) and requests (events sharing a request id). */
@Component({
  selector: 'imt-traces-view',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [AnalysisState],
  template: `
    <div class="bar">
      <span class="section-title">Traces &amp; requests</span>
      <div class="seg" role="group" aria-label="Show">
        <button class="btn" type="button" [class.active]="mode() === 'traces'" [attr.aria-pressed]="mode() === 'traces'" (click)="mode.set('traces')">Traces</button>
        <button class="btn" type="button" [class.active]="mode() === 'requests'" [attr.aria-pressed]="mode() === 'requests'" (click)="mode.set('requests')">Requests</button>
      </div>
      @if (mode() === 'requests') {
        <label class="lbl">Id field(s)
          <input class="input mono" aria-label="Request id field" [value]="idField()" (change)="idField.set($any($event.target).value.trim() || defaultField)" />
        </label>
      }
      <span class="spacer"></span>
      <button class="btn" type="button" (click)="refresh()">Refresh</button>
    </div>

    @if (mode() === 'traces') {
      <imt-analysis-state [loading]="traces.loading()" [error]="traces.error()" [hasData]="!!traces.data()" [empty]="traces.data()?.traces?.length === 0" emptyText="No traces: no events carry trace and span ids." emptyHint="Traces need events with trace and span id fields (such as TraceId and SpanId). If your logs name them differently, rename the fields in the profile." suggest (retry)="traces.retry()">
        <div class="split">
          <ul class="list tlist" aria-label="Traces">
            @for (t of traces.data()?.traces ?? []; track t.traceId) {
              <li>
                <button class="row" type="button" [class.sel]="t.traceId === selected()" [attr.aria-pressed]="t.traceId === selected()" (click)="selected.set(t.traceId)">
                  <span class="mono id" [title]="t.traceId">{{ short(t.traceId) }}</span>
                  <span class="grow ellipsis" [title]="t.root">{{ t.root }}</span>
                  <span class="num">{{ duration(t.durationMs) }}</span>
                  <span class="num">{{ t.spans }} spans</span>
                  @if (t.errors) { <span class="lvl Error">{{ t.errors }} err</span> }
                </button>
              </li>
            }
          </ul>
          <div class="wf" role="region" aria-label="Waterfall" tabindex="0">
            @if (!selected()) {
              <p class="muted pad">Select a trace to see its waterfall.</p>
            } @else {
              <imt-analysis-state [loading]="waterfall.loading()" [error]="waterfall.error()" [hasData]="!!waterfall.data()" [empty]="!rows().length" emptyText="This trace has no spans." (retry)="waterfall.retry()">
                <p class="muted pad">Total {{ duration(total()) }}</p>
                <ul class="list">
                  @for (r of rows(); track r.spanId) {
                    <li class="wrow">
                      <span class="wname ellipsis" [style.padding-left.px]="r.depth * 14" [title]="r.name">{{ r.name }}</span>
                      <span class="wtrack" role="img" [attr.aria-label]="r.name + ': starts at ' + duration(r.startOffsetMs) + ', lasts ' + duration(r.durationMs) + (r.errors ? ', ' + r.errors + ' errors' : '')">
                        <span class="wbar" [class.bad]="r.errors > 0" [style.left.%]="geo(r).left" [style.width.%]="geo(r).width"></span>
                      </span>
                      <span class="num wdur">{{ duration(r.durationMs) }}</span>
                    </li>
                  }
                </ul>
              </imt-analysis-state>
            }
          </div>
        </div>
      </imt-analysis-state>
    } @else {
      <imt-analysis-state [loading]="requests.loading()" [error]="requests.error()" [hasData]="!!requests.data()" [empty]="requests.data()?.length === 0" emptyText="No requests found for these id fields." (retry)="requests.retry()">
        <div class="body" tabindex="0" role="region" aria-label="Requests">
          <ul class="list">
            @for (r of requests.data() ?? []; track r.id) {
              <li>
                <button class="row" type="button" (click)="filter(r)" [title]="'Show only the events of ' + r.id">
                  <span class="mono id" [title]="r.id">{{ short(r.id) }}</span>
                  <span class="num">{{ duration(r.last - r.first) }}</span>
                  <span class="num">{{ count(r.events) }} events</span>
                  <span class="num" [class.bad]="r.errors">{{ r.errors }} errors</span>
                  <span class="grow ellipsis muted">{{ r.instance }}</span>
                  <span class="mono muted">{{ time.format(r.first) }}</span>
                </button>
              </li>
            }
          </ul>
        </div>
      </imt-analysis-state>
    }
  `,
  styles: [
    ANALYSIS_STYLES,
    `
      .seg { display: inline-flex; gap: 4px; }
      .lbl { display: inline-flex; align-items: center; gap: 6px; color: var(--muted); }
      .split { flex: 1; min-height: 0; display: grid; grid-template-columns: minmax(280px, 40%) 1fr; width: 100%; }
      .tlist { overflow: auto; border-right: 1px solid var(--border); }
      .row.sel { background: var(--bg-sel); }
      .id { width: 112px; flex: none; color: var(--chip-text); }
      .num { flex: none; min-width: 64px; text-align: right; }
      .bad { color: var(--danger-text); }
      .wf { overflow: auto; min-width: 0; }
      .pad { padding: 8px 12px; margin: 0; }
      .wrow { display: flex; align-items: center; gap: 10px; padding: 3px 12px; }
      .wname { width: 260px; flex: none; font-size: 11px; }
      .wtrack { position: relative; flex: 1; height: 12px; background: var(--lane-bar); border-radius: 3px; min-width: 120px; }
      .wbar { position: absolute; top: 1px; bottom: 1px; border-radius: 2px; background: var(--s1); }
      .wbar.bad { background: var(--lvl-error); }
      .wdur { width: 70px; }
    `,
  ],
})
export class TracesView {
  protected readonly time = inject(AnalysisTime);
  protected readonly mode = signal<'traces' | 'requests'>('traces');
  protected readonly selected = signal<string | null>(null);
  protected readonly idField = signal(DEFAULT_ID_FIELD);
  protected readonly defaultField = DEFAULT_ID_FIELD;
  protected readonly traces = new AsyncResource<TracesResult>();
  protected readonly waterfall = new AsyncResource<TracesResult>();
  protected readonly requests = new AsyncResource<RequestSpan[]>();
  protected readonly rows = computed(() => this.waterfall.data()?.waterfall ?? []);
  protected readonly total = computed(() => waterfallTotal(this.rows()));

  protected readonly count = formatCount;
  protected readonly duration = formatDuration;
  protected readonly short = shortId;
  private readonly backend = inject(AnalysisBackend);
  private readonly targets = inject(AnalysisTargets);
  private readonly actions = inject(AnalysisActions);

  constructor() {
    effect(() => {
      const query = this.targets.query();
      const mode = this.mode();
      const field = this.idField();
      untracked(() => {
        if (mode === 'traces') void this.traces.load(() => this.backend.queryTraces(query, ''));
        else void this.requests.load(() => this.backend.queryRequests(query, field));
      });
    });
    effect(() => {
      const id = this.selected();
      const query = this.targets.query();
      if (!id) return untracked(() => this.waterfall.reset());
      untracked(() => void this.waterfall.load(() => this.backend.queryTraces(query, id)));
    });
  }

  protected geo(row: WaterfallRow) {
    return barGeometry(row, this.total());
  }

  protected refresh(): void {
    void (this.mode() === 'traces' ? this.traces.retry() : this.requests.retry());
  }

  protected async filter(r: RequestSpan): Promise<void> {
    await this.actions.filterById(this.idField(), r.id);
  }
}
