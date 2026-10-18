import { ChangeDetectionStrategy, Component, inject, input } from '@angular/core';
import { DIALOG_REF } from './dialog.service';

/**
 * A yes/no question in the dialog host: closes with `true` for the confirming button; Cancel, Escape, the
 * close button and the backdrop close it without a result (treat that as "no").
 */
@Component({
  selector: 'imt-confirm-dialog',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <p data-testid="confirm-message">{{ message() }}</p>
    @if (detail()) {
      <p class="detail mono" data-testid="confirm-detail">{{ detail() }}</p>
    }
    <div class="foot">
      <button type="button" class="btn ghost" (click)="ref.close()">{{ cancelLabel() }}</button>
      <button type="button" class="btn primary" [class.danger]="danger()" (click)="ref.close(true)">{{ confirmLabel() }}</button>
    </div>
  `,
  styles: `
    :host { display: flex; flex-direction: column; gap: 12px; }
    p { margin: 0; color: var(--text-2); }
    .detail { font-size: 12px; color: var(--muted); overflow-wrap: anywhere; }
    .foot { display: flex; justify-content: flex-end; gap: 8px; }
    .btn.primary.danger { background: var(--lvl-error); border-color: var(--lvl-error); color: var(--accent-ink); }
  `,
})
export class ConfirmDialog {
  protected readonly ref = inject(DIALOG_REF);
  readonly message = input.required<string>();
  /** A second, smaller line (e.g. the full path). */
  readonly detail = input<string>('');
  readonly confirmLabel = input($localize`:@@ui.confirmDialog.ok:OK`);
  readonly cancelLabel = input($localize`:@@ui.confirmDialog.cancel:Cancel`);
  /** The confirming action destroys something (e.g. replaces a file). */
  readonly danger = input(false);
}
