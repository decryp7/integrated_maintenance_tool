import { ChangeDetectionStrategy, Component, inject } from '@angular/core';
import { AnnotationDto, RUN_STATUSES, RunSort, RunStatus, SpanSummary } from '../core/api.types';
import { formatCount, formatDateTime, formatDuration } from '../core/format';
import { WorkspaceStore } from '../core/workspace.store';
import { COMMAND_CONTEXT } from '../extensions/command-registry';
import { Switch } from '../shared/switch';
import { TimelineAnnotations } from './annotations.store';
import { anomalyOf } from './run-layout';
import { runAnomalyText, runDuration, runStatusLabel } from './run-text';
import { RunsStore } from './runs.store';

interface Column {
  sort: RunSort;
  label: string;
}

/**
 * The "Runs" view: every run between lifecycle markers, per lane, with start, stop, duration and status, sortable by
 * the engine, plus a summary per span kind. Choosing a run filters the timeline and events to it.
 */
@Component({
  selector: 'imt-runs-view',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [Switch],
  template: `
    <div class="head">
      <span class="section-title" i18n="@@runs.view.title">Runs</span>
      <span class="faint" i18n="@@runs.view.intro">Time between start and stop markers, per lane.</span>
      <span class="spacer"></span>
      <label class="field">
        <span class="faint" i18n="@@runs.view.kind">Kind</span>
        <select class="select" aria-label="Run kind" i18n-aria-label="@@runs.view.kindAria" (change)="runs.kind.set(valueOf($event) || null)">
          <option value="" [selected]="!runs.kind()" i18n="@@runs.view.allKinds">All</option>
          @for (s of runs.summaries(); track s.kind) { <option [value]="s.kind" [selected]="runs.kind() === s.kind">{{ s.label }}</option> }
        </select>
      </label>
      <label class="field">
        <span class="faint" i18n="@@runs.view.status">Status</span>
        <select class="select" aria-label="Run status" i18n-aria-label="@@runs.view.statusAria" (change)="runs.status.set(asStatus($event))">
          <option value="" [selected]="!runs.status()" i18n="@@runs.view.allStatuses">All</option>
          @for (s of statuses; track s) { <option [value]="s" [selected]="runs.status() === s">{{ statusLabel(s) }}</option> }
        </select>
      </label>
      <imt-switch size="small" [checked]="runs.inView()" (checkedChange)="runs.inView.set($event)" i18n="@@runs.view.inView">Visible time range only</imt-switch>
      <button type="button" class="btn" (click)="setUp()" i18n="@@runs.view.setUp">Set up runs…</button>
    </div>

    @if (runs.summaries().length) {
      <ul class="summaries" aria-label="Summary per run kind" i18n-aria-label="@@runs.view.summariesAria">
        @for (s of runs.summaries(); track s.kind) {
          <li class="card" [class.off]="!s.enabled" [attr.data-kind]="s.kind">
            <strong>{{ s.label }}</strong>
            @if (!s.enabled) {
              <span class="faint" i18n="@@runs.view.off">switched off</span>
            } @else {
              <span>{{ summaryLine(s) }}</span>
              @if (anomalies(s); as a) { <span class="warn" i18n="@@runs.view.anomalies">⚠ {{ a //i18n(ph="count") }} anomalies</span> }
              @if (s.missingMarkers.length) { <span class="warn">{{ missingText(s) }}</span> }
              @if (s.dropped) { <span class="faint">{{ droppedText(s) }}</span> }
            }
          </li>
        }
      </ul>
    }

    @if (runs.error(); as e) {
      <div class="err" role="alert"><span i18n="@@runs.view.failed">Runs could not be loaded:</span> {{ e }} <button type="button" class="btn" (click)="runs.retry()" i18n="@@runs.view.retry">Retry</button></div>
    }

    <div class="scroll">
      <table class="grid" aria-label="Runs" i18n-aria-label="@@runs.view.tableAria" [attr.aria-busy]="runs.loading()">
        <thead>
          <tr>
            @for (c of columns; track c.sort) {
              <th scope="col" [attr.aria-sort]="ariaSort(c.sort)">
                <button type="button" class="sort" [attr.data-sort]="c.sort" (click)="runs.sortBy(c.sort)">{{ c.label }}<span class="arrow" aria-hidden="true">{{ arrow(c.sort) }}</span></button>
              </th>
            }
            <th scope="col" i18n="@@runs.view.col.run">Run</th>
          </tr>
        </thead>
        <tbody>
          @for (a of runs.rows(); track rowId(a)) {
            <tr [attr.data-status]="a.run?.status" [class.anomaly]="isAnomaly(a)">
              <th scope="row" class="lane"><button type="button" class="link mono" (click)="show(a)">{{ a.instance }}@if (a.run?.group; as g) { <span class="faint"> · {{ g.value }}</span> }<span class="sr">{{ showSuffix(a) }}</span></button></th>
              <td class="mono">{{ eventTime(a.run?.start) }}</td>
              <td class="mono">{{ eventTime(a.run?.stop) }}</td>
              <td class="mono num">{{ duration(a) }}</td>
              <td>
                <span class="status" [attr.data-status]="a.run?.status">@if (isAnomaly(a)) { <span aria-hidden="true">⚠ </span> }{{ statusLabel(a.run?.status ?? 'closed') }}</span>
                @if (a.run?.tooLong) { <span class="warn small" i18n="@@runs.view.tooLong">too long</span> }
              </td>
              <td class="kind" [title]="anomaly(a)">{{ a.run?.spanLabel }}</td>
            </tr>
          } @empty {
            <tr>
              <td colspan="6" class="empty faint">
                @if (!runs.loaded()) {
                  <ng-container i18n="@@runs.view.loading">Loading…</ng-container>
                } @else if (!store.summary()?.sources?.length) {
                  <ng-container i18n="@@runs.view.noData">Open log files to see the runs between their start and stop markers.</ng-container>
                } @else {
                  <ng-container i18n="@@runs.view.none">No runs found. Use "Set up runs…" to choose what starts and stops a run.</ng-container>
                }
              </td>
            </tr>
          }
        </tbody>
      </table>
      @if (runs.more() > 0) {
        <div class="more">
          <span class="faint">{{ moreText() }}</span>
          @if (runs.limit() < 1000) { <button type="button" class="btn" (click)="runs.showMore()" i18n="@@runs.view.showMore">Show more</button> }
        </div>
      }
    </div>
  `,
  styles: `
    :host { flex: 1; min-height: 0; display: flex; flex-direction: column; background: var(--bg-1); }
    .head { display: flex; flex-wrap: wrap; align-items: center; gap: 8px 12px; padding: 8px 14px; font-size: var(--fs-sm); }
    .field { display: inline-flex; align-items: center; gap: 6px; }
    .select { height: 24px; }
    .spacer { flex: 1; }
    .summaries { list-style: none; margin: 0; padding: 0 14px 8px; display: flex; flex-wrap: wrap; gap: 8px; font-size: var(--fs-sm); }
    .card { display: flex; flex-wrap: wrap; align-items: baseline; gap: 4px 10px; padding: 6px 10px; border: 1px solid var(--border); border-radius: 6px; background: var(--bg-2); }
    .card.off { opacity: 0.7; }
    .warn { color: var(--warn-text); }
    .err { padding: 6px 14px; color: var(--danger-text); }
    .scroll { flex: 1; overflow: auto; padding: 0 14px 14px; }
    .grid { width: 100%; border-collapse: collapse; font-size: var(--fs-md); }
    .grid th, .grid td { text-align: left; padding: 4px 8px; border-bottom: 1px solid var(--border); white-space: nowrap; }
    .grid thead th { position: sticky; top: 0; background: var(--bg-1); color: var(--muted); font-weight: 500; }
    .sort { border: 0; background: transparent; color: inherit; font: inherit; cursor: pointer; padding: 2px 0; min-height: 24px; }
    .sort:focus-visible, .link:focus-visible { outline: 2px solid var(--focus-ring); outline-offset: 1px; }
    .arrow { display: inline-block; width: 1em; }
    .link { border: 0; background: transparent; color: var(--accent-text, var(--accent)); font: inherit; cursor: pointer; padding: 2px 0; min-height: 24px; text-align: left; }
    .link:hover { text-decoration: underline; }
    .lane { font-weight: 400; }
    .num { text-align: right; }
    .status[data-status='noStop'], .status[data-status='noStart'] { color: var(--warn-text); }
    .small { font-size: var(--fs-xs); margin-left: 4px; }
    .empty { padding: 20px; text-align: center; white-space: normal; }
    .sr { position: absolute; width: 1px; height: 1px; overflow: hidden; clip-path: inset(50%); white-space: nowrap; }
    .more { display: flex; align-items: center; gap: 10px; padding: 8px 0; font-size: var(--fs-sm); }
  `,
})
export class RunsView {
  protected readonly runs = inject(RunsStore);
  protected readonly store = inject(WorkspaceStore);
  private readonly ann = inject(TimelineAnnotations);
  private readonly commands = inject(COMMAND_CONTEXT);
  protected readonly statuses = RUN_STATUSES;
  protected readonly statusLabel = runStatusLabel;
  protected readonly columns: Column[] = [
    { sort: 'lane', label: $localize`:Runs table column|@@runs.view.col.lane:Lane` },
    { sort: 'start', label: $localize`:Runs table column|@@runs.view.col.start:Start` },
    { sort: 'stop', label: $localize`:Runs table column|@@runs.view.col.stop:Stop` },
    { sort: 'duration', label: $localize`:Runs table column|@@runs.view.col.duration:Duration` },
    { sort: 'status', label: $localize`:Runs table column|@@runs.view.col.status:Status` },
  ];

