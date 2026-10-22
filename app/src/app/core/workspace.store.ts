import { inject, Injectable } from '@angular/core';
import { QueryStore } from './workspace/query.store';
import { SelectionStore } from './workspace/selection.store';
import { ShellUiStore } from './workspace/shell-ui.store';
import { WorkspaceStatusStore } from './workspace/status.store';
import { TimelineStore } from './workspace/timeline-data.store';
import { UiPrefsStore } from './workspace/ui-prefs.store';
import { WorkspaceDataStore } from './workspace/workspace-data.store';

export { EMPTY_QUERY } from './workspace/query.store';
export { loadPrefs, savePrefs } from './workspace/ui-prefs.store';
export type { SchemeMode, ThemeId } from './workspace/ui-prefs.store';
export type { WorkspaceBusy } from './workspace/status.store';

/**
 * One handle on the workspace's focused stores, for code that uses several of them. It owns no state and no
 * effects: everything delegates to
 * - `WorkspaceDataStore`: the summary, generations and the commands that replace the data;
 * - `QueryStore`: the query and the search box;
 * - `TimelineStore`: the workspace timeline;
 * - `SelectionStore`: the selected record and jump-to requests;
 * - `UiPrefsStore` / `ShellUiStore`: view preferences and transient shell flags;
 * - `WorkspaceStatusStore`: busy state, error banner and notices.
 * New code injects the store it needs; this facade keeps the many existing consumers working while they move.
 */
@Injectable({ providedIn: 'root' })
export class WorkspaceStore {
  private readonly data = inject(WorkspaceDataStore);
  private readonly q = inject(QueryStore);
  private readonly tl = inject(TimelineStore);
  private readonly sel = inject(SelectionStore);
  private readonly prefs = inject(UiPrefsStore);
  private readonly shell = inject(ShellUiStore);
  private readonly status = inject(WorkspaceStatusStore);

  // Data and generations.
  readonly summary = this.data.summary;
  readonly dataVersion = this.data.dataVersion;
  readonly sourcesVersion = this.data.sourcesVersion;
  readonly appendVersion = this.data.appendVersion;
  readonly profileVersion = this.data.profileVersion;
  readonly importVersion = this.data.importVersion;
  readonly markerKindMap = this.data.markerKindMap;
  readonly fieldNames = this.data.fieldNames;
  readonly numericFields = this.data.numericFields;
  readonly applySummary = this.data.applySummary;
  readonly profileSaved = this.data.profileSaved;
  readonly applyProfile = this.data.applyProfile;
  readonly followReread = this.data.followReread;
  readonly init = this.data.init;
  readonly refreshSummary = this.data.refreshSummary;
  readonly openPaths = this.data.openPaths;
  readonly pickAndOpen = this.data.pickAndOpen;
  readonly removeSource = this.data.removeSource;
  readonly clearWorkspace = this.data.clearWorkspace;
  readonly reload = this.data.reload;

  // Query.
  readonly query = this.q.query;
  readonly textInput = this.q.textInput;
  readonly effectiveGroupBy = this.q.effectiveGroupBy;
  readonly effectiveQuery = this.q.effectiveQuery;
  readonly hasRange = this.q.hasRange;
  readonly markedOnly = this.q.markedOnly;
  readonly patchQuery = this.q.patchQuery;
  readonly setTextInput = this.q.setTextInput;
  readonly setRegex = this.q.setRegex;
  readonly setCaseSensitive = this.q.setCaseSensitive;
  readonly toggleLevel = this.q.toggleLevel;
  readonly addFilter = this.q.addFilter;
  readonly removeFilter = this.q.removeFilter;
  readonly setGroupBy = this.q.setGroupBy;
  readonly toggleGroup = this.q.toggleGroup;
  readonly clearGroups = this.q.clearGroups;
  readonly setRange = this.q.setRange;
  readonly resetRange = this.q.resetRange;
  readonly setMarkedOnly = this.q.setMarkedOnly;
  readonly clearQuery = this.q.clearQuery;
  readonly restoreQuery = this.q.restoreQuery;

  // Timeline.
  readonly timeline = this.tl.timeline;
  readonly timelineLoading = this.tl.loading;
  readonly timelineError = this.tl.error;
  readonly buckets = this.tl.buckets;
  readonly instanceCount = this.tl.instanceCount;
  readonly retryTimeline = this.tl.retry;
  readonly setBuckets = this.tl.setBuckets;

  // Selection.
  readonly selected = this.sel.selected;
  readonly anchor = this.sel.anchor;
  readonly anchorMissed = this.sel.anchorMissed;
  readonly matchTotal = this.sel.matchTotal;
  readonly selectedHidden = this.sel.selectedHidden;
  readonly select = this.sel.select;
  readonly jumpTo = this.sel.jumpTo;
  readonly anchorResolved = this.sel.anchorResolved;
  readonly setMatchTotal = this.sel.setMatchTotal;

  // Preferences and shell flags.
  readonly timezone = this.prefs.timezone;
  readonly alwaysShowLifecycle = this.prefs.alwaysShowLifecycle;
  readonly theme = this.prefs.theme;
  readonly scheme = this.prefs.scheme;
  readonly uiFont = this.prefs.uiFont;
  readonly descending = this.prefs.descending;
  readonly extraColumns = this.prefs.extraColumns;
  readonly setTimezone = this.prefs.setTimezone;
  readonly setAlwaysShowLifecycle = this.prefs.setAlwaysShowLifecycle;
  readonly setTheme = this.prefs.setTheme;
  readonly setScheme = this.prefs.setScheme;
  readonly setUiFont = this.prefs.setUiFont;
  readonly toggleSort = this.prefs.toggleSort;
  readonly addColumn = this.prefs.addColumn;
  readonly removeColumn = this.prefs.removeColumn;
  readonly sidebarOpen = this.shell.sidebarOpen;
  readonly dropHover = this.shell.dropHover;
  readonly focusSearchNonce = this.shell.focusSearchNonce;
  readonly toggleSidebar = this.shell.toggleSidebar;
  readonly setDropHover = this.shell.setDropHover;
  readonly focusSearch = this.shell.focusSearch;

  // Status.
  readonly busy = this.status.busy;
  readonly busyLabel = this.status.busyLabel;
  readonly error = this.status.error;
  readonly notice = this.status.notice;
  readonly dismissError = this.status.dismissError;
  readonly setError = this.status.setError;
  readonly setNotice = this.status.setNotice;
  readonly holdNotice = this.status.holdNotice;
  readonly releaseNotice = this.status.releaseNotice;
}
