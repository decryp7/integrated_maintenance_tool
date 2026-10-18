import { inject, Injectable } from '@angular/core';
import { ImtCommand } from '../extensions/tokens';
import { LifecycleService } from './lifecycle.service';

/** App lifetime commands: elevation and safe mode. */
@Injectable({ providedIn: 'root' })
export class LifecycleCommands {
  private readonly life = inject(LifecycleService);

  readonly commands: ImtCommand[] = [
    { id: 'app.restartElevated', title: $localize`:@@lifecycle.commands.restartElevated:Restart as administrator`, category: $localize`:Command category@@lifecycle.commands.category:App`, when: () => this.life.canRestartElevated(), run: () => this.life.restartElevated() },
    { id: 'app.restartSafeMode', title: $localize`:@@lifecycle.commands.restartSafeMode:Restart in safe mode`, category: $localize`:Command category@@lifecycle.commands.category:App`, when: () => !this.life.safeMode(), run: () => this.life.startInSafeMode() },
    { id: 'app.restartNormally', title: $localize`:@@lifecycle.commands.restartNormally:Restart normally`, category: $localize`:Command category@@lifecycle.commands.category:App`, when: () => this.life.safeMode(), run: () => this.life.restartNormally() },
  ];
}
