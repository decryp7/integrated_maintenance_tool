import { ChangeDetectionStrategy, Component, inject, input } from '@angular/core';
import { plural } from '../i18n/locale';
import { DIALOG_REF, DialogRef } from '../ui/dialog.service';
import { OpenAsk, OpenChoice } from './windowing.types';

/** "Open N file(s)" as the dialog title. */
export function openAskTitle(count: number): string {
  return plural(count, {
    one: $localize`:@@windowing.openAsk.title.one:Open ${count}:count: file`,
    other: $localize`:@@windowing.openAsk.title.other:Open ${count}:count: files`,
  });
}

/** Where the user chose to open the files (the dialog's result). */
export type OpenTarget = Exclude<OpenChoice, 'cancel'>;

@Component({
  selector: 'debrief-open-ask-dialog',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <p i18n="@@windowing.openAsk.question">Where should Debrief open {ask().count, plural, =1 {this file} other {these files}}?</p>
    @if (ask().names.length) {
      <ul class="names mono" aria-label="Files to open" i18n-aria-label="@@windowing.openAsk.files">
        @for (name of ask().names; track $index) {
          <li class="ellipsis" [title]="name">{{ name }}</li>
        }
      </ul>
    }
    <div class="foot">
      <button type="button" class="btn" (click)="choose('here')" i18n="@@windowing.openAsk.here">This window</button>
      <button type="button" class="btn primary" (click)="choose('newWindow')" i18n="@@windowing.openAsk.newWindow">New window</button>
    </div>
  `,
  styles: `
    :host { display: flex; flex-direction: column; gap: 12px; }
    p { margin: 0; }
    .names { margin: 0; padding: 6px 10px; list-style: none; max-height: 180px; overflow: auto; border: 1px solid var(--border-2); border-radius: 6px; background: var(--bg-2); font-size: var(--fs-sm); }
    .foot { display: flex; justify-content: flex-end; gap: 8px; }
  `,
})
export class OpenAskDialog {
  static readonly dialogId = 'open-ask';
  readonly ask = input.required<OpenAsk>();
  private readonly ref = inject<DialogRef<OpenTarget>>(DIALOG_REF);

  protected choose(target: OpenTarget): void {
    this.ref.close(target);
  }
}
