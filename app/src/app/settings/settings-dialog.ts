import { inject, Injectable } from '@angular/core';
import { DialogService } from '../ui/dialog.service';
import { SettingsScreen } from './settings-screen';

/** Section id of Debrief's own logs and error snapshots (Rust `settings::DIAGNOSTICS_SECTION`). */
export const DIAGNOSTICS_SECTION = 'diagnostics';
/** Command that opens the Settings screen at the Diagnostics section. */
export const SETTINGS_DIAGNOSTICS_COMMAND = 'settings.diagnostics';

/** Opens the Settings screen (gear in the activity bar, "Open settings" command, Ctrl/Cmd+,, the snapshots view). */
@Injectable({ providedIn: 'root' })
export class SettingsDialogService {
  private readonly dialogs = inject(DialogService);
  /** Opens the screen in its window (once; opening again focuses it), showing `section` when given (e.g. `'diagnostics'`). */
  open(section?: string): void {
    this.dialogs.open(SettingsScreen, { title: $localize`:Dialog title@@settings.dialog.title:Settings`, width: 960, inputs: section ? { section } : {} });
  }
}
