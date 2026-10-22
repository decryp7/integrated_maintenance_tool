import { inject, Injectable } from '@angular/core';
import { UiStateContribution } from '../lifecycle/ui-state';
import { HighlightRules, parseHighlightRules } from './bookmarks/highlight';
import { parseSavedQueries, SavedQueryStore } from './bookmarks/saved-query.store';
import { FilterSetStore } from '../lenses/filter-set.store';
import { LensStore } from '../lenses/lens.store';
import { parseFilterSets, parseSavedLenses, versioned } from '../lenses/lens-model';
import { parseSavedSql, SavedSqlStore } from './bookmarks/saved-sql.store';

/** Saved queries, saved SQL, highlight rules, lenses and filter sets as part of the project (and session) UI state. */
@Injectable({ providedIn: 'root' })
export class ProjectUiState {
  private readonly queries = inject(SavedQueryStore);
  private readonly highlights = inject(HighlightRules);
  private readonly sql = inject(SavedSqlStore);
  private readonly lenses = inject(LensStore);
  private readonly filterSets = inject(FilterSetStore);

  readonly contributions: UiStateContribution[] = [
    { key: 'savedQueries', capture: () => this.queries.items(), restore: (v) => this.queries.replace(parseSavedQueries(v)), reset: () => this.queries.replace([]) },
    { key: 'highlightRules', capture: () => this.highlights.items(), restore: (v) => this.highlights.replace(parseHighlightRules(v)), reset: () => this.highlights.replace([]) },
    { key: 'savedSql', capture: () => this.sql.items(), restore: (v) => this.sql.replace(parseSavedSql(v)), reset: () => this.sql.replace([]) },
    { key: 'lenses', capture: () => versioned(this.lenses.saved()), restore: (v) => this.lenses.replace(parseSavedLenses(v)), reset: () => this.lenses.replace([]) },
    { key: 'filterSets', capture: () => versioned(this.filterSets.sets()), restore: (v) => this.filterSets.replace(parseFilterSets(v)), reset: () => this.filterSets.replace([]) },
  ];
}
