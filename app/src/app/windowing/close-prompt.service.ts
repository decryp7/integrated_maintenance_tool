import { inject, Injectable } from '@angular/core';
import { WorkspaceStore } from '../core/workspace.store';
import { errorMessage } from '../tools/tool-errors';
import { DialogRef, DialogService } from '../ui/dialog.service';
import { ClosePrompt, ClosePromptAnswer } from './close-prompt';
import { WindowingBackend } from './windowing-backend';

/**
 * Asks what closing the last window means: quit, or keep running in the tray. The prompt only answers; this window
 * (the opener, which the engine knows as the asking window) then resolves the close, so the prompt's own window needs no
 * window-management commands.
 */
@Injectable({ providedIn: 'root' })
export class ClosePromptService {
  private readonly dialogs = inject(DialogService);
  private readonly backend = inject(WindowingBackend);
  private readonly store = inject(WorkspaceStore);
  private ref: DialogRef<ClosePromptAnswer> | null = null;

  /** Opens the prompt (once: a second request while it is open is ignored). Dismissing keeps the window open. */
  open(): DialogRef<ClosePromptAnswer> {
    if (this.ref) return this.ref;
    const ref = this.dialogs.open<ClosePromptAnswer>(ClosePrompt, { title: $localize`:@@windowing.closePrompt.title:Close Debrief?`, width: 460 });
    this.ref = ref;
    void ref.closed.then(async (answer) => {
      this.ref = null;
      if (!answer) return;
      try {
        await this.backend.resolveClose(answer.choice, answer.remember);
      } catch (e) {
        this.store.setError($localize`:@@windowing.closePrompt.failed:Could not close the window: ${errorMessage(e)}:error:`);
      }
    });
    return ref;
  }
}
