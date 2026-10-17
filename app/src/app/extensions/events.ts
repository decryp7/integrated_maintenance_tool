import { inject, Injectable, Injector, Signal } from '@angular/core';
import { toObservable } from '@angular/core/rxjs-interop';
import { filter, map, Observable } from 'rxjs';
import { Query, RecordView, WorkspaceSummary } from '../core/api.types';
import { TimeZoneMode } from '../core/format';
import { WorkspaceStore } from '../core/workspace.store';

/**
 * Read-only stream of what happens in the app, for plugins. Plugins can only observe; to change
 * something they use the `CommandContext` passed to their commands.
 */
@Injectable({ providedIn: 'root', useFactory: () => inject(StoreImtEvents) })
export abstract class ImtEvents {
  /** Latest workspace summary (null until loaded). */
  abstract readonly workspace: Signal<WorkspaceSummary | null>;
  /** Current query (search text, filters, range, ...). */
  abstract readonly query: Signal<Query>;
  /** Record shown in the inspector. */
  abstract readonly selectedRecord: Signal<RecordView | null>;
  /** Zone the user chose for showing times (`utc` or `local`); format times in it. */
  abstract readonly timezone: Signal<TimeZoneMode>;
  /** Emits after sources changed (open, remove, reload, profile save). */
  abstract readonly workspaceChanged: Observable<WorkspaceSummary>;
  /** Emits when the query changes. */
  abstract readonly querySelectionChanged: Observable<Query>;
  /** Emits when a record is selected or deselected. */
  abstract readonly recordSelected: Observable<RecordView | null>;
  /** Emits after the profile was saved and applied. */
  abstract readonly profileSaved: Observable<WorkspaceSummary>;
}

/** `ImtEvents` backed by the workspace store. */
@Injectable({ providedIn: 'root' })
export class StoreImtEvents extends ImtEvents {
  private readonly store = inject(WorkspaceStore);
  private readonly injector = inject(Injector);

  readonly workspace = this.store.summary;
  readonly query = this.store.query;
  readonly selectedRecord = this.store.selected;
  readonly timezone = this.store.timezone;
  readonly workspaceChanged: Observable<WorkspaceSummary> = toObservable(this.store.dataVersion, { injector: this.injector }).pipe(
    map(() => this.store.summary()),
    filter((s): s is WorkspaceSummary => s !== null),
  );
  readonly querySelectionChanged = toObservable(this.store.query, { injector: this.injector });
  readonly recordSelected = toObservable(this.store.selected, { injector: this.injector });
  readonly profileSaved: Observable<WorkspaceSummary> = toObservable(this.store.profileVersion, { injector: this.injector }).pipe(
    filter((v) => v > 0),
    map(() => this.store.summary()),
    filter((s): s is WorkspaceSummary => s !== null),
  );
}
