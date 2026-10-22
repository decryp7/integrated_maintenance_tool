import { inject, Injectable } from '@angular/core';
import { UiStateContribution } from '../lifecycle/ui-state';
import { HighlightRules, parseHighlightRules } from './bookmarks/highlight';
import { parseSavedQueries, SavedQueryStore } from './bookmarks/saved-query.store';
import { parseSavedSql, SavedSqlStore } from './bookmarks/saved-sql.store';
import { parseWatchedFolders, WatchedFoldersStore } from '../watched/watched-folders.store';

/** Saved queries, saved SQL and highlight rules as part of the project (and session) UI state. */
@Injectable({ providedIn: 'root' })
export class ProjectUiState {
  private readonly queries = inject(SavedQueryStore);
  private readonly highlights = inject(HighlightRules);
  private readonly sql = inject(SavedSqlStore);
  private readonly watched = inject(WatchedFoldersStore);

  readonly contributions: UiStateContribution[] = [
    { key: 'savedQueries', capture: () => this.queries.items(), restore: (v) => this.queries.replace(parseSavedQueries(v)), reset: () => this.queries.replace([]) },
    { key: 'highlightRules', capture: () => this.highlights.items(), restore: (v) => this.highlights.replace(parseHighlightRules(v)), reset: () => this.highlights.replace([]) },
    { key: 'watchedFolders', capture: () => this.watched.folders().map((f) => ({ path: f.path, enabled: f.enabled })), restore: (v) => void this.watched.replace(parseWatchedFolders(v)), reset: () => void this.watched.replace([]) },
    { key: 'savedSql', capture: () => this.sql.items(), restore: (v) => this.sql.replace(parseSavedSql(v)), reset: () => this.sql.replace([]) },
  ];
}
