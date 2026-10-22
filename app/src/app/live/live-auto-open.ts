import { effect, inject, Injectable, InjectionToken, untracked } from '@angular/core';
import { DockService } from '../dock/dock.service';
import { LiveTail } from '../workspace/live-tail';

/** Views that open (in the background) when live tail starts, unless the user closed them before. */
export const LIVE_AUTO_OPEN_VIEWS = ['tail-status', 'live-alerts'] as const;

const KEY = 'debrief.live.autoOpen.dismissed.v1';

/** Where the views the user closed are remembered (per user, across sessions). */
export interface DismissedStore {
  load(): string[];
  save(ids: string[]): void;
}

/** Browser storage; unavailable or corrupt storage reads as "nothing dismissed". */
export const localDismissedStore: DismissedStore = {
  load() {
    try {
      const v: unknown = JSON.parse(localStorage.getItem(KEY) ?? '[]');
      return Array.isArray(v) ? v.filter((x): x is string => typeof x === 'string') : [];
    } catch {
      return [];
    }
  },
  save(ids) {
    try {
      localStorage.setItem(KEY, JSON.stringify(ids));
    } catch {
      // Not remembered this time; the views just open again next time.
    }
  },
};

export const LIVE_DISMISSED_STORE = new InjectionToken<DismissedStore>('LIVE_DISMISSED_STORE', { providedIn: 'root', factory: () => localDismissedStore });

/**
 * Opens Tail status and Alerts when live tail starts. A view the user closes while live tail runs is remembered and
 * not opened automatically again (opening it by hand forgets that).
 */
@Injectable({ providedIn: 'root' })
export class LiveAutoOpen {
  private readonly live = inject(LiveTail);
  private readonly dock = inject(DockService);
  private readonly storage = inject(LIVE_DISMISSED_STORE);
  private readonly dismissed = new Set(this.storage.load());
  /** Views this opened that are still open (closing one of them is the user's choice to remember). */
  private readonly opened = new Set<string>();

  constructor() {
    let wasOn = this.live.enabled();
    effect(() => {
      const on = this.live.enabled();
      if (on === wasOn) return;
      wasOn = on;
      untracked(() => (on ? this.started() : this.opened.clear()));
    });
    effect(() => {
      const open = this.dock.openIds();
      untracked(() => this.observe(open));
    });
  }

  private started(): void {
    for (const id of LIVE_AUTO_OPEN_VIEWS) {
      if (this.dismissed.has(id) || this.dock.isOpen(id)) continue;
      this.dock.show(id, { inactive: true });
      if (this.dock.isOpen(id)) this.opened.add(id);
    }
  }

  private observe(open: ReadonlySet<string>): void {
    let changed = false;
    for (const id of [...this.opened]) {
      if (open.has(id)) continue;
      this.opened.delete(id);
      if (this.live.enabled() && !this.dismissed.has(id)) {
        this.dismissed.add(id);
        changed = true;
      }
    }
    for (const id of LIVE_AUTO_OPEN_VIEWS) if (open.has(id) && !this.opened.has(id) && this.dismissed.delete(id)) changed = true;
    if (changed) this.storage.save([...this.dismissed]);
  }
}
