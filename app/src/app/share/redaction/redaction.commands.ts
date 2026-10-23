import { inject, Injectable, Injector } from '@angular/core';
import { DebriefCommand } from '../../extensions/tokens';
import { RedactionService } from './redaction.service';

/** Toggles the redacted view (listed in the keyboard cheat sheet). */
export const REDACTED_VIEW_KEY = 'Mod+Shift+X';

/** Commands for the redacted view and redaction settings (services are resolved when a command runs). */
@Injectable({ providedIn: 'root' })
export class RedactionCommands {
  private readonly injector = inject(Injector);
  private redaction = () => this.injector.get(RedactionService);

  readonly commands: DebriefCommand[] = [
    {
      id: 'share.redactedView.toggle',
      title: $localize`:@@share.redactionCommands.toggle:Toggle redacted view`,
      category: $localize`:@@share.commands.categoryView:View`,
      // Ctrl/Cmd+Shift+X: free in the app, and not a browser or WebView shortcut on Windows, macOS or Linux.
      keybinding: REDACTED_VIEW_KEY,
      run: async () => void (await this.redaction().toggle()),
    },
    { id: 'share.redaction.settings', title: $localize`:@@share.redactionCommands.settings:Redaction settings…`, category: $localize`:@@share.commands.categoryView:View`, run: () => this.redaction().openSettings() },
  ];
}
