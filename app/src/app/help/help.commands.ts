import { EnvironmentProviders, inject, Injectable, makeEnvironmentProviders } from '@angular/core';
import { IMT_COMMANDS, ImtCommand } from '../extensions/tokens';
import { TemplatesService } from '../templates/templates.service';
import { DialogService } from '../ui/dialog.service';

/**
 * F1 opens the cheat sheet. Mod+/ is taken (focus the query bar) and "?" would fire while typing in
 * fields that do not stop it; F1 is the platform convention for help and free in the app.
 */
export const SHORTCUTS_KEY = 'F1';

/** Help and onboarding commands of the UX pass: the keyboard cheat sheet and the templates gallery. */
@Injectable({ providedIn: 'root' })
export class HelpCommands {
  private readonly dialogs = inject(DialogService);
  private readonly templates = inject(TemplatesService);

  readonly commands: ImtCommand[] = [
    {
      id: 'help.shortcuts',
      title: 'Keyboard shortcuts',
      category: 'Help',
      keybinding: SHORTCUTS_KEY,
      run: async () => {
        // Loaded on demand: the sheet is not part of the initial bundle.
        const { CheatSheet } = await import('./cheat-sheet');
        if (!this.dialogs.stack().some((d) => d.component === CheatSheet)) this.dialogs.open(CheatSheet, { title: 'Keyboard shortcuts', width: 640 });
      },
    },
    { id: 'profile.newFromTemplate', title: 'New profile from template…', category: 'Profile', run: () => void this.templates.open() },
  ];
}

/** One line in `app.config.ts`. */
export function provideHelp(): EnvironmentProviders {
  return makeEnvironmentProviders([{ provide: IMT_COMMANDS, multi: true, useFactory: () => inject(HelpCommands).commands }]);
}
