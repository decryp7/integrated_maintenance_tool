import { effect, EnvironmentProviders, inject, Injectable, makeEnvironmentProviders, provideEnvironmentInitializer, untracked } from '@angular/core';
import { WorkspaceStore } from '../core/workspace.store';
import { IMT_COMMANDS, ImtCommand } from '../extensions/tokens';
import { UI_STATE_CONTRIBUTIONS, UiStateContribution } from '../lifecycle/ui-state';
import { QueryUndo } from './query-undo';
import { UndoStack } from './undo-stack';

export const UNDO_COMMAND = 'edit.undo';
export const REDO_COMMAND = 'edit.redo';

/** Undo and redo as commands (Edit menu, palette, Mod+Z / Mod+Shift+Z), with feedback in the toasts. */
@Injectable({ providedIn: 'root' })
export class UndoCommands {
  private readonly undo = inject(UndoStack);
  private readonly store = inject(WorkspaceStore);

  constructor() {
    // Failures (an engine that refuses the previous profile, ...) are shown like any other error.
    effect(() => {
      const error = this.undo.lastError();
      if (error) untracked(() => this.store.setError(error));
    });
  }

  readonly commands: ImtCommand[] = [
    { id: UNDO_COMMAND, title: $localize`:Command title|@@undo.cmd.undo:Undo`, category: $localize`:Command palette category|@@undo.cmd.category:Edit`, keybinding: 'Mod+Z', run: () => this.step('undo') },
    { id: REDO_COMMAND, title: $localize`:Command title|@@undo.cmd.redo:Redo`, category: $localize`:Command palette category|@@undo.cmd.category:Edit`, keybinding: 'Mod+Shift+Z', run: () => this.step('redo') },
  ];

  /** Undoes or redoes the latest edit; tells the user what happened (or that there was nothing to do). */
  async step(op: 'undo' | 'redo'): Promise<void> {
    const label = op === 'undo' ? this.undo.undoLabel() : this.undo.redoLabel();
    if (!label) {
      this.store.setNotice(op === 'undo' ? $localize`:@@undo.nothingToUndo:Nothing to undo` : $localize`:@@undo.nothingToRedo:Nothing to redo`);
      return;
    }
    if (await (op === 'undo' ? this.undo.undo() : this.undo.redo())) this.store.setNotice(op === 'undo' ? $localize`:@@undo.undone:Undone: ${label}:label:` : $localize`:@@undo.redone:Redone: ${label}:label:`);
  }
}

/**
 * Forgets the history whenever saved UI state is applied: a project, a session or a bundle was opened, or a new
 * project started. The edits referred to what was there before. Saves nothing (`capture` is undefined).
 */
export function undoResetContribution(undo: UndoStack, query: QueryUndo): UiStateContribution {
  const reset = () => {
    undo.clear();
    // After the other contributions restored theirs (same task), so the restored query is not an edit.
    queueMicrotask(() => {
      undo.clear();
      query.rebase();
    });
  };
  return { key: 'undoHistory', tracksChanges: false, capture: () => undefined, restore: reset, reset };
}

/** One line in `app.config.ts`: the undo/redo commands, filter-change tracking and the reset on project switch. */
export function provideUndo(): EnvironmentProviders {
  return makeEnvironmentProviders([
    { provide: IMT_COMMANDS, multi: true, useFactory: () => inject(UndoCommands).commands },
    { provide: UI_STATE_CONTRIBUTIONS, multi: true, useFactory: () => [undoResetContribution(inject(UndoStack), inject(QueryUndo))] },
    provideEnvironmentInitializer(() => inject(QueryUndo).start()),
  ]);
}
