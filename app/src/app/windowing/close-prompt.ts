import { ChangeDetectionStrategy, Component, inject, Injectable, signal } from '@angular/core';
import { WorkspaceStore } from '../core/workspace.store';
import { errorMessage } from '../tools/tool-errors';
import { DIALOG_REF, DialogRef, DialogService } from '../ui/dialog.service';
import { WindowingBackend } from './windowing-backend';
import { CloseChoice } from './windowing.types';

/** Asks what closing the last window means: quit, or keep running in the tray. */
@Injectable({ providedIn: 'root' })
export class ClosePromptService {
  private readonly dialogs = inject(DialogService);
  private ref: DialogRef<CloseChoice> | null = null;

  /** Opens the prompt (once: a second request while it is open is ignored). Dismissing keeps the window open. */
  open(): DialogRef<CloseChoice> {
    if (this.ref) return this.ref;
    const ref = this.dialogs.open<CloseChoice>(ClosePrompt, { title: $localize`:@@windowing.closePrompt.title:Close IMT?`, width: 460 });
    this.ref = ref;
    void ref.closed.then(() => (this.ref = null));
    return ref;
  }
}

@Component({
  selector: 'imt-close-prompt',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <p i18n="@@windowing.closePrompt.question">This is the last open window. Do you want to quit IMT, or keep it running in the system tray?</p>
    <p class="faint" i18n="@@windowing.closePrompt.hint">Background work such as live tail and the process monitor keeps running in the tray, and you can reopen a window from there.</p>
    <label class="remember" i18n="@@windowing.closePrompt.remember"><input type="checkbox" [checked]="remember()" (change)="remember.set($any($event.target).checked)" /> Remember my choice</label>
    <div class="foot">
      <button type="button" class="btn" [disabled]="busy()" (click)="choose('quit')" i18n="@@windowing.closePrompt.quit">Quit</button>
      <button type="button" class="btn primary" [disabled]="busy()" (click)="choose('tray')" i18n="@@windowing.closePrompt.tray">Keep running in tray</button>
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
  private readonly ref = inject<DialogRef<CloseChoice>>(DIALOG_REF);
  private readonly backend = inject(WindowingBackend);
  private readonly store = inject(WorkspaceStore);
  protected readonly remember = signal(true);
  protected readonly busy = signal(false);

  protected async choose(choice: CloseChoice): Promise<void> {
    this.busy.set(true);
    try {
      await this.backend.resolveClose(choice, this.remember());
      this.ref.close(choice);
    } catch (e) {
      this.store.setError($localize`:@@windowing.closePrompt.failed:Could not close the window: ${errorMessage(e)}:error:`);
    } finally {
      this.busy.set(false);
    }
  }
}
