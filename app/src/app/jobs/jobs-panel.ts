import { PathText } from '../shared/path-text';
import { Tabs } from '../shared/tabs';
import { ChangeDetectionStrategy, Component, computed, effect, inject, signal, untracked } from '@angular/core';
import { AiBackend } from '../ai/ai-backend';
import { formatDuration } from '../core/format';
import { formatBytes, formatCount, formatEta, formatRate, TransferMeter } from '../shared/format/units';
import { TransferSpeed } from '../shared/format/transfer-speed';
import { COMMAND_CONTEXT } from '../extensions/command-registry';
import { Icon } from '../shared/icon';
import { PanelEmpty } from './panel-empty';
import { JobViewer } from './job-viewer';
import { KindFilter, KindOption } from '../ui/kind-filter';
import { JobsStore, JobsView } from './jobs.store';
import { JOB_KINDS, JobInfo, JobKind, JobState } from './jobs.types';
import { kindLabel, linkLabel, phaseText, queuedLabel, stateLabel } from './jobs-text';

/**
 * The Jobs pane: running, queued and finished work of this window (and app-wide downloads), with progress, ETA,
 * Cancel, Retry, move up/down for waiting AI requests, View and Clear finished. The engine decides what is shown
 * (jobs appear after about 2 s) and in which order.
 */
