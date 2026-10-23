import { inject, Injectable } from '@angular/core';
import { EngineSettingsStore } from './engine-settings.store';
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
  /** Unapplied edits live here; closing by any route asks before discarding them. */
  private readonly store = inject(EngineSettingsStore);

  /** Opens the screen (once), showing `section` when given (e.g. `'diagnostics'`). */
  open(section?: string): void {
    if (section) this.store.select(section);
    if (this.dialogs.stack().some((d) => d.component === SettingsScreen)) return;
    this.dialogs.open(SettingsScreen, { title: $localize`:Dialog title@@settings.dialog.title:Settings`, width: 960, canClose: () => this.store.canClose() });
  }
}
