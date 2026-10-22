import { ChangeDetectionStrategy, Component, computed, inject, input } from '@angular/core';
import { PlaybookRun, PlaybookStep } from '../../core/api.types';
import { COMMAND_CONTEXT } from '../../extensions/command-registry';
import { Icon } from '../../shared/icon';
import { actionLabel, actionQuery, expectLabel, finishedText, isRunnable, progressOf, progressText, recordOf, stateOf, statusIcon, statusLabel } from './playbook-model';
import { PlaybookStore } from './playbook.store';

/** The steps of one run: each with its status, what to do, the buttons to do it and a note. */
@Component({
  selector: 'imt-playbook-run',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [Icon],
  template: `
    <div class="progress" data-testid="pb-progress" role="status">
      <strong>{{ progressLabel() }}</strong>
      @if (progress().finished) {
        <span data-testid="pb-finished">{{ finishedLabel() }}</span>
      }
    </div>
    @if (store.actionError(); as e) {
      <div class="error" role="alert" data-testid="pb-run-error">
        {{ e }}
        <button type="button" class="btn" (click)="store.dismissError()" i18n="@@playbooks.dismiss">Dismiss</button>
      </div>
    }
    <ol class="steps">
      @for (step of run().playbook.steps; track step.id; let i = $index) {
        @let state = stateOf(run(), step);
        @let rec = recordOf(run(), step.id);
        @let result = store.resultOf(run().id, step.id);
        <li class="step" [attr.data-testid]="'pb-run-step-' + step.id" [attr.data-state]="state">
          <div class="step-head">
            <span class="status" [attr.data-status]="state" [attr.data-testid]="'pb-status-' + step.id">
              <imt-icon [name]="icon(state)" [size]="13" />
              <span>{{ statusText(state) }}</span>
            </span>
            <strong class="title">{{ i + 1 }}. {{ step.title }}</strong>
          </div>
          @if (step.instruction) {
            <p class="instruction">{{ step.instruction }}</p>
          }
          <div class="action">
            <span class="kind">{{ kind(step) }}</span>
            @if (query(step); as ql) {
              <code>{{ ql }}</code>
            }
            @if (step.action.type === 'openView') {
              <code>{{ step.action.view }}</code>
            }
            @if (step.action.type === 'check') {
              <span class="expect">{{ expectText(step.action.expect.kind) }}@if (step.action.expect.kind === 'atLeast' || step.action.expect.kind === 'atMost') { ({{ step.action.expect.n }}) }</span>
            }
          </div>
          <div class="buttons">
            @if (runnable(step)) {
              <button type="button" class="btn" [attr.data-testid]="'pb-run-' + step.id" [disabled]="!!store.busyStep()" (click)="store.runStep(step)">
                {{ state === 'pending' ? runLabel : rerunLabel }}
              </button>
            }
            @if (step.action.type === 'openView') {
              <button type="button" class="btn" [attr.data-testid]="'pb-open-' + step.id" (click)="open(step.action.view)" i18n="@@playbooks.openView">Open view</button>
            }
            @if (step.action.type === 'manual' || step.action.type === 'openView') {
              <button type="button" class="btn" [attr.data-testid]="'pb-done-' + step.id" [disabled]="!!store.busyStep()" (click)="store.recordManual(step, 'pass')" i18n="@@playbooks.markDone">Mark done</button>
              <button type="button" class="btn" [attr.data-testid]="'pb-skip-' + step.id" [disabled]="!!store.busyStep()" (click)="store.recordManual(step, 'skipped')" i18n="@@playbooks.skip">Skip</button>
            }
          </div>
          @if (rec) {
            @if (rec.summary) {
              <p class="summary" [attr.data-testid]="'pb-summary-' + step.id">{{ rec.summary }}</p>
            }
            @if (rec.count !== null) {
              <p class="count">
                <span [attr.data-testid]="'pb-count-' + step.id" i18n="@@playbooks.count">{{ rec.count }} events</span>
                @if (rec.sampleIds.length) {
                  <button type="button" class="btn" [attr.data-testid]="'pb-show-' + step.id" (click)="store.showEvents(showQuery(step), rec.sampleIds)" i18n="@@playbooks.showEvents">Show events</button>
                }
              </p>
            }
          }
          @if (result && result.columns.length) {
            <table class="rows" [attr.data-testid]="'pb-table-' + step.id">
              <thead>
                <tr>
                  @for (c of result.columns; track $index) {
                    <th scope="col">{{ c }}</th>
                  }
                </tr>
              </thead>
              <tbody>
                @for (r of result.rows; track $index) {
                  <tr>
                    @for (cell of r; track $index) {
                      <td>{{ cellText(cell) }}</td>
                    }
                  </tr>
                }
              </tbody>
            </table>
          }
          <label class="note">
            <span i18n="@@playbooks.note">Note</span>
            <textarea rows="2" [attr.data-testid]="'pb-note-' + step.id" [value]="store.noteFor(run(), step.id)" (change)="store.setNote(step, val($event))"></textarea>
          </label>
        </li>
      }
    </ol>
  `,
  styles: `
    :host { display: block; }
    .progress { display: flex; gap: 12px; align-items: baseline; margin-bottom: 8px; font-size: var(--fs-sm); }
    .error { display: flex; gap: 8px; align-items: center; color: var(--danger-text); font-size: var(--fs-sm); margin-bottom: 8px; }
    .steps { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 8px; }
    .step { display: flex; flex-direction: column; gap: 4px; padding: 8px 10px; border: 1px solid var(--border-2); border-radius: 6px; background: var(--bg-2); font-size: var(--fs-sm); }
    .step-head { display: flex; align-items: center; gap: 8px; }
    .status { display: inline-flex; align-items: center; gap: 4px; flex: none; padding: 0 6px; border: 1px solid currentColor; border-radius: 8px; font-size: var(--fs-xs); color: var(--text-2); }
    .status[data-status=pass] { color: var(--ok); }
    .status[data-status=fail], .status[data-status=error] { color: var(--danger-text); }
    .status[data-status=info] { color: var(--accent-text); }
    .instruction, .summary, .count { margin: 0; color: var(--text-2); }
    .summary { color: var(--text); }
    .action { display: flex; flex-wrap: wrap; align-items: baseline; gap: 6px; }
    .kind { color: var(--muted); font-size: var(--fs-xs); }
    code { padding: 1px 4px; background: var(--bg-0); border-radius: 3px; word-break: break-all; font-size: var(--fs-xs); }
    .buttons { display: flex; flex-wrap: wrap; gap: 6px; }
    .count { display: flex; align-items: center; gap: 8px; }
    .rows { border-collapse: collapse; font-size: var(--fs-xs); }
    .rows th, .rows td { text-align: left; padding: 2px 8px; border-bottom: 1px solid var(--border); }
    .note { display: flex; flex-direction: column; gap: 2px; color: var(--muted); font-size: var(--fs-xs); }
    .note textarea { font: inherit; font-size: var(--fs-sm); color: var(--text); background: var(--bg-0); border: 1px solid var(--border-2); border-radius: 4px; padding: 3px 6px; }
    .note textarea:focus-visible { outline: 2px solid var(--focus-ring); }
  `,
})
export class PlaybookRunView {
  protected readonly store = inject(PlaybookStore);
  private readonly ctx = inject(COMMAND_CONTEXT);
  readonly run = input.required<PlaybookRun>();