  constructor() {
    // The runs and annotations are queried while this view (or another one showing them) is visible.
    this.runs.gate.use();
    this.ann.gate.use();
  }

  protected valueOf(e: Event): string {
    return (e.target as HTMLSelectElement).value;
  }

  protected asStatus(e: Event): RunStatus | null {
    const v = this.valueOf(e);
    return (RUN_STATUSES as readonly string[]).includes(v) ? (v as RunStatus) : null;
  }

  protected ariaSort(column: RunSort): 'ascending' | 'descending' | 'none' {
    if (this.runs.sort() !== column) return 'none';
    return this.runs.descending() ? 'descending' : 'ascending';
  }

  protected arrow(column: RunSort): string {
    if (this.runs.sort() !== column) return '';
    return this.runs.descending() ? '↓' : '↑';
  }

  protected rowId(a: AnnotationDto): string {
    return `${a.kind}|${a.instance}|${a.run?.group?.value ?? ''}|${a.from}|${a.run?.status}`;
  }

  protected time(ts: number): string {
    return formatDateTime(ts, this.store.timezone(), true);
  }

  protected eventTime(e: { ts: number } | null | undefined): string {
    return e ? this.time(e.ts) : '–';
  }

  protected duration(a: AnnotationDto): string {
    return a.run ? runDuration(a.run) : '–';
  }