@Component({
  selector: 'debrief-jobs-panel',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [PathText, Icon, KindFilter, PanelEmpty, Tabs],
  template: `
    <div class="bar" role="toolbar" aria-label="Job filters" i18n-aria-label="@@jobs.panel.filters">
      <debrief-tabs kind="filter" label="Show" i18n-label="@@jobs.panel.show" idPrefix="jobs-view-" data-testid="jobs-view" [items]="viewItems()" [selected]="store.view()" (selectedChange)="store.setView($any($event))" />
      <debrief-kind-filter label="Kind" i18n-label="@@jobs.panel.kind" [options]="kindOptions()" [selected]="store.kinds()" (toggled)="store.toggleKind($any($event))" (cleared)="store.setKinds([])" />
      <span class="spacer"></span>
      <button type="button" class="btn ghost small" [disabled]="!store.finished().length" (click)="store.clearFinished()" data-testid="jobs-clear" i18n="@@jobs.panel.clear">Clear finished</button>
    </div>
    @for (p of store.paused(); track p.kind) {
      <div class="paused" role="alert" data-testid="jobs-paused">
        <debrief-icon name="alert" [size]="14" />
        <span class="grow">{{ pausedText(p.kind, p.reason) }}</span>
        @if (p.kind === 'ai') {
          <button type="button" class="btn small" (click)="resetAi()" i18n="@@jobs.panel.resetAi">Reset AI</button>
        }
      </div>
    }
    @if (store.error(); as e) {
      <div class="err" role="alert">
        <span class="grow">{{ e }}</span>
        <button type="button" class="btn small" (click)="store.retryRequest()" i18n="@@jobs.panel.retryRequest">Retry</button>
        <button type="button" class="icon-btn" aria-label="Dismiss" i18n-aria-label="@@jobs.panel.dismissError" (click)="store.dismissError()"><debrief-icon name="close" [size]="12" /></button>
      </div>
    }
    @if (note(); as n) {
      <p class="err" role="alert">{{ n }}</p>
    }
    <p class="sr-only" aria-live="polite" data-testid="jobs-live">{{ announcement() }}</p>
    @if (!store.jobs().length) {
      <debrief-panel-empty class="empty" data-testid="jobs-empty" icon="jobs" title="No jobs" i18n-title="@@jobs.panel.empty.title"><ng-container i18n="@@jobs.panel.empty">Long work such as imports, exports and downloads shows here while it runs.</ng-container></debrief-panel-empty>
    }
    @for (section of sections(); track section.id) {
      @if (section.jobs.length) {
        <h3 [id]="'jobs-h-' + section.id">{{ section.title }} <span class="muted">{{ section.jobs.length }}</span></h3>
        <ul role="list" [attr.aria-labelledby]="'jobs-h-' + section.id" (keydown)="onListKey($event)">
          @for (j of section.jobs; track j.id) {
            <li role="listitem" class="job" [class.unseen]="j.unseen" [attr.data-state]="j.state" [class.viewable]="viewable(j)" [attr.tabindex]="viewable(j) ? 0 : null" [attr.data-testid]="'job-' + j.id" [attr.aria-label]="rowLabel(j)" (click)="rowClick($event, j)" (keydown.enter)="rowKey($event, j)">
              <div class="row">
                <debrief-path-text class="title" [value]="j.title" />
                <span class="kind" [attr.data-kind]="j.kind">{{ kindLabel(j.kind) }}</span>
                <span class="state" [attr.data-testid]="'job-state-' + j.id">{{ j.state === 'queued' ? queuedLabel(j.position) : stateLabel(j.state) }}</span>
                <span class="actions">
                  @if (j.movable) {
                    <button type="button" class="icon-btn" [attr.aria-label]="moveLabel(j, true)" [title]="moveLabel(j, true)" [disabled]="j.position === 1" (click)="store.move(j.id, true)"><debrief-icon name="arrowUp" [size]="14" /></button>
                    <button type="button" class="icon-btn" [attr.aria-label]="moveLabel(j, false)" [title]="moveLabel(j, false)" (click)="store.move(j.id, false)"><debrief-icon name="arrowDown" [size]="14" /></button>
                  }
                  @if (j.cancellable) {
                    <button type="button" class="icon-btn" (click)="store.cancel(j.id)" [attr.data-testid]="'job-cancel-' + j.id" [attr.aria-label]="cancelLabel(j)" [title]="cancelLabel(j)"><debrief-icon name="close" [size]="14" /></button>
                  }
                  @if (j.retryable) {
                    <button type="button" class="icon-btn" (click)="store.retry(j.id)" [attr.data-testid]="'job-retry-' + j.id" [attr.aria-label]="retryLabel(j)" [title]="retryLabel(j)"><debrief-icon name="refresh" [size]="14" /></button>
                  }
                  @if (viewable(j)) {
                    <button type="button" class="icon-btn" (click)="view(j)" [attr.data-testid]="'job-view-' + j.id" [attr.aria-label]="viewLabel(j)" [title]="viewLabel(j)"><debrief-icon name="note" [size]="14" /></button>
                  }
                </span>
              </div>
              @if (j.state === 'running') {
                <div class="progress">
                  @if (j.progress.fraction !== null) {
                    <progress max="1" [value]="j.progress.fraction" [attr.aria-label]="progressLabel(j)"></progress>
                  } @else {
                    <progress [attr.aria-label]="progressLabel(j)"></progress>
                  }
                </div>
              }
              <div class="meta muted small">
                @if (j.state !== 'queued') {
                  <span>{{ elapsedText(j) }}</span>
                }
                @if (j.state === 'running' && phaseOf(j); as ph) {
                  <span class="phase" [attr.data-testid]="'job-phase-' + j.id">{{ ph }}</span>
                }
                @if (progressText(j); as p) {
                  <span [attr.data-testid]="'job-progress-' + j.id">{{ p }}</span>
                }
                @if (j.message) {
                  <span class="msg" [title]="j.message">{{ j.message }}</span>
                }
                @if (linkLabel(j.source.link); as l) {
                  <button type="button" class="link" (click)="openSource(j)" [attr.aria-label]="sourceLabel(l)">{{ l }}</button>
                }
              </div>
              @if (j.error) {
                <p class="error small">{{ j.error }}</p>
              }
            </li>
          }
        </ul>
      }
    }
  `,
  styles: `
    :host { position: relative; display: flex; flex-direction: column; gap: 6px; padding: 8px 10px; overflow: auto; height: 100%; box-sizing: border-box; font-size: var(--fs-md); }
    .bar { display: flex; flex-wrap: wrap; align-items: center; gap: 4px; }
    .spacer, .grow { flex: 1; }
    .link:focus-visible { outline: 2px solid var(--focus-ring); outline-offset: 1px; }
    .paused, .err { display: flex; align-items: center; gap: 8px; padding: 6px 8px; border-radius: 6px; border: 1px solid var(--lvl-warn); background: color-mix(in srgb, var(--lvl-warn) 12%, transparent); }
    .err { border-color: var(--lvl-error); background: color-mix(in srgb, var(--lvl-error) 10%, transparent); margin: 0; }
    h3 { margin: 8px 0 2px; font-size: var(--fs-sm); font-weight: 600; color: var(--text-2); }
    ul { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 6px; }
    .job { border: 1px solid var(--border); border-radius: 6px; padding: 6px 8px; display: flex; flex-direction: column; gap: 4px; }
    .job.unseen { border-left: 3px solid var(--accent); }
    .row { display: flex; align-items: center; gap: 6px; min-width: 0; }
    .kind { font-size: var(--fs-xs); color: var(--muted); white-space: nowrap; }
    .title { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; color: var(--text); }
    .state { font-size: var(--fs-xs); color: var(--text-2); white-space: nowrap; }
    [data-state='failed'] .state, .error { color: var(--danger-text, var(--lvl-error)); }
    .phase { font-weight: 600; color: var(--text-2); }
    .error { margin: 0; overflow-wrap: anywhere; }
    progress { width: 100%; height: 6px; accent-color: var(--accent); }
    .meta { display: flex; flex-wrap: wrap; gap: 10px; }
    .msg { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; max-width: 260px; }
    .link { all: unset; cursor: pointer; text-decoration: underline; color: var(--text-2); }
    /* Row actions show on hover and keyboard focus only. */
    .actions { display: flex; gap: 2px; opacity: 0; }
    .actions:empty { display: none; }
    .job:hover .actions, .job:focus-within .actions { opacity: 1; }
    @media (hover: none) { .actions { opacity: 1; } }
    .job.viewable { cursor: pointer; }
    .job.viewable:hover { background: var(--bg-hover); }
    .job:focus-visible { outline: 2px solid var(--focus-ring); outline-offset: -2px; }
    .count { font-variant-numeric: tabular-nums; }
    .small { font-size: var(--fs-xs); }
    .sr-only { position: absolute; width: 1px; height: 1px; overflow: hidden; clip: rect(0 0 0 0); white-space: nowrap; }
  `,
})
export class JobsPanel {
  protected readonly store = inject(JobsStore);
  private readonly viewer = inject(JobViewer);
  private readonly ai = inject(AiBackend);
  private readonly ctx = inject(COMMAND_CONTEXT);
  protected readonly viewItems = computed(() => this.views.map((v) => ({ id: v.id, label: v.label, count: this.store.viewCounts()[v.id] })));
  protected readonly views: readonly { id: JobsView; label: string }[] = [
    { id: 'all', label: $localize`:@@jobs.panel.view.all:All` },
    { id: 'running', label: $localize`:@@jobs.panel.view.running:Running` },
    { id: 'failed', label: $localize`:@@jobs.panel.view.failed:Failed` },
  ];
  protected readonly kindOptions = computed<KindOption[]>(() => JOB_KINDS.map((k) => ({ id: k, label: kindLabel(k), count: this.store.kindCounts().get(k) ?? 0 })));
  protected readonly kindLabel = kindLabel;
  protected readonly stateLabel = stateLabel;
  protected readonly linkLabel = linkLabel;
  protected readonly queuedLabel = queuedLabel;
  protected readonly note = signal<string | null>(null);
  /** The last state change, read by screen readers. */
  protected readonly announcement = signal('');

