import { effect, inject, Injectable, Injector, Signal, untracked } from '@angular/core';
import { SelectionStore } from '../core/workspace/selection.store';
import { WorkspaceDataStore } from '../core/workspace/workspace-data.store';
import { ViewRequests } from '../core/view-requests';
import { ViewRegistry } from '../extensions/view-registry';
import { PANEL } from './dock-context';

/** What the policy drives: the dock, once it has a layout. */
export interface DockPolicyTarget {
  /** True once the dock has a layout to show panels in. */
  readonly ready: Signal<boolean>;
  show(id: string): void;
  revealInspector(): void;
  syncConditionalViews(): void;
}

/**
 * The workspace's "on X, show Y" rules for the dock, kept out of the dock itself:
 * - jumping to a record (marker click, "show context") brings the events grid to the front;
 * - selecting an event slides the auto-hidden inspector out (it stays docked when the user pinned it);
 * - conditional views (e.g. the registry viewer) follow what the workspace contains;
 * - a view requested through `ViewRequests` comes to the front.
 */
@Injectable({ providedIn: 'root' })
export class DockPolicy {
  private readonly selection = inject(SelectionStore);
  private readonly data = inject(WorkspaceDataStore);
  private readonly views = inject(ViewRegistry);
  private readonly requests = inject(ViewRequests);
  private readonly injector = inject(Injector);

  /** Starts the rules for `dock`; called once by the dock. */
  start(dock: DockPolicyTarget): void {
    const injector = this.injector;
    effect(
      () => {
        const anchor = this.selection.anchor();
        if (anchor && dock.ready()) untracked(() => dock.show(PANEL.events));
      },
      { injector },
    );
    let shownId: number | null = null;
    effect(
      () => {
        const id = this.selection.selected()?.id ?? null;
        const ready = dock.ready();
        untracked(() => {
          if (id === shownId) return;
          shownId = id;
          if (id !== null && ready) dock.revealInspector();
        });
      },
      { injector },
    );
    effect(
      () => {
        const summary = this.data.summary();
        this.views.available();
        if (summary && dock.ready()) untracked(() => dock.syncConditionalViews());
      },
      { injector },
    );
    let handled = 0;
    effect(
      () => {
        const request = this.requests.request();
        if (!request || request.nonce === handled) return;
        handled = request.nonce;
        // Like a direct call: a request made while there is no layout yet is not kept for later.
        if (untracked(dock.ready)) untracked(() => dock.show(request.id));
      },
      { injector },
    );
  }
}
