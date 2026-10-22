import { ChangeDetectionStrategy, Component, inject, signal } from '@angular/core';
import { DIALOG_FORM_STYLES } from '../context/dialog-styles';
import { DIALOG_REF } from '../ui/dialog.service';
import { parseRangeTime, rangeProblem } from './import-range';

/** Asks for the part of the time to import (UTC); nothing outside it is read into the project. */
@Component({
  selector: 'imt-import-range-dialog',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <p i18n="@@import.range.hint">Only events inside this range are imported, which keeps a very large folder small. Times are UTC. Files that stopped changing well before the start are not even opened.</p>
    <div class="row">
      <label class="lbl"><span i18n="@@import.range.from">From (UTC)</span>
        <input class="input" type="datetime-local" data-testid="range-from" [value]="from()" (input)="from.set(val($event))" />
      </label>
      <label class="lbl"><span i18n="@@import.range.to">To (UTC)</span>
        <input class="input" type="datetime-local" data-testid="range-to" [value]="to()" (input)="to.set(val($event))" />
      </label>
    </div>
    @if (problem(); as p) { <p class="err" role="alert">{{ p }}</p> }
    <div class="foot">
      <button type="button" class="btn" (click)="ref.close()" i18n="@@import.range.cancel">Cancel</button>
      <button type="button" class="btn primary" data-testid="range-open" [disabled]="!!problem()" (click)="open()" i18n="@@import.range.open">Open</button>
    </div>
  `,
  styles: [DIALOG_FORM_STYLES],
})
export class ImportRangeDialog {
  protected readonly ref = inject(DIALOG_REF);
  protected readonly from = signal('');
  protected readonly to = signal('');

  protected problem(): string | null {
    return rangeProblem(parseRangeTime(this.from()), parseRangeTime(this.to()), this.from(), this.to());
  }

  protected val(e: Event): string {
    return (e.target as HTMLInputElement).value;
  }

  protected open(): void {
    if (this.problem()) return;
    this.ref.close({ from: parseRangeTime(this.from()), to: parseRangeTime(this.to()) });
  }
}