  protected sections() {
    return [
      { id: 'running', title: $localize`:@@jobs.panel.running:Running`, jobs: this.store.running() },
      { id: 'queued', title: $localize`:@@jobs.panel.queued:Queued`, jobs: this.store.queued() },
      { id: 'finished', title: $localize`:@@jobs.panel.finished:Finished`, jobs: this.store.finished() },
    ];
  }

  /** Smoothed speed and time left of the running byte jobs; one meter per job, fed from each snapshot. */
  protected readonly speeds = signal<ReadonlyMap<number, TransferSpeed>>(new Map());

  constructor() {
    let seen = new Map<number, JobState>();
    const meters = new Map<number, TransferMeter>();
    // effect-state: samples progress over time into transfer speeds (needs wall-clock history, not derivable)
    effect(() => {
      const jobs = this.store.snapshot().jobs;
      untracked(() => {
        const now = Date.now();
        const next = new Map<number, TransferSpeed>();
        for (const j of jobs) {
          const { done, total, unit } = j.progress;
          if (j.state !== 'running' || unit !== 'bytes' || done === null) continue;
          const meter = meters.get(j.id) ?? new TransferMeter();
          meters.set(j.id, meter);
          meter.sample(done, now);
          next.set(j.id, { rate: meter.rate, etaSecs: total === null ? null : meter.etaSecs(total) });
        }
        for (const id of [...meters.keys()]) if (!next.has(id)) meters.delete(id);
        this.speeds.set(next);
        const changed = jobs.filter((j) => seen.has(j.id) && seen.get(j.id) !== j.state);
        const last = changed.at(-1);
        if (last) this.announcement.set(`${last.title}: ${stateLabel(last.state)}`);
        seen = new Map(jobs.map((j) => [j.id, j.state]));
      });
    });
  }

