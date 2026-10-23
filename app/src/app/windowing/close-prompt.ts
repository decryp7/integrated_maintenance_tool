import { ChangeDetectionStrategy, Component, inject, signal } from '@angular/core';
import { DIALOG_REF, DialogRef } from '../ui/dialog.service';
import { CloseChoice } from './windowing.types';

/** What the prompt answers: the choice and whether to remember it. The window that asked carries it out (`resolve_close`). */
export interface ClosePromptAnswer {
  choice: CloseChoice;
  remember: boolean;
}

@Component({
  selector: 'debrief-close-prompt',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <p i18n="@@windowing.closePrompt.question">This is the last open window. Do you want to quit Debrief, or keep it running in the system tray?</p>
    <p class="faint" i18n="@@windowing.closePrompt.hint">Background work such as live tail and the process monitor keeps running in the tray, and you can reopen a window from there.</p>
    <label class="remember" i18n="@@windowing.closePrompt.remember"><input type="checkbox" [checked]="remember()" (change)="remember.set($any($event.target).checked)" /> Remember my choice</label>
    <div class="foot">
      <button type="button" class="btn" (click)="choose('quit')" i18n="@@windowing.closePrompt.quit">Quit</button>
      <button type="button" class="btn primary" (click)="choose('tray')" i18n="@@windowing.closePrompt.tray">Keep running in tray</button>
    </div>
  `,
  styles: `
    :host { display: flex; flex-direction: column; gap: 12px; }
    p { margin: 0; }
    .remember { display: flex; align-items: center; gap: 8px; }
    .foot { display: flex; justify-content: flex-end; gap: 8px; }
  `,
})
export class ClosePrompt {
  static readonly dialogId = 'close-prompt';
  private readonly ref = inject<DialogRef<ClosePromptAnswer>>(DIALOG_REF);
  protected readonly remember = signal(true);

  protected choose(choice: CloseChoice): void {
    this.ref.close({ choice, remember: this.remember() });
  }
}
