import { inject, Injectable } from '@angular/core';
import { FieldFilter, Query, RecordView, WorkspaceSummary } from '../core/api.types';
import { WorkspaceStore } from '../core/workspace.store';
import { DockService } from '../dock/dock.service';
import { CommandContext } from './tokens';

/** The documented, narrow way for plugin code to act on the app. */
@Injectable({ providedIn: 'root' })
export class WorkspaceCommandContext implements CommandContext {
  private readonly store = inject(WorkspaceStore);
  private readonly dock = inject(DockService);

  get summary(): WorkspaceSummary | null {
    return this.store.summary();
  }
  openPaths(paths: string[]): Promise<void> {
    return this.store.openPaths(paths);
  }
  setQuery(patch: Partial<Query>): void {
    this.store.patchQuery(patch);
  }
  addFilter(filter: FieldFilter): void {
    this.store.addFilter(filter);
  }
  selectRecord(record: RecordView | null): void {
    this.store.select(record);
  }
  showView(id: string): void {
    this.dock.show(id);
  }
  notifyError(message: string): void {
    this.store.setError(message);
  }
}
