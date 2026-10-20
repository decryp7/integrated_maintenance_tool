import { ChangeDetectionStrategy, Component, effect, inject, signal, untracked } from '@angular/core';
import { AiBackend } from '../ai/ai-backend';
import { formatDuration } from '../core/format';
import { COMMAND_CONTEXT } from '../extensions/command-registry';
import { Icon } from '../shared/icon';
import { JobViewer } from './job-viewer';
import { JobsStore } from './jobs.store';
import { JOB_KINDS, JobInfo, JobKind, JobState } from './jobs.types';
import { kindLabel, linkLabel, queuedLabel, stateLabel } from './jobs-text';

/**
 * The Jobs pane: running, queued and finished work of this window (and app-wide downloads), with progress, ETA,
 * Cancel, Retry, move up/down for waiting AI requests, View and Clear finished. The engine decides what is shown
 * (jobs appear after about 2 s) and in which order.
 */
@Component({
  selector: 'imt-jobs-panel',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [Icon],
  template: `
    <div class="bar" role="toolbar" aria-label="Job filters" i18n-aria-label="@@jobs.panel.filters">
      <span class="muted small" id="jobs-kinds" i18n="@@jobs.panel.show">Show</span>
      @for (k of kinds; track k) {
        <button type="button" class="chip" [attr.aria-pressed]="store.kinds().includes(k)" (click)="store.toggleKind(k)" [attr.data-testid]="'jobs-kind-' + k">{{ kindLabel(k) }}</button>
      }
      <span class="spacer"></span>
      <button type="button" class="btn ghost small" [disabled]="!store.finished().length" (click)="store.clearFinished()" data-testid="jobs-clear" i18n="@@jobs.panel.clear">Clear finished</button>
    </div>
    @for (p of store.paused(); track p.kind) {
      <div class="paused" role="alert" data-testid="jobs-paused">
        <imt-icon name="alert" [size]="14" />
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
        <button type="button" class="icon-btn" aria-label="Dismiss" i18n-aria-label="@@jobs.panel.dismissError" (click)="store.dismissError()"><imt-icon name="close" [size]="12" /></button>
      </div>
    }
    @if (note(); as n) {
      <p class="err" role="alert">{{ n }}</p>
    }
    <p class="sr-only" aria-live="polite" data-testid="jobs-live">{{ announcement() }}</p>
    @if (!store.jobs().length) {
      <p class="muted empty" data-testid="jobs-empty" i18n="@@jobs.panel.empty">No jobs. Long work (AI answers, imports, analyses, exports, downloads) shows here while it runs and after it finished.</p>
    }
    @for (section of sections(); track section.id) {
      @if (section.jobs.length) {
        <h3 [id]="'jobs-h-' + section.id">{{ section.title }} <span class="muted">{{ section.jobs.length }}</span></h3>
        <ul role="list" [attr.aria-labelledby]="'jobs-h-' + section.id" (keydown)="onListKey($event)">
          @for (j of section.jobs; track j.id) {
            <li role="listitem" class="job" [class.unseen]="j.unseen" [attr.data-state]="j.state" [attr.data-testid]="'job-' + j.id" [attr.aria-label]="rowLabel(j)">
              <div class="row">
                <span class="kind" [attr.data-kind]="j.kind">{{ kindLabel(j.kind) }}</span>
                <span class="title" [title]="j.title">{{ j.title }}</span>
                <span class="state" [attr.data-testid]="'job-state-' + j.id">{{ j.state === 'queued' ? queuedLabel(j.position) : stateLabel(j.state) }}</span>
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
                @if (j.etaMs !== null) {
                  <span>{{ etaText(j.etaMs) }}</span>
                }
                @if (j.progress.done !== null && j.progress.total !== null) {
                  <span>{{ j.progress.done }} / {{ j.progress.total }}</span>
                }
                @if (j.message) {
                  <span class="msg">{{ j.message }}</span>
                }
                @if (linkLabel(j.source.link); as l) {
                  <button type="button" class="link" (click)="openSource(j)" [attr.aria-label]="sourceLabel(l)">{{ l }}</button>
                }
              </div>
              @if (j.error) {
                <p class="error small">{{ j.error }}</p>
              }
              <div class="actions">
                @if (j.movable) {
                  <button type="button" class="icon-btn" [attr.aria-label]="moveLabel(j, true)" [title]="moveLabel(j, true)" [disabled]="j.position === 1" (click)="store.move(j.id, true)"><imt-icon name="arrowUp" [size]="14" /></button>
                  <button type="button" class="icon-btn" [attr.aria-label]="moveLabel(j, false)" [title]="moveLabel(j, false)" (click)="store.move(j.id, false)"><imt-icon name="arrowDown" [size]="14" /></button>
                }
                @if (j.cancellable) {
                  <button type="button" class="btn small" (click)="store.cancel(j.id)" [attr.data-testid]="'job-cancel-' + j.id" [attr.aria-label]="cancelLabel(j)" i18n="@@jobs.panel.cancel">Cancel</button>
                }
                @if (j.retryable) {
                  <button type="button" class="btn small" (click)="store.retry(j.id)" [attr.data-testid]="'job-retry-' + j.id" [attr.aria-label]="retryLabel(j)" i18n="@@jobs.panel.retry">Retry</button>
                }
                @if (j.state === 'done' && j.resultType) {
                  <button type="button" class="btn small primary" (click)="view(j)" [attr.data-testid]="'job-view-' + j.id" [attr.aria-label]="viewLabel(j)" i18n="@@jobs.panel.view">View</button>
                }
              </div>
            </li>
          }
        </ul>
      }
    }
  `,
  styles: `
    :host { display: flex; flex-direction: column; gap: 6px; padding: 8px 10px; overflow: auto; height: 100%; box-sizing: border-box; font-size: var(--fs-md); }
    .bar { display: flex; flex-wrap: wrap; align-items: center; gap: 4px; }
    .spacer, .grow { flex: 1; }
    .chip { border: 1px solid var(--border-2); background: transparent; color: var(--text-2); border-radius: 10px; padding: 1px 8px; font-size: var(--fs-xs); cursor: pointer; }
    .chip[aria-pressed='true'] { background: var(--accent); color: var(--accent-ink); border-color: var(--accent); }
    .chip:focus-visible, .link:focus-visible { outline: 2px solid var(--focus-ring); outline-offset: 1px; }
    .paused, .err { display: flex; align-items: center; gap: 8px; padding: 6px 8px; border-radius: 6px; border: 1px solid var(--lvl-warn); background: color-mix(in srgb, var(--lvl-warn) 12%, transparent); }
    .err { border-color: var(--lvl-error); background: color-mix(in srgb, var(--lvl-error) 10%, transparent); margin: 0; }
    h3 { margin: 8px 0 2px; font-size: var(--fs-xs); font-weight: 600; text-transform: uppercase; letter-spacing: 0.04em; color: var(--text-2); }
    ul { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 6px; }
    .job { border: 1px solid var(--border); border-radius: 6px; padding: 6px 8px; display: flex; flex-direction: column; gap: 4px; }
    .job.unseen { border-left: 3px solid var(--accent); }
    .row { display: flex; align-items: center; gap: 6px; min-width: 0; }
    .kind { font-size: var(--fs-2xs); padding: 0 6px; border-radius: 8px; border: 1px solid var(--border-2); color: var(--text-2); white-space: nowrap; }
    .title { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; color: var(--text); }
    .state { font-size: var(--fs-xs); color: var(--text-2); white-space: nowrap; }
    [data-state='failed'] .state, .error { color: var(--danger-text, var(--lvl-error)); }
    .error { margin: 0; overflow-wrap: anywhere; }
    progress { width: 100%; height: 6px; accent-color: var(--accent); }
    .meta { display: flex; flex-wrap: wrap; gap: 10px; }
    .msg { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; max-width: 260px; }
    .link { all: unset; cursor: pointer; text-decoration: underline; color: var(--text-2); }
    .actions { display: flex; gap: 6px; justify-content: flex-end; }
    .actions:empty { display: none; }
    .empty { margin: 12px 0; }
    .small { font-size: var(--fs-xs); }
    .sr-only { position: absolute; width: 1px; height: 1px; overflow: hidden; clip: rect(0 0 0 0); white-space: nowrap; }
  `,
})
export class JobsPanel {
  protected readonly store = inject(JobsStore);
  private readonly viewer = inject(JobViewer);
  private readonly ai = inject(AiBackend);
  private readonly ctx = inject(COMMAND_CONTEXT);
  protected readonly kinds = JOB_KINDS;
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

  constructor() {
    let seen = new Map<number, JobState>();
    effect(() => {
      const jobs = this.store.snapshot().jobs;
      untracked(() => {
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

  protected etaText(ms: number): string {
    return $localize`:@@jobs.panel.eta:about ${formatDuration(ms)}:time: left`;
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