  protected pausedText(kind: JobKind, reason: string): string {
    return kind === 'ai'
      ? $localize`:@@jobs.panel.pausedAi:AI requests are paused: ${reason}:reason:. Waiting requests stay queued.`
      : $localize`:@@jobs.panel.paused:${kindLabel(kind)}:kind: jobs are paused: ${reason}:reason:`;
  }

  protected rowLabel(j: JobInfo): string {
    return `${kindLabel(j.kind)}: ${j.title}, ${j.state === 'queued' ? queuedLabel(j.position) : stateLabel(j.state)}`;
  }

  protected elapsedText(j: JobInfo): string {
    return $localize`:@@jobs.panel.elapsed:${formatDuration(j.elapsedMs)}:time: elapsed`;
  }

  /**
   * "1.2 GB of 4 GB · 12 MB/s · about 3 min left" for byte progress (speed and time left from the job's meter),
   * "3 / 12 · about 2 min left" for counted items; '' when there is nothing to say.
   */
  protected phaseOf(j: JobInfo): string {
    return phaseText(j.progress);
  }

  protected progressText(j: JobInfo): string {
    const { done, total, unit } = j.progress;
    if (unit === 'bytes' && done !== null) {
      const speed = this.speeds().get(j.id);
      const base = total !== null ? $localize`:@@jobs.panel.bytesOf:${formatBytes(done)}:done: of ${formatBytes(total)}:total:` : formatBytes(done);
      return [base, formatRate(speed?.rate), formatEta(speed?.etaSecs)].filter(Boolean).join(' · ');
    }
    const counted = done !== null && total !== null ? `${formatCount(done)} / ${formatCount(total)}` : '';
    return [counted, j.etaMs !== null ? formatEta(j.etaMs / 1000) : ''].filter(Boolean).join(' · ');
  }

  protected progressLabel(j: JobInfo): string {
    return $localize`:@@jobs.panel.progress:${j.title}:job: progress`;
  }

  protected sourceLabel(l: string): string {
    return $localize`:@@jobs.panel.source:Started from ${l}:source:`;
  }

  protected moveLabel(j: JobInfo, up: boolean): string {
    return up ? $localize`:@@jobs.panel.moveUp:Move ${j.title}:job: up` : $localize`:@@jobs.panel.moveDown:Move ${j.title}:job: down`;
  }

  protected cancelLabel(j: JobInfo): string {
    return $localize`:@@jobs.panel.cancelLabel:Cancel ${j.title}:job:`;
  }

  protected retryLabel(j: JobInfo): string {
    return $localize`:@@jobs.panel.retryLabel:Retry ${j.title}:job:`;
  }

  protected viewLabel(j: JobInfo): string {
    return $localize`:@@jobs.panel.viewLabel:View the result of ${j.title}:job:`;
  }

  /** A finished job with a result opens it on a row click. */
  protected viewable(j: JobInfo): boolean {
    return j.state === 'done' && !!j.resultType;
  }

  protected rowClick(e: Event, j: JobInfo): void {
    if (!this.viewable(j) || (e.target as HTMLElement).closest('button')) return;
    void this.view(j);
  }

  protected rowKey(e: Event, j: JobInfo): void {
    if (e.target !== e.currentTarget || !this.viewable(j)) return;
    void this.view(j);
  }

  protected async view(j: JobInfo): Promise<void> {
    this.note.set(await this.viewer.view(j));
  }

  protected async resetAi(): Promise<void> {
    try {
      await this.ai.reset();
      this.note.set(null);
    } catch (e) {
      this.note.set(e instanceof Error ? e.message : String(e));
    }
  }

  protected async openSource(j: JobInfo): Promise<void> {
    const l = j.source.link;
    if (!l) return;
    if (l.type === 'event') await this.ctx.jumpTo(l.recordId);
    else if (l.type === 'query') this.ctx.setQuery({ ql: l.ql });
    else if (l.type === 'range') this.ctx.setQuery({ from: l.from, to: l.to });
    else if (l.type === 'chat') await this.viewer.view({ id: j.id, title: j.title, resultType: 'chat' });
  }

  /** Up and down move between the jobs’ first buttons (each job is reached with Tab too). */
  protected onListKey(e: KeyboardEvent): void {
    if (e.key !== 'ArrowDown' && e.key !== 'ArrowUp') return;
    const items = [...((e.currentTarget as HTMLElement).querySelectorAll<HTMLElement>('li'))];
    const current = items.findIndex((li) => li.contains(document.activeElement));
    const next = items[current + (e.key === 'ArrowDown' ? 1 : -1)];
    const target = next?.querySelector<HTMLElement>('button');
    if (target) {
      e.preventDefault();
      target.focus();
    }
  }
}
