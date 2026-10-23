import { inject, Injectable } from '@angular/core';
import { WorkspaceStore } from '../core/workspace.store';
import { errorMessage } from '../tools/tool-errors';
import { DialogRef, DialogService } from '../ui/dialog.service';
import { OpenAskDialog, openAskTitle, OpenTarget } from './open-ask-dialog';
import { WindowingBackend } from './windowing-backend';
import { OpenAsk, OpenChoice } from './windowing.types';

/** Asks whether files sent by another app open in this window or a new one. */
@Injectable({ providedIn: 'root' })
export class OpenAskService {
  private readonly dialogs = inject(DialogService);
  private readonly backend = inject(WindowingBackend);
  private readonly store = inject(WorkspaceStore);

  /**
   * Resolves with the chosen window, or undefined when dismissed (Escape, the close button, the
   * backdrop); a dismissal tells Rust to drop the waiting files.
   */
  open(ask: OpenAsk): DialogRef<OpenTarget> {
    const ref = this.dialogs.open<OpenTarget>(OpenAskDialog, { title: openAskTitle(ask.count), width: 460, inputs: { ask } });
    void ref.closed.then((choice) => this.resolve(choice ?? 'cancel'));
    return ref;
  }

  /** The dialog only answers; this window carries the answer out (the dialog's own window may not call `resolve_open`). */
  private async resolve(choice: OpenChoice): Promise<void> {
    try {
      await this.backend.resolveOpen(choice);
    } catch (e) {
      const message = choice === 'cancel' ? $localize`:@@windowing.openAsk.cancelFailed:Could not cancel opening the files: ${errorMessage(e)}:error:` : $localize`:@@windowing.openAsk.failed:Could not open the files: ${errorMessage(e)}:error:`;
      this.store.setError(message);
    }
  }
}
