import { inject, Injectable } from '@angular/core';
import { fileExistsPath } from '../core/api.types';
import { Platform } from '../core/platform';
import { ConfirmDialog } from './confirm-dialog';
import { DialogService } from './dialog.service';

/** Where the path of a write came from: the OS save dialog (which asks before replacing), or the app. */
export type PathOrigin = 'save-dialog' | 'app';

/** File name of a path, for messages. */
function fileName(path: string): string {
  return path.split(/[\\/]/).filter(Boolean).pop() ?? path;
}

/**
 * No silent overwrite. Every command that writes a user-chosen file takes `overwrite` and the engine refuses
 * to replace an existing file without it (a `FILE_EXISTS` error). This runs a write without permission
 * first, and when the engine refuses, asks "Replace …?" and runs it again with permission.
 *
 * A path the native save dialog returned was already confirmed there (macOS, Windows and GTK all ask
 * before returning an existing file), so the app does not ask a second time for it.
 */
@Injectable({ providedIn: 'root' })
export class OverwriteGuard {
  private readonly dialogs = inject(DialogService);
  private readonly platform = inject(Platform);

  /**
   * Runs `write(overwrite)`. Resolves with its result, or null when the file exists and the user chose not
   * to replace it. Any other failure rejects as before.
   */
  async write<T>(path: string, write: (overwrite: boolean) => Promise<T>, origin: PathOrigin = 'save-dialog'): Promise<T | null> {
    const confirmedByDialog = origin === 'save-dialog' && (this.platform.saveDialogConfirmsOverwrite?.() ?? false);
    try {
      return await write(confirmedByDialog);
    } catch (e) {
      const existing = fileExistsPath(e);
      if (existing === null) throw e;
      if (!(await this.confirm(existing || path))) return null;
      return write(true);
    }
  }

  /** Asks whether to replace `path`; true only for an explicit "Replace". */
  async confirm(path: string): Promise<boolean> {
    const ref = this.dialogs.open<boolean>(ConfirmDialog, {
      title: 'Replace file?',
      width: 460,
      inputs: {
        message: `${fileName(path)} already exists. Do you want to replace it? Its current content will be lost.`,
        detail: path,
        confirmLabel: 'Replace',
        cancelLabel: 'Keep the existing file',
        danger: true,
      },
    });
    return (await ref.closed) === true;
  }
}
