import { computed, effect, inject, Injectable, Injector, signal, untracked } from '@angular/core';
import { WorkspaceStore } from '../core/workspace.store';
import { DockService } from '../dock/dock.service';
import { NavHistory, NavPlace } from './nav-history';

/** After going back or forward, changes seen for this long (ms) are its echo (selection, panels sliding) and not recorded. */
export const NAV_ECHO_MS = 600;

/**
 * Back and forward through where the user has been (Alt+Left / Alt+Right, the mouse's back and forward buttons):
 * events they selected, time ranges they zoomed to, views they brought to the front. The places are recorded by
 * watching the workspace, so every control that moves them counts; going back or forward re-applies a place and the
 * watcher then sees it as the current one and records nothing.
 */
@Injectable({ providedIn: 'root' })
export class NavigationHistory {
  private readonly store = inject(WorkspaceStore);
  private readonly dock = inject(DockService);
  private readonly injector = inject(Injector);
  private readonly history = new NavHistory();
  private readonly version = signal(0);
  private started = false;
  /** Places seen before this time (ms) are the echo of a back or forward, not something the user did. */
  private quietUntil = 0;
  /** Clock, replaceable in tests. */
  now: () => number = () => Date.now();

  readonly canBack = computed(() => (this.version(), this.history.canBack));
  readonly canForward = computed(() => (this.version(), this.history.canForward));

  /** Starts watching (idempotent). */
  start(): void {
    if (this.started) return;
    this.started = true;
    effect(
      () => {
        const id = this.store.selected()?.id;
        if (id !== undefined) untracked(() => this.observe({ kind: 'event', id }));
      },
      { injector: this.injector },
    );
    effect(
      () => {
        const q = this.store.query();
        if (q.window) return;
        untracked(() => this.observe({ kind: 'range', from: q.from ?? null, to: q.to ?? null }));
      },
      { injector: this.injector },
    );
    effect(
      () => {
        const id = this.dock.activeViewId();
        // Tool windows (the inspector sliding out on a selection) come and go by themselves: they are not places.
        if (id && untracked(() => this.dock.kindOf?.(id)) !== 'tool') untracked(() => this.observe({ kind: 'view', id }));
      },
      { injector: this.injector },
    );
  }

  /** @internal Records a place (exposed for tests). */
  observe(place: NavPlace): void {
    if (this.now() < this.quietUntil) return;
    if (this.history.record(place, this.now())) this.version.update((v) => v + 1);
  }

  back(): boolean {
    return this.go(this.history.back());
  }

  forward(): boolean {
    return this.go(this.history.forward());
  }

  /** Forgets every place (another project or workspace replaces what they referred to). */
  clear(): void {
    this.history.clear();
    this.version.update((v) => v + 1);
  }

  private go(place: NavPlace | null): boolean {
    this.version.update((v) => v + 1);
    if (!place) return false;
    this.quietUntil = this.now() + NAV_ECHO_MS;
    switch (place.kind) {
      case 'event':
        this.store.jumpTo(place.id);
        break;
      case 'range':
        this.store.patchQuery({ from: place.from, to: place.to });
        break;
      case 'view':
        this.dock.show(place.id);
        break;
    }
    return true;
  }
}
