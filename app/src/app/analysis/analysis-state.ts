import { ChangeDetectionStrategy, Component, input, output } from '@angular/core';

/**
 * Error boundary and loading/empty states for one analysis request: a failed request shows an
 * inline error with Retry instead of breaking the panel.
 */
@Component({
  selector: 'imt-analysis-state',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    @if (error(); as message) {
      <div class="state" role="alert">
        <span class="msg">{{ message }}</span>
        <button class="btn" type="button" (click)="retry.emit()">Retry</button>
      </div>
    } @else if (loading() && !hasData()) {
      <div class="state" role="status">Loading…</div>
    } @else if (empty()) {
      <div class="state" role="status">{{ emptyText() }}</div>
    } @else {
      <ng-content />
    }
  `,
  styles: `
    :host { display: contents; }
    .state { display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 10px; padding: 28px 16px; color: var(--muted); text-align: center; }
    [role='alert'] .msg { color: var(--danger-text); }
  `,
})
export class AnalysisState {
  readonly loading = input(false);
  readonly error = input<string | null>(null);
  readonly empty = input(false);
  readonly emptyText = input('Nothing to show.');
  /** True once a result exists, so a refresh keeps showing it instead of "Loading…". */
  readonly hasData = input(false);
  readonly retry = output<void>();
}
