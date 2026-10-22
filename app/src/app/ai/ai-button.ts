import { ChangeDetectionStrategy, Component, inject, input, output } from '@angular/core';
import { AiStatusStore } from './ai-status.store';

/**
 * A small "AI" action button for other views (query bar, overview, regex and format dialogs, profile
 * editor, markers manager). Renders nothing while AI is unavailable in this run, so hosts need no checks.
 */
@Component({
  selector: 'debrief-ai-button',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    @if (status.available()) {
      <button
        type="button"
        class="btn ai-btn"
        [class.ghost]="ghost()"
        [disabled]="disabled()"
        [attr.title]="hint() || null"
        [attr.data-testid]="testId() || null"
        (click)="pressed.emit()"
      >
        <span class="spark" aria-hidden="true">✦</span>{{ label() }}
      </button>
    }
  `,
  styles: `
    :host { display: contents; }
    .ai-btn { display: inline-flex; align-items: center; gap: 6px; white-space: nowrap; }
    .spark { color: var(--accent-text); font-size: var(--fs-xs); }
  `,
})
export class AiButton {
  protected readonly status = inject(AiStatusStore);
  readonly label = input.required<string>();
  readonly hint = input('');
  readonly ghost = input(true);
  readonly disabled = input(false);
  readonly testId = input('');
  readonly pressed = output<void>();
}
