import { booleanAttribute, ChangeDetectionStrategy, Component, input, output } from '@angular/core';
import { NextSteps } from '../ui/next-steps';
import { ErrorDetails } from '../ui/error-details';

/**
 * Error boundary and loading/empty states for one analysis request: a failed request shows an
 * inline error with Retry instead of breaking the panel; an empty result says what to do next.
 */
@Component({
  selector: 'debrief-analysis-state',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [NextSteps, ErrorDetails],
  template: `
    @if (error(); as message) {
      <div class="state" role="alert">
        @if (retryable()) {
          <!-- A failure (not something to fix in the query): a sentence, Retry, and the message as technical details. -->
          <debrief-error-details summary="Couldn't finish this analysis." i18n-summary="@@analysis.state.failedSummary" [detail]="message" (retry)="retry.emit()" />
        } @else {
          <span class="msg">{{ message }}</span>
        }
        @if (suggest()) {
          <debrief-next-steps />
        }
      </div>
    } @else if (loading() && !hasData()) {
      <div class="state" role="status" i18n="@@analysis.state.loading">Loading…</div>
    } @else if (empty()) {
      <div class="state">
        <span role="status">{{ emptyText() }}</span>
        @if (emptyHint()) {
          <span class="hint">{{ emptyHint() }}</span>
        }
        @if (suggest()) {
          <debrief-next-steps />
        }
      </div>
    } @else {
      <ng-content />
    }
  `,
  styles: `
    :host { display: contents; }
    .state { display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 10px; padding: 28px 16px; color: var(--muted); text-align: center; }
    [role='alert'] .msg { color: var(--danger-text); }
    .hint { max-width: 420px; font-size: var(--fs-sm); color: var(--faint); }
  `,
})
export class AnalysisState {
  readonly loading = input(false);
  readonly error = input<string | null>(null);
  /** Whether Retry is offered (the engine says trying again as is may pass). */
  readonly retryable = input(true);
  readonly empty = input(false);
  readonly emptyText = input($localize`:@@analysis.state.nothingToShow:Nothing to show.`);
  /** What the view needs to show something (e.g. "Pick a field on the left"). */
  readonly emptyHint = input('');
  /** The view analyses the workspace's events: suggest opening files, clearing filters or widening the range. */
  readonly suggest = input(false, { transform: booleanAttribute });
  /** True once a result exists, so a refresh keeps showing it instead of "Loading…". */
  readonly hasData = input(false);
  readonly retry = output<void>();
}
