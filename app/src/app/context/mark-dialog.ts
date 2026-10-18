import { ChangeDetectionStrategy, Component, inject, input } from '@angular/core';
import { Condition } from '../core/profile.model';
import { DIALOG_FORM_STYLES } from './dialog-styles';
import { MarkerEditor } from '../markers/marker-editor';
import { MarkerRuleX } from '../markers/markers.types';
import { DIALOG_REF } from '../ui/dialog.service';

/**
 * Dialog around the shared marker editor: "Mark events like this", "Highlight lines" and editing a
 * rule from the markers manager all open this one component with different inputs.
 */
@Component({
  selector: 'imt-mark-dialog',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [MarkerEditor],
  template: `
    <p>{{ hint() }}</p>
    <imt-marker-editor
      [initial]="initial()"
      [suggestedLabel]="suggestedLabel()"
      [suggestedDescription]="suggestedDescription()"
      [rule]="rule()"
      [submitLabel]="submitLabel()"
      (saved)="ref.close(true)"
      (cancelled)="ref.close()"
    />
  `,
  styles: [DIALOG_FORM_STYLES],
})
export class MarkDialog {
  protected readonly ref = inject(DIALOG_REF);

  readonly initial = input<Condition[]>([]);
  readonly suggestedLabel = input('Marked events');
  readonly suggestedDescription = input('');
  readonly rule = input<MarkerRuleX | null>(null);
  readonly submitLabel = input($localize`:@@context.markDialog.submit:Add marker`);
  readonly hint = input($localize`:@@context.markDialog.hint:Show matching events as markers on the timeline and in the grid, and list them under "Marked events only".`);
}
