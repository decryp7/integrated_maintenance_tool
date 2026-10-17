import { ChangeDetectionStrategy, Component, computed, input, output } from '@angular/core';
import { QlError } from '../core/api.types';

/**
 * "Did you mean: Level, LevelName?" under a query input: the engine's suggestions for an unknown field,
 * as buttons that replace the misspelt name (the caller applies the choice with `applySuggestion`).
 */
@Component({
  selector: 'imt-ql-suggestions',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    @if (names().length) {
      <span class="dym" data-testid="did-you-mean">Did you mean
        @for (n of names(); track n; let last = $last) {
          <button type="button" class="link mono" [attr.aria-label]="'Replace with ' + n" (click)="pick.emit(n)">{{ n }}</button>@if (!last) {<span>,</span>}
        }?
      </span>
    }
  `,
  styles: `
    :host { display: inline; }
    .dym { color: var(--text-2); }
    .link { background: none; border: 0; padding: 0 2px; font: inherit; color: var(--chip-text); cursor: pointer; text-decoration: underline; }
    .link:focus-visible { outline: 2px solid var(--accent); outline-offset: 1px; border-radius: 3px; }
  `,
})
export class QlSuggestions {
  readonly error = input<QlError | null>(null);
  /** The chosen name. */
  readonly pick = output<string>();
  protected readonly names = computed(() => this.error()?.suggestions ?? []);
}
