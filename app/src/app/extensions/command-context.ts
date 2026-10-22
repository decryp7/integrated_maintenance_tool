import { effect, EffectRef, inject, Injectable, Injector, untracked } from '@angular/core';
import { FieldFilter, Query, RecordView, WorkspaceSummary } from '../core/api.types';
import { WorkspaceStore } from '../core/workspace.store';
import { DockService } from '../dock/dock.service';
import { Perspectives } from '../dock/perspectives';
import { WelcomeService } from '../welcome/welcome';
import { CommandContext } from './tokens';

/** How long `jumpTo` waits for the grid (it may not be open) before resolving anyway. */
export const JUMP_TIMEOUT_MS = 5000;

/** The documented, narrow way for plugin code to act on the app. */
@Injectable({ providedIn: 'root' })
export class WorkspaceCommandContext implements CommandContext {
  private readonly store = inject(WorkspaceStore);
  private readonly dock = inject(DockService);
  private readonly welcome = inject(WelcomeService);
  private readonly injector = inject(Injector);
  private readonly perspectives = inject(Perspectives);

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
  jumpTo(id: number): Promise<void> {
    // The store's anchor makes the dock reveal the grid, which loads the page and selects the row.
    this.store.jumpTo(id);
    return new Promise((resolve) => {
      let ref: EffectRef | undefined;
      let settled = false;
      const done = () => {
        if (settled) return;
        settled = true;
        clearTimeout(timer);
        ref?.destroy();
        resolve();
      };
      const timer = setTimeout(done, JUMP_TIMEOUT_MS);
      ref = effect(
        () => {
          if (this.store.selected()?.id === id || this.store.anchorMissed()) untracked(done);
        },
        { injector: this.injector, manualCleanup: true },
      );
      if (settled) ref.destroy();
    });
  }
  showView(id: string): void {
    // The welcome screen covers the dock while nothing is open: a view the user asks for must be visible.
    this.welcome.stepAside();
    this.dock.show(id);
  }
  suggestPerspective(id: string): void {
    this.perspectives.suggest(id);
  }
  notifyError(message: string): void {
    this.store.setError(message);
  }
}
