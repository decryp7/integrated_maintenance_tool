import { inject, Injectable } from '@angular/core';
import { RecordView } from '../core/api.types';
import { WorkspaceStore } from '../core/workspace.store';
import { copyTextFor } from '../context/record-actions';
import { GridSelection } from '../grid/grid-selection';
import { DebriefCommand } from '../extensions/tokens';
import { copyText } from '../tools/clipboard';

/** The selected events as JSON: one object, or an array for a multi-selection. Pure. */
export function eventsJson(records: readonly RecordView[]): string {
  return JSON.stringify(records.length === 1 ? records[0] : records, null, 2);
}

/**
 * The Edit menu's commands besides Undo and Redo: Copy (the selected text, else the selected events' messages), Copy as
 * JSON and Find (Ctrl+F focuses the search box). Cut and Paste stay with text fields (the browser's own keys): a menu
 * cannot read the clipboard without asking.
 */
@Injectable({ providedIn: 'root' })
export class EditCommands {
  private readonly store = inject(WorkspaceStore);
  private readonly grid = inject(GridSelection);

  /** The events "the selection" means: the grid's multi-selection, else the selected event. */
  private selected(): readonly RecordView[] {
    const many = this.grid.records();
    if (many.length) return many;
    const one = this.store.selected();
    return one ? [one] : [];
  }

  readonly commands: DebriefCommand[] = (() => {
    const category = $localize`:@@shell.edit.category:Edit`;
    const nothing = () => this.store.setNotice($localize`:@@shell.edit.nothingSelected:Select events or text first.`);
    return [
      {
        id: 'edit.copy',
        title: $localize`:@@shell.edit.copy:Copy`,
        category,
        run: async () => {
          const records = this.selected();
          const first = records[0];
          const windowText = window.getSelection()?.toString() ?? '';
          const text = first ? copyTextFor(first, { records: [...records] }, windowText) : windowText;
          if (!text.trim()) return nothing();
          await copyText(text);
        },
      },
      {
        id: 'edit.copyJson',
        title: $localize`:@@shell.edit.copyJson:Copy as JSON`,
        category,
        run: async () => {
          const records = this.selected();
          if (!records.length) return nothing();
          await copyText(eventsJson(records));
        },
      },
      { id: 'edit.find', title: $localize`:@@shell.edit.find:Find`, category, keybinding: 'Mod+F', run: () => this.store.focusSearch() },
    ];
  })();
}
