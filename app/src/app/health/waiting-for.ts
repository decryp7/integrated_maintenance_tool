import { ChangeDetectionStrategy, Component, computed, inject, input } from '@angular/core';
import { ServiceHealthStore } from './service-health.store';

/**
 * What a panel shows instead of its own error while a service it needs is not working: calm, neutral, no red and no
 * Retry of its own. The status chip says what happened and repairs it; this links to it. The panel loads by itself on the
 * `ok` transition (it reads `ServiceHealthStore.isServing(service)`).
 */
@Component({
  selector: 'debrief-waiting-for',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <div class="waiting" role="status" aria-live="polite" [attr.data-testid]="'waiting-for-' + service()">
      <span class="spinner" aria-hidden="true"></span>
      <strong i18n="@@health.waitingFor">Waiting for {{ label() }}</strong>
      <button type="button" class="link" (click)="health.showDetails()" data-testid="waiting-for-details" i18n="@@health.waitingFor.details">See what happened</button>
    </div>
  `,
  styles: `
    :host { display: block; }
    .waiting { display: flex; flex-direction: column; align-items: center; gap: 8px; text-align: center; color: var(--muted); padding: 16px; }
    strong { color: var(--text); font-weight: 600; }
    .link { all: unset; cursor: pointer; color: var(--accent-text); text-decoration: underline; font-size: var(--fs-sm); }
    .link:focus-visible { outline: 2px solid var(--focus-ring); outline-offset: 2px; }
    .spinner { width: 14px; height: 14px; border-radius: 50%; border: 2px solid var(--border-2); border-top-color: var(--accent); animation: waiting-spin 0.9s linear infinite; }
    @keyframes waiting-spin { to { transform: rotate(360deg); } }
    @media (prefers-reduced-motion: reduce) { .spinner { animation: none; } }
  `,
})
export class WaitingFor {
  protected readonly health = inject(ServiceHealthStore);
  /** The service id (`store`, `ai`, ...). */
  readonly service = input.required<string>();
  protected readonly label = computed(() => this.health.health(this.service())?.label ?? this.service());
}