  protected isAnomaly(a: AnnotationDto): boolean {
    return !!a.run && anomalyOf(a.run) !== null;
  }

  protected anomaly(a: AnnotationDto): string {
    return (a.run && runAnomalyText(a.run)) ?? '';
  }

  protected anomalies(s: SpanSummary): number {
    return s.noStop + s.noStart + s.tooLong;
  }

  protected summaryLine(s: SpanSummary): string {
    const runs = formatCount(s.runs);
    const lanes = formatCount(s.lanes);
    const open = formatCount(s.open);
    const avg = formatDuration(s.avgMs);
    const max = formatDuration(s.maxMs);
    return $localize`:@@runs.view.summaryLine:${runs}:runs: runs on ${lanes}:lanes: lanes · ${open}:open: still running · average ${avg}:avg: · longest ${max}:max:`;
  }

  protected missingText(s: SpanSummary): string {
    const kinds = s.missingMarkers.join(', ');
    return $localize`:@@runs.view.missing:Markers not defined: ${kinds}:kinds:`;
  }

  protected droppedText(s: SpanSummary): string {
    const n = formatCount(s.dropped);
    return $localize`:@@runs.view.dropped:${n}:count: older runs or events not kept (limits)`;
  }

  protected moreText(): string {
    const n = formatCount(this.runs.more());
    return $localize`:@@runs.view.more:${n}:count: more runs`;
  }

  /** Read by screen readers after the lane (the visible text starts the name, WCAG 2.5.3). */
  protected showSuffix(a: AnnotationDto): string {
    const at = a.run?.start ? this.time(a.run.start.ts) : this.time(a.from);
    return $localize`:@@runs.view.showSuffix: — show the run from ${at}:time:`;
  }

  protected show(a: AnnotationDto): void {
    this.ann.showRun(a);
  }

  /** Opens the Markers view, whose "Runs" section is the guided editor. */
  protected setUp(): void {
    this.commands.showView('markers');
  }
}
