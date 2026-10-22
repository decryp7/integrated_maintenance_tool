import { EnvironmentProviders, inject, Injectable, makeEnvironmentProviders } from '@angular/core';
import { IMT_COMMANDS, ImtCommand } from '../extensions/tokens';
import { SettingsDialogService } from '../settings/settings-dialog';
import { MANAGE_PARSERS_COMMAND } from '../settings/parsers/parsers.types';
import { EDIT_SHORTCUTS_COMMAND, KEYBOARD_ID } from '../settings/keyboard/keyboard.types';
import { DialogService } from '../ui/dialog.service';
import { WhatsNewService } from '../whats-new/whats-new';

/**
 * F1 opens the cheat sheet. Mod+/ is taken (focus the query bar) and "?" would fire while typing in
 * fields that do not stop it; F1 is the platform convention for help and free in the app.
 */
export const SHORTCUTS_KEY = 'F1';

/** Help and onboarding commands of the UX pass: the keyboard cheat sheet and the parser library (Settings ▸ Parsers). */
@Injectable({ providedIn: 'root' })
export class HelpCommands {
  private readonly dialogs = inject(DialogService);
  private readonly settings = inject(SettingsDialogService);
  private readonly whatsNew = inject(WhatsNewService);

  readonly commands: ImtCommand[] = [
    {
      id: 'help.shortcuts',
      title: $localize`:@@help.command.shortcuts:Keyboard shortcuts`,
      category: $localize`:@@help.command.category:Help`,
      keybinding: SHORTCUTS_KEY,
      run: async () => {
        // Loaded on demand: the sheet is not part of the initial bundle.
        const { CheatSheet } = await import('./cheat-sheet');
        if (!this.dialogs.stack().some((d) => d.component === CheatSheet)) this.dialogs.open(CheatSheet, { title: $localize`:@@help.command.shortcuts:Keyboard shortcuts`, width: 640 });
      },
    },
    { id: 'help.whatsNew', title: $localize`:@@help.command.whatsNew:What's new`, category: $localize`:@@help.command.category:Help`, run: () => this.whatsNew.open() },
    {
      id: EDIT_SHORTCUTS_COMMAND,
      title: $localize`:@@help.command.editShortcuts:Edit keyboard shortcuts…`,
      category: $localize`:@@help.command.category:Help`,
      run: () => this.settings.open(KEYBOARD_ID),
    },
    { id: MANAGE_PARSERS_COMMAND, title: $localize`:@@help.command.manageParsers:Manage parsers…`, category: $localize`:@@help.command.profileCategory:Profile`, run: (ctx) => ctx.showView('parsers') },
  ];
}

/** One line in `app.config.ts`. */
export function provideHelp(): EnvironmentProviders {
  return makeEnvironmentProviders([{ provide: IMT_COMMANDS, multi: true, useFactory: () => inject(HelpCommands).commands }]);
}