  protected readonly progress = computed(() => progressOf(this.run()));
  protected readonly progressLabel = computed(() => progressText(this.progress()));
  protected readonly finishedLabel = computed(() => finishedText(this.progress()));
  protected readonly runLabel = $localize`:@@playbooks.runStep:Run step`;
  protected readonly rerunLabel = $localize`:@@playbooks.rerunStep:Run again`;

  protected readonly stateOf = stateOf;
  protected readonly recordOf = recordOf;
  protected readonly icon = statusIcon;
  protected readonly statusText = statusLabel;
  protected readonly expectText = expectLabel;
  protected readonly runnable = (s: PlaybookStep) => isRunnable(s.action);
  protected readonly query = (s: PlaybookStep) => actionQuery(s.action);
  protected readonly kind = (s: PlaybookStep) => actionLabel(s.action.type);

  /** The query a step's events were found with (a manual step has none). */
  protected showQuery(s: PlaybookStep): string | null {
    return actionQuery(s.action);
  }
  protected open(view: string): void {
    this.ctx.showView(view);
  }
  protected val(e: Event): string {
    return (e.target as HTMLTextAreaElement).value;
  }
  protected cellText(c: unknown): string {
    if (c === null || c === undefined) return '';
    return typeof c === 'object' ? JSON.stringify(c) : String(c);
  }
}
