import { inject, Injectable } from '@angular/core';
import { DebriefCommand } from '../extensions/tokens';
import { LifecycleService } from './lifecycle.service';

/** App lifetime commands: elevation and safe mode. */
@Injectable({ providedIn: 'root' })
export class LifecycleCommands {
  private readonly life = inject(LifecycleService);

  readonly commands: DebriefCommand[] = [
    { id: 'app.restartElevated', title: $localize`:@@lifecycle.commands.restartElevated:Restart as administrator`, category: $localize`:Command category@@lifecycle.commands.category:App`, when: () => this.life.canRestartElevated(), run: () => this.life.restartElevated() },
    // Run by a dialog window's "Restart to apply" (Settings, plugins): the dialog has no session to keep, this window has.
    { id: 'app.restartKeepingSession', title: $localize`:@@lifecycle.commands.restartKeepingSession:Restart and reopen this session`, category: $localize`:Command category@@lifecycle.commands.category:App`, run: () => this.life.restartKeepingSession() },
    { id: 'app.restartSafeMode', title: $localize`:@@lifecycle.commands.restartSafeMode:Restart in safe mode`, category: $localize`:Command category@@lifecycle.commands.category:App`, when: () => !this.life.safeMode(), run: () => this.life.startInSafeMode() },
    { id: 'app.restartNormally', title: $localize`:@@lifecycle.commands.restartNormally:Restart normally`, category: $localize`:Command category@@lifecycle.commands.category:App`, when: () => this.life.safeMode(), run: () => this.life.restartNormally() },
  ];
}
