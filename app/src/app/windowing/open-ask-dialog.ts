import { ChangeDetectionStrategy, Component, inject, Injectable, input, signal } from '@angular/core';
import { WorkspaceStore } from '../core/workspace.store';
import { plural } from '../i18n/locale';
import { errorMessage } from '../tools/tool-errors';
import { DIALOG_REF, DialogRef, DialogService } from '../ui/dialog.service';
import { WindowingBackend } from './windowing-backend';
import { OpenAsk, OpenChoice } from './windowing.types';

/** "Open N file(s)" as the dialog title. */
export function openAskTitle(count: number): string {
  return plural(count, {
    one: $localize`:@@windowing.openAsk.title.one:Open ${count}:count: file`,
    other: $localize`:@@windowing.openAsk.title.other:Open ${count}:count: files`,
  });
}

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
    void ref.closed.then((choice) => (choice === undefined ? this.cancel() : undefined));
    return ref;
  }

  private async cancel(): Promise<void> {
    try {
      await this.backend.resolveOpen('cancel');
    } catch (e) {
      this.store.setError($localize`:@@windowing.openAsk.cancelFailed:Could not cancel opening the files: ${errorMessage(e)}:error:`);
    }
  }
}

/** Where the user chose to open the files (the dialog's result). */
export type OpenTarget = Exclude<OpenChoice, 'cancel'>;

@Component({
  selector: 'imt-open-ask-dialog',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <p i18n="@@windowing.openAsk.question">Where should IMT open {ask().count, plural, =1 {this file} other {these files}}?</p>
    @if (ask().names.length) {
      <ul class="names mono" aria-label="Files to open" i18n-aria-label="@@windowing.openAsk.files">
        @for (name of ask().names; track $index) {
          <li class="ellipsis" [title]="name">{{ name }}</li>
        }
      </ul>
    }
    <div class="foot">
      <button type="button" class="btn" [disabled]="busy()" (click)="choose('here')" i18n="@@windowing.openAsk.here">This window</button>
      <button type="button" class="btn primary" [disabled]="busy()" (click)="choose('newWindow')" i18n="@@windowing.openAsk.newWindow">New window</button>
    </div>
  `,
  styles: `
    :host { display: flex; flex-direction: column; gap: 12px; }
    p { margin: 0; }
    .names { margin: 0; padding: 6px 10px; list-style: none; max-height: 180px; overflow: auto; border: 1px solid var(--border-2); border-radius: 6px; background: var(--bg-2); font-size: 12px; }
    .foot { display: flex; justify-content: flex-end; gap: 8px; }
  `,
})
export class OpenAskDialog {
  readonly ask = input.required<OpenAsk>();
  private readonly ref = inject<DialogRef<OpenTarget>>(DIALOG_REF);
  private readonly backend = inject(WindowingBackend);
  private readonly store = inject(WorkspaceStore);
  protected readonly busy = signal(false);

  protected async choose(target: OpenTarget): Promise<void> {
    this.busy.set(true);
    try {
      await this.backend.resolveOpen(target);
      this.ref.close(target);
    } catch (e) {
      this.store.setError($localize`:@@windowing.openAsk.failed:Could not open the files: ${errorMessage(e)}:error:`);
    } finally {
      this.busy.set(false);
    }
  }
}
