import { inject, Injectable } from '@angular/core';
import { UiStateContribution } from '../lifecycle/ui-state';
import { HighlightRules, parseHighlightRules } from './bookmarks/highlight';
import { parseSavedQueries, SavedQueryStore } from './bookmarks/saved-query.store';

/** Saved queries and highlight rules as part of the project (and session) UI state. */
@Injectable({ providedIn: 'root' })
export class ProjectUiState {
  private readonly queries = inject(SavedQueryStore);
  private readonly highlights = inject(HighlightRules);

  readonly contributions: UiStateContribution[] = [
    { key: 'savedQueries', capture: () => this.queries.items(), restore: (v) => this.queries.replace(parseSavedQueries(v)), reset: () => this.queries.replace([]) },
    { key: 'highlightRules', capture: () => this.highlights.items(), restore: (v) => this.highlights.replace(parseHighlightRules(v)), reset: () => this.highlights.replace([]) },
  ];
}
