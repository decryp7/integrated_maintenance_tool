import { inject, Injectable } from '@angular/core';
import { isTauri } from '../core/platform';
import { InPageDialogBackend } from './in-page-dialog-backend';
import { ConfirmQuestion, DialogSession, DialogShowRequest } from './dialog-session';
import { TauriDialogBackend } from './tauri-dialog-backend';

/** Shows dialogs: as OS windows (desktop) or in the page (browser build, e2e). See ADR 0050. */
@Injectable({ providedIn: 'root', useFactory: createDialogPresenter })
export abstract class DialogPresenter {
  /** True when dialogs are OS windows: modal ones then disable their parent (the OS and a scrim). */
  abstract readonly windowed: boolean;
  abstract show(request: DialogShowRequest): DialogSession;
  /** A yes/no question as the native message box; absent when dialogs are in the page. */
  abstract readonly confirm: ((question: ConfirmQuestion) => Promise<boolean>) | undefined;
}

export function createDialogPresenter(): DialogPresenter {
  const inPage = inject(InPageDialogBackend);
  return isTauri() ? inject(TauriDialogBackend) : inPage;
}
