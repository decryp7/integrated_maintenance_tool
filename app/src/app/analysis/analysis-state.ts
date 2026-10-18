import { booleanAttribute, ChangeDetectionStrategy, Component, input, output } from '@angular/core';
import { NextSteps } from '../ui/next-steps';

/**
 * Error boundary and loading/empty states for one analysis request: a failed request shows an
 * inline error with Retry instead of breaking the panel; an empty result says what to do next.
 */
@Component({
  selector: 'imt-analysis-state',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [NextSteps],
  template: `
    @if (error(); as message) {
      <div class="state" role="alert">
        <span class="msg">{{ message }}</span>
        <span class="row">
          <button class="btn" type="button" (click)="retry.emit()" i18n="@@analysis.state.retry">Retry</button>
        </span>
        @if (suggest()) {
          <imt-next-steps />
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
          <imt-next-steps />
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
    .hint { max-width: 420px; font-size: 12px; color: var(--faint); }
    .row { display: flex; gap: 8px; }
  `,
})
export class AnalysisState {
  readonly loading = input(false);
  readonly error = input<string | null>(null);
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
