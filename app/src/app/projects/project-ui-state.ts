import { inject, Injectable } from '@angular/core';
import { UiStateContribution } from '../lifecycle/ui-state';
import { BookmarkStore, parseBookmarks } from './bookmarks/bookmark.store';
import { HighlightRules, parseHighlightRules } from './bookmarks/highlight';
import { parseSavedQueries, SavedQueryStore } from './bookmarks/saved-query.store';

/** Bookmarks, saved queries and highlight rules as part of the project (and session) UI state. */
@Injectable({ providedIn: 'root' })
export class ProjectUiState {
  private readonly bookmarks = inject(BookmarkStore);
  private readonly queries = inject(SavedQueryStore);
  private readonly highlights = inject(HighlightRules);

  readonly contributions: UiStateContribution[] = [
    { key: 'bookmarks', capture: () => this.bookmarks.items(), restore: (v) => this.bookmarks.replace(parseBookmarks(v)), reset: () => this.bookmarks.replace([]) },
    { key: 'savedQueries', capture: () => this.queries.items(), restore: (v) => this.queries.replace(parseSavedQueries(v)), reset: () => this.queries.replace([]) },
    { key: 'highlightRules', capture: () => this.highlights.items(), restore: (v) => this.highlights.replace(parseHighlightRules(v)), reset: () => this.highlights.replace([]) },
  ];
}
