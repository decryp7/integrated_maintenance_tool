import { inject, Injectable } from '@angular/core';
import { ImtCommand } from '../extensions/tokens';
import { LifecycleService } from './lifecycle.service';

/** App lifetime commands: elevation and safe mode. */
@Injectable({ providedIn: 'root' })
export class LifecycleCommands {
  private readonly life = inject(LifecycleService);

  readonly commands: ImtCommand[] = [
    { id: 'app.restartElevated', title: 'Restart as administrator', category: 'App', when: () => this.life.canRestartElevated(), run: () => this.life.restartElevated() },
    { id: 'app.restartSafeMode', title: 'Restart in safe mode', category: 'App', when: () => !this.life.safeMode(), run: () => this.life.startInSafeMode() },
    { id: 'app.restartNormally', title: 'Restart normally', category: 'App', when: () => this.life.safeMode(), run: () => this.life.restartNormally() },
  ];
}
