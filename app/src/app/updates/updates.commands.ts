import { inject, Injectable } from '@angular/core';
import { DebriefCommand } from '../extensions/tokens';
import { DialogService } from '../ui/dialog.service';
import { UpdatesStore } from './updates.store';

/** Phases from which opening the dialog starts a check. */
const CHECK_FROM = new Set(['idle', 'upToDate', 'failed']);

/** Help ▸ Check for updates… (also in the macOS app menu: `APP_MENU_COMMANDS`). */
@Injectable({ providedIn: 'root' })
export class UpdatesCommands {
  private readonly dialogs = inject(DialogService);
  private readonly store = inject(UpdatesStore);

  /** Opens the Updates dialog and checks, unless an update is already being handled. */
  open(): void {
    this.dialogs.openLazy(() => import('./updates-dialog').then((m) => m.UpdatesDialog), { title: $localize`:@@updates.dialog.title:Check for updates`, width: 520 });
    const kind = this.store.phase()?.kind ?? 'idle';
    if (CHECK_FROM.has(kind)) void this.store.check();
  }

  readonly commands: DebriefCommand[] = [{ id: 'help.checkForUpdates', title: $localize`:@@updates.command.check:Check for updates…`, category: $localize`:@@updates.command.category:Help`, run: () => this.open() }];
}
