import { inject, Provider } from '@angular/core';
import { WorkspaceStore } from '../core/workspace.store';
import { ON_RECOVERED, Recovery } from '../core/startup/startup-contract';

/** What the window says after Debrief reloaded it (ADR 0057): calm, with what was kept. */
export function recoveryNotice(why: Recovery): string {
  return why === 'terminated'
    ? $localize`:@@lifecycle.recovered.terminated:The window's content stopped unexpectedly and was reloaded. Your open files are kept.`
    : $localize`:@@lifecycle.recovered.unresponsive:The window did not finish starting and was reloaded. Your open files are kept.`;
}

/** The main window (and pop-outs) show the recovery reason as a workspace notice. */
export function provideRecoveryNotice(): Provider {
  return {
    provide: ON_RECOVERED,
    useFactory: () => {
      const store = inject(WorkspaceStore);
      return (why: Recovery) => store.setNotice(recoveryNotice(why));
    },
  };
}
