import { ChangeDetectionStrategy, Component, inject, input, output } from '@angular/core';
import { StoreHealth } from '../core/store-health';

/**
 * An inline error as people read it: a sentence saying what did not happen ("Couldn't load events."), Retry, and the
 * backend's own message folded under "Technical details" (for a bug report, not for reading first).
 */
@Component({
  selector: 'debrief-error-details',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    @if (health.down()) {
      <!-- The log store is not running: one calm state in every panel; the cause and details live in the status bar. -->
      <strong class="summary" data-testid="store-down-empty" i18n="@@ui.errorDetails.storeDown">The log store isn't running.</strong>
      <span class="faint" i18n="@@ui.errorDetails.storeDownSee">See Details in the status bar.</span>
      <button class="btn" type="button" data-testid="store-down-retry" (click)="tryAgain()" i18n="@@ui.errorDetails.tryAgain">Try again</button>
    } @else {
    <strong class="summary">{{ summary() }}</strong>
    @if (retryable()) {
      <button class="btn" type="button" (click)="retry.emit()" i18n="@@ui.errorDetails.retry">Retry</button>
    }
    @if (detail(); as d) {
      <details class="tech">
        <summary i18n="@@ui.errorDetails.technical">Technical details</summary>
        <code class="mono debrief-selectable">{{ d }}</code>
      </details>
    }
    }
  `,
  styles: `
    :host { display: flex; flex-direction: column; align-items: center; gap: 8px; text-align: center; }
    .summary { color: var(--text); font-weight: 600; }
    .faint { color: var(--muted); font-size: var(--fs-sm); }
    .tech { max-width: 100%; color: var(--muted); font-size: var(--fs-sm); }
    .tech summary { cursor: pointer; }
    .tech summary:focus-visible { outline: 2px solid var(--focus-ring); outline-offset: 2px; }
    .tech code { display: block; margin-top: 6px; color: var(--muted); white-space: pre-wrap; overflow-wrap: anywhere; text-align: start; font-size: var(--fs-xs); }
  `,
})
export class ErrorDetails {
  protected readonly health = inject(StoreHealth);
  /** What did not happen, as a sentence. */
  readonly summary = input.required<string>();
  /** The backend's message (technical); folded away. */
  readonly detail = input<string | null>(null);
  readonly retryable = input(true);
  readonly retry = output<void>();

  /** Restarts the store worker, then lets the panel load again. */
  protected async tryAgain(): Promise<void> {
    try {
      await this.health.tryAgain();
    } finally {
      this.retry.emit();
    }
  }
}
