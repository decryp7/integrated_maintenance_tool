import { ChangeDetectionStrategy, Component, inject, Injectable, input } from '@angular/core';
import { DIALOG_REF, DialogService } from '../ui/dialog.service';
import { ProjectService, UNTITLED } from './project.service';

/** What the user chose in the unsaved-changes dialog; `undefined` (Escape, close, backdrop) means cancel. */
export type UnsavedChoice = 'save' | 'discard' | 'cancel';

/**
 * Asks before an action replaces the current project (New, Open, Open recent) while it has unsaved
 * changes: Save (then continue), Discard (continue) or Cancel (stay).
 */
@Injectable({ providedIn: 'root' })
export class UnsavedChangesGuard {
  private readonly dialogs = inject(DialogService);
  private readonly projects = inject(ProjectService);

  /** Resolves true when the caller may go on. A clean project never asks. */
  async confirm(action: string): Promise<boolean> {
    if (!this.projects.dirty()) return true;
    const ref = this.dialogs.open<UnsavedChoice>(UnsavedChangesDialog, { title: $localize`:@@projects.unsaved.title:Unsaved changes`, width: 440, inputs: { name: this.projects.name(), action } });
    const choice = (await ref.closed) ?? 'cancel';
    if (choice === 'discard') return true;
    // Save As may be cancelled or fail; then the action must not go on and lose the changes.
    if (choice === 'save') return this.projects.save();
    return false;
  }
}

@Component({
  selector: 'debrief-unsaved-changes-dialog',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <p data-testid="unsaved-message" i18n="@@projects.unsaved.message">Save the changes to {{ name() }} before you {{ action() }}? Unsaved changes are lost if you don't save them.</p>
    <div class="foot">
      <button type="button" class="btn ghost" (click)="ref.close('cancel')" i18n="@@projects.unsaved.cancel">Cancel</button>
      <button type="button" class="btn" (click)="ref.close('discard')" i18n="@@projects.unsaved.discard">Discard</button>
      <button type="button" class="btn primary" (click)="ref.close('save')" i18n="@@projects.unsaved.save">Save</button>
    </div>
  `,
  styles: `
    :host { display: flex; flex-direction: column; gap: 12px; }
    p { margin: 0; color: var(--text-2); }
    .foot { display: flex; justify-content: flex-end; gap: 8px; }
  `,
})
export class UnsavedChangesDialog {
  static readonly dialogId = 'unsaved-changes';
  protected readonly ref = inject(DIALOG_REF);
  readonly name = input(UNTITLED);
  readonly action = input($localize`:Completes "Save the changes before you ..."|@@projects.unsaved.defaultAction:continue`);
}
