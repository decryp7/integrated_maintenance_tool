import { ChangeDetectionStrategy, Component, computed, effect, inject, input, output, untracked } from '@angular/core';
import { ServiceHealthStore } from '../health/service-health.store';
import { WaitingFor } from '../health/waiting-for';

/**
 * An inline error as people read it: a sentence saying what did not happen ("Couldn't load events."), Retry, and the
 * backend's own message folded under "Technical details" (for a bug report, not for reading first).
 */
@Component({
  selector: 'debrief-error-details',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [WaitingFor],
  template: `
    @if (waiting()) {
      <!-- The panel's service is not working: one calm state in every panel; the status chip says why and repairs it. -->
      <debrief-waiting-for [service]="service()" />
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
  private readonly health = inject(ServiceHealthStore);
  /** What did not happen, as a sentence. */
  readonly summary = input.required<string>();
  /** The backend's message (technical); folded away. */
  readonly detail = input<string | null>(null);
  readonly retryable = input(true);
  readonly retry = output<void>();
  /** The service the panel's data comes from (default: the log store). */
  readonly service = input('store');
  /** That service is not working: the panel waits instead of showing an error of its own. */
  protected readonly waiting = computed(() => !this.health.isServing(this.service()));

  constructor() {
    // Loads again by itself when the service comes back after this panel waited for it (an output is outside the signal
    // graph). Waiting for the first snapshot of a new window does not count: nothing was lost then.
    let waited = false;
    effect(() => {
      const heard = this.health.isReady();
      const now = this.waiting();
      if (waited && !now) untracked(() => this.retry.emit());
      waited = heard && now;
    });
  }
}
