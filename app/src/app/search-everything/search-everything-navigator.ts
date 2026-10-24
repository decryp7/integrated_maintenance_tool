import { inject, Injectable } from '@angular/core';
import { ExceptionFocus } from '../analysis/exception-focus';
import { KnowledgeStore } from '../ai/knowledge/knowledge.store';
import { WorkspaceStore } from '../core/workspace.store';
import { COMMAND_CONTEXT } from '../extensions/command-registry';
import { NOTES_VIEW } from '../notes/note-actions';
import { DialogService } from '../ui/dialog.service';
import { DocScope } from '../core/api.types';
import { SearchTarget } from './search-everything.model';

/** The events grid's view id. */
export const EVENTS_VIEW = 'events';
/** The Exceptions view's id. */
export const EXCEPTIONS_VIEW = 'exceptions';

/** Goes where a search result points: reveals an event, filters the grid, or shows the view that holds it. */
@Injectable({ providedIn: 'root' })
export class SearchEverythingNavigator {
  private readonly context = inject(COMMAND_CONTEXT);
  private readonly workspace = inject(WorkspaceStore);
  private readonly dialogs = inject(DialogService);
  private readonly knowledge = inject(KnowledgeStore);
  private readonly exceptions = inject(ExceptionFocus);

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
      case 'exception':
        this.exceptions.request(target.fingerprint);
        ctx.showView(EXCEPTIONS_VIEW);
        if (target.sample !== null) void ctx.jumpTo(target.sample);
        break;
      case 'note':
        ctx.showView(NOTES_VIEW);
        if (target.event !== null) void ctx.jumpTo(target.event);
        break;
      case 'knowledge':
        void this.editKnowledge(target.scope, target.id);
        break;
    }
  }

  /** Opens the knowledge file in the editor; the files are listed first when this window has not yet. */
  private async editKnowledge(scope: DocScope, id: string): Promise<void> {
    if (!this.knowledge.list()) await this.knowledge.load();
    const file = (scope === 'project' ? this.knowledge.project() : this.knowledge.global()).find((f) => f.id === id);
    if (file) {
      const maxBytes = this.knowledge.list()?.maxFileBytes ?? 64 * 1024;
      void import('../ai/knowledge/knowledge-editor').then((m) => m.openKnowledgeEditor(this.dialogs, file, scope, maxBytes));
    } else this.context.notifyError($localize`:@@search.knowledge.gone:This knowledge file no longer exists.`);
  }
}
