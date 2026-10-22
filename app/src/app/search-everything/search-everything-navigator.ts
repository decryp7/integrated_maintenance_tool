import { inject, Injectable } from '@angular/core';
import { WorkspaceStore } from '../core/workspace.store';
import { COMMAND_CONTEXT } from '../extensions/command-registry';
import { NOTES_VIEW } from '../notes/note-actions';
import { SettingsDialogService } from '../settings/settings-dialog';
import { SearchTarget } from './search-everything.model';

/** The events grid's view id. */
export const EVENTS_VIEW = 'events';
/** The Settings section with the knowledge files. */
export const KNOWLEDGE_SECTION = 'ai';

/** Goes where a search result points: reveals an event, filters the grid, or shows the view that holds it. */
@Injectable({ providedIn: 'root' })
export class SearchEverythingNavigator {
  private readonly context = inject(COMMAND_CONTEXT);
  private readonly workspace = inject(WorkspaceStore);
  private readonly settings = inject(SettingsDialogService);

  go(target: SearchTarget): void {
    const ctx = this.context;
    switch (target.kind) {
      case 'event':
        ctx.showView(EVENTS_VIEW);
        void ctx.jumpTo(target.id);
        break;
      case 'allEvents':
        this.workspace.setTextInput(target.text);
        ctx.showView(EVENTS_VIEW);
        break;
      case 'source':
        ctx.setQuery({ sources: [target.id] });
        ctx.showView(EVENTS_VIEW);
        break;
      case 'field':
        ctx.addFilter(target.value === null ? { field: target.field, op: 'exists' } : { field: target.field, op: 'eq', value: target.value });
        ctx.showView(EVENTS_VIEW);
        break;
      case 'marker':
        ctx.setQuery({ markers: [target.marker] });
        ctx.showView(EVENTS_VIEW);
        break;
      case 'note':
        ctx.showView(NOTES_VIEW);
        if (target.event !== null) void ctx.jumpTo(target.event);
        break;
      case 'knowledge':
        this.settings.open(KNOWLEDGE_SECTION);
        break;
    }
  }
}
