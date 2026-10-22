import { inject, Injectable } from '@angular/core';
import { UiStateContribution } from '../../lifecycle/ui-state';
import { PlaybookStore } from './playbook.store';

/**
 * The project's playbook runs as part of the project and session UI state (key `playbooks`, `PlaybookRecord` v1).
 * The engine checks saved runs on restore and its warnings are shown in the Playbooks pane.
 */
@Injectable({ providedIn: 'root' })
export class PlaybookPersistence {
  private readonly store = inject(PlaybookStore);

  readonly contribution: UiStateContribution = {
    key: 'playbooks',
    capture: () => this.store.record(),
    restore: (v) => void this.store.restoreRecord(v),
    reset: () => this.store.clearRuns(),
  };
}
