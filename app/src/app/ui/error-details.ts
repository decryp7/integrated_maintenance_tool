import { ChangeDetectionStrategy, Component, input, output } from '@angular/core';

/**
 * An inline error as people read it: a sentence saying what did not happen ("Couldn't load events."), Retry, and the
 * backend's own message folded under "Technical details" (for a bug report, not for reading first).
 */
@Component({
  selector: 'debrief-error-details',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
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
  `,
  styles: `
    :host { display: flex; flex-direction: column; align-items: center; gap: 8px; text-align: center; }
    .summary { color: var(--text); font-weight: 600; }
    .tech { max-width: 100%; color: var(--muted); font-size: var(--fs-sm); }
    .tech summary { cursor: pointer; }
    .tech summary:focus-visible { outline: 2px solid var(--focus-ring); outline-offset: 2px; }
    .tech code { display: block; margin-top: 6px; color: var(--danger-text); white-space: pre-wrap; overflow-wrap: anywhere; text-align: start; font-size: var(--fs-xs); }
  `,
})
export class ErrorDetails {
  /** What did not happen, as a sentence. */
  readonly summary = input.required<string>();
  /** The backend's message (technical); folded away. */
  readonly detail = input<string | null>(null);
  readonly retryable = input(true);
  readonly retry = output<void>();
}
