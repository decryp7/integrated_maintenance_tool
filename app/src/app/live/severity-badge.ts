import { ChangeDetectionStrategy, Component, computed, input } from '@angular/core';
import { Severity } from '../core/api.types';
import { severityLabel } from './alerts.store';

/** A distinct shape per severity, so it never relies on colour alone: octagon, triangle, circle. */
const SHAPES: Record<Severity, { outline: string; mark: string }> = {
  critical: { outline: 'M8 2h8l6 6v8l-6 6H8l-6-6V8z', mark: 'M12 7v6M12 16.5v.5' },
  warning: { outline: 'M12 3l10 18H2z', mark: 'M12 10v4.5M12 17.5v.5' },
  info: { outline: 'M12 3a9 9 0 1 0 0 18a9 9 0 1 0 0-18', mark: 'M12 11v6M12 7.5v.5' },
};

/** Severity icon plus its label ("Critical", "Warning", "Info"). */
@Component({
  selector: 'imt-severity-badge',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <svg [attr.width]="size()" [attr.height]="size()" viewBox="0 0 24 24" aria-hidden="true">
      <path [attr.d]="shape().outline" fill="none" stroke="currentColor" stroke-width="2" stroke-linejoin="round" />
      <path [attr.d]="shape().mark" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" />
    </svg>
    @if (showLabel()) { <span class="label">{{ label() }}</span> } @else { <span class="sr">{{ label() }}</span> }
  `,
  styles: `
    :host { display: inline-flex; align-items: center; gap: 5px; font-weight: 600; white-space: nowrap; }
    :host(.critical) svg { color: var(--lvl-fatal); }
    :host(.warning) svg { color: var(--lvl-warn); }
    :host(.info) svg { color: var(--accent); }
    .label { color: var(--text); }
    .sr { position: absolute; width: 1px; height: 1px; overflow: hidden; clip-path: inset(50%); }
  `,
  host: { '[class]': 'severity()' },
})
export class SeverityBadge {
  readonly severity = input.required<Severity>();
  readonly size = input(14);
  /** False shows only the icon (the label stays available to screen readers). */
  readonly showLabel = input(true);
  protected readonly shape = computed(() => SHAPES[this.severity()] ?? SHAPES.info);
  protected readonly label = computed(() => severityLabel(this.severity()));
}
